//! MS2 replays the original's sidebar toggle spring (MO-01) from `refs/interactions/ms2.json`.
//! Run on macOS: `cargo test -p app --test ms2`.
//! Each recorded run clicks `Hide sidebar` or `Show sidebar` at the original's bounds, then
//! the main surface's left edge, the first Recents row's shift and the sidebar content's
//! opacity are read from screenshots at the recorded times. The runs align on their first
//! moving frame; each later sample must lie between the port's frames one 60 Hz frame before
//! and after it, within 0.02 of progress (the inventory's spring-fit tolerance). Under
//! reduced motion the first frame after the click shows the end state.
#![cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use image::RgbaImage;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use ui::motion::spring_300;

const FRAME: f64 = 1000. / 60.;
/// Where the main surface's fill starts with the sidebar open (right of its 0.5 px hairline
/// at 340) and closed (1280x832, 340 px sidebar).
const OPEN_X: f64 = 340.5;
const CLOSED_X: f64 = 52.;
/// A row that is plain surface color from the main surface's left edge to the page's right
/// edge (between the `Projects` header and `No projects`).
const MAIN_Y: u32 = 200;
const SIDEBAR_SHIFT: f64 = 16.;
/// Plain sidebar background above its header; the content's fade and shift never reach it.
const SIDEBAR_BG: (u32, u32) = (100, 48);
/// Main surface travel between the states.
const SIDEBAR_TRAVEL: f64 = 288.;

fn rgb(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let p = image.get_pixel(x, y).0;
    [p[0], p[1], p[2]]
}

/// Where the main surface's fill starts, in CSS px: the run of surface color that reaches the
/// page's right edge at [`MAIN_Y`]. The recorded x is the surface's border box, which starts
/// 0.5 px earlier (its hairline) while the page-sidebar shape holds: within the tolerance.
fn main_x(image: &RgbaImage) -> f64 {
    let y = MAIN_Y * 2;
    let right = 1271 * 2;
    let surface = rgb(image, right, y);
    let mut x = right;
    while x > 0 && rgb(image, x - 1, y) == surface {
        x -= 1;
    }
    f64::from(x) / 2.
}

/// `(left ink column, peak ink)` of the first Recents row: the leftmost CSS px whose pixels
/// differ from the sidebar background and the largest channel difference in the row band.
fn row_ink(image: &RgbaImage, row_y: f32) -> (f64, f64) {
    let (y0, y1) = (((row_y - 15.) * 2.) as u32, ((row_y + 15.) * 2.) as u32);
    let background = rgb(image, SIDEBAR_BG.0 * 2, SIDEBAR_BG.1 * 2);
    let ink = |x: u32| {
        (y0..y1)
            .map(|y| {
                let p = rgb(image, x, y);
                (0..3).map(|c| p[c].abs_diff(background[c])).max().unwrap()
            })
            .max()
            .unwrap()
    };
    let left = (52 * 2..330 * 2)
        .find(|&x| ink(x) > 8)
        .map_or(f64::NAN, |x| f64::from(x) / 2.);
    let peak = (52 * 2..330 * 2).map(ink).max().unwrap();
    (left, f64::from(peak))
}

