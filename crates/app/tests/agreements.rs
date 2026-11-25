//! Settings ▸ Agreements and the Account page's medical credentials through the real `Root`
//! against a stateful fake of the ChatGPT backend: agree to the BAA of a HIPAA `quorum`
//! workspace, then fill in and save the CME profile.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, Identity, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[test]
fn proxy() {
    common::proxy_child();
}

const ACCOUNT: &str = "00000000-0000-4000-8000-000000000001";

#[derive(Default)]
struct Backend {
    signed_at: Mutex<Option<String>>,
    cme: Mutex<Value>,
    log: Mutex<Vec<(String, String, Value)>>,
}

impl Backend {
    fn answer(&self, method: &str, url: &str, body: &Value) -> (u16, Value) {
        let metadata = format!("/accounts/{ACCOUNT}/metadata");
        let sign = format!("/quorum/workspace/{ACCOUNT}/sign_baa");
        match (method, url) {
            ("GET", "/me") => (200, json!({"name": "Dr. Pat", "email": "pat@example.com"})),
            ("GET", "/accounts/check/v4-2023-04-27") => (
                200,
                json!({"accounts": {ACCOUNT: {"account": {
                    "plan_type": "quorum", "structure": "workspace",
                    "is_hipaa_compliant_workspace": true,
                }}}}),
            ),
            ("GET", u) if u == metadata => {
                let at = self.signed_at.lock().unwrap().clone();
                (
                    200,
                    json!({"workspace_baa_metadata": {"signed": at.is_some(), "timestamp": at}}),
                )
            }
            ("POST", u) if u == sign => {
                let at = "2026-11-25T10:00:00".to_owned();
                *self.signed_at.lock().unwrap() = Some(at.clone());
                (200, json!({"timestamp": at}))
            }
            ("GET", "/cme/account") => (200, self.cme.lock().unwrap().clone()),
            ("PATCH", "/cme/account") => {
                let mut saved = body.clone();
                saved["is_npi_editable"] = true.into();
                *self.cme.lock().unwrap() = saved.clone();
                (200, saved)
            }
            _ => (404, json!({"detail": "not found"})),
        }
    }

    fn writes(&self) -> Vec<(String, String, Value)> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, url, _)| m != "GET" && !url.starts_with("/wham/statsig/"))
            .cloned()
            .collect()
    }
}

impl Host for Backend {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(request) = request else {
            anyhow::bail!("not an HTTP request");
        };
        let method = request["method"].as_str().context("method")?.to_owned();
        let url = request["url"].as_str().context("url")?.to_owned();
        let body = request["body"]
            .as_str()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null);
        let (status, answer) = self.answer(&method, &url, &body);
        self.log.lock().unwrap().push((method, url, body));
        Ok(json!({"status": status, "bodyJsonString": answer.to_string()}))
    }

    fn identity(&self) -> Option<Identity> {
        Some(Identity {
            account_id: ACCOUNT.into(),
            user_id: "user-pat".into(),
            device_id: "device".into(),
        })
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

fn open(fx: &mut Fx, link: &str) -> Result<()> {
    let link = app::deep_link::parse(link).context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    Ok(())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_AGREEMENTS_EVIDENCE", name)
}

#[test]
fn agree_to_the_baa_and_save_medical_credentials() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend {
        cme: Mutex::new(json!({"npi": null, "degree": null, "is_npi_editable": true})),
        ..Default::default()
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));

    open(&mut fx, "codex://settings/agreements")?;
    fx.wait("Agree", |fx| visible(fx, "agreements-agree"))?;
    ensure!(visible(&mut fx, "agreements-services-view"));
    shot(&mut fx, "unsigned")?;
    click(&mut fx, "agreements-agree")?;
    fx.wait("agreed", |fx| visible(fx, "agreements-baa-agreed-on"))?;
    ensure!(visible(&mut fx, "agreements-baa-view"));
    ensure!(!visible(&mut fx, "agreements-agree"));
    ensure!(
        backend.writes()
            == [(
                "POST".into(),
                format!("/quorum/workspace/{ACCOUNT}/sign_baa"),
                Value::Null
            )],
        "{:?}",
        backend.writes()
    );
    shot(&mut fx, "signed")?;

    open(&mut fx, "codex://settings/account")?;
    fx.wait("medical credentials", |fx| {
        visible(fx, "medical-credentials-field-0")
    })?;
    // Save stays off until every field passes.
    click(&mut fx, "medical-credentials-field-0")?;
    fx.type_text("123-456-789")?;
    click(&mut fx, "medical-credentials-save")?;
    ensure!(backend.writes().len() == 1, "saved an invalid form");
    shot(&mut fx, "credentials-invalid")?;
    click(&mut fx, "medical-credentials-field-0")?;
    fx.type_text("0")?;
    for (i, value) in [(1, " MD "), (2, "de"), (3, "A123")] {
        click(&mut fx, &format!("medical-credentials-field-{i}"))?;
        fx.type_text(value)?;
    }
    click(&mut fx, "medical-credentials-eligible")?;
    click(&mut fx, "medical-credentials-save")?;
    fx.wait("saved", |_| backend.writes().len() == 2)?;
    ensure!(
        backend.writes()[1]
            == (
                "PATCH".into(),
                "/cme/account".into(),
                json!({
                    "npi": "1234567890", "degree": "MD", "medical_license_state": "DE",
                    "medical_license_number": "A123", "physician_eligible_for_cme": true,
                })
            ),
        "{:?}",
        backend.writes()
    );
    fx.wait("form reset", |fx| {
        visible(fx, "medical-credentials-field-0")
    })?;
    shot(&mut fx, "credentials-saved")?;
    Ok(())
}
