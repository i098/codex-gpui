//! `settings` parity screen: route `/settings/general-settings` (settings-page and
//! general-settings chunks) as refs/settings shows it, and the signed-out `/login` view
//! (login-route chunk) that replaces the app when `account/read` reports no account.

use crate::{
    MONO, SANS, SetTheme, Shell,
    composer::{popover, shadow},
    fixture,
    icon::icon,
    in_layer, layout_color, over, rounded_clip, sidebar_section, text,
};
use bridge::Bridge;
use bridge::types::{AccountReadResponse, ConfigReadResponse};
use gpui::{
    Anchor, AnyElement, AnyView, App, AppContext as _, Bounds, ClipboardItem, Context, Corners,
    Div, Entity, EventEmitter, FocusHandle, Font, FontWeight, Global, HighlightStyle, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render,
    RenderImage, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun,
    Window, anchored, canvas, deferred, div, font, img, point, prelude::FluentBuilder as _, px,
    size,
};
use serde_json::{Value, json};
use shapes::Corner::{self, Round, Squircle};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
use theme::{ActiveTheme as _, Theme};

/// The app's theme setting (`ThemeSetting`): `system`, `light` or `dark`.
pub const THEME: &str = "host.appearance-theme";

/// The live app-server connection. Settings and sign-in send their writes through it when
/// the app sets this global; the parity fixture runs without one.
pub struct AppServer(pub Arc<Bridge>);

impl Global for AppServer {}

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let account: AccountReadResponse = fixture::response("readback.jsonl", "account/read", 0);
    if account.account.is_none() {
        return cx.new(Login::new).into();
    }
    let config: ConfigReadResponse = fixture::response("readback.jsonl", "config/read", 0);
    let skills: Value = fixture::response("readback.jsonl", "skills/list", 0);
    let (shell, _) = shell(&config.config, &skills, None, cx);
    shell.update(cx, |shell, _| shell.can_go_back = true);
    shell.into()
}

/// The settings nav item `label` if it opens a page here (not in the browser).
pub fn section(label: &str) -> Option<&'static str> {
    NAV.iter()
        .flat_map(|(_, items)| items.iter())
        .map(|&(l, _)| l)
        .find(|&l| l == label && l != EXTERNAL)
}

/// The Settings route's shell for a `config/read` config and `skills/list` result: settings
/// nav in the sidebar panel, the page in the main panel, open at the nav item `section`
/// (`General` if `None`). The page emits [`SettingChanged`].
pub fn shell(
    config: &Value,
    skills: &Value,
    section: Option<&'static str>,
    cx: &mut App,
) -> (Entity<Shell>, Entity<SettingsPage>) {
    let mut page = SettingsPage::new(config, skills);
    if let Some(label) = section {
        page.section = label;
    }
    let page = cx.new(|_| page);
    let nav = cx.new(|cx| SettingsNav::new(page.clone(), cx)).into();
    let pane = cx.new(|cx| SettingsPane::new(page.clone(), cx)).into();
    let shell = cx.new(|_| {
        let mut shell = Shell::new(nav, pane);
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    });
    (shell, page)
}

/// Sends `method` to the app-server in the background; failures are logged.
fn send(method: &'static str, params: Value, cx: &App) {
    let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
        return;
    };
    let bridge = bridge.clone();
    cx.background_executor()
        .spawn(async move {
            if let Err(e) = bridge.request::<_, Value>(method, params).await {
                eprintln!("{method}: {e:#}");
            }
        })
        .detach();
}

/// Settings navigation (`gWa` order in app-initial): `(heading, [(label, icon)])`.
const NAV: &[(&str, &[(&str, &str)])] = &[
    (
        "Personal",
        &[
            ("General", "settings-general"),
            ("Notifications", "bell"),
            ("Import", "settings-import"),
            ("Profile", "settings-profile"),
            ("Appearance", "settings-appearance"),
            ("Account", "settings-profile"),
            ("Archived chats", "settings-archived-chats"),
            ("Parental controls", "settings-profile"),
            ("Trusted contact", "settings-profile"),
            ("Voice", "settings-voice"),
            ("Storage", "settings-storage"),
            ("Configuration", "settings-configuration"),
            ("Personalization", "settings-personalization"),
            ("Mini & Pets", "settings-mini-pets"),
            ("Keyboard shortcuts", "settings-keyboard-shortcuts"),
            ("Usage & billing", "settings-usage"),
            ("Data controls", "settings-data-controls"),
        ],
    ),
    (
        "Integrations",
        &[
            ("Plugins", "settings-plugins"),
            ("Passwords", "settings-passwords"),
            ("Computer use", "settings-computer-use"),
            ("Computer history", "settings-computer-history"),
            ("Appshots", "settings-appshots"),
            ("Browser", "settings-browser"),
            ("Cloud computer", "settings-cloud-computer"),
        ],
    ),
    (
        "Coding",
        &[
            ("Hooks", "settings-hooks"),
            ("Connections", "settings-connections"),
            ("Codex Cloud", "settings-codex-cloud"),
            ("Legacy Codex Cloud", "settings-legacy-codex-cloud"),
            ("Code Review", "settings-code-review"),
            ("Git", "settings-git"),
            ("Worktrees", "settings-worktrees"),
        ],
    ),
];

