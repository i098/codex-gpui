//! Client for `codex app-server`: newline-delimited JSON-RPC over stdio, without the
//! `"jsonrpc"` field, the same transport the Codex desktop app uses.
//!
//! Protocol types are hand-rolled in [`types`] from `codex-rs/app-server-protocol` at tag
//! `rust-v0.162.0-alpha.17.2` (the exact version the desktop app bundles). The upstream crate
//! is not a dependency: it pulls in `codex-protocol`, `codex-rollout`, `codex-secrets`, `rmcp`,
//! `zstd` and most of the agent core, while the UI reads a few fields of about a dozen methods.
//! Fields the UI does not read stay `serde_json::Value` or are left out.
//!
//! The API is runtime-agnostic: one reader thread per connection, `async-channel` for
//! replies and events, so GPUI's executors (or any other) can drive it.
//!
//! A `Backend` carries the client's messages: `Lines` writes them to `codex app-server` (or
//! a replay) as they are; `acp::Acp` translates them for an Agent Client Protocol agent.

mod acp;
mod automation;
pub use automation::AutomationTurn;
mod cloud;
pub use cloud::CLOUD_ENDPOINT;
pub mod imports;
mod replay;
pub mod security;
pub mod storage;
pub mod types;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context, Result, anyhow};
use async_channel::{Receiver, Sender};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use types::*;

/// A message the server sent without being asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Notification {
        method: String,
        params: Value,
    },
    /// Answer with [`Bridge::respond`] using the same `id` (approvals, user input, ...).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// A spawned server process exited (stdout closed); replayed transcripts never send it.
    Closed,
}

/// JSON-RPC error returned by the server; reach it with `err.downcast_ref::<RpcError>()`.
#[derive(Debug, Clone, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Value,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

type Reply = Result<Value, RpcError>;
/// `None` once the server has closed stdout: later requests fail instead of hanging.
type Pending = Arc<Mutex<Option<HashMap<i64, Sender<Reply>>>>>;
type Subscribers = Arc<Mutex<Vec<std::sync::mpsc::Sender<Event>>>>;
type EventSubscribers = Arc<Mutex<Vec<Sender<Event>>>>;

struct PendingRequest<'a> {
    id: i64,
    pending: &'a Pending,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.lock().as_mut() {
            pending.remove(&self.id);
        }
    }
}

/// Where a [`Bridge`] sends the client's requests, notifications and answers.
trait Backend: Send {
    fn send(&mut self, msg: Value) -> Result<()>;
}

/// Newline-delimited JSON as is: the `codex app-server` transport.
struct Lines(Box<dyn Write + Send>);

impl Backend for Lines {
    fn send(&mut self, msg: Value) -> Result<()> {
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        self.0.write_all(line.as_bytes())?;
        self.0.flush()?;
        Ok(())
    }
}

/// Where server messages land: answers wake their [`Bridge::request`], the rest are events.
#[derive(Clone)]
struct Sink {
    pending: Pending,
    events: EventSubscribers,
    subscribers: Subscribers,
}

impl Sink {
    fn new() -> (Sink, Receiver<Event>) {
        let (events, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(Some(HashMap::new())));
        (
            Sink {
                pending,
                events: Arc::new(Mutex::new(vec![events])),
                subscribers: Arc::default(),
            },
            rx,
        )
    }

    fn reply(&self, id: i64, reply: Reply) {
        if let Some(waiter) = self.pending.lock().as_mut().and_then(|map| map.remove(&id)) {
            let _ = waiter.try_send(reply);
        }
    }

    fn event(&self, event: Event) {
        self.subscribers
            .lock()
            .retain(|s| s.send(event.clone()).is_ok());
        let mut subscribers = self.events.lock();
        subscribers.retain(|subscriber| !subscriber.is_closed());
        if let Some((last, others)) = subscribers.split_last() {
            for subscriber in others {
                let _ = subscriber.try_send(event.clone());
            }
            let _ = last.try_send(event);
        }
    }

    fn notify(&self, method: &str, params: Value) {
        self.event(Event::Notification {
            method: method.into(),
            params,
        });
    }

    fn incoming(&self, msg: Incoming) {
        match (msg.method, msg.id) {
            (Some(method), Some(id)) => self.event(Event::Request {
                id,
                method,
                params: msg.params,
            }),
            (Some(method), None) => self.notify(&method, msg.params),
            (None, Some(id)) => {
                if let Some(id) = id.as_i64() {
                    self.reply(id, msg.error.map_or(Ok(msg.result), Err));
                }
            }
            (None, None) => {}
        }
    }

