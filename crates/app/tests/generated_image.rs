#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use bridge::Event;
use common::{Fx, GREETING, GREETING_PROMPT, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

const MAGENTA: [u8; 4] = [255, 0, 255, 255];

fn item(fx: &mut Fx, turn: &str, method: &str, status: &str, path: &str) -> Vec<String> {
    let root = fx.root.clone();
    let event = Event::Notification {
        method: method.into(),
        params: json!({ "threadId": GREETING, "turnId": turn, "item": {
            "type": "imageGeneration", "id": "image-item", "status": status,
            "revisedPrompt": "a magenta square", "result": "", "savedPath": path } }),
    };
    fx.cx
        .update(|cx| root.update(cx, |root, cx| root.on_event(event, cx)));
    fx.cx.run_until_parked();
    fx.read(|root, cx| root.thread().unwrap().read(cx).activity_labels(turn, cx))
        .unwrap_or_default()
}

#[test]
fn generated_image_opens_in_the_side_panel() -> Result<()> {
    common::install_panic_hook();
    std::fs::create_dir_all(common::CWD)?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ig_magenta.png");
    image::RgbaImage::from_pixel(256, 256, image::Rgba(MAGENTA)).save(&path)?;
    let path = path.to_string_lossy().into_owned();

    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        script(Some("sessions/greeting.jsonl"), |_| {})?,
    )?;
    fx.type_text(GREETING_PROMPT)?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let turn = fx
        .read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .map(|t| t.id.clone())
        })
        .context("turn")?;
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "before")?;

    let labels = item(&mut fx, &turn, "item/started", "inProgress", &path);
    ensure!(
        labels.iter().any(|l| l == "Generating image..."),
        "{labels:?}"
    );
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "generating")?;
    let labels = item(&mut fx, &turn, "item/completed", "completed", &path);
    ensure!(labels.iter().any(|l| l == "Generated image"), "{labels:?}");
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "generated")?;

    // Click the drawn tile: the center of its magenta pixels, in window points.
    let shot = fx.shot()?;
    let scale = shot.width() as f32 / 1280.;
    let hits: Vec<(u32, u32)> = shot
        .enumerate_pixels()
        .filter(|(_, _, p)| p.0 == MAGENTA)
        .map(|(x, y, _)| (x, y))
        .collect();
    ensure!(hits.len() > 1000, "tile not drawn: {} pixels", hits.len());
    let n = hits.len() as f32;
    let x = hits.iter().map(|h| h.0 as f32).sum::<f32>() / n / scale;
    let y = hits.iter().map(|h| h.1 as f32).sum::<f32>() / n / scale;
    fx.click((x, y))?;
    fx.wait("image tab", |fx| {
        fx.read(|root, _| root.side_panel() == Some(SidePanel::Image))
    })?;
    ensure!(fx.read(|root, _| root.side_tabs().contains(&SidePanel::Image)));
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "side-panel")?;
    Ok(())
}
