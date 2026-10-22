#![cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use std::{env, path::Path, time::Duration};

fn fixture_home(message: &mut Value) {
    if message["msg"]["result"]["userAgent"].is_string() {
        message["msg"]["result"]["codexHome"] =
            json!(env::var("CGX_STORAGE_HOME").expect("owned fixture home"));
    }
}

#[test]
#[ignore = "Requires an owned Codex home and CGX_STORAGE_CAPTURE output directory"]
fn capture_storage() -> Result<()> {
    common::install_panic_hook();
    let output = env::var("CGX_STORAGE_CAPTURE").context("CGX_STORAGE_CAPTURE")?;
    let home = env::var("CGX_STORAGE_HOME").context("CGX_STORAGE_HOME")?;
    anyhow::ensure!(
        Path::new(&home).is_dir(),
        "the owned fixture home is missing"
    );
    std::fs::create_dir_all(&output)?;
    let mut recording = common::script(None, fixture_home)?;
    if let Ok(path) = env::var("CGX_STORAGE_REPLAY") {
        for line in std::fs::read_to_string(path)?.lines() {
            let mut entry: Value = serde_json::from_str(line)?;
            fixture_home(&mut entry);
            recording.push(entry);
        }
    }
    let mut fx = common::Fx::open("dark", (1280., 832.), recording)?;
    fx.keys("cmd-,")?;
    let storage = fx.at("settings", "Storage")?;
    fx.click(storage)?;
    for _ in 0..100 {
        fx.advance(Duration::from_millis(20))?;
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut image = fx.reference_shot()?;
    image.save(Path::new(&output).join("storage.png"))?;
    let mut state = "storage";
    if let Ok(y) = env::var("CGX_STORAGE_REVIEW_Y") {
        let y: f32 = y.parse()?;
        fx.click((1110., y))?;
        for _ in 0..100 {
            fx.advance(Duration::from_millis(20))?;
            std::thread::sleep(Duration::from_millis(20));
        }
        image = fx.reference_shot()?;
        state = "cleanup-review";
        image.save(Path::new(&output).join("cleanup-review.png"))?;
    }
    if let Ok(references) = env::var("CGX_STORAGE_REFS") {
        let metrics = parity::diff_ref(
            Path::new(&references),
            Path::new(&output),
            state,
            "dark",
            &image,
        )?;
        std::fs::write(
            Path::new(&output).join("counts.json"),
            serde_json::to_vec_pretty(&metrics)?,
        )?;
        assert!(
            metrics.pass,
            "Storage differs from the original: {:?}",
            metrics.counts
        );
    }
    Ok(())
}

#[test]
fn deleted_chat_is_not_cached_after_leaving_its_route() -> Result<()> {
    common::install_panic_hook();
    let mut fx = common::Fx::new()?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    fx.cx.update(|cx| {
        sidebar.update(cx, |_, cx| {
            cx.emit(ui::screens::sidebar::OpenThread(common::EXPLAIN.to_owned()));
        });
    });
    fx.wait_thread(common::EXPLAIN)?;
    fx.keys("cmd-,")?;
    let root = fx.root.clone();
    for id in [common::GREETING, common::EXPLAIN] {
        fx.cx.update(|cx| {
            root.update(cx, |root, cx| {
                root.on_event(
                    bridge::Event::Notification {
                        method: "thread/deleted".to_owned(),
                        params: json!({"threadId":id}),
                    },
                    cx,
                );
            });
        });
        assert_eq!(
            fx.thread_id(),
            (id != common::EXPLAIN).then(|| common::EXPLAIN.to_owned())
        );
    }
    Ok(())
}
