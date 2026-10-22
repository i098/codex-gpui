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
    local: app::host::LocalHost,
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
}

#[test]
fn activity_source_failure_is_visible_and_retry_recovers() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = std::sync::Arc::new(ActivityHost {
        local: app::host::LocalHost::new(home.path()),
        fail: std::sync::atomic::AtomicBool::new(true),
    });
    let mut app = Fx::new()?;
    app.root
        .update(&mut app.cx, |root, cx| root.set_host(host.clone(), cx));
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
