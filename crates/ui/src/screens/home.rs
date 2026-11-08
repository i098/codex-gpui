//! `home` parity screen: hero, composer utility bar and the Home composer, laid out as
//! refs/home (CSS px).

use super::composer::fixture_composer;
use crate::composer::{Composer, label, pill};
use crate::{Shell, focus, icon::icon, rounded_clip, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Bounds, Context, Corners, Div, Entity,
    Focusable as _, Global, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Render, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window, actions, anchored, canvas, deferred, div,
    point, px, relative,
};
use theme::{ActiveTheme as _, Theme};

static PROJECT_TOOLTIP_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| textsys::letter_spacing(-0.15));
static CONTROL_TEXT_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(Default::default);

mod work;
pub use work::{LibraryFile, LibraryLoad, WorkEvent, local_matches};

/// Identifies the window whose animations a static capture finishes.
#[derive(Clone, Copy)]
pub struct FinishedAnimation(pub AnyWindowHandle);

impl Global for FinishedAnimation {}

/// Whether a static capture of `window` shows its animations finished.
pub fn animation_finished(window: &Window, cx: &App) -> bool {
    cx.try_global::<FinishedAnimation>()
        .is_some_and(|state| state.0 == window.window_handle())
}

actions!(
    home,
    [
        OpenFolder,
        /// Account access error page: read the account again.
        ReloadAccount,
        /// Account access error page: open the `Log out?` dialog.
        SignOutAccount,
        /// Rate limit reset banner: `See resets`.
        SeeResets,
        /// Subagents banner: `Try now` enables `features.multi_agent`.
        TryMultiAgent,
        ClearProject,
        SetupWindowsSandboxElevated,
        SetupWindowsSandboxUnelevated
    ]
);

/// A Home announcement from the account's state (`home-announcement-state`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HomeBanner {
    RateLimitReset,
    Subagents,
}

/// A banner's dismiss `×`: remember it so the banner stays hidden.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = home, no_json)]
pub struct DismissBanner(pub HomeBanner);

/// A `composer/items` task suggestion (`home-task-suggestions`): its card copy, the prompt a
/// click puts in the composer, and the raw item the interaction reports echo back.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub auto_send: bool,
    pub item: serde_json::Value,
}

/// A suggestion card's click (its id): fill the composer with its prompt.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = home, no_json)]
pub struct UseSuggestion(pub String);

/// A suggestion card's `Dismiss suggestion` (its id).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = home, no_json)]
pub struct DismissSuggestion(pub String);

/// `Choose project` menu: whether it is open, where its chip was last laid out, and its
/// `Search projects` field.
pub struct ProjectMenu {
    pub open: bool,
    bounds: Bounds<Pixels>,
    search: Entity<Composer>,
}

impl ProjectMenu {
    pub fn new(cx: &mut Context<HomeMain>) -> Self {
        let search = cx
            .new(|cx| Composer::embedded_field("Search projects", "", px(13.), px(18.571_428), cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self {
            open: false,
            bounds: Bounds::default(),
            search,
        }
    }
}

/// `Work in` menu state: whether it is open, and where its chip was last laid out.
#[derive(Default)]
pub struct LocationMenu {
    pub open: bool,
    pub mode: super::hotkey::Mode,
    pub worktree_available: Option<bool>,
    /// The selected Git project's branches and the starting branch (`None`: its `HEAD`).
    pub branches: Vec<String>,
    pub branch: Option<String>,
    /// Local environments `(name, config path)` and the chosen config path.
    pub environments: Vec<(String, String)>,
    pub environment: Option<String>,
    /// Cloud environments `(label, id)`; while empty, `Cloud` stays disabled.
    pub cloud_environments: Vec<(String, String)>,
    pub cloud_environment: Option<String>,
    selected: usize,
    bounds: Bounds<Pixels>,
}

/// The utility bar's dropdowns besides `Work in`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picker {
    Branch,
    Environment,
}

/// `Work in` rows: `This computer`, `New worktree`, `Remote`, `Cloud`.
const LOCATION_MODES: [Option<super::hotkey::Mode>; 4] = [
    Some(super::hotkey::Mode::Local),
    Some(super::hotkey::Mode::Worktree),
    None,
    Some(super::hotkey::Mode::Cloud),
];

/// The original's four rotating Chat home headlines (`p9`), one picked per visit.
pub const CHAT_HEADLINES: [&str; 4] = [
    "Ready when you are.",
    "What’s on your mind today?",
    "What’s on the agenda today?",
    "Where should we begin?",
];

/// The Work home headline (`home.hero.workModeWhatShouldWeWorkOn`).
pub const WORK_HEADLINE: &str = "What should we work on?";

pub struct HomeMain {
    pub composer: Entity<Composer>,
    pub project: Option<SharedString>,
    pub location: LocationMenu,
    pub projects: ProjectMenu,
    /// The announcement slot's `Try ChatGPT Voice` beacon is shown. BOUNDARY: the original
    /// shows it when `/beacons/home` offers it; the port has no such feed yet, so it starts
    /// hidden (flows.md Home rows 5 and 6) and dismissal lasts for the session only.
    pub beacon: bool,
    /// The eligible account banner, shown in the beacon's slot in its place.
    pub banner: Option<HomeBanner>,
    /// `composer/items` suggestions (at most 3), and whether they are still loading.
    pub suggestions: Vec<Suggestion>,
    pub suggestions_loading: bool,
    /// Windows sandbox setup is needed and in progress.
    pub windows_sandbox_setup_needed: bool,
    pub windows_sandbox_setup_pending: bool,
    /// The ChatGPT or Work headline; `None` shows the Codex hero.
    pub headline: Option<&'static str>,
    pub picker: Option<Picker>,
    /// Where each [`Picker`] trigger was last laid out.
    picker_bounds: [Bounds<Pixels>; 2],
    /// `account/read` failed: Home shows `We couldn’t load your account` instead.
    pub account_error: bool,
    /// The Work rail's `Files`, `Plugins` and run-location menus.
    pub work: work::WorkControls,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = fixture_composer(window, cx, Composer::home);
        Self::with_composer(composer, None, cx)
    }

    /// Home around an existing `composer`, titled with `project`.
    pub fn with_composer(
        composer: Entity<Composer>,
        project: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            composer,
            project,
            location: LocationMenu::default(),
            projects: ProjectMenu::new(cx),
            beacon: false,
            banner: None,
            suggestions: Vec::new(),
            suggestions_loading: false,
            windows_sandbox_setup_needed: false,
            windows_sandbox_setup_pending: false,
            headline: None,
            picker: None,
            picker_bounds: Default::default(),
            account_error: false,
            work: work::WorkControls::new(cx),
        }
    }

    /// Opens `picker` (closing any other) or closes it when it is already open.
    pub fn toggle_picker(&mut self, picker: Picker, cx: &mut Context<Self>) {
        self.location.open = false;
        self.picker = (self.picker != Some(picker)).then_some(picker);
        cx.notify();
    }
}

