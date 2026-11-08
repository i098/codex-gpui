use super::{Service, ToolCall};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

const MAX_FRAME: usize = 8 * 1024 * 1024;
type Pending = Arc<Mutex<HashMap<String, Arc<Cancel>>>>;

pub(super) struct Cancel {
    stopped: Arc<AtomicBool>,
    sender: async_channel::Sender<()>,
    receiver: async_channel::Receiver<()>,
}

impl Cancel {
    pub(super) fn new() -> Self {
        let (sender, receiver) = async_channel::bounded(1);
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.sender.try_send(());
    }

    pub(super) fn check(&self) -> Result<()> {
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "App tool call cancelled"
        );
        Ok(())
    }

    pub(super) async fn until<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        use std::{future::poll_fn, pin::pin, task::Poll};
        self.check()?;
        let mut future = pin!(future);
        let mut cancelled = pin!(self.receiver.recv());
        poll_fn(|cx| {
            if self.stopped.load(Ordering::Acquire) || cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(anyhow::anyhow!("App tool call cancelled")));
            }
            future.as_mut().poll(cx)
        })
        .await
    }
}

pub(super) struct Pipe {
    directory: tempfile::TempDir,
    stopped: Arc<AtomicBool>,
    clients: Arc<Mutex<HashMap<u64, UnixStream>>>,
    listener: Option<JoinHandle<()>>,
}

impl Pipe {
    pub(super) fn start(home: &Path, service: Arc<Service>) -> Result<Self> {
        std::fs::create_dir_all(home)?;
        let home = home.canonicalize()?;
        let root = home.join(".tmp");
        std::fs::create_dir_all(&root)?;
        ensure!(
            root.canonicalize()?.starts_with(&home),
            "App tools temporary root escapes CODEX_HOME"
        );
        let directory = tempfile::Builder::new()
            .prefix("gpui-app-tools-")
            .tempdir_in(root)?;
        crate::private::restrict(directory.path(), 0o700)?;
        let socket = directory.path().join("tools.sock");
        let listener = UnixListener::bind(&socket).context("Bind the native app tools socket")?;
        listener.set_nonblocking(true)?;
        crate::private::restrict(&socket, 0o600)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let (stop, sockets) = (stopped.clone(), clients.clone());
        let listener = thread::Builder::new()
            .name("app-tools-pipe".into())
            .spawn(move || {
                let mut next_client = 0u64;
                while !stop.load(Ordering::Acquire) {
                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            // ponytail: 20 ms accept polling; use readiness I/O for high connection rates.
                            thread::park_timeout(Duration::from_millis(20));
                            continue;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    };
                    if stream.set_nonblocking(false).is_err() {
                        continue;
                    }
                    let Ok(shutdown) = stream.try_clone() else {
                        continue;
                    };
                    next_client += 1;
                    let client = next_client;
                    {
                        let mut clients = sockets.lock().unwrap();
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        clients.insert(client, shutdown);
                    }
                    let worker_service = service.clone();
                    let worker_sockets = sockets.clone();
                    let worker = thread::Builder::new()
                        .name("app-tools-client".into())
                        .spawn(move || {
                            let _ = serve(stream, &worker_service);
                            worker_sockets.lock().unwrap().remove(&client);
                        });
                    if worker.is_err() {
                        sockets.lock().unwrap().remove(&client);
                    }
                }
            })?;
        Ok(Self {
            directory,
            stopped,
            clients,
            listener: Some(listener),
        })
    }

    pub(super) fn path(&self) -> std::path::PathBuf {
        self.directory.path().join("tools.sock")
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        for stream in self.clients.lock().unwrap().values() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(listener) = self.listener.take() {
            listener.thread().unpark();
            let _ = listener.join();
        }
    }
}

