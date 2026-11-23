//! Settings ▸ Browser ▸ Browsing history through the real `Root`: open from the Browser page,
//! search, the Agent filter, remove one visit, remove a selection and clear all.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use ui::screens::settings::browser_settings::{BrowsingHistory, Visit};

#[test]
fn proxy() {
    common::proxy_child();
}

fn click(fx: &mut Fx, selector: &str) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, selector)?
        .with_context(|| format!("{selector} is not on screen"))?;
    let center = bounds.center();
    fx.click((f32::from(center.x), f32::from(center.y)))
}

fn visible(fx: &mut Fx, selector: &str) -> bool {
    fx.cx
        .debug_bounds(fx.window, selector)
        .is_ok_and(|b| b.is_some())
}

fn urls(fx: &mut Fx) -> Vec<String> {
    fx.cx.update(|cx| {
        cx.global::<BrowsingHistory>()
            .0
            .iter()
            .map(|visit| visit.url.clone())
            .collect()
    })
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_BROWSING_HISTORY_EVIDENCE", name)
}

#[test]
fn search_filter_remove_and_clear() -> Result<()> {
    common::skip_unless_macos!("browsing_history");
    let mut fx = Fx::new()?;
    let visit = |url: &str, title: &str, time, agent| Visit {
        url: url.into(),
        title: title.into(),
        time,
        agent,
    };
    fx.cx.update(|cx| {
        cx.set_global(BrowsingHistory(vec![
            // A long unbroken title and URL, with RTL, CJK and emoji text.
            visit(
                &format!("https://long.example/{}", "a".repeat(300)),
                &format!("{} مرحبا 你好 🚀 {}", "x".repeat(200), "word ".repeat(40)),
                1_763_899_000_000,
                false,
            ),
            visit(
                "https://docs.rs/gpui",
                "gpui - Rust",
                1_763_900_000_000,
                false,
            ),
            visit(
                "https://github.com/i098",
                "i098 on GitHub",
                1_763_900_060_000,
                true,
            ),
            visit(
                "https://example.com/",
                "Example Domain",
                1_763_900_120_000,
                false,
            ),
        ]))
    });
    let link = app::deep_link::parse("codex://settings/browser-use").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("Manage", |fx| visible(fx, "browsing-history-manage"))?;
    shot(&mut fx, "browser")?;
    click(&mut fx, "browsing-history-manage")?;
    fx.wait("history page", |fx| visible(fx, "browsing-history-clear"))?;
    shot(&mut fx, "history")?;

    click(&mut fx, "browsing-history-search")?;
    fx.type_text("github")?;
    // Newest first: with no filter the first Remove would hit example.com.
    fx.wait("search", |fx| visible(fx, "browsing-history-remove"))?;
    shot(&mut fx, "search")?;
    click(&mut fx, "browsing-history-remove")?;
    ensure!(
        urls(&mut fx).len() == 3 && !urls(&mut fx).iter().any(|url| url.contains("github")),
        "{:?}",
        urls(&mut fx)
    );
    // Remove took focus from the field: click it again before erasing `github`.
    click(&mut fx, "browsing-history-search")?;
    fx.keys(&["backspace"; 6].join(" "))?;

    click(&mut fx, "browsing-history-select")?;
    fx.wait("Remove selected", |fx| {
        visible(fx, "browsing-history-remove-selected")
    })?;
    shot(&mut fx, "selected")?;
    click(&mut fx, "browsing-history-remove-selected")?;
    // Every row's checkbox shares the selector, which finds the last (oldest) row.
    ensure!(
        urls(&mut fx) == ["https://docs.rs/gpui", "https://example.com/"],
        "{:?}",
        urls(&mut fx)
    );

    click(&mut fx, "browsing-history-clear")?;
    fx.keys("tab enter")?;
    ensure!(urls(&mut fx).is_empty(), "{:?}", urls(&mut fx));
    shot(&mut fx, "cleared")?;
    click(&mut fx, "browsing-history-back")?;
    fx.wait("Browser page", |fx| visible(fx, "browsing-history-manage"))
}
