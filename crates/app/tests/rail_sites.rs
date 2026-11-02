//! Rail `Sites` (docs/flows.md section 14.1 row 7) through the real `Root`: the click opens
//! `/sites` and the page lists `GET /websites?limit=20` from the host; a row opens
//! `/sites/:projectId` from `GET /websites/{project_id}`.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::ReplayHost;
use common::{BACK, Fx};
use std::sync::Arc;
use ui::screens::sites::{Detail, Load, Site};

/// The rail's pinned `Sites` item (refs/home/dark@1280x832.spec.json).
const RAIL_SITES: (f32, f32) = (26., 343.);
/// The first site row, the details header's `Edit` and `Sites` (sidebar closed, 1280 wide).
const FIRST_ROW: (f32, f32) = (300., 165.);
const EDIT: (f32, f32) = (1222., 88.);
const SITES_BACK: (f32, f32) = (110., 88.);
const LAUNCH: &str = "site-000000000000000000000001";

fn load(fx: &Fx) -> Option<Load> {
    fx.read(|root, cx| root.sites_page().map(|page| page.read(cx).load.clone()))
}

fn detail(fx: &Fx) -> Option<Load<Site>> {
    fx.read(|root, cx| {
        let page = root.sites_page()?.read(cx);
        page.detail.as_ref().map(|Detail { load, .. }| load.clone())
    })
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

/// A row opens the site's details; `Edit` continues it in a new chat that mentions the site;
/// the `Sites` button returns to the list.
#[test]
fn site_row_opens_details() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = ReplayHost::from_recording(include_str!("../../../fixtures/site-detail.json"))?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.click(RAIL_SITES)?;
    fx.wait("sites listed", |fx| {
        matches!(load(fx), Some(Load::Ready(_)))
    })?;
    fx.capture_evidence("CGX_SITES_SHOTS", "sites-list")?;
    fx.click(FIRST_ROW)?;
    ensure!(
        fx.route() == Route::Site(LAUNCH.into()),
        "route {:?}",
        fx.route()
    );
    ensure!(fx.shell(|s| s.selected_rail == Some("sites") && s.can_go_back));
    fx.wait(
        "site loaded",
        |fx| matches!(detail(fx), Some(Load::Ready(site)) if site.title == "Launch page"),
    )?;
    fx.capture_evidence("CGX_SITES_SHOTS", "site-detail")?;
    fx.click(SITES_BACK)?;
    ensure!(fx.route() == Route::Sites, "Sites: {:?}", fx.route());
    fx.click(BACK)?;
    ensure!(
        fx.route() == Route::Site(LAUNCH.into()),
        "Back: {:?}",
        fx.route()
    );
    fx.wait("site reloaded", |fx| {
        matches!(detail(fx), Some(Load::Ready(_)))
    })?;
    fx.click(EDIT)?;
    ensure!(fx.route() == Route::Home, "Edit: {:?}", fx.route());
    let prompt = fx.composer_text()?;
    ensure!(
        prompt == format!("[@Launch page](sites-project://{LAUNCH}) "),
        "prompt {prompt:?}"
    );
    Ok(())
}
