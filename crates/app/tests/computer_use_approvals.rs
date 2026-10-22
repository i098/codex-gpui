#[cfg(target_os = "macos")]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn proxy() {
    common::proxy_child();
}

use app::computer_use_requests::approval;
use serde_json::{Value, json};
use ui::composer::computer_use::ComputerUseChoice;

fn request() -> Value {
    json!({
        "threadId": "thread-native", "turnId": "turn-native",
        "serverName": "lcu", "mode": "form", "message": "Allow TextEdit?",
        "requestedSchema": {"type": "object", "properties": {}},
        "_meta": {
            "codex_approval_kind": "mcp_tool_call", "connector_id": "computer-use",
            "connector_name": "Computer Use", "tool_params": {"app": "com.apple.TextEdit"},
            "tool_params_display": [{"name": "app", "value": "TextEdit"}],
            "riskLevel": "high", "subtitle": "The app can contain private data.",
            "persist": ["session"]
        }
    })
}

#[test]
fn grants_only_offered_native_scopes() {
    let params = request();
    let card = approval(&params).unwrap();
    assert_eq!(card.app, "com.apple.TextEdit");
    assert_eq!(card.display_name, "TextEdit");
    assert!(card.high_risk);
    assert_eq!(
        card.subtitle.as_deref(),
        Some("The app can contain private data.")
    );
    assert_eq!(
        card.response(ComputerUseChoice::Once),
        Some(json!({"action": "accept", "content": {}}))
    );
    assert_eq!(
        card.response(ComputerUseChoice::Conversation),
        Some(json!({
            "action": "accept", "content": {}, "_meta": {"persist": "session"}
        }))
    );
    assert_eq!(card.response(ComputerUseChoice::Always), None);
    assert_eq!(
        card.response(ComputerUseChoice::Decline),
        Some(json!({"action": "decline"}))
    );
    assert_eq!(
        card.response(ComputerUseChoice::Cancel),
        Some(json!({"action": "cancel"}))
    );

    let mut params = params;
    params["_meta"]["persist"] = json!(["always"]);
    let card = approval(&params).unwrap();
    assert_eq!(card.response(ComputerUseChoice::Conversation), None);
    assert_eq!(
        card.response(ComputerUseChoice::Always),
        Some(json!({
            "action": "accept", "content": {}, "_meta": {"persist": "always"}
        }))
    );
    params["_meta"]["persist"] = json!("always");
    let card = approval(&params).unwrap();
    assert_eq!(card.response(ComputerUseChoice::Conversation), None);
    assert_eq!(card.response(ComputerUseChoice::Always), None);
}

#[test]
fn keeps_schema_fields_out_of_native_empty_form_approval() {
    let mut params = request();
    params["requestedSchema"]["properties"] = json!({"password": {"type": "string"}});
    assert!(approval(&params).is_none());
    params["requestedSchema"]["properties"] = json!({});
    params["requestedSchema"]["required"] = json!(["password"]);
    assert!(approval(&params).is_none());
    params["requestedSchema"]["required"] = json!([]);
    params["_meta"]["connector_id"] = json!("another-connector");
    assert!(approval(&params).is_none());
    params["_meta"]["connector_id"] = json!("computer-use-extra");
    assert!(approval(&params).is_none());
}

#[test]
fn rejects_other_servers_claiming_native_metadata() {
    for server in [json!("untrusted"), Value::Null] {
        let mut params = request();
        params["serverName"] = server;
        assert!(approval(&params).is_none());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_cards_answer_the_bound_request_and_remember_disclosure() -> anyhow::Result<()> {
    use anyhow::ensure;
    use bridge::Event;
    use common::{EXPLAIN, Fx, SUBTRACT, script};

    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let root = fx.root.clone();
    let show = |fx: &mut Fx, id: &str, thread: &str, high: bool| {
        let mut params = request();
        params["threadId"] = json!(thread);
        params["turnId"] = Value::Null;
        params["_meta"]["persist"] = json!(["session", "always"]);
        params["_meta"]["riskLevel"] = json!(if high { "high" } else { "low" });
        params["_meta"]["subtitle"] = if high {
            json!("The app can contain private data.")
        } else {
            Value::Null
        };
        fx.cx.update(|cx| {
            root.update(cx, |root, cx| {
                root.on_event(
                    Event::Request {
                        id: json!(id),
                        method: "mcpServer/elicitation/request".into(),
                        params,
                    },
                    cx,
                )
            })
        });
        fx.cx.run_until_parked();
    };
    show(&mut fx, "other-thread", EXPLAIN, false);
    show(&mut fx, "native-app-approval", SUBTRACT, false);
    fx.capture_evidence("CGX_COMPUTER_USE_EVIDENCE", "native-dark-low")?;
    let composer = fx.composer();
    fx.focus_composer()?;
    fx.keys("enter")?;
    fx.wait("native session answer", |fx| fx.sent().unwrap().contains(&json!({
        "reply": "native-app-approval", "result": {"action": "accept", "content": {}, "_meta": {"persist": "session"}}
    })))?;
    ensure!(
        fx.read(|_, cx| cx
            .global::<app::host_settings::Settings>()
            .read_all()
            .get("has-approved-computer-use-app")
            == Some(&json!(true))),
        "acceptance did not retain the disclosure choice"
    );
    show(&mut fx, "native-high", SUBTRACT, true);
    fx.capture_evidence("CGX_COMPUTER_USE_EVIDENCE", "native-dark-high")?;
    fx.focus_composer()?;
    fx.keys("tab enter")?;
    fx.wait("native always answer", |fx| fx.sent().unwrap().contains(&json!({
        "reply": "native-high", "result": {"action": "accept", "content": {}, "_meta": {"persist": "always"}}
    })))?;
    let mut params = request();
    params["threadId"] = json!(SUBTRACT);
    params["turnId"] = Value::Null;
    params["_meta"]["persist"] = json!(["always"]);
    fx.cx.update(|cx| {
        root.update(cx, |root, cx| {
            root.on_event(
                Event::Request {
                    id: json!("native-once"),
                    method: "mcpServer/elicitation/request".into(),
                    params,
                },
                cx,
            )
        })
    });
    fx.cx.run_until_parked();
    fx.focus_composer()?;
    fx.keys("enter")?;
    fx.wait("native one-use answer", |fx| {
        fx.sent().unwrap().contains(&json!({
            "reply": "native-once", "result": {"action": "accept", "content": {}}
        }))
    })?;
    show(&mut fx, "native-deny", SUBTRACT, false);
    fx.focus_composer()?;
    fx.keys("escape")?;
    fx.wait("native denial", |fx| {
        fx.sent().unwrap().contains(&json!({
            "reply": "native-deny", "result": {"action": "decline"}
        }))
    })?;
    show(&mut fx, "withdrawn-native", SUBTRACT, false);
    fx.cx.update(|cx| {
        root.update(cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: "serverRequest/resolved".into(),
                    params: json!({"threadId": SUBTRACT, "requestId": "withdrawn-native"}),
                },
                cx,
            )
        })
    });
    fx.cx.run_until_parked();
    ensure!(
        fx.read(|_, cx| composer.read(cx).approval().is_none()),
        "the withdrawn native request stayed visible"
    );
    let responses: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|message| message.get("reply").is_some())
        .collect();
    ensure!(
        responses.len() == 4
            && responses
                .iter()
                .all(|message| message["reply"] != "other-thread"
                    && message["reply"] != "withdrawn-native"),
        "native cards answered another request: {responses:?}"
    );
    Ok(())
}
