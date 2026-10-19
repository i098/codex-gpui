//! S035b `autos-landing`: AM-036 (a suggestion card click), against `refs/interactions/s035b.json`
//! (the original on the route1 Linux stack). The original's start reads the TPP models through
//! the desktop host; the oracle answers that read with the host's 499. A live run showed that
//! a successful start next needs an Apple DeviceCheck token, which this host cannot make, so
//! this release answers the click with an unsupported message and sends no request.

use anyhow::{Context as _, Result};
use app::host::{Host, ReplayHost, Request, json_body, tpp_models};
use serde_json::Value;

const RECORDING: &str = include_str!("../../../refs/interactions/s035b.json");

/// The recorded `httpFetch.fetch` request and the oracle's answer.
fn recorded_exchange() -> Result<(Request, Value)> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let exchange = &recording["exchanges"][1];
    Ok((
        serde_json::from_value(exchange["request"].clone())?,
        exchange["response"].clone(),
    ))
}

#[test]
fn models_read_matches_the_recorded_request_and_fails_like_the_original() -> Result<()> {
    let host = ReplayHost::from_recording(RECORDING)?;
    let identity = host.identity().context("recorded identity")?;
    assert_eq!(identity.user_id, "user-000000000000000000000000");
    assert_eq!(identity.account_id, "<id>");
    assert_eq!(identity.device_id, "<id>");
    let (request, answer) = recorded_exchange()?;
    assert_eq!(tpp_models(&identity), request);
    let response = host.fetch(&request)?;
    assert_eq!(response, answer);
    assert_eq!(response["value"]["status"], 499);
    let error = json_body(&response).unwrap_err().to_string();
    assert!(error.contains("499"), "{error}");
    assert!(error.contains("Authenticated principal changed"), "{error}");
    Ok(())
}

#[test]
fn cloud_identity_comes_from_the_token_claims() -> Result<()> {
    use app::host::{CloudHost, Credentials, LocalHost};
    use base64::Engine as _;
    let claims = serde_json::json!({"https://api.openai.com/auth": {
        "chatgpt_account_id": "acct", "chatgpt_user_id": "user"
    }});
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    let token = format!("eyJhbGciOiJSUzI1NiJ9.{payload}.signature");
    let host = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(move |_| {
            Ok(Some(Credentials {
                token: token.clone(),
                backend_origin: None,
                routing_override: None,
            }))
        }),
        "http://127.0.0.1:1/backend-api",
    );
    let identity = host.identity().context("identity")?;
    assert_eq!(identity.account_id, "acct");
    assert_eq!(identity.user_id, "user");
    let parts: Vec<_> = identity.device_id.split('-').collect();
    assert_eq!(
        parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12]
    );
    assert!(parts[2].starts_with('4'), "{}", identity.device_id);
    assert!(
        parts[3].starts_with(['8', '9', 'a', 'b']),
        "{}",
        identity.device_id
    );
    assert!(
        identity
            .device_id
            .chars()
            .all(|c| c == '-' || c.is_ascii_hexdigit())
    );
    let signed_out = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(|_| Ok(None)),
        "http://127.0.0.1:1/backend-api",
    );
    assert!(signed_out.identity().is_none());
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn suggestion_click_says_unsupported_and_sends_nothing() -> Result<()> {
    use anyhow::ensure;
    use app::Route;
    use app::host::Identity;
    use common::{Fx, RAIL_SCHEDULED, changed};
    use image::{GenericImageView as _, RgbaImage};
    use parking_lot::Mutex;
    use std::{sync::Arc, time::Duration};

    const TOAST: &str = "Starting suggested tasks isn’t supported in this version";

    /// The replay host, recording every request the app sends.
    struct Recorder {
        replay: ReplayHost,
        sent: Mutex<Vec<Request>>,
    }

    impl Host for Recorder {
        fn fetch(&self, request: &Request) -> Result<Value> {
            self.sent.lock().push(request.clone());
            self.replay.fetch(request)
        }

        fn identity(&self) -> Option<Identity> {
            self.replay.identity()
        }
    }

    let recording: Value = serde_json::from_str(RECORDING)?;
    let step = &recording["steps"][0];
    let at = (
        step["at"]["x"].as_f64().context("x")? as f32,
        step["at"]["y"].as_f64().context("y")? as f32,
    );
    let bbox = |b: &Value| ["x", "y", "w", "h"].map(|k| b[k].as_f64().unwrap() as f32);
    let rest_dom = &recording["result"]["dom"][0];
    // The port shows the five landing examples (no server suggestions): the first five
    // recorded card boxes.
    let cards: Vec<[f32; 4]> = rest_dom["cards"]
        .as_array()
        .context("cards")?
        .iter()
        .take(5)
        .map(|c| bbox(&c["bbox"]))
        .collect();
    let icon = bbox(&recording["result"]["dom"][1]["cards"][0]["icon"]["bbox"]);
    let brightest = |shot: &RgbaImage, [x, y, w, h]: [f32; 4]| {
        let s = shot.width() as f32 / 1280.;
        shot.view(
            (x * s) as u32,
            (y * s) as u32,
            (w * s) as u32,
            (h * s) as u32,
        )
        .pixels()
        .map(|(_, _, p)| p[0].max(p[1]).max(p[2]))
        .max()
        .unwrap_or(0)
    };
    let proof = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));

    let host = Arc::new(Recorder {
        replay: ReplayHost::from_recording(RECORDING)?,
        sent: Mutex::new(Vec::new()),
    });
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("host automation list", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    let pending = |fx: &Fx| fx.read(|root, cx| root.suggestions().map(|p| p.read(cx).pending()));
    ensure!(pending(&fx) == Some(None), "page at rest");
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(300))?;
    let rest = fx.shot()?;
    rest.save(proof.join("s035b-before.png"))?;
    for card in &cards {
        ensure!(
            brightest(&rest, *card) >= 240,
            "card {card:?} at rest has white text"
        );
    }

    let before = host.sent.lock().len();
    fx.click(at)?;
    fx.wait("suggested-task click", |fx| {
        fx.read(|root, cx| !root.toasts(cx).is_empty())
    })?;
    fx.advance(Duration::from_millis(
        step["waitMs"].as_u64().unwrap_or(300),
    ))?;
    let toasts = fx.read(|root, cx| root.toasts(cx));
    ensure!(toasts == [TOAST], "toasts {toasts:?}");
    let sent = host.sent.lock()[before..].to_vec();
    ensure!(sent.is_empty(), "the click sent {sent:?}");
    ensure!(pending(&fx) == Some(None), "no card is Starting…");
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(300))?;
    let after = fx.shot()?;
    after.save(proof.join("s035b-after.png"))?;
    for card in &cards {
        ensure!(
            brightest(&after, *card) >= 240,
            "card {card:?} stays enabled"
        );
    }
    ensure!(
        changed(&rest, &after, icon) == 0,
        "no spinner in the icon slot"
    );
    println!(
        "s035b: click shows `{TOAST}`, sends no request, keeps the route and every card enabled"
    );
    Ok(())
}
