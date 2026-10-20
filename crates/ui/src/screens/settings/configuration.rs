use super::*;

const APPROVAL_OPTIONS: &[(&str, &str)] = &[
    ("on-request", "On request"),
    ("never", "Never ask for approval"),
];
const SANDBOX_OPTIONS: &[(&str, &str)] = &[
    ("read-only", "Read only"),
    ("workspace-write", "Workspace write"),
    ("danger-full-access", "Full access"),
];

impl SettingsPage {
    pub(super) fn load_configuration(&mut self, cx: &mut Context<Self>) {
        if self.configuration_loaded
            || self.configuration_loading
            || self.configuration_error.is_some()
        {
            return;
        }
        let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
            return;
        };
        let bridge = bridge.clone();
        self.configuration_loading = true;
        cx.spawn(async move |page, cx| {
            let result = async {
                let config: Value = bridge
                    .request("config/read", json!({"includeLayers": true, "cwd": null}))
                    .await?;
                let requirements: Value =
                    bridge.request("configRequirements/read", json!({})).await?;
                anyhow::Ok((config, requirements))
            }
            .await;
            let _ = page.update(cx, |page, cx| {
                page.configuration_loading = false;
                match result {
                    Ok((config, requirements)) => {
                        for key in [
                            "approval_policy",
                            "sandbox_mode",
                            "sandbox_workspace_write",
                            "web_search",
                        ] {
                            page.values[key] = config["config"][key].clone();
                        }
                        page.values["origins"] = config["origins"].clone();
                        page.values["requirements"] = requirements["requirements"].clone();
                        page.configuration_loaded = true;
                    }
                    Err(error) => {
                        page.configuration_error =
                            Some(format!("Couldn't load configuration. {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn finish_config(
        &mut self,
        path: &'static str,
        value: Value,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.pending_config.retain(|key| *key != path);
        if let Some(error) = error {
            self.configuration_error = Some(error);
        } else {
            set_config_value(&mut self.values, path.trim_start_matches("config."), value);
        }
        cx.notify();
    }

    pub(super) fn config_option_allowed(&self, key: &str, value: &str) -> bool {
        if key == "config.web_search" && value == "disabled" {
            return true;
        }
        let restriction = match key {
            "config.approval_policy" => "allowedApprovalPolicies",
            "config.sandbox_mode" => "allowedSandboxModes",
            "config.web_search" => "allowedWebSearchModes",
            _ => return true,
        };
        self.values["requirements"][restriction]
            .as_array()
            .is_none_or(|allowed| allowed.iter().any(|option| option == value))
    }

    pub(super) fn config_lock(&self, key: &str) -> Option<&'static str> {
        if !self.configuration_loaded {
            return Some("Configuration is not available.");
        }
        let path = key.strip_prefix("config.").unwrap_or(key);
        let origins = &self.values["origins"];
        let origin = path
            .split_once('.')
            .map(|(table, _)| &origins[table])
            .filter(|origin| !origin.is_null())
            .unwrap_or(&origins[path]);
        if matches!(
            origin["name"]["type"].as_str(),
            Some(
                "mdm"
                    | "sessionFlags"
                    | "legacyManagedConfigTomlFromFile"
                    | "legacyManagedConfigTomlFromMdm"
            )
        ) {
            return Some("This value is managed by admin policy.");
        }
        let (options, message) = match key {
            "config.approval_policy" => (
                APPROVAL_OPTIONS,
                "Approval policy is restricted by this installation.",
            ),
            "config.sandbox_mode" => (
                SANDBOX_OPTIONS,
                "Sandbox settings are restricted by this installation.",
            ),
            _ => return None,
        };
        (!options
            .iter()
            .any(|(value, _)| self.config_option_allowed(key, value)))
        .then_some(message)
    }
}

impl SettingsPane {
    pub(super) fn configuration(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let mut rows = Vec::new();
        for (key, title, detail, options) in [
            (
                "config.approval_policy",
                "Approval policy",
                "Choose when ChatGPT asks for approval",
                APPROVAL_OPTIONS,
            ),
            (
                "config.sandbox_mode",
                "Sandbox settings",
                "Choose how much ChatGPT can do when running commands",
                SANDBOX_OPTIONS,
            ),
        ] {
            let lock = self.page.read(cx).config_lock(key);
            let control = self.configuration_select(key, options, t, window, cx);
            rows.push(settings_row(title, lock.or(Some(detail)), control, t));
        }
        rows.extend(self.configuration_network(t, window, cx));
        vec![("Permissions", rows)]
    }

    pub(super) fn configuration_select(
        &self,
        key: &'static str,
        options: &'static [(&'static str, &'static str)],
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.page.read(cx);
        if page.config_lock(key).is_some() || page.pending_config.contains(&key) {
            return select(
                config_label(key, page.choice(key, options[0].0), options),
                None,
                true,
                t,
            )
            .into_any_element();
        }
        let handle = focus::handle(key, 0, window, cx);
        let focus = handle.read(cx).handle.clone();
        focus::ring(
            div()
                .id(key)
                .relative()
                .child(self.select_menu(key, options, t, cx)),
            &handle,
            px(10.),
            px(2.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
            focus.focus(window, cx)
        })
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            focus::tab(event, window, cx);
            let key_name = event.keystroke.key.as_str();
            match key_name {
                "tab" => this.open_select = None,
                "escape" => this.open_select = None,
                "enter" | "space" if this.open_select == Some(key) => {
                    let value = options[this.config_menu_index].0;
                    this.page
                        .update(cx, |page, cx| page.set(key, json!(value), cx));
                    this.open_select = None;
                }
                "enter" | "space" | "down" | "up" | "home" | "end" => {
                    let was_open = this.open_select == Some(key);
                    this.open_select = Some(key);
                    let count = options.len();
                    let mut index = if !was_open || key_name == "home" {
                        0
                    } else if key_name == "end" {
                        count - 1
                    } else if key_name == "up" {
                        (this.config_menu_index + count - 1) % count
                    } else {
                        (this.config_menu_index + 1) % count
                    };
                    for _ in 0..count {
                        if this
                            .page
                            .read(cx)
                            .config_option_allowed(key, options[index].0)
                        {
                            break;
                        }
                        index = if key_name == "up" || key_name == "end" {
                            (index + count - 1) % count
                        } else {
                            (index + 1) % count
                        };
                    }
                    this.config_menu_index = index;
                }
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
        }))
        .into_any_element()
    }
}

pub(super) fn config_label(
    key: &str,
    value: &str,
    options: &[(&'static str, &'static str)],
) -> SharedString {
    if let Some((_, label)) = options.iter().find(|(option, _)| *option == value) {
        return (*label).into();
    }
    match (key, value) {
        ("config.approval_policy", "untrusted") => "Untrusted".into(),
        ("config.approval_policy", "on-failure") => "On failure".into(),
        _ if key.starts_with("config.") => value.to_owned().into(),
        _ => options[0].1.into(),
    }
}

pub(super) fn option_description(key: &str, value: &str) -> Option<&'static str> {
    match (key, value) {
        ("config.approval_policy", "on-request") => Some("Ask when escalation is requested"),
        ("config.approval_policy", "never") => {
            Some("Blocked actions fail instead of requesting approval")
        }
        ("config.sandbox_mode", "read-only") => Some("Can read files, but cannot edit them"),
        ("config.sandbox_mode", "workspace-write") => {
            Some("Can edit files, but only in this workspace")
        }
        ("config.sandbox_mode", "danger-full-access") => {
            Some("Can edit files outside this workspace")
        }
        ("config.web_search", "disabled") => Some("Don't allow web search"),
        ("config.web_search", "cached") => Some("Use OpenAI's maintained search index"),
        ("config.web_search", "indexed") => Some("Allow indexed external web access"),
        ("config.web_search", "live") => Some("Allow unrestricted, current web access"),
        _ => None,
    }
}
