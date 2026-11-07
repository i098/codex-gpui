use std::path::{Path, PathBuf};

use futures_lite::future::block_on;
use serde_json::{Value, json};

use super::*;
use crate::{Bridge, Event};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn recorded_server_messages(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["dir"] == "recv" && line["msg"].get("method").is_some())
        .map(|line| line["msg"].clone())
        .collect()
}

fn recorded_files() -> Vec<PathBuf> {
    let mut files = vec![fixtures().join("readback.jsonl")];
    files.extend(
        std::fs::read_dir(fixtures().join("sessions"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl")),
    );
    files
}

#[test]
fn every_recorded_notification_decodes_and_typed_methods_are_not_left_in_other() {
    let mut typed = 0;
    for path in recorded_files() {
        for msg in recorded_server_messages(&path) {
            let method = msg["method"].as_str().unwrap();
            let decoded = ServerNotification::decode(method, &msg["params"])
                .unwrap_or_else(|err| panic!("{path:?} {method}: {err:#}"));
            match decoded {
                ServerNotification::Other { method: kept, .. } => assert_eq!(kept, method),
                _ => typed += 1,
            }
        }
    }
    assert!(typed > 100, "typed variants decoded only {typed} messages");
}

#[test]
fn turn_completed_keeps_thread_and_turn_ids() {
    let msg = recorded_server_messages(&fixtures().join("sessions/greeting.jsonl"))
        .into_iter()
        .find(|m| m["method"] == "turn/completed")
        .unwrap();
    let ServerNotification::TurnCompleted(turn) =
        ServerNotification::decode("turn/completed", &msg["params"]).unwrap()
    else {
        panic!("turn/completed is not TurnCompleted");
    };
    assert_eq!(turn.thread_id, msg["params"]["threadId"]);
    assert_eq!(turn.turn.id, msg["params"]["turn"]["id"]);
}

#[test]
fn malformed_known_notification_is_an_error() {
    let err = ServerNotification::decode("turn/started", &json!({"threadId": 7})).unwrap_err();
    assert!(format!("{err:#}").contains("turn/started"));
}

#[test]
fn events_split_into_notifications_and_requests() {
    let note = Event::Notification {
        method: "deprecationNotice".into(),
        params: json!({}),
    };
    let request = Event::Request {
        id: json!(5),
        method: "item/tool/requestUserInput".into(),
        params: json!({"questions": []}),
    };
    assert!(ServerNotification::from_event(&note).is_some());
    assert!(ServerRequest::from_event(&note).is_none());
    assert!(ServerNotification::from_event(&request).is_none());
    assert!(matches!(
        ServerRequest::from_event(&request).unwrap().unwrap(),
        ServerRequest::UserInput { id, .. } if id == json!(5)
    ));
    assert!(ServerNotification::from_event(&Event::Closed).is_none());
}

#[test]
fn unknown_server_request_keeps_its_id_for_the_reply() {
    let ServerRequest::Other { id, method, .. } =
        ServerRequest::decode(&json!("abc"), "item/tool/call", &json!({})).unwrap()
    else {
        panic!("unknown method decoded as typed");
    };
    assert_eq!((id, method.as_str()), (json!("abc"), "item/tool/call"));
}

#[test]
fn config_write_params_match_the_shape_the_app_sends() {
    let edits = [
        ConfigEdit::upsert("features.memories", true),
        ConfigEdit::replace("model", "gpt-6"),
    ];
    assert_eq!(
        config::params(&edits),
        json!({
            "edits": [
                {"keyPath": "features.memories", "value": true, "mergeStrategy": "upsert"},
                {"keyPath": "model", "value": "gpt-6", "mergeStrategy": "replace"},
            ],
            "filePath": null,
            "expectedVersion": null,
            "reloadUserConfig": true,
        })
    );
}

#[test]
fn thread_list_params_match_the_recorded_request() {
    let recorded = std::fs::read_to_string(fixtures().join("readback.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|l| l["dir"] == "send" && l["msg"]["method"] == "thread/list")
        .unwrap();
    assert_eq!(
        ThreadListQuery::default().params(),
        recorded["msg"]["params"]
    );
    let archived = ThreadListQuery {
        archived: true,
        search_term: Some("math".into()),
        ..Default::default()
    }
    .params();
    assert_eq!(
        (&archived["archived"], &archived["searchTerm"]),
        (&json!(true), &json!("math"))
    );
}

#[test]
fn turn_start_params_include_only_set_overrides() {
    let plain = TurnStart::new("t1", "hi").params();
    assert_eq!(
        plain,
        json!({"threadId": "t1", "input": [{"type": "text", "text": "hi", "text_elements": []}]})
    );
    let fast = TurnStart {
        effort: Some("high".into()),
        service_tier: Some("priority".into()),
        ..TurnStart::new("t1", "hi")
    }
    .params();
    assert_eq!(
        (&fast["effort"], &fast["serviceTier"]),
        (&json!("high"), &json!("priority"))
    );
    assert!(fast.get("model").is_none());
}

fn script(name: &str, lines: &[Value]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("bridge-api-{}-{name}.jsonl", std::process::id()));
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    std::fs::write(&path, text.join("\n")).unwrap();
    path
}

fn exchange(id: u64, method: &str, params: Value, result: Value) -> [Value; 2] {
    [
        json!({"t": 0, "dir": "send", "msg": {"id": id, "method": method, "params": params}}),
        json!({"t": 0, "dir": "recv", "msg": {"id": id, "result": result}}),
    ]
}

#[test]
fn paged_follows_cursors_and_rejects_a_repeated_one() {
    let page = |id, cursor: Value, data: Value, next: Value| {
        exchange(
            id,
            "experimentalFeature/list",
            json!({"cursor": cursor}),
            json!({"data": data, "nextCursor": next}),
        )
    };
    let lines: Vec<Value> = [
        page(1, Value::Null, json!([{"name": "a"}]), json!("c1")),
        page(2, json!("c1"), json!([{"name": "b"}]), Value::Null),
    ]
    .concat();
    let path = script("paged", &lines);
    let (bridge, _events) = Bridge::replay(&path).unwrap();
    let all = block_on(bridge.experimental_features()).unwrap();
    assert_eq!(all, [json!({"name": "a"}), json!({"name": "b"})]);

    let looping: Vec<Value> = [
        page(1, Value::Null, json!([]), json!("c1")),
        page(2, json!("c1"), json!([]), json!("c1")),
    ]
    .concat();
    let path = script("loop", &looping);
    let (bridge, _events) = Bridge::replay(&path).unwrap();
    let err = block_on(bridge.experimental_features()).unwrap_err();
    assert!(format!("{err:#}").contains("did not advance"), "{err:#}");
}

#[test]
fn thread_list_all_decodes_recorded_threads() {
    let path = fixtures().join("readback.jsonl");
    let (bridge, _events) = Bridge::replay(&path).unwrap();
    let threads = block_on(bridge.thread_list_all(&ThreadListQuery::default())).unwrap();
    assert!(!threads.is_empty());
    assert!(threads.iter().all(|t| !t.id.is_empty()));
}
