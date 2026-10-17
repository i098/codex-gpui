//! Replays the original composer's Escape, permission, and Tab interactions.

mod common;

use anyhow::{Result, bail, ensure};
use common::{Fx, exchange, proxy_child, recorded_thread, script};
use gpui::EntityInputHandler as _;
use serde_json::{Value, json};
use std::time::Duration;

#[test]
fn proxy() {
    proxy_child();
}

fn recording() -> Value {
    serde_json::from_str(include_str!("../../../refs/interactions/s024.json")).unwrap()
}

fn thread_step(rec: &Value) -> &Value {
    rec["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["waitThread"].is_string())
        .unwrap()
}

fn open_thread(fx: &mut Fx, rec: &Value) -> Result<()> {
    let step = thread_step(rec);
    let prefix = step["target"]["text"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    fx.click(fx.at("home", &prefix)?)?;
    fx.wait_thread(step["waitThread"].as_str().unwrap())
}

fn fixture(rec: &Value) -> Result<Fx> {
    let mut lines = script(None, |_| {})?;
    let list_id = lines
        .iter()
        .find(|line| line["msg"]["method"] == "thread/list")
        .unwrap()["msg"]["id"]
        .clone();
    let list = lines
        .iter()
        .find(|line| line["dir"] == "recv" && line["msg"]["id"] == list_id)
        .unwrap()["msg"]["result"]
        .clone();
    lines.extend(exchange("thread/list", list));
    lines.extend(exchange(
        "thread/resume",
        json!({"thread":recorded_thread(thread_step(rec)["waitThread"].as_str().unwrap())?}),
    ));
    for method in ["thread/start", "turn/start", "turn/start", "turn/interrupt"] {
        let mut refused = exchange(method, json!({}));
        refused[1]["msg"].as_object_mut().unwrap().remove("result");
        refused[1]["msg"]["error"] =
            json!({"code":-32000,"message":format!("route1: {method} is not allowed")});
        lines.extend(refused);
    }
    Fx::captured("dark", (1280., 832.), lines)
}

fn notify(fx: &mut Fx, step: &Value) {
    let event = bridge::Event::Notification {
        method: step["method"].as_str().unwrap().into(),
        params: step["params"].clone(),
    };
    fx.root
        .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
    fx.cx.run_until_parked();
}

fn center(bounds: &Value) -> (f32, f32) {
    (
        (bounds["x"].as_f64().unwrap() + bounds["w"].as_f64().unwrap() / 2.) as f32,
        (bounds["y"].as_f64().unwrap() + bounds["h"].as_f64().unwrap() / 2.) as f32,
    )
}

fn interrupts(fx: &mut Fx) -> Result<usize> {
    Ok(fx
        .sent()?
        .iter()
        .filter(|request| request["method"] == "turn/interrupt")
        .count())
}

#[test]
fn recorded_escape_permissions_and_tab_order() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec = recording();
    let mut fx = fixture(&rec)?;
    fx.shot()?;
    for (index, step) in rec["steps"].as_array().unwrap().iter().enumerate() {
        match step["type"].as_str().unwrap() {
            "click" if step["waitThread"].is_string() => open_thread(&mut fx, &rec)?,
            // Keep the existing 320 px menu geometry, as in f2a.rs.
            "click"
                if step["target"]["text"] == "Ask for approval"
                    && step["target"]["selector"].is_null() =>
            {
                fx.click((600., 580.))?
            }
            "click"
                if step["target"]["text"] == "Approve for me"
                    && step["target"]["selector"].is_null() =>
            {
                fx.click((600., 628.))?
            }
            "click"
                if step["target"]["text"] == "Full access"
                    && step["target"]["selector"].is_null() =>
            {
                fx.click((600., 684.))?
            }
            "click" => fx.click(center(&step["target"]["bbox"]))?,
            "type" => fx.type_text(step["text"].as_str().unwrap())?,
            "key" => fx.keys(step["portKey"].as_str().unwrap())?,
            "bridge" => {
                notify(&mut fx, step);
                let running = step["method"] == "turn/started";
                fx.wait("recorded turn state", |fx| {
                    fx.read(|root, cx| {
                        root.thread()
                            .unwrap()
                            .read(cx)
                            .turns()
                            .iter()
                            .any(|turn| turn.status == bridge::types::TurnStatus::InProgress)
                    }) == running
                })?;
            }
            other => bail!("unsupported recorded step {other}"),
        }
        let frame = fx.shot()?;
        if step["target"]["text"] == "Full access" && step["target"]["selector"].is_null() {
            ensure!(
                fx.read(|root, cx| root
                    .dialog()
                    .is_some_and(|dialog| dialog.read(cx).kind() == ui::dialog::Kind::FullAccess)),
                "Full access did not show its warning"
            );
            ensure!(
                fx.composer()
                    .read_with(&fx.cx, |composer, _| composer.permission())
                    != ui::composer::Permission::FullAccess,
                "Full access changed before confirmation"
            );
            frame.save(
                std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
                    .join("s024-full-access-confirm.png"),
            )?;
        }
        if let Some(tab) = rec["result"]["tab"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tab| tab["afterStep"] == index && index != 0)
        {
            let active = &tab["active"];
            let bounds = &active["bbox"];
            let x = bounds["x"].as_f64().unwrap();
            let y = bounds["y"].as_f64().unwrap() + bounds["h"].as_f64().unwrap() / 2.;
            let color = if active["computed"]["boxShadow"] == "none" {
                [255, 255, 255]
            } else {
                [58, 131, 247]
            };
            ensure!(
                (1..=4).any(
                    |offset| frame.get_pixel((x * 2.) as u32 - offset, (y * 2.) as u32).0[..3]
                        == color
                ),
                "Tab step {index}: missing ring on {}",
                active["aria"]
            );
        }
        for dom in rec["result"]["dom"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|dom| dom["afterStep"] == index)
        {
            if dom["selector"] == "[aria-label=\"Change permissions\"]" {
                let permission = match dom["text"].as_str().unwrap() {
                    "Ask for approval" => ui::composer::Permission::Ask,
                    "Approve for me" => ui::composer::Permission::ApproveForMe,
                    "Full access" => ui::composer::Permission::FullAccess,
                    other => bail!("unexpected recorded permission {other}"),
                };
                ensure!(
                    fx.composer()
                        .read_with(&fx.cx, |composer, _| composer.permission())
                        == permission,
                    "permission pill differs at step {index}"
                );
            }
            if dom["state"] == "stop confirmation armed" {
                ensure!(
                    fx.composer()
                        .read_with(&fx.cx, |composer, _| composer.stop_armed()),
                    "Escape did not show Esc"
                );
                frame.save(
                    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("s024-armed.png"),
                )?;
                ensure!(
                    interrupts(&mut fx)? == 0,
                    "first Escape interrupted the turn"
                );
            }
            if dom["state"] == "menu closed by Escape" {
                ensure!(
                    !fx.composer()
                        .read_with(&fx.cx, |composer, _| composer.stop_armed()),
                    "menu Escape armed Stop"
                );
                ensure!(
                    interrupts(&mut fx)? == 0,
                    "menu Escape interrupted the turn"
                );
            }
        }
    }
    // Prewarm falls back to ordinary RPC after the stub refuses it. Compare the ordinary requests.
    let expected: Vec<_> = rec["result"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["type"] == "mcp-request")
        .collect();
    fx.wait("recorded start and interrupt requests", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .filter(|request| {
                matches!(
                    request["method"].as_str(),
                    Some("thread/start" | "turn/start" | "turn/interrupt")
                )
            })
            .count()
            == expected.len()
    })?;
    let actual: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| {
            matches!(
                request["method"].as_str(),
                Some("thread/start" | "turn/start" | "turn/interrupt")
            )
        })
        .collect();
    for (actual, expected) in actual.iter().zip(expected) {
        ensure!(
            actual["method"] == expected["method"],
            "request order: {actual} != {expected}"
        );
        let keys: &[&str] = if actual["method"] == "turn/interrupt" {
            &["threadId", "turnId"]
        } else {
            &["approvalPolicy", "approvalsReviewer", "permissions"]
        };
        for key in keys {
            ensure!(
                actual["params"][*key] == expected["params"][*key],
                "{key}: {actual} != {expected}"
            );
        }
        ensure!(
            actual["params"].get("sandbox").is_none(),
            "obsolete sandbox parameter"
        );
    }
    ensure!(fx.composer_focused()?, "Escape moved focus from the editor");
    Ok(())
}

