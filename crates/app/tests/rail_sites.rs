//! Rail `Sites` (docs/flows.md section 14.1 row 7) through the real `Root`: the click opens
//! `/sites` and the page lists `GET /websites?limit=20` from the host.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::ReplayHost;
use common::{BACK, Fx};
use std::sync::Arc;
use ui::screens::sites::Load;

/// The rail's pinned `Sites` item (refs/home/dark@1280x832.spec.json).
const RAIL_SITES: (f32, f32) = (26., 343.);

fn load(fx: &Fx) -> Option<Load> {
    fx.read(|root, cx| root.sites_page().map(|page| page.read(cx).load.clone()))
}

/// A refused read shows `Unable to load sites`; a host that answers lists the owned sites.
#[test]
fn rail_sites() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(RAIL_SITES)?;
    ensure!(fx.route() == Route::Sites, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("sites") && s.can_go_back && !s.new_tab));
    fx.wait("refused read", |fx| {
        matches!(load(fx), Some(Load::Failed(_)))
    })?;
    let host = ReplayHost::from_recording(include_str!("../../../fixtures/space-sites.json"))?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.wait("sites listed", |fx| {
        matches!(load(fx), Some(Load::Ready(_)))
    })?;
    let titles = fx.read(|root, cx| {
        root.sites_page()
            .map(|page| page.read(cx).titles().join(", "))
            .unwrap_or_default()
    });
    ensure!(titles == "Launch page, Team handbook", "titles {titles:?}");
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(())
}
