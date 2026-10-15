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

mod replay;
pub mod types;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

pub struct Bridge {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Pending,
    next_id: AtomicI64,
    child: Option<Child>,
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

impl Bridge {
    /// Spawns the app-server with piped stdin/stdout; stderr is left as configured on `cmd`.
    pub fn spawn(mut cmd: Command) -> Result<(Bridge, Receiver<Event>)> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {:?}", cmd.get_program()))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        Ok(Self::from_io(stdout, stdin, Some(child)))
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
        let pending: Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let (events_tx, events) = async_channel::unbounded();
        let reader_pending = pending.clone();
        std::thread::spawn(move || read_loop(BufReader::new(read), &reader_pending, &events_tx));
        let bridge = Bridge {
            writer: Mutex::new(Box::new(write)),
            pending,
            next_id: AtomicI64::new(1),
            child,
        };
        (bridge, events)
    }

    pub async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = message(method, params)?;
        msg["id"] = id.into();
        let (tx, rx) = async_channel::bounded(1);
        self.pending
            .lock()
            .as_mut()
            .context("app-server closed")?
            .insert(id, tx);
        if let Err(err) = self.write(&msg) {
            if let Some(map) = self.pending.lock().as_mut() {
                map.remove(&id);
            }
            return Err(err);
        }
        let result = rx
            .recv()
            .await
            .map_err(|_| anyhow!("app-server closed before answering {method}"))??;
        serde_json::from_value(result).with_context(|| format!("decoding {method} response"))
    }

    pub fn notify(&self, method: &str, params: impl Serialize) -> Result<()> {
        self.write(&message(method, params)?)
    }

    /// Answers a server [`Event::Request`].
    pub fn respond(&self, id: Value, result: impl Serialize) -> Result<()> {
        self.write(&json!({ "id": id, "result": serde_json::to_value(result)? }))
    }

    fn write(&self, msg: &Value) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut writer = self.writer.lock();
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    /// Sends `initialize` (opting into the experimental API, like the desktop app), then `initialized`.
    pub async fn initialize(&self, client_name: &str, version: &str) -> Result<InitializeResponse> {
        let params = json!({
            "clientInfo": { "name": client_name, "title": null, "version": version },
            "capabilities": { "experimentalApi": true, "requestAttestation": false },
        });
        let response = self.request("initialize", params).await?;
        self.notify("initialized", ())?;
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

fn read_loop(reader: impl BufRead, pending: &Pending, events: &Sender<Event>) {
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
        let event = match (msg.method, msg.id) {
            (Some(method), Some(id)) => Event::Request {
                id,
                method,
                params: msg.params,
            },
            (Some(method), None) => Event::Notification {
                method,
                params: msg.params,
            },
            (None, Some(id)) => {
                let waiter = id
                    .as_i64()
                    .and_then(|id| pending.lock().as_mut().and_then(|map| map.remove(&id)));
                if let Some(waiter) = waiter {
                    let _ = waiter.try_send(msg.error.map_or(Ok(msg.result), Err));
                }
                continue;
            }
            (None, None) => continue,
        };
        let _ = events.try_send(event);
    }
    // Dropping the senders wakes every waiter with "closed".
    pending.lock().take();
}
