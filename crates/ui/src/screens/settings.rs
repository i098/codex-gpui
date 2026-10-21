//! `settings` parity screen: route `/settings/general-settings` (settings-page and
//! general-settings chunks) as refs/settings shows it, and the signed-out `/login` view
//! (login-route chunk) that replaces the app when `account/read` reports no account.

use crate::{
    MONO, SANS, SetTheme, Shell,
    composer::{Composer, shadow},
    fixture, focus,
    icon::icon,
    in_layer, layout_color,
    motion::{self, HoverFade},
    over, rounded_clip, sidebar_section, text,
};
use bridge::Bridge;
use bridge::types::{AccountReadResponse, ConfigReadResponse};
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Bounds, Context, Corners, Div, Entity, EventEmitter,
    FocusHandle, Focusable, Font, FontWeight, Global, HighlightStyle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, RenderImage, SharedString,
    Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun, Window, canvas, div, font,
    img, point, prelude::FluentBuilder as _, px, size,
};
use serde_json::{Value, json};
use shapes::Corner::{self, Round, Squircle};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
use theme::{ActiveTheme as _, Theme};

#[path = "computer_use.rs"]
pub mod computer_use;
#[path = "settings_general.rs"]
mod general;
#[path = "settings_route.rs"]
mod routing;
pub use routing::SettingsRoute;
#[path = "settings_profile.rs"]
mod profile;
#[path = "settings_search.rs"]
mod search;

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
    let (shell, page) = shell(&config.config, &skills, None, cx);
    page.update(cx, |page, cx| {
        page.targets = crate::screens::computer_use::Load::Ready(fixture::response(
            "settings-general.jsonl",
            "openIn.getTargets",
            0,
        ));
        page.set_requirements(
            Ok(fixture::response(
                "readback.jsonl",
                "configRequirements/read",
                0,
            )),
            matches!(
                account.account,
                Some(bridge::types::Account::Chatgpt { .. })
            ),
            cx,
        );
        page.root_loading = false;
        cx.notify();
    });
    shell.update(cx, |shell, _| shell.can_go_back = true);
    shell.into()
}