fn serve(stream: UnixStream, service: &Arc<Service>) -> Result<()> {
    let output = Arc::new(Mutex::new(stream.try_clone()?));
    let mut input = std::io::BufReader::new(stream);
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let result = (|| {
        loop {
            let mut request = match read_frame(&mut input)? {
                Some(request) => request,
                None => return Ok(()),
            };
            let id = request
                .get_mut("id")
                .map(Value::take)
                .filter(|id| id.is_string() || id.is_number());
            let Some(id) = id else {
                continue;
            };
            let key = id.to_string();
            if request["jsonrpc"] != "2.0" || !request["method"].is_string() {
                write_error(&output, id, -32602, "Invalid app tool request")?;
                continue;
            }
            match request["method"].as_str().unwrap() {
                "tools/list" => list_tools(&output, id, &request, service)?,
                "tools/cancel" => {
                    if let Some(cancel) = pending.lock().unwrap().remove(&key) {
                        cancel.stop();
                    }
                }
                "tools/call" => start_call(&output, &pending, service, id, key, &mut request)?,
                _ => write_error(&output, id, -32602, "Invalid app tool request")?,
            }
        }
    })();
    for cancel in pending.lock().unwrap().drain().map(|(_, cancel)| cancel) {
        cancel.stop();
    }
    result
}

fn list_tools(
    output: &Mutex<UnixStream>,
    id: Value,
    request: &Value,
    service: &Service,
) -> Result<()> {
    let params = request.get("params");
    if params.is_some_and(|params| {
        !params.is_object()
            || params
                .get("threadStartKind")
                .is_none_or(|kind| kind != "default" && kind != "all")
    }) {
        write_error(output, id, -32602, "Invalid app tool request")
    } else {
        write_frame(
            &mut *output.lock().unwrap(),
            &json!({"id": id, "jsonrpc": "2.0", "result": {"tools": service.catalog}}),
        )
    }
}

/// Runs a `tools/call` on its own thread; a newer call with the same `key` cancels it.
fn start_call(
    output: &Arc<Mutex<UnixStream>>,
    pending: &Pending,
    service: &Arc<Service>,
    id: Value,
    key: String,
    request: &mut Value,
) -> Result<()> {
    let call = serde_json::from_value::<ToolCall>(request["params"].take());
    let Ok(call) = call else {
        return write_error(output, id, -32602, "Invalid app tool request");
    };
    if !call.valid() {
        return write_error(output, id, -32602, "Invalid app tool request");
    }
    let cancel = Arc::new(Cancel::new());
    if let Some(old) = pending.lock().unwrap().insert(key.clone(), cancel.clone()) {
        old.stop();
    }
    let (service, pending, output) = (service.clone(), pending.clone(), output.clone());
    thread::Builder::new().name("app-tools-call".into()).spawn(move || {
        let result = pollster::block_on(service.call(call, &cancel));
        let mut calls = pending.lock().unwrap();
        if calls.get(&key).is_some_and(|current| Arc::ptr_eq(current, &cancel)) {
            calls.remove(&key);
            if cancel.check().is_ok() {
                let response = match result {
                    Ok(result) => json!({"id": id, "jsonrpc": "2.0", "result": result}),
                    Err(_) => json!({"id": id, "jsonrpc": "2.0", "error": {"code": -32000, "message": "Codex app tool request failed"}}),
                };
                let response_failed = write_frame(&mut *output.lock().unwrap(), &response).is_err();
                if response_failed
                    && write_error(&output, response["id"].clone(), -32000, "Codex app tool request failed").is_err()
                {
                    let _ = output.lock().unwrap().shutdown(std::net::Shutdown::Both);
                }
            }
        }
    })?;
    Ok(())
}

fn write_error(output: &Mutex<UnixStream>, id: Value, code: i64, message: &str) -> Result<()> {
    write_frame(
        &mut *output.lock().unwrap(),
        &json!({"id": id, "jsonrpc": "2.0", "error": {"code": code, "message": message}}),
    )
}

pub(super) fn read_frame(stream: &mut impl std::io::Read) -> Result<Option<Value>> {
    loop {
        let mut header = [0u8; 4];
        loop {
            match stream.read(&mut header[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        stream.read_exact(&mut header[1..])?;
        let size = u32::from_ne_bytes(header) as usize;
        ensure!(size <= MAX_FRAME, "App tools frame exceeds 8 MiB");
        let mut frame = std::io::Read::take(&mut *stream, size as u64);
        let parsed = serde_json::from_reader(&mut frame);
        std::io::copy(&mut frame, &mut std::io::sink())?;
        ensure!(
            frame.limit() == 0,
            "App tools connection closed inside a frame"
        );
        if let Ok(value) = parsed {
            return Ok(Some(value));
        }
    }
}

pub(super) fn write_frame(stream: &mut impl std::io::Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_FRAME, "App tools frame exceeds 8 MiB");
    stream.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}
