//! Settings ▸ Security through the real `Root` against a stateful fake of the ChatGPT
//! backend: create and change the account PIN, a refused PIN, and the lockdown switch.
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
    pin: Mutex<Option<String>>,
    lockdown: Mutex<bool>,
    log: Mutex<Vec<(String, String, Value)>>,
}

impl Backend {
    fn answer(&self, method: &str, url: &str, body: &Value) -> (u16, Value) {
        let mut pin = self.pin.lock().unwrap();
        let mut lockdown = self.lockdown.lock().unwrap();
        let refused = (400, json!({"detail": {"error_code": "invalid_pin"}}));
        match (method, url) {
            ("GET", "/locked_chats/status") => (200, json!({"has_pin": pin.is_some()})),
            ("GET", "/settings/user") => (
                200,
                json!({"settings": {"lockdown_mode_enabled": *lockdown}}),
            ),
            ("POST", "/locked_chats/unlock") => match (&*pin, body["pin_confirmation"].as_str()) {
                (None, Some(_)) => {
                    *pin = body["pin"].as_str().map(str::to_owned);
                    (200, json!({}))
                }
                (Some(current), None) if body["pin"] == current.as_str() => (200, json!({})),
                _ => refused,
            },
            ("POST", "/locked_chats/pin/change")
                if pin.as_deref().is_some_and(|p| body["current_pin"] == p) =>
            {
                *pin = body["new_pin"].as_str().map(str::to_owned);
                (200, json!({}))
            }
            ("PATCH", url) if url.starts_with("/settings/account_user_setting?") => {
                *lockdown = url.ends_with("value=true");
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
        let body = request["body"]
            .as_str()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null);
        let (status, answer) = self.answer(&method, &url, &body);
        self.log.lock().unwrap().push((method, url, body));
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

fn type_into(fx: &mut Fx, field: &str, text: &str) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, &format!("account-pin-field-{field}"))?
        .with_context(|| format!("{field} field"))?;
    // The input sits under the 18 px label and its 6 px gap.
    let at = (
        f32::from(bounds.center().x),
        f32::from(bounds.bottom()) - 12.,
    );
    fx.click(at)?;
    fx.type_text(text)
}

fn posts(backend: &Backend) -> usize {
    let log = backend.log.lock().unwrap();
    log.iter().filter(|(m, ..)| m != "GET").count()
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_SECURITY_EVIDENCE", name)
}

#[test]
fn pin_and_lockdown() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/security").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("Create a PIN", |fx| visible(fx, "account-pin-create"))?;
    shot(&mut fx, "no-pin")?;

    click(&mut fx, "account-pin-create")?;
    type_into(&mut fx, "New PIN", "1234")?;
    type_into(&mut fx, "Confirm new PIN", "1235")?;
    click(&mut fx, "account-pin-submit")?;
    ensure!(posts(&backend) == 0, "sent mismatched PINs");
    shot(&mut fx, "mismatch")?;
    type_into(&mut fx, "Confirm new PIN", "")?;
    fx.keys("backspace")?;
    fx.type_text("4")?;
    click(&mut fx, "account-pin-submit")?;
    fx.wait("PIN created", |fx| visible(fx, "account-pin-change"))?;
    ensure!(backend.pin.lock().unwrap().as_deref() == Some("1234"));
    shot(&mut fx, "has-pin")?;

    click(&mut fx, "account-pin-change")?;
    type_into(&mut fx, "Current PIN", "9999")?;
    click(&mut fx, "account-pin-submit")?;
    fx.wait("refused PIN", |_| posts(&backend) == 2)?;
    fx.advance(std::time::Duration::from_millis(50))?;
    shot(&mut fx, "incorrect")?;
    type_into(&mut fx, "Current PIN", "")?;
    fx.keys(&["backspace"; 4].join(" "))?;
    fx.type_text("1234")?;
    click(&mut fx, "account-pin-submit")?;
    fx.wait("new PIN step", |fx| {
        visible(fx, "account-pin-field-Confirm new PIN")
    })?;
    type_into(&mut fx, "New PIN", "4321")?;
    type_into(&mut fx, "Confirm new PIN", "4321")?;
    click(&mut fx, "account-pin-submit")?;
    fx.wait("PIN changed", |fx| visible(fx, "account-pin-change"))?;
    ensure!(backend.pin.lock().unwrap().as_deref() == Some("4321"));

    click(&mut fx, "lockdown-mode")?;
    fx.wait("lockdown on", |_| *backend.lockdown.lock().unwrap())?;
    fx.advance(std::time::Duration::from_millis(300))?;
    shot(&mut fx, "lockdown-on")?;
    Ok(())
}
