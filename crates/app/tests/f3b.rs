//! Replays SH-138 through SH-140 from the original renderer recording.
//! The original returns focus to the body, not the composer or menu trigger.
//! Close confirmation returns a native-host response; it sends no app-server request.
//! The port keeps the close preference in its persisted window state.
#![cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use gpui::{AppContext as _, Focusable as _, px, size};
use serde_json::Value;
use std::fs;
use ui::dialog::Kind;

fn recording() -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/f3b.json"
    ))?)?)
}

#[test]
fn proxy() {
    proxy_child();
}

fn replay_step(fx: &mut Fx, step: &Value) -> Result<()> {
    match step["type"].as_str().context("step type")? {
        "key" => fx.keys(step["portKey"].as_str().context("port key")?)?,
        "type" => fx.type_text(step["text"].as_str().context("typed text")?)?,
        "click" => {
            let at = &step["at"];
            let b = &step["target"]["bbox"];
            let n = |v: &Value| v.as_f64().unwrap_or_default() as f32;
            let p = if at.is_object() {
                (n(&at["x"]), n(&at["y"]))
            } else {
                (n(&b["x"]) + n(&b["w"]) / 2., n(&b["y"]) + n(&b["h"]) / 2.)
            };
            fx.click(p)?;
        }
        other => anyhow::bail!("unsupported step {other}"),
    }
    if fx.cx.update(|cx| !cx.windows().is_empty()) {
        fx.shot()?;
    }
    Ok(())
}

fn check(fx: &mut Fx, rec: &Value, after: &str) -> Result<()> {
    let d = rec["result"]["dom"]
        .as_array()
        .context("recorded DOM")?
        .iter()
        .find(|d| d["after"] == after)
        .context("checkpoint")?;
    if d["response"]["confirmed"] == true {
        ensure!(
            fx.cx.update(|cx| cx.windows().is_empty()),
            "confirmed close left the window open"
        );
        return Ok(());
    }
    if d["dialog"].is_null() {
        ensure!(
            fx.read(|root, _| root.dialog().is_none()),
            "{after}: dialog remains open"
        );
        ensure!(
            !fx.composer_focused()?,
            "{after}: focus did not return to the root"
        );
        return Ok(());
    }
    let dialog = fx
        .read(|root, _| root.dialog().cloned())
        .context("dialog missing")?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        let dialog = dialog.read(cx);
        ensure!(
            dialog.focus_handle(cx).is_focused(window),
            "{after}: dialog not focused"
        );
        let expected = if d["focus"]["tag"] == "input" {
            "Search shortcuts"
        } else {
            d["focus"]["text"].as_str().unwrap_or_default()
        };
        if after != "close-checked" {
            ensure!(
                dialog.focused_control() == expected,
                "{after}: wrong focused control"
            );
        }
        if dialog.kind() == Kind::Shortcuts {
            match after {
                "shortcuts-open" => {
                    let titles: Vec<_> = rec["catalog"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|s| s["rows"].as_array().unwrap())
                        .map(|r| r["title"].as_str().unwrap())
                        .collect();
                    ensure!(
                        dialog.visible_titles() == titles,
                        "active shortcut catalog differs"
                    );
                }
                "shortcuts-filter" => {
                    ensure!(dialog.query() == "archive");
                    ensure!(dialog.visible_titles() == ["Archive chat"]);
                }
                "shortcuts-no-match" => {
                    ensure!(dialog.query() == "zzzz-no-shortcut");
                    ensure!(dialog.visible_titles().is_empty());
                }
                _ => {}
            }
        } else {
            ensure!(dialog.do_not_ask_again() == (after == "close-checked"));
        }
        Ok::<_, anyhow::Error>(())
    })??;
    let pixel = &d["pixel"];
    let image = fx.shot()?;
    let scale = image.width() as f32 / 1280.;
    let x = (pixel["x"].as_f64().context("pixel x")? as f32 * scale) as u32;
    let y = (pixel["y"].as_f64().context("pixel y")? as f32 * scale) as u32;
    let got = image.get_pixel(x, y).0;
    let want = pixel["rgba"].as_array().context("recorded pixel")?;
    ensure!(
        got.iter()
            .zip(want)
            .all(|(g, w)| (*g as i64 - w.as_i64().unwrap()).abs() <= 2),
        "{after}: pixel {got:?}, expected {want:?}"
    );
    let dir = std::path::Path::new("/tmp/cgx-f3b-port");
    fs::create_dir_all(dir)?;
    image.save(dir.join(format!("{after}.png")))?;
    Ok(())
}

