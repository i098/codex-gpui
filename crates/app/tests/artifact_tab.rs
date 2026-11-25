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

    let evidence = "CGX_ARTIFACT_TAB_EVIDENCE";
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
    fx.capture_evidence(evidence, "artifact-pptx")?;

    if !cfg!(target_os = "macos") {
        println!("artifact_tab: PDF pages require macOS");
        return Ok(());
    }
    fx.action(Box::new(OpenFile {
        path: "preview/guide.pdf".into(),
        line: None,
    }))?;
    fx.wait("guide.pdf", |fx| {
        fx.read(|r, cx| r.file_view().is_some_and(|f| f.read(cx).media_loaded()))
    })?;
    // The PDF replaces the .pptx in the one preview artifact tab, as a file tab would.
    ensure!(
        fx.read(|r, _| matches!(r.side_tabs(), [SidePanel::Artifact(_)])),
        "the PDF did not take over the preview artifact tab"
    );
    fx.capture_evidence(evidence, "artifact-pdf")?;

    // The light page in the dark side panel, below the header's light text.
    let shot = fx.shot()?;
    let scale = shot.width() as f32 / 1280.;
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, pixel) in shot.enumerate_pixels() {
        let (x_px, y_px) = (x as f32 / scale, y as f32 / scale);
        if x_px > 660. && y_px > 100. && pixel.0[..3].iter().all(|&c| c > 200) {
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

#[cfg(target_os = "macos")]
#[path = "artifact_tab/live_host.rs"]
mod live_host;

#[cfg(target_os = "macos")]
#[test]
fn artifact_tab_streams_model_commits_during_the_turn() -> Result<()> {
    use serde_json::json;

    let host = live_host::Host::start(SUBTRACT)?;
    let mut lines = script(None, |_| {})?;
    let turn = json!({"id":"artifact-turn","status":"inProgress","items":[],"error":null});
    lines.extend(exchange("turn/start", json!({"turn":turn})));
    for (method, params) in [
        ("turn/started", json!({"threadId":SUBTRACT,"turn":turn})),
        (
            "item/completed",
            json!({"threadId":SUBTRACT,"turnId":"artifact-turn","item":{
                "type":"mcpToolCall","id":"create-artifact","server":"artifact_session","tool":"js","status":"completed",
                "result":{"content":[],"_meta":{"codex/artifactSession":{
                    "schema":"openai.artifact-session.result/v1","ok":true,
                    "artifact":{"ref":live_host::ARTIFACT,"kind":"spreadsheet","stateVersion":1,"title":"Quarterly Sales"}
                }}}
            }}),
        ),
    ] {
        lines.push(json!({"dir":"recv","msg":{"method":method,"params":params}}));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.cx.update(|cx| {
        cx.set_global(app::ArtifactSessionHost::new(
            host.root().join("host.sock"),
            live_host::TOKEN.into(),
            host.root().into(),
        ));
    });
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.focus_composer()?;
    fx.type_text("Create a quarterly sales workbook.")?;
    fx.keys("enter")?;
    fx.wait("artifact session tab", |fx| {
        matches!(
            fx.read(|root, _| root.side_panel()),
            Some(SidePanel::ArtifactSession(_))
        )
    })?;
    let side = fx.read(|root, _| root.side_panel());
    let panel = |shot: &image::RgbaImage| {
        let (w, h) = shot.dimensions();
        image::imageops::crop_imm(shot, w * 3 / 4, h / 4, w / 4, h / 2).to_image()
    };
    fx.wait("initial rendered workbook", |fx| {
        fx.shot().is_ok_and(|shot| {
            panel(&shot)
                .pixels()
                .filter(|p| p[0] > 220 && p[1] > 220 && p[2] > 220)
                .count()
                > 10_000
        })
    })?;
    let initial = panel(&fx.shot()?);
    fx.capture_evidence("CGX_ARTIFACT_TAB_EVIDENCE", "artifact-live-initial")?;

    host.commit()?;
    fx.wait("rendered workbook after a host commit", |fx| {
        fx.shot().is_ok_and(|shot| {
            panel(&shot)
                .pixels()
                .zip(initial.pixels())
                .filter(|(a, b)| a != b)
                .count()
                > 100
        })
    })?;
    ensure!(
        fx.read(|root, cx| {
            root.side_panel() == side && root.thread().is_some_and(|t| t.read(cx).is_running())
        }),
        "the same artifact tab must update before the turn ends"
    );
    fx.capture_evidence("CGX_ARTIFACT_TAB_EVIDENCE", "artifact-live-updated")?;
    Ok(())
}