/// Opens in the browser instead of a settings page (trailing arrow in the nav).
const EXTERNAL: &str = "Profile";

/// State the nav and the page share: the open section, the search query and the settings.
///
/// The original keeps settings in three stores, addressed here by key path:
/// - `desktop.<key>` and `features.<name>`: `config.toml`, read by `config/read`, written
///   with `config/value/write`.
/// - `host.<kebab-key>`: the Electron host's settings file (`set-setting` host fetch).
///   The port has no host process, so these keep their value for the session only.
/// - `session.<name>`: rows whose store the original bundle does not expose by key (Full
///   access menu visibility, Bottom panel, Show predictions); session only.
pub struct SettingsPage {
    section: &'static str,
    query: String,
    /// `config/read` config, plus a `host` object for host settings.
    values: Value,
    /// `projectlessWorkspaceRoot` default: `~/Documents/Codex`.
    projectless_root: String,
}

impl SettingsPage {
    fn new(config: &Value, skills: &Value) -> Self {
        // The fixture's home directory, from the user skills under `~/.agents`.
        let home = skills["data"][0]["skills"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["path"].as_str()?.split_once("/.agents/"))
            .map(|(home, _)| home.to_string())
            .next()
            .unwrap_or_default();
        let projectless_root = config["desktop"]["projectlessWorkspaceRoot"]
            .as_str()
            .map_or_else(|| format!("{home}/Documents/Codex"), str::to_string);
        let values = config.clone();
        Self {
            section: "General",
            query: String::new(),
            values,
            projectless_root,
        }
    }

    fn get(&self, path: &str) -> &Value {
        let (store, key) = path.split_once('.').unwrap_or((path, ""));
        &self.values[store][key]
    }

    fn flag(&self, path: &str, default: bool) -> bool {
        self.get(path).as_bool().unwrap_or(default)
    }

    fn choice<'a>(&'a self, path: &str, default: &'a str) -> &'a str {
        self.get(path).as_str().unwrap_or(default)
    }

    /// Stores a setting and reports it ([`SettingChanged`]): the app persists config settings
    /// with `config/value/write` and keeps the others for the session.
    fn set(&mut self, path: &'static str, value: Value, cx: &mut Context<Self>) {
        let (store, key) = path.split_once('.').unwrap_or((path, ""));
        // `IndexMut` makes a missing store an object on its first key.
        self.values[store][key] = value.clone();
        cx.emit(SettingChanged { path, value });
        cx.notify();
    }
}

/// A settings row changed `path` (`store.key`, see [`SettingsPage`]) to `value`.
pub struct SettingChanged {
    pub path: &'static str,
    pub value: Value,
}

impl EventEmitter<SettingChanged> for SettingsPage {}

/// `--sidebar-navigation` background, opaque.
fn sidebar_color(t: &Theme) -> Hsla {
    over(t.sidebar_background, layout_color(t))
}

/// The settings sidebar: title, search field and the section list.
pub struct SettingsNav {
    page: Entity<SettingsPage>,
    focus: FocusHandle,
    /// Chromium's composited layer of the scrolling section list.
    list: Layer,
}

impl SettingsNav {
    fn new(page: Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        Self {
            page,
            focus: cx.focus_handle(),
            list: Layer::default(),
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.platform || k.modifiers.control {
            return;
        }
        let mut query = self.page.read(cx).query.clone();
        match (k.key.as_str(), &k.key_char) {
            ("backspace", _) => _ = query.pop(),
            ("escape", _) => query.clear(),
            (_, Some(c)) => query.push_str(c),
            _ => return,
        }
        self.page.update(cx, |p, cx| {
            p.query = query;
            cx.notify();
        });
        cx.stop_propagation();
    }

    fn search(&self, query: &str, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let (s, color) = if query.is_empty() {
            ("Search".to_string(), t.text_tertiary)
        } else {
            (query.to_string(), t.text)
        };
        div()
            .id("settings-search")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .h(px(36.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .relative()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(9999.)),
                fill: Some(t.list_hover),
                ..Default::default()
            }))
            .child(icon("search", 16., t.text_secondary))
            .child(text(s, 14., 18., 430., color))
    }

    fn item(
        &self,
        label: &'static str,
        glyph: &'static str,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let fill = over(t.list_hover, sidebar_color(t));
        div()
            .id(label)
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .relative()
            .rounded(Theme::RADIUS_LG)
            .when(selected, |d| {
                d.child(deco(&self.list, 12.5, Squircle, t.list_hover, None))
            })
            .hover(move |s| s.bg(fill))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(
                        glyph,
                        16.,
                        if selected { t.icon_primary } else { t.text },
                    ))
                    .child(text(label, 14., 21., 400., t.text)),
            )
            .when(label == EXTERNAL, |d| {
                d.child(icon("arrow-up-right", 16., t.text_secondary).opacity(0.5))
            })
            .when(label != EXTERNAL, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.page.update(cx, |p, cx| {
                        p.section = label;
                        cx.notify();
                    })
                }))
            })
    }
}

