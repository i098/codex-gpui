use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenSettings, window, cx| {
            this.navigate(Route::Settings, window, cx)
        }),
    )
    .on_action(cx.listener(
        |this, action: &settings::usage::analytics::OpenChat, window, cx| {
            this.navigate(Route::Thread(action.0.clone()), window, cx);
        },
    ))
    .on_action(cx.listener(Root::open_settings_subtree))
    .on_action(cx.listener(Root::open_settings_parent))
    .on_action(
        cx.listener(|this, _: &settings::storage::ManageWorktrees, window, cx| {
            this.open_settings_section("Worktrees", window, cx)
        }),
    )
    .on_action(cx.listener(
        |this, _: &settings::storage::ManageArchivedChats, window, cx| {
            this.open_settings_section("Archived chats", window, cx)
        },
    ))
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
    root.load_voice(cx);
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
    let section = root
        .settings_section
        .take()
        .or_else(|| match &root.state.route {
            Route::CloudEnvironments(path)
                if path == "/cloud-environments/new" || path == "/settings/codex-cloud" =>
            {
                Some("Codex Cloud")
            }
            Route::CloudEnvironments(_) => Some("Legacy Codex Cloud"),
            _ => None,
        });
    let (shell, page) = settings::shell(
        &config,
        &root.skills,
        section,
        root.settings_integrations.clone(),
        cx,
    );
    page.update(cx, |page, _| page.set_toaster(root.toaster.clone()));
    let cloud = crate::cloud::attach(root, &page, window, cx);
    if let Route::CloudEnvironments(path) = &root.state.route {
        cloud.update(cx, |cloud, cx| cloud.open_path(path, cx));
    }
    root.settings_page = Some(page.clone());
    crate::settings_configuration::attach(root, &page, cx);
    if let Route::SettingsSubtree(path) = &root.state.route
        && let Some(view) = root.settings_subtree(path, cx)
    {
        page.update(cx, |page, cx| page.show_subtree(view, cx));
    }
    cx.subscribe_in(
        &page,
        window,
        |this, _, _: &settings::RequestFullAccessAvailable, window, cx| {
            this.open_dialog(DialogKind::FullAccessAvailable, window, cx)
        },
    )
    .detach();
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
    cx.subscribe(
        &page,
        |this, page, e: &settings::HostSettingsChanged, cx| {
            let pairs = e
                .0
                .iter()
                .map(|(path, value)| (path.strip_prefix("host.").unwrap_or(path), value.clone()));
            let result = match cx.try_global::<host_settings::Settings>() {
                Some(_) => cx
                    .global_mut::<host_settings::Settings>()
                    .write_batch(pairs),
                None => Err(anyhow::anyhow!("host settings could not be loaded")),
            };
            match result {
                Ok(()) => {
                    this.sync_terminal(cx);
                    host_settings::apply(cx);
                }
                Err(error) => {
                    eprintln!("codex-gpui: settings.write: {error:#}");
                    this.toast(Toast::danger("Couldn't save your changes. Try again"), cx);
                }
            }
            let mut values = cx
                .try_global::<host_settings::Settings>()
                .map(|settings| Value::Object(settings.read_all().clone()))
                .unwrap_or(Value::Null);
            values["appearance-theme"] = json!(this.state.theme);
            values["confirm-before-close"] = json!(this.state.confirm_before_close);
            page.update(cx, |page, cx| page.refresh_host(values, cx));
        },
    )
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
/// Quit asks about an unsaved instructions draft first, then runs the shell's guarded quit.
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
    crate::window_shell::request_quit(cx);
}