/// The port time at which the spring has moved `progress` of the way (the spring is linear,
/// so both directions share it).
fn spring_time(progress: f64) -> f64 {
    let (mut low, mut high) = (0., 300.);
    for _ in 0..40 {
        let mid = (low + high) / 2.;
        let moved = spring_300(0., 1., Duration::from_secs_f64(mid / 1000.));
        if f64::from(moved) < progress {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.
}

fn center(bbox: &Value) -> (f32, f32) {
    let v = |k: &str| bbox[k].as_f64().unwrap() as f32;
    (v("x") + v("w") / 2., v("y") + v("h") / 2.)
}

struct Frame {
    main_x: f64,
    row_left: f64,
    row_ink: f64,
}

#[test]
fn recorded_sidebar_toggle() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/ms2.json"))?;
    let mut fx = Fx::new()?;
    let row_y = fx.at("home", "Change the greeting")?.1;
    let rest = fx.shot()?;
    ensure!(main_x(&rest) == OPEN_X, "main at rest: {}", main_x(&rest));
    let (row_left_rest, row_ink_rest) = row_ink(&rest, row_y);
    ensure!(row_left_rest.is_finite() && row_ink_rest > 40.);
    for run in recording["runs"].as_array().context("runs")? {
        let name = run["name"].as_str().unwrap();
        let reduced = run["reduced"] == true;
        let collapse = run["action"] == "collapse";
        // The original's surface border box starts at 340 open and 52 closed.
        let recorded_start = if collapse { 340. } else { 52. };
        let (start_x, end_x) = if collapse {
            (OPEN_X, CLOSED_X)
        } else {
            (CLOSED_X, OPEN_X)
        };
        fx.cx.update(|cx| cx.set_reduce_motion(reduced));
        if fx.sidebar_open() != collapse {
            fx.click(common::TOGGLE_SIDEBAR)?;
            fx.advance(Duration::from_millis(400))?;
        }
        ensure!(
            main_x(&fx.shot()?) == start_x,
            "{name}: not at the start state"
        );
        let button = center(&run["button"]["bbox"]);
        let samples = run["samples"].as_array().unwrap();
        let progress =
            |s: &Value| (s["mainX"].as_f64().unwrap() - recorded_start).abs() / SIDEBAR_TRAVEL;
        let moving: Vec<&Value> = samples
            .iter()
            .filter(|s| (0.001..0.999).contains(&progress(s)))
            .collect();
        fx.click(button)?;
        if reduced {
            ensure!(
                moving.is_empty(),
                "{name}: the original moved under reduced motion"
            );
            let first = fx.shot()?;
            ensure!(
                main_x(&first) == end_x,
                "{name}: first frame main x {} != {end_x}",
                main_x(&first)
            );
            continue;
        }
        ensure!(moving.len() >= 2, "{name}: {} moving frames", moving.len());
        // Align: the recorded first moving frame happens at the port time of its progress.
        let offset = moving[0]["ms"].as_f64().unwrap() - spring_time(progress(moving[0]));
        let tick = |t: f64| (t * 10.).round().max(0.) as i64;
        let times: BTreeSet<i64> = moving
            .iter()
            .flat_map(|s| {
                let t = s["ms"].as_f64().unwrap() - offset;
                [tick(t - FRAME), tick(t + FRAME)]
            })
            .collect();
        let mut frames = BTreeMap::new();
        let mut elapsed = 0i64;
        for &t in &times {
            fx.advance(Duration::from_micros(((t - elapsed) * 100) as u64))?;
            elapsed = t;
            let image = fx.shot()?;
            let (row_left, row_ink) = row_ink(&image, row_y);
            frames.insert(
                t,
                Frame {
                    main_x: main_x(&image),
                    row_left,
                    row_ink,
                },
            );
        }
        let at = |t: f64| &frames[&tick(t)];
        let between = |value: f64, a: f64, b: f64, tolerance: f64| {
            (a.min(b) - tolerance..=a.max(b) + tolerance).contains(&value)
        };
        for sample in &moving {
            let t = sample["ms"].as_f64().unwrap() - offset;
            let x = sample["mainX"].as_f64().unwrap();
            let opacity = sample["opacity"].as_f64().unwrap();
            let shift = 60. - sample["row0X"].as_f64().unwrap();
            let (before, after) = (at(t - FRAME), at(t + FRAME));
            ensure!(
                between(x, before.main_x, after.main_x, 0.02 * SIDEBAR_TRAVEL + 1.),
                "{name} at {t:.1} ms: recorded main x {x}, port {}..{}",
                before.main_x,
                after.main_x
            );
            // The sidebar content fades with the progress and slides in from -16 px: its
            // layer alpha scales the row's ink, the shift moves the row's left edge (clipped
            // at the panel's edge).
            let (alpha_before, alpha_after) =
                (before.row_ink / row_ink_rest, after.row_ink / row_ink_rest);
            ensure!(
                between(opacity, alpha_before, alpha_after, 0.02 + 2. / row_ink_rest),
                "{name} at {t:.1} ms: recorded opacity {opacity}, port {alpha_before}..{alpha_after}"
            );
            let left = (row_left_rest - shift).max(52.);
            ensure!(
                opacity < 0.2 || between(left, before.row_left, after.row_left, 1.),
                "{name} at {t:.1} ms: recorded row left {left}, port {}..{}",
                before.row_left,
                after.row_left
            );
            ensure!(
                (shift / SIDEBAR_SHIFT - (1. - opacity)).abs() < 0.02,
                "{name}: the recorded shift and opacity do not share one progress"
            );
        }
        fx.advance(Duration::from_millis(300))?;
        ensure!(
            main_x(&fx.shot()?) == end_x,
            "{name}: did not settle at {end_x}"
        );
    }
    Ok(())
}