#[test]
fn escape_ignores_composition_and_suggestions_and_expires_confirmation() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec = recording();
    let mut fx = fixture(&rec)?;
    fx.shot()?;
    open_thread(&mut fx, &rec)?;
    let index = rec["steps"]
        .as_array()
        .unwrap()
        .iter()
        .position(|step| step["method"] == "turn/started")
        .unwrap();
    notify(&mut fx, &rec["steps"][index]);
    fx.wait("running turn", |fx| {
        fx.composer()
            .read_with(&fx.cx, |composer, _| composer.send_label() == "Stop")
    })?;
    fx.click((600., 745.))?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |composer, cx| {
            composer.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx)
        })
    })?;
    fx.keys("escape escape")?;
    ensure!(
        interrupts(&mut fx)? == 0,
        "Escape interrupted during composition"
    );
    ensure!(!composer.read_with(&fx.cx, |composer, _| composer.stop_armed()));
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |composer, cx| composer.unmark_text(window, cx))
    })?;
    fx.keys("cmd-a backspace")?;
    fx.type_text("/")?;
    fx.keys("escape")?;
    ensure!(
        !composer.read_with(&fx.cx, |composer, _| composer.stop_armed()),
        "suggestion Escape armed Stop"
    );
    fx.keys("cmd-a backspace escape")?;
    ensure!(composer.read_with(&fx.cx, |composer, _| composer.stop_armed()));
    fx.advance(Duration::from_millis(2100))?;
    ensure!(
        !composer.read_with(&fx.cx, |composer, _| composer.stop_armed()),
        "Esc hint did not expire"
    );
    fx.keys("escape")?;
    ensure!(
        interrupts(&mut fx)? == 0,
        "expired confirmation interrupted the turn"
    );
    fx.keys("escape")?;
    fx.wait("confirmed interrupt", |fx| interrupts(fx).unwrap() == 1)?;
    Ok(())
}
