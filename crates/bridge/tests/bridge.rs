use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use bridge::types::*;
use bridge::{Bridge, Event, RpcError};
use futures_lite::future::block_on;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn check<T: DeserializeOwned>(what: &str, value: &Value) {
    if let Err(err) = serde_json::from_value::<T>(value.clone()) {
        panic!("{what} does not decode: {err}");
    }
}

fn check_response(method: &str, result: &Value) {
    match method {
        "initialize" => check::<InitializeResponse>(method, result),
        "thread/start" => check::<ThreadStartResponse>(method, result),
        "turn/start" => check::<TurnStartResponse>(method, result),
        "thread/list" => check::<ThreadListResponse>(method, result),
        "thread/read" => check::<ThreadReadResponse>(method, result),
        "model/list" => check::<ModelListResponse>(method, result),
        "config/read" => check::<ConfigReadResponse>(method, result),
        "account/read" => check::<AccountReadResponse>(method, result),
        _ => {}
    }
}

fn check_event(event: &Event) {
    let (method, params) = match event {
        Event::Notification { method, params } | Event::Request { method, params, .. } => {
            (method.as_str(), params)
        }
    };
    match method {
        "thread/started" => check::<ThreadStartedNotification>(method, params),
        "turn/started" | "turn/completed" => check::<TurnNotification>(method, params),
        "item/started" | "item/completed" => check::<ItemNotification>(method, params),
        "item/agentMessage/delta" => check::<AgentMessageDeltaNotification>(method, params),
        "turn/diff/updated" => check::<TurnDiffUpdatedNotification>(method, params),
        "error" => check::<ErrorNotification>(method, params),
        "item/commandExecution/requestApproval" => check::<CommandApprovalRequest>(method, params),
        "item/fileChange/requestApproval" => check::<FileChangeApprovalRequest>(method, params),
        _ => {}
    }
}

/// Plays every recorded client message against `Bridge::replay`, asserts each request gets its
/// recorded answer and that every recorded server message comes back; returns the events.
fn replay_all(path: &Path) -> Vec<Event> {
    let lines: Vec<Value> = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let (bridge, events) = Bridge::replay(path).unwrap();
    let events = block_on(async {
        for msg in lines
            .iter()
            .filter(|l| l["dir"] == "send")
            .map(|l| &l["msg"])
        {
            let params = msg.get("params").cloned().unwrap_or_default();
            match (msg["method"].as_str(), msg.get("id")) {
                (Some(method), Some(id)) => {
                    let got = bridge.request::<_, Value>(method, params).await;
                    let expected = lines
                        .iter()
                        .filter(|l| l["dir"] == "recv")
                        .map(|l| &l["msg"])
                        .find(|m| m.get("method").is_none() && m["id"] == *id);
                    match expected.map(|m| m.get("error")) {
                        Some(None) => {
                            let got = got.unwrap();
                            assert_eq!(got, expected.unwrap()["result"], "{method}");
                            check_response(method, &got);
                        }
                        Some(Some(err)) => {
                            let got = got.unwrap_err();
                            assert_eq!(got.downcast_ref::<RpcError>().unwrap().code, err["code"]);
                        }
                        None => assert!(got.is_err(), "{method} has no recorded answer"),
                    }
                }
                (Some(method), None) => bridge.notify(method, params).unwrap(),
                (None, Some(id)) => bridge.respond(id.clone(), &msg["result"]).unwrap(),
                (None, None) => {}
            }
        }
        // Closing stdin ends the replay server, which closes the event stream.
        drop(bridge);
        let mut out = Vec::new();
        while let Ok(event) = events.recv().await {
            out.push(event);
        }
        out
    });
    let recorded = lines
        .iter()
        .filter(|l| l["dir"] == "recv" && l["msg"].get("method").is_some());
    assert_eq!(
        events.len(),
        recorded.count(),
        "{path:?}: every server message replayed"
    );
    events.iter().for_each(check_event);
    events
}

