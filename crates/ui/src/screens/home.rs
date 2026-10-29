//! `home` parity screen: hero, composer utility bar and the Home composer, laid out as
//! refs/home (CSS px).

use super::composer::fixture_composer;
use crate::composer::{Composer, label, pill};
use crate::{Shell, focus, icon::icon, rounded_clip, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, Corners, Div, Entity, Global, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement, Pixels, Render,
    ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window, actions,
    div, px, relative,
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

pub struct HomeMain {
    pub composer: Entity<Composer>,
    pub project: Option<SharedString>,
    /// The announcement slot's `Try ChatGPT Voice` beacon is shown. BOUNDARY: the original
    /// shows it when `/beacons/home` offers it; the port has no such feed yet, so it starts
    /// hidden (flows.md Home rows 5 and 6) and dismissal lasts for the session only.
    pub beacon: bool,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            composer: fixture_composer(window, cx, Composer::home),
            project: None,
            beacon: false,
        }
    }
}

impl HomeMain {
    pub fn set_beacon(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.beacon = visible;
        cx.notify();
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
                                .child(chip(
                                    "work-in",
                                    "laptop",
                                    "This computer".into(),
                                    t.text,
                                    compact,
                                    t,
                                    &work_focus,
                                    cx,
                                )),
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
        let hero = div()
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
            );
        let column = div()
            .w_full()
            .max_w(px(768.))
            .px(px(16.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .child(div().h(px(8.)))
            .when(self.beacon, |column| {
                column.child(self.beacon(&t, window, cx))
            })
            .child(utility_bar(
                self.project.clone(),
                self.composer.read(cx).utility_compact(),
                &t,
                window,
                cx,
            ))
            .child(self.composer.clone());
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
            .child(div().flex_none().h(px(46.)))
            .child(crate::composer::drop_target(page, &self.composer, cx))
    }
}
