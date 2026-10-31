//! Explore ▸ `Images` through the real `Root`: the menu item opens `/images` and the page reads
//! the Library's image nodes from the host.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::ReplayHost;
use common::{BACK, Fx};
use std::sync::Arc;
use ui::screens::images::Load;

/// The rail's Explore `…` item, then the menu's first row, `Images`.
const RAIL_EXPLORE: (f32, f32) = (26., 246.);
const EXPLORE_IMAGES: (f32, f32) = (100., 242.);

fn load(fx: &Fx) -> Option<Load> {
    fx.read(|root, cx| root.images_page().map(|page| page.read(cx).load.clone()))
}

/// Without a ChatGPT identity the page shows `Couldn't load images`; the recorded Library lists
/// its one image file and skips the notes file.
#[test]
fn rail_images() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(RAIL_EXPLORE)?;
    fx.click(EXPLORE_IMAGES)?;
    ensure!(fx.route() == Route::Images, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("explore") && s.can_go_back && !s.explore_menu));
    fx.wait("refused read", |fx| {
        matches!(load(fx), Some(Load::Failed(_)))
    })?;
    fx.capture_evidence("CGX_IMAGES_EVIDENCE", "images-error")?;
    let host = ReplayHost::from_recording(include_str!("../../../fixtures/images-library.json"))?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.wait("images", |fx| matches!(load(fx), Some(Load::Ready(_))))?;
    fx.capture_evidence("CGX_IMAGES_EVIDENCE", "images-ready")?;
    let names = fx.read(|root, cx| {
        root.images_page()
            .map(|page| page.read(cx).names().join(", "))
            .unwrap_or_default()
    });
    ensure!(names == "walking-route.svg", "names {names:?}");
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(())
}
