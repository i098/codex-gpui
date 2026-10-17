//! Replays each interaction recording through its dedicated state, pixel, and request checks.
//! `selftest.json` uses the shared step contract below.
//! `REPLAY=<stem>` selects one exact recording; unset or empty checks every recording.
//! Unknown contracts and failed adapters fail the combined result.

mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::Route;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use common::*;
use image::RgbaImage;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn recordings() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("replay: macOS only, skipped");
        return Ok(());
    }
    let filter = std::env::var("REPLAY").unwrap_or_default();
    let mut files = fs::read_dir(format!("{ROOT}/refs/interactions"))?
        .map(|e| e.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|p| p.extension().is_some_and(|e| e == "json") && matches_recording(p, &filter));
    files.sort();
    ensure!(!files.is_empty(), "no recording matches {filter:?}");
    let mut failed = Vec::new();
    for file in &files {
        let name = file
            .file_stem()
            .context("recording name")?
            .to_string_lossy();
        let result = if name == "selftest" {
            run(file)
        } else {
            replay_adapter(file).and_then(|sent| {
                if name == "s017" {
                    let rec: Value = serde_json::from_slice(&fs::read(file)?)?;
                    check_attachment_requests(&rec["result"], &sent)?;
                }
                Ok("dedicated adapter".to_owned())
            })
        };
        match result {
            Ok(note) => println!("PASS {name} {note}"),
            Err(err) => {
                println!("FAIL {name}: {err:#}");
                failed.push(format!("{name}: {err:#}"));
            }
        }
    }
    println!(
        "{} recordings, {} passed, {} failed",
        files.len(),
        files.len() - failed.len(),
        failed.len()
    );
    ensure!(
        failed.is_empty(),
        "{} recordings failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    Ok(())
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
fn incomplete_port_reference_returns_an_error() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    let step = json!({
        "type": "click",
        "portReference": { "screen": "home" }
    });
    let error = run_step(&mut fx, &step).expect_err("a reference needs its text");
    ensure!(error.to_string() == "reference text", "{error:#}");
    Ok(())
}

#[test]
fn unknown_and_recursive_contracts_are_rejected() {
    for name in ["unknown-contract.json", "replay.json"] {
        let error = replay_adapter(Path::new(name)).expect_err("a contract needs an adapter");
        assert!(
            error.to_string().contains("unsupported recording contract"),
            "{error:#}"
        );
    }
}

fn check_attachment_requests(result: &Value, sent: &[Value]) -> Result<()> {
    let count = result["requests"]
        .as_array()
        .context("attachment requests")?
        .len();
    // The dedicated adapter tests restoration after these original paste writes.
    let writes = sent
        .iter()
        .filter(|r| {
            r["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("fs/") && m != "fs/readFile")
        })
        .take(count);
    check_requests(result, writes)
}

#[test]
fn later_restoration_cannot_hide_wrong_attachment_writes() -> Result<()> {
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s017.json"))?;
    let original = rec["result"]["requests"]
        .as_array()
        .context("recorded requests")?;
    let mut sent = original.clone();
    sent[0]["params"]["recursive"] = false.into();
    sent.extend(original.iter().cloned());
    let error = check_attachment_requests(&rec["result"], &sent)
        .expect_err("later writes must not conceal an incorrect initial paste");
    ensure!(
        error.to_string().contains("fs/createDirectory"),
        "{error:#}"
    );
    Ok(())
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

fn run(path: &Path) -> Result<String> {
    let rec: Value = serde_json::from_slice(&fs::read(path)?)?;
    let steps = rec["steps"].as_array().context("no steps")?;
    let view = &rec["viewport"];
    let size = |key, default| view[key].as_f64().map_or(default, |v| v as f32);
    let theme = if view["theme"] == "light" {
        "light"
    } else {
        "dark"
    };
    let mut lines = script(None, |_| {})?;
    for (method, _) in recorded(&rec["result"])
        .iter()
        .filter(|(m, _)| m.starts_with("fs/"))
    {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured(theme, (size("w", 1280.), size("h", 832.)), lines)?;
    let checks = pixel_checks(&rec, steps.len())?;

    let rest = fx.shot()?;
    let before = fx.sent()?.len();
    check_pixels(&mut fx, &rest, &checks, None)?;
    for (i, step) in steps.iter().enumerate() {
        run_step(&mut fx, step).with_context(|| format!("step {i} {step}"))?;
        check_pixels(&mut fx, &rest, &checks, Some(i))?;
    }
    check_state(&mut fx, &rec["result"]["state"])?;
    // The port awaits each app-server answer (real I/O) before its next request: the check
    // runs once the recorded requests are there or nothing new came for 300 ms.
    let mut last = (before, Instant::now());
    fx.wait("the recorded requests", |fx| {
        let sent = fx.sent().unwrap_or_default();
        if sent.len() != last.0 {
            last = (sent.len(), Instant::now());
        }
        check_requests(&rec["result"], sent[before..].iter()).is_ok()
            || last.1.elapsed() > Duration::from_millis(300)
    })?;
    let sent = fx.sent()?.split_off(before);
    check_requests(&rec["result"], sent.iter())?;
    Ok(format!(
        "{} steps, {} pixel checks",
        steps.len(),
        checks.len()
    ))
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
                    reference
                        .get("screen")
                        .and_then(Value::as_str)
                        .context("reference screen")?,
                    reference
                        .get("text")
                        .and_then(Value::as_str)
                        .context("reference text")?,
                )?
            } else {
                target(step)?
            };
            fx.click(at)?;
            if let Some(id) = step["waitThread"].as_str() {
                fx.wait_thread(id)?;
            }
        }
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
                Route::Automation(id) => format!("scheduled?automationId={id}"),
                Route::Mobile => "mobile".to_owned(),
                Route::Archive => "archive".to_owned(),
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

/// `{method, params}` with null fields dropped (the port leaves out what the original sends
/// as `null`, e.g. `cursor`), v4 UUIDs as `<id>` and a `dataBase64` JSON document decoded.
fn normalise(method: &str, params: Option<&Value>) -> (String, Value) {
    fn strip(v: Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.into_iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| match (k.as_str(), &v) {
                        ("dataBase64", Value::String(s)) => (k, decoded(s)),
                        _ => (scrub(&k), strip(v)),
                    })
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.into_iter().map(strip).collect()),
            Value::String(s) => Value::String(scrub(&s)),
            v => v,
        }
    }
    fn decoded(base64: &str) -> Value {
        BASE64
            .decode(base64)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map_or_else(|| Value::String(base64.into()), strip)
    }
    let params = match params {
        // The recorder keeps params as a JSON string.
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    (method.into(), strip(params))
}

/// Every `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` hex id as `<id>`.
fn scrub(s: &str) -> String {
    let is_id = |w: &str| {
        w.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    };
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if rest.get(..36).is_some_and(is_id) {
            out.push_str("<id>");
            rest = &rest[36..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The normalised `result.requests`, in order.
fn recorded(result: &Value) -> Vec<(String, Value)> {
    result["requests"]
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
        .collect()
}

fn check_requests<'a>(result: &Value, sent: impl Iterator<Item = &'a Value>) -> Result<()> {
    let want = recorded(result);
    let got: Vec<_> = sent
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
