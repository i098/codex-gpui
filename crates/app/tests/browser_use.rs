use app::Route;
use app::browser_use::{HandoffTarget, SettingsDestination, handoff_route, settings_route};

#[test]
fn handoff_keeps_connection_identity_and_decodes_query_values() {
    assert_eq!(
        handoff_route(
            "/browser/handoff?thread_id=source%2Fthread&tab_id=9007199254740991&connection_thread_id=owner"
        ),
        Some(Route::BrowserHandoff(Some(HandoffTarget {
            thread_id: "source/thread".into(),
            tab_id: "9007199254740991".into(),
            connection_thread_id: "owner".into(),
        })))
    );
    assert_eq!(
        handoff_route("/browser/handoff?thread_id=source&tab_id=1"),
        Some(Route::BrowserHandoff(Some(HandoffTarget {
            thread_id: "source".into(),
            tab_id: "1".into(),
            connection_thread_id: "source".into(),
        })))
    );
}

#[test]
fn malformed_handoffs_show_unavailable_instead_of_selecting_a_thread() {
    for query in [
        "",
        "thread_id=t",
        "tab_id=1",
        "thread_id=&tab_id=1",
        "thread_id=%20%EF%BB%BF&tab_id=1",
        "thread_id=t&tab_id=0",
        "thread_id=t&tab_id=01",
        "thread_id=t&tab_id=-1",
        "thread_id=t&tab_id=1.0",
        "thread_id=t&tab_id=9007199254740992",
        "thread_id=t&tab_id=18446744073709551616",
        "thread_id=t&tab_id=1&thread_id=other",
        "thread_id=t&tab_id=1&tab_id=2",
        "thread_id=t&tab_id=1&connection_thread_id=",
        "thread_id=t&tab_id=1&connection_thread_id=a&connection_thread_id=b",
        "thread_id=t&tab_id=1#fragment",
    ] {
        assert_eq!(
            handoff_route(&format!("/browser/handoff?{query}")),
            Some(Route::BrowserHandoff(None)),
            "{query}"
        );
    }
    assert_eq!(
        handoff_route("/browser/handoff"),
        Some(Route::BrowserHandoff(None))
    );
    assert_eq!(
        handoff_route("/browser/handoff#fragment"),
        Some(Route::BrowserHandoff(None))
    );
    assert_eq!(
        handoff_route("/browser/handoff/other?thread_id=t&tab_id=1"),
        None
    );
    assert_eq!(
        handoff_route("https://example.com/browser/handoff?thread_id=t&tab_id=1"),
        None
    );
}

#[test]
fn handoff_limits_identifiers_by_utf8_bytes() {
    for id in ["a".repeat(128), "é".repeat(64)] {
        assert!(matches!(
            handoff_route(&format!("/browser/handoff?thread_id={id}&tab_id=1")),
            Some(Route::BrowserHandoff(Some(_)))
        ));
    }
    for id in ["a".repeat(129), "é".repeat(65)] {
        assert_eq!(
            handoff_route(&format!("/browser/handoff?thread_id={id}&tab_id=1")),
            Some(Route::BrowserHandoff(None))
        );
        assert_eq!(
            handoff_route(&format!(
                "/browser/handoff?thread_id=t&tab_id=1&connection_thread_id={id}"
            )),
            Some(Route::BrowserHandoff(None))
        );
    }
}

#[test]
fn unavailable_browser_services_redirect_child_routes_to_their_settings_parent() {
    for path in [
        "/settings/browser-use",
        "/settings/browser-use/extensions",
        "/settings/browser-use/downloads?q=file",
        "/settings/browser-use/history?q=page",
        "/settings/browser-use/contact-info",
        "/settings/browser-use/passwords",
        "/settings/browser-use/site-settings",
        "/settings/browser-use/site-settings/camera",
    ] {
        assert_eq!(
            settings_route(path),
            Some(Route::BrowserSettings(SettingsDestination::Browser))
        );
    }
    for path in [
        "/settings/computer-use/chrome",
        "/settings/computer-use/google-chrome",
        "/settings/computer-use/messages",
        "/settings/computer-use/unknown",
    ] {
        assert_eq!(
            settings_route(path),
            Some(Route::BrowserSettings(SettingsDestination::ComputerUse))
        );
    }
    assert_eq!(settings_route("/settings/browser-use/unknown"), None);
    assert_eq!(
        settings_route("/settings/computer-use/chrome/unknown"),
        None
    );
}

#[test]
fn browser_routes_never_persist_link_identities_or_transient_pages() {
    use app::{State, ThemeSetting};
    let state = State {
        route: handoff_route("/browser/handoff?thread_id=private-thread&tab_id=1").unwrap(),
        theme: ThemeSetting::Dark,
        width: 840.,
        ..Default::default()
    };
    let saved = serde_json::to_value(&state).unwrap();
    assert_eq!(saved["route"], "Home");
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["width"], 840.);
    let settings = State {
        route: settings_route("/settings/browser-use/history").unwrap(),
        ..state
    };
    assert_eq!(
        serde_json::to_value(&settings).unwrap()["route"],
        "Settings"
    );
}

#[cfg(target_os = "macos")]
fn open_browser_path(fx: &mut common::Fx, path: &str) -> anyhow::Result<()> {
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_browser_path(path, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

#[cfg(target_os = "macos")]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn handoff_returns_home_without_editing_its_draft() -> anyhow::Result<()> {
    use common::Fx;
    let mut fx = Fx::with("dark", (840., 650.), None, |_| {})?;
    fx.type_text("Keep this draft")?;
    open_browser_path(&mut fx, "/browser/handoff?thread_id=missing&tab_id=1")?;
    assert!(matches!(fx.route(), Route::BrowserHandoff(Some(_))));
    fx.type_text("not a chat")?;
    assert_eq!(fx.composer_text()?, "Keep this draft");
    fx.capture_evidence("CGX_BROWSER_USE_SHOTS", "handoff-unavailable")?;
    fx.click((770., 36.))?;
    assert_eq!(fx.route(), Route::Home);
    assert_eq!(fx.composer_text()?, "Keep this draft");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn unavailable_browser_settings_keep_the_composer_inactive() -> anyhow::Result<()> {
    let mut fx = common::Fx::with("dark", (840., 650.), None, |_| {})?;
    fx.type_text("Keep this draft")?;
    open_browser_path(&mut fx, "/settings/browser-use/passwords")?;
    assert_eq!(
        fx.route(),
        Route::BrowserSettings(SettingsDestination::Browser)
    );
    fx.type_text("not a password")?;
    assert_eq!(fx.composer_text()?, "Keep this draft");
    fx.capture_evidence("CGX_BROWSER_USE_SHOTS", "browser-unavailable")?;
    fx.keys("cmd-[")?;
    assert_eq!(fx.route(), Route::Home);
    assert_eq!(fx.composer_text()?, "Keep this draft");
    Ok(())
}
