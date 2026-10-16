//! Replays the original clipboard interactions in `refs/interactions/s017.json`.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT};
use gpui::{ClipboardItem, Focusable as _, Image, ImageFormat};
use serde_json::Value;
use std::{
    cell::RefCell,
    fs,
    path::Path,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};
use ui::composer::Submission;

#[test]
fn clipboard_attachments_match_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s017.json"))?)?;
    let mut fx = Fx::new()?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    fx.shot()?;
    let submitted = Rc::new(RefCell::new(None::<Submission>));
    let capture = submitted.clone();
    composer.update(&mut fx.cx, |c, _| {
        c.on_submit(move |s, _, _| *capture.borrow_mut() = Some(s.clone()))
    });
    let text = rec["steps"][0]["clipboard"]["text"]
        .as_str()
        .context("recorded clipboard text")?;
    fx.cx
        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())));
    fx.keys("cmd-v")?;
    assert_eq!(
        fx.composer_text()?,
        rec["result"]["textPaste"]["editor"].as_str().unwrap()
    );
    // The recording retains the data URL and its decoded bytes for a dependency-free replay.
    let bytes: Vec<u8> = serde_json::from_value(rec["steps"][1]["clipboard"]["pngBytes"].clone())?;
    let image = Image::from_bytes(ImageFormat::Png, bytes.clone());
    fx.cx
        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_image(&image)));
    fx.keys("cmd-v")?;
    assert_eq!(
        fx.composer_text()?,
        rec["result"]["imagePaste"]["editor"].as_str().unwrap()
    );
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let start = Instant::now();
    while submitted.borrow().is_none() {
        let frame = fx.shot()?;
        fx.keys("enter")?;
        if submitted.borrow().is_some() {
            frame.save(proof.join("s017-attachments.png"))?;
        }
        ensure!(
            start.elapsed() < Duration::from_secs(10),
            "attachment persistence timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
    let submission = submitted.borrow_mut().take().unwrap();
    assert_eq!(submission.text, "");
    assert_eq!(submission.attachments.len(), 2);
    let text_path = submission
        .attachments
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == "Pasted text.txt"))
        .context("text attachment")?;
    assert_eq!(fs::read_to_string(text_path)?, text);
    let image_path = submission
        .attachments
        .iter()
        .find(|p| p.extension().is_some_and(|n| n == "png"))
        .context("image attachment")?;
    assert_eq!(fs::read(image_path)?, bytes);
    for path in &submission.attachments {
        fs::remove_file(path)?;
        fs::remove_dir(path.parent().unwrap())?;
    }

    // Restoring the card must not overflow the shaper's fixed-point pen for a long line.
    fx.cx
        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())));
    fx.keys("cmd-v")?;
    let start = Instant::now();
    while fx.composer_text()? != text {
        fx.shot()?;
        fx.click((560., 702.))?;
        ensure!(
            start.elapsed() < Duration::from_secs(10),
            "text restoration timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
    fx.shot()?.save(proof.join("s017-restored.png"))?;
    fx.keys("enter")?;
    let restored = submitted
        .borrow_mut()
        .take()
        .context("restored text submission")?;
    assert_eq!(restored.text, text);
    assert!(restored.attachments.is_empty());
    Ok(())
}
