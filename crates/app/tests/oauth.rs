mod common;

use anyhow::{Result, bail, ensure};
use app::host::{Host, Identity, Request};
use app::oauth::{ConnectRequest, ConnectionEvent};
use common::Fx;
use gpui::{AppContext as _, px, size};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct CallbackHost {
    identity: Mutex<Identity>,
    starts: AtomicUsize,
    callbacks: AtomicUsize,
    fail_callback: AtomicBool,
}

impl CallbackHost {
    fn new() -> Self {
        Self {
            identity: Mutex::new(Identity {
                account_id: "account".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            }),
            starts: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
            fail_callback: AtomicBool::new(false),
        }
    }
}

impl Host for CallbackHost {
    fn identity(&self) -> Option<Identity> {
        Some(self.identity.lock().unwrap().clone())
    }

    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(request) = request else {
            bail!("unexpected local request")
        };
        let body = match request["url"].as_str() {
            Some("/aip/connectors/links/oauth") => {
                let attempt = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
                json!({"redirect_url": format!("https://provider.example/authorize?state=attempt-{attempt}")})
            }
            Some("/aip/connectors/links/oauth/callback") => {
                self.callbacks.fetch_add(1, Ordering::SeqCst);
                if self.fail_callback.load(Ordering::SeqCst) {
                    bail!("provider callback failed");
                }
                json!({"link": {"id": "linked-account", "connector_id": "connector", "name": "Example"}})
            }
            _ => bail!("unexpected HTTP request"),
        };
        Ok(json!({"status": 200, "body": body.to_string()}))
    }
}

type Events = Arc<Mutex<Vec<ConnectionEvent>>>;

fn setup() -> Result<(Fx, Arc<CallbackHost>, Events)> {
    let mut fx = Fx::new()?;
    let host = Arc::new(CallbackHost::new());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    Ok((fx, host, Arc::new(Mutex::new(Vec::new()))))
}

fn start(fx: &mut Fx, events: &Events) -> Result<()> {
    let events = events.clone();
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.start_app_connection(
                ConnectRequest::new("connector", "Example"),
                window,
                cx,
                move |_, event, _, _| events.lock().unwrap().push(event.clone()),
            )
        });
    })?;
    fx.wait("provider URL", |fx| fx.cx.take_opened_url().is_some())
}

fn receive(fx: &mut Fx, state: &str, repeat: bool) -> Result<()> {
    let root = fx.root.clone();
    let url = format!("codex://connector/oauth_callback?code=code&state={state}");
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            assert!(root.receive_oauth_redirect(&url, window, cx));
            if repeat {
                assert!(root.receive_oauth_redirect(&url, window, cx));
            }
        });
    })?;
    Ok(())
}

#[test]
fn one_callback_claim_completes_once_and_does_not_persist_provider_data() -> Result<()> {
    let (mut fx, host, events) = setup()?;
    let route = fx.route();
    start(&mut fx, &events)?;
    receive(&mut fx, "attempt-1", true)?;
    fx.wait("callback completion", |_| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ConnectionEvent::Connected(_)))
    })?;
    ensure!(host.callbacks.load(Ordering::SeqCst) == 1);
    ensure!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, ConnectionEvent::Connected(_)))
            .count()
            == 1
    );
    ensure!(fx.route() == route);
    ensure!(fx.read(|root, _| root.oauth_page().is_none()));
    receive(&mut fx, "attempt-1", false)?;
    fx.cx.run_until_parked();
    ensure!(host.callbacks.load(Ordering::SeqCst) == 1);
    Ok(())
}

#[test]
fn native_callback_returns_to_the_window_that_started_oauth() -> Result<()> {
    let (mut fx, host, events) = setup()?;
    start(&mut fx, &events)?;
    let bridge = fx
        .cx
        .update(|cx| cx.global::<ui::screens::settings::AppServer>().0.clone());
    let unrelated_host = Arc::new(CallbackHost::new());
    let other_window = fx.cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        cx.new(|cx| {
            let mut root = app::Root::on_host(
                app::State::default(),
                "local".into(),
                bridge,
                "/tmp".into(),
                window,
                cx,
            )
            .unwrap();
            root.set_host(unrelated_host.clone(), cx);
            root
        })
    })?;
    let other = fx.cx.read_window(&other_window, |root, _| root)?;
    fx.cx.update(|cx| {
        app::deep_link::open(
            "codex://connector/oauth_callback?code=code&state=attempt-1",
            cx,
        );
    });
    fx.wait("owning window callback", |_| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ConnectionEvent::Connected(_)))
    })?;
    assert_eq!(host.callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(unrelated_host.callbacks.load(Ordering::SeqCst), 0);
    assert!(other.read_with(&fx.cx, |root, _| root.oauth_page().is_none()));
    Ok(())
}