impl Render for SettingsNav {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let page = self.page.read(cx);
        let (section, query) = (page.section, page.query.clone());
        let needle = query.to_lowercase();
        let mut sections = Vec::new();
        for (heading, items) in NAV {
            let rows: Vec<_> = items
                .iter()
                .filter(|(label, _)| label.to_lowercase().contains(&needle))
                .map(|&(label, glyph)| self.item(label, glyph, label == section, &t, cx))
                .collect();
            if rows.is_empty() {
                continue;
            }
            sections.push(sidebar_section(
                *heading,
                div().flex().flex_col().gap(px(1.)).children(rows),
                cx,
            ));
        }
        let empty = sections.is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .px(px(8.))
                    .pb(px(8.))
                    .child(
                        div()
                            .h(px(52.))
                            .ml(px(8.))
                            .pr(px(4.))
                            .flex()
                            .items_center()
                            .child(
                                text("Settings", 18., 24., 600., t.text).font_family("OpenAI Sans"),
                            ),
                    )
                    .child(self.search(&query, &t, cx)),
            )
            .child(
                div()
                    .id("settings-nav")
                    .flex_1()
                    .overflow_y_scroll()
                    // A composited scroller: a cc layer of its own over the sidebar's (cc
                    // layers are flat), so its text blends onto the final sidebar pixel.
                    .top_layer()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .pt(px(4.))
                    .pb(px(8.))
                    .child(probe(&self.list))
                    .children(sections)
                    .when(empty, |d| {
                        d.child(div().px(px(16.)).py(px(8.)).child(text(
                            "No results found",
                            14.,
                            21.,
                            430.,
                            t.text_tertiary,
                        )))
                    }),
            )
    }
}

/// The open settings page in the main panel.
pub struct SettingsPane {
    page: Entity<SettingsPage>,
    /// The composited contents layer of the page's scroll container.
    layer: Layer,
    /// Key of the select whose options menu is open.
    open_select: Option<&'static str>,
    /// Pointer over that menu: a press there is not a click outside.
    menu_hovered: bool,
}

/// `Confirm before closing a window` options, default first.
const CLOSE_OPTIONS: &[(&str, &str)] = &[
    ("keyboardOnly", "Close shortcut only"),
    ("always", "Always"),
    ("never", "Never"),
];
/// `Send shortcut` options (`settings.general.enterBehavior.*`), default first.
const SEND_OPTIONS: &[(&str, &str)] = &[
    ("enter", "Enter"),
    ("cmdIfMultiline", "⌘ + Enter for multiline prompts"),
    ("cmdEnter", "⌘ + Enter always"),
];

impl SettingsPane {
    fn new(page: Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        Self {
            page,
            layer: Layer::default(),
            open_select: None,
            menu_hovered: false,
        }
    }

