use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenSettings, window, cx| {
            this.navigate(Route::Settings, window, cx)
        }),
    )
    .on_action(cx.listener(|this, _: &LogOut, window, cx| {
        if this.instructions_changed(cx) {
            this.confirm_instructions_leave(SettingsLeave::LogOut, window, cx);
            return;
        }
        if !this.logout_pending {
            let email = this.account_email.clone();
            let dialog = cx.new(|cx| Dialog::logout(email, cx));
            this.show_dialog(dialog, window, cx);
        }
    }))
    .on_action(cx.listener(|this, a: &SetTheme, window, cx| {
        let theme = serde_json::from_value(json!(a.0)).unwrap_or_default();
        this.set_theme(theme, window, cx)
    }))
}

pub(super) fn page(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    root.load_computer_use(cx);
    // The Appearance page shows the theme the app keeps.
    let mut config = root.config.clone();
    config["host"] = cx
        .try_global::<host_settings::Settings>()
        .map(|settings| Value::Object(settings.read_all().clone()))
        .unwrap_or(Value::Null);
    let (store, key) = settings::THEME.split_once('.').expect("store.key");
    config[store][key] = json!(root.state.theme);
    config["host"]["confirm-before-close"] = json!(root.state.confirm_before_close);
    let section = root.settings_section.take();
    let (shell, page) = settings::shell(&config, &root.skills, section, cx);
    root.settings_page = Some(page.clone());
    cx.subscribe(&page, |this, page, e: &SettingChanged, cx| {
        this.write_setting(e, cx);
        if e.path.starts_with("host.") && e.path != settings::THEME {
            let mut values = cx
                .try_global::<host_settings::Settings>()
                .map(|settings| Value::Object(settings.read_all().clone()))
                .unwrap_or(Value::Null);
            values["appearance-theme"] = json!(this.state.theme);
            values["confirm-before-close"] = json!(this.state.confirm_before_close);
            page.update(cx, |page, cx| page.refresh_host(values, cx));
        }
    })
    .detach();
    let editor = page.read(cx).personalization.clone();
    cx.subscribe_in(
        &editor,
        window,
        |this, _, event: &settings::personalization::Event, window, cx| {
            this.instructions_event(event, window, cx);
        },
    )
    .detach();
    cx.subscribe_in(
        &page,
        window,
        |this, _, event: &settings::personalization::Event, window, cx| {
            this.instructions_event(event, window, cx);
        },
    )
    .detach();
    shell
}
pub(crate) fn request_quit(cx: &mut App) {
    for window in cx.windows() {
        let Some(window) = window.downcast::<Root>() else {
            continue;
        };
        if window
            .read(cx)
            .is_ok_and(|root| root.instructions_changed(cx))
        {
            if let Err(error) = window.update(cx, |root, window, cx| {
                window.activate_window();
                root.confirm_instructions_leave(SettingsLeave::Quit, window, cx);
            }) {
                eprintln!("codex-gpui: confirm quit: {error:#}");
            }
            return;
        }
    }
    cx.quit();
}
