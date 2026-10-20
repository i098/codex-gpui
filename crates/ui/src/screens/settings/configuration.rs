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

const VERBOSITY_OPTIONS: &[(&str, &str)] = &[
    ("", "Model default"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
];
const SUMMARY_OPTIONS: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("concise", "Concise"),
    ("detailed", "Detailed"),
    ("none", "None"),
];

impl SettingsPage {
    pub fn set_model_features(&mut self, models: &[Model], cx: &mut Context<Self>) {
        self.supports_max_effort = false;
        self.supports_ultra_effort = false;
        self.supports_persistent_effort = false;
        for effort in models
            .iter()
            .flat_map(|model| &model.supported_reasoning_efforts)
        {
            match effort["reasoningEffort"].as_str() {
                Some("max") => self.supports_max_effort = true,
                Some("ultra") => self.supports_ultra_effort = true,
                Some("persistent") => self.supports_persistent_effort = true,
                _ => {}
            }
        }
        cx.notify();
    }

    pub fn finish_reasoning_efforts(&mut self, values: Value, cx: &mut Context<Self>) {
        self.pending_config
            .retain(|key| *key != "host.enabled-reasoning-efforts");
        self.refresh_host(values, cx);
    }

    pub fn set_dependency_state(&mut self, state: DependencyState, cx: &mut Context<Self>) {
        self.dependency_state = state;
        cx.notify();
    }

    pub fn dependencies_ready(&self) -> bool {
        self.dependency_state.operation.is_none()
            && (self.dependency_state.data.is_some() || self.dependency_state.error.is_some())
    }

