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

    app.keys("alt-cmd-u")?;
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
