//! Sidebar `Your dot` (`/dots/home`) through the real `Root` against a fake ChatGPT backend:
//! an available dot opens on ChatGPT; otherwise the page shows `Couldn't load your dot` and
//! the sidebar drops the row, as the desktop original does.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::Route;
use app::host::{Host, Identity, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn proxy() {
    common::proxy_child();
}

const ACCOUNT: &str = "00000000-0000-4000-8000-000000000000";

/// Answers `/accounts/check` with `tbo_config.tbo_available` and counts the reads.
struct Backend {
    available: bool,
    reads: AtomicUsize,
}

impl Host for Backend {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(request) = request else {
            anyhow::bail!("not an HTTP request");
        };
        let (status, body) = match request["url"].as_str().context("url")? {
            "/accounts/check/v4-2023-04-27" => {
                self.reads.fetch_add(1, Ordering::SeqCst);
                let account = json!({"structure": "personal", "plan_type": "plus",
                    "tbo_config": {"tbo_available": self.available, "plan_eligible": self.available}});
                (200, json!({"accounts": {ACCOUNT: {"account": account}}}))
            }
            _ => (404, json!({"detail": "not found"})),
        };
        Ok(json!({"status": status, "bodyJsonString": body.to_string()}))
    }

    fn identity(&self) -> Option<Identity> {
        Some(Identity {
            account_id: ACCOUNT.into(),
            user_id: "user".into(),
            device_id: "device".into(),
        })
    }
}

fn visible(fx: &mut Fx, selector: &str) -> bool {
    fx.cx
        .debug_bounds(fx.window, selector)
        .is_ok_and(|b| b.is_some())
}

fn click(fx: &mut Fx, selector: &str) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, selector)?
        .with_context(|| format!("{selector} is not on screen"))?;
    let center = bounds.center();
    fx.click((f32::from(center.x), f32::from(center.y)))
}

/// Clicks `Your dot`, with `backend` as the host (none: no ChatGPT identity).
fn click_your_dot(backend: Option<Arc<Backend>>) -> Result<Fx> {
    let mut fx = Fx::new()?;
    if let Some(backend) = backend {
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(backend, cx));
    }
    click(&mut fx, "your-dot")?;
    Ok(fx)
}

fn backend(available: bool) -> Arc<Backend> {
    Arc::new(Backend {
        available,
        reads: AtomicUsize::new(0),
    })
}

#[test]
fn unreadable_account_offers_try_again() -> Result<()> {
    let mut fx = click_your_dot(None)?;
    ensure!(fx.route() == Route::Dots, "route {:?}", fx.route());
    fx.wait("the error", |fx| visible(fx, "dots-retry"))?;
    fx.capture_evidence("CGX_DOTS_EVIDENCE", "dots-error")?;
    ensure!(fx.cx.take_opened_url().is_none(), "a URL opened");
    // Without an answered read the row stays, as on a signed-out launch.
    ensure!(visible(&mut fx, "your-dot"), "the row went away");
    Ok(())
}

#[test]
fn unavailable_dot_shows_the_error_and_drops_the_row() -> Result<()> {
    let backend = backend(false);
    let mut fx = click_your_dot(Some(backend.clone()))?;
    ensure!(fx.route() == Route::Dots, "route {:?}", fx.route());
    fx.wait("the error", |fx| visible(fx, "dots-retry"))?;
    fx.wait("the row to hide", |fx| !visible(fx, "your-dot"))?;
    fx.capture_evidence("CGX_DOTS_EVIDENCE", "dots-unavailable")?;
    click(&mut fx, "dots-retry")?;
    fx.wait("a second read", |_| {
        backend.reads.load(Ordering::SeqCst) == 2
    })?;
    fx.wait("the error again", |fx| visible(fx, "dots-retry"))?;
    ensure!(fx.cx.take_opened_url().is_none(), "a URL opened");
    Ok(())
}

#[test]
fn available_dot_opens_on_chatgpt() -> Result<()> {
    let mut fx = click_your_dot(Some(backend(true)))?;
    fx.wait("Home", |fx| fx.route() == Route::Home)?;
    ensure!(
        fx.cx.take_opened_url().as_deref() == Some("https://chatgpt.com/dots/home"),
        "the dot did not open"
    );
    ensure!(visible(&mut fx, "your-dot"), "the row went away");
    Ok(())
}
