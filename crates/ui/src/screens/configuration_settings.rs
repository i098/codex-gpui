//! Settings > Configuration (`configuration-panel`): the user `config.toml` agent defaults,
//! model features and the Codex dependencies feature.

use super::*;
use crate::settings_kit::{Choice, MultiSelect, Select};

/// `approval_policy` options, default first.
const APPROVAL: &[(&str, &str)] = &[
    ("on-request", "On request"),
    ("never", "Never ask for approval"),
];
const APPROVAL_DETAILS: &[(&str, &str)] = &[
    ("on-request", "Ask when escalation is requested"),
    (
        "never",
        "Blocked actions fail instead of requesting approval",
    ),
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
/// Reasoning efforts model controls always show (`hMt` plus `persistent`), listed checked and
/// locked in the `Available reasoning efforts` menu.
// shortcut: assumes a model supports `persistent` and none supports `max`/`ultra`; read
// `model/list` here when native settings receive it.
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "persistent"];

impl SettingsPane {
    pub(super) fn render_configuration(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.page.read(cx);
        let sandbox = page.choice("sandbox_mode", "read-only").to_owned();
        // `tbn`: an unset web search follows the sandbox.
        let web_default = if sandbox == "danger-full-access" {
            "live"
        } else {
            "cached"
        };
        let row = |title, detail, control| settings_row(title, Some(detail), control, t);
        let mut agent = vec![
            row(
                "Approval policy",
                "Choose when ChatGPT asks for approval",
                self.select_menu(
                    "approval_policy",
                    APPROVAL,
                    None,
                    |s| s.with_details(APPROVAL_DETAILS),
                    cx,
                ),
            ),
            row(
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
        if sandbox == "workspace-write" {
            agent.push(row(
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
        }
        agent.extend([
            row(
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
                "Hide thinking",
                "Hide reasoning summaries in chats",
                self.toggle("hide_agent_reasoning", false, t, window, cx),
            ),
            row(
                "Show raw thinking",
                "Show the model's raw reasoning in place of its summary, when it sends any",
                self.toggle("show_raw_agent_reasoning", false, t, window, cx),
            ),
        ]);
        let efforts = self
            .efforts
            .get_or_insert_with(|| {
                let choices = EFFORTS
                    .iter()
                    .map(|&effort| Choice {
                        value: effort.into(),
                        label: crate::composer::effort_label(effort).to_owned().into(),
                        checked: true,
                        disabled: true,
                    })
                    .collect();
                cx.new(|cx| MultiSelect::new("Available reasoning efforts", choices, cx))
            })
            .clone();
        let features = vec![row(
            "Available reasoning efforts",
            "Choose which reasoning effort levels appear in model controls. Availability varies by model",
            efforts.into_any_element(),
        )];
        let dependencies = vec![row(
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
            .on_click(|_, _, cx| {
                if let Ok(home) = crate::composer::codex_home() {
                    cx.open_with_system(&home.join("config.toml"));
                }
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
            section(
                header(scope.into_any_element(), Some(open.into_any_element())),
                agent,
            ),
            section(header(title("Model features"), None), features),
            section(header(title("Workspace Dependencies"), None), dependencies),
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
                            .children(sections),
                    ),
            )
            .into_any_element()
    }
}