/// The settings nav item `label` if it opens a page here (not in the browser).
pub fn section(label: &str) -> Option<&'static str> {
    routing::SECTIONS
        .iter()
        .find(|(slug, title)| *slug == label || *title == label)
        .map(|(_, title)| *title)
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
        page.route = SettingsRoute::for_section(label).unwrap_or_else(|| page.route.clone());
        page.section = page.route.section;
    }
    let page = cx.new(|_| page);
    let nav = cx.new(|cx| SettingsNav::new(page.clone(), cx)).into();
    let pane = cx.new(|cx| SettingsPane::new(page.clone(), cx)).into();
    let mut profile = page.read(cx).section == "Profile";
    let shell = cx.new(|_| {
        let mut shell = Shell::new(nav, pane);
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell.sidebar_open = !profile;
        shell
    });
    shell.update(cx, |_, cx| {
        cx.observe(&page, move |shell, page, cx| {
            let next = page.read(cx).section == "Profile";
            if next != profile {
                profile = next;
                shell.sidebar_open = !profile;
                cx.notify();
            }
        })
        .detach();
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
/// - `host.<kebab-key>`: the desktop host's `settings.write(key, value)` service.
/// - `session.<name>`: rows whose store the original bundle does not expose by key (Full
///   access menu visibility, Bottom panel, Show predictions); session only.
pub struct SettingsPage {
    section: &'static str,
    query: String,
    /// `config/read` config, plus a `host` object for host settings.
    values: Value,
    /// `projectlessWorkspaceRoot` default: `~/Documents/Codex`.
    projectless_root: String,
    root_loading: bool,
    pending_instructions: Vec<SettingChanged>,
    route: SettingsRoute,
    folder_pending: bool,
    modal: Option<Entity<crate::confirmation::Confirmation>>,
    targets: crate::screens::computer_use::Load<Value>,
    notices: crate::screens::computer_use::Load<SharedString>,
    notices_requested: bool,
    pending: std::collections::HashSet<&'static str>,
    failed: std::collections::HashSet<String>,
    search_focus: Option<FocusHandle>,
    profile: crate::screens::computer_use::Load<Option<Value>>,
    profile_requested: bool,
    audio_available: bool,
    unsaved: bool,
    requirements: crate::screens::computer_use::Load<(Value, bool)>,
    search_select_all: bool,
    search_target: Option<(u64, String)>,
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
        let projectless_root = format!("{home}/Documents/Codex");
        let values = config.clone();
        Self {
            section: "General",
            query: String::new(),
            values,
            projectless_root,
            root_loading: true,
            pending_instructions: Vec::new(),
            route: SettingsRoute::for_section("General").expect("General route"),
            folder_pending: false,
            modal: None,
            targets: crate::screens::computer_use::Load::Loading,
            notices: crate::screens::computer_use::Load::Loading,
            notices_requested: false,
            pending: std::collections::HashSet::new(),
            failed: std::collections::HashSet::new(),
            search_focus: None,
            profile: crate::screens::computer_use::Load::Loading,
            profile_requested: false,
            audio_available: false,
            unsaved: false,
            requirements: crate::screens::computer_use::Load::Loading,
            search_select_all: false,
            search_target: None,
        }
    }

    fn get(&self, path: &str) -> &Value {
        match path.split_once('.') {
            Some((store, key)) => &self.values[store][key],
            None => &self.values[path],
        }
    }

    fn flag(&self, path: &str, default: bool) -> bool {
        self.get(path).as_bool().unwrap_or(default)
    }

    fn choice<'a>(&'a self, path: &str, default: &'a str) -> &'a str {
        self.get(path).as_str().unwrap_or(default)
    }

    /// Stores a setting and reports it to the app's config or host settings service.
    fn set(&mut self, path: &'static str, value: Value, cx: &mut Context<Self>) {
        if self.pending.contains(path) {
            return;
        }
        self.pending_instructions
            .retain(|change| change.path != path);
        match path.split_once('.') {
            Some((store, key)) => self.values[store][key] = value.clone(),
            None => self.values[path] = value.clone(),
        }
        self.pending.insert(path);
        self.failed.remove(path);
        cx.emit(SettingChanged { path, value });
        cx.notify();
    }

    pub fn refresh_host(&mut self, values: Value, cx: &mut Context<Self>) {
        self.values["host"] = values;
        cx.notify();
    }

    fn draft_instructions(&mut self, path: &'static str, value: String) {
        self.pending_instructions
            .retain(|change| change.path != path);
        if self.choice(path, "") != value {
            self.pending_instructions.push(SettingChanged {
                path,
                value: json!(value),
            });
        }
    }

    pub fn take_pending_instructions(&mut self) -> Vec<SettingChanged> {
        std::mem::take(&mut self.pending_instructions)
    }

    pub fn navigate(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let Some(route) = SettingsRoute::parse(path) else {
            return false;
        };
        self.section = route.section;
        self.route = route;
        cx.notify();
        true
    }

    pub fn route(&self) -> &SettingsRoute {
        &self.route
    }

    pub fn modal(&self) -> Option<Entity<crate::confirmation::Confirmation>> {
        self.modal.clone()
    }

    fn notify_error(&mut self, message: &'static str, cx: &mut Context<Self>) {
        cx.emit(SettingsNotice(message));
    }

    pub fn set_targets(&mut self, result: Result<Value, String>, cx: &mut Context<Self>) {
        self.targets = match result {
            Ok(value) => crate::screens::computer_use::Load::Ready(value),
            Err(error) => crate::screens::computer_use::Load::Failed(error),
        };
        cx.notify();
    }

    pub fn set_projectless_root(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        if let Ok(root) = result {
            self.projectless_root = root;
            self.root_loading = false;
            cx.notify();
        }
    }

    pub fn request_notices(&mut self) -> bool {
        if self.section != "Open source licenses" || self.notices_requested {
            return false;
        }
        self.notices_requested = true;
        true
    }

    pub fn set_notices(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.notices = match result {
            Ok(value) => crate::screens::computer_use::Load::Ready(value.into()),
            Err(error) => crate::screens::computer_use::Load::Failed(error),
        };
        cx.notify();
    }

    pub fn set_requirements(
        &mut self,
        result: Result<Value, String>,
        chatgpt: bool,
        cx: &mut Context<Self>,
    ) {
        self.requirements = match result {
            Ok(value) => crate::screens::computer_use::Load::Ready((value, chatgpt)),
            Err(error) => crate::screens::computer_use::Load::Failed(error),
        };
        cx.notify();
    }

    pub fn set_audio_available(&mut self, available: bool, cx: &mut Context<Self>) {
        self.audio_available = available;
        cx.notify();
    }

    pub fn audio_pending(&mut self, cx: &mut Context<Self>) {
        self.values["host"]["toys-audio-visualizer-enabled"] = json!(false);
        self.pending.insert("host.toys-audio-visualizer-enabled");
        cx.notify();
    }

    pub fn finish_write(&mut self, path: &str, value: Value, failed: bool, cx: &mut Context<Self>) {
        match path.split_once('.') {
            Some((store, key)) => self.values[store][key] = value,
            None => self.values[path] = value,
        }
        self.pending.remove(path);
        if failed {
            self.failed.insert(path.to_owned());
        } else {
            self.failed.remove(path);
        }
        cx.notify();
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_select_all = true;
        if let Some(focus) = &self.search_focus {
            focus.focus(window, cx);
            cx.notify();
        }
    }

    pub fn request_profile(&mut self) -> bool {
        if self.section != "Profile" || self.profile_requested {
            return false;
        }
        self.profile_requested = true;
        true
    }

    pub fn set_profile(&mut self, result: Result<Option<Value>, String>, cx: &mut Context<Self>) {
        self.profile = match result {
            Ok(profile) => crate::screens::computer_use::Load::Ready(profile),
            Err(error) => crate::screens::computer_use::Load::Failed(error),
        };
        cx.notify();
    }

    pub fn set_unsaved(&mut self, unsaved: bool, cx: &mut Context<Self>) {
        self.unsaved = unsaved;
        cx.notify();
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.unsaved
    }

    pub fn ask_discard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        proceed: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.unsaved {
            proceed(self, window, cx);
            return;
        }
        if self.modal.is_some() {
            return;
        }
        let dialog = cx.new(|cx| {
            crate::confirmation::Confirmation::new(
                "Discard changes?",
                "You have unsaved changes. If you leave now, your changes will be lost.",
                "Discard changes",
                window,
                cx,
            )
            .destructive()
            .cancel_label("Keep editing")
        });
        cx.subscribe_in(&dialog, window, move |page, _, outcome, window, cx| {
            page.modal = None;
            if *outcome == crate::confirmation::Outcome::Confirm {
                page.unsaved = false;
                page.pending_instructions.clear();
                cx.emit(SettingsDiscarded);
                proceed(page, window, cx);
            }
            cx.notify();
        })
        .detach();
        self.modal = Some(dialog);
        cx.notify();
    }

    fn request_navigation(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_discard(window, cx, move |page, _, cx| {
            page.search_target = None;
            page.navigate(&path, cx);
        });
    }

    fn request_search(
        &mut self,
        result: search::SearchResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_discard(window, cx, move |page, _, cx| {
            let serial = page
                .search_target
                .as_ref()
                .map_or(1, |(serial, _)| serial.wrapping_add(1));
            page.search_target = Some((serial, result.target.clone()));
            page.navigate(&result.route.path, cx);
        });
    }
}

