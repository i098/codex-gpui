//! Settings > Configuration (`configuration-panel`): the user `config.toml` agent defaults,
//! model features and the Codex dependencies feature.

use super::*;
use crate::settings_kit::{Choice, MultiSelect, Select, SelectionChanged};

/// `approval_policy` options, default first.
const APPROVAL: &[(&str, &str)] = &[
    ("on-request", "On request"),
    ("never", "Never ask for approval"),
    ("granular", "Per-category approval"),
];
const APPROVAL_DETAILS: &[(&str, &str)] = &[
    ("on-request", "Ask when escalation is requested"),
    (
        "never",
        "Blocked actions fail instead of requesting approval",
    ),
    ("granular", "Choose which actions can request approval"),
];
/// `sandbox_mode` options, default first.
const SANDBOX: &[(&str, &str)] = &[
    ("read-only", "Read only"),
    ("workspace-write", "Workspace write"),
    ("danger-full-access", "Full access"),
];
const SANDBOX_DETAILS: &[(&str, &str)] = &[
    ("read-only", "Can read files, but cannot edit them"),
    (
        "workspace-write",
        "Can edit files, but only in this workspace",
    ),
    (
        "danger-full-access",
        "Can edit files outside this workspace",
    ),
];
/// `web_search` options; the unset default follows the sandbox mode.
const WEB_SEARCH: &[(&str, &str)] = &[
    ("disabled", "Disabled"),
    ("cached", "Cached"),
    ("indexed", "Indexed"),
    ("live", "Live"),
];
const WEB_SEARCH_DETAILS: &[(&str, &str)] = &[
    ("disabled", "Don't allow web search"),
    ("cached", "Use OpenAI's maintained search index"),
    ("indexed", "Allow indexed external web access"),
    ("live", "Allow unrestricted, current web access"),
];
/// `model_verbosity` options; unset is the model default.
const VERBOSITY: &[(&str, &str)] = &[
    ("", "Model default"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
];
const VERBOSITY_DETAILS: &[(&str, &str)] = &[
    ("low", "Keep responses concise"),
    ("medium", "Balance detail and brevity"),
    ("high", "Include more detail in responses"),
];
/// `model_reasoning_summary` options, default first.
const SUMMARY: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("concise", "Concise"),
    ("detailed", "Detailed"),
    ("none", "None"),
];
const SUMMARY_DETAILS: &[(&str, &str)] = &[
    ("auto", "Let the model choose the summary detail"),
    ("concise", "Show a brief reasoning summary"),
    ("detailed", "Show a more detailed reasoning summary"),
    ("none", "Don't show reasoning summaries"),
];
/// Efforts model controls always show, listed checked and locked (original's `Me`; plus
/// `persistent` when a catalog model supports it).
const LOCKED_EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];
/// Efforts the menu lets the user hide, when a catalog model supports them (original's `Zn`).
const OPTIONAL_EFFORTS: [&str; 2] = ["max", "ultra"];
/// The `enabled-reasoning-efforts` host setting and its default.
const ENABLED_EFFORTS: &str = "host.enabled-reasoning-efforts";
const DEFAULT_ENABLED_EFFORTS: [&str; 6] =
    ["low", "medium", "high", "xhigh", "ultra", "persistent"];

/// The stored enabled efforts, or the default when unset.
fn enabled_efforts(page: &SettingsPage) -> Vec<String> {
    match page.get(ENABLED_EFFORTS).as_array() {
        Some(list) => list
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        None => DEFAULT_ENABLED_EFFORTS.map(String::from).to_vec(),
    }
}

/// A `config/read` layer `name` (`ConfigLayerSource`) as text.
fn layer_label(name: &Value) -> String {
    let kind = match name["type"].as_str().unwrap_or_default() {
        "user" => "User config",
        "project" => "Project config",
        "system" => "System config",
        "mdm" => "Managed preferences",
        "enterpriseManaged" => "Enterprise config",
        "sessionFlags" => "Launch flags",
        "packagedDefaults" => "Packaged defaults",
        _ => "Managed config",
    };
    let place = ["file", "dotCodexFolder", "name", "domain"]
        .iter()
        .find_map(|key| name[key].as_str());
    match place {
        Some(place) => format!("{kind} ({place})"),
        None => kind.to_owned(),
    }
}

/// Where `key`'s value comes from when a layer other than the user `config.toml` sets it:
/// native writes go to the user layer, so that layer's value may not take effect.
fn origin_note(origins: &Value, key: &str) -> Option<String> {
    let name = &origins[key]["name"];
    (name.is_object() && name["type"] != "user").then(|| format!("Set by {}", layer_label(name)))
}

