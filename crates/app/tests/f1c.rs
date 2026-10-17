//! Replays the original shell focus boundaries, including disabled history controls.
//! Traversal must not write to the host. `getAuthStatus` reads only when `refreshToken` is
//! explicitly false; `includeToken` selects returned data, not a refresh.
//! Refreshing calls and unknown editing methods remain writes.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT, script};
use gpui::{
    KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput,
    point, px,
};
use serde_json::Value;
use std::{fs, time::Duration};

#[test]
fn proxy() {
    common::proxy_child();
}

fn is_write(request: &Value) -> bool {
    request["method"].as_str().is_some_and(|method| {
        if method == "getAuthStatus" {
            return request["params"]["refreshToken"] != false;
        }
        request["params"]["refreshToken"] == true
            || (method != "initialized"
                && !["/read", "/list", "/get", "/search"]
                    .iter()
                    .any(|suffix| method.ends_with(suffix)))
    })
}

#[test]
fn request_checks_reject_refreshes_and_edits() {
    use serde_json::json;

    for include_token in [false, true] {
        assert!(!is_write(&json!({
            "method": "getAuthStatus",
            "params": {"includeToken": include_token, "refreshToken": false}
        })));
    }
    for request in [
        json!({"method": "getAuthStatus", "params": {"refreshToken": true}}),
        json!({"method": "getAuthStatus", "params": {"refreshToken": null}}),
        json!({"method": "getAuthStatus"}),
        json!({"method": "account/read", "params": {"refreshToken": true}}),
        json!({"method": "thread/start"}),
        json!({"method": "config/batchWrite"}),
        json!({"method": "getAndEdit"}),
    ] {
        assert!(is_write(&request), "{request}");
    }
}

fn step(fx: &mut Fx, step: &Value) -> Result<()> {
    if step["type"] == "key" {
        let prefix = if step["shift"] == true { "shift-" } else { "" };
        let key = step["key"].as_str().context("key")?.to_lowercase();
        let key = Keystroke::parse(&format!("{prefix}{key}"))?;
        fx.cx.update_window(fx.window, |_, window, cx| {
            window.dispatch_keystroke(key.clone(), cx);
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: key }), cx);
        })?;
    } else {
        let bbox = &step["target"]["bbox"];
        let number = |key: &str| bbox[key].as_f64().with_context(|| format!("missing {key}"));
        let at = (
            (number("x")? + number("w")? / 2.) as f32,
            (number("y")? + number("h")? / 2.) as f32,
        );
        match step["phase"].as_str() {
            Some("down") => {
                fx.hover(at)?;
                fx.input(PlatformInput::MouseDown(MouseDownEvent {
                    position: point(px(at.0), px(at.1)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }))?;
            }
            Some("up") => fx.input(PlatformInput::MouseUp(MouseUpEvent {
                position: point(px(at.0), px(at.1)),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
            }))?,
            None => fx.click(at)?,
            phase => anyhow::bail!("unsupported pointer phase {phase:?}"),
        }
    }
    fx.cx.run_until_parked();
    fx.cx.advance_clock(Duration::from_millis(160));
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn shell_tab_boundaries_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("f1c: requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/f1c.json"))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.shot()?;
    let before = fx.sent()?.len();
    for (index, input) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        step(&mut fx, input)?;
        let image = fx.shot()?;
        let observation = &recording["result"]["dom"][index];
        let scale = image.width() as f32 / 1280.;
        for sample in observation["pixels"].as_array().context("pixels")? {
            let x = sample["x"].as_f64().context("pixel x")? as f32;
            let y = sample["y"].as_f64().context("pixel y")? as f32;
            let got = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            let want = sample["rgb"].as_array().context("pixel rgb")?;
            let tolerance = sample["tolerance"].as_u64().unwrap_or(1) as u8;
            ensure!(
                (0..3).all(|c| got[c].abs_diff(want[c].as_u64().unwrap() as u8) <= tolerance),
                "step {index}: {} ring at ({x}, {y}) {got:?} != {want:?}",
                observation["aria"]
            );
        }
        if observation["aria"] == "Do anything" {
            ensure!(
                fx.composer_focused()?,
                "step {index}: Tab did not reach the composer"
            );
        }
    }
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .skip(before)
        .filter(is_write)
        .collect();
    ensure!(
        serde_json::to_value(writes)? == recording["result"]["requests"],
        "focus sent an app-server write"
    );
    ensure!(
        fx.route() == app::Route::Home,
        "history did not return Home"
    );
    println!("f1c: recorded forward/reverse traversal, history focus, and request checks passed");
    Ok(())
}
