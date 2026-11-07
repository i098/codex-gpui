use anyhow::Result;
use app::host::{Host, ReplayHost, Request};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../fixtures/chatgpt-health-profile.json");

fn recording() -> Value {
    serde_json::from_str(RECORDING).expect("fixture JSON")
}

fn request(method: &str, url: &str, body: Option<&str>) -> Request {
    let identity = &recording()["identity"];
    let mut http = json!({
        "url": url, "method": method,
        "headers": {
            "OAI-Language": "en-US", "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1", "originator": "Codex Desktop",
            "ChatGPT-Account-ID": identity["accountId"], "oai-did": identity["deviceId"],
        },
        "expectedIdentity": {"accountId": identity["accountId"], "userId": identity["userId"]},
        "returnErrorResponse": true,
    });
    if url.starts_with("/aip/") {
        http["headers"]["OAI-Product-Sku"] = "CODEX".into();
    }
    if let Some(body) = body {
        http["body"] = body.into();
    }
    Request::Http(http)
}

fn body(response: &Value) -> Value {
    let wire = &response["value"]["response"]["__response"];
    assert_eq!(wire["status"], 200);
    serde_json::from_str(wire["body"].as_str().expect("body")).expect("JSON body")
}

#[test]
fn every_recorded_exchange_replays_offline() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let data = recording();
    let exchanges = data["exchanges"].as_array().expect("exchanges");
    assert_eq!(exchanges.len(), 7);
    for exchange in exchanges {
        let request: Request = serde_json::from_value(exchange["request"].clone())?;
        assert_eq!(host.fetch(&request)?, exchange["response"]);
    }
    let identity = host.identity().expect("recorded identity");
    assert_eq!(identity.user_id, data["identity"]["userId"]);
    Ok(())
}

#[test]
fn health_chats_page_by_offset_until_the_total() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let first = body(&host.fetch(&request("GET", "/aip/olympic/chats?offset=0", None))?);
    let next = first["offset"].as_u64().unwrap() + first["limit"].as_u64().unwrap();
    assert!(next < first["total"].as_u64().unwrap());
    let second = body(&host.fetch(&request(
        "GET",
        &format!("/aip/olympic/chats?offset={next}"),
        None,
    ))?);
    assert_eq!(second["offset"], next);
    assert_eq!(
        second["offset"].as_u64().unwrap() + second["limit"].as_u64().unwrap(),
        second["total"].as_u64().unwrap()
    );
    assert_eq!(second["items"][0]["title"], "");
    Ok(())
}

#[test]
fn profile_page_maps_as_the_original_rea() -> Result<()> {
    use ui::screens::profile::{self, Period};
    let host = ReplayHost::from_recording(RECORDING)?;
    let page = body(&host.fetch(&request("GET", "/profiles/me/page", None))?);
    let profile = profile::parse(&page).map_err(anyhow::Error::msg)?;
    assert_eq!(profile.name(), "Demo User");
    assert_eq!(profile.initials(), "DU");
    let summary = profile.summary.clone().expect("stats");
    assert_eq!(
        summary.lifetime_tokens.map(profile::compact).as_deref(),
        Some("5K")
    );
    assert_eq!(
        summary.longest_task_secs.map(profile::duration).as_deref(),
        Some("2m")
    );
    let insights = profile.insights.clone().expect("insights");
    // A zero Fast Mode share and an empty plugin list still show their rows as unused.
    assert_eq!(insights.fast_mode_percent, Some(Some(0.)));
    assert_eq!(insights.reasoning, Some((Some("low".into()), Some(100.))));
    assert_eq!(profile.plugins, Some(Vec::new()));
    let daily = profile.activity[0].as_deref().expect("daily buckets");
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
    let levels = profile::levels(daily, today, Period::Daily);
    // Wed 14 and Thu 15 Oct sit in the last column: 600 tokens tops the scale, 400 is level 3.
    assert_eq!(levels[levels.len() - 4..levels.len() - 2], [3, 4]);
    Ok(())
}

#[test]
fn unknown_mutations_and_reads_are_refused() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    for refused in [
        request(
            "PATCH",
            "/profiles/me",
            Some(r#"{"profile_visibility":"public"}"#),
        ),
        request("POST", "/profiles/me/page/pins", Some(r#"{"pins":[]}"#)),
        request("POST", "/aip/olympic/debug/sync-connections", None),
        request(
            "POST",
            "/aip/olympic/oauth/callback",
            Some(r#"{"full_redirect_url":"x"}"#),
        ),
        request("GET", "/aip/olympic/chats?offset=99", None),
    ] {
        assert!(host.fetch(&refused).is_err(), "{refused:?}");
    }
    Ok(())
}
