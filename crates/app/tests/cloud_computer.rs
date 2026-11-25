//! Settings ▸ Cloud computer through the real `Root` against a stateful fake of the ChatGPT
//! backend: the cookie sites with per-site, per-host and Clear all deletion, and the passkeys.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::Mutex;

#[test]
fn proxy() {
    common::proxy_child();
}

struct Backend {
    hosts: Mutex<Vec<Value>>,
    passkeys: Mutex<Vec<Value>>,
    /// The next cookie `DELETE` fails once, like a flaky backend.
    fail_next_delete: Mutex<bool>,
    log: Mutex<Vec<(String, String)>>,
}

fn host(hostname: &str, domain: &str, count: u64) -> Value {
    json!({"hostname": hostname, "registrable_domain": domain, "cookie_count": count})
}

impl Backend {
    fn new() -> Self {
        Self {
            hosts: Mutex::new(vec![
                host("example.com", "example.com", 2),
                host("www.example.com", "example.com", 3),
                host("acme.org", "acme.org", 1),
            ]),
            passkeys: Mutex::new(vec![json!({
                "credential_id": "pk/1", "relying_party_id": "github.com",
                "user_display_name": "Ada Lovelace", "user_name": "ada"
            })]),
            fail_next_delete: Mutex::new(false),
            log: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, method: &str, url: &str) -> (u16, Value) {
        let mut hosts = self.hosts.lock().unwrap();
        let mut passkeys = self.passkeys.lock().unwrap();
        match (method, url) {
            ("GET", "/flora/browser/config") => (200, json!({"webauthn_enabled": true})),
            ("GET", "/flora/browser/cookies") => (200, json!({"hosts": *hosts})),
            ("GET", "/flora/browser/passkeys") => (200, json!({"passkeys": *passkeys})),
            ("DELETE", "/agent/browser_context") => {
                hosts.clear();
                (204, Value::Null)
            }
            ("DELETE", path) if path.starts_with("/flora/browser/cookies/") => {
                if std::mem::take(&mut *self.fail_next_delete.lock().unwrap()) {
                    return (500, json!({"detail": "boom"}));
                }
                let rest = &path["/flora/browser/cookies/".len()..];
                let (name, query) = rest.split_once('?').unwrap_or((rest, ""));
                let subdomains = query == "include_subdomains=true";
                hosts.retain(|h| {
                    let h = h["hostname"].as_str().unwrap_or_default();
                    h != name && !(subdomains && h.ends_with(&format!(".{name}")))
                });
                (204, Value::Null)
            }
            ("DELETE", "/flora/browser/passkeys/pk%2F1") => {
                passkeys.clear();
                (204, Value::Null)
            }
            _ => (404, json!({"detail": "not found"})),
        }
    }

    fn called(&self, method: &str, url: &str) -> bool {
        let log = self.log.lock().unwrap();
        log.iter().any(|(m, u)| m == method && u == url)
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

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_CLOUD_COMPUTER_EVIDENCE", name)
}

#[test]
fn manage_cloud_browser_cookies_and_passkeys() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = std::sync::Arc::new(Backend::new());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/cloud-computer").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("passkeys", |fx| {
        visible(fx, "cloud-cookies-link") && visible(fx, "cloud-passkey-delete-0")
    })?;
    shot(&mut fx, "main")?;

    // Passkey: Delete passkey ▸ Delete; the id is one encoded path segment.
    click(&mut fx, "cloud-passkey-delete-0")?;
    fx.keys("tab enter")?;
    fx.wait("passkey deleted", |fx| {
        backend.called("DELETE", "/flora/browser/passkeys/pk%2F1")
            && !visible(fx, "cloud-passkey-delete-0")
    })?;

    // Cookies: two sites; example.com lists its hosts under the toggle.
    click(&mut fx, "cloud-cookies-link")?;
    fx.wait("sites", |fx| {
        visible(fx, "cloud-site-delete-1") && visible(fx, "cloud-site-hosts-0")
    })?;
    ensure!(!visible(&mut fx, "cloud-site-hosts-1"));
    click(&mut fx, "cloud-site-hosts-0")?;
    fx.wait("hosts", |fx| visible(fx, "cloud-host-delete-0-1"))?;
    shot(&mut fx, "cookies")?;

    // A failed delete keeps the dialog open; the retry deletes only that host.
    *backend.fail_next_delete.lock().unwrap() = true;
    click(&mut fx, "cloud-host-delete-0-1")?;
    fx.keys("tab enter")?;
    let one_host = "/flora/browser/cookies/www.example.com?include_subdomains=false";
    fx.wait("failed", |_| backend.called("DELETE", one_host))?;
    fx.advance(std::time::Duration::from_millis(50))?;
    shot(&mut fx, "delete-failed")?;
    fx.keys("enter")?;
    fx.wait("host deleted", |fx| {
        backend.hosts.lock().unwrap().len() == 2 && !visible(fx, "cloud-site-hosts-0")
    })?;

    // The site row deletes the domain with its subdomains.
    click(&mut fx, "cloud-site-delete-1")?;
    fx.keys("tab enter")?;
    fx.wait("site deleted", |fx| {
        backend.called(
            "DELETE",
            "/flora/browser/cookies/acme.org?include_subdomains=true",
        ) && !visible(fx, "cloud-site-delete-1")
    })?;

    click(&mut fx, "cloud-cookies-clear-all")?;
    fx.keys("tab enter")?;
    fx.wait("cleared", |fx| {
        backend.called("DELETE", "/agent/browser_context") && !visible(fx, "cloud-site-delete-0")
    })?;
    shot(&mut fx, "empty")?;

    click(&mut fx, "cloud-cookies-back")?;
    fx.wait("main again", |fx| visible(fx, "cloud-cookies-link"))?;
    Ok(())
}
