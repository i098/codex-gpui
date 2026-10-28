use anyhow::Result;
use app::host::{Exchange, Host, ReplayHost, Request};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../fixtures/gpts.json");

fn exchanges() -> Result<Vec<Exchange>> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    Ok(serde_json::from_value(recording["exchanges"].clone())?)
}

#[test]
fn every_gpts_exchange_replays_offline() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let exchanges = exchanges()?;
    assert_eq!(exchanges.len(), 6);
    for exchange in exchanges {
        assert_eq!(host.fetch(&exchange.request)?, exchange.response);
    }
    Ok(())
}

#[test]
fn gpts_mutations_and_unrecorded_reads_are_refused() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let read = exchanges()?.remove(2).request;
    let Request::Http(mut http) = read else {
        panic!("recorded GPT detail is an HTTP request")
    };
    for (method, url) in [
        ("POST", "/gizmos"),
        ("PATCH", "/gizmos/g-000000001-demo-gpt"),
        ("DELETE", "/gizmos/g-000000001-demo-gpt"),
        ("GET", "/gizmos/g-unrecorded"),
    ] {
        http["method"] = json!(method);
        http["url"] = json!(url);
        assert!(
            host.fetch(&Request::Http(http.clone())).is_err(),
            "{method} {url}"
        );
    }
    Ok(())
}