    /// A select for `key` with its options menu `(value, label)`; the first option is the
    /// default. A click outside closes the menu.
    fn select_menu(
        &self,
        key: &'static str,
        options: &'static [(&'static str, &'static str)],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.page.read(cx).choice(key, options[0].0).to_string();
        let label = options
            .iter()
            .find(|(value, _)| *value == current)
            .map_or(options[0].1, |(_, label)| label);
        let open = self.open_select == Some(key);
        let hover = t.menu_item_hover;
        let menu = open.then(|| {
            let rows = options.iter().map(|&(value, label)| {
                div()
                    .id(value)
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(12.))
                    .when(value == current, |d| d.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .child(text(label, 14., 20., 430., t.text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page.update(cx, |p, cx| p.set(key, json!(value), cx));
                        this.open_select = None;
                        this.menu_hovered = false;
                        cx.notify();
                    }))
            });
            // Below the trigger, or above it when the window's bottom edge is in the way.
            deferred(
                div().absolute().top_0().right_0().child(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(32.)))
                        .child(
                            div()
                                .id("select-menu")
                                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                    this.menu_hovered = *hovered
                                }))
                                .child(popover(t, 280., |d| d.children(rows))),
                        ),
                ),
            )
        });
        select(label, None, false, t)
            .id(key)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_select = (!open).then_some(key);
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

    fn toggle(
        &self,
        key: &'static str,
        default: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.page.read(cx).flag(key, default);
        switch(key, on, false, t)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.page
                    .update(cx, |p, cx| p.set(key, Value::Bool(!on), cx))
            }))
            .into_any_element()
    }

    /// Pill segmented control (`Bottom` / `Right`, `Queue` / `Steer`).
    fn segmented(
        &self,
        key: &'static str,
        default: &'static str,
        options: &'static [(&'static str, &'static str)],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.page.read(cx).choice(key, default).to_string();
        let fill = over(t.text_alpha_5, t.background_panel);
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .children(options.iter().map(|&(value, label)| {
                let selected = current == value;
                div()
                    .id(label)
                    .h(px(24.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .when(selected, |d| d.bg(fill))
                    .child(text(
                        label,
                        13.,
                        18.,
                        430.,
                        if selected { t.text } else { t.text_tertiary },
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.page.update(cx, |p, cx| p.set(key, json!(value), cx));
                        if key == THEME {
                            window.dispatch_action(Box::new(SetTheme(value)), cx);
                        }
                    }))
            }))
            .into_any_element()
    }

    /// Appearance ▸ Theme: the app applies and keeps the choice (`SetTheme`).
    /// ponytail: the chrome theme, font size and motion rows are not here; add them with the
    /// settings they drive.
    fn appearance(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<(&'static str, Vec<Div>)> {
        let options = &[("light", "Light"), ("dark", "Dark"), ("system", "System")];
        let theme = self.segmented(THEME, "system", options, t, cx);
        vec![("Theme", vec![settings_row("Theme", None, theme, t)])]
    }

    fn general(
        &self,
        t: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let page = self.page.read(cx);
        // The `w-48` path span middle-truncates to the width it has (`zLi`).
        let mono = Font {
            weight: FontWeight(430.),
            ..font(MONO)
        };
        let root = middle_truncate(&page.projectless_root, |s| {
            let run = TextRun {
                len: s.len(),
                font: mono.clone(),
                color: t.text_secondary,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(s.to_string().into(), px(12.), &[run], None);
            line.width <= px(192.)
        });
        let path = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .w(px(192.))
                    .overflow_hidden()
                    .child(text(root, 12., 16., 430., t.text_secondary).font_family(MONO)),
            )
            .child(button("Change", t.text_alpha_5, None, t))
            .into_any_element();
        let hotkey = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(text("Off", 13., 18.571_428, 430., t.text_secondary))
            .child(
                div()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("pencil", 16., t.text_tertiary)),
            )
            .into_any_element();
        let row = |title, description, control| settings_row(title, description, control, t);
        vec![
            (
                "Permissions",
                vec![
                    row(
                        "Default permissions",
                        Some(
                            "By default, ChatGPT can read and edit files in its workspace. It can ask for additional access when needed",
                        ),
                        // Default permissions are always shown.
                        switch("default-permissions", true, true, t).into_any_element(),
                    ),
                    row(
                        "Full access",
                        Some(
                            "When ChatGPT runs with full access, it can edit any file on your computer and run commands with network, without your approval. This significantly increases the risk of data loss, leaks, or unexpected behavior. <a>Learn more</a> about elevated risks.",
                        ),
                        self.toggle("session.fullAccess", true, t, cx),
                    ),
                ],
            ),
            (
                "General",
                vec![
                    row(
                        "Projectless task folder",
                        Some(
                            "The location where tasks started outside of projects store their data by default.",
                        ),
                        path,
                    ),
                    row(
                        "Default file open destination",
                        Some("Where files and folders open by default"),
                        select(
                            "Zed",
                            Some(app_icon(over(t.primary_soft_alpha, t.background_panel))),
                            false,
                            t,
                        )
                        .into_any_element(),
                    ),
                    row(
                        "Language",
                        Some("Language for the app UI"),
                        select("Auto detect", None, false, t).into_any_element(),
                    ),
                    row(
                        "Show in menu bar",
                        Some("Keep ChatGPT in the macOS menu bar when the main window is closed"),
                        self.toggle("host.mac-menu-bar-enabled", true, t, cx),
                    ),
                    row(
                        "Confirm before closing a window",
                        Some("Warn when a tab-close shortcut would close the window"),
                        self.select_menu("host.confirm-before-close", CLOSE_OPTIONS, t, cx),
                    ),
                    row(
                        "Full view by default",
                        Some("Start new tasks with Chat and content in a single tab strip"),
                        self.toggle("host.default-workspace-full-view", false, t, cx),
                    ),
                    row(
                        "Bottom panel",
                        Some("Show the bottom panel control in the app header"),
                        self.toggle("session.bottomPanel", true, t, cx),
                    ),
                    row(
                        "Default terminal location",
                        Some(
                            "Choose where the terminal shortcut and environment actions open terminal tabs",
                        ),
                        self.segmented(
                            "desktop.defaultTerminalLocation",
                            "bottom",
                            &[("bottom", "Bottom"), ("right", "Right")],
                            t,
                            cx,
                        ),
                    ),
                    row(
                        "Prevent sleep while running",
                        Some("Keep your computer awake while ChatGPT is running a task"),
                        self.toggle("desktop.preventSleepWhileRunning", false, t, cx),
                    ),
                    row(
                        "Compress local chat history",
                        Some(
                            "Save disk space by compressing older local chat history. Restart ChatGPT to apply changes.",
                        ),
                        self.toggle("features.local_thread_store_compression", false, t, cx),
                    ),
                    row(
                        "Speed",
                        Some(
                            "Choose how quickly ChatGPT runs across chats, subagents, and compaction",
                        ),
                        select("Standard", None, false, t).into_any_element(),
                    ),
                    row(
                        "Suggested prompts",
                        Some(
                            "Suggest what to do next by searching project files and connected apps",
                        ),
                        self.toggle("host.ambient-suggestions-enabled", true, t, cx),
                    ),
                    row(
                        "Open source licenses",
                        Some("Third-party notices for bundled dependencies"),
                        button("View", t.text_alpha_5, None, t).into_any_element(),
                    ),
                    row(
                        "Plugins",
                        Some("Allow ChatGPT to use installed plugins"),
                        self.toggle("features.plugins", true, t, cx),
                    ),
                ],
            ),
            (
                "Composer",
                vec![
                    row(
                        "Show predictions",
                        Some("Predict your next message based on the conversation"),
                        self.toggle("session.messagePredictions", true, t, cx),
                    ),
                    row(
                        "Plain text composer",
                        Some(
                            "Keep code, Markdown, and links as literal text while writing messages",
                        ),
                        self.toggle("desktop.composerPlainTextMode", false, t, cx),
                    ),
                    row(
                        "Show context window usage",
                        None,
                        self.toggle("host.show-context-window-usage", false, t, cx),
                    ),
                    row(
                        "Send shortcut",
                        Some("Choose when Enter sends a prompt or inserts a new line"),
                        self.select_menu("desktop.composerEnterBehavior", SEND_OPTIONS, t, cx),
                    ),
                    row(
                        "Follow-up behavior",
                        Some(
                            "Queue follow-ups while ChatGPT runs or steer the current run. Press Ctrl+⏎ to do the opposite for one message",
                        ),
                        self.segmented(
                            "desktop.followUpQueueMode",
                            "steer",
                            &[("queue", "Queue"), ("steer", "Steer")],
                            t,
                            cx,
                        ),
                    ),
                ],
            ),
            (
                "Popout Window",
                vec![
                    row(
                        "Popout Window hotkey",
                        Some(
                            "Set a global shortcut for Popout Window. Leave unset to keep it off.",
                        ),
                        hotkey,
                    ),
                    row(
                        "Default to standalone chat",
                        Some("Start new chats outside of any project"),
                        self.toggle(
                            "host.hotkey-window-projectless-default-enabled",
                            false,
                            t,
                            cx,
                        ),
                    ),
                ],
            ),
            (
                "Notifications",
                vec![
                    row(
                        "Notifications are off",
                        Some("Turn on notifications in System Settings to use the options below"),
                        button(
                            "Open System Settings",
                            t.primary_soft_alpha,
                            Some(t.border),
                            t,
                        )
                        .into_any_element(),
                    ),
                    row(
                        "Turn completion notifications",
                        Some("Set when ChatGPT alerts you that it's finished"),
                        select("Only when unfocused", None, true, t).into_any_element(),
                    ),
                    row(
                        "Enable permission notifications",
                        Some("Show alerts when notification permissions are required"),
                        switch("permissions-notifications", true, true, t).into_any_element(),
                    ),
                    row(
                        "Enable question notifications",
                        Some("Show alerts when input is needed to continue"),
                        switch("question-notifications", true, true, t).into_any_element(),
                    ),
                    row(
                        "Enable dot notifications",
                        Some("Show alerts when your dot sends you a message"),
                        switch("dot-notifications", true, true, t).into_any_element(),
                    ),
                    row(
                        "Notification sound",
                        Some("Sound for task completion, permission requests, and questions"),
                        select("Default", None, true, t).into_any_element(),
                    ),
                ],
            ),
            (
                "Toys",
                vec![
                    row(
                        "Confetti cannon",
                        Some("Let Codex fire confetti in the app when you ask!"),
                        self.toggle("host.toys-confetti-enabled", false, t, cx),
                    ),
                    row(
                        "Audio visualizer",
                        Some(
                            "Animate conversation rails with system audio. Audio is processed on your device, never saved or uploaded.",
                        ),
                        self.toggle("host.toys-audio-visualizer-enabled", false, t, cx),
                    ),
                ],
            ),
        ]
    }
}