    /// Dropping the waiters wakes them with "closed"; later requests fail at once.
    fn close(&self) {
        self.pending.lock().take();
        self.subscribers.lock().clear();
        self.events.lock().clear();
    }
}

pub struct Bridge {
    backend: Mutex<Box<dyn Backend>>,
    pending: Pending,
    next_id: AtomicI64,
    event_subscribers: EventSubscribers,
    initialized: async_lock::Mutex<Option<InitializeResponse>>,
    child: Option<Child>,
    acp: bool,
    cloud_account_id: Option<String>,
    automation_turns: Mutex<HashMap<String, Arc<AutomationTurn>>>,
    subscribers: Subscribers,
}

/// `$CODEX_BIN`, else the binary bundled in ChatGPT.app, else `codex` on `PATH`; runs `app-server`.
pub fn default_command() -> Command {
    const BUNDLED: &str = "/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex";
    let bin = std::env::var_os("CODEX_BIN")
        .map(PathBuf::from)
        .or_else(|| Path::new(BUNDLED).exists().then(|| BUNDLED.into()))
        .unwrap_or_else(|| "codex".into());
    let mut cmd = Command::new(bin);
    cmd.arg("app-server");
    cmd
}

/// Installs the pinned Computer Use relay and hooks for the exact app-server executable.
pub fn setup_computer_use() -> Result<()> {
    let codex = default_command();
    let status = Command::new("/bin/bash")
        .args([
            "-c",
            include_str!("../../../tools/lcu-setup.sh"),
            "lcu-setup.sh",
            "--codex",
        ])
        .arg(codex.get_program())
        .status()
        .context("starting Computer Use setup")?;
    anyhow::ensure!(status.success(), "Computer Use setup failed: {status}");
    Ok(())
}

/// `$CODEX_GPUI_ACP` split on whitespace (`omp acp`): an ACP agent to run instead of
/// `codex app-server`, with [`Bridge::spawn_acp`].
pub fn acp_command() -> Option<Command> {
    let line = std::env::var("CODEX_GPUI_ACP").ok()?;
    let mut words = line.split_whitespace();
    let mut cmd = Command::new(words.next()?);
    cmd.args(words);
    Some(cmd)
}

impl Bridge {
    /// Spawns the app-server with piped stdin/stdout; stderr is left as configured on `cmd`.
    pub fn spawn(cmd: Command) -> Result<(Bridge, Receiver<Event>)> {
        let (child, stdout, stdin) = spawn_piped(cmd)?;
        Ok(Self::from_io(stdout, stdin, Some(child)))
    }

    /// Spawns an Agent Client Protocol agent (`omp acp`, ...) and talks to it through
    /// `acp::Acp`: chats, streaming, Stop and permission prompts. The other app-server
    /// methods fail with a "not supported by the ACP backend" error.
    pub fn spawn_acp(cmd: Command) -> Result<(Bridge, Receiver<Event>)> {
        let (child, stdout, stdin) = spawn_piped(cmd)?;
        Ok(Self::acp_io(stdout, stdin, Some(child)))
    }

    /// Serves responses and notifications from a recorded JSONL transcript (see `replay.rs`).
    pub fn replay(path: &Path) -> Result<(Bridge, Receiver<Event>)> {
        let script = replay::load(path)?;
        let (client_read, server_write) = std::io::pipe()?;
        let (server_read, client_write) = std::io::pipe()?;
        std::thread::spawn(move || {
            replay::serve(&script, BufReader::new(server_read), server_write)
        });
        Ok(Self::from_io(client_read, client_write, None))
    }

    fn from_io(
        read: impl Read + Send + 'static,
        write: impl Write + Send + 'static,
        child: Option<Child>,
    ) -> (Bridge, Receiver<Event>) {
        let (sink, events) = Sink::new();
        let listener = sink.clone();
        let spawned = child.is_some();
        std::thread::spawn(move || {
            read_loop(BufReader::new(read), &listener);
            if spawned {
                listener.event(Event::Closed);
            }
            listener.close();
        });
        let backend = Box::new(Lines(Box::new(write)));
        (Self::new(backend, sink, child, false), events)
    }

    fn acp_io(
        read: impl Read + Send + 'static,
        write: impl Write + Send + 'static,
        child: Option<Child>,
    ) -> (Bridge, Receiver<Event>) {
        let (sink, events) = Sink::new();
        let backend = Box::new(acp::Acp::start(read, Lines(Box::new(write)), sink.clone()));
        (Self::new(backend, sink, child, true), events)
    }

