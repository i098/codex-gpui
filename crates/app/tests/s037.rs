//! S037 `autos-list`: AM-014 (click a task row selects it and opens its thread) and AM-015
//! (row hover shows the list fill and the `Scheduled task actions` button), replayed from
//! `refs/interactions/s037.json` (the original on the route1 Linux stack, two fixture tasks).

use anyhow::Result;
use app::host::{Host, Request};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../refs/interactions/s037.json");

/// The desktop host of the recording: `list-automations` answers the fixture tasks.
struct Fixture(Value);

impl Host for Fixture {
    fn fetch(&self, request: &Request) -> Result<Value> {
        anyhow::ensure!(
            *request == Request::post("list-automations", None),
            "unexpected host request {request:?}"
        );
        Ok(
            json!({"type": "fetch-response", "responseType": "success", "status": 200,
            "headers": {"content-type": "application/json"}, "bodyJsonString": self.0.to_string()}),
        )
    }
}

#[test]
fn fixture_host_answers_the_recorded_tasks() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let host = Fixture(recording["fixture"].clone());
    let body = app::host::json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    let tasks = ui::screens::automations::tasks(&body);
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, "s037-weekly-status");
    assert_eq!(tasks[0].name, "Weekly status update");
    assert!(host.fetch(&Request::post("inbox-items", None)).is_err());
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn task_row_hover_and_click_match_original() -> Result<()> {
    use anyhow::{Context as _, ensure};
    use app::Route;
    use common::{Fx, RAIL_SCHEDULED};
    use std::{sync::Arc, time::Duration};

    let recording: Value = serde_json::from_str(RECORDING)?;
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(Fixture(recording["fixture"].clone())), cx)
    });
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("host automation list", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    let dom = recording["result"]["dom"].as_array().context("dom")?;
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
            // Fills match to the unit; the 16 px ellipsis glyph's center is a raster away.
            let tolerance = if (x, y) == (310., 192.) { 24 } else { 1 };
            ensure!(
                (0..3).all(|c| actual[c].abs_diff(expected[c]) <= tolerance),
                "{}: pixel ({x}, {y}) {actual:?} != {expected:?}",
                entry["note"]
            );
        }
        Ok(())
    };
    // At rest: no fill, no action button.
    fx.hover((700., 600.))?;
    fx.advance(Duration::from_millis(300))?;
    check(&mut fx, &dom[0])?;
    // AM-015: the hovered row gets the list fill and its action button.
    let at = |step: &Value| {
        (
            step["at"]["x"].as_f64().unwrap() as f32,
            step["at"]["y"].as_f64().unwrap() as f32,
        )
    };
    let steps = recording["steps"].as_array().context("steps")?;
    fx.hover(at(&steps[0]))?;
    fx.advance(Duration::from_millis(steps[0]["waitMs"].as_u64().unwrap()))?;
    check(&mut fx, &dom[1])?;
    // AM-014: the click selects the task and opens its thread.
    fx.click(at(&steps[1]))?;
    fx.advance(Duration::from_millis(steps[1]["waitMs"].as_u64().unwrap()))?;
    ensure!(
        fx.route() == Route::Automation("s037-weekly-status".into()),
        "route {:?}",
        fx.route()
    );
    ensure!(fx.shell(|s| s.can_go_back), "history entry");
    ensure!(fx.shell(|s| s.selected_rail) == Some("scheduled"));
    fx.hover(at(&steps[2]))?;
    fx.advance(Duration::from_millis(steps[2]["waitMs"].as_u64().unwrap()))?;
    check(&mut fx, &dom[2])?;
    // The click sends nothing to the app-server (the recording's requests are page-mount
    // reads); the port's sidebar and thread are local state too.
    println!("s037: row hover fill and actions, click selects the task and opens its thread");
    Ok(())
}
