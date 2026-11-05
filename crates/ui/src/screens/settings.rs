//! `settings` parity screen: route `/settings/general-settings` (settings-page and
//! general-settings chunks) as refs/settings shows it, and the signed-out `/login` view
//! (login-route chunk) that replaces the app when `account/read` reports no account.

use crate::{
    MONO, SetTheme, Shell,
    composer::{Composer, shadow},
    fixture, focus,
    icon::icon,
    in_layer, layout_color,
    motion::{self, HoverFade},
    over, rounded_clip,
    screens::signin::Login,
    sidebar_section, text,
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
#[path = "settings_import.rs"]
pub mod imports;

#[path = "keyboard_shortcuts.rs"]
pub mod keyboard_shortcuts;
#[path = "voice.rs"]
pub mod voice;
fn focus_on_pointer(
    focus: Entity<focus::ControlFocus>,
) -> impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) {
    move |_, window, cx| {
        let handle = focus.read(cx).handle.clone();
        handle.focus(window, cx);
        cx.stop_propagation();
    }
}

mod account_page;
mod ads_controls;
pub mod backend;
mod code_review;
pub mod computer_history;
mod languages;
pub mod parental_controls;
mod passwords;
pub mod personalization;
pub mod pets;
pub mod security_settings;
pub mod trusted_contact;
pub mod workspace {
    use crate::{SANS, composer::popover, focus, icon::icon, text};
    use gpui::{
        Anchor, Context, Corners, EventEmitter, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, anchored,
        deferred, div, point, px,
    };
    use shapes::Corner::Round;
    use theme::ActiveTheme as _;

    pub enum Event {
        PickFolder,
        CreateProject,
    }

    #[derive(Clone, Copy)]
    pub enum Pending {
        Picking,
        Creating,
    }

    pub struct Workspace {
        pending: Option<Pending>,
        error: Option<String>,
        menu: bool,
        menu_hovered: bool,
    }

    impl EventEmitter<Event> for Workspace {}

    impl Workspace {
        pub fn new(_: &mut Context<Self>) -> Self {
            Self {
                pending: None,
                error: None,
                menu: false,
                menu_hovered: false,
            }
        }

        pub fn busy(&self) -> bool {
            self.pending.is_some()
        }

        pub fn begin(&mut self, pending: Pending, cx: &mut Context<Self>) {
            self.pending = Some(pending);
            self.error = None;
            self.menu = false;
            self.menu_hovered = false;
            cx.notify();
        }

        pub fn finish(&mut self, error: Option<String>, cx: &mut Context<Self>) {
            self.pending = None;
            self.error = error;
            cx.notify();
        }
    }

    impl Render for Workspace {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let t = cx.theme().clone();
            let description_color = t.text.opacity(0.498);
            let busy = self.busy();
            let add_focus = focus::handle("workspace-add", 0, window, cx);
            let skip_focus = focus::handle("workspace-skip", 0, window, cx);
            let button_text = match t.appearance {
                theme::Appearance::Dark => gpui::rgb(0x2d2d2d).into(),
                theme::Appearance::Light => t.text_inverse,
            };
            let mut add = focus::ring(
                div()
                    .id("workspace-add")
                    .role(gpui::Role::Button)
                    .aria_label("Add project")
                    .relative()
                    .w_full()
                    .h(px(40.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(9999.)),
                        corner: Round,
                        fill: Some(t.button_primary_solid),
                        border: Some((px(1.), t.border)),
                        ..Default::default()
                    }))
                    .child(text("Add project", 13., 18., 430., button_text)),
                &add_focus,
                theme::Theme::RADIUS_FULL,
                px(2.),
                cx,
            );
            if busy {
                add = add.opacity(0.5);
            } else {
                add = add
                    .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(add_focus))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = !this.menu;
                        cx.notify();
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        if !this.menu_hovered {
                            this.menu = false;
                            cx.notify();
                        }
                    }));
            }
            let menu = self.menu.then(|| {
                let on_hover =
                    cx.listener(|this, hovered: &bool, _, _| this.menu_hovered = *hovered);
                let rows = [
                    ("Start from scratch", false),
                    ("Use an existing folder", true),
                ]
                .into_iter()
                .map(|(label, pick)| {
                    let focus = focus::handle(label, 0, window, cx);
                    focus::ring(
                        div()
                            .id(label)
                            .role(gpui::Role::MenuItem)
                            .aria_label(label)
                            .relative()
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .hover(|s| s.bg(t.menu_item_hover))
                            .child(text(label, 14., 20., 430., t.text)),
                        &focus,
                        px(8.),
                        px(0.),
                        cx,
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(if pick {
                            Event::PickFolder
                        } else {
                            Event::CreateProject
                        });
                    }))
                });
                deferred(
                    div().absolute().top_0().left_0().child(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .offset(point(px(0.), px(44.)))
                            .child(
                                div()
                                    .id("workspace-add-menu")
                                    .on_hover(on_hover)
                                    .child(popover(&t, 240., |d| d.children(rows))),
                            ),
                    ),
                )
                .into_any_element()
            });
            let skip_label = if matches!(self.pending, Some(Pending::Creating)) {
                "Creating a new project…"
            } else {
                "Skip"
            };
            let mut skip = focus::ring(
                div()
                    .id("workspace-skip")
                    .role(gpui::Role::Button)
                    .aria_label(skip_label)
                    .relative()
                    .w_full()
                    .h(px(36.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text(skip_label, 14., 18., 500., description_color)),
                &skip_focus,
                theme::Theme::RADIUS_FULL,
                px(2.),
                cx,
            );
            if busy {
                skip = skip.opacity(0.5);
            } else {
                skip = skip
                    .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(skip_focus))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::CreateProject)));
            }
            div().size_full().bg(t.background_surface_under).font_family(SANS)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" { this.menu = false; cx.notify(); }
            }))
            .flex().items_center().justify_center().pt(px(12.)).px(px(20.))
            // Chromium paints the content into a transparent composited layer, then blends it.
            .child(div().w_full().max_w(px(330.)).flex().flex_col().items_center().gap(px(24.)).layer()
                .child(icon("folder-add", 40., t.text))
                .child(div().w_full().flex().flex_col().gap(px(24.))
                    .child(div().flex().flex_col().gap(px(24.)).items_center()
                        .child(text("Select a project", 24., 36., 600., t.text))
                        .child(text("ChatGPT will be able to edit files and run commands in selected folders", 16., 24., 430., description_color)
                            .w_full().whitespace_normal().text_center()))
                    .child(div().w_full().flex().flex_col().gap(px(12.))
                        .child(div().relative().child(add).children(menu))
                        .children(self.error.clone().map(|error| text(error, 14., 20., 430., t.text_error).whitespace_normal().text_center()))
                        .child(skip))))
        }
    }
}

