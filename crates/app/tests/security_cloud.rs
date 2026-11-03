//! Explore ▸ `Security Cloud` (docs/flows.md section 14.1 row 5) through the real `Root`: the
//! recorded MCP catalog lists the destination, and the page reads the recorded
//! `defense_factory_*` results (fixtures/security-cloud.jsonl) through `mcpServer/tool/call`.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::Fx;
use ui::screens::security_cloud::Tab;

/// The rail's Explore `…` item.
const RAIL_EXPLORE: (f32, f32) = (26., 246.);

fn snapshot(fx: &Fx) -> Option<bridge::security_cloud::Snapshot> {
    fx.read(|root, cx| {
        root.security_cloud_page()
            .and_then(|page| page.read(cx).snapshot.clone())
    })
}

fn tab(fx: &mut Fx, tab: Tab) {
    fx.root.update(&mut fx.cx, |root, cx| {
        if let Some(page) = root.security_cloud_page() {
            page.update(cx, |page, cx| {
                page.tab = tab;
                cx.notify();
            });
        }
    });
}

/// The live account's empty Security Cloud: $500 free credits, no open findings, a refused
/// GitHub repository read, and no scans.
#[test]
fn explore_security_cloud_reads_the_recorded_account() -> Result<()> {
    let mut fx = Fx::with("dark", (1280., 832.), Some("security-cloud.jsonl"), |_| {})?;
    fx.wait("catalog lists Security Cloud", |fx| {
        fx.shell(|s| s.security_cloud_available)
    })?;
    fx.click(RAIL_EXPLORE)?;
    ensure!(
        fx.shell(|s| s.explore_menu),
        "Explore did not open its menu"
    );
    fx.capture_evidence("CGX_SECURITY_CLOUD_EVIDENCE", "explore-menu")?;
    fx.action(Box::new(ui::OpenSecurityCloud))?;
    ensure!(fx.route() == Route::SecurityCloud, "route {:?}", fx.route());
    fx.wait("snapshot loaded", |fx| snapshot(fx).is_some())?;
    let snapshot = snapshot(&fx).expect("loaded");
    ensure!(
        snapshot.free_credits == Some(500.),
        "credits {:?}",
        snapshot.free_credits
    );
    ensure!(
        snapshot
            .findings
            .as_ref()
            .is_ok_and(|f| f.items.is_empty() && f.total == 0),
        "findings {:?}",
        snapshot.findings
    );
    ensure!(snapshot.summary == Ok(0), "summary {:?}", snapshot.summary);
    ensure!(
        snapshot.repositories == Err("GitHub could not complete the request.".to_owned()),
        "repositories {:?}",
        snapshot.repositories
    );
    ensure!(
        snapshot.scans == Ok(Vec::new()),
        "scans {:?}",
        snapshot.scans
    );
    fx.capture_evidence("CGX_SECURITY_CLOUD_EVIDENCE", "findings")?;
    tab(&mut fx, Tab::Repositories);
    fx.capture_evidence("CGX_SECURITY_CLOUD_EVIDENCE", "repositories")?;
    tab(&mut fx, Tab::Scans);
    fx.capture_evidence("CGX_SECURITY_CLOUD_EVIDENCE", "scans")?;
    Ok(())
}