impl Render for SettingsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let section = self.page.read(cx).section;
        // Pages other than General have no reference capture yet: title only, plus the Theme
        // control on Appearance.
        let groups = match section {
            "General" => self.general(&t, window, cx),
            "Appearance" => self.appearance(&t, cx),
            _ => Vec::new(),
        };
        let card_border = over(t.border, t.background_panel);
        let groups = groups.into_iter().map(|(title, rows)| {
            let last = rows.len() - 1;
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(46.))
                        .pb(px(6.))
                        .flex()
                        .items_center()
                        .child(text(title, 14., 21., 500., t.text)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .relative()
                        // The 1 px border's box (the border itself is in the decoration).
                        .p(px(1.))
                        .child(deco(
                            &self.layer,
                            20.,
                            Squircle,
                            t.background_panel,
                            Some(t.border),
                        ))
                        // `overflow: hidden`: the rows draw under the card's rounded clip.
                        .child(rounded_clip(
                            px(19.),
                            Squircle,
                            div()
                                .flex()
                                .flex_col()
                                .children(rows.into_iter().enumerate().map(|(i, r)| {
                                    r.when(i < last, |d| {
                                        // `after:inset-x-4 after:bottom-0 after:h-px
                                        // after:bg-border`
                                        d.child(
                                            div()
                                                .absolute()
                                                .left(px(16.))
                                                .right(px(16.))
                                                .bottom_0()
                                                .h(px(1.))
                                                .bg(card_border),
                                        )
                                    })
                                })),
                        )),
                )
        });
        let column = |d: Div| d.w_full().max_w(px(768.));
        div()
            .size_full()
            .flex()
            .flex_col()
            // Draggable toolbar row of the page (`h-toolbar`).
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
                            .pt(px(16.))
                            .pb(px(1.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                column(div()).px(px(20.)).child(
                                    div()
                                        .min_h(px(60.))
                                        .py(px(12.))
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .child(text(section, 28., 33.6, 500., t.text)),
                                ),
                            ),
                    )
                    .child(
                        div().flex().flex_col().items_center().child(
                            column(div())
                                .p(px(20.))
                                .flex()
                                .flex_col()
                                .gap(px(40.))
                                .children(groups),
                        ),
                    ),
            )
    }
}

