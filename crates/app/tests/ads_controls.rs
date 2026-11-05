//! Settings ▸ Ads controls through the real `Root` against a stateful fake of the ChatGPT
//! backend: the paged ads history, personalized ads, and delete all ads data.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[test]
fn proxy() {
    common::proxy_child();
}

#[derive(Default)]
struct Backend {
    personalized: Mutex<bool>,
    deleted: Mutex<bool>,
    log: Mutex<Vec<(String, String)>>,
}

fn ad(id: &str, brand: &str, action: &str) -> Value {
    json!({
        "id": id,
        "action": action,
        "occurred_at": "2026-10-05T12:00:00Z",
        "ad_data": {"advertiser_brand": {"name": brand}, "title": "Spring sale"},
    })
}

impl Backend {
    fn answer(&self, method: &str, url: &str) -> (u16, Value) {
        let mut personalized = self.personalized.lock().unwrap();
        let mut deleted = self.deleted.lock().unwrap();
        match (method, url) {
            ("GET", "/settings/user") => (
                200,
                json!({"settings": {"bazaar_personalization_enabled": *personalized}}),
            ),
            ("GET", _) if *deleted => (200, json!({"history": [], "preferences": []})),
            ("GET", "/bazaar/history") => (
                200,
                json!({"history": [ad("a1", "Acme", "viewed")], "cursor": "next page"}),
            ),
            ("GET", "/bazaar/history?cursor=next+page") => {
                (200, json!({"history": [ad("a2", "Globex", "opened")]}))
            }
            ("GET", "/bazaar/preferences") => {
                (200, json!({"preferences": [{"content": "Hiking gear"}]}))
            }
            ("PATCH", url) if url.starts_with("/settings/account_user_setting?") => {
                *personalized = url
                    == "/settings/account_user_setting?feature=bazaar_personalization_enabled&value=true";
                (200, json!({}))
            }
            ("DELETE", "/bazaar/profile") => {
                *deleted = true;
                (200, json!({}))
            }
            _ => (404, json!({"detail": "not found"})),
        }
    }
}

impl Host for Backend {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(request) = request else {
            anyhow::bail!("not an HTTP request");
        };
        let method = request["method"].as_str().context("method")?.to_owned();
        let url = request["url"].as_str().context("url")?.to_owned();
        ensure!(request["headers"]["X-OpenAI-Attach-Auth"] == "1", "no auth");
        let (status, answer) = self.answer(&method, &url);
        self.log.lock().unwrap().push((method, url));
        Ok(json!({"status": status, "bodyJsonString": answer.to_string()}))
    }
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

fn called(backend: &Backend, method: &str, url: &str) -> bool {
    let log = backend.log.lock().unwrap();
    log.iter().any(|(m, u)| m == method && u == url)
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_ADS_EVIDENCE", name)
}

#[test]
fn history_personalize_and_delete() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/ads-controls").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("ads controls", |fx| visible(fx, "ads-history-link"))?;
    shot(&mut fx, "main")?;

    click(&mut fx, "ads-personalize")?;
    fx.wait("personalized", |_| *backend.personalized.lock().unwrap())?;

    click(&mut fx, "ads-history-link")?;
    fx.wait("first page", |fx| visible(fx, "ads-load-more"))?;
    click(&mut fx, "ads-load-more")?;
    fx.wait("second page", |fx| {
        called(&backend, "GET", "/bazaar/history?cursor=next+page") && !visible(fx, "ads-load-more")
    })?;
    shot(&mut fx, "history")?;
    click(&mut fx, "ads-back")?;
    fx.wait("main again", |fx| visible(fx, "ads-delete"))?;

    click(&mut fx, "ads-delete")?;
    fx.keys("tab enter")?;
    fx.wait("deleted", |_| called(&backend, "DELETE", "/bazaar/profile"))?;
    click(&mut fx, "ads-topics-link")?;
    fx.wait("empty topics", |_| {
        called(&backend, "GET", "/bazaar/preferences")
    })?;
    fx.advance(std::time::Duration::from_millis(50))?;
    shot(&mut fx, "topics-empty")?;
    Ok(())
}