    fn request_dependencies(&mut self, action: DependencyAction, cx: &mut Context<Self>) {
        if action == DependencyAction::Cancel {
            if self.dependency_state.operation != Some(DependencyAction::Reinstall)
                || self.dependency_state.cancel_pending
            {
                return;
            }
        } else if action != DependencyAction::Load
            && (self.dependency_state.operation.is_some()
                || !self.configuration_loaded
                || !self.flag("config.features.workspace_dependencies", true))
        {
            return;
        }
        cx.emit(action);
    }

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
        self.request_dependencies(DependencyAction::Load, cx);
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
                            "model_verbosity",
                            "model_reasoning_summary",
                            "features",
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
        for (key, title, detail, options) in [
            (
                "config.model_verbosity",
                "Output detail",
                "Choose how much detail ChatGPT includes in responses",
                VERBOSITY_OPTIONS,
            ),
            (
                "config.model_reasoning_summary",
                "Reasoning summary",
                "Choose how ChatGPT summarizes its reasoning",
                SUMMARY_OPTIONS,
            ),
        ] {
            rows.push(settings_row(
                title,
                self.page.read(cx).config_lock(key).or(Some(detail)),
                self.configuration_select(key, options, t, window, cx),
                t,
            ));
        }
        let mut groups = vec![("Codex", rows)];
        if let Some(group) = self.model_features(t, window, cx) {
            groups.push(group);
        }
        groups.push(self.workspace_dependencies(t, window, cx));
        groups
    }

    fn toggle_reasoning_effort(&mut self, effort: &'static str, cx: &mut Context<Self>) {
        let page = self.page.read(cx);
        let mut enabled = page
            .get("host.enabled-reasoning-efforts")
            .as_array()
            .cloned()
            .unwrap_or_else(|| {
                crate::composer::DEFAULT_REASONING_EFFORTS
                    .iter()
                    .map(|effort| json!(effort))
                    .collect()
            });
        if enabled.iter().any(|value| value == effort) {
            enabled.retain(|value| value != effort);
        } else {
            enabled.push(json!(effort));
        }
        self.page.update(cx, |page, cx| {
            page.set("host.enabled-reasoning-efforts", json!(enabled), cx)
        });
    }

    pub(super) fn ensure_reasoning_select(&mut self, cx: &mut Context<Self>) {
        if self.reasoning_select.is_some() {
            return;
        }
        let choices = reasoning_choices(self.page.read(cx));
        let disabled = self
            .page
            .read(cx)
            .pending_config
            .contains(&"host.enabled-reasoning-efforts");
        let select = cx.new(|cx| {
            MultiSelect::new("Available reasoning efforts", choices, cx)
                .menu_width(240.)
                .row_padding(5.)
                .menu_offset(30.)
        });
        if disabled {
            select.update(cx, |select, cx| select.set_disabled(true, cx));
        }
        cx.subscribe(
            &select,
            |this, _, change: &settings_kit::SelectionChanged, cx| {
                let effort = match change.value.as_ref() {
                    "max" => "max",
                    "ultra" => "ultra",
                    _ => return,
                };
                this.toggle_reasoning_effort(effort, cx);
            },
        )
        .detach();
        self.reasoning_select = Some(select);
    }

    pub(super) fn sync_reasoning_select(&self, cx: &mut Context<Self>) {
        let Some(select) = &self.reasoning_select else {
            return;
        };
        let page = self.page.read(cx);
        let choices = reasoning_choices(page);
        let disabled = page
            .pending_config
            .contains(&"host.enabled-reasoning-efforts");
        select.update(cx, |select, cx| {
            select.set_choices(choices, cx);
            select.set_disabled(disabled, cx);
        });
    }

    fn model_features(
        &self,
        t: &Theme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(&'static str, Vec<Div>)> {
        let page = self.page.read(cx);
        if !page.supports_max_effort
            && !page.supports_ultra_effort
            && !page.supports_persistent_effort
        {
            return None;
        }
        let control = self.reasoning_select.as_ref()?.clone().into_any_element();
        Some((
            "Model features",
            vec![settings_row(
                "Available reasoning efforts",
                Some(
                    "Choose which reasoning effort levels appear in model controls. Availability varies by model",
                ),
                control,
                t,
            )],
        ))
    }

    fn workspace_dependencies(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (&'static str, Vec<Div>) {
        let key = "config.features.workspace_dependencies";
        let page = self.page.read(cx);
        let busy =
            page.dependency_state.operation.is_some() || page.dependency_state.cancel_pending;
        let locked = page.config_lock(key).is_some() || busy || page.pending_config.contains(&key);
        let enabled = page.flag(key, true);
        let feature = if locked {
            switch(key, enabled, true, t).into_any_element()
        } else {
            self.toggle(key, true, t, window, cx)
        };
        let reinstalling =
            self.page.read(cx).dependency_state.operation == Some(DependencyAction::Reinstall);
        let cancelling = self.page.read(cx).dependency_state.cancel_pending;
        let mut button = |action: DependencyAction,
                          label: &'static str,
                          glyph: &'static str,
                          danger: bool,
                          disabled: bool,
                          cx: &mut Context<Self>| {
            let color = if danger { t.text_error } else { t.text };
            let fill = if danger {
                Hsla {
                    a: 0.1,
                    ..t.text_error
                }
            } else {
                t.text_alpha_5
            };
            let control = div()
                .id(label)
                .h(px(28.))
                .px(px(9.))
                .relative()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(on_card(12.5, Squircle, fill, None))
                .child(icon(glyph, 14., color).runtime())
                .child(text(label, 13., 18., 430., color));
            if disabled {
                return control.opacity(0.4).into_any_element();
            }
            let state = focus::handle(label, 0, window, cx);
            let handle = state.read(cx).handle.clone();
            focus::ring(control, &state, px(12.5), px(2.), cx)
                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    handle.focus(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page
                        .update(cx, |page, cx| page.request_dependencies(action, cx));
                }))
                .into_any_element()
        };
        let diagnose = button(
            DependencyAction::Diagnose,
            "Diagnose",
            "settings-diagnose",
            false,
            busy || !enabled,
            cx,
        );
        let repair = if reinstalling {
            button(
                DependencyAction::Cancel,
                "Cancel download",
                "close",
                true,
                cancelling,
                cx,
            )
        } else {
            button(
                DependencyAction::Reinstall,
                "Reinstall",
                "workspace-reinstall",
                true,
                busy || !enabled,
                cx,
            )
        };
        (
            "Workspace Dependencies",
            vec![
                settings_row(
                    "Codex dependencies",
                    Some("Allow ChatGPT to install and expose bundled Node.js and Python tools"),
                    feature,
                    t,
                ),
                settings_row(
                    "Diagnose issues in Codex Workspace",
                    Some("Checks the current bundle and records diagnostic logs"),
                    diagnose,
                    t,
                ),
                settings_row(
                    "Reset and install Workspace",
                    Some("Downloads a fresh bundle, installs it, and reloads tools"),
                    repair,
                    t,
                ),
            ],
        )
    }

    pub(super) fn dependency_footer(&self, t: &Theme, cx: &App) -> Div {
        let state = &self.page.read(cx).dependency_state;
        let version = state
            .data
            .as_ref()
            .and_then(|data| data["bundleVersion"].as_str())
            .unwrap_or(
                if state.operation == Some(DependencyAction::Load) && state.data.is_none() {
                    "Checking…"
                } else if state.error.is_some() {
                    "Unavailable"
                } else {
                    "Not installed"
                },
            );
        div()
            .px(px(16.))
            .pt(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(text("Current version:", 12., 16., 430., t.text_secondary))
                    .child(text(version.to_owned(), 12., 16., 430., t.text_secondary)),
            )
            .children(
                state
                    .error
                    .clone()
                    .map(|error| text(error, 12., 16., 430., t.text_error).whitespace_normal()),
            )
    }

    pub(super) fn configuration_heading(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let enabled = self.page.read(cx).configuration_loaded;
        let open = settings_kit::external_action_row(
            "open-config-toml",
            "Open config.toml",
            enabled,
            cx.listener(|this, _, _, cx| {
                this.page.update(cx, |_, cx| cx.emit(OpenConfigToml));
            }),
            window,
            cx,
        );
        let scope = self.user_config_scope(t, window, cx);
        div()
            .flex()
            .flex_col()
            .child(settings_kit::section_header("Agent defaults", t))
            .child(
                div()
                    .h(px(46.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(scope)
                    .child(open),
            )
    }

    fn user_config_scope(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const KEY: &str = "configuration-scope";
        let enabled = self.page.read(cx).configuration_loaded;
        let open = enabled && self.open_select == Some(KEY);
        let menu = open.then(|| {
            deferred(
                div().absolute().top_0().left_0().child(
                    anchored()
                        .anchor(Anchor::TopLeft)
                        .offset(point(px(0.), px(30.)))
                        .child(
                            div()
                                .id("configuration-scope-menu")
                                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                    this.menu_hovered = *hovered
                                }))
                                .child(popover(t, 240., |d| {
                                    d.child(div().px(px(8.)).py(px(4.)).child(text(
                                        "Global config",
                                        13.,
                                        18.571_428,
                                        430.,
                                        t.text_tertiary,
                                    )))
                                    .child(
                                        div()
                                            .id("user-config")
                                            .px(px(8.))
                                            .py(px(5.))
                                            .rounded(px(12.))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(text(
                                                "User config",
                                                13.,
                                                18.571_428,
                                                430.,
                                                t.text,
                                            ))
                                            .child(
                                                icon("settings-menu-check", 16., t.text)
                                                    .opacity(0.75),
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_select = None;
                                                this.menu_hovered = false;
                                                cx.notify();
                                            })),
                                    )
                                })),
                        ),
                ),
            )
        });
        let trigger = select_control(
            text("User config", 13., 18., 500., t.text),
            None,
            !enabled,
            if open {
                t.ghost_hover
            } else {
                t.primary_soft_alpha
            },
            t,
            icon("chevron-down", 14., t.text_tertiary).runtime(),
        )
        .id(KEY)
        .relative();
        if !enabled {
            return trigger.into_any_element();
        }
        let state = focus::handle(KEY, 0, window, cx);
        let handle = state.read(cx).handle.clone();
        focus::ring(trigger, &state, px(12.5), px(2.), cx)
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                handle.focus(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_select = (!open).then_some(KEY);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                focus::tab(event, window, cx);
                match event.keystroke.key.as_str() {
                    "escape" | "tab" => this.open_select = None,
                    "enter" | "space" => {
                        this.open_select = if this.open_select == Some(KEY) {
                            None
                        } else {
                            Some(KEY)
                        }
                    }
                    "down" | "up" | "home" | "end" => this.open_select = Some(KEY),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                if open && !this.menu_hovered {
                    this.open_select = None;
                    cx.notify();
                }
            }))
            .children(menu)
            .into_any_element()
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
            return select_control(
                text(
                    config_label(key, page.choice(key, options[0].0), options),
                    13.,
                    18.,
                    430.,
                    t.text,
                ),
                None,
                true,
                t.primary_soft_alpha,
                t,
                icon("chevron-down", 14., t.text_tertiary).runtime(),
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
                    let index = this.config_menu_index.unwrap_or_else(|| {
                        options
                            .iter()
                            .position(|(value, _)| {
                                this.page.read(cx).config_option_allowed(key, value)
                            })
                            .unwrap_or(0)
                    });
                    let value = options[index].0;
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
                    } else {
                        let current_index = this.config_menu_index.unwrap_or_else(|| {
                            options
                                .iter()
                                .position(|(value, _)| {
                                    this.page.read(cx).config_option_allowed(key, value)
                                })
                                .unwrap_or(0)
                        });
                        if key_name == "up" {
                            (current_index + count - 1) % count
                        } else {
                            (current_index + 1) % count
                        }
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
                    this.config_menu_index = Some(index);
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
        ("config.model_verbosity", "low") => Some("Keep responses concise"),
        ("config.model_verbosity", "medium") => Some("Balance detail and brevity"),
        ("config.model_verbosity", "high") => Some("Include more detail in responses"),
        ("config.model_reasoning_summary", "auto") => {
            Some("Let the model choose the summary detail")
        }
        ("config.model_reasoning_summary", "concise") => Some("Show a brief reasoning summary"),
        ("config.model_reasoning_summary", "detailed") => {
            Some("Show a more detailed reasoning summary")
        }
        ("config.model_reasoning_summary", "none") => Some("Don't show reasoning summaries"),
        _ => None,
    }
}

