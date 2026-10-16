//! Replays the original file drag of `refs/interactions/s017b.json` (CP-030) as gpui
//! `FileDropEvent`s: the `Drop to attach` overlay and the drag-active card surface while a
//! file is over the page column, nothing once it leaves, and the dropped files attached.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT};
use gpui::{ExternalPaths, FileDropEvent, Focusable as _, PlatformInput, point, px};
use image::RgbaImage;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};
use ui::composer::Submission;

fn bbox(v: &Value) -> [f32; 4] {
    let n = |k| v[k].as_f64().unwrap() as f32;
    [n("x"), n("y"), n("w"), n("h")]
}

fn point_of(v: &Value) -> (f32, f32) {
    (
        v["x"].as_f64().unwrap() as f32,
        v["y"].as_f64().unwrap() as f32,
    )
}

/// `rgb(r, g, b)` or `rgba(r, g, b, a)`.
fn css(v: &Value) -> [f32; 4] {
    let s = v.as_str().unwrap().trim();
    let inner = s
        .strip_suffix(')')
        .and_then(|s| s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")))
        .unwrap_or_else(|| panic!("not rgb(): {s}"));
    let n: Vec<f32> = inner
        .split(',')
        .map(|n| n.trim().parse().unwrap())
        .collect();
    [n[0], n[1], n[2], n.get(3).copied().unwrap_or(1.)]
}

/// The most common colour of `shot` inside the CSS-px `bbox` (as `replay.rs`).
fn dominant(shot: &RgbaImage, [x, y, w, h]: [f32; 4]) -> [u8; 3] {
    let k = shot.width() as f32 / 1280.;
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for py in ((y * k) as u32)..(((y + h) * k) as u32).min(shot.height()) {
        for px in ((x * k) as u32)..(((x + w) * k) as u32).min(shot.width()) {
            let p = shot.get_pixel(px, py).0;
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|&(_, n)| n).unwrap().0
}

fn over([r, g, b, a]: [f32; 4], back: [u8; 3]) -> [f32; 3] {
    let mut out = [r, g, b];
    for (c, back) in out.iter_mut().zip(back) {
        *c = *c * a + back as f32 * (1. - a);
    }
    out
}
fn close(got: [u8; 3], want: [f32; 3]) -> bool {
    got.iter()
        .zip(want)
        .all(|(g, w)| (*g as f32 - w).abs() <= 2.)
}

fn drop_event(event: FileDropEvent) -> PlatformInput {
    PlatformInput::FileDrop(event)
}

fn entered(at: (f32, f32), paths: &[PathBuf]) -> PlatformInput {
    drop_event(FileDropEvent::Entered {
        position: point(px(at.0), px(at.1)),
        paths: ExternalPaths(paths.iter().cloned().collect()),
    })
}

fn pending(at: (f32, f32)) -> PlatformInput {
    drop_event(FileDropEvent::Pending {
        position: point(px(at.0), px(at.1)),
    })
}

#[test]
fn file_drag_over_the_composer_matches_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s017b.json"))?)?;
    let steps = rec["steps"].as_array().context("steps")?;
    let home = &rec["result"]["home"];
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR")).join("s017b");
    fs::create_dir_all(&proof)?;
    let notes = proof.join("notes.txt");
    fs::write(&notes, "hello from s017b\n")?;
    let dot = proof.join("dot.png");
    let png: Vec<u8> = serde_json::from_value(steps[3]["files"][0]["pngBytes"].clone())?;
    fs::write(&dot, &png)?;

    let mut fx = Fx::new()?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    let rest = fx.shot()?;
    let overlay = bbox(&home["overlay"]["bbox"]);
    let pill = bbox(&home["overlay"]["pill"]["bbox"]);
    let card = bbox(&home["cardDrag"]["bbox"]);
    let page_rest = dominant(&rest, overlay);
    let card_rest = dominant(&rest, card);
    ensure!(
        close(
            card_rest,
            over(css(&home["rest"]["computed"]["backgroundColor"]), [0; 3])
        ),
        "composer surface at rest: {card_rest:?}"
    );

    // Step 0: the file enters over the sidebar and moves onto the editor.
    let (from, to) = (point_of(&steps[0]["from"]), point_of(&steps[0]["to"]));
    fx.input(entered(from, std::slice::from_ref(&notes)))?;
    let outside = fx.shot()?;
    ensure!(
        dominant(&outside, overlay) == page_rest,
        "the overlay must not show while the file is over the sidebar"
    );
    fx.input(pending(to))?;
    let dragging = fx.shot()?;
    dragging.save(proof.join("drag.png"))?;
    let tint = css(&home["overlay"]["rgba"]);
    let want_page = over(tint, page_rest);
    let got_page = dominant(&dragging, overlay);
    ensure!(
        close(got_page, want_page),
        "overlay over the page column: port {got_page:?}, original {want_page:?}"
    );
    let got_pill = dominant(&dragging, pill);
    let want_pill = css(&home["overlay"]["pill"]["computed"]["backgroundColor"]);
    ensure!(
        close(got_pill, [want_pill[0], want_pill[1], want_pill[2]]),
        "`Drop to attach` pill fill: port {got_pill:?}, original {want_pill:?}"
    );
    let text = css(&home["overlay"]["pill"]["computed"]["color"]);
    let k = dragging.width() as f32 / 1280.;
    let light_px = (((pill[1] * k) as u32)..(((pill[1] + pill[3]) * k) as u32))
        .flat_map(|y| {
            (((pill[0] * k) as u32)..(((pill[0] + pill[2]) * k) as u32)).map(move |x| (x, y))
        })
        .filter(|&(x, y)| {
            let p = dragging.get_pixel(x, y).0;
            (p[0] as f32 - text[0]).abs() < 40. && (p[1] as f32 - text[1]).abs() < 40.
        })
        .count();
    ensure!(
        light_px > 60,
        "the pill shows its text: {light_px} text-coloured px"
    );
    // The card surface drops to 50% under the overlay (`data-composer-drag-active`).
    let want_card = over(
        tint,
        over(css(&home["cardDrag"]["rgba"]), page_rest).map(|c| c.round() as u8),
    );
    let got_card = dominant(&dragging, card);
    ensure!(
        close(got_card, want_card),
        "drag-active composer surface: port {got_card:?}, original {want_card:?}"
    );

    // Step 1: the file leaves the page column, then the window.
    fx.input(pending(point_of(&steps[1]["to"])))?;
    let left = fx.shot()?;
    ensure!(
        dominant(&left, overlay) == page_rest,
        "overlay after dragleave"
    );
    ensure!(
        dominant(&left, card) == card_rest,
        "composer surface after dragleave"
    );
    fx.input(pending(to))?;
    ensure!(
        dominant(&fx.shot()?, overlay) != page_rest,
        "overlay comes back"
    );
    fx.input(drop_event(FileDropEvent::Exited))?;
    fx.input(drop_event(FileDropEvent::Ended))?;
    let exited = fx.shot()?;
    ensure!(
        dominant(&exited, overlay) == page_rest,
        "overlay after the drag left the window"
    );
    ensure!(
        dominant(&exited, card) == card_rest,
        "composer surface after the drag left the window"
    );

    // Steps 2 and 3: drops attach the text file and the image at their own paths.
    let submitted = Rc::new(RefCell::new(None::<Submission>));
    let capture = submitted.clone();
    composer.update(&mut fx.cx, |c, _| {
        c.on_submit(move |s, _, _| *capture.borrow_mut() = Some(s.clone()))
    });
    for (step, path) in [(&steps[2], &notes), (&steps[3], &dot)] {
        let to = point_of(&step["to"]);
        fx.input(entered(point_of(&step["from"]), std::slice::from_ref(path)))?;
        fx.input(pending(to))?;
        fx.input(drop_event(FileDropEvent::Submit {
            position: point(px(to.0), px(to.1)),
        }))?;
        fx.input(drop_event(FileDropEvent::Ended))?;
        let dropped = fx.shot()?;
        ensure!(
            dominant(&dropped, overlay) == page_rest,
            "overlay after the drop"
        );
    }
    let attached = fx.shot()?;
    attached.save(proof.join("attached.png"))?;
    // The chips sit above the editor; the card grows upward from its bottom anchor.
    ensure!(
        common::changed(&rest, &attached, [441., 560., 736., 252.]) > 2000,
        "the attachments row shows the dropped files"
    );
    fx.keys("enter")?;
    let submission = submitted.borrow_mut().take().context("submission")?;
    assert_eq!(submission.text, "");
    assert_eq!(submission.attachments, vec![notes.clone(), dot.clone()]);
    ensure!(
        notes.exists() && dot.exists(),
        "dropped files stay where they are"
    );
    Ok(())
}