    fn new(backend: Box<dyn Backend>, sink: Sink, child: Option<Child>, acp: bool) -> Self {
        Bridge {
            backend: Mutex::new(backend),
            pending: sink.pending,
            event_subscribers: sink.events,
            initialized: async_lock::Mutex::new(None),
            next_id: AtomicI64::new(1),
            child,
            acp,
            cloud_account_id: None,
            automation_turns: Mutex::new(HashMap::new()),
            subscribers: sink.subscribers,
        }
    }

    pub fn subscribe_async(&self) -> Receiver<Event> {
        let (sender, receiver) = async_channel::unbounded();
        let mut subscribers = self.event_subscribers.lock();
        if self.pending.lock().is_some() {
            subscribers.push(sender);
        }
        receiver
    }

    /// Whether the app-server has closed its response stream.
    pub fn is_closed(&self) -> bool {
        self.pending.lock().is_none()
    }

    /// Whether this connection runs an ACP agent ([`Bridge::spawn_acp`]).
    pub fn is_acp(&self) -> bool {
        self.acp
    }

    /// Observes events without taking them from the renderer's event stream.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<Event> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.subscribers.lock().push(tx);
        rx
    }

    pub async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R> {
        let msg = message(method, params)?;
        self.observe_automation_request(method, &msg["params"])
            .await?;
        self.request_message(method, msg).await
    }

    async fn request_inner<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R> {
        self.request_message(method, message(method, params)?).await
    }

    async fn request_message<R: DeserializeOwned>(
        &self,
        method: &str,
        mut msg: Value,
    ) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        msg["id"] = id.into();
        let (tx, rx) = async_channel::bounded(1);
        self.pending
            .lock()
            .as_mut()
            .context("app-server closed")?
            .insert(id, tx);
        let _pending = PendingRequest {
            id,
            pending: &self.pending,
        };
        self.write(msg)?;
        let result = rx
            .recv()
            .await
            .map_err(|_| anyhow!("app-server closed before answering {method}"))??;
        serde_json::from_value(result).with_context(|| format!("decoding {method} response"))
    }

    pub fn notify(&self, method: &str, params: impl Serialize) -> Result<()> {
        self.write(message(method, params)?)
    }

    /// Answers a server [`Event::Request`].
    pub fn respond(&self, id: Value, result: impl Serialize) -> Result<()> {
        self.write(json!({ "id": id, "result": serde_json::to_value(result)? }))
    }

    fn write(&self, msg: Value) -> Result<()> {
        self.backend.lock().send(msg)
    }

    /// Sends `initialize` (opting into the experimental API, like the desktop app), then `initialized`.
    pub async fn initialize(&self, client_name: &str, version: &str) -> Result<InitializeResponse> {
        let mut initialized = self.initialized.lock().await;
        if let Some(response) = initialized.as_ref() {
            return Ok(response.clone());
        }
        let params = json!({
            "clientInfo": { "name": client_name, "title": null, "version": version },
            "capabilities": { "experimentalApi": true, "requestAttestation": false },
        });
        let response: InitializeResponse = self.request("initialize", params).await?;
        self.notify("initialized", ())?;
        *initialized = Some(response.clone());
        Ok(response)
    }

    /// `cwd: None` uses the server's working directory.
    pub async fn thread_start(&self, cwd: Option<&str>) -> Result<ThreadStartResponse> {
        self.request("thread/start", json!({ "cwd": cwd })).await
    }

    pub async fn turn_start(&self, thread_id: &str, text: &str) -> Result<TurnStartResponse> {
        let input = json!([{ "type": "text", "text": text, "text_elements": [] }]);
        let params = json!({ "threadId": thread_id, "input": input });
        self.request("turn/start", params).await
    }

    pub async fn turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<()> {
        let params = json!({ "threadId": thread_id, "turnId": turn_id });
        self.request::<_, Value>("turn/interrupt", params).await?;
        Ok(())
    }

    pub async fn thread_list(&self) -> Result<ThreadListResponse> {
        self.request("thread/list", json!({})).await
    }

    pub async fn thread_read(
        &self,
        thread_id: &str,
        include_turns: bool,
    ) -> Result<ThreadReadResponse> {
        let params = json!({ "threadId": thread_id, "includeTurns": include_turns });
        self.request("thread/read", params).await
    }

    pub async fn model_list(&self) -> Result<ModelListResponse> {
        self.request("model/list", json!({})).await
    }

    pub async fn config_read(&self) -> Result<ConfigReadResponse> {
        self.request("config/read", json!({})).await
    }

    pub async fn account_read(&self) -> Result<AccountReadResponse> {
        self.request("account/read", json!({})).await
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_piped(mut cmd: Command) -> Result<(Child, ChildStdout, ChildStdin)> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {:?}", cmd.get_program()))?;
    let stdin = child.stdin.take().context("no stdin")?;
    let stdout = child.stdout.take().context("no stdout")?;
    Ok((child, stdout, stdin))
}

