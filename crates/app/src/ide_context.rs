//! `/ide-context`: the connected IDE's selection and open files, fetched over the desktop IPC
//! router and put in front of the prompt, as the original app does.
//!
//! The router is a Unix socket; frames are a `u32` little-endian length and a JSON message. The
//! IDE extension answers the `ide-context` request; with no router or no IDE the turn goes out
//! unchanged.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const MAX_FRAME: usize = 256 * 1024 * 1024;
const CLIENT_TYPE: &str = "codex_gpui";
const TIMEOUT: Duration = Duration::from_secs(3);
/// The `sourceClientId` the router expects before `initialize` has assigned one.
const UNINITIALIZED_CLIENT: &str = "initializing-client";

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdeContext {
    active_file: Option<ActiveFile>,
    #[serde(default)]
    open_tabs: Vec<Tab>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ActiveFile {
    path: String,
    active_selection_content: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Tab {
    label: String,
    path: String,
}

/// `text` led by the IDE's active file, selection and open tabs; unchanged when there are none.
pub(crate) fn prompt(text: String, context: &IdeContext) -> String {
    let mut body = String::new();
    if let Some(file) = &context.active_file {
        body.push_str(&format!("\n## Active file: {}\n", file.path));
        if let Some(selection) = file
            .active_selection_content
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            body.push_str(&format!("\n## Active selection of the file:\n{selection}"));
        }
    }
    if !context.open_tabs.is_empty() {
        body.push_str("\n## Open tabs:\n");
        for tab in &context.open_tabs {
            body.push_str(&format!("- {}: {}\n", tab.label, tab.path));
        }
    }
    if body.is_empty() {
        return text;
    }
    format!("# Context from my IDE setup:\n{body}\n## My request:\n{text}\n")
}

fn frame(message: &Value) -> Vec<u8> {
    let json = message.to_string();
    let mut bytes = Vec::with_capacity(4 + json.len());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(json.as_bytes());
    bytes
}

#[cfg(unix)]
mod socket {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    /// The router's socket: the temp-dir path the IDE extensions use.
    fn candidates() -> Vec<PathBuf> {
        use std::os::unix::fs::MetadataExt as _;
        let uid = std::env::var_os("HOME")
            .and_then(|home| std::fs::metadata(home).ok())
            .map_or(0, |home| home.uid());
        vec![
            std::env::temp_dir()
                .join("codex-ipc")
                .join(format!("ipc-{uid}.sock")),
        ]
    }

    fn read_frame(stream: &mut UnixStream) -> Result<Value> {
        let mut length = [0; 4];
        stream.read_exact(&mut length)?;
        let length = u32::from_le_bytes(length) as usize;
        if length == 0 || length > MAX_FRAME {
            bail!("invalid IPC frame length {length}");
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// Sends `request` and reads frames until the response with its id; broadcasts are skipped.
    fn call(stream: &mut UnixStream, request: &Value) -> Result<Value> {
        stream.write_all(&frame(request))?;
        loop {
            let message = read_frame(stream)?;
            if message["type"] == "response" && message["requestId"] == request["requestId"] {
                if message["resultType"] == "error" {
                    bail!("{}", message["error"].as_str().unwrap_or("ipc error"));
                }
                return Ok(message["result"].clone());
            }
        }
    }

    pub(super) fn fetch_at(path: &Path, workspace_root: &str) -> Result<IdeContext> {
        let mut stream = UnixStream::connect(path).context("connect to the IPC router")?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let id = || crate::uuid_v4();
        let initialized = call(
            &mut stream,
            &json!({
                "type": "request", "requestId": id(), "sourceClientId": UNINITIALIZED_CLIENT,
                "version": 0, "method": "initialize", "params": { "clientType": CLIENT_TYPE },
            }),
        )?;
        let client_id = initialized["clientId"].as_str().context("no clientId")?;
        let result = call(
            &mut stream,
            &json!({
                "type": "request", "requestId": id(), "sourceClientId": client_id,
                "version": 0, "method": "ide-context", "params": { "workspaceRoot": workspace_root },
                "timeoutMs": TIMEOUT.as_millis() as u64,
            }),
        )?;
        Ok(serde_json::from_value(result["ideContext"].clone())?)
    }

    pub(super) fn fetch(workspace_root: &str) -> Option<IdeContext> {
        candidates()
            .iter()
            .find_map(|path| fetch_at(path, workspace_root).ok())
    }
}

/// The connected IDE's context for `workspace_root`; `None` with no IDE, router or answer.
/// Blocks up to a few seconds: run it off the UI thread.
pub(crate) fn fetch(workspace_root: &str) -> Option<IdeContext> {
    #[cfg(unix)]
    return socket::fetch(workspace_root);
    #[cfg(not(unix))]
    {
        let _ = workspace_root;
        None
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    fn read(stream: &mut impl Read) -> Value {
        let mut length = [0; 4];
        stream.read_exact(&mut length).unwrap();
        let mut body = vec![0; u32::from_le_bytes(length) as usize];
        stream.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn context() -> IdeContext {
        serde_json::from_value(json!({
            "activeFile": { "path": "/w/a.rs", "activeSelectionContent": "let x = 1;" },
            "openTabs": [{ "label": "a.rs", "path": "/w/a.rs" }, { "label": "b.rs", "path": "/w/b.rs" }],
        }))
        .unwrap()
    }

    #[test]
    fn prompt_lists_file_selection_and_tabs_before_request() {
        assert_eq!(
            prompt("fix it".into(), &context()),
            "# Context from my IDE setup:\n\n## Active file: /w/a.rs\n\n## Active selection of \
             the file:\nlet x = 1;\n## Open tabs:\n- a.rs: /w/a.rs\n- b.rs: /w/b.rs\n\n## My \
             request:\nfix it\n"
        );
    }

    #[test]
    fn empty_context_leaves_prompt_unchanged() {
        assert_eq!(prompt("hi".into(), &IdeContext::default()), "hi");
    }

    #[test]
    fn fetch_initializes_then_requests_context_and_skips_broadcasts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let init = read(&mut stream);
            assert_eq!(init["method"], "initialize");
            assert_eq!(init["params"]["clientType"], CLIENT_TYPE);
            let reply = |stream: &mut std::os::unix::net::UnixStream, value: Value| {
                stream.write_all(&frame(&value)).unwrap();
            };
            reply(
                &mut stream,
                json!({ "type": "broadcast", "method": "noise", "params": {} }),
            );
            reply(
                &mut stream,
                json!({
                    "type": "response", "requestId": init["requestId"],
                    "resultType": "success", "result": { "clientId": "c-7" },
                }),
            );
            let request = read(&mut stream);
            assert_eq!(request["method"], "ide-context");
            assert_eq!(request["sourceClientId"], "c-7");
            assert_eq!(request["params"]["workspaceRoot"], "/w");
            reply(
                &mut stream,
                json!({
                    "type": "response", "requestId": request["requestId"], "resultType": "success",
                    "result": { "ideContext": {
                        "activeFile": { "path": "/w/a.rs", "activeSelectionContent": "let x = 1;" },
                        "openTabs": [{ "label": "a.rs", "path": "/w/a.rs" }, { "label": "b.rs", "path": "/w/b.rs" }],
                    } },
                }),
            );
        });
        assert_eq!(socket::fetch_at(&path, "/w").unwrap(), context());
        server.join().unwrap();
    }

    #[test]
    fn router_error_response_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let init = read(&mut stream);
            stream
                .write_all(&frame(&json!({
                    "type": "response", "requestId": init["requestId"],
                    "resultType": "error", "error": "no-client-found",
                })))
                .unwrap();
        });
        let error = socket::fetch_at(&path, "/w").unwrap_err();
        assert_eq!(error.to_string(), "no-client-found");
        server.join().unwrap();
    }

    #[test]
    fn missing_router_gives_none() {
        assert!(socket::fetch_at(std::path::Path::new("/nonexistent/ipc.sock"), "/w").is_err());
    }
}