#[path = "settings/appshots.rs"]
mod appshots;
#[path = "settings/debug.rs"]
pub mod debug;
#[path = "settings/file_type_handlers.rs"]
pub mod file_type_handlers;
#[path = "settings/notifications.rs"]
pub mod notifications;
#[path = "settings_subtrees.rs"]
pub mod subtrees;
#[path = "settings/usage.rs"]
pub mod usage;

#[path = "storage.rs"]
pub mod storage;

#[path = "settings_integrations.rs"]
pub mod integrations;

pub mod account;
#[path = "archived_chats.rs"]
pub mod archived_chats;
pub mod data_controls;

#[path = "git_settings.rs"]
mod git_settings;

#[path = "appearance_settings.rs"]
mod appearance_settings;

#[path = "configuration_settings.rs"]
mod configuration_settings;

#[path = "worktrees.rs"]
pub mod worktrees;

#[path = "environments.rs"]
pub mod environments;

#[path = "browser_settings.rs"]
mod browser_settings;

/// The app's theme setting (`ThemeSetting`): `system`, `light` or `dark`.
pub const THEME: &str = "host.appearance-theme";

/// `Show Full access in the composer`; turning it on first asks `Make Full Access available?`.
pub const FULL_ACCESS_SHOWN: &str = "host.show-full-access-in-composer";

/// The live app-server connection. Settings and sign-in send their writes through it when
/// the app sets this global; the parity fixture runs without one.
pub struct AppServer(pub Arc<Bridge>);

impl Global for AppServer {}

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let account: AccountReadResponse = fixture::response("readback.jsonl", "account/read", 0);
    if account.account.is_none() {
        return cx.new(Login::new).into();
    }
    if !cx.has_global::<imports::ImportState>() {
        cx.set_global(imports::ImportState {
            policy: bridge::imports::Policy {
                enabled: true,
                sync: true,
                cursor: true,
            },
            ..Default::default()
        });
    }
    // The reference was captured with macOS notifications turned off for the app.
    if !cx.has_global::<notifications::State>() {
        cx.set_global(notifications::State {
            permission: Some(computer_use::Load::Ready(
                notifications::Permission::Disabled,
            )),
            ..Default::default()
        });
    }
    let config: ConfigReadResponse = fixture::response("readback.jsonl", "config/read", 0);
    let skills: Value = fixture::response("readback.jsonl", "skills/list", 0);
    let integrations = cx.new(|_| integrations::State::default());
    let (shell, _) = shell(&config.config, &skills, None, integrations, cx);
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

/// The nav item a `/settings/<slug>` route opens (the original's `lQa` section slugs and
/// the `work-settings` and usage-tab redirects). A trailing `?query` is ignored. Slugs
/// with no native page open General, as the original does for unknown slugs.
pub fn slug_section(slug: &str) -> &'static str {
    match slug.split('?').next().unwrap_or_default() {
        "notifications" => "Notifications",
        "import" => "Import",
        "account" => "Account",
        "appearance" => "Appearance",
        "archived-chats" => "Archived chats",
        "parental-controls" => "Parental controls",
        "trusted-contact" => "Trusted contact",
        "voice" => "Voice",
        "storage" => "Storage",
        "agent" | "work-settings" => "Configuration",
        "personalization" => "Personalization",
        "pets" => "Mini & Pets",
        "keyboard-shortcuts" => "Keyboard shortcuts",
        "usage" | "billing" | "analytics" | "consumer-view" | "usage-code-review" => {
            "Usage & billing"
        }
        "data-controls" => "Data controls",
        "plugins-settings" | "skills-settings" => "Plugins",
        "mcp-settings" => "MCP servers",
        "passwords" => "Passwords",
        "computer-use" => "Computer use",
        "chronicle" => "Computer history",
        "appshots" => "Appshots",
        "browser-use" => "Browser",
        "cloud-computer" => "Cloud computer",
        "hooks-settings" => "Hooks",
        "connections" => "Connections",
        "codex-cloud" | "cloud-environments" | "environments" => "Codex Cloud",
        "code-review" => "Code Review",
        "git-settings" => "Git",
        "worktrees" | "local-environments" => "Worktrees",
        _ => "General",
    }
}

