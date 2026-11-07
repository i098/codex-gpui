//! Settings ▸ `Profile` (the original's `appRoute: "/profile"`) through the real `Root`: the
//! nav row opens the native `/profile` page, which reads `GET /profiles/me/page`.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::ReplayHost;
use common::{BACK, Fx};
use serde_json::Value;
use std::sync::Arc;
use ui::screens::profile::{Load, Page};

/// The recorded profile reads, with the exact headers `usage::http` sends (no `originator`).
fn host() -> Result<ReplayHost> {
    let mut recording: Value = serde_json::from_str(include_str!(
        "../../../fixtures/chatgpt-health-profile.json"
    ))?;
    for exchange in recording["exchanges"].as_array_mut().into_iter().flatten() {
        if let Some(headers) = exchange["request"]["request"]["headers"].as_object_mut() {
            headers.remove("originator");
        }
    }
    ReplayHost::from_recording(&recording.to_string())
}

fn load(fx: &Fx) -> Option<Load> {
    fx.read(|root, cx| {
        let page = root
            .shell()
            .read(cx)
            .main()
            .clone()
            .downcast::<Page>()
            .ok()?;
        Some(page.read(cx).load.clone())
    })
}

#[test]
fn settings_profile_opens_the_profile_page() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(host()?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.keys("cmd-,")?;
    fx.capture_evidence("CGX_PROFILE_EVIDENCE", "settings")?;
    let profile = fx.at("settings", "Profile")?;
    fx.click(profile)?;
    ensure!(fx.route() == Route::Profile, "route {:?}", fx.route());
    fx.wait("profile page", |fx| {
        matches!(load(fx), Some(Load::Ready(_)))
    })?;
    fx.capture_evidence("CGX_PROFILE_EVIDENCE", "profile")?;
    let Some(Load::Ready(page)) = load(&fx) else {
        unreachable!("waited for Ready");
    };
    ensure!(page.name() == "Demo User", "name {:?}", page.name());
    ensure!(page.summary.is_some() && page.insights.is_some());
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Settings, "Back: {:?}", fx.route());
    Ok(())
}
