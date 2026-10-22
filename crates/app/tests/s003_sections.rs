//! RS-066 replays the recorded local section without changing default Recents paging.
#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use common::{Fx, ROOT, proxy_child, script};
use image::RgbaImage;
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

#[test]
fn proxy() {
    proxy_child();
}

fn section_state(rec: &Value) -> Result<&Value> {
    rec["fixture"]["seed"]["value"]
        .as_object()
        .context("original section account map")?
        .values()
        .next()
        .context("original section account")
}

fn threads(rec: &Value) -> Result<Vec<Value>> {
    let mut lines = script(None, |_| {})?;
    let data = lines
        .iter_mut()
        .find_map(|line| {
            line.pointer_mut("/msg/result/data")
                .filter(|data| data[0]["preview"].is_string())
        })
        .context("recorded thread/list")?
        .as_array_mut()
        .context("thread/list rows")?;
    let template = data[0].clone();
    let state = section_state(rec)?;
    let keys = state["sections"][0]["itemKeys"]
        .as_array()
        .context("original custom section members")?;
    let rows = rec["result"]["states"][1]["rows"]
        .as_array()
        .context("original expanded rows")?;
    for (index, key) in keys.iter().enumerate() {
        let mut thread = template.clone();
        thread["id"] = json!(
            key.as_str()
                .and_then(|key| key.strip_prefix("codex:thread:local:"))
                .context("original local thread key")?
        );
        thread["name"] = rows.get(index).context("original row title")?["title"].clone();
        data.push(thread);
    }
    Ok(lines)
}

fn control(image: &RgbaImage, reference: &Value, out: &Path, tag: &str) -> Result<()> {
    let crop: [u32; 4] = serde_json::from_value(reference["crop"].clone())?;
    let [x, y, width, height] = crop;
    let port = image::imageops::crop_imm(image, x, y, width, height).to_image();
    let screen = if reference["label"] == "Show more" {
        "s003-sections-more"
    } else {
        "s003-sections-less"
    };
    let refs = out.join("references");
    let directory = refs.join(screen);
    fs::create_dir_all(&directory)?;
    let stem = format!("dark@{}x{}", width / 2, height / 2);
    fs::write(
        directory.join(format!("{stem}.png")),
        BASE64.decode(
            reference["pngBase64"]
                .as_str()
                .context("original control PNG")?,
        )?,
    )?;
    fs::write(
        directory.join(format!("{stem}.spec.json")),
        serde_json::to_vec(&reference["spec"])?,
    )?;
    let metrics = parity::diff_ref(&refs, &out.join(tag), screen, "dark", &port)?;
    ensure!(
        metrics.pass,
        "{} differs: outside={} text={:.3}%",
        reference["label"],
        metrics.counts.diff_px_outside_text,
        metrics.counts.text_diff_ratio * 100.,
    );
    Ok(())
}

fn check(fx: &mut Fx, rec: &Value, state: &Value, out: &Path, tag: &str) -> Result<()> {
    let expected = if state["expanded"] == "false" {
        0
    } else {
        state["rows"]
            .as_array()
            .context("original section rows")?
            .len()
    };
    let shown = fx.read(|root, cx| {
        root.sidebar()
            .read(cx)
            .section_shown("custom:s003-sections")
    });
    ensure!(
        shown == Some(expected),
        "{tag}: section shows {shown:?}, expected {expected}"
    );
    let recents = fx.read(|root, cx| root.sidebar().read(cx).recents_shown());
    ensure!(
        recents
            == rec["provenance"]["baselineRows"]
                .as_array()
                .context("original Recents")?
                .len(),
        "{tag}: custom members entered default Recents"
    );
    let image = fx.shot()?;
    image.save(out.join(format!("{tag}.png")))?;
    if expected == 0 {
        for sample in rec["result"]["collapsedPixels"]
            .as_array()
            .context("original hidden rows")?
        {
            let [x, y]: [u32; 2] = serde_json::from_value(sample["at"].clone())?;
            let rgba: [u8; 4] = serde_json::from_value(sample["rgba"].clone())?;
            ensure!(
                image.get_pixel(x, y).0 == rgba,
                "{tag}: a collapsed custom row remains visible"
            );
        }
    } else {
        let label = &state["buttons"][0]["text"];
        let reference = rec["result"]["controls"]
            .as_array()
            .context("original control references")?
            .iter()
            .find(|reference| &reference["label"] == label)
            .context("recorded paging control")?;
        control(&image, reference, out, tag)?;
    }
    Ok(())
}

pub fn replay_recording(rec: &Value) -> Result<()> {
    let out = tempfile::Builder::new()
        .prefix("s003-sections-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))?
        .keep();
    println!("S003_SECTION_OUTPUT {}", out.display());
    let mut fx = Fx::captured("dark", (1280., 832.), threads(rec)?)?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    let state = section_state(rec)?;
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_sections(state, cx))?;
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(600))?;
    let before = fx.sent()?.len();
    check(&mut fx, rec, &rec["result"]["states"][0], &out, "rest")?;
    for (index, step) in rec["steps"]
        .as_array()
        .context("original steps")?
        .iter()
        .enumerate()
    {
        let bbox = &step["target"]["bbox"];
        let number = |key: &str| bbox[key].as_f64().context("recorded click bounds");
        fx.click((
            (number("x")? + number("w")? / 2.) as f32,
            (number("y")? + number("h")? / 2.) as f32,
        ))?;
        fx.advance(Duration::from_millis(
            step["waitMs"].as_u64().context("recorded wait")?,
        ))?;
        fx.hover((1270., 820.))?;
        fx.advance(Duration::from_millis(600))?;
        let state = rec["result"]["states"]
            .as_array()
            .context("original states")?
            .iter()
            .find(|state| state["afterStep"].as_u64() == Some(index as u64))
            .context("original transition state")?;
        check(&mut fx, rec, state, &out, &format!("step-{index}"))?;
    }
    let sent = fx.sent()?;
    ensure!(
        json!(sent[before..]) == rec["result"]["requests"],
        "section paging sent an unrecorded request"
    );
    Ok(())
}

