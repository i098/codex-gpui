#![cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result};
use std::{env, path::Path, time::Duration};
use ui::screens::plugins::{Action, Resource};

/// Captures the Plugins and Skills directories for the original/native gate.
#[test]
#[ignore = "Requires the CGX_PLUGINS_CAPTURE output directory"]
fn capture_plugins() -> Result<()> {
    common::install_panic_hook();
    let output = env::var("CGX_PLUGINS_CAPTURE").context("CGX_PLUGINS_CAPTURE")?;
    std::fs::create_dir_all(&output)?;
    let mut lines = common::script(None, |_| {})?;
    let skills: serde_json::Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    // Each directory route loads the page again.
    for _ in 0..2 {
        lines.extend(common::exchange("skills/list", skills.clone()));
    }
    let mut fx = common::Fx::open("dark", (1280., 832.), lines)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    let mut failed = Vec::new();
    for (state, route) in [("plugins", "/plugins"), ("skills", "/skills")] {
        let page = fx
            .read(|root, _| root.plugins_page().cloned())
            .context("plugin page")?;
        fx.cx
            .update(|cx| page.update(cx, |page, cx| page.act(Action::Navigate(route.into()), cx)));
        fx.wait(state, |fx| {
            fx.read(|root, cx| {
                root.plugins_page().is_some_and(|page| {
                    let data = &page.read(cx).data;
                    !matches!(data.plugins, Resource::Loading)
                        && !matches!(data.skills, Resource::Loading)
                })
            })
        })?;
        for _ in 0..50 {
            fx.advance(Duration::from_millis(20))?;
            std::thread::sleep(Duration::from_millis(20));
        }
        let image = fx.reference_shot()?;
        image.save(Path::new(&output).join(format!("{state}.png")))?;
        if let Ok(references) = env::var("CGX_PLUGINS_REFS") {
            let metrics = parity::diff_ref(
                Path::new(&references),
                Path::new(&output),
                state,
                "dark",
                &image,
            )?;
            println!(
                "{state}: outside={} text={:.3}% {}",
                metrics.counts.diff_px_outside_text,
                metrics.counts.text_diff_ratio * 100.,
                if metrics.pass { "PASS" } else { "FAIL" }
            );
            if !metrics.pass {
                failed.push(state);
            }
        }
    }
    anyhow::ensure!(failed.is_empty(), "differs from the original: {failed:?}");
    Ok(())
}
