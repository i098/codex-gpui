//! The ACP backend: Agent Client Protocol v1 (<https://agentclientprotocol.com>) to an agent
//! such as `omp acp`, answered to the app as `codex app-server` would, so `Root` and
//! `ThreadView` handle it unchanged. An ACP session is a thread with the same id and each
//! `session/prompt` is a turn. Methods without an ACP counterpart fail with "not supported
//! by the ACP backend"; none gets a made-up success. A permission request keeps the agent's
//! request id and is answered only with the option the user picked.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{Backend, Event, Lines, RpcError, Sink};

/// The app's side; the reader thread shares the same state.
pub(crate) struct Acp(Arc<Mutex<State>>);

struct State {
    out: Lines,
    sink: Sink,
    /// `agentInfo.name` from `initialize`.
    agent: String,
    next_id: i64,
    /// Requests sent to the agent, by id: what their answer completes.
    calls: HashMap<i64, Call>,
    /// The sessions of this connection, oldest first.
    sessions: Vec<Session>,
    /// Unanswered `session/request_permission`s by request id (JSON text): session id, options.
    permissions: HashMap<String, (String, Vec<Value>)>,
}

enum Call {
    /// The app's `initialize` request id.
    Initialize(i64),
    /// The app's `thread/start` request id and the session's working directory.
    NewSession(i64, String),
    /// The session whose turn the prompt is.
    Prompt(String),
}

struct Session {
    /// The app-server `Thread`, `turns` included; the last turn runs while `started` is set.
    thread: Value,
    /// Unix ms the running turn started at.
    started: Option<i64>,
    /// The agent message being streamed: item id and text so far.
    message: Option<(String, String)>,
}

impl Acp {
    pub(crate) fn start(read: impl Read + Send + 'static, out: Lines, sink: Sink) -> Acp {
        let state = Arc::new(Mutex::new(State {
            out,
            sink,
            agent: "ACP agent".into(),
            next_id: 1,
            calls: HashMap::new(),
            sessions: Vec::new(),
            permissions: HashMap::new(),
        }));
        let reader = state.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(read).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let done = serde_json::from_str(&line)
                    .map_err(anyhow::Error::from)
                    .and_then(|msg| reader.lock().incoming(msg));
                if let Err(err) = done {
                    eprintln!("bridge: acp: {err:#}: {line}");
                }
            }
            reader.lock().closed();
        });
        Acp(state)
    }
}

impl Backend for Acp {
    fn send(&mut self, msg: &Value) -> Result<()> {
        let mut state = self.0.lock();
        match (msg["method"].as_str(), msg["id"].as_i64()) {
            (Some(method), Some(id)) => state.request(id, method, &msg["params"]),
            // `initialized`: ACP has no such notification.
            (Some(_), None) => Ok(()),
            (None, _) => state.answer(&msg["id"], &msg["result"]),
        }
    }
}

impl State {
    fn write(&mut self, mut msg: Value) -> Result<()> {
        msg["jsonrpc"] = "2.0".into();
        self.out.send(&msg)
    }

