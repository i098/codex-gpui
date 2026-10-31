//! Settings ▸ Passwords through the real `Root` against a stateful fake of the ChatGPT
//! backend: the 1Password connection (set up, connecting poll, disconnect) and the paged saved
//! passwords with search, details and delete.
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
    /// The 1Password state the next `GET` answers, and the `GET`s left until `connecting`
    /// becomes `connected`.
    manager: Mutex<(String, u32)>,
    deleted: Mutex<Vec<String>>,
    log: Mutex<Vec<(String, String, Value)>>,
}

fn password(id: &str, origin: &str, account: &str) -> Value {
    json!({"credential_id": id, "origin": origin, "account_label": account, "created_at": 1_760_000_000})
}

impl Backend {
    fn answer(&self, method: &str, url: &str) -> (u16, Value) {
        let mut manager = self.manager.lock().unwrap();
        let deleted = self.deleted.lock().unwrap();
        let keep = |items: Vec<Value>| -> Vec<Value> {
            items
                .into_iter()
                .filter(|p| !deleted.iter().any(|id| p["credential_id"] == id.as_str()))
                .collect()
        };
        match (method, url) {
            ("GET", "/password-managers/1password") => {
                if manager.0 == "connecting" {
                    if manager.1 == 0 {
                        manager.0 = "connected".into();
                    } else {
                        manager.1 -= 1;
                    }
                }
                (
                    200,
                    json!({"available": true, "enrollment_url": "https://1p.example/enroll", "state": manager.0}),
                )
            }
            ("POST", "/password-managers/1password/enrollment") => {
                *manager = ("connecting".into(), 1);
                (
                    200,
                    json!({"authorization_url": "https://my.1password.com/oauth/authorize?state=s"}),
                )
            }
            ("DELETE", "/password-managers/1password") => {
                manager.0 = "disconnected".into();
                (204, Value::Null)
            }
            ("GET", "/saved-credentials") => (
                200,
                json!({
                    "items": keep(vec![password("p1", "https://acme.com", "ada@acme.com")]),
                    "next_cursor": "page 2",
                }),
            ),
            ("GET", "/saved-credentials?cursor=page+2") => (
                200,
                json!({"items": keep(vec![password("p2", "https://globex.com", "grace@globex.com")])}),
            ),
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
        let body = request["body"]
            .as_str()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null);
        if method == "DELETE"
            && let Some(id) = url.strip_prefix("/saved-credentials/")
        {
            self.deleted.lock().unwrap().push(id.to_owned());
        }
        let (status, answer) = self.answer(&method, &url);
        self.log.lock().unwrap().push((method, url, body));
        Ok(json!({"status": status, "bodyJsonString": answer.to_string()}))
    }

    fn account_user_id(&self) -> Option<String> {
        Some("user-1".into())
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

fn calls(backend: &Backend, method: &str, url: &str) -> Vec<Value> {
    let log = backend.log.lock().unwrap();
    log.iter()
        .filter(|(m, u, _)| m == method && u == url)
        .map(|(_, _, body)| body.clone())
        .collect()
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_PASSWORDS_EVIDENCE", name)
}

#[test]
fn connect_1password_and_manage_saved_passwords() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    backend.manager.lock().unwrap().0 = "disconnected".into();
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/passwords").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("first page", |fx| {
        visible(fx, "password-row-0") && visible(fx, "password-manager-set-up")
    })?;
    ensure!(!visible(&mut fx, "password-manager-disconnect"));
    shot(&mut fx, "main")?;

    // Set up: the enrollment carries the account user, then the page polls until connected.
    click(&mut fx, "password-manager-set-up")?;
    fx.keys("tab enter")?;
    fx.wait("enrolled", |_| {
        !calls(&backend, "POST", "/password-managers/1password/enrollment").is_empty()
    })?;
    ensure!(
        calls(&backend, "POST", "/password-managers/1password/enrollment")
            == [json!({"expected_account_user_id": "user-1"})]
    );
    fx.wait("connecting", |fx| {
        visible(fx, "password-manager-disconnect")
    })?;
    fx.advance(std::time::Duration::from_millis(3200))?;
    fx.wait("connected", |fx| {
        backend.manager.lock().unwrap().0 == "connected" && !visible(fx, "password-manager-set-up")
    })?;
    ensure!(calls(&backend, "GET", "/password-managers/1password").len() >= 3);
    shot(&mut fx, "connected")?;

    // Paging, then search over the loaded passwords.
    click(&mut fx, "passwords-load-more")?;
    fx.wait("second page", |fx| {
        visible(fx, "password-row-1") && !visible(fx, "passwords-load-more")
    })?;
    click(&mut fx, "passwords-search")?;
    fx.type_text("globex")?;
    fx.wait("filtered", |fx| {
        visible(fx, "password-row-0") && !visible(fx, "password-row-1")
    })?;
    shot(&mut fx, "search")?;
    fx.keys(&["backspace"; 6].join(" "))?;
    fx.wait("unfiltered", |fx| visible(fx, "password-row-1"))?;

    // Details ▸ Delete password ▸ Delete.
    click(&mut fx, "password-row-0")?;
    fx.advance(std::time::Duration::from_millis(50))?;
    shot(&mut fx, "details")?;
    fx.keys("tab enter")?;
    fx.advance(std::time::Duration::from_millis(50))?;
    fx.keys("tab enter")?;
    fx.wait("deleted p1", |fx| {
        !calls(&backend, "DELETE", "/saved-credentials/p1").is_empty()
            && !visible(fx, "password-row-0")
            && visible(fx, "passwords-load-more")
    })?;
    click(&mut fx, "passwords-load-more")?;
    fx.wait("p2 again", |fx| visible(fx, "password-row-0"))?;

    // The row's Remove button goes straight to the confirmation.
    click(&mut fx, "password-remove-0")?;
    fx.keys("tab enter")?;
    fx.wait("deleted p2", |fx| {
        !calls(&backend, "DELETE", "/saved-credentials/p2").is_empty()
            && !visible(fx, "password-row-0")
    })?;
    shot(&mut fx, "empty")?;

    click(&mut fx, "password-manager-disconnect")?;
    fx.wait("disconnected", |fx| {
        !calls(&backend, "DELETE", "/password-managers/1password").is_empty()
            && visible(fx, "password-manager-set-up")
    })?;
    Ok(())
}
