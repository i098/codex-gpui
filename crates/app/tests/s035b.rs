//! AM-036 starts a native guided conversation through the desktop host.
//! The original HTTP 499 recording remains a failure, not success evidence.

use anyhow::{Context as _, Result};
use app::host::Host;

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
fn suggestion_start_failure_keeps_the_cards_and_route() -> Result<()> {
    use anyhow::ensure;
    use app::Route;
    use app::host::{Identity, ReplayHost, Request};
    use common::{Fx, RAIL_SCHEDULED};
    use image::{GenericImageView as _, RgbaImage};
    use parking_lot::Mutex;
    use serde_json::Value;
    use std::{sync::Arc, time::Duration};

    const RECORDING: &str = include_str!("../../../refs/interactions/s035b.json");
    const TOAST: &str = "Couldn’t start this suggested task";

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
    let cards: Vec<[f32; 4]> = rest_dom["cards"]
        .as_array()
        .context("cards")?
        .iter()
        .map(|c| bbox(&c["bbox"]))
        .collect();
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
    let proof = std::env::var_os("S035B_SHOTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    std::fs::create_dir_all(&proof)?;

    let host = Arc::new(Recorder {
        replay: ReplayHost::from_recording(include_str!("../../../refs/interactions/host.json"))?,
        sent: Mutex::new(Vec::new()),
    });
    let mut fx = Fx::new()?;
    fx.cx
        .test_window(fx.window)
        .simulate_scale_factor_change(recording["viewport"]["dpr"].as_f64().context("dpr")? as f32);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("host automation list", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(300))?;
    let rest = fx.shot()?;
    ensure!(rest.dimensions() == (1280, 832), "recorded physical size");
    rest.save(proof.join("s035b-before.png"))?;
    for card in &cards {
        ensure!(
            brightest(&rest, *card) >= 240,
            "card {card:?} at rest has white text"
        );
    }

    host.sent.lock().clear();
    fx.click(at)?;
    fx.wait("suggested-task click", |fx| {
        fx.read(|root, cx| !root.toasts(cx).is_empty())
    })?;
    fx.advance(Duration::from_millis(
        step["waitMs"].as_u64().unwrap_or(300),
    ))?;
    let toasts = fx.read(|root, cx| root.toasts(cx));
    ensure!(toasts == [TOAST], "toasts {toasts:?}");
    let sent = host.sent.lock();
    ensure!(
        matches!(sent.as_slice(), [Request::Fetch { url, .. }] if url == "vscode://codex/start-suggested-task"),
        "the click sent {sent:?}"
    );
    drop(sent);
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    fx.advance(Duration::from_millis(300))?;
    let after = fx.shot()?;
    after.save(proof.join("s035b-after.png"))?;
    for card in &cards {
        ensure!(
            brightest(&after, *card) >= 240,
            "card {card:?} stays enabled"
        );
    }
    println!("s035b: refused start shows `{TOAST}` and keeps the route and every card enabled");
    Ok(())
}

#[cfg(unix)]
fn suggestion_server_script(refuse_turn: bool) -> String {
    use serde_json::json;
    let methods = if refuse_turn {
        vec!["model/list", "thread/start", "turn/start", "thread/archive"]
    } else {
        vec!["model/list", "thread/start", "turn/start"]
    };
    let mut script = String::new();
    for (index, method) in methods.into_iter().enumerate() {
        let result = match method {
            "model/list" => json!({"data": [{
                "id": "test", "model": "gpt-6.1-sol", "displayName": "Test",
                "description": "", "hidden": false, "isDefault": true,
                "defaultReasoningEffort": "low", "supportedReasoningEfforts": []
            }]}),
            "thread/start" => json!({"thread": {"id": "suggested-thread"}}),
            "turn/start" => json!({"turn": {
                "id": "suggested-turn", "items": [], "status": "inProgress"
            }}),
            _ => json!({}),
        };
        let reply = if refuse_turn && method == "turn/start" {
            json!({"id": index + 1, "error": {"code": -32000, "message": "refused"}})
        } else {
            json!({"id": index + 1, "result": result})
        };
        script.push_str(&format!(
            "IFS= read -r request || exit 1\nprintf '%s\\n' \"$request\" >> \"$1\"\nprintf '%s\\n' '{reply}'\n"
        ));
    }
    script
}

#[cfg(unix)]
#[test]
fn native_suggestion_starts_the_selected_prompt_and_cleans_up_a_refused_turn() -> Result<()> {
    use app::host::{LocalHost, Request, json_body};
    use bridge::Bridge;
    use serde_json::{Value, json};
    use std::sync::Arc;

    for refuse_turn in [false, true] {
        let home = tempfile::tempdir()?;
        let log = home.path().join("requests.jsonl");
        let mut command = std::process::Command::new("sh");
        command
            .args([
                "-c",
                &suggestion_server_script(refuse_turn),
                "suggested-task",
            ])
            .arg(&log);
        let (bridge, _) = Bridge::spawn(command)?;
        let host = LocalHost::new(home.path()).with_bridge(Arc::new(bridge));
        for suggestion in [
            json!({"cwd": home.path(), "user_message": "   "}),
            json!({"cwd": home.path(), "user_message": "Plan", "locked": {"is_locked": true}}),
        ] {
            assert!(
                host.fetch(&Request::post("start-suggested-task", Some(suggestion)))
                    .is_err()
            );
        }
        let request = Request::post(
            "start-suggested-task",
            Some(json!({
                "cwd": home.path(), "user_message": "Help me plan meals.", "title": "Meals"
            })),
        );
        let result = host.fetch(&request);
        if refuse_turn {
            assert!(result.unwrap_err().to_string().contains("refused"));
        } else {
            assert_eq!(json_body(&result?)?["thread"]["id"], "suggested-thread");
        }
        let requests = std::fs::read_to_string(log)?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(requests[0]["method"], "model/list");
        assert_eq!(requests[1]["method"], "thread/start");
        assert_eq!(requests[2]["method"], "turn/start");
        if refuse_turn {
            assert_eq!(requests[3]["method"], "thread/archive");
        }
        assert_eq!(requests[1]["params"]["model"], "gpt-6.1-sol");
        assert_eq!(requests[1]["params"]["sandbox"], "read-only");
        assert_eq!(requests[2]["params"]["threadId"], "suggested-thread");
        assert_eq!(
            requests[2]["params"]["input"][0]["text"],
            "Help me plan meals."
        );
        assert!(!home.path().join("automations").exists());
    }
    Ok(())
}
