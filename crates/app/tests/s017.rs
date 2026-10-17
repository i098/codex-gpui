//! Replays the original clipboard interactions in `refs/interactions/s017.json`: a large text
//! paste goes to the app-server as the recorded `fs/createDirectory`, `fs/writeFile` (file) and
//! `fs/writeFile` (registry) requests; an image paste sends none and keeps its bytes.

mod common;

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use common::{Fx, ROOT, exchange, proxy_child, script};
use gpui::{ClipboardItem, Focusable as _, Image, ImageFormat};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs,
    path::Path,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};
use ui::composer::Submission;

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn clipboard_attachments_match_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    // The recording's attachment paths are under the route1 app-server's codex home.
    // SAFETY: set before the app and its threads start.
    unsafe { std::env::set_var("CODEX_HOME", "/tmp/cgx-mac/home") };
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s017.json"))?)?;
    // Two text pastes (directory, file, registry) and one removal (registry, file, registry).
    let mut lines = script(None, |_| {})?;
    let pastes = ["fs/createDirectory", "fs/writeFile", "fs/writeFile"].repeat(2);
    for method in pastes
        .into_iter()
        .chain(["fs/writeFile", "fs/remove", "fs/writeFile"])
    {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
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

    // The text paste's three recorded requests (`replay::recordings` compares their params)
    // under a v4 UUID directory, the only name the original's registry reader accepts. The
    // port reads the registry on first use where the original read it at startup; the image
    // paste sent none.
    let sent = fs_requests(&mut fx)?;
    assert_eq!(sent[0]["method"], "fs/readFile");
    let sent = &sent[1..];
    assert_eq!(
        sent.len(),
        rec["result"]["requests"].as_array().unwrap().len()
    );
    let id = directory_id(&sent[0])?;
    ensure!(is_uuid_v4(id), "directory {id} is not a v4 UUID");
    let text_path = submission
        .attachments
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == "Pasted text.txt"))
        .context("text attachment")?;
    assert_eq!(
        Some(text_path.to_string_lossy().as_ref()),
        sent[1]["params"]["path"].as_str()
    );
    let image_path = submission
        .attachments
        .iter()
        .find(|p| p.extension().is_some_and(|n| n == "png"))
        .context("image attachment")?;
    assert_eq!(fs::read(image_path)?, bytes);
    fs::remove_file(image_path)?;
    fs::remove_dir(image_path.parent().unwrap())?;

    // Restoring the card removes its file like the original (registry, file, registry) and
    // must not overflow the shaper's fixed-point pen for a long line.
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
    fx.wait("removal requests", |fx| {
        fs_requests(fx).is_ok_and(|sent| sent.len() >= 10)
    })?;
    let sent = fs_requests(&mut fx)?;
    let methods: Vec<_> = sent[4..]
        .iter()
        .map(|m| m["method"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        [
            "fs/createDirectory",
            "fs/writeFile",
            "fs/writeFile",
            "fs/writeFile",
            "fs/remove",
            "fs/writeFile"
        ]
    );
    assert_eq!(
        sent[8]["params"],
        json!({ "path": sent[5]["params"]["path"], "force": true })
    );
    let data = sent[9]["params"]["dataBase64"].as_str().unwrap();
    let registry = String::from_utf8(BASE64.decode(data)?)?;
    ensure!(
        registry.contains(id) && !registry.contains(directory_id(&sent[4])?),
        "registry after removal: {registry}"
    );
    Ok(())
}

/// The app's `fs/*` requests so far.
fn fs_requests(fx: &mut Fx) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|m| m["method"].as_str().is_some_and(|m| m.starts_with("fs/")))
        .collect())
}

/// The last component of an `fs/createDirectory` path.
fn directory_id(request: &Value) -> Result<&str> {
    request["params"]["path"]
        .as_str()
        .and_then(|p| p.rsplit('/').next())
        .context("directory path")
}

fn is_uuid_v4(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            14 => c == b'4',
            19 => b"89ab".contains(&c),
            _ => c.is_ascii_hexdigit(),
        })
}
