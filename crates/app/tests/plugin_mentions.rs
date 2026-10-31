//! The `@` list's `Plugins` section lists the installed, enabled plugins of `plugin/list`;
//! picking one inserts a chip and sends `[@title](plugin://id)`, as the original does.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, exchange, script};
use gpui::Focusable as _;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    common::proxy_child();
}

fn plugin(id: &str, title: &str, installed: bool) -> serde_json::Value {
    json!({
        "id": id, "name": id, "installed": installed, "enabled": installed,
        "interface": { "displayName": title, "shortDescription": format!("{title} plugin") },
    })
}

#[test]
fn mention_lists_and_inserts_plugins() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let list = json!({ "marketplaces": [{ "name": "openai-curated", "plugins": [
        plugin("github@openai-curated", "GitHub", true),
        plugin("gmail@openai-curated", "Gmail", false),
    ] }] });
    let mut lines = script(None, |_| {})?;
    // The readback's own `plugin/list` would answer first (the replay falls back to recording
    // order), so this list replaces it.
    lines.retain(|l| l["msg"]["method"] != "plugin/list");
    for _ in 0..2 {
        lines.extend(exchange("plugin/list", list.clone()));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    let sent = Rc::new(RefCell::new(None));
    let slot = sent.clone();
    composer.update(&mut fx.cx, |c, _| {
        c.on_submit(move |s, _, _| *slot.borrow_mut() = Some(s.text.clone()))
    });
    fx.type_text("@")?;
    fx.wait("plugin rows", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
            .is_some_and(|s| {
                s.iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|r| r.0.starts_with("GitHub"))
            })
    })?;
    let sections = fx
        .read(|root, cx| root.composer(cx).read(cx).mention_state())
        .context("mention list")?;
    let plugins = &sections
        .iter()
        .find(|(h, _)| h.as_deref() == Some("Plugins"))
        .context("Plugins section")?
        .1;
    ensure!(
        plugins
            .iter()
            .map(|r| r.0.as_str())
            .eq(["GitHub | GitHub plugin"]),
        "Plugins must list only the installed plugin: {plugins:?}"
    );
    fx.capture_evidence("CGX_PLUGIN_MENTION_EVIDENCE", "plugins-listed")?;
    fx.type_text("git")?;
    fx.keys("enter")?;
    let chips = fx.read(|root, cx| root.composer(cx).read(cx).chips());
    ensure!(
        chips.first().is_some_and(|c| c.0 == "GitHub"),
        "no GitHub chip: {chips:?}"
    );
    fx.keys("enter")?;
    let text = sent.borrow().clone().context("the prompt was not sent")?;
    ensure!(
        text.contains("[@GitHub](plugin://github@openai-curated)"),
        "sent {text:?}"
    );
    Ok(())
}