/// The Settings route's shell for a `config/read` config and `skills/list` result: settings
/// nav in the sidebar panel, the page in the main panel, open at the nav item `section`
/// (`General` if `None`). The page emits [`SettingChanged`].
pub fn shell(
    config: &Value,
    skills: &Value,
    section: Option<&'static str>,
    integrations: Entity<integrations::State>,
    cx: &mut App,
) -> (Entity<Shell>, Entity<SettingsPage>) {
    let page = cx.new(|cx| {
        let mut page = SettingsPage::new(config, skills, integrations, cx);
        if let Some(label) = section {
            page.section = label;
        }
        page
    });
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
            ("Ads controls", "settings-data-controls"),
            ("Debug", "settings-configuration"),
        ],
    ),
    (
        "Integrations",
        &[
            ("Plugins", "settings-plugins"),
            ("File type handlers", "context-file-default"),
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
    subtree: Option<AnyView>,
    query: String,
    /// `config/read` config, plus a `host` object for host settings.
    values: Value,
    /// `projectlessWorkspaceRoot` default: `~/Documents/Codex`.
    projectless_root: String,
    pending_instructions: Vec<SettingChanged>,
    integrations: Entity<integrations::State>,
    pub personalization: Entity<personalization::Personalization>,
    toaster: Option<Entity<crate::toast::Toaster>>,
    cloud: Option<Entity<super::cloud::CloudPage>>,
    /// Every effort a listed `model/list` model supports, in catalog order.
    catalog_efforts: Vec<String>,
    /// The selected host's `config/read` with `includeLayers` (`origins`, `layers`).
    config_layers: Value,
    /// The `Filter settings by host` choices, `(host id, label)`, the window's host first;
    /// empty while no other host is connected.
    hosts: Vec<(SharedString, SharedString)>,
    /// The host whose `config.toml` the page reads and writes.
    config_host: SharedString,
}

impl SettingsPage {
    fn new(
        config: &Value,
        skills: &Value,
        integrations: Entity<integrations::State>,
        cx: &mut Context<Self>,
    ) -> Self {
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
            subtree: None,
            query: String::new(),
            values,
            projectless_root,
            pending_instructions: Vec::new(),
            integrations,
            personalization: cx.new(personalization::Personalization::new),
            toaster: None,
            cloud: None,
            catalog_efforts: Vec::new(),
            config_layers: Value::Null,
            hosts: Vec::new(),
            config_host: SharedString::default(),
        }
    }

    pub fn set_toaster(&mut self, toaster: Entity<crate::toast::Toaster>) {
        self.toaster = Some(toaster);
    }

    /// `store.key` (`browser_use.default_origin_policy.access` nests further), or a
    /// top-level `config.toml` key such as `approval_policy`.
    pub fn get(&self, path: &str) -> &Value {
        path.split('.').fold(&self.values, |value, key| &value[key])
    }

    fn flag(&self, path: &str, default: bool) -> bool {
        self.get(path).as_bool().unwrap_or(default)
    }

    fn choice<'a>(&'a self, path: &str, default: &'a str) -> &'a str {
        self.get(path).as_str().unwrap_or(default)
    }

    /// Stores a setting and reports it to the app's config or host settings service.
    pub fn set(&mut self, path: &'static str, value: Value, cx: &mut Context<Self>) {
        self.pending_instructions
            .retain(|change| change.path != path);
        // `IndexMut` makes each missing level an object on its first key.
        *path
            .split('.')
            .fold(&mut self.values, |value, key| &mut value[key]) = value.clone();
        cx.emit(SettingChanged {
            path,
            value: value.clone(),
        });
        if let Some(on) = value.as_bool() {
            let reduce_motion = self.choice("host.reduced-motion-preference", "") == "on";
            debug::apply(path, on, reduce_motion, cx);
        }
        cx.notify();
    }

    /// Stores host settings together; the app saves them in one write, so a failed save keeps
    /// all of the previous values.
    pub fn set_host_batch(&mut self, pairs: Vec<(&'static str, Value)>, cx: &mut Context<Self>) {
        for (path, value) in &pairs {
            let (store, key) = path.split_once('.').unwrap_or((path, ""));
            self.values[store][key] = value.clone();
        }
        cx.emit(HostSettingsChanged(pairs));
        cx.notify();
    }

    pub fn set_cloud(&mut self, cloud: Entity<super::cloud::CloudPage>, cx: &mut Context<Self>) {
        self.cloud = Some(cloud);
        self.open_cloud_section(cx);
        cx.notify();
    }

    pub fn refresh_host(&mut self, values: Value, cx: &mut Context<Self>) {
        self.values["host"] = values;
        cx.notify();
    }

    /// The efforts the selected host's `model/list` catalog supports (original's `Kn`).
    pub fn set_models(&mut self, models: &[bridge::types::Model], cx: &mut Context<Self>) {
        self.catalog_efforts.clear();
        for effort in models
            .iter()
            .filter(|m| !m.hidden)
            .flat_map(|m| &m.supported_reasoning_efforts)
            .filter_map(|e| e["reasoningEffort"].as_str())
        {
            if !self.catalog_efforts.iter().any(|e| e == effort) {
                self.catalog_efforts.push(effort.to_owned());
            }
        }
        cx.notify();
    }

    pub fn set_config_layers(&mut self, read: Value, cx: &mut Context<Self>) {
        self.config_layers = read;
        cx.notify();
    }

    /// The `Filter settings by host` choices; the selection resets to `hosts[0]`.
    pub fn set_hosts(&mut self, hosts: Vec<(SharedString, SharedString)>, cx: &mut Context<Self>) {
        self.config_host = hosts.first().map(|(id, _)| id.clone()).unwrap_or_default();
        self.hosts = hosts;
        cx.notify();
    }

    /// The host the config rows read and write; empty before [`Self::set_hosts`].
    pub fn config_host(&self) -> &str {
        &self.config_host
    }

    /// The `Filter settings by host` choices, `(host id, label)`.
    pub fn hosts(&self) -> &[(SharedString, SharedString)] {
        &self.hosts
    }

    /// Points the config rows at `host` and asks the app to read its `config.toml`.
    pub fn select_config_host(&mut self, host: SharedString, cx: &mut Context<Self>) {
        if self.config_host != host {
            self.config_host = host;
            cx.emit(SelectConfigHost);
            cx.notify();
        }
    }

    /// Replaces the `config.toml` values with another host's, keeping the desktop `host` store.
    pub fn set_config_values(&mut self, mut config: Value, cx: &mut Context<Self>) {
        config["host"] = self.values["host"].take();
        self.values = config;
        cx.notify();
    }

    pub fn show_subtree(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.subtree = Some(view);
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

    pub fn select_section(&mut self, label: &'static str, cx: &mut Context<Self>) {
        let editor = self.personalization.read(cx);
        if editor.dirty(cx) || editor.saving() {
            cx.emit(personalization::Event::Section(label));
            return;
        }
        self.personalization
            .update(cx, |editor, cx| editor.discard(cx));
        self.section = label;
        self.subtree = None;
        self.open_cloud_section(cx);
        cx.notify();
    }

    fn open_cloud_section(&mut self, cx: &mut Context<Self>) {
        if matches!(self.section, "Codex Cloud" | "Legacy Codex Cloud")
            && let Some(cloud) = &self.cloud
        {
            cloud.update(cx, |cloud, cx| cloud.open_section(self.section, cx));
            self.subtree = Some(cloud.clone().into());
        }
    }
}

/// A settings row changed `path` (`store.key`, see [`SettingsPage`]) to `value`.
pub struct SettingChanged {
    pub path: &'static str,
    pub value: Value,
}

/// Several `host.<key>` settings changed together (see [`SettingsPage::set_host_batch`]).
pub struct HostSettingsChanged(pub Vec<(&'static str, Value)>);

/// `Open config.toml`: the app opens the selected host's user config layer.
pub struct OpenConfigToml;

/// Turning on [`FULL_ACCESS_SHOWN`]: the app confirms before it stores the change.
pub struct RequestFullAccessAvailable;

/// `Filter settings by host` picked [`SettingsPage::config_host`].
pub struct SelectConfigHost;

impl EventEmitter<SettingChanged> for SettingsPage {}
impl EventEmitter<HostSettingsChanged> for SettingsPage {}
impl EventEmitter<personalization::Event> for SettingsPage {}
impl EventEmitter<OpenConfigToml> for SettingsPage {}
impl EventEmitter<RequestFullAccessAvailable> for SettingsPage {}
impl EventEmitter<SelectConfigHost> for SettingsPage {}

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
    /// `Filter settings by host`, while another host is connected.
    hosts: Option<Entity<crate::settings_kit::Select>>,
}

impl SettingsNav {
    fn new(page: Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<imports::ImportState>(|_, cx| cx.notify())
            .detach();
        Self {
            page,
            focus: cx.focus_handle(),
            list: Layer::default(),
            hosts: None,
        }
    }

    /// The `Filter settings by host` menu (original `settings-host-dropdown`, `filter`
    /// variant): the window's host, then each other connected host.
    fn host_filter(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.page.read(cx);
        if page.hosts.len() < 2 {
            return None;
        }
        let current = page.config_host.clone();
        let choices: Vec<_> = page
            .hosts
            .iter()
            .map(|(id, label)| crate::settings_kit::Choice {
                value: id.clone(),
                label: label.clone(),
                checked: false,
                disabled: false,
            })
            .collect();
        let select = match &self.hosts {
            Some(select) => select.clone(),
            None => {
                let select = cx.new(|cx| {
                    crate::settings_kit::Select::new(current.clone(), Vec::new(), None, cx)
                        .with_width(200.)
                });
                let page = self.page.clone();
                cx.subscribe(
                    &select,
                    move |_, _, selected: &crate::settings_kit::Selected, cx| {
                        page.update(cx, |page, cx| {
                            page.select_config_host(selected.0.clone(), cx)
                        });
                    },
                )
                .detach();
                self.hosts = Some(select.clone());
                select
            }
        };
        select.update(cx, |select, cx| {
            let known = select.choices().iter().map(|c| &c.value);
            if !known.eq(choices.iter().map(|c| &c.value)) {
                select.set_choices(choices, cx);
            }
            select.set_value(current, cx);
        });
        Some(
            div()
                .id("settings-host-filter")
                .aria_label("Filter settings by host")
                .child(select)
                .into_any_element(),
        )
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
            (_, Some(c)) if !c.chars().any(char::is_control) => query.push_str(c),
            _ => return,
        }
        self.page.update(cx, |p, cx| {
            p.query = query;
            cx.notify();
        });
        cx.stop_propagation();
    }

    fn search(
        &mut self,
        query: &str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = focus::handle("settings-search", 0, window, cx);
        self.focus = focus.read(cx).handle.clone();
        let (s, color) = if query.is_empty() {
            ("Search".to_string(), t.text_tertiary)
        } else {
            (query.to_string(), t.text)
        };
        let control = div()
            .id("settings-search")
            .key_context("Editable")
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(text(s, 14., 18., 430., color)),
            )
            .when(!query.is_empty(), |d| {
                d.child(
                    div()
                        .id("settings-search-clear")
                        .role(gpui::Role::Button)
                        .aria_label("Clear settings search")
                        .cursor_pointer()
                        .child(icon("close", 16., t.text_secondary))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.page.update(cx, |p, cx| {
                                p.query.clear();
                                cx.notify();
                            });
                            this.focus.focus(window, cx);
                        })),
                )
            });
        focus::ring(control, &focus, Theme::RADIUS_FULL, px(2.), cx)
    }

    /// A nav row: `overflow: hidden`, so Skia draws a runtime icon (the viewport cuts the last
    /// one at 1440: no fitted image) under the row's AA clip; its own fill is outside it.
    fn item(
        &self,
        label: &'static str,
        glyph: &'static str,
        selected: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = focus::handle(label, 0, window, cx);
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
            });
        let control = div()
            .id((gpui::ElementId::from("settings-nav-item"), label))
            .role(gpui::Role::Button)
            .aria_label(label)
            .relative()
            .flex()
            .flex_col()
            .when(selected, |d| {
                d.child(deco(&self.list, 12.5, Squircle, t.list_hover, None))
            })
            .child(rounded_clip(Theme::RADIUS_LG, Squircle, row));
        focus::ring(control, &focus, Theme::RADIUS_LG, px(2.), cx)
            .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
            .when(label != EXTERNAL, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| this.open(label, window, cx)))
            })
    }

    fn open(&mut self, label: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self.page.read(cx).subtree.is_some() {
            window.dispatch_action(
                Box::new(subtrees::OpenSection {
                    label: label.into(),
                }),
                cx,
            );
            return;
        }
        self.page.update(cx, |p, cx| p.select_section(label, cx));
    }

    /// A search result: a matching row's text over the page it is on. Pointer-only: the same
    /// pages stay keyboard-reachable through the nav items, and result tab stops would
    /// reorder the tab path into the page.
    fn result(
        &self,
        index: usize,
        label: &'static str,
        row: &'static str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = gpui::ElementId::NamedInteger("settings-search-result".into(), index as u64);
        let fill = over(t.list_hover, sidebar_color(t));
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(format!("{row} in {label}"))
            .flex()
            .flex_col()
            .px(px(8.))
            .py(px(4.))
            .rounded(Theme::RADIUS_LG)
            .cursor_pointer()
            .hover(move |s| s.bg(fill))
            .child(text(row, 14., 20., 400., t.text))
            .child(text(label, 12., 16., 400., t.text_tertiary))
            .on_click(cx.listener(move |this, _, window, cx| this.open(label, window, cx)))
    }
}