impl HomeMain {
    pub fn set_beacon(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.beacon = visible;
        cx.notify();
    }

    /// Follows the sidebar's mode (a [`crate::screens::sidebar::MODES`] index).
    pub fn set_mode(&mut self, mode: usize, cx: &mut Context<Self>) {
        let (headline, placeholder) = match crate::screens::sidebar::MODES[mode] {
            "ChatGPT" => {
                // `Math.random()` in the original; the clock is random enough for a greeting.
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize);
                (
                    Some(CHAT_HEADLINES[nanos % CHAT_HEADLINES.len()]),
                    "Ask ChatGPT",
                )
            }
            "Work" => (Some(WORK_HEADLINE), "Work with ChatGPT"),
            _ => (None, "Do anything"),
        };
        self.headline = headline;
        self.close_work_menus();
        let chat = placeholder == "Ask ChatGPT";
        self.composer.update(cx, |composer, cx| {
            composer.set_placeholder(placeholder, cx);
            composer.set_chat(chat, cx);
        });
        cx.notify();
    }

    /// The ChatGPT home's `Chat | Work` pill (224×36, centered in the toolbar row); the
    /// selected segment is 120 px wide, the other 104.
    fn chat_work_toggle(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let work = self.headline == Some(WORK_HEADLINE);
        let segment =
            |label: &'static str, selected: bool, window: &mut Window, cx: &mut Context<Self>| {
                let focus =
                    focus::handle(SharedString::from(format!("home-{label}")), 0, window, cx);
                let control = div()
                    .id(label)
                    .role(gpui::Role::Tab)
                    .aria_label(label)
                    .when(selected, |d| d.aria_selected(true))
                    .h_full()
                    .w(px(if selected { 120. } else { 104. }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .when(selected, |d| {
                        d.bg(crate::over(t.text_alpha_5, t.background_panel))
                            .border_1()
                            .border_color(t.border)
                    })
                    .child(text(label, 14., 20., 500., t.text));
                focus::ring(control, &focus, Theme::RADIUS_FULL, px(2.), cx)
            };
        let chat =
            segment("Chat", !work, window, cx).on_click(crate::dispatch::<crate::SwitchMode1>);
        let work =
            segment("Work", work, window, cx).on_click(crate::dispatch::<crate::SwitchMode2>);
        div().h_full().flex().justify_center().items_center().child(
            div()
                .id("home-chat-work")
                .role(gpui::Role::TabList)
                .h(px(36.))
                .flex()
                .rounded_full()
                .bg(t.text_alpha_5)
                .child(chat)
                .child(work),
        )
    }

    /// `aside` 736×74: copy on the left, the call to action and the dismiss `×` on the right.
    /// A [`HomeBanner`] wins over the Voice beacon, as the original's priority arbitration.
    fn beacon(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (title, body, cta, dismiss_label): (&str, &str, &'static str, &'static str) = match self
            .banner
        {
            Some(HomeBanner::RateLimitReset) => (
                "You have a new rate limit reset available",
                "You were granted a rate limit reset that will expire in 30 days.",
                "See resets",
                "Dismiss rate limit reset banner",
            ),
            Some(HomeBanner::Subagents) => (
                "Subagents in Codex",
                "Delegate work to subagents that work in parallel. Note: may increase token usage.",
                "Try now",
                "Dismiss subagent banner",
            ),
            None => (
                "Try ChatGPT Voice",
                "Orchestrate tasks, connect tools, and explore code",
                "Start Voice",
                "Dismiss ChatGPT beacon banner",
            ),
        };
        let banner = self.banner;
        let start_focus = focus::handle("home-beacon-start", 0, window, cx);
        let close_focus = focus::handle("home-beacon-dismiss", 0, window, cx);
        let hover = t.ghost_hover;
        div()
            .h(px(74.))
            .mb(px(8.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(text(title, 14., 20., 500., t.text))
                    .child(text(body, 13., 18., 400., t.text_tertiary)),
            )
            .child(
                focus::ring(
                    div().id("home-beacon-start"),
                    &start_focus,
                    Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                )
                .role(gpui::Role::Button)
                .aria_label(cta)
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(t.composer_primary)
                .child(text(cta, 13., 18., 400., t.text_composer_primary))
                .on_click(move |_, window, cx| match banner {
                    Some(HomeBanner::RateLimitReset) => {
                        window.dispatch_action(Box::new(SeeResets), cx)
                    }
                    Some(HomeBanner::Subagents) => {
                        window.dispatch_action(Box::new(TryMultiAgent), cx)
                    }
                    None => window.dispatch_action(Box::new(crate::StartVoiceChat), cx),
                }),
            )
            .child(
                focus::ring(
                    div().id("home-beacon-dismiss"),
                    &close_focus,
                    Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                )
                .role(gpui::Role::Button)
                .aria_label(dismiss_label)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(move |s| s.bg(hover))
                .child(icon("close", 16., t.text_tertiary))
                .on_click(cx.listener(move |this, _, window, cx| match banner {
                    Some(banner) => {
                        window.dispatch_action(Box::new(DismissBanner(banner)), cx);
                        this.banner = None;
                        cx.notify();
                    }
                    None => this.set_beacon(false, cx),
                })),
            )
    }

    /// The suggestion cards under the composer; skeleton rows while they load.
    fn suggestions(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let list = div().pt(px(12.)).flex().flex_col().gap(px(8.));
        if self.suggestions_loading {
            return list.children(
                (0..3).map(|_| div().h(px(56.)).rounded(px(16.)).bg(t.surface_secondary)),
            );
        }
        let hover = t.ghost_hover;
        list.children(
            self.suggestions
                .iter()
                .enumerate()
                .map(|(index, suggestion)| {
                    let card_focus = focus::handle("home-suggestion", index as isize, window, cx);
                    let close_focus =
                        focus::handle("home-suggestion-dismiss", index as isize, window, cx);
                    let (use_id, dismiss_id) = (suggestion.id.clone(), suggestion.id.clone());
                    focus::ring(
                        div().id(("home-suggestion", index)),
                        &card_focus,
                        px(16.),
                        px(2.),
                        cx,
                    )
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(suggestion.title.clone()))
                    .px(px(16.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(t.border)
                    .hover(move |s| s.bg(hover))
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(UseSuggestion(use_id.clone())), cx)
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(text(suggestion.title.clone(), 14., 20., 500., t.text))
                            .children(
                                suggestion
                                    .description
                                    .clone()
                                    .map(|body| text(body, 13., 18., 400., t.text_tertiary)),
                            ),
                    )
                    .child(
                        focus::ring(
                            div().id(("home-suggestion-dismiss", index)),
                            &close_focus,
                            Theme::RADIUS_FULL,
                            px(2.),
                            cx,
                        )
                        .role(gpui::Role::Button)
                        .aria_label("Dismiss suggestion")
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .hover(move |s| s.bg(hover))
                        .child(icon("close", 16., t.text_tertiary))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            window.dispatch_action(
                                Box::new(DismissSuggestion(dismiss_id.clone())),
                                cx,
                            )
                        }),
                    )
                }),
        )
    }

    fn windows_sandbox_banner(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let elevated_focus = focus::handle("home-windows-sandbox-elevated", 0, window, cx);
        let unelevated_focus = focus::handle("home-windows-sandbox-unelevated", 0, window, cx);
        let actions = if self.windows_sandbox_setup_pending {
            div().child(text("Setting up…", 13., 18., 400., t.text_tertiary))
        } else {
            div().flex().gap(px(8.)).children([
                focus::ring(
                    div().id("home-windows-sandbox-elevated"),
                    &elevated_focus,
                    Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                )
                .role(gpui::Role::Button)
                .aria_label("Set up Windows sandbox as administrator")
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(t.composer_primary)
                .child(text(
                    "Run as administrator",
                    13.,
                    18.,
                    400.,
                    t.text_composer_primary,
                ))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(SetupWindowsSandboxElevated), cx)
                }),
                focus::ring(
                    div().id("home-windows-sandbox-unelevated"),
                    &unelevated_focus,
                    Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                )
                .role(gpui::Role::Button)
                .aria_label("Set up Windows sandbox without administrator access")
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(t.text_alpha_5)
                .child(text("Without administrator", 13., 18., 400., t.text))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(SetupWindowsSandboxUnelevated), cx)
                }),
            ])
        };
        div()
            .h(px(74.))
            .mb(px(8.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(text("Set up Windows sandbox", 14., 20., 500., t.text))
                    .child(text(
                        "Choose how to configure the protected environment",
                        13.,
                        18.,
                        400.,
                        t.text_tertiary,
                    )),
            )
            .child(actions)
    }
}