impl SettingsPane {
    pub(super) fn render_configuration(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.page.read(cx);
        let sandbox = page.choice("sandbox_mode", "read-only").to_owned();
        let approval = page.get("approval_policy").clone();
        let origins = page.config_layers["origins"].clone();
        let ignored: Vec<(String, String)> = page.config_layers["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|layer| {
                let reason = layer["disabledReason"].as_str()?;
                Some((
                    format!("{} ignored", layer_label(&layer["name"])),
                    reason.to_owned(),
                ))
            })
            .collect();
        let enabled = enabled_efforts(page);
        let supports = |effort: &str| page.catalog_efforts.iter().any(|e| e == effort);
        let optional: Vec<&'static str> = OPTIONAL_EFFORTS
            .into_iter()
            .filter(|e| supports(e))
            .collect();
        let persistent = supports("persistent");
        // `tbn`: an unset web search follows the sandbox.
        let web_default = if sandbox == "danger-full-access" {
            "live"
        } else {
            "cached"
        };
        let row = |key: &str, title, detail: &'static str, control| {
            let detail = match origin_note(&origins, key) {
                None => description(detail, t).into_any_element(),
                Some(origin) => div()
                    .flex()
                    .flex_col()
                    .child(description(detail, t))
                    .child(text(origin, 12., 16., 430., t.text_secondary))
                    .into_any_element(),
            };
            settings_row_content(title, Some(detail), control, t)
        };
        let mut agent = vec![
            row(
                "approval_policy",
                "Approval policy",
                "Choose when ChatGPT asks for approval",
                self.approval_policy_menu(&approval, cx),
            ),
            row(
                "sandbox_mode",
                "Sandbox settings",
                "Choose how much ChatGPT can do when running commands",
                self.select_menu(
                    "sandbox_mode",
                    SANDBOX,
                    None,
                    |s| s.with_details(SANDBOX_DETAILS),
                    cx,
                ),
            ),
        ];
        if approval["granular"].is_object() {
            for (key, title) in [
                ("sandbox_approval", "Sandbox escalation"),
                ("rules", "Command rules"),
                ("mcp_elicitations", "MCP questions"),
                ("request_permissions", "Permission requests"),
                ("skill_approval", "Skill approval"),
            ] {
                let on = approval["granular"][key].as_bool().unwrap_or(false);
                let focus = focus::handle(key, 0, window, cx);
                let control = focus::ring(
                    switch(key, on, false, t).relative(),
                    &focus,
                    Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                )
                .role(gpui::Role::Switch)
                .aria_label(title)
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.toggle_approval_category(key, cx);
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_approval_category(key, cx);
                }))
                .into_any_element();
                agent.push(row(
                    "approval_policy",
                    title,
                    "Allow this category to ask for approval",
                    control,
                ));
            }
        }
        if sandbox == "workspace-write" {
            agent.push(row(
                "sandbox_workspace_write.network_access",
                "Allow network access",
                "Allow network access when the sandbox is set to workspace write",
                self.toggle(
                    "sandbox_workspace_write.network_access",
                    false,
                    t,
                    window,
                    cx,
                ),
            ));
            agent.push(row(
                "sandbox_workspace_write.writable_roots",
                "Additional writable folders",
                "Allow edits in these folders when using workspace write",
                self.writable_folders(t, window, cx),
            ));
        }
        agent.extend([
            row(
                "web_search",
                "Web search",
                "Choose how ChatGPT accesses the web",
                self.select_menu_or(
                    "web_search",
                    WEB_SEARCH,
                    web_default,
                    None,
                    |s| s.with_details(WEB_SEARCH_DETAILS),
                    cx,
                ),
            ),
            row(
                "model_verbosity",
                "Output detail",
                "Choose how much detail ChatGPT includes in responses",
                self.select_menu(
                    "model_verbosity",
                    VERBOSITY,
                    None,
                    |s| s.with_details(VERBOSITY_DETAILS),
                    cx,
                ),
            ),
            row(
                "model_reasoning_summary",
                "Reasoning summary",
                "Choose how ChatGPT summarizes its reasoning",
                self.select_menu(
                    "model_reasoning_summary",
                    SUMMARY,
                    None,
                    |s| s.with_details(SUMMARY_DETAILS),
                    cx,
                ),
            ),
            // The Codex CLI's local thinking display keys; they change what chats draw only.
            row(
                "hide_agent_reasoning",
                "Hide thinking",
                "Hide reasoning summaries in chats",
                self.toggle("hide_agent_reasoning", false, t, window, cx),
            ),
            row(
                "show_raw_agent_reasoning",
                "Show raw thinking",
                "Show the model's raw reasoning in place of its summary, when it sends any",
                self.toggle("show_raw_agent_reasoning", false, t, window, cx),
            ),
        ]);
        agent.extend(ignored.into_iter().map(|(title, reason)| {
            settings_row_content(
                title,
                Some(text(reason, 12., 16., 430., t.text_secondary).into_any_element()),
                div().into_any_element(),
                t,
            )
        }));
        // Original `Kn`: no row unless the catalog offers an optional or `persistent` effort.
        let features = (!optional.is_empty() || persistent).then(|| {
            let locked = LOCKED_EFFORTS
                .into_iter()
                .chain(persistent.then_some("persistent"))
                .map(|effort| (effort, true, true));
            let choices: Vec<Choice> = locked
                .chain(
                    optional
                        .iter()
                        .map(|&effort| (effort, enabled.iter().any(|e| e == effort), false)),
                )
                .map(|(effort, checked, disabled)| Choice {
                    value: effort.into(),
                    label: crate::composer::effort_label(effort).to_owned().into(),
                    checked,
                    disabled,
                })
                .collect();
            let values: Vec<String> = choices.iter().map(|c| c.value.to_string()).collect();
            let efforts = match &self.efforts {
                Some((known, select)) if *known == values => select.clone(),
                _ => {
                    let select = cx.new(|cx| {
                        MultiSelect::new("Available reasoning efforts", choices, cx)
                    });
                    cx.subscribe(&select, |this, _, changed: &SelectionChanged, cx| {
                        this.page.update(cx, |page, cx| {
                            let mut list = enabled_efforts(page);
                            list.retain(|e| *e != changed.value.as_ref());
                            if changed.checked {
                                list.push(changed.value.to_string());
                            }
                            page.set(ENABLED_EFFORTS, json!(list), cx);
                        });
                    })
                    .detach();
                    self.efforts = Some((values, select.clone()));
                    select
                }
            };
            vec![row(
                "",
                "Available reasoning efforts",
                "Choose which reasoning effort levels appear in model controls. Availability varies by model",
                efforts.into_any_element(),
            )]
        });
        let dependencies = vec![row(
            "features.workspace_dependencies",
            "Codex dependencies",
            "Allow ChatGPT to install and expose bundled Node.js and Python tools",
            self.toggle("features.workspace_dependencies", true, t, window, cx),
        )];
        // The scope menu: native settings edit only the user `config.toml` (`Codex`).
        let scope = self
            .selects
            .entry("configuration-scope")
            .or_insert_with(|| {
                let choices = vec![Choice {
                    value: "user".into(),
                    label: "Codex".into(),
                    checked: false,
                    disabled: false,
                }];
                cx.new(|cx| Select::new("user", choices, None, cx).with_label_weight(500.))
            })
            .clone();
        let open = div()
            .id("open-config-toml")
            .role(gpui::Role::Button)
            .aria_label("Open config.toml")
            .h(px(28.))
            .px(px(9.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(text("Open config.toml", 13., 18., 430., t.text_tertiary))
            .child(icon("arrow-up-right", 14., t.text_tertiary))
            .on_click({
                // The app opens the selected host's actual user layer (`config/read` layers).
                let page = self.page.clone();
                move |_, _, cx| page.update(cx, |_, cx| cx.emit(OpenConfigToml))
            });
        let header = |left: AnyElement, right: Option<AnyElement>| {
            div()
                .h(px(46.))
                .pb(px(6.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .child(left)
                .children(right)
        };
        let title = |label| text(label, 14., 21., 500., t.text).into_any_element();
        let section = |head: Div, rows| {
            div()
                .flex()
                .flex_col()
                .child(head)
                .child(super::worktrees::card(rows, Some(&self.layer), t))
        };
        let sections = [
            Some(section(
                header(scope.into_any_element(), Some(open.into_any_element())),
                agent,
            )),
            features.map(|rows| section(header(title("Model features"), None), rows)),
            Some(section(
                header(title("Workspace Dependencies"), None),
                dependencies,
            )),
        ];
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div().pt(px(16.)).pb(px(1.)).child(
                            div().w_full().max_w(px(768.)).mx_auto().px(px(20.)).child(
                                div()
                                    .py(px(12.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(text("Configuration", 28., 33.6, 500., t.text))
                                    .child(text(
                                        "Choose how ChatGPT works on your tasks",
                                        14.,
                                        24.,
                                        430.,
                                        t.text_secondary,
                                    )),
                            ),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(40.))
                            .children(sections.into_iter().flatten()),
                    ),
            )
            .into_any_element()
    }

    fn toggle_approval_category(&self, key: &str, cx: &mut Context<Self>) {
        self.page.update(cx, |page, cx| {
            let mut policy = page.get("approval_policy").clone();
            let Some(granular) = policy.get_mut("granular").and_then(Value::as_object_mut) else {
                return;
            };
            let on = granular.get(key).and_then(Value::as_bool).unwrap_or(false);
            granular.insert(key.to_owned(), json!(!on));
            page.set("approval_policy", policy, cx);
        });
    }

    fn approval_policy_menu(&mut self, policy: &Value, cx: &mut Context<Self>) -> AnyElement {
        let current = if policy["granular"].is_object() {
            "granular"
        } else {
            policy.as_str().unwrap_or("on-request")
        };
        let page = self.page.clone();
        let select = self.selects.entry("approval_policy").or_insert_with(|| {
            let choices = APPROVAL
                .iter()
                .map(|&(value, label)| Choice {
                    value: value.into(),
                    label: label.into(),
                    checked: false,
                    disabled: false,
                })
                .collect();
            let select =
                cx.new(|cx| Select::new(current, choices, None, cx).with_details(APPROVAL_DETAILS));
            cx.subscribe(
                &select,
                move |_, _, selected: &crate::settings_kit::Selected, cx| {
                    let value = if selected.0 == "granular" {
                        json!({"granular": {
                            "sandbox_approval": true, "rules": true, "mcp_elicitations": true,
                            "request_permissions": true, "skill_approval": true,
                        }})
                    } else {
                        json!(selected.0)
                    };
                    page.update(cx, |page, cx| page.set("approval_policy", value, cx));
                },
            )
            .detach();
            select
        });
        select.update(cx, |select, cx| select.set_value(current, cx));
        select.clone().into_any_element()
    }

    fn writable_folders(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roots = self
            .page
            .read(cx)
            .get("sandbox_workspace_write.writable_roots")
            .as_array()
            .cloned()
            .unwrap_or_default();
        let add_focus = focus::handle("add-writable-folder", 0, window, cx);
        let add = focus::ring(
            button("Add folder", t.text_alpha_5, None, t).id("add-writable-folder"),
            &add_focus,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        )
        .role(gpui::Role::Button)
        .aria_label("Add writable folder")
        .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(add_focus))
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.add_writable_folder(cx);
                cx.stop_propagation();
            }
        }))
        .on_click(cx.listener(|this, _, _, cx| this.add_writable_folder(cx)));
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(roots.iter().enumerate().map(|(index, path)| {
                let key_path = path.clone();
                let click_path = path.clone();
                let name = path.as_str().unwrap_or_default();
                let focus = focus::handle(("writable-root", index), 0, window, cx);
                focus::ring(
                    button(format!("{name}  ×"), t.text_alpha_5, None, t)
                        .id(("writable-root", index)),
                    &focus,
                    Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                )
                .role(gpui::Role::Button)
                .aria_label(format!("Remove writable folder {name}"))
                .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.remove_writable_folder(&key_path, cx);
                        cx.stop_propagation();
                    }
                }))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.remove_writable_folder(&click_path, cx)),
                )
            }))
            .child(add)
            .into_any_element()
    }

    fn remove_writable_folder(&self, path: &Value, cx: &mut Context<Self>) {
        self.page.update(cx, |page, cx| {
            let mut roots = page
                .get("sandbox_workspace_write.writable_roots")
                .as_array()
                .cloned()
                .unwrap_or_default();
            roots.retain(|root| root != path);
            page.set("sandbox_workspace_write.writable_roots", json!(roots), cx);
        });
    }

    fn add_writable_folder(&self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: None,
        });
        let page = self.page.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                page.update(cx, |page, cx| {
                    let mut roots = page
                        .get("sandbox_workspace_write.writable_roots")
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    for path in paths {
                        if let Some(path) = path.to_str().filter(|_| path.is_absolute()) {
                            let value = json!(path);
                            if !roots.contains(&value) {
                                roots.push(value);
                            }
                        }
                    }
                    page.set("sandbox_workspace_write.writable_roots", json!(roots), cx);
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_one_approval_category_preserves_the_others() {
        let mut app = gpui::HeadlessAppContext::with_asset_source(
            textsys::text_system(),
            std::sync::Arc::new(theme::Assets),
        );
        app.update(|cx| {
            let initial = json!({"granular": {
                "sandbox_approval": true, "rules": false, "mcp_elicitations": true,
                "request_permissions": false, "skill_approval": false,
            }});
            let integrations = cx.new(|_| integrations::State::default());
            let (_, page) = shell(
                &json!({"approval_policy": initial}),
                &json!({}),
                Some("Configuration"),
                integrations,
                cx,
            );
            let pane = cx.new(|cx| SettingsPane::new(page.clone(), cx));
            pane.update(cx, |pane, cx| pane.toggle_approval_category("rules", cx));
            pane.update(cx, |pane, cx| {
                pane.toggle_approval_category("request_permissions", cx)
            });
            let mut expected = initial;
            expected["granular"]["rules"] = json!(true);
            expected["granular"]["request_permissions"] = json!(true);
            assert_eq!(page.read(cx).get("approval_policy"), &expected);
        });
    }
}