/// A settings row changed `path` (`store.key`, see [`SettingsPage`]) to `value`.
pub struct SettingChanged {
    pub path: &'static str,
    pub value: Value,
}

pub struct SettingsNotice(pub &'static str);
impl EventEmitter<SettingsNotice> for SettingsPage {}

pub struct SettingsDiscarded;
impl EventEmitter<SettingsDiscarded> for SettingsPage {}

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
    rows: Vec<(&'static str, FocusHandle)>,
    results: Vec<search::SearchResult>,
    query_key: String,
    highlighted: Option<usize>,
    autofocused: bool,
}

impl SettingsNav {
    fn new(page: Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle().tab_stop(true);
        page.update(cx, |page, _| page.search_focus = Some(focus.clone()));
        let rows = NAV
            .iter()
            .flat_map(|(_, items)| items.iter())
            .map(|(label, _)| (*label, cx.focus_handle().tab_stop(true)))
            .collect();
        Self {
            page,
            focus,
            list: Layer::default(),
            rows,
            results: Vec::new(),
            query_key: String::new(),
            highlighted: None,
            autofocused: false,
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return;
        }
        let mut query = self.page.read(cx).query.clone();
        if !query.trim().is_empty() && matches!(k.key.as_str(), "up" | "down" | "enter") {
            self.refresh_search(&query);
            match k.key.as_str() {
                "enter" => {
                    if let Some(index) = self.highlighted {
                        self.activate_search(index, window, cx);
                    }
                }
                "down" if !self.results.is_empty() => {
                    self.highlighted = Some(
                        self.highlighted
                            .map_or(0, |index| (index + 1).min(self.results.len() - 1)),
                    )
                }
                "up" if !self.results.is_empty() => {
                    self.highlighted = Some(
                        self.highlighted
                            .map_or(self.results.len() - 1, |index| index.saturating_sub(1)),
                    )
                }
                _ => {}
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let selected = self.page.read(cx).search_select_all;
        match (k.key.as_str(), &k.key_char) {
            ("backspace", _) => {
                if selected {
                    query.clear();
                } else {
                    query.pop();
                }
            }
            ("escape", _) => query.clear(),
            (_, Some(c)) => {
                if selected {
                    query.clear();
                }
                query.push_str(c);
            }
            _ => return,
        }
        self.refresh_search(&query);
        self.page.update(cx, |page, cx| {
            page.query = query;
            page.search_select_all = false;
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
            .role(gpui::Role::SearchInput)
            .aria_label("Search settings")
            .aria_value(query.to_owned())
            .on_action(cx.listener(|this, _: &crate::SelectAll, _, cx| {
                this.page.update(cx, |page, cx| {
                    page.search_select_all = true;
                    cx.notify();
                });
                cx.stop_propagation();
            }))
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
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(text(s, 14., 18., 430., color).when(
                        self.page.read(cx).search_select_all && !query.is_empty(),
                        |d| d.bg(t.text_selection),
                    )),
            )
            .when(!query.is_empty(), |d| {
                d.child(
                    div()
                        .id("settings-search-clear")
                        .size(px(16.))
                        .role(gpui::Role::Button)
                        .aria_label("Clear settings search")
                        .focusable()
                        .tab_index(0)
                        .child(icon("close", 12., t.text_secondary))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.page.update(cx, |page, cx| {
                                page.query.clear();
                                page.search_select_all = false;
                                cx.notify();
                            });
                            this.highlighted = None;
                            this.focus.focus(window, cx);
                        })),
                )
            })
    }

    /// A nav row: `overflow: hidden`, so Skia draws a runtime icon (the viewport cuts the last
    /// one at 1440: no fitted image) under the row's AA clip; its own fill is outside it.
    fn item(
        &self,
        label: &'static str,
        glyph: &'static str,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let fill = over(t.list_hover, sidebar_color(t));
        let row = div()
            .id(label)
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(Theme::RADIUS_LG)
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
            .track_focus(
                &self
                    .rows
                    .iter()
                    .find(|(name, _)| *name == label)
                    .expect("registered navigation row")
                    .1,
            )
            .focus_visible(|style| style.border_2().border_color(t.focus_ring))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(route) = SettingsRoute::for_section(label) {
                    this.page.update(cx, |page, cx| {
                        page.request_navigation(route.path, window, cx)
                    });
                }
            }));
        div()
            .relative()
            .flex()
            .flex_col()
            .when(selected, |d| {
                d.child(deco(&self.list, 12.5, Squircle, t.list_hover, None))
            })
            .child(rounded_clip(Theme::RADIUS_LG, Squircle, row))
    }

    fn nav_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.platform && key.key == "f" {
            self.page
                .update(cx, |page, cx| page.focus_search(window, cx));
        } else if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt {
            let query = self.page.read(cx).query.to_lowercase();
            let visible = self
                .rows
                .iter()
                .filter(|(label, _)| label.to_lowercase().contains(&query))
                .collect::<Vec<_>>();
            let current = visible
                .iter()
                .position(|(_, focus)| focus.is_focused(window));
            match key.key.as_str() {
                "up" | "down" if !visible.is_empty() => {
                    let index = match (current, key.key.as_str()) {
                        (Some(index), "up") => index.saturating_sub(1),
                        (Some(index), _) => (index + 1).min(visible.len() - 1),
                        (None, "up") => visible.len() - 1,
                        _ => 0,
                    };
                    visible[index].1.focus(window, cx);
                    if let Some(route) = SettingsRoute::for_section(visible[index].0) {
                        self.page.update(cx, |page, cx| {
                            page.request_navigation(route.path, window, cx)
                        });
                    }
                }
                "enter" | "space" => {
                    let Some(index) = current else {
                        return;
                    };
                    let label = visible[index].0;
                    if let Some(route) = SettingsRoute::for_section(label) {
                        self.page.update(cx, |page, cx| {
                            page.request_navigation(route.path, window, cx)
                        });
                    }
                }
                _ => return,
            }
        } else {
            return;
        }
        cx.stop_propagation();
    }
}

