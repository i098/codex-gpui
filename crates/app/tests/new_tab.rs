//! PANEL-20: the side panel's `New tab` button opens the `New tab` page, and a pick
//! replaces that tab with the tab it opens, as the original's `thread-side-panel-new-tab`.
mod common;

use anyhow::Result;
use app::SidePanel;
use common::*;
use std::path::Path;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn new_tab_page_opens_a_picked_tab() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("new_tab: the Metal replay requires macOS");
        return Ok(());
    }
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    // The titlebar's `New tab` button, shown while no side panel is open.
    fx.click((1258., 22.))?;
    fx.wait("new tab", |fx| {
        fx.read(|r, _| r.side_tabs() == [SidePanel::NewTab])
    })?;
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fx.shot()?.save(proof.join("new-tab.png"))?;
    // The `Browser` row replaces the `New tab` tab with a browser tab.
    fx.click((755., 174.))?;
    fx.wait("browser tab", |fx| {
        fx.read(|r, _| matches!(r.side_tabs(), [SidePanel::Browser(_)]))
    })?;
    // The strip's `New tab` button opens the page after the other tabs.
    fx.click((928., 21.))?;
    fx.wait("strip new tab", |fx| {
        fx.read(|r, _| matches!(r.side_tabs(), [SidePanel::Browser(_), SidePanel::NewTab]))
    })?;
    Ok(())
}
