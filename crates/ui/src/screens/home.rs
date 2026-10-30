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

actions!(home, [OpenFolder]);

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
    /// The announcement slot's `Try ChatGPT Voice` beacon is shown. BOUNDARY: the original
    /// shows it when `/beacons/home` offers it; the port has no such feed yet, so it starts
    /// hidden (flows.md Home rows 5 and 6) and dismissal lasts for the session only.
    pub beacon: bool,
    /// The ChatGPT or Work headline; `None` shows the Codex hero.
    pub headline: Option<&'static str>,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            composer: fixture_composer(window, cx, Composer::home),
            project: None,
            location: LocationMenu::default(),
            beacon: false,
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
fn utility_bar(
    owner: &Entity<HomeMain>,
    location: &LocationMenu,
    project: Option<SharedString>,
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
        let column = div()
            .w_full()
            // The ChatGPT composer is 640 px wide.
            .max_w(px(if chat { 672. } else { 768. }))
            .px(px(16.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .child(div().h(px(8.)))
            .when(self.beacon, |column| {
                column.child(self.beacon(&t, window, cx))
            })
            // The Chat and Work homes have no `Work in` row above the composer.
            .when(!chat, |column| {
                column.child(utility_bar(
                    &cx.entity(),
                    &self.location,
                    self.project.clone(),
                    self.composer.read(cx).utility_compact(),
                    &t,
                    window,
                    cx,
                ))
            })
            .child(self.composer.clone());
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