/// The Home main panel (no shell).
pub fn home_main(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| HomeMain::new(window, cx)).into()
}

/// The Home sidebar and `main` in the shell, with an unread Explore destination (the
/// composer reference shows one; the Home reference does not).
pub fn shell(main: AnyView, cx: &mut App) -> AnyView {
    let sidebar = super::sidebar::home_sidebar(cx, None);
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, main);
        shell.unread_destinations = Some(std::sync::Arc::from([SharedString::from("explore")]));
        shell
    })
    .into()
}

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let main = home_main(window, cx);
    let sidebar = super::sidebar::home_sidebar(cx, None);
    cx.new(|_| Shell::new(sidebar, main)).into()
}

fn open_folder_key(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
        window.dispatch_action(Box::new(OpenFolder), cx);
        cx.stop_propagation();
    }
}

/// `Choose project` / `This computer` chip of the utility bar. The pill is `overflow: hidden`:
/// Skia draws its icon under that AA clip (its padding box; light has no fitted images).
/// `compact` hides the label, as the original's `collapse` attribute does.
#[allow(clippy::too_many_arguments)]
fn chip(
    id: &'static str,
    glyph: &str,
    s: SharedString,
    fg: Hsla,
    compact: bool,
    t: &Theme,
    focus: &Entity<focus::ControlFocus>,
    cx: &App,
) -> Stateful<Div> {
    focus::shadow_ring(
        div().id((id, 0_usize)).relative().flex_none(),
        focus,
        Theme::RADIUS_FULL,
        px(2.),
        cx,
    )
    .when(id == "choose-project", |chip| {
        chip.role(gpui::Role::Button)
            .font_features(PROJECT_TOOLTIP_FEATURES.clone())
            .aria_label("Choose project")
    })
    .child(
        pill(id, t)
            .font_features(CONTROL_TEXT_FEATURES.clone())
            .child(rounded_clip(
                px(13.),
                shapes::Corner::Round,
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .gap(px(6.))
                    .child(icon(glyph, 16., fg).runtime())
                    .when(!compact, |d| d.child(label(s, fg))),
            )),
    )
}

/// Opens or closes the `Choose project` menu (`None` toggles); opening clears and focuses
/// its search field.
fn set_project_menu(
    owner: &Entity<HomeMain>,
    open: Option<bool>,
    window: &mut Window,
    cx: &mut App,
) {
    owner.update(cx, |main, cx| {
        main.projects.open = open.unwrap_or(!main.projects.open);
        if main.projects.open {
            let search = main.projects.search.clone();
            search.update(cx, |search, cx| search.set_text("", cx));
            window.focus(&search.focus_handle(cx), cx);
        }
        cx.notify();
    });
}

