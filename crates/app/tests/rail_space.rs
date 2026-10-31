//! Rail `Space` opens `/space` and lists the account's Spaces from `GET /spaces/v2`.

mod common;

use anyhow::{Result, bail, ensure};
use app::Route;
use app::host::{Host, ReplayHost, Request};
use common::{BACK, Fx, RAIL_SPACE};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use ui::screens::space::{Retry, Spaces};

const RECORDING: &str = include_str!("../../../fixtures/space-sites.json");

fn names(fx: &Fx) -> Option<Vec<String>> {
    fx.read(|root, cx| {
        let page = root.space_page()?.read(cx);
        let Spaces::Ready(_) = &page.spaces else {
            return None;
        };
        Some(
            page.visible(cx)
                .into_iter()
                .map(|space| space.name)
                .collect(),
        )
    })
}

#[test]
fn rail_space_lists_spaces_from_every_page() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.capture_evidence("CGX_SPACE_EVIDENCE", "before-rail-space")?;
    let host = Arc::new(ReplayHost::from_recording(RECORDING)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.click(RAIL_SPACE)?;
    ensure!(fx.route() == Route::Space, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("space") && s.can_go_back));
    fx.wait("the Spaces list", |fx| names(fx).is_some())?;
    // Newest `updated_at` first, across both `next_cursor` pages.
    ensure!(names(&fx) == Some(vec!["Launch notes".into(), "Demo space".into()]));
    fx.capture_evidence("CGX_SPACE_EVIDENCE", "after-rail-space")?;

    let query = fx.read(|root, cx| root.space_page().unwrap().read(cx).query.clone());
    fx.cx
        .update(|cx| query.update(cx, |q, cx| q.set_text("  DEMO ", cx)));
    ensure!(names(&fx) == Some(vec!["Demo space".into()]));
    fx.cx
        .update(|cx| query.update(cx, |q, cx| q.set_text("zzz", cx)));
    ensure!(
        names(&fx) == Some(Vec::new()),
        "no-match state keeps an empty list"
    );

    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(())
}

/// Fails the first `/spaces/v2` read, then answers an empty list.
struct FlakyHost(AtomicBool);

impl Host for FlakyHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(http) = request else {
            bail!("unexpected local request");
        };
        ensure!(http["url"] == "/spaces/v2?limit=100", "unexpected {http}");
        if !self.0.swap(true, Ordering::SeqCst) {
            return Ok(
                json!({"value": {"responseType": "error", "status": 403, "error": "Forbidden"}}),
            );
        }
        Ok(json!({"status": 200, "body": r#"{"items":[],"next_cursor":null}"#}))
    }
}

#[test]
fn rail_space_load_error_retries_to_empty_state() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(FlakyHost(AtomicBool::new(false)));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenSpace), cx);
    })?;
    let spaces = |fx: &Fx| fx.read(|root, cx| root.space_page().map(|p| p.read(cx).spaces.clone()));
    fx.wait("the load error", |fx| spaces(fx) == Some(Spaces::Failed))?;
    fx.capture_evidence("CGX_SPACE_EVIDENCE", "after-rail-space-error")?;
    let page = fx.read(|root, _| root.space_page().cloned()).unwrap();
    fx.cx.update(|cx| page.update(cx, |_, cx| cx.emit(Retry)));
    fx.wait("the empty list", |fx| {
        spaces(fx) == Some(Spaces::Ready(Vec::new()))
    })?;
    Ok(())
}
