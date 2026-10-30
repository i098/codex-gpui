#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::json;
use std::{path::Path, process::Command};

#[test]
fn proxy() {
    common::proxy_child();
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        result.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn pending(fx: &Fx) -> Vec<String> {
    fx.read(|root, cx| {
        root.diff_panel()
            .map(|panel| {
                let panel = panel.read(cx);
                panel
                    .artifact_annotations()
                    .iter()
                    .map(|c| c.text())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn drafting(fx: &Fx) -> bool {
    fx.read(|root, cx| {
        root.diff_panel()
            .is_some_and(|panel| panel.read(cx).artifact_annotation_draft().is_some())
    })
}

/// A changed image in Changes takes a region and a point annotation, and Cmd+Enter adds the
/// draft, then sends the batch as one `artifact-direct-comment` turn and clears it.
#[test]
fn image_preview_annotations_send_with_cmd_enter() -> Result<()> {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Annotation proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    let image = root.join("picture.png");
    image::RgbaImage::from_pixel(128, 64, image::Rgba([255, 0, 0, 255])).save(&image)?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "before"])?;
    image::RgbaImage::from_pixel(128, 64, image::Rgba([0, 0, 255, 255])).save(&image)?;
    let patch = git(root, &["diff", "--full-index", "--", "picture.png"])?;
    let mut recording = common::script(None, |_| {})?;
    for row in &mut recording {
        if let Some(thread) = row.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), recording)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    let turn = fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .turns()
            .last()
            .unwrap()
            .id
            .clone()
    });
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "turn/diff/updated".into(),
                params: json!({"threadId":SUBTRACT,"turnId":turn,"diff":patch}),
            },
            cx,
        )
    });
    fx.wait("after image", |fx| {
        fx.read(|root, cx| root.diff_panel()?.read(cx).preview_page(0, 1)) == Some((1, 1))
    })?;
    let evidence = "CGX_ARTIFACT_ANNOTATION_EVIDENCE";
    fx.capture_evidence(evidence, "preview")?;

    // The after image is 128x64 px centered at (1118, 209).
    fx.drag((1070., 190.), (1110., 225.), 4)?;
    fx.wait("region draft", |fx| drafting(fx))?;
    fx.type_text("Darken this blue.")?;
    fx.capture_evidence(evidence, "region-draft")?;
    fx.keys("cmd-enter")?;
    fx.wait("region annotation", |fx| {
        pending(fx) == ["Darken this blue."]
    })?;
    fx.click((1170., 235.))?;
    fx.wait("point draft", |fx| drafting(fx))?;
    fx.type_text("Round this corner.")?;
    fx.keys("escape")?;
    ensure!(
        !drafting(&fx) && pending(&fx).len() == 1,
        "Escape kept the point draft"
    );
    fx.click((1170., 235.))?;
    fx.type_text("Round this corner.")?;
    fx.keys("cmd-enter")?;
    fx.wait("point annotation", |fx| pending(fx).len() == 2)?;
    fx.capture_evidence(evidence, "review-bar")?;

    fx.keys("cmd-enter")?;
    fx.wait("annotation turn", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|request| request["method"] == "turn/start"))
    })?;
    let requests = fx.sent()?;
    let request = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .unwrap();
    let text = request["params"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|input| input["text"].as_str())
        .collect::<String>();
    let path = image.display();
    ensure!(
        text.starts_with("# Diff comments:\n")
            && text.contains("## User Comment 1\nFile: artifact:picture.png\n")
            && text.contains(&format!("Artifact path: {path}\n"))
            && text.contains("Artifact type: image\nAnnotation target: Page 1 region\n")
            && text.contains("Artifact annotation: region on page 1/1 at (")
            && text.contains("Comment:\nDarken this blue.\n")
            && text.contains("## User Comment 2\n")
            && text.contains("Annotation target: Page 1 point\n")
            && text.contains("Comment:\nRound this corner.\n")
            && text.ends_with("## My request:\n"),
        "The annotation turn lost its context: {text}"
    );
    fx.wait("sent batch cleared", |fx| pending(fx).is_empty())?;
    Ok(())
}
