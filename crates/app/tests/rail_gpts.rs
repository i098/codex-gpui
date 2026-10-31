//! Explore ▸ `GPTs` through the real `Root`: `OpenGpts` opens `/gpts`, the page lists the
//! user's GPTs and the catalog, and a row opens the GPT's page.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::ReplayHost;
use common::{BACK, Fx};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::gpts::{Load, Open, View};

fn view(fx: &Fx) -> Option<View> {
    fx.read(|root, cx| root.gpts_page().map(|page| page.read(cx).view.clone()))
}

/// `fixtures/gpts.json` records no session; the host identity comes from one more exchange
/// with the recording's device id.
fn host() -> Result<ReplayHost> {
    let mut recording: Value = serde_json::from_str(include_str!("../../../fixtures/gpts.json"))?;
    let device = recording["exchanges"][0]["request"]["request"]["headers"]["oai-did"].clone();
    recording["exchanges"].as_array_mut().unwrap().push(json!({
        "request": {"transport": "http", "request": {
            "url": "/identity", "method": "GET", "headers": {"oai-did": device},
            "expectedIdentity": {"accountId": "account", "userId": "user"},
        }},
        "response": {},
    }));
    ReplayHost::from_recording(&recording.to_string())
}

#[test]
fn rail_gpts() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.action(Box::new(ui::OpenGpts))?;
    ensure!(fx.route() == Route::Gpts(None), "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("explore") && s.can_go_back && !s.explore_menu));
    fx.wait("refused read", |fx| {
        matches!(view(fx), Some(View::List(Load::Failed(_))))
    })?;
    fx.capture_evidence("CGX_GPTS_EVIDENCE", "gpts-error")?;
    let host = Arc::new(host()?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.wait("lists", |fx| {
        matches!(view(fx), Some(View::List(Load::Ready(_))))
    })?;
    fx.capture_evidence("CGX_GPTS_EVIDENCE", "gpts-list")?;
    let names = fx.read(|root, cx| {
        root.gpts_page()
            .map(|page| page.read(cx).names().join(", "))
            .unwrap_or_default()
    });
    ensure!(names == "Demo GPT, Demo GPT", "names {names:?}");
    let page = fx.read(|root, _| root.gpts_page().cloned()).unwrap();
    page.update(&mut fx.cx, |_, cx| {
        cx.emit(Open("g-000000001-demo-gpt".into()))
    });
    fx.advance(std::time::Duration::ZERO)?;
    ensure!(
        fx.route() == Route::Gpts(Some("g-000000001-demo-gpt".into())),
        "route {:?}",
        fx.route()
    );
    fx.wait("detail", |fx| {
        matches!(view(fx), Some(View::Gpt(_, Load::Ready(_))))
    })?;
    fx.capture_evidence("CGX_GPTS_EVIDENCE", "gpt-detail")?;
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Gpts(None), "Back: {:?}", fx.route());
    Ok(())
}