    fn call(&mut self, call: Call, method: &str, params: Value) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        self.calls.insert(id, call);
        self.write(json!({ "id": id, "method": method, "params": params }))
    }

    fn index(&self, thread_id: Option<&str>) -> Option<usize> {
        let id = thread_id?;
        self.sessions
            .iter()
            .position(|s| s.thread["id"].as_str() == Some(id))
    }

    /// App-server request `id` from the app.
    fn request(&mut self, id: i64, method: &str, params: &Value) -> Result<()> {
        let reply = match method {
            "initialize" => {
                let params = json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": false, "writeTextFile": false },
                        "terminal": false,
                    },
                    "clientInfo": params["clientInfo"],
                });
                return self.call(Call::Initialize(id), "initialize", params);
            }
            "thread/start" if params["ephemeral"] != true => {
                let cwd = match params["cwd"].as_str() {
                    Some(cwd) => cwd.to_owned(),
                    None => std::env::current_dir()?.to_string_lossy().into_owned(),
                };
                let params = json!({ "cwd": cwd, "mcpServers": [] });
                return self.call(Call::NewSession(id, cwd), "session/new", params);
            }
            // ACP has no account, model catalog or Codex config, and needs no OpenAI sign-in.
            "account/read" => Ok(json!({ "account": null, "requiresOpenaiAuth": false })),
            "model/list" => Ok(json!({ "data": [], "nextCursor": null })),
            "config/read" => Ok(json!({ "config": {} })),
            "thread/list" => {
                let data: Vec<&Value> = self.sessions.iter().rev().map(|s| &s.thread).collect();
                Ok(json!({ "data": data, "nextCursor": null }))
            }
            "thread/read" | "thread/resume" => match self.index(params["threadId"].as_str()) {
                Some(i) => Ok(json!({ "thread": self.sessions[i].thread })),
                None => Err(error("Unknown thread for this ACP connection")),
            },
            "turn/start" if params["outputSchema"].is_null() => self.turn_start(params)?,
            "turn/interrupt" => self.interrupt(params)?,
            _ => Err(unsupported(method)),
        };
        self.sink.reply(id, reply);
        Ok(())
    }

    /// `session/prompt` with the text input; the turn starts at once and ends with its answer.
    fn turn_start(&mut self, params: &Value) -> Result<Result<Value, RpcError>> {
        let mut text = String::new();
        for input in params["input"].as_array().into_iter().flatten() {
            match input["text"].as_str() {
                Some(part) if input["type"] == "text" => text.push_str(part),
                _ => return Ok(Err(unsupported("Attaching files or images"))),
            }
        }
        let Some(i) = self.index(params["threadId"].as_str()) else {
            return Ok(Err(error("Unknown thread for this ACP connection")));
        };
        if self.sessions[i].started.is_some() {
            return Ok(Err(error("A turn is already running in this ACP session")));
        }
        let thread = &self.sessions[i].thread;
        let sid = thread["id"].as_str().unwrap_or_default().to_owned();
        let turn_id = format!(
            "{sid}-turn-{}",
            thread["turns"].as_array().map_or(0, Vec::len) + 1
        );
        let prompt = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] });
        self.call(Call::Prompt(sid.clone()), "session/prompt", prompt)?;
        let now = now_ms();
        let turn = json!({
            "id": turn_id,
            "items": [{
                "type": "userMessage",
                "id": format!("{turn_id}-user"),
                "content": [{ "type": "text", "text": text }],
            }],
            "status": "inProgress",
            "error": null,
            "startedAt": now / 1000,
            "completedAt": null,
            "durationMs": null,
        });
        let session = &mut self.sessions[i];
        let thread = &mut session.thread;
        if thread["preview"] == "" {
            thread["preview"] = text.into();
        }
        thread["updatedAt"] = (now / 1000).into();
        thread["status"] = json!({ "type": "active" });
        if let Some(turns) = thread["turns"].as_array_mut() {
            turns.push(turn.clone());
        }
        session.started = Some(now);
        session.message = None;
        let status = json!({ "threadId": sid, "status": { "type": "active" } });
        self.sink.notify("thread/status/changed", status);
        self.sink
            .notify("turn/started", json!({ "threadId": sid, "turn": turn }));
        Ok(Ok(json!({ "turn": turn })))
    }

    /// `session/cancel`; the turn ends when the agent answers the prompt `cancelled`.
    fn interrupt(&mut self, params: &Value) -> Result<Result<Value, RpcError>> {
        let running = self
            .index(params["threadId"].as_str())
            .map(|i| &self.sessions[i])
            .filter(|s| s.started.is_some() && turn_id(&s.thread) == params["turnId"]);
        let Some(sid) = running
            .and_then(|s| s.thread["id"].as_str())
            .map(str::to_owned)
        else {
            return Ok(Err(error("No such turn is running")));
        };
        self.write(json!({ "method": "session/cancel", "params": { "sessionId": sid } }))?;
        // ACP: the client answers the session's open permission requests `cancelled`.
        let mut open = Vec::new();
        self.permissions.retain(|id, (session, _)| {
            let keep = *session != sid;
            if !keep {
                open.push(id.clone());
            }
            keep
        });
        for id in open {
            let id: Value = serde_json::from_str(&id)?;
            self.write(json!({ "id": id, "result": { "outcome": { "outcome": "cancelled" } } }))?;
        }
        Ok(Ok(json!({})))
    }

    /// The app's decision on a permission request it got as
    /// `item/commandExecution/requestApproval`.
    fn answer(&mut self, id: &Value, result: &Value) -> Result<()> {
        // Already answered `cancelled`: the turn was stopped.
        let Some((_, options)) = self.permissions.remove(&id.to_string()) else {
            return Ok(());
        };
        let kind = match result["decision"].as_str() {
            Some("accept") => "allow_once",
            Some("acceptForSession") => "allow_always",
            Some("decline") => "reject_once",
            _ => "",
        };
        // No other option is picked in place of one the agent did not offer.
        let outcome = match options.iter().find(|o| o["kind"] == kind) {
            Some(option) => json!({ "outcome": "selected", "optionId": option["optionId"] }),
            None => json!({ "outcome": "cancelled" }),
        };
        self.write(json!({ "id": id, "result": { "outcome": outcome } }))
    }

    /// One message from the agent.
    fn incoming(&mut self, msg: Value) -> Result<()> {
        match (msg["method"].as_str(), msg.get("id")) {
            (Some(method), Some(id)) => self.agent_request(id.clone(), method, &msg["params"]),
            (Some("session/update"), None) => {
                self.update(&msg["params"]);
                Ok(())
            }
            (Some(_), None) | (None, None) => Ok(()),
            (None, Some(id)) => {
                if let Some(call) = id.as_i64().and_then(|id| self.calls.remove(&id)) {
                    let result = match msg.get("error") {
                        Some(e) => Err(serde_json::from_value(e.clone())
                            .unwrap_or_else(|_| error(e.to_string()))),
                        None => Ok(msg["result"].clone()),
                    };
                    self.answered(call, result);
                }
                Ok(())
            }
        }
    }

    fn answered(&mut self, call: Call, result: Result<Value, RpcError>) {
        match call {
            Call::Initialize(id) => {
                let reply = result.and_then(|r| {
                    if r["protocolVersion"] != 1 {
                        return Err(error(format!(
                            "ACP protocol version {} is not supported",
                            r["protocolVersion"]
                        )));
                    }
                    let info = &r["agentInfo"];
                    if let Some(name) = info["name"].as_str() {
                        self.agent = name.to_owned();
                    }
                    let version = info["version"].as_str().unwrap_or_default();
                    Ok(json!({
                        "userAgent": format!("{} {version}", self.agent).trim_end(),
                        "codexHome": "",
                        "platformOs": std::env::consts::OS,
                    }))
                });
                self.sink.reply(id, reply);
            }
            Call::NewSession(id, cwd) => {
                let reply = result.and_then(|r| {
                    let sid = r["sessionId"]
                        .as_str()
                        .ok_or_else(|| error("session/new gave no sessionId"))?;
                    let model = r["configOptions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|o| o["category"] == "model")
                        .map_or(&r["models"]["currentModelId"], |o| &o["currentValue"]);
                    let now = now_ms() / 1000;
                    let thread = json!({
                        "id": sid, "preview": "", "name": null, "model": model,
                        "modelProvider": self.agent, "cwd": cwd, "createdAt": now,
                        "updatedAt": now, "section": null, "status": { "type": "idle" },
                        "gitInfo": null, "turns": [],
                    });
                    let model = model.as_str().unwrap_or_default();
                    let reply = json!({
                        "thread": thread, "model": model, "cwd": cwd, "reasoningEffort": null,
                    });
                    self.sink
                        .notify("thread/started", json!({ "thread": thread }));
                    self.sessions.push(Session {
                        thread,
                        started: None,
                        message: None,
                    });
                    Ok(reply)
                });
                self.sink.reply(id, reply);
            }
            Call::Prompt(sid) => {
                let (status, message) = match result {
                    Ok(r) => match r["stopReason"].as_str() {
                        Some("end_turn") => ("completed", None),
                        Some("cancelled") => ("interrupted", None),
                        reason => {
                            let reason = reason.unwrap_or("no stop reason");
                            ("failed", Some(format!("The agent stopped: {reason}")))
                        }
                    },
                    Err(e) => ("failed", Some(e.message)),
                };
                self.end_turn(&sid, status, message);
            }
        }
    }

    fn end_turn(&mut self, sid: &str, status: &str, message: Option<String>) {
        let Some(i) = self.index(Some(sid)) else {
            return;
        };
        let session = &mut self.sessions[i];
        let Some(started) = session.started.take() else {
            return;
        };
        close_message(session, &self.sink);
        let now = now_ms();
        session.thread["status"] = json!({ "type": "idle" });
        session.thread["updatedAt"] = (now / 1000).into();
        let Some(turn) = last_turn(&mut session.thread) else {
            return;
        };
        turn["status"] = status.into();
        turn["error"] = message.map_or(Value::Null, |message| {
            json!({ "message": message, "codexErrorInfo": null, "additionalDetails": null })
        });
        turn["completedAt"] = (now / 1000).into();
        turn["durationMs"] = (now - started).into();
        let turn = turn.clone();
        self.sink
            .notify("turn/completed", json!({ "threadId": sid, "turn": turn }));
        let status = json!({ "threadId": sid, "status": { "type": "idle" } });
        self.sink.notify("thread/status/changed", status);
    }

    /// `session/update` of a running turn: streamed text and tool calls.
    fn update(&mut self, params: &Value) {
        let Some(i) = self.index(params["sessionId"].as_str()) else {
            return;
        };
        let (session, sink) = (&mut self.sessions[i], &self.sink);
        if session.started.is_none() {
            return;
        }
        let sid = session.thread["id"].clone();
        let turn = turn_id(&session.thread);
        let update = &params["update"];
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                let Some(delta) = update["content"]["text"].as_str() else {
                    return;
                };
                let count = last_turn(&mut session.thread).map_or(0, |t| items(t).len());
                let (id, text) = session.message.get_or_insert_with(|| {
                    let id = format!("{}-item-{count}", turn.as_str().unwrap_or_default());
                    let item = json!({ "type": "agentMessage", "id": id, "text": "" });
                    let params = json!({ "threadId": sid, "turnId": turn, "item": item });
                    sink.notify("item/started", params);
                    (id, String::new())
                });
                text.push_str(delta);
                let params =
                    json!({ "threadId": sid, "turnId": turn, "itemId": id, "delta": delta });
                sink.notify("item/agentMessage/delta", params);
            }
            Some("tool_call") => {
                close_message(session, sink);
                let item = json!({
                    "type": "mcpToolCall", "id": update["toolCallId"], "server": self.agent,
                    "tool": update["title"].as_str().unwrap_or("tool"), "status": "inProgress",
                });
                if let Some(t) = last_turn(&mut session.thread) {
                    items(t).push(item.clone());
                }
                let params = json!({ "threadId": sid, "turnId": turn, "item": item });
                sink.notify("item/started", params);
            }
            Some("tool_call_update") => {
                let status = match update["status"].as_str() {
                    Some(status @ ("completed" | "failed")) => status,
                    _ => return,
                };
                let Some(item) = last_turn(&mut session.thread).and_then(|t| {
                    items(t)
                        .iter_mut()
                        .find(|i| i["type"] == "mcpToolCall" && i["id"] == update["toolCallId"])
                }) else {
                    return;
                };
                item["status"] = status.into();
                if let Some(title) = update["title"].as_str() {
                    item["tool"] = title.into();
                }
                let params = json!({ "threadId": sid, "turnId": turn, "item": item });
                sink.notify("item/completed", params);
            }
            _ => {}
        }
    }

    /// A request from the agent: permission prompts go to the app under the agent's id.
    fn agent_request(&mut self, id: Value, method: &str, params: &Value) -> Result<()> {
        let running = self
            .index(params["sessionId"].as_str())
            .filter(|&i| self.sessions[i].started.is_some());
        match (method, running) {
            ("session/request_permission", Some(i)) => {
                let thread = &self.sessions[i].thread;
                let tool = &params["toolCall"];
                let request = json!({
                    "threadId": thread["id"], "turnId": turn_id(thread),
                    "itemId": tool["toolCallId"], "command": tool["title"], "cwd": thread["cwd"],
                    "reason": format!("{} asks over ACP", self.agent),
                });
                let sid = thread["id"].as_str().unwrap_or_default().to_owned();
                let options = params["options"].as_array().cloned().unwrap_or_default();
                self.permissions.insert(id.to_string(), (sid, options));
                self.sink.event(Event::Request {
                    id,
                    method: "item/commandExecution/requestApproval".into(),
                    params: request,
                });
                Ok(())
            }
            // Not a running turn of this connection: refused, never approved.
            ("session/request_permission", None) => {
                self.write(json!({ "id": id, "result": { "outcome": { "outcome": "cancelled" } } }))
            }
            // `initialize` offered no file system or terminal.
            _ => self.write(json!({ "id": id, "error": {
                "code": -32601, "message": format!("{method} is not supported by this client"),
            } })),
        }
    }

    /// The agent exited: requests fail and running turns end `failed`.
    fn closed(&mut self) {
        self.sink.close();
        self.permissions.clear();
        let running: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.started.is_some())
            .filter_map(|s| s.thread["id"].as_str().map(str::to_owned))
            .collect();
        for sid in running {
            self.end_turn(&sid, "failed", Some("The ACP agent exited".into()));
        }
    }
}

