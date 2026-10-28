//! Offline replay of `fixtures/orbit-tasks.json`: remote Cloud tasks and Orbit rooms.

use anyhow::Result;
use app::host::{Host, ReplayHost, Request, json_body, response_wire};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../../fixtures/orbit-tasks.json");

fn request(path: &str, method: &str) -> Request {
    Request::Http(json!({
        "url": path, "method": method,
        "headers": {"OAI-Language":"en-US","X-OpenAI-Attach-Auth":"1","X-OpenAI-Attach-Integrity-State":"1","originator":"Codex Desktop"},
        "expectedIdentity": {"accountId":"acct-fixture","userId":"user-fixture"},
        "returnErrorResponse": true
    }))
}

fn host() -> Result<ReplayHost> {
    ReplayHost::from_recording(FIXTURE)
}

#[test]
fn every_recorded_exchange_replays_with_its_response() -> Result<()> {
    let host = host()?;
    let recording: Value = serde_json::from_str(FIXTURE)?;
    let exchanges = recording["exchanges"].as_array().unwrap();
    assert_eq!(exchanges.len(), 12);
    for exchange in exchanges {
        let request: Request = serde_json::from_value(exchange["request"].clone())?;
        assert_eq!(host.fetch(&request)?, exchange["response"]);
    }
    Ok(())
}

#[test]
fn archived_tasks_page_through_the_cursor() -> Result<()> {
    let host = host()?;
    let first = json_body(&host.fetch(&request(
        "/wham/tasks/list?limit=20&task_filter=archived",
        "GET",
    ))?)?;
    assert_eq!(first["items"][0]["id"], "task-fixture-3");
    let cursor = first["cursor"].as_str().unwrap();
    let second = json_body(&host.fetch(&request(
        &format!("/wham/tasks/list?limit=20&cursor={cursor}&task_filter=archived"),
        "GET",
    ))?)?;
    assert_eq!(second["items"][0]["id"], "task-fixture-4");
    assert!(second["cursor"].is_null());
    Ok(())
}

#[test]
fn task_detail_parses_with_the_native_projection() -> Result<()> {
    let host = host()?;
    let detail = json_body(&host.fetch(&request("/wham/tasks/task-fixture-1", "GET"))?)?;
    let turns = json_body(&host.fetch(&request("/wham/tasks/task-fixture-1/turns", "GET"))?)?;
    let task = app::parse_task_response(&detail, &turns)?;
    assert_eq!(task.title, "Update the greeting");
    Ok(())
}

#[test]
fn missing_task_replays_the_recorded_404() -> Result<()> {
    let response = host()?.fetch(&request("/wham/tasks/task-fixture-404", "GET"))?;
    assert_eq!(response_wire(&response)["status"], 404);
    Ok(())
}

#[test]
fn orbit_room_replays_room_messages_and_read() -> Result<()> {
    let host = host()?;
    let room = json_body(&host.fetch(&request("/messaging/rooms/room-fixture-1", "GET"))?)?;
    assert_eq!(room["read_receipts"][0]["account_user_id"], "user-fixture");
    let messages =
        json_body(&host.fetch(&request("/messaging/rooms/room-fixture-1/messages", "GET"))?)?;
    assert_eq!(messages["items"][0]["room_id"], "room-fixture-1");
    host.fetch(&request("/messaging/rooms/room-fixture-1/read", "POST"))?;
    Ok(())
}

#[test]
fn unknown_mutations_and_live_requests_are_refused() -> Result<()> {
    let host = host()?;
    for (path, method) in [
        ("/wham/tasks/task-fixture-2/cancel", "POST"),
        ("/wham/tasks/task-fixture-2/archive", "POST"),
        ("/wham/tasks/task-fixture-1", "DELETE"),
        ("/messaging/rooms/room-fixture-2/read", "POST"),
        ("/messaging/rooms/room-fixture-1/messages", "POST"),
    ] {
        assert!(
            host.fetch(&request(path, method)).is_err(),
            "{method} {path}"
        );
    }
    Ok(())
}
