//! Settings ▸ Trusted contact through the real `Root` against a stateful fake of the ChatGPT
//! backend's `/trusted_contact` service: invite, pending status, resend and remove.
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
    contacts: Mutex<Vec<Value>>,
    log: Mutex<Vec<(String, String, Value)>>,
}

impl Backend {
    fn answer(&self, method: &str, url: &str, body: &Value) -> (u16, Value) {
        let mut contacts = self.contacts.lock().unwrap();
        match (method, url) {
            ("GET", "/trusted_contact/enabled") => (200, json!({"enabled": true})),
            ("GET", "/trusted_contact") => (200, json!({"contacts": *contacts})),
            ("POST", "/trusted_contact") if body["email"] == "blocked@example.com" => (
                400,
                json!({"detail": {"error_code": "trusted-contacts-blocked-email"}}),
            ),
            ("POST", "/trusted_contact") => {
                let mut contact = body.clone();
                contact["trusted_contact_id"] = "tc-1".into();
                contact["status"] = "pending".into();
                contacts.push(contact.clone());
                (200, contact)
            }
            ("PUT", "/trusted_contact/tc-1") => (200, contacts[0].clone()),
            ("DELETE", "/trusted_contact/tc-1") => {
                contacts.clear();
                (200, json!({"success": true}))
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
        .debug_bounds(fx.window, &format!("trusted-contact-field-{field}"))?
        .with_context(|| format!("{field} field"))?;
    // The input sits under the 18 px label and its 6 px gap.
    let at = (
        f32::from(bounds.center().x),
        f32::from(bounds.bottom()) - 12.,
    );
    fx.click(at)?;
    fx.type_text(text)
}

fn calls(backend: &Backend, method: &str) -> Vec<(String, Value)> {
    backend
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, ..)| m == method)
        .map(|(_, url, body)| (url.clone(), body.clone()))
        .collect()
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_TRUSTED_CONTACT_EVIDENCE", name)
}

#[test]
fn invite_resend_and_remove() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/trusted-contact").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("Add contact", |fx| {
        visible(fx, "add-trusted-contact-button")
    })?;
    shot(&mut fx, "empty")?;

    click(&mut fx, "add-trusted-contact-button")?;
    type_into(&mut fx, "Name", "Ada Lovelace")?;
    type_into(&mut fx, "Email", "blocked@example.com")?;
    // Send stays inert until the consent box is checked.
    click(&mut fx, "trusted-contact-send")?;
    ensure!(calls(&backend, "POST").is_empty(), "sent without consent");
    click(&mut fx, "trusted-contact-consent")?;
    click(&mut fx, "trusted-contact-send")?;
    fx.wait("blocked answer", |fx| {
        calls(&backend, "POST").len() == 1 && visible(fx, "trusted-contact-send")
    })?;
    shot(&mut fx, "blocked")?;
    type_into(&mut fx, "Email", "")?;
    fx.keys(&["backspace"; 19].join(" "))?;
    fx.type_text("ada@example.com")?;
    shot(&mut fx, "form")?;
    click(&mut fx, "trusted-contact-send")?;
    fx.wait("pending contact", |fx| {
        visible(fx, "trusted-contact-resend")
    })?;
    let (url, body) = calls(&backend, "POST").pop().context("POST")?;
    ensure!(url == "/trusted_contact", "{url}");
    ensure!(
        body == json!({"name": "Ada Lovelace", "email": "ada@example.com", "phone_number": null, "relationship": null}),
        "{body}"
    );
    shot(&mut fx, "pending")?;

    click(&mut fx, "trusted-contact-resend")?;
    fx.wait("resend", |_| !calls(&backend, "PUT").is_empty())?;
    let (url, body) = calls(&backend, "PUT").pop().context("PUT")?;
    ensure!(
        url == "/trusted_contact/tc-1" && body["email"] == "ada@example.com",
        "{url} {body}"
    );

    click(&mut fx, "trusted-contact-remove")?;
    fx.keys("tab enter")?;
    fx.wait("removed", |fx| visible(fx, "add-trusted-contact-button"))?;
    ensure!(calls(&backend, "DELETE") == [("/trusted_contact/tc-1".into(), Value::Null)]);
    shot(&mut fx, "removed")?;
    Ok(())
}