/// Ends the streamed agent message, if any: `item/completed` with its whole text.
fn close_message(session: &mut Session, sink: &Sink) {
    let Some((id, text)) = session.message.take() else {
        return;
    };
    let sid = session.thread["id"].clone();
    let Some(turn) = last_turn(&mut session.thread) else {
        return;
    };
    let item = json!({ "type": "agentMessage", "id": id, "text": text });
    items(turn).push(item.clone());
    let params = json!({ "threadId": sid, "turnId": turn["id"], "item": item });
    sink.notify("item/completed", params);
}

fn last_turn(thread: &mut Value) -> Option<&mut Value> {
    thread["turns"].as_array_mut()?.last_mut()
}

fn turn_id(thread: &Value) -> Value {
    thread["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .map_or(Value::Null, |turn| turn["id"].clone())
}

/// A turn's `items`, which this module always creates as an array.
fn items(turn: &mut Value) -> &mut Vec<Value> {
    if !turn["items"].is_array() {
        turn["items"] = json!([]);
    }
    turn["items"].as_array_mut().expect("items array")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn error(message: impl Into<String>) -> RpcError {
    RpcError {
        code: -32600,
        message: message.into(),
        data: Value::Null,
    }
}

fn unsupported(what: &str) -> RpcError {
    RpcError {
        code: -32601,
        message: format!("{what} is not supported by the ACP backend"),
        data: Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bridge;
    use async_channel::Receiver;
    use futures_lite::future::block_on;
    use std::io::Write;

    /// A [`Bridge`] on a scripted agent: `agent` gets each message the bridge sends and
    /// returns the lines to answer with. The second receiver yields what the agent got.
    fn connect(
        mut agent: impl FnMut(&Value) -> Vec<Value> + Send + 'static,
    ) -> (Bridge, Receiver<Event>, Receiver<Value>) {
        let (client_read, mut agent_write) = std::io::pipe().unwrap();
        let (agent_read, client_write) = std::io::pipe().unwrap();
        let (got_tx, got) = async_channel::unbounded();
        std::thread::spawn(move || {
            for line in BufReader::new(agent_read).lines() {
                let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
                assert_eq!(msg["jsonrpc"], "2.0");
                for reply in agent(&msg) {
                    writeln!(agent_write, "{reply}").unwrap();
                }
                let _ = got_tx.try_send(msg);
            }
        });
        let (bridge, events) = Bridge::acp_io(client_read, client_write, None);
        (bridge, events, got)
    }

    fn result(msg: &Value, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result })
    }

    fn setup(msg: &Value) -> Option<Value> {
        match msg["method"].as_str()? {
            "initialize" => Some(result(
                msg,
                json!({ "protocolVersion": 1, "agentInfo": { "name": "fake", "version": "1" } }),
            )),
            "session/new" => Some(result(msg, json!({ "sessionId": "s1" }))),
            _ => None,
        }
    }

    fn chunk(text: &str) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1",
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } } })
    }

    fn permission(id: &str) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": {
            "sessionId": "s1", "toolCall": { "toolCallId": "t1", "title": "rm -rf build" },
            "options": [
                { "optionId": "yes", "name": "Allow", "kind": "allow_once" },
                { "optionId": "no", "name": "Reject", "kind": "reject_once" },
            ] } })
    }

    fn start(bridge: &Bridge) -> String {
        block_on(bridge.initialize("test", "0")).unwrap();
        let started: Value =
            block_on(bridge.request("thread/start", json!({ "cwd": "/tmp" }))).unwrap();
        let input = json!([{ "type": "text", "text": "hi", "text_elements": [] }]);
        let _: Value =
            block_on(bridge.request("turn/start", json!({ "threadId": "s1", "input": input })))
                .unwrap();
        started["thread"]["id"].as_str().unwrap().to_owned()
    }

    /// Notifications up to and including `method`.
    fn until(events: &Receiver<Event>, method: &str) -> Vec<(String, Value)> {
        let mut seen = Vec::new();
        loop {
            let Event::Notification { method: m, params } = events.recv_blocking().unwrap() else {
                panic!("unexpected request");
            };
            let last = m == method;
            seen.push((m, params));
            if last {
                return seen;
            }
        }
    }

    fn request(events: &Receiver<Event>) -> (Value, Value) {
        loop {
            if let Event::Request { id, method, params } = events.recv_blocking().unwrap() {
                assert_eq!(method, "item/commandExecution/requestApproval");
                return (id, params);
            }
        }
    }

    #[test]
    fn a_prompt_streams_into_one_completed_turn() {
        let (bridge, events, _) = connect(|msg| match msg["method"].as_str() {
            Some("session/prompt") => vec![
                chunk("Hel"),
                chunk("lo"),
                result(msg, json!({ "stopReason": "end_turn" })),
            ],
            _ => setup(msg).into_iter().collect(),
        });
        assert_eq!(start(&bridge), "s1");
        let seen = until(&events, "turn/completed");
        let deltas: String = seen
            .iter()
            .filter(|(m, _)| m == "item/agentMessage/delta")
            .map(|(_, p)| p["delta"].as_str().unwrap())
            .collect();
        assert_eq!(deltas, "Hello");
        let (_, completed) = seen.last().unwrap();
        assert_eq!(completed["threadId"], "s1");
        assert_eq!(completed["turn"]["status"], "completed");
        let read: Value =
            block_on(bridge.request("thread/read", json!({ "threadId": "s1" }))).unwrap();
        let items = &read["thread"]["turns"][0]["items"];
        assert_eq!(items[0]["content"][0]["text"], "hi");
        assert_eq!(items[1]["text"], "Hello");
        let archive = block_on(bridge.request::<_, Value>("thread/archive", json!({})));
        let err = archive.unwrap_err();
        assert_eq!(err.downcast_ref::<RpcError>().unwrap().code, -32601);
    }

    #[test]
    fn permissions_keep_their_id_and_are_never_approved_unasked() {
        let mut prompt = Value::Null;
        let (bridge, events, got) =
            connect(
                move |msg| match (msg["method"].as_str(), msg["id"].as_str()) {
                    (Some("session/prompt"), _) => {
                        prompt = msg["id"].clone();
                        vec![permission("p1")]
                    }
                    (None, Some("p1")) => vec![permission("p2")],
                    (None, Some("p2")) => vec![permission("p3")],
                    (Some("session/cancel"), _) => vec![result(
                        &json!({ "id": prompt }),
                        json!({ "stopReason": "cancelled" }),
                    )],
                    _ => setup(msg).into_iter().collect(),
                },
            );
        start(&bridge);
        let answer = |id: &str| loop {
            let msg = got.recv_blocking().unwrap();
            if msg["id"] == id && msg.get("method").is_none() {
                return msg["result"]["outcome"].clone();
            }
        };
        // The agent did not offer `allow_always`: no other option is picked for it.
        let (id, params) = request(&events);
        assert_eq!(
            (id.as_str(), params["command"].as_str()),
            (Some("p1"), Some("rm -rf build"))
        );
        bridge
            .respond(id, json!({ "decision": "acceptForSession" }))
            .unwrap();
        assert_eq!(answer("p1"), json!({ "outcome": "cancelled" }));
        let (id, _) = request(&events);
        bridge
            .respond(id, json!({ "decision": "decline" }))
            .unwrap();
        assert_eq!(
            answer("p2"),
            json!({ "outcome": "selected", "optionId": "no" })
        );
        // Stop answers the open request `cancelled`; the turn ends interrupted.
        let (id, params) = request(&events);
        assert_eq!(id, "p3");
        let turn = params["turnId"].as_str().unwrap();
        block_on(bridge.turn_interrupt("s1", turn)).unwrap();
        assert_eq!(answer("p3"), json!({ "outcome": "cancelled" }));
        let seen = until(&events, "turn/completed");
        assert_eq!(seen.last().unwrap().1["turn"]["status"], "interrupted");
        // A late click on the stopped prompt sends nothing: the next message the agent gets
        // is the next request.
        bridge
            .respond(json!("p3"), json!({ "decision": "accept" }))
            .unwrap();
        let _: Value = block_on(bridge.request("thread/start", json!({ "cwd": "/tmp" }))).unwrap();
        assert_eq!(got.recv_blocking().unwrap()["method"], "session/new");
    }
}
