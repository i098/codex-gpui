use app::notifications::valid_preference;
use serde_json::json;
use ui::screens::settings::notifications::{DOT, PERMISSIONS, QUESTIONS, SOUND, TURN};

#[test]
fn rejects_invalid_host_preferences() {
    for key in [PERMISSIONS, QUESTIONS, DOT] {
        assert!(valid_preference(key, &json!(false)));
        assert!(!valid_preference(key, &json!("false")));
    }
    assert!(valid_preference(TURN, &json!("always")));
    assert!(!valid_preference(TURN, &json!("never")));
    assert!(!valid_preference(
        SOUND,
        &json!({"fileName":"../outside.wav", "name":"outside"})
    ));
    assert!(!valid_preference("appearance-theme", &json!("dark")));
}

#[cfg(target_os = "macos")]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn native_preferences_respect_permission_and_keep_confirmed_values_on_failure() -> anyhow::Result<()>
{
    use app::host_settings::Settings;
    use gpui::BorrowAppContext as _;
    use ui::screens::settings::notifications::{Action, State};
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("host-settings.json");
    let mut fx = common::Fx::new()?;
    fx.cx
        .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
    fx.keys("cmd-,")?;
    fx.wait("notification permission", |fx| {
        fx.read(|_, cx| cx.global::<State>().permission.is_some())
    })?;
    fx.scroll((1000., 700.), -1800.)?;
    fx.advance(std::time::Duration::from_millis(500))?;
    fx.capture_evidence("CGX_NOTIFICATIONS_EVIDENCE", "desktop-permission-disabled")?;
    let apply = |cx: &mut gpui::App, value| {
        let dispatch = cx.global::<State>().dispatch.clone().unwrap();
        dispatch(Action::SetPreference(QUESTIONS, value), cx);
    };
    fx.cx.update(|cx| {
        cx.update_global::<State, _>(|state, _| state.permission = Some(false));
        apply(cx, json!(false));
        assert!(!cx.global::<Settings>().read_all().contains_key(QUESTIONS));
        cx.update_global::<State, _>(|state, _| state.permission = Some(true));
        apply(cx, json!(false));
    });
    assert_eq!(Settings::load(path.clone())?.read_all()[QUESTIONS], false);
    fx.advance(std::time::Duration::ZERO)?;
    fx.advance(std::time::Duration::from_millis(200))?;
    fx.capture_evidence("CGX_NOTIFICATIONS_EVIDENCE", "desktop-enabled")?;
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    fx.cx.update(|cx| {
        apply(cx, json!(true));
        assert_eq!(cx.global::<Settings>().read_all()[QUESTIONS], false);
        assert_eq!(cx.global::<State>().desktop[QUESTIONS], false);
    });
    fx.advance(std::time::Duration::from_millis(200))?;
    fx.capture_evidence("CGX_NOTIFICATIONS_EVIDENCE", "preference-save-error")?;
    fx.advance(std::time::Duration::from_secs(6))?;
    fx.click(fx.at("settings", "Notifications")?)?;
    fx.wait("unavailable account gate", |fx| {
        fx.read(|_, cx| {
            cx.global::<State>().account
                == ui::screens::settings::notifications::AccountGate::Hidden
        })
    })?;
    fx.capture_evidence("CGX_NOTIFICATIONS_EVIDENCE", "chatgpt-unavailable-account")?;
    Ok(())
}