impl Render for SettingsNav {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.autofocused {
            self.autofocused = true;
            self.focus.focus(window, cx);
        }
        let t = cx.theme().clone();
        let page = self.page.read(cx);
        let (section, query) = (page.section, page.query.clone());
        self.refresh_search(&query);
        let mut sections = Vec::new();
        if query.trim().is_empty() {
            for (heading, items) in NAV {
                let rows = items
                    .iter()
                    .map(|&(label, glyph)| self.item(label, glyph, label == section, &t, cx))
                    .collect::<Vec<_>>();
                let rows = div().flex().flex_col().gap(px(1.)).children(rows);
                sections.push(sidebar_section(*heading, rows, cx));
            }
        } else {
            sections.extend(
                self.results
                    .iter()
                    .enumerate()
                    .map(|(index, result)| self.search_row(index, result, &t, cx)),
            );
        }
        let empty = sections.is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::nav_key))
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
    selects: std::collections::HashMap<&'static str, Entity<crate::settings_kit::Select>>,
    git_fields: Vec<(&'static str, Entity<Composer>)>,
    scroll: gpui::ScrollHandle,
    rendered_route: String,
    search_flash: Rc<RefCell<Option<search::Flash>>>,
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
    ("cmdAlways", "⌘ + Enter always"),
];

