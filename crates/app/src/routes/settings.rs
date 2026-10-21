use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenSettings, window, cx| {
            this.navigate(Route::Settings, window, cx)
        }),
    )
    .on_action(cx.listener(
        |this, _: &settings::integrations::OpenPluginManager, window, cx| {
            this.open_settings_section("Plugins", window, cx)
        },
    ))
    .on_action(cx.listener(|this, _: &LogOut, window, cx| {
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

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    root.load_computer_use(cx);
    root.load_settings_integrations(cx);
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
    shell
}