#[test]
fn replay_readback() {
    let path = fixtures().join("readback.jsonl");
    if !path.exists() {
        println!("skipping: {path:?} not recorded yet");
        return;
    }
    replay_all(&path);
}

#[test]
fn replay_sessions() {
    let Ok(dir) = std::fs::read_dir(fixtures().join("sessions")) else {
        println!("skipping: fixtures/sessions not recorded yet");
        return;
    };
    for path in dir.map(|e| e.unwrap().path()) {
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        // Streamed deltas must add up to each completed agent message, in order.
        let mut streamed: HashMap<String, String> = HashMap::new();
        for event in replay_all(&path) {
            let Event::Notification { method, params } = event else {
                continue;
            };
            match method.as_str() {
                "item/agentMessage/delta" => {
                    let d: AgentMessageDeltaNotification = serde_json::from_value(params).unwrap();
                    *streamed.entry(d.item_id).or_default() += &d.delta;
                }
                "item/completed" => {
                    let n: ItemNotification = serde_json::from_value(params).unwrap();
                    if let ThreadItem::AgentMessage { id, text } = n.item {
                        assert_eq!(
                            streamed.get(&id).map_or(text.as_str(), |s| s),
                            text,
                            "{path:?}"
                        );
                    }
                }
                _ => {}
            }
        }
        println!("replayed {path:?}");
    }
}

#[test]
fn replay_answers_unrecorded_request_with_error() {
    let path = std::env::temp_dir().join(format!("bridge-empty-{}.jsonl", std::process::id()));
    std::fs::write(&path, "").unwrap();
    let (bridge, _events) = Bridge::replay(&path).unwrap();
    let err = block_on(bridge.model_list()).unwrap_err();
    assert_eq!(err.downcast_ref::<RpcError>().unwrap().code, -32601);
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "needs `ssh mac` with a signed-in /tmp/cgx-mac/home-live"]
fn live_turn() {
    let mut cmd = Command::new("ssh");
    cmd.args([
        "mac",
        "cd /tmp/cgx-mac/proj-live && CODEX_HOME=/tmp/cgx-mac/home-live \
         /Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex app-server",
    ]);
    let (bridge, events) = Bridge::spawn(cmd).unwrap();
    block_on(async {
        let init = bridge.initialize("codex-gpui", "0.1.0").await.unwrap();
        println!("server: {} on {}", init.user_agent, init.platform_os);
        let models = bridge.model_list().await.unwrap();
        println!(
            "models: {:?}",
            models.data.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
        println!(
            "account: {:?}",
            bridge.account_read().await.unwrap().account.is_some()
        );
        println!(
            "model: {}",
            bridge.config_read().await.unwrap().config["model"]
        );

        let thread = bridge.thread_start(None).await.unwrap().thread;
        bridge
            .turn_start(&thread.id, "Reply with the single word: pong")
            .await
            .unwrap();
        let mut text = String::new();
        loop {
            let event = events.recv().await.expect("app-server exited mid-turn");
            check_event(&event);
            let Event::Notification { method, params } = event else {
                continue;
            };
            match method.as_str() {
                "item/agentMessage/delta" => {
                    let d: AgentMessageDeltaNotification = serde_json::from_value(params).unwrap();
                    text += &d.delta;
                }
                "turn/completed" => {
                    let n: TurnNotification = serde_json::from_value(params).unwrap();
                    assert_eq!(n.turn.status, TurnStatus::Completed, "{:?}", n.turn.error);
                    break;
                }
                _ => {}
            }
        }
        println!("agent: {text:?}");
        assert!(text.to_lowercase().contains("pong"));

        let read = bridge.thread_read(&thread.id, true).await.unwrap();
        assert_eq!(read.thread.turns.len(), 1);
        let listed = bridge.thread_list().await.unwrap();
        println!("thread/list: {} threads", listed.data.len());
    });
}
