use anyhow::Result;
use app::host::{Host, ReplayHost, Request};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../fixtures/conversations.json");

fn exchanges() -> Result<Vec<Value>> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    Ok(recording["exchanges"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

#[test]
fn every_conversation_exchange_replays_offline() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let exchanges = exchanges()?;
    assert_eq!(exchanges.len(), 7);
    for exchange in exchanges {
        let request: Request = serde_json::from_value(exchange["request"].clone())?;
        assert_eq!(host.fetch(&request)?, exchange["response"]);
    }
    Ok(())
}

#[test]
fn streams_carry_the_frozen_clock_and_complete() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    for exchange in exchanges()? {
        let request: Request = serde_json::from_value(exchange["request"].clone())?;
        let Request::Http(http) = &request else {
            panic!("conversation fixtures use the HTTP transport");
        };
        if !http["url"].as_str().unwrap().starts_with("/f/conversation") {
            continue;
        }
        let response = host.fetch(&request)?;
        let body = response
            .pointer("/value/response/__response/body")
            .and_then(Value::as_str);
        let body = body.unwrap();
        assert!(body.contains("message_stream_complete"));
        assert!(body.trim_end().ends_with("data: [DONE]"));
    }
    let detail: Value = serde_json::from_str(
        host.fetch(&serde_json::from_value(exchanges()?[1]["request"].clone())?)?
            .pointer("/value/response/__response/body")
            .and_then(Value::as_str)
            .unwrap(),
    )?;
    // 2026-10-15T17:00:00-07:00
    assert!(detail["update_time"].as_i64().unwrap() < 1_792_108_800);
    Ok(())
}

#[test]
fn unknown_conversation_mutations_are_refused() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let id = "68f00000-0000-4000-8000-0000000000c1";
    for (method, url, body) in [
        (
            "PATCH",
            format!("/conversation/{id}"),
            json!({"title": "x"}),
        ),
        ("DELETE", format!("/conversation/{id}"), json!(null)),
        (
            "POST",
            "/conversation/message_feedback".into(),
            json!({"rating": "thumbs_up"}),
        ),
        (
            "POST",
            "/f/conversation".into(),
            json!({"action": "next", "messages": []}),
        ),
    ] {
        let request = Request::Http(json!({
            "url": url, "method": method, "body": body.to_string(), "returnErrorResponse": true
        }));
        assert!(host.fetch(&request).is_err(), "{method} {url} was replayed");
    }
    Ok(())
}
