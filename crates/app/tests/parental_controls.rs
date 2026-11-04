//! Settings ▸ Parental controls through the real `Root` against a stateful fake of the ChatGPT
//! backend's `/amphora` family service: invite a teen (creating the family), open the invited
//! member, resend and cancel the invitation.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, Identity, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn proxy() {
    common::proxy_child();
}

const ACCOUNT: &str = "00000000-0000-4000-8000-000000000000";
const USER: &str = "user-parent";

#[derive(Default)]
struct Backend {
    family: Mutex<Option<Vec<Value>>>,
    log: Mutex<Vec<(String, String, Value)>>,
    /// Set once the first family load has failed, as with no backend.
    loaded_once: AtomicBool,
}

impl Backend {
    fn answer(&self, method: &str, url: &str, body: &Value) -> (u16, Value) {
        let mut family = self.family.lock().unwrap();
        match (method, url) {
            ("GET", "/accounts/check/v4-2023-04-27") => (
                200,
                json!({"accounts": {ACCOUNT: {"account": {"structure": "personal"}}}}),
            ),
            ("GET", "/amphora") if !self.loaded_once.swap(true, Ordering::SeqCst) => {
                (500, json!({"detail": "unavailable"}))
            }
            ("GET", "/amphora") => match &*family {
                Some(_) => (200, json!({"id": "fam-1", "role": "owner"})),
                None => (200, json!({})),
            },
            ("GET", "/amphora/u18_graduation_unlink_setting_notices") => {
                (200, json!({"notices": []}))
            }
            ("GET", "/amphora/notifications?limit=20") => (200, json!({"items": []})),
            ("POST", "/amphora") if body["role"] == "parent" => {
                *family = Some(vec![json!({
                    "user_id": USER, "name": "Pat Parent", "email": "pat@example.com",
                    "role": "owner", "status": "accepted",
                })]);
                (200, json!({"id": "fam-1"}))
            }
            ("GET", "/amphora/fam-1/members") => {
                (200, json!({"items": family.clone().unwrap_or_default()}))
            }
            ("POST", "/amphora/fam-1/members") => {
                let member = &body["members"][0];
                family.as_mut().unwrap().push(json!({
                    "email": member["account_identifier"], "role": "member_role",
                    "status": "invited",
                }));
                (200, json!({}))
            }
            ("POST", "/amphora/fam-1/members/resend") => (200, json!({})),
            ("POST", "/amphora/fam-1/remove") => {
                family
                    .as_mut()
                    .unwrap()
                    .retain(|m| m["email"] != body["email"]);
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
            user_id: USER.into(),
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

fn posts(backend: &Backend) -> Vec<(String, Value)> {
    backend
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, url, _)| m == "POST" && url.starts_with("/amphora"))
        .map(|(_, url, body)| (url.clone(), body.clone()))
        .collect()
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_PARENTAL_CONTROLS_EVIDENCE", name)
}

#[test]
fn invite_resend_and_cancel() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/parental-controls").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    // The first load fails, as the original shows without a reachable backend.
    fx.wait("load error", |fx| visible(fx, "parental-controls-retry"))?;
    shot(&mut fx, "error")?;
    click(&mut fx, "parental-controls-retry")?;
    fx.wait("Add family member", |fx| {
        visible(fx, "parental-controls-add")
    })?;
    shot(&mut fx, "empty")?;

    click(&mut fx, "parental-controls-add")?;
    fx.wait("form", |fx| visible(fx, "parental-controls-identifier"))?;
    // Send stays inert until a role and an identifier are chosen.
    click(&mut fx, "parental-controls-send")?;
    ensure!(posts(&backend).is_empty(), "sent an empty form");
    click(&mut fx, "parental-controls-role-teen")?;
    click(&mut fx, "parental-controls-identifier")?;
    fx.type_text("not-an-email")?;
    click(&mut fx, "parental-controls-send")?;
    ensure!(posts(&backend).is_empty(), "sent an invalid email");
    fx.keys(&["backspace"; 12].join(" "))?;
    fx.type_text("kid@example.com")?;
    shot(&mut fx, "form")?;
    click(&mut fx, "parental-controls-send")?;
    fx.wait("invited member", |fx| {
        visible(fx, "parental-controls-member-1")
    })?;
    ensure!(
        posts(&backend)
            == [
                ("/amphora".into(), json!({"role": "parent"})),
                (
                    "/amphora/fam-1/members".into(),
                    json!({"members": [{"account_identifier": "kid@example.com", "role": "teen"}]})
                ),
            ],
        "{:?}",
        posts(&backend)
    );
    shot(&mut fx, "members")?;

    // Accepted members sort first, so the invite is the second row.
    click(&mut fx, "parental-controls-member-1")?;
    fx.wait("detail", |fx| visible(fx, "parental-controls-resend"))?;
    shot(&mut fx, "invited")?;
    click(&mut fx, "parental-controls-resend")?;
    fx.keys("tab enter")?;
    fx.wait("resend", |_| posts(&backend).len() == 3)?;
    ensure!(
        posts(&backend)[2]
            == (
                "/amphora/fam-1/members/resend".into(),
                json!({"account_identifier": "kid@example.com"})
            )
    );

    fx.wait("resent", |fx| {
        visible(fx, "parental-controls-cancel-invite")
    })?;
    click(&mut fx, "parental-controls-cancel-invite")?;
    fx.wait("cancelled", |fx| visible(fx, "parental-controls-add"))?;
    ensure!(
        posts(&backend).last()
            == Some(&(
                "/amphora/fam-1/remove".into(),
                json!({"email": "kid@example.com"})
            ))
    );
    ensure!(!visible(&mut fx, "parental-controls-member-1"));
    shot(&mut fx, "cancelled")?;
    Ok(())
}