pub(super) fn subtitle(t: &Theme, window: &mut Window, cx: &mut App) -> AnyElement {
    const PREFIX: &str = "Configure permissions, web access, and agent responses for new chats ";
    const TEXT: &str =
        "Configure permissions, web access, and agent responses for new chats Learn more";
    const URL: &str = "https://developers.openai.com/codex/config-basic";
    let range = PREFIX.len()..TEXT.len();
    let styled = StyledText::new(TEXT).with_highlights(Some((
        range.clone(),
        HighlightStyle {
            color: Some(t.text_accent),
            ..Default::default()
        },
    )));
    let state = focus::handle("configuration-learn-more", 0, window, cx);
    let control = div()
        .id("configuration-learn-more")
        .relative()
        .text_size(px(14.))
        .line_height(px(24.))
        .font_weight(FontWeight(430.))
        .text_color(t.text_secondary)
        .child(
            InteractiveText::new("configuration-help-text", styled)
                .on_click(vec![range], |_, _, cx| cx.open_url(URL)),
        );
    focus::ring(control, &state, px(2.), px(2.), cx)
        .on_key_down(|event: &KeyDownEvent, window, cx| {
            focus::tab(event, window, cx);
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.open_url(URL);
                cx.stop_propagation();
            }
        })
        .into_any_element()
}

fn reasoning_choices(page: &SettingsPage) -> Vec<settings_kit::Choice> {
    let mut choices = Vec::with_capacity(7);
    for effort in crate::composer::DEFAULT_REASONING_EFFORTS[..4]
        .iter()
        .copied()
        .chain(page.supports_persistent_effort.then_some("persistent"))
    {
        choices.push(settings_kit::Choice {
            value: effort.into(),
            label: crate::composer::effort_label(effort).into(),
            checked: true,
            disabled: true,
        });
    }
    for (effort, supported) in [
        ("max", page.supports_max_effort),
        ("ultra", page.supports_ultra_effort),
    ] {
        if supported {
            choices.push(settings_kit::Choice {
                value: effort.into(),
                label: crate::composer::effort_label(effort).into(),
                checked: crate::composer::reasoning_effort_enabled(
                    effort,
                    page.get("host.enabled-reasoning-efforts"),
                ),
                disabled: false,
            });
        }
    }
    choices
}
