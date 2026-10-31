//! A pasted image with a text-only model (`inputModalities` without `image`) blocks the send
//! with the original's `composer.submit.imageInputsUnsupported` reason.

mod common;

use anyhow::{Result, ensure};
use common::{Fx, exchange, proxy_child, script};
use gpui::{ClipboardItem, Focusable as _, Image, ImageFormat};
use serde_json::json;
use std::{cell::Cell, rc::Rc};

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn text_only_model_blocks_image_send() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    // SAFETY: set before the app and its threads start.
    unsafe { std::env::set_var("CODEX_HOME", "/tmp/cgx-mac/home") };
    let mut lines = script(None, |line| {
        if let Some(models) = line["msg"]["result"]["data"].as_array_mut()
            && models.iter().any(|model| model["isDefault"].is_boolean())
        {
            for model in models {
                model["inputModalities"] = json!(["text"]);
            }
        }
    })?;
    for method in ["fs/createDirectory", "fs/writeFile"] {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    let sent = Rc::new(Cell::new(false));
    let flag = sent.clone();
    composer.update(&mut fx.cx, |c, _| {
        c.on_submit(move |_, _, _| flag.set(true))
    });
    let png = include_bytes!("../../../assets/icons/settings-zed.png");
    fx.cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
            ImageFormat::Png,
            png.to_vec(),
        )))
    });
    fx.type_text("Describe this")?;
    fx.keys("cmd-v")?;
    fx.wait("image saved", |fx| {
        fx.composer()
            .read_with(&fx.cx, |c, _| c.upload_status() != Some("Files uploading…"))
    })?;
    fx.capture_evidence("CGX_IMAGE_MODEL_EVIDENCE", "image-text-only-model")?;
    ensure!(
        fx.composer().read_with(&fx.cx, |c, _| c.upload_status())
            == Some("Remove images or switch models to send this message"),
        "a text-only model did not block the image"
    );
    fx.keys("enter")?;
    fx.capture_evidence("CGX_IMAGE_MODEL_EVIDENCE", "image-send-blocked")?;
    ensure!(!sent.get(), "the image was sent to a text-only model");
    Ok(())
}
