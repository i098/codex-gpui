//! A `https://chatgpt.com/library/share/*` link opened in the app reaches the invitation card;
//! Accept opens the shared folder and Decline returns to the Library.

mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::host::{Host, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use ui::confirmation::Outcome;
use ui::screens::space::library::Share;

/// An in-memory account with one pending folder invitation.
#[derive(Default)]
struct Backend {
    accepted: Mutex<bool>,
    log: Mutex<Vec<String>>,
}

impl Host for Backend {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(http) = request else {
            bail!("unexpected local request");
        };
        let method = http["method"].as_str().context("method")?;
        let url = http["url"].as_str().context("url")?;
        self.log.lock().unwrap().push(format!("{method} {url}"));
        let path = url.split('?').next().unwrap_or(url);
        let accepted = *self.accepted.lock().unwrap();
        let reply = match (method, path) {
            ("GET", "/spaces/v2") => json!({"items": [], "next_cursor": null}),
            ("GET", "/files/library/directories/path") if accepted => {
                json!({"items": [{"id": "dir-shared", "name": "Trip photos"}]})
            }
            ("GET", "/files/library/directories/path") => bail!("403"),
            ("GET", "/sharing/invitations") => json!({"invitations": [{
                "invitation_id": "inv-1", "status": "pending", "role_id": "role-library-folder-editor",
                "resource": {"resource_type": "library_folder", "resource_id": "dir-shared"},
                "resource_name": "Trip photos", "inviter_name": "Ada", "inviter_email": "ada@example.com"
            }], "next_cursor": null}),
            ("POST", "/sharing/invitations/inv-1/accept") => {
                *self.accepted.lock().unwrap() = true;
                json!({})
            }
            ("POST", "/sharing/invitations/inv-1/decline") => json!({}),
            ("GET", "/files/library/nodes") => json!({"items": [], "cursor": null}),
            _ => bail!("unexpected {method} {url}"),
        };
        Ok(json!({"status": 200, "body": reply.to_string()}))
    }
}

fn open(fx: &mut Fx, url: &str) -> Result<()> {
    let link = app::deep_link::parse(url).context("parses")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

fn shown(fx: &Fx) -> Option<Share> {
    fx.read(|root, cx| root.space_page()?.read(cx).library.as_ref()?.share.clone())
}

/// Answers the invitation dialog the page shows.
fn press(fx: &mut Fx, outcome: Outcome) -> Result<()> {
    fx.wait("the invitation dialog", |fx| {
        fx.read(|root, cx| {
            root.space_page()
                .is_some_and(|p| p.read(cx).confirmation().is_some())
        })
    })?;
    let dialog = fx
        .read(|root, cx| root.space_page()?.read(cx).confirmation().cloned())
        .context("dialog")?;
    fx.cx
        .update(|cx| dialog.update(cx, |_, cx| cx.emit(outcome)));
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn share_link_shows_invitation_then_accept_opens_the_folder() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    open(&mut fx, "https://chatgpt.com/library/share/dir-shared")?;
    fx.wait("the invitation card", |fx| {
        matches!(shown(fx), Some(Share::Invitation(_)))
    })?;
    fx.wait("the dialog", |fx| {
        fx.read(|root, cx| {
            root.space_page()
                .is_some_and(|p| p.read(cx).confirmation().is_some())
        })
    })?;
    fx.capture_evidence("CGX_LIBRARY_SHARE_EVIDENCE", "after-invitation-v2")?;
    press(&mut fx, Outcome::Confirm)?;
    fx.wait("the shared folder", |fx| {
        shown(fx).is_none()
            && fx.read(|root, cx| {
                root.space_page()
                    .and_then(|p| p.read(cx).library.as_ref().map(|l| l.path.clone()))
                    == Some(vec![("dir-shared".into(), "Trip photos".into())])
            })
    })?;
    fx.capture_evidence("CGX_LIBRARY_SHARE_EVIDENCE", "after-accept-v2")?;
    let log = backend.log.lock().unwrap();
    ensure!(
        log.iter()
            .any(|l| l == "POST /sharing/invitations/inv-1/accept"),
        "{log:#?}"
    );
    Ok(())
}

#[test]
fn share_link_decline_returns_to_the_library() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    open(&mut fx, "https://chatgpt.com/library/share/dir-shared")?;
    fx.wait("the invitation card", |fx| {
        matches!(shown(fx), Some(Share::Invitation(_)))
    })?;
    press(&mut fx, Outcome::Alternate)?;
    fx.wait("the Library", |fx| shown(fx).is_none())?;
    ensure!(
        backend
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "POST /sharing/invitations/inv-1/decline")
    );
    Ok(())
}