impl SettingsPane {
    fn new(page: Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, page, cx| {
            if page.read(cx).section != "Git" {
                page.update(cx, |page, cx| {
                    for change in page.take_pending_instructions() {
                        page.set(change.path, change.value, cx);
                    }
                });
            }
            cx.notify();
        })
        .detach();
        Self {
            page,
            layer: Layer::default(),
            selects: std::collections::HashMap::new(),
            git_fields: Vec::new(),
            scroll: gpui::ScrollHandle::new(),
            rendered_route: String::new(),
            search_flash: Rc::default(),
        }
    }

    /// A select for `key` with its options menu `(value, label)`; the first option is the
    /// default. A click outside closes the menu.
    fn select_menu(
        &mut self,
        key: &'static str,
        options: &'static [(&'static str, &'static str)],
        _t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choices = if self.selects.contains_key(key) {
            Vec::new()
        } else {
            options
                .iter()
                .map(|&(value, label)| (value.to_owned(), label.to_owned()))
                .collect()
        };
        self.choose(key, choices, false, cx)
    }

    fn choose(
        &mut self,
        key: &'static str,
        options: Vec<(String, String)>,
        searchable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.page.read(cx);
        let default = self
            .selects
            .get(key)
            .map(|select| select.read(cx).default_value())
            .unwrap_or_else(|| options.first().map_or("", |(value, _)| value));
        let current = if key == "host.open-in-target-preferences" {
            page.get(key)["global"]
                .as_str()
                .or_else(|| match &page.targets {
                    crate::screens::computer_use::Load::Ready(value) => {
                        value["preferredTarget"].as_str()
                    }
                    _ => None,
                })
                .unwrap_or("")
        } else {
            page.choice(key, default)
        }
        .to_owned();
        let select = self.selects.entry(key).or_insert_with(|| {
            let choices = options
                .into_iter()
                .map(|(value, label)| crate::settings_kit::Choice {
                    value: value.into(),
                    label: label.into(),
                    checked: false,
                    disabled: false,
                })
                .collect();
            let select = cx.new(|cx| {
                general::present_select(
                    key,
                    crate::settings_kit::Select::new(
                        current.clone(),
                        choices,
                        searchable.then_some(if key == "desktop.localeOverride" {
                            "Search languages"
                        } else {
                            "Search…"
                        }),
                        cx,
                    ),
                )
            });
            let page = self.page.clone();
            cx.subscribe(
                &select,
                move |_, _, selected: &crate::settings_kit::Selected, cx| {
                    let value = if selected.0.is_empty() {
                        Value::Null
                    } else {
                        json!(selected.0.as_ref())
                    };
                    page.update(cx, |page, cx| {
                        let value = if key == "host.open-in-target-preferences" {
                            let mut preference = page.get(key).clone();
                            preference["global"] = value;
                            preference
                        } else {
                            value
                        };
                        page.set(key, value, cx);
                    });
                },
            )
            .detach();
            select
        });
        select.update(cx, |select, cx| select.set_value(current, cx));
        select.clone().into_any_element()
    }

    fn toggle(
        &self,
        key: &'static str,
        default: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = if key == "host.git-review-mode" {
            self.page.read(cx).choice(key, "full") == "last-turn-only"
        } else {
            self.page.read(cx).flag(key, default)
        };
        let focus = focus::handle(key, 0, window, cx);
        focus::ring(
            switch(key, on, self.page.read(cx).pending.contains(key), t).relative(),
            &focus,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
            focus.read(cx).handle.clone().focus(window, cx);
            cx.stop_propagation();
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            let value = if key == "host.git-review-mode" {
                json!(if on { "full" } else { "last-turn-only" })
            } else {
                Value::Bool(!on)
            };
            this.page.update(cx, |p, cx| p.set(key, value, cx))
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
                    .role(gpui::Role::RadioButton)
                    .aria_label(label)
                    .aria_toggled(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .focusable()
                    .tab_index(0)
                    .focus_visible(crate::focus::trigger_focus)
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
    fn appearance(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<SettingsRow>)> {
        let options = &[("light", "Light"), ("dark", "Dark"), ("system", "System")];
        let theme = self.segmented(THEME, "system", options, t, cx);
        vec![("Theme", vec![settings_row("Theme", None, theme, t)])]
    }

    fn git(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<SettingsRow>)> {
        const FIELDS: &[(&str, &str, &str)] = &[
            ("host.git-branch-prefix", "Branch prefix", "codex/"),
            (
                "host.git-pr-watch-instructions",
                "Watch and fix pull requests",
                "Add pull request watch guidance…",
            ),
            (
                "host.git-commit-instructions",
                "Commit instructions",
                "Add commit message guidance…",
            ),
            (
                "host.git-pr-instructions",
                "Pull request instructions",
                "Add pull request guidance…",
            ),
        ];
        if self.git_fields.is_empty() {
            for &(key, _, placeholder) in FIELDS {
                let multiline = key != "host.git-branch-prefix";
                let value = self
                    .page
                    .read(cx)
                    .choice(key, if multiline { "" } else { "codex/" })
                    .to_owned();
                let editor =
                    cx.new(|cx| Composer::settings_field(placeholder, &value, multiline, cx));
                let field = editor.clone();
                if !multiline {
                    cx.on_blur(&editor.focus_handle(cx), window, move |this, _, cx| {
                        this.save_git_field(key, &field, cx);
                    })
                    .detach();
                }
                if multiline {
                    let mut pending = None;
                    let mut previous = value.clone();
                    cx.observe(&editor, move |this, editor, cx| {
                        let text = editor.read(cx).field_text().to_owned();
                        if previous == text {
                            return;
                        }
                        previous = text.clone();
                        drop(pending.take());
                        this.page
                            .update(cx, |page, _| page.draft_instructions(key, text.clone()));
                        if this.page.read(cx).choice(key, "") != text {
                            pending = Some(cx.spawn(async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_secs(3))
                                    .await;
                                let _ = this.update(cx, |this, cx| {
                                    if editor.read(cx).field_text() == text {
                                        this.save_git_field(key, &editor, cx);
                                    }
                                });
                            }));
                        }
                        cx.notify();
                    })
                    .detach();
                }
                self.git_fields.push((key, editor));
            }
        }
        let branch = self.git_fields[0].1.clone();
        let mut groups = vec![(
            "Git",
            vec![
                settings_row(
                    "Disable Git-based diffs",
                    Some("Only show Last Turn in the Changes panel"),
                    self.toggle("host.git-review-mode", false, t, window, cx),
                    t,
                ),
                settings_row(
                    "Branch prefix",
                    Some("Prefix for new branches"),
                    div().w(px(224.)).child(branch).into_any_element(),
                    t,
                ),
                settings_row(
                    "Pull request merge method",
                    None,
                    self.select_menu(
                        "host.git-pull-request-merge-method",
                        &[("merge", "Merge"), ("squash", "Squash")],
                        t,
                        cx,
                    ),
                    t,
                ),
                settings_row(
                    "Always force push",
                    Some("Use --force-with-lease when pushing"),
                    self.toggle("host.git-always-force-push", false, t, window, cx),
                    t,
                ),
                settings_row(
                    "Create draft pull requests",
                    None,
                    self.toggle("host.git-create-pull-request-as-draft", true, t, window, cx),
                    t,
                ),
                settings_row(
                    "Review delivery",
                    None,
                    self.segmented(
                        "desktop.reviewDelivery",
                        "inline",
                        &[("inline", "Inline"), ("detached", "Detached")],
                        t,
                        cx,
                    ),
                    t,
                ),
            ],
        )];
        for (index, &(key, title, _)) in FIELDS.iter().enumerate().skip(1) {
            let editor = self.git_fields[index].1.clone();
            let saved = self.page.read(cx).choice(key, "") == editor.read(cx).field_text();
            let field = editor.clone();
            let row = div()
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(editor)
                .child(
                    div().flex().justify_end().child(
                        button(
                            if saved { "Saved" } else { "Save" },
                            t.text_alpha_5,
                            None,
                            t,
                        )
                        .id(key)
                        .role(gpui::Role::Button)
                        .focusable()
                        .tab_index(0)
                        .focus_visible(crate::focus::trigger_focus)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.save_git_field(key, &field, cx)),
                        ),
                    ),
                );
            let mut rows = Vec::new();
            if key == "host.git-pr-watch-instructions" {
                rows.push(settings_row(
                    "Auto-merge when ready",
                    Some("Continue watching until the pull request is merged"),
                    self.toggle("host.git-pr-watch-auto-merge", false, t, window, cx),
                    t,
                ));
            }
            rows.push(SettingsRow {
                title,
                detail: None,
                body: row,
            });
            groups.push((title, rows));
        }
        groups
    }

    fn save_git_field(
        &mut self,
        key: &'static str,
        editor: &Entity<Composer>,
        cx: &mut Context<Self>,
    ) {
        let value = editor.read(cx).field_text().to_owned();
        let default = if key == "host.git-branch-prefix" {
            "codex/"
        } else {
            ""
        };
        if self.page.read(cx).choice(key, default) != value {
            self.page
                .update(cx, |page, cx| page.set(key, json!(value), cx));
        }
    }
}

