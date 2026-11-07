//! PANEL-18: a file link to an image or PDF opens the Files tab on a preview of it, as the
//! original's image and PDF preview panels do, instead of the text viewer's lines.
mod common;

use anyhow::{Result, ensure};
use common::*;
use std::path::Path;
use ui::screens::diff::panel::OpenFile;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn image_and_pdf_links_open_a_preview() -> Result<()> {
    common::skip_unless_macos!("file_preview");
    let assets = Path::new(CWD).join("preview");
    std::fs::create_dir_all(&assets)?;
    std::fs::write(
        assets.join("avatar.png"),
        include_bytes!("../../ui/assets/avatar-decoded-64.png"),
    )?;
    std::fs::write(
        assets.join("guide.pdf"),
        include_bytes!("../../../fixtures/media/guide.pdf"),
    )?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (name, shot) in [
        ("avatar.png", "file-preview-image.png"),
        ("guide.pdf", "file-preview-pdf.png"),
    ] {
        fx.action(Box::new(OpenFile {
            path: format!("preview/{name}"),
            line: None,
        }))?;
        fx.wait(name, |fx| {
            fx.read(|r, cx| r.file_view().is_some_and(|f| f.read(cx).media_loaded()))
        })?;
        let contents = fx.read(|r, cx| r.file_view().map(|f| f.read(cx).contents.clone()));
        ensure!(
            contents == Some(None),
            "{name} was read as text: {contents:?}"
        );
        fx.shot()?.save(proof.join(shot))?;
    }
    println!("file_preview: shots in {}", proof.display());
    Ok(())
}