/// The original's composer project picker: `Search projects`, the matching projects (the
/// selected one checked), then `New project` and, with a project selected, `Don't work in a
/// project`.
fn project_menu(
    owner: &Entity<HomeMain>,
    menu: &ProjectMenu,
    composer: &Composer,
    t: &Theme,
    window: &Window,
    cx: &App,
) -> Option<gpui::AnyElement> {
    if !menu.open {
        return None;
    }
    let query = menu.search.read(cx).field_text().trim().to_lowercase();
    let selected = composer.search_roots().first().cloned();
    let matches: Vec<(SharedString, String)> = composer
        .slash_projects()
        .iter()
        .filter(|(label, _)| label.to_lowercase().contains(&query))
        .cloned()
        .collect();
    let hover = t.menu_item_hover;
    // cmdk highlights the first item, so it shows the hover fill and Enter activates it.
    let row = |id: &'static str,
               index: usize,
               glyph: &'static str,
               label: SharedString,
               highlighted: bool| {
        div()
            .id((id, index))
            .role(gpui::Role::MenuItem)
            .aria_label(label.clone())
            .px(px(8.))
            .py(px(5.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(15.))
            .when(highlighted, |d| d.bg(hover))
            .hover(move |s| s.bg(hover))
            .child(icon(glyph, 16., t.text).opacity(0.75))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(text(label, 13., 18.571_428, 430., t.text)),
            )
    };
    // `py-(--spacing)` around a 1 px `bg-border` line, inset by the row padding.
    let separator = || {
        div()
            .px(px(8.))
            .py(px(4.))
            .child(div().h(px(1.)).bg(t.border))
    };
    let close = |owner: &Entity<HomeMain>| {
        let owner = owner.clone();
        move |window: &mut Window, cx: &mut App| set_project_menu(&owner, Some(false), window, cx)
    };
    let mut actions: Vec<(
        &'static str,
        &'static str,
        &'static str,
        Box<dyn gpui::Action>,
    )> = Vec::new();
    if query.is_empty() {
        actions.push((
            "project-menu-new",
            "settings-mcp-plus",
            "New project",
            Box::new(super::new_project::NewProject),
        ));
    }
    if selected.is_some() {
        // [INFERENCE] the clear row's `LeftIcon` comes from the caller; `close` stands in.
        actions.push((
            "project-menu-clear",
            "close",
            "Don't work in a project",
            Box::new(ClearProject),
        ));
    }
    let enter = matches
        .first()
        .map(|(_, root)| Box::new(super::sidebar::SelectProject(root.clone())) as Box<_>)
        .or_else(|| actions.first().map(|action| action.3.boxed_clone()));
    let footer_first = matches.is_empty();
    let footer: Vec<_> = actions
        .into_iter()
        .enumerate()
        .map(|(i, (id, glyph, label, action))| {
            let close = close(owner);
            row(id, 0, glyph, label.into(), footer_first && i == 0).on_click(
                move |_, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx);
                    close(window, cx);
                },
            )
        })
        .collect();
    let list = if matches.is_empty() {
        // `No projects` with no query, `No projects found` for a search without matches.
        div().p(px(8.)).child(text(
            if query.is_empty() {
                "No projects"
            } else {
                "No projects found"
            },
            13.,
            18.571_428,
            430.,
            t.text_tertiary,
        ))
    } else {
        div().children(matches.into_iter().enumerate().map(|(i, (label, root))| {
            let checked = selected.as_ref() == Some(&root);
            let close = close(owner);
            row("project-option", i, "folder", label, i == 0)
                .child(
                    div()
                        .size(px(16.))
                        .children(checked.then(|| icon("menu-check", 16., t.text))),
                )
                .on_click(move |_, window, cx| {
                    window
                        .dispatch_action(Box::new(super::sidebar::SelectProject(root.clone())), cx);
                    close(window, cx);
                })
        }))
    };
    let search_row = div()
        .px(px(8.))
        .py(px(5.))
        .flex()
        .items_center()
        .gap(px(6.))
        .child(icon("shortcut-search", 14., t.text_tertiary))
        .child(div().flex_1().min_w_0().child(menu.search.clone()));
    let keys = close(owner);
    let popup = crate::composer::popover(t, 260., |d| {
        d.child(search_row)
            .child(separator())
            .child(list)
            .when(!footer.is_empty(), |d| {
                d.child(separator()).children(footer)
            })
    })
    .id("project-menu")
    .role(gpui::Role::Menu)
    .aria_label("Choose project")
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    // Enter is the field's `Submit` binding, which would blur it. Without a highlighted item it
    // does nothing, as the original's `preventDefault` does.
    .capture_action({
        let close = close(owner);
        move |_: &crate::composer::Submit, window, cx| {
            cx.stop_propagation();
            let Some(action) = enter.as_ref() else { return };
            window.dispatch_action(action.boxed_clone(), cx);
            close(window, cx);
        }
    })
    .capture_key_down(move |event, window, cx| {
        if event.keystroke.key == "escape" {
            cx.stop_propagation();
            keys(window, cx);
        }
    });
    let trigger = menu.bounds;
    let dismiss = close(owner);
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| dismiss(window, cx))
                    .child(
                        anchored()
                            // `sideOffset` 0 plus the content's `m-px`, less the popover's
                            // shadow-layer inset (measured against the original).
                            .position(point(trigger.left() + px(1.25), trigger.top() - px(18.1)))
                            .anchor(gpui::Anchor::BottomLeft)
                            .snap_to_window_with_margin(px(8.))
                            .child(popup),
                    ),
            ),
        )
        .with_priority(4)
        .into_any_element(),
    )
}

/// Opens or closes the `Work in` menu: `None` toggles.
fn set_location(owner: &Entity<HomeMain>, open: Option<bool>, cx: &mut App) {
    owner.update(cx, |main, cx| {
        main.location.open = open.unwrap_or(!main.location.open);
        main.picker = None;
        main.location.selected = LOCATION_MODES
            .iter()
            .position(|mode| *mode == Some(main.location.mode))
            .unwrap_or(0);
        cx.notify();
    });
}

fn pick_location(owner: &Entity<HomeMain>, index: usize, cx: &mut App) {
    owner.update(cx, |main, cx| {
        let location = &mut main.location;
        location.mode = LOCATION_MODES[index].unwrap_or_default();
        if location.mode == super::hotkey::Mode::Cloud && location.cloud_environment.is_none() {
            location.cloud_environment = location.cloud_environments.first().map(|e| e.1.clone());
        }
        location.open = false;
        cx.notify();
    });
}

/// Which `Work in` rows can be picked: worktree and cloud starts need a Git project, cloud
/// also a loaded cloud environment. `Remote` has no native remote hosts yet.
fn location_enabled(location: &LocationMenu, has_project: bool) -> [bool; 4] {
    [
        true,
        has_project,
        false,
        has_project && !location.cloud_environments.is_empty(),
    ]
}

/// The next enabled row after (`step` 1) or before (`step` -1) `from`, wrapping.
fn step_enabled(enabled: [bool; 4], from: usize, step: isize) -> usize {
    (1..=4)
        .map(|offset| (from as isize + step * offset).rem_euclid(4) as usize)
        .find(|&index| enabled[index])
        .unwrap_or(from)
}

/// What a menu row's click does (`None`: disabled).
type RowAction = Option<std::rc::Rc<dyn Fn(&mut Window, &mut App)>>;

