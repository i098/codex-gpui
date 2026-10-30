//! `home` parity screen: hero, composer utility bar and the Home composer, laid out as
//! refs/home (CSS px).

use super::composer::fixture_composer;
use crate::composer::{Composer, label, pill};
use crate::{Shell, focus, icon::icon, rounded_clip, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Bounds, Context, Corners, Div, Entity, Global,
    Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels,
    Render, ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window,
    actions, anchored, canvas, deferred, div, point, px, relative,
};
use theme::{ActiveTheme as _, Theme};

static PROJECT_TOOLTIP_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| textsys::letter_spacing(-0.15));
static CONTROL_TEXT_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(Default::default);

/// Identifies the window whose scroll animation a static capture finishes.
#[derive(Clone, Copy)]
pub struct FinishedScrollAnimation(pub AnyWindowHandle);

impl Global for FinishedScrollAnimation {}

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
    /// The announcement slot's `Try ChatGPT Voice` beacon is shown. BOUNDARY: the original
    /// shows it when `/beacons/home` offers it; the port has no such feed yet, so it starts
    /// hidden (flows.md Home rows 5 and 6) and dismissal lasts for the session only.
    pub beacon: bool,
    /// The eligible account banner, shown in the beacon's slot in its place.
    pub banner: Option<HomeBanner>,
    /// `composer/items` suggestions (at most 3), and whether they are still loading.
    pub suggestions: Vec<Suggestion>,
    pub suggestions_loading: bool,
    /// The ChatGPT or Work headline; `None` shows the Codex hero.
    pub headline: Option<&'static str>,
    pub picker: Option<Picker>,
    /// Where each [`Picker`] trigger was last laid out.
    picker_bounds: [Bounds<Pixels>; 2],
    /// `account/read` failed: Home shows `We couldn’t load your account` instead.
    pub account_error: bool,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = fixture_composer(window, cx, Composer::home);
        Self::with_composer(composer, None)
    }

    /// Home around an existing `composer`, titled with `project`.
    pub fn with_composer(composer: Entity<Composer>, project: Option<SharedString>) -> Self {
        Self {
            composer,
            project,
            location: LocationMenu::default(),
            beacon: false,
            banner: None,
            suggestions: Vec::new(),
            suggestions_loading: false,
            headline: None,
            picker: None,
            picker_bounds: Default::default(),
            account_error: false,
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
        let (headline, placeholder) = match crate::screens::sidebar::MODES[mode].0 {
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
        self.composer
            .update(cx, |composer, cx| composer.set_placeholder(placeholder, cx));
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
}

/// The Home main panel (no shell).
pub fn home_main(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| HomeMain::new(window, cx)).into()
}

/// The Home sidebar and `main` in the shell.
pub fn shell(main: AnyView, cx: &mut App) -> AnyView {
    let sidebar = super::sidebar::home_sidebar(cx, None);
    cx.new(|_| Shell::new(sidebar, main)).into()
}

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let main = home_main(window, cx);
    shell(main, cx)
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
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFolder), cx))
            .on_key_down(open_folder_key)
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
    let enabled = location_enabled(location, has_project);
    let rows = ["This computer", "New worktree", "Remote", "Cloud"]
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let owner = owner.clone();
            let action: RowAction = enabled[i].then(|| {
                std::rc::Rc::new(move |_: &mut Window, cx: &mut App| pick_location(&owner, i, cx))
                    as _
            });
            let checked = LOCATION_MODES[i] == Some(location.mode);
            menu_row(
                ("work-in-option", i),
                (label.into(), None, checked, action),
                i == location.selected,
                t,
            )
        });
    let dismiss = owner.clone();
    let popup = crate::composer::popover(t, 260., |d| d.children(rows))
        .id("work-in-menu")
        .role(gpui::Role::Menu)
        .aria_label("Work in");
    Some(floating(
        popup,
        location.bounds,
        move |cx| set_location(&dismiss, Some(false), cx),
        window,
    ))
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
    let finished = cx
        .try_global::<FinishedScrollAnimation>()
        .is_some_and(|state| state.0 == window.window_handle());
    div().mx(px(13.)).pb(px(20.)).mb(px(-24.)).child(
        div()
            .px(px(4.))
            .pb(px(4.))
            .relative()
            // `_attached_1hrlq`: `--radius-2xl` top corners, `--codex-corner-shape`.
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners {
                    top_left: px(20.),
                    top_right: px(20.),
                    ..Corners::all(px(0.))
                },
                corner: shapes::Corner::Squircle,
                fill: Some(t.composer_action_bar),
                ..Default::default()
            }))
            // The mask covers the viewport, not the translated scroll contents.
            .child(
                div().py(px(3.)).child(
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
                                .child(
                                    crate::tooltip::tooltip(
                                        chip(
                                            "choose-project",
                                            "folder",
                                            project.unwrap_or_else(|| "Choose project".into()),
                                            t.text_tertiary,
                                            compact,
                                            t,
                                            &project_focus,
                                            cx,
                                        ),
                                        "Select a project to run your chat in",
                                        Some("⌥⇧⌘O"),
                                        crate::tooltip::Side::Top,
                                    )
                                    .focus_handle(&project_focus.read(cx).handle),
                                )
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
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click({
                                        let toggle = toggle.clone();
                                        let focus = work_focus.read(cx).handle.clone();
                                        move |_, window, cx| {
                                            window.focus(&focus, cx);
                                            toggle(cx)
                                        }
                                    })
                                    .on_key_down(move |event, _, cx| {
                                        let key = event.keystroke.key.as_str();
                                        let menu = &key_owner.read(cx).location;
                                        match key {
                                            "enter" | "space" if menu.open => {
                                                pick_location(&key_owner, menu.selected, cx);
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
        let column = div()
            .w_full()
            // The ChatGPT composer is 640 px wide.
            .max_w(px(if chat { 672. } else { 768. }))
            .px(px(16.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .child(div().h(px(8.)))
            .when(self.beacon || self.banner.is_some(), |column| {
                column.child(self.beacon(&t, window, cx))
            })
            // The Chat and Work homes have no `Work in` row above the composer.
            .when(!chat, |column| {
                let picker = picker_menu(self, &owner, &t, window);
                column.child(utility_bar(
                    &owner,
                    &self.location,
                    self.project.clone(),
                    picker,
                    self.composer.read(cx).utility_compact(),
                    &t,
                    window,
                    cx,
                ))
            })
            .child(self.composer.clone())
            .when(
                self.suggestions_loading || !self.suggestions.is_empty(),
                |column| column.child(self.suggestions(&t, window, cx)),
            );
        let toolbar = div().flex_none().h(px(46.));
        if chat {
            // The headline sits right above the composer, the pair a little above the middle
            // (headline top at 296 px of the main panel's 788 at 1280×832).
            let page = div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(24.))
                .child(div().flex_grow(2.))
                .child(hero.w_full().px(px(20.)))
                .child(div().h(px(14.)))
                .child(column)
                .child(div().flex_grow(3.));
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(toolbar.child(self.chat_work_toggle(&t, window, cx)))
                .child(crate::composer::drop_target(page, &self.composer, cx))
                .into_any_element();
        }
        // The page column under the `home-mode-toggle` row is the file drop target.
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
            // `home-mode-toggle` toolbar row.
            .child(toolbar)
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