impl Render for SettingsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let section = self.page.read(cx).section;
        let route = &self.page.read(cx).route.path;
        if self.rendered_route != *route {
            self.rendered_route = route.clone();
            self.scroll.set_offset(point(px(0.), px(0.)));
            self.selects.clear();
        }
        if section == "Computer use" {
            return super::computer_use::render(window, cx);
        }
        if section == "Open source licenses" {
            return self.licenses(&t, cx);
        }
        if section == "Profile" {
            return self.profile(&t, cx);
        }
        // Unimplemented sections retain their title-only surface.
        let groups = match section {
            "General" => self.general(&t, window, cx),
            "Appearance" => self.appearance(&t, cx),
            "Git" => self.git(&t, window, cx),
            _ => Vec::new(),
        };
        self.sync_search_flash(cx);
        let target_group = groups
            .iter()
            .position(|(_, rows)| self.target_row_index(rows).is_some());
        let card_border = over(t.border, t.background_panel);
        let groups = groups.into_iter().map(|(title, rows)| {
            let last = rows.len().saturating_sub(1);
            let target_row = self.target_row_index(&rows);
            let target_level = target_row.map_or(search::TargetLevel::Group, |index| {
                self.row_target(&rows[index])
            });
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
                            self.track_search_layout(
                                div()
                                    .flex()
                                    .flex_col()
                                    .children(rows.into_iter().enumerate().map(|(i, r)| {
                                        self.highlight_row(r, &t).when(i < last, |d| {
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
                                target_row,
                                target_level,
                            ),
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
                    .track_scroll(&self.scroll)
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
                            self.track_search_layout(
                                column(div())
                                    .p(px(20.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(40.))
                                    .children(groups),
                                target_group,
                                search::TargetLevel::Group,
                            ),
                        ),
                    ),
            )
            .into_any_element()
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

const ROW_PADDING: f32 = 12.;
const ROW_TEXT_GAP: f32 = 2.;
const ROW_LABEL_LINE_HEIGHT: f32 = 18.571_428;

struct SettingsRow {
    title: &'static str,
    detail: Option<&'static str>,
    body: Div,
}

/// One settings row: title, optional description (an `<a>…</a>` span renders as the inline
/// link) and the trailing control.
fn settings_row(
    title: &'static str,
    detail: Option<&'static str>,
    control: AnyElement,
    t: &Theme,
) -> SettingsRow {
    let body = div()
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.))
        .px(px(16.))
        .py(px(ROW_PADDING))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(ROW_TEXT_GAP))
                .child(text(title, 13., ROW_LABEL_LINE_HEIGHT, 500., t.text))
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
        );
    SettingsRow {
        title,
        detail,
        body,
    }
}

fn description(s: &'static str, t: &Theme) -> impl IntoElement {
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
    let ranges = link.clone().into_iter().collect();
    let styled = StyledText::new(plain).with_highlights(link.clone().map(|range| {
        (
            range,
            HighlightStyle {
                color: Some(t.text_accent),
                ..Default::default()
            },
        )
    }));
    div()
        .id(s)
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(FontWeight(430.))
        .text_color(t.text_secondary)
        .when(link.is_some(), |d| {
            d.role(gpui::Role::Link)
                .aria_label("Learn more")
                .focusable()
                .tab_index(0)
                .on_key_down(|event, _, cx| {
                    if event.keystroke.key == "enter" {
                        cx.open_url("https://developers.openai.com/codex/config-basic");
                        cx.stop_propagation();
                    }
                })
        })
        .child(
            gpui::InteractiveText::new(s, styled).on_click(ranges, |_, _, cx| {
                cx.open_url("https://developers.openai.com/codex/config-basic");
            }),
        )
}

/// `Switch`: 32×20 track, 16 px thumb. `disabled` is the control's `opacity: .6`: Chromium
/// paints the track and thumb into one `saveLayerAlpha(153)`.
/// Toggling moves the thumb 2 px -> 14 px and fades the track colour, both 150 ms `ease-out`
/// (`refs/interactions/ms6a.json`); the tracks stay active under reduced motion.
pub(super) fn switch(id: &'static str, on: bool, disabled: bool, t: &Theme) -> Stateful<Div> {
    let (track_off, track_on) = (t.switch_track, t.switch_track_checked);
    let thumb = shapes::Decoration {
        radius: Corners::all(px(9999.)),
        fill: Some(t.control_thumb),
        border: Some((px(1.), t.control_thumb)),
        shadows: vec![shadow(t.shadow_sm, 1., 2., -1., false)],
        ..Default::default()
    };
    div()
        .id(id)
        .role(gpui::Role::Switch)
        .aria_label(if id == "default-permissions" {
            "Default permissions are always shown"
        } else {
            id
        })
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .aria_disabled(disabled)
        .flex_none()
        .w(px(32.))
        .h(px(20.))
        .child(
            canvas(
                move |_, window, cx| {
                    let state = window.use_keyed_state("switch-motion", cx, |_, cx| {
                        (
                            cx.background_executor().now(),
                            HoverFade::settled(on as u8 as f32),
                        )
                    });
                    let (progress, animating) = state.update(cx, |(epoch, fade), cx| {
                        let now = cx.background_executor().now() - *epoch;
                        fade.transition(on as u8 as f32, now, motion::BASIC, motion::EASE_OUT);
                        (fade.value(now), fade.is_animating(now))
                    });
                    if animating {
                        window.request_animation_frame();
                    }
                    progress
                },
                move |b, progress, window, cx| {
                    let fill = match progress {
                        p if p <= 0. => track_off,
                        p if p >= 1. => track_on,
                        p => motion::mix_oklab(track_off.into(), track_on.into(), p).into(),
                    };
                    let track = surface(9999., Round, fill, None);
                    let x = 2. + 12. * progress;
                    let paint = |window: &mut Window| {
                        shapes::paint(&track, None, b, window, cx);
                        let at =
                            Bounds::new(b.origin + point(px(x), px(2.)), size(px(16.), px(16.)));
                        shapes::paint(&thumb, None, at, window, cx);
                    };
                    if disabled {
                        shapes::with_layer(153, b, window, paint)
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
        .rounded(px(12.5))
        .h(px(28.))
        .px(px(9.))
        .flex()
        .items_center()
        .relative()
        .child(on_card(12.5, Squircle, fill, border))
        .child(text(label, 13., 18., 430., t.text))
}
/// Dropdown trigger: value (with an optional leading app icon) and a chevron.
pub(crate) fn select(
    label: impl Into<SharedString>,
    leading: Option<AnyElement>,
    disabled: bool,
    open: bool,
    t: &Theme,
) -> Div {
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
            if open {
                t.ghost_hover
            } else {
                t.primary_soft_alpha
            },
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
/// Rasterized at paint, inside the scroller's `.layer()`: its tile origin sets the bilinear
/// sample positions' float error (every 9th column at 1440 otherwise).
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
    let raster = move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
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
            shapes::ImageComposition::Direct(page),
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
    };
    canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            if let Some((at, image)) = raster(bounds, window, cx) {
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
    restart_blocked: bool,
}

enum Stage {
    Welcome,
    /// Browser sign-in started: the `loginId` that `account/login/cancel` takes.
    Pending(String),
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
            restart_blocked: false,
        }
    }

    pub fn restart_ready(&self) -> bool {
        !self.restart_blocked && matches!(self.stage, Stage::Welcome) && self.api_key.is_empty()
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

    /// `account/login/start` as the original's `loginWithChatGpt`: a ChatGPT login opens its
    /// `authUrl` in the browser and waits in `Pending` for `account/login/completed`.
    fn start(&mut self, params: Value, cx: &mut Context<Self>) {
        let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
            return;
        };
        let bridge = bridge.clone();
        self.restart_blocked = true;
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
                            this.stage = Stage::Pending(id.into());
                        }
                    }
                    Err(e) => this.error = Some(format!("Sign-in failed: {e}")),
                }
                cx.notify();
            })
        })
        .detach();
    }

    /// The ChatGPT sign-in request of the original (`appBrand`, hosted success page).
    fn start_chatgpt(&mut self, cx: &mut Context<Self>) {
        self.start(
            json!({ "type": "chatgpt", "appBrand": "chatgpt", "useHostedLoginSuccessPage": true }),
            cx,
        );
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
                            .on_click(cx.listener(|this, _, _, cx| this.start_chatgpt(cx))),
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
                            .on_click(cx.listener(|this, _, _, cx| this.start_chatgpt(cx))),
                    )
                    .into_any_element(),
            ),
            Stage::Pending(id) => {
                let id = id.clone();
                (
                    text(
                        "Continue signing in with your browser",
                        14.,
                        20.,
                        400.,
                        t.text_description,
                    ),
                    login_button("cancel", "Cancel sign-in", &t)
                        .h(px(42.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            send("account/login/cancel", json!({ "loginId": id }), cx);
                            this.go(Stage::Welcome, cx);
                        }))
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

    #[test]
    fn search_reveals_an_offscreen_row_and_navigation_resets_the_scroll() {
        use super::*;
        use gpui::{HeadlessAppContext, point, px, size};
        use std::sync::Arc;
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        let page = cx.update(|cx| {
            cx.set_global(Theme::dark());
            let page = cx.new(|_| SettingsPage::new(&json!({}), &json!({})));
            page.update(cx, |page, cx| {
                page.set_requirements(
                    Ok(json!({"requirements": null, "supportsIndependentSpeedModes": true})),
                    true,
                    cx,
                );
                page.search_target = Some((1, "Speed".into()));
            });
            page
        });
        let window = cx
            .open_window(size(px(900.), px(600.)), |_, cx| {
                cx.new(|cx| SettingsPane::new(page.clone(), cx))
            })
            .unwrap();
        for _ in 0..6 {
            cx.advance_clock(std::time::Duration::from_millis(16));
            cx.test_window(window.into())
                .simulate_frame_request(gpui::RequestFrameOptions {
                    require_presentation: true,
                    ..Default::default()
                });
            cx.run_until_parked();
        }
        cx.update(|cx| {
            window
                .update(cx, |pane, _, _| {
                    let row = pane
                        .search_flash
                        .borrow()
                        .as_ref()
                        .and_then(|flash| flash.row_bounds)
                        .expect("the search target must have measured bounds");
                    let viewport = pane.scroll.bounds();
                    assert!(
                        row.top() >= viewport.top() && row.bottom() <= viewport.bottom(),
                        "the whole target row must be visible: row={row:?}, viewport={viewport:?}",
                    );
                })
                .unwrap()
        });
        #[cfg(target_os = "macos")]
        if let Some(path) = std::env::var_os("CGX_GENERAL_EVIDENCE") {
            let path = std::path::Path::new(&path);
            std::fs::create_dir_all(path).unwrap();
            let metrics = cx.update(|cx| {
                window
                    .update(cx, |pane, _, _| {
                        json!({
                            "scrollY": f32::from(pane.scroll.offset().y),
                            "maxScrollY": f32::from(pane.scroll.max_offset().y),
                            "viewportY": f32::from(pane.scroll.bounds().origin.y),
                            "viewportHeight": f32::from(pane.scroll.bounds().size.height),
                            "rowBounds": pane.search_flash.borrow().as_ref().and_then(|flash| flash.row_bounds)
                                .map(|bounds| [f32::from(bounds.top()), f32::from(bounds.bottom())])
                        })
                    })
                    .unwrap()
            });
            std::fs::write(path.join("scroll-layout.json"), metrics.to_string()).unwrap();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(path.join("scroll-layout.png"))
                .unwrap();
        }
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.navigate("/settings/notifications", cx);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            window
                .update(cx, |pane, _, _| {
                    assert_eq!(pane.scroll.offset(), point(px(0.), px(0.)));
                })
                .unwrap()
        });
    }
}