/// A menu row: label, leading icon, checked, and its [`RowAction`].
type Row = (SharedString, Option<&'static str>, bool, RowAction);

fn menu_row(id: (&'static str, usize), row: Row, highlighted: bool, t: &Theme) -> gpui::AnyElement {
    let (label, glyph, checked, action) = row;
    let enabled = action.is_some();
    div()
        .id(id)
        .role(gpui::Role::MenuItemRadio)
        .aria_label(label.clone())
        .aria_disabled(!enabled)
        .px(px(8.))
        .py(px(6.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(12.))
        .opacity(if enabled { 1. } else { 0.5 })
        .when(highlighted, |d| d.bg(t.menu_item_hover))
        .when(enabled, |d| {
            let hover = t.menu_item_hover;
            d.hover(move |s| s.bg(hover))
        })
        .when_some(glyph, |d, glyph| d.child(icon(glyph, 16., t.text_tertiary)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(text(label, 14., 20., 430., t.text)),
        )
        .child(
            div()
                .size(px(16.))
                .children(checked.then(|| icon("menu-check", 16., t.text))),
        )
        .when_some(action, |d, action| {
            d.on_click(move |_, window, cx| action(window, cx))
        })
        .into_any_element()
}

/// `popup` over a full-window backdrop that closes it on a click anywhere else. It opens
/// upward from `trigger`: the bar sits at the bottom of the window.
fn floating(
    popup: Stateful<Div>,
    trigger: Bounds<Pixels>,
    dismiss: impl Fn(&mut App) + 'static,
    window: &Window,
) -> gpui::AnyElement {
    let position = point(trigger.left(), trigger.top() - px(4.));
    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .w(window.viewport_size().width)
                .h(window.viewport_size().height)
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| dismiss(cx))
                .child(
                    anchored()
                        .position(position)
                        .anchor(gpui::Anchor::BottomLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            popup
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                        ),
                ),
        ),
    )
    .with_priority(4)
    .into_any_element()
}

/// `Work in`: local, managed-worktree and cloud starts of the selected project.
fn location_menu(
    owner: &Entity<HomeMain>,
    location: &LocationMenu,
    has_project: bool,
    t: &Theme,
    window: &Window,
) -> Option<gpui::AnyElement> {
    if !location.open {
        return None;
    }
    let trigger = location.bounds;
    let enabled = location_enabled(location, has_project);
    // The original's rows: `Work in` label, This computer, New worktree (the port's managed
    // worktree start, only with a project), Cloud, Remote. The index is the row's place in
    // `LOCATION_MODES`, which `pick_location` and the keyboard walk use.
    // (index into `pick_location`, label, icon)
    let rows = [
        Some((0, "This computer", "laptop")),
        has_project.then_some((1, "New worktree", "thread-fork")),
        Some((3, "Cloud", "settings-codex-cloud")),
        Some((2, "Remote", "project-marker-globe")),
    ]
    .into_iter()
    .flatten()
    .map(|(i, label, glyph)| {
        let owner = owner.clone();
        let enabled = enabled[i];
        let checked = LOCATION_MODES[i] == Some(location.mode);
        div()
            .id(("work-in-option", i))
            .role(gpui::Role::MenuItemRadio)
            .aria_label(label)
            .aria_disabled(!enabled)
            .px(px(8.))
            .py(px(5.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(15.))
            .when(i == location.selected, |d| d.bg(t.menu_item_hover))
            .when(enabled, |d| {
                let hover = t.menu_item_hover;
                d.hover(move |s| s.bg(hover))
            })
            .child(icon(glyph, 16., t.text).opacity(0.75))
            .child(
                div()
                    .flex_1()
                    .child(text(label, 13., 18.5714, 430., t.text)),
            )
            .when(checked, |d| {
                d.child(icon("archive-check", 16., t.text).opacity(0.75))
            })
            .when(enabled, |d| {
                d.on_click(move |_, _, cx| pick_location(&owner, i, cx))
            })
    });
    let dismiss = owner.clone();
    let popup = crate::composer::popover(t, 280., |d| {
        d.child(div().px(px(8.)).py(px(4.)).child(text(
            "Work in",
            13.,
            18.5714,
            430.,
            t.text_tertiary,
        )))
        .children(rows)
    })
    .id("work-in-menu")
    .role(gpui::Role::Menu)
    .aria_label("Work in")
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    // Opens upward from the chip: the bar sits at the bottom of the window. A full-window
    // backdrop under the popup closes it on a click anywhere else, the chip included.
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        set_location(&dismiss, Some(false), cx)
                    })
                    .child(
                        anchored()
                            // `sideOffset` 0 plus the content's `m-px`; the popover's 16.5 px
                            // shadow-layer inset comes off too (measured against the original).
                            .position(point(trigger.left() + px(1.), trigger.top() - px(18.5)))
                            .anchor(gpui::Anchor::BottomLeft)
                            .snap_to_window_with_margin(px(8.))
                            .child(popup),
                    ),
            ),
        )
        .with_priority(4)
        .into_any_element(),
    )
}

/// Closes the open picker after running `f` on the Home state.
fn pick(
    owner: &Entity<HomeMain>,
    f: impl Fn(&mut HomeMain, &mut Window, &mut App) + 'static,
) -> RowAction {
    let owner = owner.clone();
    Some(std::rc::Rc::new(
        move |window: &mut Window, cx: &mut App| {
            owner.update(cx, |main, cx| {
                main.picker = None;
                f(main, window, cx);
                cx.notify();
            });
        },
    ))
}

