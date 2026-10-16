//! Replays the route1 recordings of the original app (`refs/interactions/*.json`, format in
//! the interaction inventory §5) on the headless [`app::Root`] and checks that the port ends
//! up in the same state:
//!
//! - **pixels**: for each `result.dom` entry with `bbox` and `computed.backgroundColor`, the
//!   most common colour of the port's window inside the bbox must equal that colour blended
//!   over its backdrop (the port's own pixels there before the first step, or the entry's
//!   `backdrop` css colour). The entry is checked before the first step when its `state` is
//!   `rest`, else after step `afterStep` (default: the last). Mode, not the centre pixel,
//!   because text and icons sit on the centre of most boxes.
//! - **state**: `result.state` (port-only keys, see [`check_state`]), after the last step.
//! - **bridge**: the recorded `result.requests` that have a `method` must be sent by the
//!   port during the steps, in that order (exactly those with `result.requestsExact`).
//!
//! Steps carry a position (`at`, else the centre of `target.bbox`); `key` steps may carry a
//! `portKey` (gpui syntax, e.g. `cmd-a`) where the recorder's Linux Chrome used another chord.
//! `REPLAY=<stem>` selects one exact file stem (e.g. `f2a`); unset or empty runs all scripts.
//! Recorder determinism fixtures contain `runs` instead of `steps` and report `SKIP`.

mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::Route;
use common::*;
use image::RgbaImage;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn recordings() {
    // The headless renderer is Metal and the text system is CoreText, like `flows.rs`.
    if !cfg!(target_os = "macos") {
        println!("replay: macOS only, skipped");
        return;
    }
    let filter = std::env::var("REPLAY").unwrap_or_default();
    let mut files: Vec<_> = fs::read_dir(format!("{ROOT}/refs/interactions"))
        .expect("refs/interactions")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter(|p| matches_recording(p, &filter))
        .collect();
    files.sort();
    let mut failed = Vec::new();
    for file in &files {
        let name = file.file_stem().unwrap().to_string_lossy();
        match run(file) {
            Ok(Some(note)) => println!("PASS {name} {note}"),
            Ok(None) => println!("SKIP {name}: recorder determinism bundle, checked by f0a"),
            Err(err) => {
                println!("FAIL {name}: {err:#}");
                failed.push(format!("{name}: {err:#}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

fn matches_recording(path: &Path, filter: &str) -> bool {
    filter.is_empty() || path.file_stem().is_some_and(|stem| stem == filter)
}

#[test]
fn recording_filter_matches_only_the_exact_stem() {
    let files = [
        Path::new("/lanes/f2a/refs/interactions/f0a.json"),
        Path::new("/lanes/f2a/refs/interactions/f2a.json"),
        Path::new("/lanes/f2a/refs/interactions/f2a-extra.json"),
    ];
    let selected: Vec<_> = files
        .iter()
        .copied()
        .filter(|p| matches_recording(p, "f2a"))
        .collect();
    assert_eq!(selected, vec![files[1]]);
    assert!(files.iter().all(|p| matches_recording(p, "")));
    assert!(!matches_recording(files[1], "/f2a.json"));
}

#[test]
fn recorder_determinism_fixture_is_skipped() {
    assert_eq!(
        run(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../refs/interactions/f0a.json"
        )))
        .unwrap(),
        None
    );
}

/// A `result.dom` entry whose background colour the port must show.
struct Pixels {
    selector: String,
    /// The step after which it holds; `None`: before the first.
    after: Option<usize>,
    bbox: [f32; 4],
    color: [f32; 4],
    backdrop: Option<[u8; 3]>,
}

fn run(path: &Path) -> Result<Option<String>> {
    let rec: Value = serde_json::from_slice(&fs::read(path)?)?;
    if rec.get("steps").is_none() && rec["runs"].is_array() {
        return Ok(None);
    }
    let steps = rec["steps"].as_array().context("no steps")?;
    let view = &rec["viewport"];
    let size = |key, default| view[key].as_f64().map_or(default, |v| v as f32);
    let theme = if view["theme"] == "light" {
        "light"
    } else {
        "dark"
    };
    let mut fx = Fx::captured(
        theme,
        (size("w", 1280.), size("h", 832.)),
        script(None, |_| {})?,
    )?;
    let checks = pixel_checks(&rec, steps.len())?;

    let rest = fx.shot()?;
    let before = fx.sent()?.len();
    check_pixels(&mut fx, &rest, &checks, None)?;
    for (i, step) in steps.iter().enumerate() {
        run_step(&mut fx, step).with_context(|| format!("step {i} {step}"))?;
        check_pixels(&mut fx, &rest, &checks, Some(i))?;
    }
    check_state(&mut fx, &rec["result"]["state"])?;
    let sent = fx.sent()?.split_off(before);
    check_requests(&rec["result"], &sent)?;
    Ok(Some(format!(
        "{} steps, {} pixel checks",
        steps.len(),
        checks.len()
    )))
}

// --- Steps ---

fn run_step(fx: &mut Fx, step: &Value) -> Result<()> {
    let kind = step["type"].as_str().context("no type")?;
    match kind {
        "hover" => fx.hover(target(step)?)?,
        "click" if step["button"] == "right" => fx.right_click(target(step)?)?,
        "click" => {
            let at = if let Some(reference) = step["portReference"].as_object() {
                fx.at(
                    reference["screen"].as_str().context("reference screen")?,
                    reference["text"].as_str().context("reference text")?,
                )?
            } else {
                target(step)?
            };
            fx.click(at)?;
            if let Some(id) = step["waitThread"].as_str() {
                fx.wait_thread(id)?;
            }
        }
        "key" => fx.keys(&port_key(step)?)?,
        "type" => fx.type_text(step["text"].as_str().context("no text")?)?,
        "bridge" => {
            let event = bridge::Event::Notification {
                method: step["method"]
                    .as_str()
                    .context("notification method")?
                    .into(),
                params: step["params"].clone(),
            };
            fx.root
                .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
            fx.cx.run_until_parked();
        }
        // The DOM's positive deltaY scrolls down; gpui's negative delta does.
        "scroll" | "wheel" => fx.scroll(
            target(step)?,
            -step["deltaY"].as_f64().context("no deltaY")? as f32,
        )?,
        "drag" => {
            let from = point_of(&step["from"]).map_or_else(|| target(step), Ok)?;
            let to = point_of(&step["to"]).context("no to")?;
            fx.drag(from, to, step["moves"].as_u64().unwrap_or(10) as u32)?
        }
        "resize" => {
            let viewport = step["viewport"].as_str().context("no viewport")?;
            let (w, h) = viewport.split_once('x').context("viewport is WxH")?;
            fx.resize(w.parse()?, h.parse()?)?
        }
        _ => bail!("unsupported step type {kind:?}"),
    }
    if let Some(ms) = step["waitMs"].as_u64() {
        fx.cx.advance_clock(Duration::from_millis(ms));
        fx.cx.run_until_parked();
    }
    Ok(())
}

fn point_of(v: &Value) -> Option<(f32, f32)> {
    Some((v["x"].as_f64()? as f32, v["y"].as_f64()? as f32))
}

fn bbox_of(v: &Value) -> Option<[f32; 4]> {
    let n = |k: &str| v[k].as_f64().map(|n| n as f32);
    Some([n("x")?, n("y")?, n("w")?, n("h")?])
}

/// Where the step points: `at`, else the centre of `target.bbox`.
fn target(step: &Value) -> Result<(f32, f32)> {
    if let Some(at) = point_of(&step["at"]) {
        return Ok(at);
    }
    let [x, y, w, h] = bbox_of(&step["target"]["bbox"]).context("no `at` and no target.bbox")?;
    Ok((x + w / 2., y + h / 2.))
}

/// `Meta+Shift+k` as gpui's `cmd-shift-k`, unless the step has a `portKey`.
fn port_key(step: &Value) -> Result<String> {
    if let Some(key) = step["portKey"].as_str() {
        return Ok(key.into());
    }
    let chord = step["key"].as_str().context("no key")?;
    let (mods, key) = chord.rsplit_once('+').unwrap_or(("", chord));
    let key = match key {
        "ArrowUp" => "up".to_owned(),
        "ArrowDown" => "down".to_owned(),
        "ArrowLeft" => "left".to_owned(),
        "ArrowRight" => "right".to_owned(),
        " " | "Space" => "space".to_owned(),
        "Escape" | "Enter" | "Backspace" | "Delete" | "Tab" | "Home" | "End" | "PageUp"
        | "PageDown" => key.to_lowercase(),
        k if k.chars().count() == 1 => k.to_lowercase(),
        k => bail!("unsupported key {k:?}"),
    };
    let mut out = String::new();
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        out += match m {
            "Meta" | "Cmd" | "Command" => "cmd-",
            "Control" | "Ctrl" => "ctrl-",
            "Alt" | "Option" => "alt-",
            "Shift" => "shift-",
            m => bail!("unsupported modifier {m:?}"),
        };
    }
    Ok(out + &key)
}

// --- Pixels ---

fn pixel_checks(rec: &Value, steps: usize) -> Result<Vec<Pixels>> {
    let mut checks = Vec::new();
    for entry in rec["result"]["dom"].as_array().into_iter().flatten() {
        // Entries without a box or a plain background (marquee, tooltips) are not pixel facts.
        let (Some(bbox), Some(css)) = (
            bbox_of(&entry["bbox"]),
            entry["computed"]["backgroundColor"].as_str(),
        ) else {
            continue;
        };
        let after = match (entry["afterStep"].as_u64(), entry["state"] == "rest") {
            (Some(i), _) => Some(i as usize),
            (None, true) => None,
            (None, false) => steps.checked_sub(1),
        };
        ensure!(
            after.is_none_or(|i| i < steps),
            "afterStep out of range: {entry}"
        );
        checks.push(Pixels {
            selector: entry["selector"].as_str().unwrap_or("?").into(),
            after,
            bbox,
            color: css_color(css)?,
            backdrop: entry["backdrop"]
                .as_str()
                .map(css_color)
                .transpose()?
                .map(|c| [c[0] as u8, c[1] as u8, c[2] as u8]),
        });
    }
    Ok(checks)
}

/// `rgb(r, g, b)` or `rgba(r, g, b, a)` as `[r, g, b, a]`.
fn css_color(css: &str) -> Result<[f32; 4]> {
    let inner = css
        .trim()
        .strip_suffix(')')
        .and_then(|s| s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")))
        .with_context(|| format!("not an rgb()/rgba() colour: {css:?}"))?;
    let n: Vec<f32> = inner
        .split(',')
        .map(|n| n.trim().parse())
        .collect::<Result<_, _>>()?;
    ensure!(n.len() == 3 || n.len() == 4, "colour {css:?}");
    Ok([n[0], n[1], n[2], n.get(3).copied().unwrap_or(1.)])
}

/// The most common colour of `shot` inside the CSS-px `bbox`.
fn dominant(shot: &RgbaImage, size: (f32, f32), [x, y, w, h]: [f32; 4]) -> [u8; 3] {
    let k = shot.width() as f32 / size.0;
    let (x0, y0) = ((x * k) as u32, (y * k) as u32);
    let (x1, y1) = (((x + w) * k) as u32, ((y + h) * k) as u32);
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for py in y0..y1.min(shot.height()) {
        for px in x0..x1.min(shot.width()) {
            let p = shot.get_pixel(px, py).0;
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map_or([0; 3], |(c, _)| c)
}

/// The checks that hold after step `after`, against the port's window now.
fn check_pixels(
    fx: &mut Fx,
    rest: &RgbaImage,
    checks: &[Pixels],
    after: Option<usize>,
) -> Result<()> {
    let due: Vec<_> = checks.iter().filter(|c| c.after == after).collect();
    if due.is_empty() {
        return Ok(());
    }
    let shot = fx.shot()?;
    for c in due {
        let backdrop = c
            .backdrop
            .unwrap_or_else(|| dominant(rest, fx.size, c.bbox));
        let [r, g, b, a] = c.color;
        let want = [r, g, b]
            .into_iter()
            .zip(backdrop)
            .map(|(f, back)| (f * a + back as f32 * (1. - a)).round())
            .collect::<Vec<_>>();
        let got = dominant(&shot, fx.size, c.bbox);
        let off = want.iter().zip(got).any(|(w, g)| (w - g as f32).abs() > 2.);
        ensure!(
            !off,
            "{} {:?} after step {after:?}: port shows rgb{got:?}, recorded rgba({r}, {g}, {b}, {a}) over rgb{backdrop:?} is rgb{want:?}",
            c.selector,
            c.bbox
        );
    }
    Ok(())
}

// --- State ---

/// `result.state`: port-side facts after the last step. Keys: `route` (`home`, `settings`,
/// `scheduled`, `mobile`, `thread:<id>`), `selectedRail` (id or null), `sidebarOpen`,
/// `canGoBack`, `canGoForward`, `commandMenu` (open), `composerFocused`. A slice that needs
/// another fact adds its key here.
fn check_state(fx: &mut Fx, state: &Value) -> Result<()> {
    for (key, want) in state.as_object().into_iter().flatten() {
        let got: Value = match key.as_str() {
            "route" => match fx.route() {
                Route::Home => "home".to_owned(),
                Route::Thread(id) => format!("thread:{id}"),
                Route::Settings => "settings".to_owned(),
                Route::Scheduled => "scheduled".to_owned(),
                Route::Mobile => "mobile".to_owned(),
            }
            .into(),
            "selectedRail" => fx.shell(|s| s.selected_rail).into(),
            "sidebarOpen" => fx.sidebar_open().into(),
            "canGoBack" => fx.shell(|s| s.can_go_back).into(),
            "canGoForward" => fx.shell(|s| s.can_go_forward).into(),
            "commandMenu" => fx.read(|r, _| r.command_menu().is_some()).into(),
            "composerFocused" => fx.composer_focused()?.into(),
            "diffFiles" => fx.read(|root, cx| {
                serde_json::json!(
                    root.diff_panel()
                        .map(|panel| panel
                            .read(cx)
                            .files
                            .iter()
                            .map(|file| (
                                format!("{}{}", file.dir, file.name),
                                file.added,
                                file.removed
                            ))
                            .collect::<Vec<_>>())
                        .unwrap_or_default()
                )
            }),
            "diffSearch" => fx.read(|root, cx| {
                serde_json::json!(root.diff_panel().and_then(|panel| panel.read(cx).search()))
            }),
            _ => bail!("unknown state key {key:?}"),
        };
        ensure!(&got == want, "state {key}: port {got}, expected {want}");
    }
    Ok(())
}

// --- Bridge ---

/// `{method, params}` with null fields dropped: the port leaves out what the original sends
/// as `null` (`cursor`).
fn normalise(method: &str, params: Option<&Value>) -> (String, Value) {
    fn strip(v: Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.into_iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| (k, strip(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.into_iter().map(strip).collect()),
            v => v,
        }
    }
    let params = match params {
        // The recorder keeps params as a JSON string.
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    (method.into(), strip(params))
}

fn check_requests(result: &Value, sent: &[Value]) -> Result<()> {
    let want: Vec<_> = result["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| match r {
            Value::String(s) => {
                let (method, params) = s.split_once(' ').unwrap_or((s, "null"));
                Some(normalise(method, Some(&Value::String(params.into()))))
            }
            r => Some(normalise(r["method"].as_str()?, r.get("params"))),
        })
        .collect();
    let got: Vec<_> = sent
        .iter()
        .map(|m| normalise(m["method"].as_str().unwrap_or("?"), m.get("params")))
        .collect();
    if result["requestsExact"] == true {
        ensure!(got == want, "port sent {got:?}, recorded {want:?}");
    } else {
        let mut rest = got.iter();
        for w in &want {
            ensure!(
                rest.any(|g| g == w),
                "port did not send {w:?} (in order); it sent {got:?}"
            );
        }
    }
    Ok(())
}
