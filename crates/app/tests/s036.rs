//! S036 `autos-list`: AM-005 (the Scheduled sidebar's `New task` row opens the `Schedule a
//! task` create dialog; hover fills the row), replayed from `refs/interactions/s036.json` (the
//! original on the route1 Linux stack, no tasks).

use anyhow::Result;
use serde_json::Value;

const RECORDING: &str = include_str!("../../../refs/interactions/s036.json");

#[test]
fn recording_holds_the_dialog_facts() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let dom = recording["result"]["dom"].as_array().unwrap();
    let open = &dom[2];
    assert_eq!(
        open["dialog"]["bbox"],
        serde_json::json!({"x": 300, "y": 83, "w": 680, "h": 422})
    );
    assert_eq!(open["focused"]["aria"], "Instructions…");
    assert_eq!(open["create"]["disabled"], true);
    assert_eq!(open["overlay"]["backgroundColor"], "rgba(0, 0, 0, 0.5)");
    for closed in [3, 6, 9, 11] {
        let entry = dom.iter().find(|e| e["afterStep"] == closed).unwrap();
        assert!(entry["dialog"].is_null(), "{}", entry["note"]);
        assert!(entry["focused"].is_null(), "{}", entry["note"]);
    }
    assert!(recording["result"].get("requests").is_none());
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

/// The app-server of [`common::Fx::captured`].
#[cfg(target_os = "macos")]
#[test]
fn proxy() {
    common::proxy_child();
}

#[cfg(target_os = "macos")]
#[test]
fn new_task_opens_and_closes_the_schedule_dialog() -> Result<()> {
    use anyhow::{Context as _, ensure};
    use app::Route;
    use common::{Fx, RAIL_SCHEDULED, script};
    use gpui::Focusable as _;
    use std::time::Duration;
    use ui::dialog::Kind;

    let recording: Value = serde_json::from_str(RECORDING)?;
    let steps = recording["steps"].as_array().context("steps")?;
    let dom = recording["result"]["dom"].as_array().context("dom")?;
    let at = |step: &Value| {
        (
            step["at"]["x"].as_f64().unwrap() as f32,
            step["at"]["y"].as_f64().unwrap() as f32,
        )
    };
    let wait = |step: &Value| Duration::from_millis(step["waitMs"].as_u64().unwrap());

    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("scheduled route", |fx| fx.route() == Route::Scheduled)?;
    let before = fx.sent()?.len();

    let check = |fx: &mut Fx, entry: &Value| -> Result<()> {
        let image = fx.shot()?;
        let scale = image.width() as f32 / 1280.;
        for sample in entry["pixels"].as_array().context("pixels")? {
            let (x, y) = (
                sample["x"].as_f64().unwrap() as f32,
                sample["y"].as_f64().unwrap() as f32,
            );
            let expected: [u8; 3] = serde_json::from_value(sample["rgb"].clone())?;
            let actual = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            // Fills match to the unit; the 20 px close glyph's centre is a raster away.
            let tolerance = if (x, y) == (950., 115.) { 40 } else { 1 };
            ensure!(
                (0..3).all(|c| actual[c].abs_diff(expected[c]) <= tolerance),
                "{}: pixel ({x}, {y}) {actual:?} != {expected:?}",
                entry["note"]
            );
        }
        Ok(())
    };
    let dialog_is = |fx: &mut Fx, open: bool, after: &str| -> Result<()> {
        let dialog = fx.read(|root, _| root.dialog().cloned());
        ensure!(
            dialog.is_some() == open,
            "{after}: dialog open = {}",
            dialog.is_some()
        );
        if let Some(dialog) = dialog {
            fx.cx.update_window(fx.window, |_, window, cx| {
                let dialog = dialog.read(cx);
                ensure!(dialog.kind() == Kind::ScheduleTask);
                ensure!(
                    dialog.focus_handle(cx).is_focused(window),
                    "{after}: not focused"
                );
                ensure!(dialog.focused_control() == "Instructions…");
                Ok(())
            })??;
        }
        Ok(())
    };

    // At rest: no fill, no dialog.
    fx.hover((700., 760.))?;
    fx.advance(Duration::from_millis(300))?;
    check(&mut fx, &dom[0])?;
    // Hover fills the row.
    fx.hover(at(&steps[0]))?;
    fx.advance(wait(&steps[0]))?;
    check(&mut fx, &dom[1])?;
    // AM-005: the click opens `Schedule a task` with the instructions editor focused.
    fx.click(at(&steps[1]))?;
    fx.advance(wait(&steps[1]))?;
    fx.hover(at(&steps[2]))?;
    fx.advance(wait(&steps[2]))?;
    dialog_is(&mut fx, true, "open")?;
    check(&mut fx, &dom[2])?;
    ensure!(fx.route() == Route::Scheduled);
    // Escape closes it.
    fx.keys("escape")?;
    fx.advance(wait(&steps[3]))?;
    dialog_is(&mut fx, false, "escape")?;
    check(&mut fx, &dom[3])?;
    // Cancel: hover fill, click closes.
    fx.click(at(&steps[4]))?;
    fx.advance(wait(&steps[4]))?;
    fx.hover(at(&steps[5]))?;
    fx.advance(wait(&steps[5]))?;
    check(&mut fx, &dom[4])?;
    fx.click(at(&steps[6]))?;
    fx.advance(wait(&steps[6]))?;
    dialog_is(&mut fx, false, "cancel")?;
    // `Close dialog`: ghost hover fill, click closes.
    fx.click(at(&steps[7]))?;
    fx.advance(wait(&steps[7]))?;
    fx.hover(at(&steps[8]))?;
    fx.advance(wait(&steps[8]))?;
    check(&mut fx, &dom[6])?;
    fx.click(at(&steps[9]))?;
    fx.advance(wait(&steps[9]))?;
    dialog_is(&mut fx, false, "close")?;
    // A click on the dimmed overlay closes it.
    fx.click(at(&steps[10]))?;
    fx.advance(wait(&steps[10]))?;
    dialog_is(&mut fx, true, "reopen")?;
    fx.click(at(&steps[11]))?;
    fx.advance(wait(&steps[11]))?;
    dialog_is(&mut fx, false, "overlay")?;
    // The port's dialog sends nothing; the original's `editorRequests` are its pickers' reads.
    ensure!(
        fx.sent()?.len() == before,
        "dialog sent an app-server request"
    );
    println!(
        "s036: New task opens the schedule dialog; Escape, Cancel, Close and the overlay close it"
    );
    Ok(())
}