#[test]
fn a_failed_callback_can_retry_but_unknown_and_replaced_states_cannot_connect() -> Result<()> {
    let (mut fx, host, events) = setup()?;
    start(&mut fx, &events)?;
    receive(&mut fx, "unknown", false)?;
    ensure!(host.callbacks.load(Ordering::SeqCst) == 0);
    host.fail_callback.store(true, Ordering::SeqCst);
    receive(&mut fx, "attempt-1", false)?;
    fx.wait("callback failure", |_| {
        events.lock().unwrap().contains(&ConnectionEvent::Failed)
    })?;
    host.fail_callback.store(false, Ordering::SeqCst);
    receive(&mut fx, "attempt-1", false)?;
    fx.wait("callback retry", |_| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ConnectionEvent::Connected(_)))
    })?;
    ensure!(host.callbacks.load(Ordering::SeqCst) == 2);
    start(&mut fx, &events)?;
    start(&mut fx, &events)?;
    receive(&mut fx, "attempt-2", false)?;
    ensure!(events.lock().unwrap().contains(&ConnectionEvent::Replaced));
    ensure!(host.callbacks.load(Ordering::SeqCst) == 2);
    Ok(())
}

#[test]
fn account_switch_and_cancellation_prevent_connection_side_effects() -> Result<()> {
    let (mut fx, host, events) = setup()?;
    start(&mut fx, &events)?;
    host.identity.lock().unwrap().account_id = "other-account".into();
    receive(&mut fx, "attempt-1", false)?;
    fx.wait("account-change rejection", |_| {
        events.lock().unwrap().contains(&ConnectionEvent::Failed)
    })?;
    ensure!(host.callbacks.load(Ordering::SeqCst) == 0);
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.cancel_app_connection("local", "connector", window, cx);
        })
    })?;
    receive(&mut fx, "attempt-1", false)?;
    fx.cx.run_until_parked();
    ensure!(host.callbacks.load(Ordering::SeqCst) == 0);
    ensure!(events.lock().unwrap().contains(&ConnectionEvent::Cancelled));
    Ok(())
}

#[test]
fn an_account_notification_discards_a_queued_callback_result() -> Result<()> {
    let (mut fx, host, events) = setup()?;
    start(&mut fx, &events)?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.receive_oauth_redirect(
                "codex://connector/oauth_callback?code=code&state=attempt-1",
                window,
                cx,
            );
            root.on_event(
                bridge::Event::Notification {
                    method: "account/updated".into(),
                    params: json!({"authMode": "chatgpt"}),
                },
                cx,
            );
        });
    })?;
    fx.wait("in-flight request", |_| {
        host.callbacks.load(Ordering::SeqCst) == 1
    })?;
    fx.cx.run_until_parked();
    ensure!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ConnectionEvent::Connected(_)))
    );
    ensure!(fx.read(|root, _| root.oauth_page().is_none()));
    ensure!(events.lock().unwrap().contains(&ConnectionEvent::Replaced));
    Ok(())
}

#[test]
fn an_old_callback_does_not_replace_the_active_connection_page() -> Result<()> {
    let (mut fx, host, events) = setup()?;
    start(&mut fx, &events)?;
    start(&mut fx, &events)?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.receive_oauth_redirect(
                "codex://connector/oauth_callback?code=code&state=attempt-2",
                window,
                cx,
            );
            root.receive_oauth_redirect(
                "codex://connector/oauth_callback?code=code&state=attempt-1",
                window,
                cx,
            );
            assert_eq!(
                root.oauth_page().unwrap().read(cx).state,
                ui::screens::oauth::CallbackState::Finishing
            );
        });
    })?;
    fx.wait("current callback", |_| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ConnectionEvent::Connected(_)))
    })?;
    ensure!(host.callbacks.load(Ordering::SeqCst) == 1);
    ensure!(fx.read(|root, _| root.oauth_page().is_none()));
    Ok(())
}