/// The open [`Picker`]: the worktree starting branch, or the worktree
/// (`composer.worktreeEnvironment`) or cloud (`composer.environmentSelector`) environment.
fn picker_menu(
    main: &HomeMain,
    owner: &Entity<HomeMain>,
    t: &Theme,
    window: &Window,
) -> Option<gpui::AnyElement> {
    let picker = main.picker?;
    let location = &main.location;
    let rows: Vec<Row> = match picker {
        Picker::Branch => location
            .branches
            .iter()
            .map(|branch| {
                let checked = location.branch.as_ref() == Some(branch);
                let choice = branch.clone();
                let action = pick(owner, move |main, _, _| {
                    main.location.branch = Some(choice.clone())
                });
                (branch.clone().into(), Some("settings-git"), checked, action)
            })
            .collect(),
        Picker::Environment if location.mode == super::hotkey::Mode::Cloud => location
            .cloud_environments
            .iter()
            .map(|(label, id)| {
                let checked = location.cloud_environment.as_ref() == Some(id);
                let choice = id.clone();
                let action = pick(owner, move |main, _, _| {
                    main.location.cloud_environment = Some(choice.clone())
                });
                (label.clone().into(), None, checked, action)
            })
            .collect(),
        Picker::Environment => std::iter::once((
            "Work without environment".into(),
            None,
            location.environment.is_none(),
            pick(owner, |main, _, _| main.location.environment = None),
        ))
        .chain(location.environments.iter().map(|(name, path)| {
            let checked = location.environment.as_ref() == Some(path);
            let choice = path.clone();
            let action = pick(owner, move |main, _, _| {
                main.location.environment = Some(choice.clone())
            });
            (name.clone().into(), None, checked, action)
        }))
        .collect(),
    };
    let (id, label): (&'static str, &'static str) = match picker {
        Picker::Branch => ("branch-menu", "What branch should this chat start from?"),
        Picker::Environment => ("environment-menu", "Select environment"),
    };
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| menu_row((id, index), row, false, t));
    let popup = crate::composer::popover(t, 280., |d| d.children(rows))
        .id(id)
        .role(gpui::Role::Menu)
        .aria_label(label)
        .on_key_down({
            let owner = owner.clone();
            move |event, _, cx| {
                if event.keystroke.key == "escape" {
                    owner.update(cx, |main, cx| {
                        main.picker = None;
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            }
        });
    let trigger = main.picker_bounds[picker as usize];
    let dismiss = owner.clone();
    Some(floating(
        popup,
        trigger,
        move |cx| {
            dismiss.update(cx, |main, cx| {
                main.picker = None;
                cx.notify();
            })
        },
        window,
    ))
}

/// Records where `picker`'s trigger was laid out, so its menu can anchor there.
fn picker_bounds(owner: &Entity<HomeMain>, picker: Picker) -> impl IntoElement {
    let owner = owner.clone();
    canvas(
        move |bounds, _, cx| {
            owner.update(cx, |main, _| main.picker_bounds[picker as usize] = bounds)
        },
        |_, (), _, _| {},
    )
    .absolute()
    .size_full()
}

fn scroll_fades(offset: Pixels, maximum: Pixels) -> (Pixels, Pixels) {
    if maximum <= px(0.) {
        return (px(0.), px(0.));
    }
    let progress = -offset / maximum;
    // `edge-fade-horizontal` reaches full width at 0.1% and 99.9%.
    (
        px(16. * (progress * 1000.).clamp(0., 1.)),
        px(16. * ((1. - progress) * 1000.).clamp(0., 1.)),
    )
}

/// `Composer utility bar` (`_rail_1hrlq` attached above the composer): project, `Work in`
/// and, for worktree and cloud starts, the starting branch and environment.
#[allow(clippy::too_many_arguments)]
fn utility_bar(
    owner: &Entity<HomeMain>,
    location: &LocationMenu,
    project: Option<SharedString>,
    picker: Option<gpui::AnyElement>,
    project_menu: Option<gpui::AnyElement>,
    compact: bool,
    t: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let project_focus = focus::handle("choose-project", 0, window, cx);
    let work_focus = focus::handle("work-in", 0, window, cx);
    let scroll = window
        .use_keyed_state("home-utility-scroll", cx, |window, cx| {
            for (index, focus) in [&project_focus, &work_focus].into_iter().enumerate() {
                let handle = focus.read(cx).handle.clone();
                cx.on_focus(&handle, window, move |_, window, cx| {
                    cx.defer_in(window, move |scroll: &mut ScrollHandle, _, cx| {
                        scroll.scroll_to_item(index);
                        cx.notify();
                    });
                })
                .detach();
            }
            ScrollHandle::new()
        })
        .read(cx)
        .clone();
    let has_project = project.is_some() && location.worktree_available == Some(true);
    let enabled = location_enabled(location, has_project);
    let location_menu = location_menu(owner, location, has_project, t, window);
    let mode = location.mode;
    let extra_chip =
        |picker: Picker, glyph: &'static str, label: String, window: &mut Window, cx: &mut App| {
            let id = match picker {
                Picker::Branch => "home-branch",
                _ => "home-environment",
            };
            let focus = focus::handle(id, 0, window, cx);
            let key_owner = owner.clone();
            let click_owner = owner.clone();
            chip(
                id,
                glyph,
                label.into(),
                t.text_tertiary,
                compact,
                t,
                &focus,
                cx,
            )
            .role(gpui::Role::Button)
            .aria_label(match picker {
                Picker::Branch => "Starting branch",
                _ => "Environment",
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                click_owner.update(cx, |main, cx| main.toggle_picker(picker, cx))
            })
            .on_key_down(move |event, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    key_owner.update(cx, |main, cx| main.toggle_picker(picker, cx));
                    cx.stop_propagation();
                }
            })
            .child(picker_bounds(owner, picker))
        };
    let branch_chip = (mode != super::hotkey::Mode::Local).then(|| {
        let branch = location.branch.clone().unwrap_or_else(|| "HEAD".into());
        extra_chip(Picker::Branch, "settings-git", branch, window, cx)
    });
    let environment_chip = match mode {
        super::hotkey::Mode::Worktree => Some(
            location
                .environment
                .as_ref()
                .and_then(|path| location.environments.iter().find(|(_, p)| p == path))
                .map_or_else(
                    || "Work without environment".to_owned(),
                    |(name, _)| name.clone(),
                ),
        ),
        super::hotkey::Mode::Cloud => Some(
            location
                .cloud_environment
                .as_ref()
                .and_then(|id| location.cloud_environments.iter().find(|(_, i)| i == id))
                .map_or_else(
                    || "Select environment".to_owned(),
                    |(label, _)| label.clone(),
                ),
        ),
        super::hotkey::Mode::Local => None,
    }
    .map(|label| {
        extra_chip(
            Picker::Environment,
            "sidebar-cmd-environments",
            label,
            window,
            cx,
        )
    });
    let finished = animation_finished(window, cx);
    div().mx(px(13.)).pb(px(20.)).mb(px(-24.)).child(
        div()
            .px(px(6.))
            .pb(px(4.))
            .relative()
            // `_item`'s round `--radius-2xl` top corners, under the `_attached_1hrlq` rail's
            // `overflow: clip` squircle (`--codex-corner-shape`), whose box runs 20 px lower.
            .child(crate::rounded_clip(
                px(20.),
                shapes::Corner::Squircle,
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom(px(-20.))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom(px(20.))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners {
                                    top_left: px(20.),
                                    top_right: px(20.),
                                    ..Corners::all(px(0.))
                                },
                                fill: Some(t.composer_action_bar),
                                ..Default::default()
                            })),
                    ),
            ))
            // The mask covers the viewport, not the translated scroll contents.
            .child(
                div().py(px(4.)).child(
                    div()
                        .mx(px(-2.))
                        .relative()
                        .saved_layer()
                        .child(
                            div()
                                .id("home-utility-scroll")
                                .p(px(2.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .overflow_x_scroll()
                                .track_scroll(&scroll)
                                .child(project_chip(
                                    owner,
                                    project,
                                    project_menu,
                                    compact,
                                    t.text_tertiary,
                                    "Select a project to run your chat in",
                                    Some("⌥⇧⌘O"),
                                    t,
                                    &project_focus,
                                    cx,
                                ))
                                .child({
                                    let toggle = {
                                        let owner = owner.clone();
                                        move |cx: &mut App| set_location(&owner, None, cx)
                                    };
                                    let key_owner = owner.clone();
                                    chip(
                                        "work-in",
                                        match mode {
                                            super::hotkey::Mode::Worktree => "thread-fork",
                                            super::hotkey::Mode::Cloud => "settings-codex-cloud",
                                            super::hotkey::Mode::Local => "laptop",
                                        },
                                        match mode {
                                            super::hotkey::Mode::Worktree => "New worktree",
                                            super::hotkey::Mode::Cloud => "Cloud",
                                            super::hotkey::Mode::Local => "This computer",
                                        }
                                        .into(),
                                        t.text,
                                        compact,
                                        t,
                                        &work_focus,
                                        cx,
                                    )
                                    .role(gpui::Role::Button)
                                    .aria_label("Work in")
                                    // Ghost `[data-state=open]` fill, as on the composer's menus.
                                    .when(location.open, |d| d.bg(t.ghost_hover).rounded_full())
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click({
                                        let toggle = toggle.clone();
                                        let focus = work_focus.read(cx).handle.clone();
                                        let key_owner_click = owner.clone();
                                        move |_, window, cx| {
                                            window.focus(&focus, cx);
                                            toggle(cx);
                                            // Radix focuses the content: a pointer open
                                            // highlights no row.
                                            key_owner_click.update(cx, |main, _| {
                                                main.location.selected = usize::MAX
                                            });
                                        }
                                    })
                                    .on_key_down(move |event, _, cx| {
                                        let key = event.keystroke.key.as_str();
                                        let menu = &key_owner.read(cx).location;
                                        match key {
                                            "enter" | "space" if menu.open => {
                                                if menu.selected != usize::MAX {
                                                    pick_location(&key_owner, menu.selected, cx);
                                                }
                                            }
                                            "enter" | "space" => toggle(cx),
                                            "escape" => set_location(&key_owner, Some(false), cx),
                                            "up" | "down" if menu.open => {
                                                let step = if key == "up" { -1 } else { 1 };
                                                key_owner.update(cx, |main, cx| {
                                                    main.location.selected = step_enabled(
                                                        enabled,
                                                        main.location.selected,
                                                        step,
                                                    );
                                                    cx.notify();
                                                });
                                            }
                                            _ => return,
                                        }
                                        cx.stop_propagation();
                                    })
                                    .child({
                                        let owner = owner.clone();
                                        canvas(
                                            move |b, _, cx| {
                                                owner.update(cx, |main, _| main.location.bounds = b)
                                            },
                                            |_, (), _, _| {},
                                        )
                                        .absolute()
                                        .size_full()
                                    })
                                    .children(location_menu)
                                })
                                .children(branch_chip)
                                .children(environment_chip)
                                .children(picker),
                        )
                        .child(shapes::x_fade_with(move || {
                            if finished {
                                (px(16.), px(0.))
                            } else {
                                scroll_fades(scroll.offset().x, scroll.max_offset().x)
                            }
                        })),
                ),
            ),
    )
}

