//! S011 (TH-127, TH-128, SH-132): replays refs/interactions/s011.json on the port: drag-select
//! the answer paragraph (two lines, code chips), copy it with ⌘C, collapse by click, select
//! backwards, press on the `select-none` disclosure, drag into the gap under the paragraph. The
//! selected text, the clipboard, the highlight pixels and the app-server traffic must match
//! the original (Linux Chrome records Ctrl+C for the Mac's ⌘C).
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::Value;
use std::fs;

#[test]
fn proxy() {
    proxy_child();
}

fn at(v: &Value) -> (f32, f32) {
    (
        v["x"].as_f64().unwrap() as f32,
        v["y"].as_f64().unwrap() as f32,
    )
}

fn text(v: &Value) -> &str {
    v["text"].as_str().unwrap()
}

fn selection(fx: &mut Fx) -> Result<Option<String>> {
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.read(cx)
            .thread()
            .and_then(|t| t.read(cx).selected_text(window, cx))
    })
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

fn drag(fx: &mut Fx, step: &Value) -> Result<()> {
    assert_eq!(step["type"], "drag");
    fx.drag(at(&step["from"]), at(&step["to"]), 12)
}

#[test]
fn selection_and_copy_match_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s011: the Metal renderer requires macOS");
        return Ok(());
    }
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s011.json"))?)?;
    let (steps, result) = (&rec["steps"], &rec["result"]);
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let row = fx.at("home", "In two sentences")?;
    fx.click(row)?;
    fx.wait_thread(EXPLAIN)?;
    let rest = (1270., 820.);
    fx.hover(rest)?;
    let before = fx.shot()?;
    let paragraph =
        ["x", "y", "w", "h"].map(|k| steps[1]["target"]["bbox"][k].as_f64().unwrap() as f32);
    let sent = fx.sent()?.len();

    // TH-127: the paragraph, from its first glyph into its second line.
    drag(&mut fx, &steps[1])?;
    fx.hover(rest)?;
    let shot = fx.shot()?;
    shot.save(format!(
        "{}/s011-paragraph.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    assert_eq!(
        selection(&mut fx)?.as_deref(),
        Some(text(&result["paragraph"]["selection"]))
    );
    let scale = shot.width() as f64 / 1280.;
    for sample in result["paragraphPixels"]
        .as_array()
        .context("paragraphPixels")?
    {
        let (x, y) = (
            (sample["x"].as_f64().unwrap() * scale) as u32,
            (sample["y"].as_f64().unwrap() * scale) as u32,
        );
        let want: [u8; 3] = serde_json::from_value(sample["rgb"].clone())?;
        let got = shot.get_pixel(x, y).0;
        ensure!(
            (0..3).all(|i| got[i].abs_diff(want[i]) <= 2),
            "{}: port {:?} vs original {:?}",
            sample["name"],
            &got[..3],
            want
        );
    }
    ensure!(
        changed(&before, &shot, paragraph) > 2000,
        "the highlight did not paint"
    );

    // TH-128, SH-132: ⌘C copies the selection and keeps it.
    fx.keys("cmd-c")?;
    assert_eq!(
        clipboard(&mut fx).as_deref(),
        Some(text(&result["paragraphClipboard"]))
    );
    assert_eq!(
        selection(&mut fx)?.as_deref(),
        Some(text(&result["afterCopy"]["selection"]))
    );

    // A click outside the text collapses it.
    fx.click(at(&steps[3]["at"]))?;
    fx.hover(rest)?;
    assert_eq!(selection(&mut fx)?, None);
    let collapsed = fx.shot()?;
    ensure!(
        changed(&before, &collapsed, paragraph) == 0,
        "the highlight did not clear"
    );

    // Backwards drag on line 1, past a code chip; ⌘C again.
    drag(&mut fx, &steps[4])?;
    assert_eq!(
        selection(&mut fx)?.as_deref(),
        Some(text(&result["backwards"]["selection"]))
    );
    fx.keys("cmd-c")?;
    assert_eq!(
        clipboard(&mut fx).as_deref(),
        Some(text(&result["backwardsClipboard"]))
    );

    // A drag from the `select-none` disclosure leaves the selection alone.
    drag(&mut fx, &steps[6])?;
    assert_eq!(
        selection(&mut fx)?.as_deref(),
        Some(text(&result["blocks"]["selection"]))
    );

    // Collapse, then from past the end of line 1 into the gap under the paragraph: from the
    // space hanging at the wrap to the paragraph's end. The original goes on into the hidden
    // user-message timestamp, which the port does not select.
    fx.click(at(&steps[7]["at"]))?;
    assert_eq!(selection(&mut fx)?, None);
    drag(&mut fx, &steps[8])?;
    let below = selection(&mut fx)?.context("selection below the paragraph")?;
    let recorded = text(&result["below"]["selection"]);
    ensure!(
        recorded.starts_with(&below) && below.ends_with("README."),
        "below: port {below:?} vs original {recorded:?}"
    );

    let requests: Vec<_> = fx
        .sent()?
        .into_iter()
        .skip(sent)
        .filter(|r| {
            r["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("thread/"))
        })
        .collect();
    ensure!(
        requests.is_empty(),
        "selection sent app-server requests: {requests:?}"
    );
    println!("s011: selection text, clipboard, highlight pixels and traffic match the original");
    Ok(())
}
