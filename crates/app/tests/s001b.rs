//! Replays S001b (RS-087 Pin click) against the original renderer's recording.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::{Fx, ROOT, changed, exchange, proxy_child, script};
use image::RgbaImage;
use serde_json::{Value, json};
use std::{fs, time::Duration};

/// The pinned row, the `Pinned` header and the `Recents` header after the pin (CSS px, from
/// the recording's `dom`).
const PINNED_ROW: [f32; 4] = [60., 203., 272., 30.];
const PINNED_HEADER: [f32; 4] = [68., 174., 46., 21.];
const RECENTS_HEADER: [f32; 4] = [68., 341., 54., 21.];
/// Rest-state buttons after the pin and how far their rows moved: Unpin and Archive on the
/// pinned row (up from y 295 to 208), Pin on the first Recents row (down from y 326 to 375).
const UNPIN: ([f32; 4], f32) = ([276., 208., 20., 20.], -87.);
const ARCHIVE: ([f32; 4], f32) = ([304., 208., 20., 20.], -87.);
const OTHER_PIN: ([f32; 4], f32) = ([276., 375., 20., 20.], 49.);

fn center(b: &Value) -> (f32, f32) {
    let n = |key: &str| b[key].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}

/// Device pixels of `a` inside the CSS-px `rect` that differ from `b` at the rect moved up by
/// `dy` (the row's position before the pin).
fn moved(a: &RgbaImage, b: &RgbaImage, [x, y, w, h]: [f32; 4], dy: f32) -> usize {
    let (x0, y0) = ((x * 2.) as u32, (y * 2.) as u32);
    (y0..((y + h) * 2.) as u32)
        .flat_map(|py| (x0..((x + w) * 2.) as u32).map(move |px| (px, py)))
        .filter(|&(px, py)| a.get_pixel(px, py) != b.get_pixel(px, (py as f32 - dy * 2.) as u32))
        .count()
}

#[test]
fn proxy() {
    proxy_child();
}

/// The recording, the port after the recorded hover and Pin click (once `thread/section/move`
/// left), and the screen before the click.
fn pin(lines: Vec<Value>) -> Result<(Value, Fx, RgbaImage)> {
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s001b.json"))?)?;
    let steps = rec["steps"].as_array().unwrap();
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let initial = fx.shot()?;
    fx.hover(center(&steps[0]["target"]["bbox"]))?;
    fx.advance(Duration::from_millis(300))?;
    fx.click(center(&steps[1]["target"]["bbox"]))?;
    fx.wait("pin request", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|r| r["method"] == "thread/section/move"))
    })?;
    Ok((rec, fx, initial))
}

#[test]
fn pin_moves_the_row_to_pinned_without_a_toast() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/section/move", json!({})));
    let (rec, mut fx, initial) = pin(lines)?;
    let want = &rec["result"]["requests"][2]["params"];
    fx.advance(Duration::from_millis(300))?;
    ensure!(fx.route() == Route::Home, "Pin opened the chat");
    ensure!(
        fx.read(|r, cx| r.toasts(cx)).is_empty(),
        "Pin added an unrecorded toast"
    );
    let pinned = fx.shot()?;
    ensure!(
        changed(&initial, &pinned, PINNED_ROW) > 100,
        "The row did not move under Pinned"
    );
    ensure!(
        changed(&initial, &pinned, PINNED_HEADER) > 50,
        "No Pinned header"
    );
    ensure!(
        changed(&initial, &pinned, RECENTS_HEADER) > 50,
        "Recents did not move below Pinned"
    );

    // Rest: the Unpin icon hides like Pin and Archive (`opacity-0 group-hover:opacity-100`): the
    // row's button corner renders as it did before the pin, text track and all.
    fx.hover((800., 600.))?;
    fx.advance(Duration::from_millis(400))?;
    let rest = fx.shot()?;
    rest.save(format!("{}/s001b.png", env!("CARGO_TARGET_TMPDIR")))?;
    for (name, (rect, dy)) in [("Unpin", UNPIN), ("Archive", ARCHIVE), ("Pin", OTHER_PIN)] {
        ensure!(
            moved(&rest, &initial, rect, dy) == 0,
            "The {name} button corner differs from its rest state before the pin"
        );
    }
    fx.hover((150., 218.))?;
    fx.advance(Duration::from_millis(200))?;
    let hovered = fx.shot()?;
    ensure!(
        moved(&hovered, &initial, UNPIN.0, UNPIN.1) > 20,
        "The Unpin icon did not fade in on hover"
    );

    let sent = fx.sent()?;
    let moves: Vec<_> = sent
        .iter()
        .filter(|r| r["method"] == "thread/section/move")
        .map(|r| r["params"].clone())
        .collect();
    assert_eq!(moves, vec![want.clone()]);
    ensure!(
        !sent.iter().any(|r| r["method"] == "thread/resume"),
        "Pin resumed the chat"
    );
    Ok(())
}

#[test]
fn refused_pin_puts_the_row_back_silently() -> Result<()> {
    // No recorded `thread/section/move`: the replay answers an error.
    let (_, mut fx, initial) = pin(script(None, |_| {})?)?;
    fx.advance(Duration::from_millis(1000))?;
    let after = fx.shot()?;
    ensure!(
        changed(&initial, &after, PINNED_HEADER) == 0 && changed(&initial, &after, PINNED_ROW) == 0,
        "The refused pin left the row under Pinned"
    );
    ensure!(
        fx.read(|r, cx| r.toasts(cx)).is_empty(),
        "The refused pin showed a toast"
    );
    ensure!(fx.route() == Route::Home, "The refused pin opened the chat");
    Ok(())
}