/// `zLi`: `s` if it fits, else the longest `head…tail` (head `ceil(n/2)`, tail `floor(n/2)`
/// characters) that fits, found by binary search over `n`.
fn middle_truncate(s: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(s) {
        return s.into();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi, mut best) = (0, chars.len() as isize - 1, "…".to_string());
    while lo <= hi {
        let n = (lo + hi) as usize / 2;
        let head: String = chars[..n.div_ceil(2)].iter().collect();
        let tail: String = chars[chars.len() - n / 2..].iter().collect();
        let candidate = format!("{head}…{tail}");
        if fits(&candidate) {
            best = candidate;
            lo = n as isize + 1;
        } else {
            hi = n as isize - 1;
        }
    }
    best
}

/// One settings row: title, optional description (an `<a>…</a>` span renders as the inline
/// link) and the trailing control.
fn settings_row(
    title: &'static str,
    detail: Option<&'static str>,
    control: AnyElement,
    t: &Theme,
) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.))
        .px(px(16.))
        .py(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(text(title, 13., 18.571_428, 500., t.text))
                .children(detail.map(|s| description(s, t))),
        )
        .child(
            div()
                .flex_none()
                .min_w(px(160.))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .child(control),
        )
}

fn description(s: &'static str, t: &Theme) -> Div {
    let (plain, link) = match s.split_once("<a>") {
        Some((before, rest)) => {
            let (label, after) = rest.split_once("</a>").unwrap_or((rest, ""));
            let start = before.len();
            (
                format!("{before}{label}{after}"),
                Some(start..start + label.len()),
            )
        }
        None => (s.to_string(), None),
    };
    // `text()` without its string child.
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(FontWeight(430.))
        .text_color(t.text_secondary)
        .child(StyledText::new(plain).with_highlights(link.map(|r| {
            let color = Some(t.text_accent);
            (
                r,
                HighlightStyle {
                    color,
                    ..Default::default()
                },
            )
        })))
}

/// `Switch`: 32×20 track, 16 px thumb. `disabled` is the control's `opacity: .6`: Chromium
/// paints the track and thumb into one `saveLayerAlpha(153)`.
fn switch(id: &'static str, on: bool, disabled: bool, t: &Theme) -> Stateful<Div> {
    let track = surface(
        9999.,
        Round,
        if on {
            t.switch_track_checked
        } else {
            t.switch_track
        },
        None,
    );
    let thumb = shapes::Decoration {
        radius: Corners::all(px(9999.)),
        fill: Some(t.control_thumb),
        border: Some((px(1.), t.control_thumb)),
        shadows: vec![shadow(t.shadow_sm, 1., 2., -1., false)],
        ..Default::default()
    };
    let x = if on { 14. } else { 2. };
    div().id(id).flex_none().w(px(32.)).h(px(20.)).child(
        canvas(
            |_, _, _| {},
            move |b, (), window, cx| {
                let paint = |window: &mut Window| {
                    shapes::paint(&track, None, b, window, cx);
                    let at = Bounds::new(b.origin + point(px(x), px(2.)), size(px(16.), px(16.)));
                    shapes::paint(&thumb, None, at, window, cx);
                };
                if disabled {
                    window.with_skia_layer(153, paint)
                } else {
                    paint(window)
                }
            },
        )
        .size_full(),
    )
}

/// A settings surface through `shapes`: CSS `radius`, corner shape, fill and 1 px border.
fn deco(l: &Layer, radius: f32, corner: Corner, fill: Hsla, border: Option<Hsla>) -> AnyElement {
    let l = l.clone();
    in_layer(surface(radius, corner, fill, border), move || l.get()).into_any_element()
}

/// A control's surface on a settings card: Chromium blits it onto the card's opaque pixels in
/// the scroller's layer, as on the page (`shapes` takes the `.layer()` origin for its tiles).
fn on_card(radius: f32, corner: Corner, fill: Hsla, border: Option<Hsla>) -> AnyElement {
    shapes::decoration(surface(radius, corner, fill, border)).into_any_element()
}

/// The `shapes` paint of [`deco`] and [`on_card`].
fn surface(radius: f32, corner: Corner, fill: Hsla, border: Option<Hsla>) -> shapes::Decoration {
    shapes::Decoration {
        radius: Corners::all(px(radius)),
        corner,
        fill: Some(fill),
        border: border.map(|c| (px(1.), c)),
        ..Default::default()
    }
}

/// The origin (window px) of the composited layer Chromium paints a scroll container's
/// contents into (the scroller is `.layer()` for text). [`probe`] records it at prepaint.
type Layer = Rc<Cell<Point<Pixels>>>;

