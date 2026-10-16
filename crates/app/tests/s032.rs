//! S032 replays SH-013 (window drag region) from the original renderer recording: a press
//! on the empty 44 px header starts a native window move; the header buttons are `no-drag`.
//! Run on macOS: cargo test -p app --test s032 -- --nocapture.
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::Value;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn titlebar_drag_moves_the_window_but_its_buttons_do_not() -> Result<()> {
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s032.json"))?;
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    let point = |v: &Value| -> Result<(f32, f32)> {
        Ok((
            v["x"].as_f64().context("x")? as f32,
            v["y"].as_f64().context("y")? as f32,
        ))
    };
    let mut moves = 0;
    for step in rec["steps"].as_array().context("steps")? {
        let target = step["target"]["text"].as_str().unwrap_or("?");
        match step["type"].as_str() {
            Some("drag") => {
                fx.drag(point(&step["from"])?, point(&step["to"])?, 6)?;
                if step["nativeMove"] == true {
                    moves += 1;
                }
                let seen = fx.cx.test_window(fx.window).window_moves();
                ensure!(
                    seen == moves,
                    "drag on {target:?}: {seen} window moves, want {moves}"
                );
            }
            Some("click") => fx.click(point(&step["at"])?)?,
            kind => anyhow::bail!("unexpected step {kind:?}"),
        }
    }
    ensure!(moves == 1, "the recording drags the empty header once");
    ensure!(
        fx.sidebar_open() == rec["result"]["state"]["sidebarOpen"],
        "the sidebar toggle still acts after the drags"
    );
    Ok(())
}
