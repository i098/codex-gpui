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
use ui::screens::space::{PageRetry, Pages, Retry, Spaces, View};

const RECORDING: &str = include_str!("../../../fixtures/space-sites.json");

/// The `Pages` and `Sites` pills of the landing's tab bar, at 1280×832.
const PAGES_TAB: (f32, f32) = (280., 244.);
const SITES_TAB: (f32, f32) = (492., 244.);

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

fn page_titles(fx: &Fx) -> Option<Vec<String>> {
    fx.read(|root, cx| {
        let page = root.space_page()?.read(cx);
        let Pages::Ready(pages) = &page.pages else {
            return None;
        };
        Some(pages.iter().map(|page| page.title.clone()).collect())
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
    fx.wait("the Pages list", |fx| page_titles(fx).is_some())?;
    // Newest `last_used_at` first: the second `next_cursor` page holds the older Page.
    ensure!(
        page_titles(&fx) == Some(vec!["Launch notes page".into(), "Older page".into()]),
        "pages {:?}",
        page_titles(&fx)
    );
    fx.capture_evidence("CGX_SPACE_EVIDENCE", "after-rail-space")?;

    // The tab bar between the Spaces and the Pages: `Pages` switches the list, and a
    // tab whose source has not loaded does not.
    let view = |fx: &Fx| fx.read(|root, cx| root.space_page().map(|page| page.read(cx).view));
    fx.click(PAGES_TAB)?;
    ensure!(view(&fx) == Some(View::Pages), "view {:?}", view(&fx));
    fx.click(SITES_TAB)?;
    ensure!(
        view(&fx) == Some(View::Pages),
        "a tab without a source switched the view"
    );
    // Hovering a Page row records it, which hides its divider and the one above it.
    let hovered = |fx: &Fx| fx.read(|root, cx| root.space_page().unwrap().read(cx).hovered);
    for (at, row) in [((300., 296.), 0), ((300., 326.), 1)] {
        fx.hover(at)?;
        fx.advance(ui::motion::BASIC)?;
        ensure!(
            hovered(&fx) == Some(row),
            "hover {at:?}: {:?}",
            hovered(&fx)
        );
    }
    // `reference_shot` parks the pointer, so this one keeps it on the second Page.
    if let Some(dir) = std::env::var_os("CGX_SPACE_EVIDENCE") {
        std::fs::create_dir_all(&dir)?;
        fx.shot()?
            .save(std::path::Path::new(&dir).join("hover-rail-space.png"))?;
    }
    fx.hover((640., 22.))?;
    fx.advance(ui::motion::BASIC)?;
    ensure!(hovered(&fx).is_none(), "leave: {:?}", hovered(&fx));

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

/// Fails the first `/pages` read, then answers an empty page list.
struct PagesFlakyHost(AtomicBool);

impl Host for PagesFlakyHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(http) = request else {
            bail!("unexpected local request");
        };
        ensure!(
            http["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("/pages?")),
            "unexpected {http}"
        );
        if !self.0.swap(true, Ordering::SeqCst) {
            return Ok(
                json!({"value": {"responseType": "error", "status": 500, "error": "Server Error"}}),
            );
        }
        Ok(json!({"status": 200, "body": r#"{"items":[],"next_cursor":null}"#}))
    }
}

#[test]
fn rail_space_pages_error_retries_to_empty_state() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(PagesFlakyHost(AtomicBool::new(false)));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenSpace), cx);
    })?;
    let pages = |fx: &Fx| fx.read(|root, cx| root.space_page().map(|p| p.read(cx).pages.clone()));
    fx.wait("the pages load error", |fx| {
        pages(fx) == Some(Pages::Failed)
    })?;
    let page = fx.read(|root, _| root.space_page().cloned()).unwrap();
    fx.cx
        .update(|cx| page.update(cx, |_, cx| cx.emit(PageRetry)));
    fx.wait("the empty page list", |fx| {
        pages(fx) == Some(Pages::Ready(Vec::new()))
    })?;
    Ok(())
}