/// The utility bar's `Choose project` chip; it opens the project menu.
#[allow(clippy::too_many_arguments)]
fn project_chip(
    owner: &Entity<HomeMain>,
    project: Option<SharedString>,
    project_menu: Option<gpui::AnyElement>,
    compact: bool,
    fg: Hsla,
    tip: &'static str,
    shortcut: Option<&'static str>,
    t: &Theme,
    focus: &Entity<focus::ControlFocus>,
    cx: &App,
) -> crate::tooltip::Tooltip {
    let toggle = {
        let owner = owner.clone();
        move |window: &mut Window, cx: &mut App| set_project_menu(&owner, None, window, cx)
    };
    let key_toggle = toggle.clone();
    let bounds_owner = owner.clone();
    let open = project_menu.is_some();
    let chip = chip(
        "choose-project",
        "folder",
        project.unwrap_or_else(|| "Choose project".into()),
        fg,
        compact,
        t,
        focus,
        cx,
    )
    // Ghost `[data-state=open]` fill.
    .when(open, |d| d.bg(t.ghost_hover).rounded_full())
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .on_click(move |_, window, cx| toggle(window, cx))
    .on_key_down(move |event, window, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            key_toggle(window, cx);
            cx.stop_propagation();
        }
    })
    .child(
        canvas(
            move |b, _, cx| bounds_owner.update(cx, |main, _| main.projects.bounds = b),
            |_, (), _, _| {},
        )
        .absolute()
        .size_full(),
    )
    .children(project_menu);
    crate::tooltip::tooltip(chip, tip, shortcut, crate::tooltip::Side::Top)
        .focus_handle(&focus.read(cx).handle)
}

/// The Work home's `Composer utility bar`, attached under the card (`absolute inset-x-0
/// top-full`, `data-composer-rail-placement=below`): `Choose project`, `Files` and `Plugins`,
/// and the run-location button at the end.
#[allow(clippy::too_many_arguments)]
fn work_utility_bar(
    owner: &Entity<HomeMain>,
    project: Option<SharedString>,
    project_menu: Option<gpui::AnyElement>,
    work_menus: [Option<gpui::AnyElement>; 3],
    compact: bool,
    t: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let focus = focus::handle("choose-project", 0, window, cx);
    let (pills, run) = work::rail_controls(owner, work_menus, compact, t, window, cx);
    // `padding-top: --radius-2xl` and `margin-top: -(tuck + --radius-2xl)` tuck 4 px of
    // the rail under the card.
    div()
        .absolute()
        .left_0()
        .right_0()
        .top(relative(1.))
        .mx(px(13.))
        .pt(px(20.))
        .mt(px(-24.))
        .child(
            div()
                .px(px(4.))
                .pt(px(4.))
                .relative()
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners {
                        bottom_left: px(20.),
                        bottom_right: px(20.),
                        ..Corners::all(px(0.))
                    },
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.composer_action_bar),
                    ..Default::default()
                }))
                .child(
                    div().py(px(4.)).child(
                        div()
                            .py(px(2.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(project_chip(
                                        owner,
                                        project,
                                        project_menu,
                                        compact,
                                        t.text,
                                        "Change project",
                                        None,
                                        t,
                                        &focus,
                                        cx,
                                    ))
                                    .children(pills),
                            )
                            .child(run),
                    ),
                ),
        )
}

