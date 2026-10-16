//! S035b `autos-landing`: AM-036 (a suggestion card click starts its guided conversation),
//! replayed from `refs/interactions/s035b.json` (the original on the route1 Linux stack).
//! The original's start reads the TPP models through the desktop host first; the oracle
//! answers that read with the host's 499 `Authenticated principal changed`, so the recorded
//! outcome is the danger toast `Couldn’t start this suggested task` with every card enabled
//! again. The replay host answers the port's request only when it equals the recorded one.

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
fn suggestion_click_starts_and_fails_like_original() -> Result<()> {
    use anyhow::ensure;
    use app::Route;
    use app::host::Identity;
    use common::{Fx, RAIL_SCHEDULED, changed};
    use gpui::{
        Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point,
        px,
    };
    use image::{GenericImageView as _, RgbaImage};
    use parking_lot::Mutex;
    use std::{sync::Arc, time::Duration};

    const TOAST: &str = "Couldn’t start this suggested task";

    /// The replay host plus the log of the ChatGPT requests the port asked it. Their answer waits for
    /// `released` (at most `TIMEOUT`), so the test observes the in-flight state like the
    /// recording's 104 ms. While gated, the test must not run the app until it parks.
    struct Recorder {
        replay: ReplayHost,
        sent: Mutex<Vec<Request>>,
        released: Mutex<bool>,
    }

    impl Host for Recorder {
        fn fetch(&self, request: &Request) -> Result<Value> {
            if let Request::Http(_) = request {
                self.sent.lock().push(request.clone());
                let started = std::time::Instant::now();
                while !*self.released.lock() && started.elapsed() < common::TIMEOUT {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
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
    let starting = &recording["result"]["dom"][1];
    // The port shows the five landing examples (no server suggestions): the first five
    // recorded card boxes.
    let cards: Vec<[f32; 4]> = starting["cards"]
        .as_array()
        .context("cards")?
        .iter()
        .take(5)
        .map(|c| bbox(&c["bbox"]))
        .collect();
    let icon = bbox(&starting["cards"][0]["icon"]["bbox"]);
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

    let host = Arc::new(Recorder {
        replay: ReplayHost::from_recording(RECORDING)?,
        sent: Mutex::new(Vec::new()),
        released: Mutex::new(false),
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
    // At rest: white title text in every card; the page background behind the card content.
    let backdrop = rest
        .get_pixel(
            ((cards[0][0] + cards[0][2] - 4.) * 2.) as u32,
            ((cards[0][1] + cards[0][3] / 2.) * 2.) as u32,
        )
        .0;
    let dim = (0..3)
        .map(|c| (f32::from(backdrop[c]) + 0.4 * (255. - f32::from(backdrop[c]))) as u8)
        .max()
        .unwrap()
        + 3;
    for card in &cards {
        ensure!(
            brightest(&rest, *card) >= 240,
            "card {card:?} at rest has white text"
        );
    }

    // The click, without running the host answer: the Starting state is observable.
    fx.hover(at)?;
    let position = point(px(at.0), px(at.1));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
            cx,
        );
    })?;
    ensure!(
        pending(&fx) == Some(Some(0)),
        "the clicked card is Starting…"
    );
    let shot = fx.cx.capture_screenshot(fx.window)?;
    // Every card is disabled at opacity 0.4: nothing in any card is brighter than white at
    // 0.4 over the backdrop. The icon slot shows the rays spinner instead of the emoji.
    for card in &cards {
        let max = brightest(&shot, *card);
        ensure!(
            max <= dim,
            "card {card:?} while starting: brightest {max} > {dim}"
        );
    }
    ensure!(changed(&rest, &shot, icon) > 0, "spinner in the icon slot");
    // A disabled card has no hover fill. Raw dispatch: `Fx::hover` runs the app until it
    // parks, which waits for the gated host answer.
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position: point(
                    px(cards[1][0] + cards[1][2] / 2.),
                    px(cards[1][1] + cards[1][3] / 2.),
                ),
                pressed_button: None,
                modifiers: Modifiers::default(),
            }),
            cx,
        );
    })?;
    ensure!(
        brightest(&fx.cx.capture_screenshot(fx.window)?, cards[1]) <= dim,
        "no hover fill while disabled"
    );
    ensure!(pending(&fx) == Some(Some(0)), "still the first card");

    // The host answers the models read with the recorded 499: toast, cards enabled.
    *host.released.lock() = true;
    fx.wait("suggested-task start", |fx| {
        fx.read(|root, cx| !root.toasts(cx).is_empty())
    })?;
    fx.advance(Duration::from_millis(
        step["waitMs"].as_u64().unwrap_or(300),
    ))?;
    let (request, _) = recorded_exchange()?;
    ensure!(
        *host.sent.lock() == [request],
        "the port sent {:?}",
        host.sent.lock()
    );
    let toasts = fx.read(|root, cx| root.toasts(cx));
    ensure!(toasts == [TOAST], "toasts {toasts:?}");
    ensure!(pending(&fx) == Some(None), "cards enabled again");
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(300))?;
    let restored = fx.shot()?;
    for card in &cards {
        ensure!(brightest(&restored, *card) >= 240, "card {card:?} restored");
    }
    println!(
        "s035b: click shows Starting…, disables every card, sends the recorded models read, then toasts and restores"
    );
    Ok(())
}