/// Row titles and descriptions by nav item (`<label>\t<text>` lines), from the original's
/// settings search documents.
const SEARCH_INDEX: &str = include_str!("settings/search_index.tsv");

/// The most row results the nav lists for one query.
const MAX_RESULTS: usize = 50;

/// `(nav label, row text)` index entries whose text contains the lowercase `needle`.
fn row_matches(needle: &str) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
    SEARCH_INDEX
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(move |(_, row)| row.to_lowercase().contains(needle))
}

impl Render for SettingsNav {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let host_filter = self.host_filter(cx);
        let page = self.page.read(cx);
        let (section, query) = (page.section, page.query.clone());
        let needle = query.to_lowercase();
        let import_visible = imports::visible(cx);
        let cloud_access = page
            .cloud
            .as_ref()
            .map(|cloud| cloud.read(cx).access())
            .unwrap_or_default();
        let integrations = page.integrations.read(cx);
        let history_visible =
            computer_history::visible(&integrations.evaluations, integrations.gates_loaded);
        let visible = |label: &str| match label {
            "Import" => import_visible,
            "Cloud computer" => false,
            "Codex Cloud" => cloud_access.cloud,
            "Legacy Codex Cloud" => cloud_access.legacy,
            "Computer history" => history_visible,
            // The original lists Code Review for its cloud review surface (`sx`).
            "Code Review" => cloud_access.legacy,
            "Debug" => debug::allowed(),
            _ => true,
        };
        let mut sections = Vec::new();
        for (heading, items) in NAV {
            let rows: Vec<_> = items
                .iter()
                .filter(|(label, _)| label.to_lowercase().contains(&needle) && visible(label))
                .map(|&(label, glyph)| self.item(label, glyph, label == section, &t, window, cx))
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
        if !needle.is_empty() {
            let results: Vec<_> = row_matches(&needle)
                .filter(|(label, _)| visible(label))
                .take(MAX_RESULTS)
                .enumerate()
                .map(|(i, (label, row))| self.result(i, label, row, &t, cx))
                .collect();
            if !results.is_empty() {
                sections.push(sidebar_section(
                    "Matching settings",
                    div().flex().flex_col().gap(px(1.)).children(results),
                    cx,
                ));
            }
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
                            .justify_between()
                            .child(
                                text("Settings", 18., 24., 600., t.text).font_family("OpenAI Sans"),
                            )
                            .children(host_filter),
                    )
                    .child(self.search(&query, &t, window, cx)),
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
    /// ChatGPT notification menus by category, with the channels each was built for.
    channel_selects:
        std::collections::HashMap<String, (Vec<String>, Entity<crate::settings_kit::MultiSelect>)>,
    /// File type handlers menus by preference key.
    file_selects: std::collections::HashMap<String, Entity<crate::settings_kit::Select>>,
    git_fields: Vec<(&'static str, Entity<Composer>)>,
    storage: Entity<storage::StoragePage>,
    git_saved_until: std::collections::HashMap<&'static str, std::time::Instant>,
    appearance: appearance_settings::State,
    pets: pets::State,
    /// `Available reasoning efforts` on the Configuration page.
    efforts: Option<(Vec<String>, Entity<crate::settings_kit::MultiSelect>)>,
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

/// The integrated terminal's shell when none is chosen: `$SHELL`, else `/bin/sh`.
pub fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| s.starts_with('/'))
        .unwrap_or_else(|| "/bin/sh".into())
}

/// The absolute paths `/etc/shells` lists, in order, without comments, blanks or repeats.
fn parse_shells(text: &str) -> Vec<&str> {
    let mut shells = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('/') && !shells.contains(&line) {
            shells.push(line);
        }
    }
    shells
}