/// An empty first child of the layer's root that records the root's origin.
fn probe(l: &Layer) -> impl IntoElement {
    let l = l.clone();
    canvas(move |b, _, _| l.set(b.origin), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
}

/// Toolbar `_Button` (`Change`, `View`: `bg-text/5`, transparent 1 px border; `Open System
/// Settings`: outlined).
fn button(label: &'static str, fill: Hsla, border: Option<Hsla>, t: &Theme) -> Div {
    div()
        .h(px(28.))
        .px(px(9.))
        .flex()
        .items_center()
        .relative()
        .child(on_card(12.5, Squircle, fill, border))
        .child(text(label, 13., 18., 430., t.text))
}
/// Dropdown trigger: value (with an optional leading app icon) and a chevron.
fn select(label: &'static str, leading: Option<AnyElement>, disabled: bool, t: &Theme) -> Div {
    div()
        .h(px(28.))
        .px(px(13.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(4.))
        .relative()
        .child(on_card(
            12.5,
            Squircle,
            t.primary_soft_alpha,
            Some(t.border),
        ))
        .when(disabled, |d| d.opacity(0.4))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .children(leading)
                .child(text(label, 13., 18., 430., t.text)),
        )
        .child(icon("chevron-down", 14., t.text_tertiary))
}

/// The `Zed` app icon `<img>` (the app's 64 px `apps/zed.png` in an 18 px box) over the
/// opaque `page`. Chromium draws it at its unsnapped position with bilinear filtering; on
/// macOS Skia does the same (`shapes::raster_images`), drawn 1:1. Elsewhere GPUI scales it.
fn app_icon(page: Hsla) -> AnyElement {
    const PATH: &str = "icons/settings-zed.png";
    if !cfg!(target_os = "macos") {
        return img(PATH).size(px(18.)).flex_none().into_any_element();
    }
    // (box, page) -> the drawn device rect and its image.
    type Cached = (Bounds<Pixels>, Hsla, Bounds<Pixels>, Arc<RenderImage>);
    thread_local! {
        static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
    }
    canvas(
        move |bounds, window, cx| {
            if let Some((b, p, at, image)) = CACHE.with_borrow(|c| c.clone())
                && b == bounds
                && p == page
            {
                return Some((at, image));
            }
            let src = image::load_from_memory(&cx.asset_source().load(PATH).ok()??).ok()?;
            let mut src = src.to_rgba8();
            shapes::premultiply(&mut src);
            let image = Arc::new(shapes::raster_images(
                &[(&src, bounds)],
                bounds,
                Corners::default(),
                Round,
                page,
                window,
            )?);
            // The box's snapped device px rect, which `raster_images` fills.
            let s = window.scale_factor();
            let dev = |v: Pixels| (f32::from(v) * s).round();
            let (x, y) = (dev(bounds.left()), dev(bounds.top()));
            let (w, h) = (dev(bounds.right()) - x, dev(bounds.bottom()) - y);
            let at = Bounds::new(point(px(x / s), px(y / s)), size(px(w / s), px(h / s)));
            CACHE.set(Some((bounds, page, at, image.clone())));
            Some((at, image))
        },
        |_, image, window, _| {
            if let Some((at, image)) = image {
                window
                    .paint_image(at, at, Corners::default(), image, 0, false)
                    .ok();
            }
        },
    )
    .size(px(18.))
    .flex_none()
    .into_any_element()
}

/// Signed-out start (`LoginRoute`, welcome v2): ChatGPT sign-in, another way (API key), sign up.
pub struct Login {
    stage: Stage,
    api_key: String,
    focus: FocusHandle,
    error: Option<String>,
}

enum Stage {
    Welcome,
    /// Browser sign-in started: `(loginId, authUrl)`.
    Pending(String, String),
    OtherMethods,
    ApiKey,
}

impl Login {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            stage: Stage::Welcome,
            api_key: String::new(),
            focus: cx.focus_handle(),
            error: None,
        }
    }

    /// `account/login/completed`: a failed sign-in shows its error and returns to the start.
    pub fn apply(&mut self, event: &bridge::Event, cx: &mut Context<Self>) {
        let bridge::Event::Notification { method, params } = event else {
            return;
        };
        if method != "account/login/completed" || params["success"] != false {
            return;
        }
        let raw = params["error"].as_str().unwrap_or_default();
        self.stage = Stage::Welcome;
        self.error = Some(format!("Sign-in failed: {raw}"));
        cx.notify();
    }

    /// `account/login/start`; a ChatGPT login opens its `authUrl` and waits in `Pending`.
    fn start(&mut self, params: Value, cx: &mut Context<Self>) {
        let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
            return;
        };
        let bridge = bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .request::<_, Value>("account/login/start", params)
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(r) => {
                        if let (Some(id), Some(url)) =
                            (r["loginId"].as_str(), r["authUrl"].as_str())
                        {
                            cx.open_url(url);
                            this.stage = Stage::Pending(id.into(), url.into());
                        }
                    }
                    Err(e) => this.error = Some(format!("Sign-in failed: {e}")),
                }
                cx.notify();
            })
        })
        .detach();
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match (k.key.as_str(), &k.key_char) {
            ("backspace", _) => _ = self.api_key.pop(),
            ("enter", _) => self.submit_api_key(cx),
            (_, Some(c)) if !k.modifiers.platform && !k.modifiers.control => {
                self.api_key.push_str(c)
            }
            _ => return,
        }
        cx.notify();
    }

    fn submit_api_key(&mut self, cx: &mut Context<Self>) {
        if !self.api_key.is_empty() {
            let key = self.api_key.clone();
            self.start(json!({ "type": "apiKey", "apiKey": key }), cx);
        }
    }

    fn go(&mut self, stage: Stage, cx: &mut Context<Self>) {
        self.stage = stage;
        self.error = None;
        cx.notify();
    }
}

