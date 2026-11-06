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
    // The search state answers one `plugin/search` page from the recorded catalog.
    let catalog: serde_json::Value = ui::fixture::response("readback.jsonl", "plugin/list", 0);
    let mut found = Vec::new();
    for market in catalog["marketplaces"].as_array().into_iter().flatten() {
        for plugin in market["plugins"].as_array().into_iter().flatten() {
            if plugin["name"]
                .as_str()
                .is_some_and(|name| name.contains('g'))
            {
                found.push(serde_json::json!({
                    "marketplaceName": market["name"],
                    "marketplacePath": market["path"],
                    "plugin": plugin,
                }));
            }
        }
    }
    found.truncate(6);
    lines.extend(common::exchange(
        "plugin/search",
        serde_json::json!({"data": found, "nextCursor": "page-2"}),
    ));
    let mut fx = common::Fx::open("dark", (1280., 832.), lines)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    // The directory search: a query on the Plugins tab shows the server's page with `See more`.
    // This runs before the other routes so the page keeps its first `plugin/list` read.
    fx.wait("plugins", |fx| {
        fx.read(|root, cx| {
            root.plugins_page().is_some_and(|page| {
                let data = &page.read(cx).data;
                !matches!(data.plugins, Resource::Loading)
                    && !matches!(data.skills, Resource::Loading)
            })
        })
    })?;
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("plugin page")?;
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.query.update(cx, |query, cx| query.set_text("g", cx))
        })
    });
    fx.advance(Duration::from_millis(250))?;
    fx.wait("plugin search", |fx| {
        fx.read(|_, cx| {
            page.read(cx)
                .search
                .as_ref()
                .is_some_and(|search| matches!(search.results, Resource::Ready(_)))
        })
    })?;
    for _ in 0..50 {
        fx.advance(Duration::from_millis(20))?;
        std::thread::sleep(Duration::from_millis(20));
    }
    fx.reference_shot()?
        .save(Path::new(&output).join("plugins-search.png"))?;
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.query.update(cx, |query, cx| query.set_text("", cx))
        })
    });
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
