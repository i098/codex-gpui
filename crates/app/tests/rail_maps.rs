//! Explore ▸ `Maps` (docs/flows.md section 14.1 row 5) through the real `Root`: the menu item
//! opens `/maps` and the page reads `GET /local/home` from the host.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::ReplayHost;
use common::{BACK, Fx};
use std::sync::Arc;
use ui::screens::maps::Load;

/// The rail's Explore `…` item, then the menu's first row, `Maps`.
const RAIL_EXPLORE: (f32, f32) = (26., 246.);
const EXPLORE_MAPS: (f32, f32) = (100., 242.);

fn load(fx: &Fx) -> Option<Load> {
    fx.read(|root, cx| root.maps_page().map(|page| page.read(cx).load.clone()))
}

/// Without a ChatGPT identity the page shows `Couldn't load recommendations`; a host that
/// answers shows its prompt cards.
#[test]
fn rail_maps() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(RAIL_EXPLORE)?;
    fx.capture_evidence("CGX_MAPS_EVIDENCE", "explore-menu")?;
    fx.click(EXPLORE_MAPS)?;
    ensure!(fx.route() == Route::Maps, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("explore") && s.can_go_back && !s.explore_menu));
    fx.wait("refused read", |fx| {
        matches!(load(fx), Some(Load::Failed(_)))
    })?;
    fx.capture_evidence("CGX_MAPS_EVIDENCE", "maps-error")?;
    let host = ReplayHost::from_recording(include_str!("../../../fixtures/maps-home.json"))?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.wait("recommendations", |fx| {
        matches!(load(fx), Some(Load::Ready(_)))
    })?;
    fx.capture_evidence("CGX_MAPS_EVIDENCE", "maps-ready")?;
    let titles = fx.read(|root, cx| {
        root.maps_page()
            .map(|page| page.read(cx).titles().join(", "))
            .unwrap_or_default()
    });
    ensure!(
        titles == "Quiet cafés to work from, Parks for a walk, Dinner tonight",
        "titles {titles:?}"
    );
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(())
}
