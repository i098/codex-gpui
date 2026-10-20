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
    params["_meta"]["persist"] = json!("always");
    let card = approval(&params).unwrap();
    assert_eq!(card.response(ComputerUseChoice::Conversation), None);
    assert_eq!(
        card.response(ComputerUseChoice::Always),
        Some(json!({
            "action": "accept", "content": {}, "_meta": {"persist": "always"}
        }))
    );
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
}