/// `{"method", "params"}`, leaving `params` out when it serializes to `null`.
fn message(method: &str, params: impl Serialize) -> Result<Value> {
    let mut msg = json!({ "method": method });
    let params = serde_json::to_value(params)?;
    if !params.is_null() {
        msg["params"] = params;
    }
    Ok(msg)
}

#[derive(Deserialize)]
struct Incoming {
    id: Option<Value>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    result: Value,
    error: Option<RpcError>,
}

fn read_loop(reader: impl BufRead, sink: &Sink) {
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Incoming = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(err) => {
                eprintln!("bridge: skipping unparsable line ({err}): {line}");
                continue;
            }
        };
        sink.incoming(msg);
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn dropping_a_request_releases_its_pending_reply() {
        use std::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };
        let (input, producer) = std::io::pipe().unwrap();
        let (bridge, _) = Bridge::from_io(input, std::io::sink(), None);
        {
            let mut request = pin!(bridge.request::<_, Value>("process/spawn", json!({})));
            assert!(matches!(
                request
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            assert_eq!(bridge.pending.lock().as_ref().unwrap().len(), 1);
        }
        assert!(bridge.pending.lock().as_ref().unwrap().is_empty());
        drop(producer);
    }

    #[test]
    fn closing_one_window_preserves_other_event_subscribers() {
        let (input, mut producer) = std::io::pipe().unwrap();
        let (bridge, first) = Bridge::from_io(input, std::io::sink(), None);
        let second = bridge.subscribe_async();
        let observer = bridge.subscribe();
        writeln!(
            producer,
            "{{\"method\":\"thread/started\",\"params\":{{\"threadId\":\"one\"}}}}"
        )
        .unwrap();
        let event = first.recv_blocking().unwrap();
        assert_eq!(second.recv_blocking().unwrap(), event);
        assert_eq!(
            observer
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            event
        );
        drop(first);
        writeln!(producer, "{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"one\",\"turnId\":\"turn\"}}}}").unwrap();
        assert_eq!(
            second.recv_blocking().unwrap(),
            Event::Notification {
                method: "turn/completed".into(),
                params: json!({"threadId": "one", "turnId": "turn"}),
            }
        );
        drop(producer);
        assert!(second.recv_blocking().is_err());
        assert!(bridge.is_closed());
        assert!(bridge.subscribe_async().is_closed());
    }

    #[test]
    fn concurrent_windows_initialize_the_shared_server_once() {
        use std::{
            future::Future as _,
            pin::pin,
            sync::atomic::{AtomicUsize, Ordering},
            task::Poll,
        };
        let (input, mut response) = std::io::pipe().unwrap();
        let (requests, output) = std::io::pipe().unwrap();
        let (bridge, _) = Bridge::from_io(input, output, None);
        let (release, ready) = std::sync::mpsc::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let server = std::thread::spawn(move || {
            for line in BufReader::new(requests).lines() {
                let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                        if first {
                            ready.recv().unwrap();
                        }
                        let reply = if first {
                            json!({"id":request["id"],"result":{"userAgent":"shared","codexHome":"/fixture","platformOs":"macos"}})
                        } else {
                            json!({"id":request["id"],"error":{"code":-32600,"message":"Already initialized"}})
                        };
                        writeln!(response, "{reply}").unwrap();
                    }
                    "initialized" => break,
                    _ => panic!("unexpected initialization method"),
                }
            }
        });
        futures_lite::future::block_on(async {
            let mut first = pin!(bridge.initialize("primary", "1"));
            let mut second = pin!(bridge.initialize("popout", "1"));
            futures_lite::future::poll_fn(|cx| {
                assert!(first.as_mut().poll(cx).is_pending());
                assert!(second.as_mut().poll(cx).is_pending());
                release.send(()).unwrap();
                Poll::Ready(())
            })
            .await;
            let (first, second) = futures_lite::future::zip(first, second).await;
            assert_eq!(first.unwrap().user_agent, "shared");
            assert_eq!(second.unwrap().user_agent, "shared");
        });
        server.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
