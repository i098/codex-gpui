mod common;

use anyhow::Result;
use common::{Fx, GREETING};

#[test]
fn activity_shortcut_preserves_the_thread_and_opens_the_sidebar() -> Result<()> {
    let mut app = Fx::new()?;
    app.keys("cmd-1")?;
    app.wait_thread(GREETING)?;
    app.action(Box::new(ui::ToggleSidebar))?;
    assert!(!app.sidebar_open());

    app.keys("alt-cmd-u")?;
    assert!(app.sidebar_open());
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    assert_eq!(app.route(), app::Route::Thread(GREETING.into()));

    app.action(Box::new(ui::ToggleSidebar))?;
    assert!(!app.sidebar_open());
    app.keys("alt-cmd-u")?;
    assert!(app.sidebar_open());
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    assert_eq!(app.route(), app::Route::Thread(GREETING.into()));

    app.action(Box::new(ui::OpenSettings))?;
    assert_eq!(app.route(), app::Route::Settings);
    app.keys("alt-cmd-u")?;
    assert_eq!(app.route(), app::Route::Home);
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    Ok(())
}

#[test]
fn logout_clears_the_activity_session_and_disables_its_shortcut() -> Result<()> {
    let mut lines = common::script(None, |_| {})?;
    lines.extend(common::exchange("account/logout", serde_json::json!({})));
    let mut app = Fx::scripted(lines)?;
    app.action(Box::new(ui::ToggleActivity))?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    app.action(Box::new(ui::LogOut))?;
    app.keys("tab enter")?;
    app.wait("sign-out", |app| app.read(|root, _| root.signed_out()))?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    app.keys("alt-cmd-u")?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    Ok(())
}

struct ActivityHost {
    local: common::ActivityFixtureHost,
    fail: std::sync::atomic::AtomicBool,
}

impl app::host::Host for ActivityHost {
    fn fetch(&self, request: &app::host::Request) -> Result<serde_json::Value> {
        if matches!(request, app::host::Request::Fetch { url, .. } if url.ends_with("/inbox-items"))
            && self.fail.load(std::sync::atomic::Ordering::SeqCst)
        {
            anyhow::bail!("activity source unavailable");
        }
        app::host::Host::fetch(&self.local, request)
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        app::host::Host::projects(&self.local)
    }

    fn identity(&self) -> Option<app::host::Identity> {
        app::host::Host::identity(&self.local)
    }
}

#[test]
fn activity_source_failure_is_visible_and_retry_recovers() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = std::sync::Arc::new(ActivityHost {
        local: common::ActivityFixtureHost(app::host::LocalHost::new(home.path())),
        fail: std::sync::atomic::AtomicBool::new(true),
    });
    let mut app = Fx::new()?;
    app.root
        .update(&mut app.cx, |root, cx| root.set_host(host.clone(), cx));
    app.wait("activity policy", |app| {
        app.read(|root, cx| root.sidebar().read(cx).activity_available)
    })?;
    app.action(Box::new(ui::ToggleActivity))?;
    app.wait("activity source error", |app| {
        app.read(|root, cx| root.sidebar().read(cx).activity_sources_state)
            == ui::screens::sidebar::ListState::Failed
    })?;
    host.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    app.action(Box::new(ui::screens::sidebar::RetryActivity))?;
    app.wait("activity source recovery", |app| {
        app.read(|root, cx| root.sidebar().read(cx).activity_sources_state)
            == ui::screens::sidebar::ListState::Ready
    })?;
    Ok(())
}

struct PolicyHost {
    local: app::host::LocalHost,
    enabled: bool,
    fail_next: std::sync::atomic::AtomicBool,
}

impl app::host::Host for PolicyHost {
    fn fetch(&self, request: &app::host::Request) -> Result<serde_json::Value> {
        if let app::host::Request::Http(http) = request {
            if http["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("/composer/items"))
            {
                anyhow::bail!("task suggestions are not under test");
            }
            assert_eq!(http["url"], "/wham/statsig/bootstrap");
            if self
                .fail_next
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                anyhow::bail!("temporary policy transport error");
            }
            let body = serde_json::json!({
                "statsigPayload": serde_json::json!({
                    "feature_gates": {"4039078146": {"value": self.enabled}}
                }).to_string()
            });
            return Ok(serde_json::json!({"status": 200, "body": body.to_string()}));
        }
        app::host::Host::fetch(&self.local, request)
    }

    fn identity(&self) -> Option<app::host::Identity> {
        Some(app::host::Identity {
            account_id: "activity-account".into(),
            user_id: "activity-user".into(),
            device_id: "activity-device".into(),
        })
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        app::host::Host::projects(&self.local)
    }
}

#[test]
fn disabled_activity_gate_rejects_keyboard_toggle() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = std::sync::Arc::new(PolicyHost {
        local: app::host::LocalHost::new(home.path()),
        enabled: false,
        fail_next: std::sync::atomic::AtomicBool::new(false),
    });
    let mut app = Fx::new()?;
    app.wait("activity policy", |app| {
        app.read(|root, cx| root.sidebar().read(cx).activity_available)
    })?;
    app.action(Box::new(ui::ToggleActivity))?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    app.root
        .update(&mut app.cx, |root, cx| root.set_host(host, cx));
    app.advance(std::time::Duration::from_millis(100))?;
    app.keys("alt-cmd-u")?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    assert!(!app.read(|root, cx| root.sidebar().read(cx).activity_available));
    assert!(!app.read(|_, cx| {
        cx.global::<ui::screens::settings::keyboard_shortcuts::KeyboardState>()
            .active_commands
            .contains("togglePriorityFilter")
    }));
    app.action(Box::new(ui::ToggleActivity))?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    app.action(Box::new(ui::OpenCommandMenu))?;
    app.type_text("Toggle activity view")?;
    app.keys("enter escape")?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    let enabled = std::sync::Arc::new(PolicyHost {
        local: app::host::LocalHost::new(home.path()),
        enabled: true,
        fail_next: std::sync::atomic::AtomicBool::new(true),
    });
    app.root
        .update(&mut app.cx, |root, cx| root.set_host(enabled.clone(), cx));
    app.wait("transient policy failure", |_| {
        !enabled.fail_next.load(std::sync::atomic::Ordering::SeqCst)
    })?;
    assert!(!app.read(|root, cx| root.sidebar().read(cx).activity_available));
    app.advance(std::time::Duration::from_secs(1))?;
    app.wait("restored activity policy", |app| {
        app.read(|root, cx| root.sidebar().read(cx).activity_available)
    })?;
    app.keys("alt-cmd-u")?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    app.root
        .update(&mut app.cx, |root, cx| root.set_host(enabled, cx));
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    Ok(())
}

#[test]
fn configured_activity_shortcut_replaces_the_default() -> Result<()> {
    use ui::screens::settings::keyboard_shortcuts::{Change, KeyboardState};
    let mut app = Fx::new()?;
    app.wait("activity policy", |app| {
        app.read(|root, cx| root.sidebar().read(cx).activity_available)
    })?;
    app.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(
            Change::Bindings {
                command: "togglePriorityFilter".into(),
                keys: vec!["Command+Shift+Y".into()],
            },
            cx,
        );
    });
    app.keys("alt-cmd-u")?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_none()));
    app.keys("cmd-shift-y")?;
    assert!(app.read(|root, cx| root.sidebar().read(cx).activity_clock.is_some()));
    Ok(())
}
