use anyhow::Result;
use app::host::{Host, ReplayHost, Request, json_body};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../fixtures/space-sites.json");

fn get(url: &str) -> Request {
    let recording: Value = serde_json::from_str(RECORDING).unwrap();
    let mut request = recording["exchanges"][0]["request"]["request"].clone();
    request["url"] = url.into();
    Request::Http(request)
}

#[test]
fn every_recorded_exchange_replays_offline() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let host = ReplayHost::from_recording(RECORDING)?;
    let exchanges = recording["exchanges"].as_array().unwrap();
    assert_eq!(exchanges.len(), 8);
    for exchange in exchanges {
        let request: Request = serde_json::from_value(exchange["request"].clone())?;
        assert_eq!(host.fetch(&request)?, exchange["response"]);
        json_body(&host.fetch(&request)?)?;
    }
    let sites = json_body(&host.fetch(&get("/websites?limit=20"))?)?;
    assert_eq!(sites["items"][0]["slug"], "launch");
    assert_eq!(sites["items"][1]["sharing"]["access_mode"], "workspace");
    Ok(())
}

#[test]
fn unknown_requests_and_mutations_are_refused() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    assert!(host.fetch(&get("/websites?limit=21")).is_err());
    let Request::Http(mut create) = get("/websites") else {
        unreachable!()
    };
    create["method"] = "POST".into();
    create["body"] = json!({"title": "New site"}).to_string().into();
    assert!(host.fetch(&Request::Http(create.clone())).is_err());
    create["url"] = "/websites?limit=20".into();
    assert!(host.fetch(&Request::Http(create)).is_err());
    Ok(())
}
