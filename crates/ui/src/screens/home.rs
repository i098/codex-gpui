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

/// Identifies the window whose scroll animation a static capture finishes.
#[derive(Clone, Copy)]
pub struct FinishedScrollAnimation(pub AnyWindowHandle);

impl Global for FinishedScrollAnimation {}

actions!(
    home,
    [
        OpenFolder,
        ClearProject,
        SetupWindowsSandboxElevated,
        SetupWindowsSandboxUnelevated
    ]
);

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
    selected: usize,
    bounds: Bounds<Pixels>,
}

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
    /// Windows sandbox setup is needed and in progress.
    pub windows_sandbox_setup_needed: bool,
    pub windows_sandbox_setup_pending: bool,
    /// The ChatGPT or Work headline; `None` shows the Codex hero.
    pub headline: Option<&'static str>,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            composer: fixture_composer(window, cx, Composer::home),
            project: None,
            location: LocationMenu::default(),
            projects: ProjectMenu::new(cx),
            beacon: false,
            windows_sandbox_setup_needed: false,
            windows_sandbox_setup_pending: false,
            headline: None,
        }
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

    /// `aside` 736×74: copy on the left, `Start Voice` and the dismiss `×` on the right.
    fn beacon(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
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
                    .child(text("Try ChatGPT Voice", 14., 20., 500., t.text))
                    .child(text(
                        "Orchestrate tasks, connect tools, and explore code",
                        13.,
                        18.,
                        400.,
                        t.text_tertiary,
                    )),
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
                .aria_label("Start Voice")
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(t.composer_primary)
                .child(text("Start Voice", 13., 18., 400., t.text_composer_primary))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::StartVoiceChat), cx)
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
                .aria_label("Dismiss ChatGPT beacon banner")
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(move |s| s.bg(hover))
                .child(icon("close", 16., t.text_tertiary))
                .on_click(cx.listener(|this, _, _, cx| this.set_beacon(false, cx))),
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
        main.location.selected = usize::from(main.location.mode == super::hotkey::Mode::Worktree);
        cx.notify();
    });
}

fn pick_location(owner: &Entity<HomeMain>, index: usize, cx: &mut App) {
    owner.update(cx, |main, cx| {
        main.location.mode = if index == 1 {
            super::hotkey::Mode::Worktree
        } else {
            super::hotkey::Mode::Local
        };
        main.location.open = false;
        cx.notify();
    });
}

/// `Work in` keeps local and managed-worktree starts on the selected project.
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
    let worktree = location.mode == super::hotkey::Mode::Worktree;
    // The original's rows: `Work in` label, This computer, Cloud, Remote. The port keeps
    // its managed-worktree start under This computer while a project is selected.
    // (index into `pick_location`, label, icon, enabled, checked)
    let rows = [
        Some((0, "This computer", "laptop", true, !worktree)),
        has_project.then_some((1, "New worktree", "thread-fork", true, worktree)),
        Some((2, "Cloud", "settings-codex-cloud", false, false)),
        Some((3, "Remote", "project-marker-globe", false, false)),
    ]
    .into_iter()
    .flatten()
    .map(|(i, label, glyph, enabled, checked)| {
        let owner = owner.clone();
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

/// `Composer utility bar` (`_rail_1hrlq` attached above the composer): project and
/// environment.
#[allow(clippy::too_many_arguments)]
fn utility_bar(
    owner: &Entity<HomeMain>,
    location: &LocationMenu,
    project: Option<SharedString>,
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
    let location_menu = location_menu(owner, location, has_project, t, window);
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
                                        {
                                            let toggle = {
                                                let owner = owner.clone();
                                                move |window: &mut Window, cx: &mut App| {
                                                    set_project_menu(&owner, None, window, cx)
                                                }
                                            };
                                            let key_toggle = toggle.clone();
                                            let bounds_owner = owner.clone();
                                            chip(
                                                "choose-project",
                                                "folder",
                                                project.unwrap_or_else(|| "Choose project".into()),
                                                t.text_tertiary,
                                                compact,
                                                t,
                                                &project_focus,
                                                cx,
                                            )
                                            // Ghost `[data-state=open]` fill.
                                            .when(project_menu.is_some(), |d| {
                                                d.bg(t.ghost_hover).rounded_full()
                                            })
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(move |_, window, cx| toggle(window, cx))
                                            .on_key_down(move |event, window, cx| {
                                                if matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    key_toggle(window, cx);
                                                    cx.stop_propagation();
                                                }
                                            })
                                            .child(
                                                canvas(
                                                    move |b, _, cx| {
                                                        bounds_owner.update(cx, |main, _| {
                                                            main.projects.bounds = b
                                                        })
                                                    },
                                                    |_, (), _, _| {},
                                                )
                                                .absolute()
                                                .size_full(),
                                            )
                                            .children(project_menu)
                                        },
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
                                        if location.mode == super::hotkey::Mode::Worktree {
                                            "thread-fork"
                                        } else {
                                            "laptop"
                                        },
                                        if location.mode == super::hotkey::Mode::Worktree {
                                            "New worktree"
                                        } else {
                                            "This computer"
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
                                                key_owner.update(cx, |main, cx| {
                                                    main.location.selected = usize::from(
                                                        has_project && main.location.selected == 0,
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
                                }),
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
        let hero_focus = focus::handle("home-project-title", 0, window, cx);
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
        let project_menu = project_menu(
            &cx.entity(),
            &self.projects,
            self.composer.read(cx),
            &t,
            window,
            cx,
        );
        let column = div()
            .w_full()
            // The ChatGPT composer is 640 px wide.
            .max_w(px(if chat { 672. } else { 768. }))
            .px(px(16.))
            .flex()
            .flex_col()
            .when(!chat, |column| column.pt(px(8.)).child(div().h(px(8.))))
            .when(self.beacon, |column| {
                column.child(self.beacon(&t, window, cx))
            })
            .when(self.windows_sandbox_setup_needed, |column| {
                column.child(self.windows_sandbox_banner(&t, window, cx))
            })
            // The Chat and Work homes have no `Work in` row above the composer.
            .when(!chat, |column| {
                column.child(utility_bar(
                    &cx.entity(),
                    &self.location,
                    self.project.clone(),
                    project_menu,
                    self.composer.read(cx).utility_compact(),
                    &t,
                    window,
                    cx,
                ))
            })
            .child(self.composer.clone());
        let toolbar = div().flex_none().h(px(46.));
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
                .child(toolbar.child(self.chat_work_toggle(&t, window, cx)))
                .child(crate::composer::drop_target(page, &self.composer, cx));
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
    }
}