impl Render for HomeMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        if self.account_error {
            return account_error(&t, window, cx).into_any_element();
        }
        let hero_focus = focus::handle("home-project-title", 0, window, cx);
        let owner = cx.entity();
        // `ZLt`: the Chat and Work homes show only the `heading-xl` headline.
        let hero = if let Some(headline) = self.headline {
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(text(headline, 28., 33.6, 400., t.text).line_height(relative(1.2)))
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(24.))
                // `opacity-30` on the logo's box.
                .child(icon("codex-logo", 56., t.text).opacity(0.3))
                .child(
                    div()
                        .id("home-project-title")
                        .when(self.project.is_some(), |title| {
                            focus::ring(title, &hero_focus, px(4.), px(2.), cx)
                                .role(gpui::Role::Button)
                                .aria_label("Choose project")
                                .on_key_down(open_folder_key)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenFolder), cx)
                                })
                        })
                        .child(
                            text(
                                self.project.as_ref().map_or_else(
                                    || "What should we build?".to_owned(),
                                    |project| format!("What should we build in {project}?"),
                                ),
                                28.,
                                33.6,
                                400.,
                                t.text,
                            )
                            // `.heading-xl`: a unitless `line-height: 1.2`.
                            .line_height(relative(1.2)),
                        ),
                )
        };
        let chat = self.headline.is_some();
        let compact = self.composer.read(cx).utility_compact();
        let mut project_menu = project_menu(
            &cx.entity(),
            &self.projects,
            self.composer.read(cx),
            &t,
            window,
            cx,
        );
        let work_rail = if self.headline == Some(WORK_HEADLINE) {
            let menu = project_menu.take();
            let work_menus = self.work_menus(&t, window, cx);
            Some(work_utility_bar(
                &owner,
                self.project.clone(),
                menu,
                work_menus,
                compact,
                &t,
                window,
                cx,
            ))
        } else {
            None
        };
        let column = div()
            .w_full()
            // The ChatGPT composer is 640 px wide.
            .max_w(px(if chat { 672. } else { 768. }))
            .px(px(16.))
            .flex()
            .flex_col()
            .when(!chat, |column| column.pt(px(8.)).child(div().h(px(8.))))
            .when(self.beacon || self.banner.is_some(), |column| {
                column.child(self.beacon(&t, window, cx))
            })
            .when(self.windows_sandbox_setup_needed, |column| {
                column.child(self.windows_sandbox_banner(&t, window, cx))
            })
            // The Chat and Work homes have no `Work in` row above the composer.
            .when(!chat, |column| {
                let picker = picker_menu(self, &owner, &t, window);
                column.child(utility_bar(
                    &owner,
                    &self.location,
                    self.project.clone(),
                    picker,
                    project_menu,
                    compact,
                    &t,
                    window,
                    cx,
                ))
            })
            // The Work rail comes first so the card paints over its tucked top.
            .map(|column| match work_rail {
                Some(rail) => column.child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .child(rail)
                        .child(self.composer.clone()),
                ),
                None => column.child(self.composer.clone()),
            })
            .when(
                self.suggestions_loading || !self.suggestions.is_empty(),
                |column| column.child(self.suggestions(&t, window, cx)),
            );
        if chat {
            // `_Hero`: `min-block-size: var(--home-composer-top)`, `max(0px, 42svh - 44px)`
            // (the toolbar inset), `items-end`, 24 px above and 22 px below the headline; the
            // composer starts at its bottom edge.
            let top = (window.viewport_size().height * 0.42 - px(44.)).max(px(0.));
            let page = div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .w_full()
                        .flex_none()
                        .min_h(top)
                        .pt(px(24.))
                        .pb(px(22.))
                        .flex()
                        .items_end()
                        .justify_center()
                        .child(hero.w_full().max_w(px(672.)).px(px(20.))),
                )
                .child(column);
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_none()
                        .h(px(46.))
                        .child(self.chat_work_toggle(&t, window, cx)),
                )
                .child(crate::composer::drop_target(page, &self.composer, cx))
                .into_any_element();
        }
        // The page column is the file drop target.
        let page = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pt(px(24.))
            .child(
                div()
                    .flex_grow(1.)
                    .flex_basis(px(0.))
                    .flex()
                    .items_end()
                    .justify_center()
                    .pb(px(96.))
                    // The original's chain: `flex w-full justify-center`, `mx-auto flex
                    // w-[min(100%,768px)] justify-center px-panel`, `flex min-h-28 w-full items-end
                    // justify-center`, then the hero `flex w-full flex-col items-center`. The outer
                    // `justify-center` has no effect: the child's auto margins take the free space
                    // first (taffy would apply both).
                    .child(
                        div().w_full().flex().child(
                            div()
                                .mx_auto()
                                .flex()
                                .w_full()
                                .max_w(px(768.))
                                .min_w_0()
                                .justify_center()
                                .px(px(20.))
                                .child(
                                    div()
                                        .flex()
                                        .min_h(px(112.))
                                        .w_full()
                                        .items_end()
                                        .justify_center()
                                        .child(hero.w_full()),
                                ),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_grow(1.)
                    .flex_shrink_0()
                    .flex_basis(px(0.))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(
                        div()
                            .pt(px(6.))
                            .pb(px(16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(column),
                    ),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(crate::composer::drop_target(page, &self.composer, cx))
            .into_any_element()
    }
}

/// `home.accountAccessError.*`: the account lookup failed, so no Home mode is available.
fn account_error(t: &Theme, window: &mut Window, cx: &mut App) -> Div {
    use super::recovery::{button, color};
    let primary = (t.text, t.text.opacity(0.8), color(0x00000014), t.surface);
    let secondary = (
        gpui::transparent_black(),
        t.surface_secondary,
        t.border,
        t.text,
    );
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .p(px(24.))
        .text_center()
        .child(text(
            "We couldn’t load your account",
            28.,
            33.6,
            400.,
            t.text,
        ))
        .child(
            div()
                .id("account-error-description")
                .role(gpui::Role::Alert)
                .child(text(
                    "Try reloading, or sign out and sign in again",
                    14.,
                    21.,
                    400.,
                    t.text_secondary,
                )),
        )
        .child(
            div()
                .flex()
                .gap(px(12.))
                .pt(px(4.))
                .child(button(
                    "account-reload",
                    "Reload",
                    &ReloadAccount,
                    primary,
                    window,
                    cx,
                ))
                .child(button(
                    "account-sign-out",
                    "Sign out",
                    &SignOutAccount,
                    secondary,
                    window,
                    cx,
                )),
        )
}
