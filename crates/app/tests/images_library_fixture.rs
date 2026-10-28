use anyhow::Result;
use app::host::{Exchange, Host as _, ReplayHost, Request};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../fixtures/images-library.json");

fn http(method: &str, url: &str, body: Option<&str>) -> Request {
    let exchange = serde_json::from_str::<Value>(RECORDING).unwrap()["exchanges"][0].clone();
    let mut request = exchange["request"]["request"].clone();
    request["method"] = method.into();
    request["url"] = url.into();
    match body {
        Some(body) => request["body"] = body.into(),
        None => {
            request.as_object_mut().unwrap().remove("body");
        }
    }
    Request::Http(request)
}

#[test]
fn every_images_and_library_exchange_replays_offline() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let exchanges: Vec<Exchange> =
        serde_json::from_value(serde_json::from_str::<Value>(RECORDING)?["exchanges"].clone())?;
    assert!(exchanges.len() >= 20);
    for exchange in exchanges {
        assert_eq!(host.fetch(&exchange.request)?, exchange.response);
        let wire = &exchange.response["value"]["response"]["__response"];
        assert_eq!(wire["status"], 200);
        serde_json::from_str::<Value>(wire["body"].as_str().unwrap())?;
    }
    Ok(())
}

#[test]
fn library_image_tab_lists_only_images() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let response = host.fetch(&http(
        "GET",
        "/files/library/nodes?entry_surface=library&categories=image&include_files=true&include_saved_entities=true",
        None,
    ))?;
    let body: Value = serde_json::from_str(
        response["value"]["response"]["__response"]["body"]
            .as_str()
            .unwrap(),
    )?;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["file"]["mime_type"], "image/svg+xml");
    Ok(())
}

#[test]
fn unknown_library_mutations_are_refused() {
    let host = ReplayHost::from_recording(RECORDING).unwrap();
    let batch = json!({"file_ids": ["file-owned-notes"]}).to_string();
    for request in [
        http("POST", "/files/library/files/delete-batch", Some(&batch)),
        http("DELETE", "/files/library/files/file-owned-notes", None),
        http(
            "POST",
            "/files/library/files/file-owned-notes/move",
            Some("{}"),
        ),
        http(
            "POST",
            "/files/library/favorites/status",
            Some("{\"file_ids\":[\"other\"]}"),
        ),
    ] {
        assert!(host.fetch(&request).is_err(), "{request:?}");
    }
}
