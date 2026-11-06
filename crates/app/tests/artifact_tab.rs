//! PANEL-25: a file link to an artifact opens an artifact tab, not a file tab. A PDF there
//! takes region annotations, and Cmd+Enter sends them as one `artifact-direct-comment` turn.
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::*;
use std::path::Path;
use ui::screens::diff::panel::OpenFile;

#[test]
fn proxy() {
    proxy_child();
}

fn pending(fx: &Fx) -> usize {
    fx.read(|root, cx| {
        root.file_view()
            .map_or(0, |file| file.read(cx).artifact_annotations().len())
    })
}

fn drafting(fx: &Fx) -> bool {
    fx.read(|root, cx| {
        root.file_view()
            .is_some_and(|file| file.read(cx).artifact_annotation_draft().is_some())
    })
}

#[test]
fn artifact_links_open_an_artifact_tab_that_sends_annotations() -> Result<()> {
    let assets = Path::new(CWD).join("preview");
    std::fs::create_dir_all(&assets)?;
    std::fs::write(
        assets.join("guide.pdf"),
        include_bytes!("../../../fixtures/media/guide.pdf"),
    )?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;

    // An Office file has no renderer here: the tab says so and reads nothing.
    fx.action(Box::new(OpenFile {
        path: "preview/deck.pptx".into(),
        line: None,
    }))?;
    ensure!(
        fx.read(|r, _| matches!(r.side_panel(), Some(SidePanel::Artifact(_)))),
        "a .pptx link did not open an artifact tab"
    );
    fx.advance(std::time::Duration::from_millis(50))?;
    ensure!(
        fx.read(|r, cx| r.file_view().map(|f| f.read(cx).contents.is_none())) == Some(true),
        "the .pptx tab read the file"
    );

    if !cfg!(target_os = "macos") {
        println!("artifact_tab: PDF pages require macOS");
        return Ok(());
    }
    let evidence = "CGX_ARTIFACT_TAB_EVIDENCE";
    fx.action(Box::new(OpenFile {
        path: "preview/guide.pdf".into(),
        line: None,
    }))?;
    fx.wait("guide.pdf", |fx| {
        fx.read(|r, cx| r.file_view().is_some_and(|f| f.read(cx).media_loaded()))
    })?;
    ensure!(
        fx.read(|r, _| r
            .side_tabs()
            .iter()
            .filter(|side| matches!(side, SidePanel::Artifact(_)))
            .count())
            == 2,
        "the PDF did not open its own artifact tab"
    );
    fx.capture_evidence(evidence, "artifact-pdf")?;

    // The white page in the dark side panel.
    let shot = fx.shot()?;
    let scale = shot.width() as f32 / 1280.;
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, pixel) in shot.enumerate_pixels() {
        if x as f32 / scale > 660. && pixel.0[..3].iter().all(|&c| c > 245) {
            (left, top) = (left.min(x), top.min(y));
            (right, bottom) = (right.max(x), bottom.max(y));
        }
    }
    ensure!(right > left && bottom > top, "no PDF page painted");
    let at = |fx: f32, fy: f32| {
        (
            (left as f32 + (right - left) as f32 * fx) / scale,
            (top as f32 + (bottom - top) as f32 * fy) / scale,
        )
    };
    fx.drag(at(0.2, 0.1), at(0.6, 0.3), 4)?;
    fx.wait("region draft", |fx| drafting(fx))?;
    fx.type_text("Shorten this heading.")?;
    fx.capture_evidence(evidence, "artifact-draft")?;
    fx.keys("cmd-enter")?;
    fx.wait("annotation added", |fx| pending(fx) == 1 && !drafting(fx))?;
    fx.capture_evidence(evidence, "artifact-review-bar")?;

    fx.keys("cmd-enter")?;
    fx.wait("annotation turn", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|request| request["method"] == "turn/start"))
    })?;
    let requests = fx.sent()?;
    let text = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .and_then(|request| request["params"]["input"].as_array())
        .context("turn/start has no input")?
        .iter()
        .filter_map(|input| input["text"].as_str())
        .collect::<String>();
    ensure!(
        text.contains("File: artifact:guide.pdf\n")
            && text.contains("Artifact type: pdf\nAnnotation target: Page 1 region\n")
            && text.contains("Comment:\nShorten this heading.\n"),
        "the annotation turn lost its context: {text}"
    );
    fx.wait("sent batch cleared", |fx| pending(fx) == 0)?;
    Ok(())
}