/// `Integrated terminal shell` options `(path, basename)`: the existing shells `/etc/shells`
/// lists, one per real file (`/bin/bash` and a merged `/usr/bin/bash` show once).
static SHELL_OPTIONS: std::sync::LazyLock<Vec<(&str, &str)>> = std::sync::LazyLock::new(|| {
    let text = std::fs::read_to_string("/etc/shells").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    parse_shells(&text)
        .into_iter()
        .filter(|path| {
            std::fs::canonicalize(path).is_ok_and(|real| real.is_file() && seen.insert(real))
        })
        .map(|path| {
            // Leaked once per process: the select menu takes `'static` options.
            let path: &'static str = path.to_owned().leak();
            (path, path.rsplit('/').next().unwrap_or(path))
        })
        .collect()
});

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
        cx.observe_global::<notifications::State>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<super::mini::CustomPets>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<super::mini::MiniOpen>(|_, cx| cx.notify())
            .detach();
        Self {
            storage: cx.new(|cx| storage::StoragePage::new(page.clone(), cx)),
            page,
            layer: Layer::default(),
            selects: std::collections::HashMap::new(),
            channel_selects: std::collections::HashMap::new(),
            file_selects: std::collections::HashMap::new(),
            git_fields: Vec::new(),
            git_saved_until: Default::default(),
            appearance: Default::default(),
            pets: Default::default(),
            efforts: None,
        }
    }

    /// A select for `key` with its options menu `(value, label)`; the first option is the
    /// default, and an empty value clears the setting. `search` adds a filter field with that
    /// placeholder. A click outside closes the menu.
    fn select_menu(
        &mut self,
        key: &'static str,
        options: &'static [(&'static str, &'static str)],
        search: Option<&'static str>,
        present: fn(crate::settings_kit::Select) -> crate::settings_kit::Select,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.select_menu_or(key, options, options[0].0, search, present, cx)
    }

    /// [`Self::select_menu`] whose unset value shows `default` instead of the first option.
    fn select_menu_or(
        &mut self,
        key: &'static str,
        options: &'static [(&'static str, &'static str)],
        default: &str,
        search: Option<&'static str>,
        present: fn(crate::settings_kit::Select) -> crate::settings_kit::Select,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.page.read(cx).choice(key, default).to_owned();
        let page = self.page.clone();
        let select = self.selects.entry(key).or_insert_with(|| {
            let choices = options
                .iter()
                .map(|&(value, label)| crate::settings_kit::Choice {
                    value: value.into(),
                    label: label.into(),
                    checked: false,
                    disabled: false,
                })
                .collect();
            let select = cx.new(|cx| {
                present(crate::settings_kit::Select::new(
                    current.clone(),
                    choices,
                    search,
                    cx,
                ))
            });
            cx.subscribe(
                &select,
                move |_, _, selected: &crate::settings_kit::Selected, cx| {
                    page.update(cx, |page, cx| {
                        let value = match selected.0.as_ref() {
                            "" => Value::Null,
                            value => json!(value),
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
        // A string setting shown as a switch: its (on, off) values.
        let strings = match key {
            "host.git-review-mode" => Some(("last-turn-only", "full")),
            browser_settings::FULL_CDP => Some(("allow", "deny")),
            _ => None,
        };
        let on = match strings {
            Some((on, off)) => self.page.read(cx).choice(key, off) == on,
            None => self.page.read(cx).flag(key, default),
        };
        let focus = focus::handle(key, 0, window, cx);
        focus::ring(
            switch(key, on, false, t).relative(),
            &focus,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
        .on_click(cx.listener(move |this, _, _, cx| {
            if key == FULL_ACCESS_SHOWN && !on {
                return this
                    .page
                    .update(cx, |_, cx| cx.emit(RequestFullAccessAvailable));
            }
            let value = match strings {
                Some((on_value, off_value)) => json!(if on { off_value } else { on_value }),
                None => Value::Bool(!on),
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
        window: &mut Window,
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
                let focus = focus::handle((gpui::ElementId::from(key), value), 0, window, cx);
                let control = div()
                    .id(label)
                    .role(gpui::Role::Button)
                    .aria_label(label)
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
                    ));
                focus::ring(control.relative(), &focus, Theme::RADIUS_FULL, px(2.), cx)
                    .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.page.update(cx, |p, cx| p.set(key, json!(value), cx));
                        if key == THEME {
                            window.dispatch_action(Box::new(SetTheme(value)), cx);
                        }
                    }))
            }))
            .into_any_element()
    }

    fn general(
        &mut self,
        t: &Theme,
        window: &mut Window,
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
        let notifications = self.notifications(t, window, cx);
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
                        self.toggle(FULL_ACCESS_SHOWN, true, t, window, cx),
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
                            430.,
                            Some(app_icon(over(t.primary_soft_alpha, t.background_panel))),
                            false,
                            false,
                            t,
                        )
                        .into_any_element(),
                    ),
                    row(
                        "Language",
                        Some("Language for the app UI"),
                        self.select_menu(
                            "desktop.localeOverride",
                            languages::LANGUAGES,
                            Some("Search languages"),
                            |select| {
                                let mut select = select.keep_default_visible();
                                select.search_aliases(|| {
                                    languages::ENGLISH_NAMES
                                        .iter()
                                        .map(|&(code, name)| (code.into(), name.into()))
                                        .collect()
                                });
                                select
                            },
                            cx,
                        ),
                    ),
                    row(
                        "Show in menu bar",
                        Some("Keep ChatGPT in the macOS menu bar when the main window is closed"),
                        self.toggle("host.mac-menu-bar-enabled", true, t, window, cx),
                    ),
                    row(
                        "Confirm before closing a window",
                        Some("Warn when a tab-close shortcut would close the window"),
                        self.select_menu(
                            "host.confirm-before-close",
                            CLOSE_OPTIONS,
                            None,
                            |select| select,
                            cx,
                        ),
                    ),
                    row(
                        "Full view by default",
                        Some("Start new tasks with Chat and content in a single tab strip"),
                        self.toggle("host.default-workspace-full-view", false, t, window, cx),
                    ),
                    row(
                        "Bottom panel",
                        Some("Show the bottom panel control in the app header"),
                        self.toggle("session.bottomPanel", true, t, window, cx),
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
                            window,
                            cx,
                        ),
                    ),
                    row(
                        "Integrated terminal shell",
                        Some("Choose which shell opens in the integrated terminal."),
                        if SHELL_OPTIONS.is_empty() {
                            select("No shells available", 430., None, true, false, t)
                                .into_any_element()
                        } else {
                            self.select_menu_or(
                                "desktop.integratedTerminalShell",
                                SHELL_OPTIONS.as_slice(),
                                &default_shell(),
                                None,
                                |select| select,
                                cx,
                            )
                        },
                    ),
                    row(
                        "Prevent sleep while running",
                        Some("Keep your computer awake while ChatGPT is running a task"),
                        self.toggle("desktop.preventSleepWhileRunning", false, t, window, cx),
                    ),
                    row(
                        "Compress local chat history",
                        Some(
                            "Save disk space by compressing older local chat history. Restart ChatGPT to apply changes.",
                        ),
                        self.toggle(
                            "features.local_thread_store_compression",
                            false,
                            t,
                            window,
                            cx,
                        ),
                    ),
                    row(
                        "Speed",
                        Some(
                            "Choose how quickly ChatGPT runs across chats, subagents, and compaction",
                        ),
                        select("Standard", 430., None, false, false, t).into_any_element(),
                    ),
                    row(
                        "Suggested prompts",
                        Some(
                            "Suggest what to do next by searching project files and connected apps",
                        ),
                        self.toggle("host.ambient-suggestions-enabled", true, t, window, cx),
                    ),
                    row(
                        "Open source licenses",
                        Some("Third-party notices for bundled dependencies"),
                        button("View", t.text_alpha_5, None, t)
                            .id("open-source-licenses")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(subtrees::OpenSubtree {
                                        path: subtrees::LICENSES_PATH.into(),
                                    }),
                                    cx,
                                );
                            })
                            .into_any_element(),
                    ),
                    row(
                        "Plugins",
                        Some("Allow ChatGPT to use installed plugins"),
                        self.toggle("features.plugins", true, t, window, cx),
                    ),
                ]
                .into_iter()
                .chain(crate::updater::managed_row(t, cx))
                .chain(crate::updater::automatic_row(t, cx))
                .chain(
                    cx.try_global::<crate::updater::Updates>()
                        .is_some_and(|updates| updates.available && !updates.managed)
                        .then(|| {
                            row(
                                "Beta updates",
                                Some("Get early versions before they reach everyone"),
                                self.toggle("host.update-beta", false, t, window, cx),
                            )
                        }),
                )
                .collect(),
            ),
            (
                "Composer",
                vec![
                    row(
                        "Show predictions",
                        Some("Predict your next message based on the conversation"),
                        self.toggle("session.messagePredictions", true, t, window, cx),
                    ),
                    row(
                        "Plain text composer",
                        Some(
                            "Keep code, Markdown, and links as literal text while writing messages",
                        ),
                        self.toggle("desktop.composerPlainTextMode", false, t, window, cx),
                    ),
                    row(
                        "Show context window usage",
                        None,
                        self.toggle("host.show-context-window-usage", false, t, window, cx),
                    ),
                    row(
                        "Send shortcut",
                        Some("Choose when Enter sends a prompt or inserts a new line"),
                        self.select_menu(
                            "desktop.composerEnterBehavior",
                            SEND_OPTIONS,
                            None,
                            |select| select.with_width(220.),
                            cx,
                        ),
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
                            window,
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
                            window,
                            cx,
                        ),
                    ),
                ],
            ),
            ("Notifications", notifications),
            (
                "Toys",
                vec![
                    row(
                        "Confetti cannon",
                        Some("Let Codex fire confetti in the app when you ask!"),
                        self.toggle("host.toys-confetti-enabled", false, t, window, cx),
                    ),
                    row(
                        "Audio visualizer",
                        Some(
                            "Animate conversation rails with system audio. Audio is processed on your device, never saved or uploaded.",
                        ),
                        self.toggle("host.toys-audio-visualizer-enabled", false, t, window, cx),
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
        if section == "Import" {
            return imports::render(window, cx);
        }
        imports::deactivate(cx);
        if let Some(view) = &self.page.read(cx).subtree {
            return view.clone().into_any_element();
        }
        if section == "Computer use" {
            return super::computer_use::render(window, cx);
        }
        if section == "Voice" {
            return voice::render(window, cx);
        }
        if section == "Keyboard shortcuts" {
            return keyboard_shortcuts::render(window, cx);
        }
        if section == "Usage & billing" {
            return usage::render(window, cx);
        }
        if section == "Personalization" {
            return div()
                .size_full()
                .child(self.page.read(cx).personalization.clone())
                .into_any_element();
        }
        if section == "Storage" {
            return self.storage.clone().into_any_element();
        }
        if matches!(section, "Plugins" | "Hooks" | "MCP servers") {
            return integrations::render(
                section,
                self.page.read(cx).integrations.clone(),
                window,
                cx,
            );
        }
        if section == "Archived chats" {
            return archived_chats::render(window, cx);
        }
        // Only `codex://settings/security` opens it: the original keeps it out of the nav.
        if section == "Security" {
            return security_settings::render(window, cx);
        }
        if section == "Ads controls" {
            return ads_controls::render(window, cx);
        }
        if section == "Passwords" {
            return passwords::render(window, cx);
        }
        if section == "Parental controls" {
            return parental_controls::render(window, cx);
        }
        if section == "Data controls" {
            return data_controls::render(window, cx);
        }
        if section == "Account" {
            return account::render(window, cx);
        }
        if section == "Trusted contact" {
            return trusted_contact::render(window, cx);
        }
        if section == "Worktrees" {
            return worktrees::render(self.page.clone(), window, cx);
        }
        if section == "Git" {
            return self.render_git(&t, window, cx);
        }
        if section == "Configuration" {
            return self.render_configuration(&t, window, cx);
        }
        if section == "Appearance" {
            return self.render_appearance(&t, window, cx);
        }
        if section == "Connections" {
            return super::connections::render(window, cx);
        }
        if section == "Mini & Pets" {
            return self.render_pets(&t, window, cx);
        }
        if section == "Computer history" {
            return computer_history::render(self.page.clone(), window, cx);
        }
        if section == "Code Review" {
            return code_review::render(self.page.clone(), window, cx);
        }
        // Unimplemented sections retain their title-only surface.
        let groups = match section {
            "General" => self.general(&t, window, cx),
            "Notifications" => self.notifications_page(&t, window, cx),
            "Browser" => self.browser(&t, window, cx),
            "Debug" => self.debug_page(&t, window, cx),
            "File type handlers" => self.file_type_handlers_page(&t, window, cx),
            "Appshots" => self.appshots_page(&t, window, cx),
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

/// One settings row: title, optional description (an `<a>…</a>` span renders as the inline
/// link) and the trailing control.
pub(crate) fn settings_row(
    title: &'static str,
    detail: Option<&'static str>,
    control: AnyElement,
    t: &Theme,
) -> Div {
    settings_row_content(
        title,
        detail.map(|value| description(value, t).into_any_element()),
        control,
        t,
    )
}

pub(crate) fn settings_row_content(
    title: impl Into<SharedString>,
    detail: Option<AnyElement>,
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
                .children(detail),
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
/// Toggling moves the thumb 2 px -> 14 px and fades the track colour, both 150 ms `ease-out`
/// (`refs/interactions/ms6a.json`); the tracks stay active under reduced motion.
pub(crate) fn switch(
    id: impl Into<gpui::ElementId>,
    on: bool,
    disabled: bool,
    t: &Theme,
) -> Stateful<Div> {
    switch_sized(id, on, disabled, t, (32., 20., 16.))
}

pub(crate) fn compact_switch(id: impl Into<gpui::ElementId>, on: bool, t: &Theme) -> Stateful<Div> {
    switch_sized(id, on, false, t, (28., 16., 12.))
}

fn switch_sized(
    id: impl Into<gpui::ElementId>,
    on: bool,
    disabled: bool,
    t: &Theme,
    dimensions: (f32, f32, f32),
) -> Stateful<Div> {
    let (track_off, track_on) = (t.switch_track, t.switch_track_checked);
    let (width, height, thumb_size) = dimensions;
    let thumb = shapes::Decoration {
        radius: Corners::all(px(9999.)),
        fill: Some(t.control_thumb),
        border: Some((px(1.), t.control_thumb)),
        shadows: vec![shadow(t.shadow_sm, 1., 2., -1., false)],
        ..Default::default()
    };
    div().id(id).flex_none().w(px(width)).h(px(height)).child(
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
                    let at = Bounds::new(
                        b.origin + point(px(x), px(2.)),
                        size(px(thumb_size), px(thumb_size)),
                    );
                    shapes::paint(&thumb, None, at, window, cx);
                };
                if disabled {
                    shapes::with_composited_opacity(153, b, window, cx, paint)
                } else {
                    paint(window)
                }
            },
        )
        .size_full(),
    )
}

/// A settings surface through `shapes`: CSS `radius`, corner shape, fill and 1 px border.
pub(super) fn deco(
    l: &Layer,
    radius: f32,
    corner: Corner,
    fill: Hsla,
    border: Option<Hsla>,
) -> AnyElement {
    let l = l.clone();
    in_layer(surface(radius, corner, fill, border), move || l.get()).into_any_element()
}

/// A control's surface on a settings card: Chromium blits it onto the card's opaque pixels in
/// the scroller's layer, as on the page (`shapes` takes the `.layer()` origin for its tiles).
pub(crate) fn on_card(radius: f32, corner: Corner, fill: Hsla, border: Option<Hsla>) -> AnyElement {
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
pub(super) type Layer = Rc<Cell<Point<Pixels>>>;

/// An empty first child of the layer's root that records the root's origin.
pub(super) fn probe(l: &Layer) -> impl IntoElement {
    let l = l.clone();
    canvas(move |b, _, _| l.set(b.origin), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
}

/// Toolbar `_Button` (`Change`, `View`: `bg-text/5`, transparent 1 px border; `Open System
/// Settings`: outlined).
pub(crate) fn button(
    label: impl Into<SharedString>,
    fill: Hsla,
    border: Option<Hsla>,
    t: &Theme,
) -> Div {
    div()
        .h(px(28.))
        .px(px(9.))
        .flex()
        .items_center()
        .relative()
        .child(on_card(12.5, Squircle, fill, border))
        .child(text(label, 13., 18., 430., t.text))
}
/// Dropdown trigger: value (with an optional leading app icon) at `weight`, and a chevron.
pub(crate) fn select(
    label: impl Into<SharedString>,
    weight: f32,
    leading: Option<AnyElement>,
    disabled: bool,
    open: bool,
    t: &Theme,
) -> Div {
    select_with(
        label,
        weight,
        leading,
        disabled,
        open,
        TriggerStyle::default(),
        t,
    )
}

/// A select trigger's text size, horizontal padding and fill.
#[derive(Clone, Copy)]
pub(crate) struct TriggerStyle {
    pub size: f32,
    pub line_height: f32,
    pub padding: (f32, f32),
    /// `bg-token-main-surface-primary` instead of the soft primary fill.
    pub surface: bool,
}

impl Default for TriggerStyle {
    fn default() -> Self {
        Self {
            size: 13.,
            line_height: 18.,
            padding: (13., 13.),
            surface: false,
        }
    }
}

pub(crate) fn select_with(
    label: impl Into<SharedString>,
    weight: f32,
    leading: Option<AnyElement>,
    disabled: bool,
    open: bool,
    style: TriggerStyle,
    t: &Theme,
) -> Div {
    let fill = if open {
        t.ghost_hover
    } else if style.surface {
        t.surface
    } else {
        t.primary_soft_alpha
    };
    div()
        .h(px(28.))
        .pl(px(style.padding.0))
        .pr(px(style.padding.1))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(4.))
        .relative()
        .child(on_card(12.5, Squircle, fill, Some(t.border)))
        .when(disabled, |d| d.opacity(0.4))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .children(leading)
                .child(text(label, style.size, style.line_height, weight, t.text)),
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

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;
    use serde_json::json;

    #[test]
    fn search_finds_rows_case_insensitively_on_real_pages() {
        for (label, _) in super::row_matches("") {
            assert!(
                super::section(label).is_some() || label == "MCP servers",
                "{label} is not a settings page"
            );
        }
        assert!(super::row_matches("speed").any(|r| r == ("General", "Speed")));
    }

    #[test]
    fn leaving_cloud_restores_the_selected_settings_page() {
        let mut app = gpui::HeadlessAppContext::with_asset_source(
            textsys::text_system(),
            std::sync::Arc::new(theme::Assets),
        );
        app.update(|cx| {
            let integrations = cx.new(|_| super::integrations::State::default());
            let (_, settings) = super::shell(
                &json!({}),
                &json!({}),
                Some("Codex Cloud"),
                integrations,
                cx,
            );
            let cloud = cx.new(crate::screens::cloud::CloudPage::new);
            settings.update(cx, |settings, cx| settings.set_cloud(cloud, cx));
            assert!(settings.read(cx).subtree.is_some());
            settings.update(cx, |settings, cx| settings.select_section("General", cx));
            assert_eq!(settings.read(cx).section, "General");
            assert!(settings.read(cx).subtree.is_none());
        });
    }

    #[test]
    fn top_level_config_keys_read_and_write_without_a_store() {
        let mut app = gpui::HeadlessAppContext::with_asset_source(
            textsys::text_system(),
            std::sync::Arc::new(theme::Assets),
        );
        app.update(|cx| {
            let integrations = cx.new(|_| super::integrations::State::default());
            let (_, settings) = super::shell(
                &json!({"approval_policy": "never", "desktop": {"x": 1}}),
                &json!({}),
                Some("Configuration"),
                integrations,
                cx,
            );
            assert_eq!(
                settings.read(cx).choice("approval_policy", "on-request"),
                "never"
            );
            assert_eq!(
                settings.read(cx).choice("sandbox_mode", "read-only"),
                "read-only"
            );
            settings.update(cx, |settings, cx| {
                settings.set("approval_policy", json!("on-request"), cx);
                settings.set(super::browser_settings::FULL_CDP, json!("allow"), cx);
            });
            let page = settings.read(cx);
            assert_eq!(page.choice("approval_policy", "never"), "on-request");
            assert_eq!(page.values["desktop"]["x"], 1);
            assert_eq!(
                page.choice(super::browser_settings::FULL_CDP, "deny"),
                "allow"
            );
            assert_eq!(
                page.values["browser_use"]["default_origin_policy"]["full_cdp_access"],
                "allow"
            );
        });
    }

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
    fn etc_shells_parse_to_unique_paths() {
        let text = "# /etc/shells: valid login shells\n/bin/sh\n\n  /bin/bash \n/bin/sh\n# /bin/zsh\nnologin\n/usr/bin/fish\n";
        assert_eq!(
            super::parse_shells(text),
            ["/bin/sh", "/bin/bash", "/usr/bin/fish"]
        );
    }
}