#[test]
fn modal_recording() -> Result<()> {
    let rec = recording()?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let before = fx.sent()?.len();
    fx.shot()?;
    for step in rec["steps"].as_array().context("steps")? {
        replay_step(&mut fx, step)?;
        if let Some(after) = step["after"].as_str() {
            check(&mut fx, &rec, after)?;
        }
    }
    ensure!(
        fx.read(|root, _| root.config()["host"]["confirm-before-close"] == "keyboardOnly"),
        "Cancel persisted the checkbox"
    );
    for step in rec["confirmationSteps"]
        .as_array()
        .context("confirmation steps")?
    {
        replay_step(&mut fx, step)?;
        if let Some(after) = step["after"].as_str() {
            check(&mut fx, &rec, after)?;
        }
    }
    ensure!(
        fx.sent()?[before..] == *rec["result"]["requests"].as_array().context("requests")?,
        "dialog sent an app-server request"
    );
    ensure!(
        fx.read(|root, _| root.config()["host"]["confirm-before-close"] == "never"),
        "confirmation did not disable future warnings"
    );
    Ok(())
}

#[test]
fn keyboard_trap_and_help_menu() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.menu("Help", "Keyboard Shortcuts")?;
    fx.shot()?;
    fx.keys("shift-tab")?;
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).focused_control() == "Close dialog"));
    fx.keys("tab")?;
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).focused_control() == "Search shortcuts"));
    fx.keys("cmd-/")?;
    ensure!(fx.read(|r, _| r.dialog().is_none()));
    fx.keys("cmd-w")?;
    fx.shot()?;
    fx.keys("enter")?;
    ensure!(
        fx.cx.update(|cx| cx.windows().len()) == 1,
        "Enter must default to Cancel"
    );
    fx.keys("cmd-w")?;
    fx.shot()?;
    fx.keys("shift-tab space escape")?;
    fx.keys("cmd-w")?;
    fx.shot()?;
    ensure!(
        !fx.read(|r, cx| r.dialog().unwrap().read(cx).do_not_ask_again()),
        "dismiss retained the checkbox"
    );
    Ok(())
}

#[test]
fn close_preference_survives_restart() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.cx
        .update_window(fx.window, |_, window, _| window.remove_window())?;
    let path =
        std::path::Path::new("/tmp").join(format!("cgx-f3b-state-{}.json", std::process::id()));
    let mut state = app::State {
        path: Some(path.clone()),
        ..app::State::default()
    };
    for first in [true, false] {
        let (bridge, events) =
            bridge::Bridge::replay(&std::path::Path::new(ROOT).join("fixtures/readback.jsonl"))?;
        let window = fx.cx.open_window(size(px(1280.), px(832.)), |window, cx| {
            cx.new(|cx| app::Root::new(state, bridge, events, CWD.into(), window, cx))
        })?;
        fx.root = fx.cx.read_window(&window, |root, _| root)?;
        fx.window = window.into();
        fx.wait("startup", |fx| fx.read(|root, _| root.ready()))?;
        fx.shot()?;
        fx.keys("cmd-w")?;
        if first {
            fx.shot()?;
            fx.click((457., 428.))?;
            fx.click((766., 465.))?;
        }
        ensure!(
            fx.cx.update(|cx| cx.windows().is_empty()),
            "close preference was not applied"
        );
        state = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(state.confirm_before_close == app::ConfirmBeforeClose::Never);
        state.path = Some(path.clone());
    }
    fs::remove_file(path)?;
    Ok(())
}
