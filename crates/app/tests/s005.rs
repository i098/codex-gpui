//! S005 replays user-message hover and copy from the original renderer.
//! Run on macOS: cargo test -p app --test s005 -- --nocapture.
mod common;

use anyhow::{Result, ensure};
use common::*;
use serde_json::Value;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn user_message_hover_and_copy() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s005: macOS renderer required");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s005.json"))?;
    let steps = rec["steps"].as_array().unwrap();
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(target(&steps[0]))?;
    fx.wait_thread(GREETING)?;
    fx.hover((1270., 820.))?;
    let rest = fx.shot()?;
    fx.hover(target(&steps[1]))?;
    let hovered = fx.shot()?;
    if let Some(path) = std::env::var_os("S005_PROOF") {
        hovered.save(path)?;
    }
    let bbox = |b: &Value| ["x", "y", "w", "h"].map(|k| b[k].as_f64().unwrap() as f32);
    let actions = bbox(&rec["result"]["dom"][1]["bbox"]);
    let button = bbox(&steps[2]["target"]["bbox"]);
    let copy_icon = [button[0] + 5., button[1] + 5., 16., 16.];
    ensure!(
        changed(&rest, &hovered, actions) > 100,
        "user-message actions did not appear"
    );
    fx.hover(target(&steps[2]))?;
    advance(&mut fx, steps[2]["waitMs"].as_u64().unwrap());
    let ready = fx.shot()?;
    let rgba: Vec<f32> = rec["result"]["dom"][2]["computed"]["backgroundColor"]
        .as_str()
        .unwrap()
        .trim_start_matches("rgba(")
        .trim_end_matches(')')
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    let sample = ((button[0] + 2.) as u32 * 2, (button[1] + 13.) as u32 * 2);
    let backdrop = rest.get_pixel(sample.0, sample.1);
    let fill = ready.get_pixel(sample.0, sample.1);
    for channel in 0..3 {
        let expected =
            (rgba[channel] * rgba[3] + backdrop[channel] as f32 * (1. - rgba[3])).round() as u8;
        ensure!(
            fill[channel].abs_diff(expected) <= 1,
            "copy hover fill differs from the recording"
        );
    }
    let before = fx.sent()?.len();
    fx.click(target(&steps[3]))?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|item| item.text());
    ensure!(
        copied.as_deref() == rec["result"]["clipboard"][0]["values"]["text/plain"].as_str(),
        "copy did not write the recorded message"
    );
    let checked = fx.shot()?;
    ensure!(
        changed(&ready, &checked, copy_icon) > 10,
        "copy did not show a checkmark"
    );
    advance(
        &mut fx,
        rec["result"]["beforeReset"]["elapsedMs"].as_u64().unwrap(),
    );
    ensure!(
        changed(&checked, &fx.shot()?, copy_icon) == 0,
        "copy feedback reset too soon"
    );
    let remaining = rec["result"]["afterReset"]["elapsedMs"].as_u64().unwrap()
        - rec["result"]["beforeReset"]["elapsedMs"].as_u64().unwrap();
    advance(&mut fx, remaining);
    let reset = fx.shot()?;
    if let Some(path) = std::env::var_os("S005_PROOF") {
        ready.save(std::path::Path::new(&path).with_extension("ready.png"))?;
        reset.save(std::path::Path::new(&path).with_extension("reset.png"))?;
    }
    ensure!(
        changed(&checked, &reset, copy_icon) > 10,
        "copy feedback did not reset"
    );
    fx.hover(target(&steps[5]))?;
    ensure!(
        changed(&rest, &fx.shot()?, actions) > 100,
        "focus did not keep the actions visible"
    );
    let before_keyboard = fx.shot()?;
    fx.keys(&steps[6]["key"].as_str().unwrap().to_lowercase())?;
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard())
            .and_then(|item| item.text())
            == copied,
        "keyboard copy changed the message"
    );
    ensure!(
        changed(&before_keyboard, &fx.shot()?, copy_icon) > 10,
        "Enter did not activate Copy message"
    );
    advance(&mut fx, steps[6]["waitMs"].as_u64().unwrap());
    fx.hover(target(&steps[2]))?;
    advance(&mut fx, 250);
    fx.shot()?;
    let label = fx
        .cx
        .update(|cx| ui::tooltip::shown(cx).map(|s| s.label.to_string()));
    ensure!(
        label.as_deref() == Some("Copy message"),
        "the copy label did not reset: {label:?}"
    );
    fx.click(target(&steps[7]))?;
    ensure!(
        changed(&rest, &fx.shot()?, actions) == 0,
        "actions remained visible after focus and hover left"
    );
    ensure!(
        fx.sent()?.len() == before,
        "copy sent an app-server request"
    );
    Ok(())
}

fn target(step: &Value) -> (f32, f32) {
    if let (Some(x), Some(y)) = (step["at"]["x"].as_f64(), step["at"]["y"].as_f64()) {
        return (x as f32, y as f32);
    }
    let b = &step["target"]["bbox"];
    let n = |k: &str| b[k].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}

fn advance(fx: &mut Fx, ms: u64) {
    fx.cx.advance_clock(std::time::Duration::from_millis(ms));
    fx.cx.run_until_parked();
}