#[test]
fn custom_section_more_less_and_collapse_follow_the_original() -> Result<()> {
    let rec: Value = serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s003_sections.json"
    ))?)?;
    replay_recording(&rec)
}

#[test]
fn section_paging_follows_all_original_input_transitions() -> Result<()> {
    let rec: Value = serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s003_sections.json"
    ))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), threads(&rec)?)?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    let state = section_state(&rec)?;
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_sections(state, cx))?;
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(600))?;
    let before = fx.sent()?.len();
    let baseline = rec["provenance"]["baselineRows"]
        .as_array()
        .context("original Recents")?
        .len();
    for state in rec["result"]["states"]
        .as_array()
        .context("original section states")?
    {
        if let Some(index) = state["afterStep"].as_u64() {
            let step = &rec["steps"][index as usize];
            let bbox = &step["target"]["bbox"];
            let number = |key: &str| bbox[key].as_f64().context("recorded click bounds");
            fx.click((
                (number("x")? + number("w")? / 2.) as f32,
                (number("y")? + number("h")? / 2.) as f32,
            ))?;
            fx.advance(Duration::from_millis(
                step["waitMs"].as_u64().context("recorded wait")?,
            ))?;
            fx.hover((1270., 820.))?;
            fx.advance(Duration::from_millis(600))?;
        }
        let expected = if state["expanded"] == "false" {
            0
        } else {
            state["rows"]
                .as_array()
                .context("original visible section rows")?
                .len()
        };
        let (shown, recents) = fx.read(|root, cx| {
            let sidebar = root.sidebar().read(cx);
            (
                sidebar.section_shown("custom:s003-sections"),
                sidebar.recents_shown(),
            )
        });
        ensure!(
            shown == Some(expected),
            "section rows: {shown:?}, expected {expected}"
        );
        ensure!(
            recents == baseline,
            "section paging changed default Recents"
        );
        println!("S003_SECTION_STATE rows={expected} recents={recents}");
    }
    let sent = fx.sent()?;
    ensure!(
        json!(sent[before..]) == rec["result"]["requests"],
        "section paging sent an unrecorded request"
    );
    Ok(())
}

#[test]
fn recorded_start_front_insert_preserves_custom_row_identity() -> Result<()> {
    use common::{EXPLAIN, GREETING, SUBTRACT};

    let rec: Value = serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s003_sections.json"
    ))?)?;
    let section = &section_state(&rec)?["sections"][0];
    // Real readback rows provide genuine thread/read replies for the native click.
    let state = json!({
        "sections": [{
            "id": section["id"],
            "name": section["name"],
            "itemKeys": [
                format!("codex:thread:local:{GREETING}"),
                format!("codex:thread:local:{SUBTRACT}")
            ]
        }],
        "collapsedSectionIds": []
    });
    let mut started = script(Some("sessions/explain.jsonl"), |_| {})?
        .into_iter()
        .find(|line| {
            line["msg"]["method"] == "thread/started"
                && line["msg"]["params"]["thread"]["id"] == EXPLAIN
        })
        .context("recorded Explain thread/started notification")?;
    let params = started["msg"]["params"].take();
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_sections(&state, cx))?;
    fx.hover((1270., 820.))?;
    fx.advance(Duration::from_millis(600))?;
    let before = fx.shot()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "thread/started".into(),
                params,
            },
            cx,
        )
    });
    fx.advance(Duration::from_millis(600))?;
    ensure!(
        fx.read(|root, _| root.recents())
            .first()
            .map(String::as_str)
            == Some(EXPLAIN),
        "the recorded start event did not insert Explain at the front"
    );
    let after = fx.shot()?;
    let bbox = &rec["result"]["states"][0]["rows"][0]["bbox"];
    let number = |key: &str| bbox[key].as_f64().context("recorded custom row bounds");
    let (x, y, w, h) = (number("x")?, number("y")?, number("w")?, number("h")?);
    ensure!(
        common::changed(&before, &after, [x as f32, y as f32, w as f32, h as f32]) == 0,
        "front insertion changed the visible custom row identity"
    );
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("s003-section-row-identity");
    fs::create_dir_all(&out)?;
    before.save(out.join("before.png"))?;
    after.save(out.join("inserted.png"))?;
    fx.click(((x + w / 2.) as f32, (y + h / 2.) as f32))?;
    fx.wait_thread(GREETING)?;
    fx.shot()?.save(out.join("opened-greeting.png"))?;
    println!("S003_ROW_IDENTITY_OUTPUT {}", out.display());
    Ok(())
}