/// 46 px outlined pill button of the sign-in screen.
fn login_button(id: &'static str, label: &'static str, t: &Theme) -> Stateful<Div> {
    let hover = over(t.ghost_hover, t.surface);
    div()
        .id(id)
        .h(px(46.))
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded_full()
        .border_1()
        .border_color(over(t.border, t.surface))
        .bg(t.surface)
        .hover(move |s| s.bg(hover))
        .child(text(label, 14., 20., 500., t.text))
}

impl Render for Login {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let title = |s: &'static str| {
            text(s, 28., 36., 400., t.text)
                .w(px(316.))
                .whitespace_normal()
                .text_center()
        };
        let (heading, body): (Div, AnyElement) = match &self.stage {
            Stage::Welcome => (
                title("Sign in to ChatGPT"),
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("continue")
                            .h(px(48.))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(8.))
                            .rounded_full()
                            .bg(t.button_primary_solid)
                            .child(icon("codex-logo", 24., t.text_inverse))
                            .child(text("Continue to sign in", 14., 20., 500., t.text_inverse))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start(json!({ "type": "chatgpt" }), cx)
                            })),
                    )
                    .child(
                        login_button("another-way", "Sign in another way", &t).on_click(
                            cx.listener(|this, _, _, cx| this.go(Stage::OtherMethods, cx)),
                        ),
                    )
                    .child(
                        div()
                            .id("sign-up")
                            .h(px(36.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .child(text("Sign up", 14., 20., 500., t.text_description).underline())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start(json!({ "type": "chatgpt" }), cx)
                            })),
                    )
                    .into_any_element(),
            ),
            Stage::Pending(id, url) => {
                let (id, url) = (id.clone(), url.clone());
                (
                    title("Continue signing in with your browser"),
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(24.))
                        .child(
                            login_button("cancel", "Cancel sign-in", &t)
                                .h(px(42.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    send("account/login/cancel", json!({ "loginId": id }), cx);
                                    this.go(Stage::Welcome, cx);
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .justify_center()
                                .gap(px(8.))
                                .child(text(
                                    "Browser didn't open?",
                                    14.,
                                    20.,
                                    400.,
                                    t.text_description,
                                ))
                                .child(
                                    div()
                                        .id("copy-link")
                                        .child(
                                            text("Copy sign-in link", 14., 20., 500., t.text)
                                                .underline(),
                                        )
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                url.clone(),
                                            ))
                                        }),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Stage::OtherMethods => (
                title("Sign in another way"),
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        login_button("api-key", "OpenAI API Key", &t)
                            .on_click(cx.listener(|this, _, _, cx| this.go(Stage::ApiKey, cx))),
                    )
                    .child(
                        div()
                            .id("back")
                            .h(px(36.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .child(text("Back", 14., 20., 500., t.text_tertiary))
                            .on_click(cx.listener(|this, _, _, cx| this.go(Stage::Welcome, cx))),
                    )
                    .into_any_element(),
            ),
            Stage::ApiKey => {
                let (value, color) = if self.api_key.is_empty() {
                    ("sk-…".to_string(), t.text_tertiary)
                } else {
                    ("•".repeat(self.api_key.chars().count()), t.text)
                };
                (
                    title("Sign in with an API key"),
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(32.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(text("OpenAI API key", 14., 20., 500., t.text))
                                .child(
                                    div()
                                        .id("api-key-field")
                                        .track_focus(&self.focus)
                                        .on_key_down(cx.listener(Self::key_down))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.focus.focus(window, cx)
                                        }))
                                        .h(px(40.))
                                        .px(px(12.))
                                        .flex()
                                        .items_center()
                                        .rounded(Theme::RADIUS_LG)
                                        .border_1()
                                        .border_color(over(t.border, t.surface))
                                        .child(text(value, 14., 20., 430., color)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .child(login_button("api-cancel", "Cancel", &t).flex_1().on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.api_key.clear();
                                        this.go(Stage::OtherMethods, cx)
                                    }),
                                ))
                                .child(
                                    login_button("api-continue", "Continue", &t)
                                        .flex_1()
                                        .when(self.api_key.is_empty(), |d| d.opacity(0.5))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.submit_api_key(cx)),
                                        ),
                                ),
                        )
                        .into_any_element(),
                )
            }
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .pb(px(48.))
            .font_family(SANS)
            .bg(t.surface)
            .child(
                div()
                    .w(px(340.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(32.))
                    .child(icon("codex-logo", 40., t.text))
                    .child(heading)
                    .child(body)
                    .children(self.error.clone().map(|e| {
                        text(SharedString::from(e), 14., 20., 430., t.text_error)
                            .whitespace_normal()
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn middle_truncate_matches_reference() {
        // One cell per character, 26 cells wide (the reference's `w-48` at 12 px).
        let fits = |s: &str| s.chars().count() <= 26;
        let path = "/Users/user/Documents/Codex";
        assert_eq!(
            super::middle_truncate(path, fits),
            "/Users/user/D…uments/Codex"
        );
        assert_eq!(super::middle_truncate("/Users/user", fits), "/Users/user");
    }
}
