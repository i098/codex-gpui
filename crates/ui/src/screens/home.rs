//! `home` parity screen: hero, composer utility bar and the Home composer, laid out as
//! refs/home (CSS px).

use super::composer::fixture_composer;
use crate::composer::{Composer, label, pill};
use crate::{Shell, focus, icon::icon, rounded_clip, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, App, AppContext as _, Context, Corners, Div, Entity, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Window, actions, canvas, div, px,
};
use theme::{ActiveTheme as _, Theme};

actions!(home, [OpenFolder]);

pub struct HomeMain {
    pub composer: Entity<Composer>,
    pub project: Option<SharedString>,
    pub utility_compact: bool,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            composer: fixture_composer(window, cx, Composer::home),
            project: None,
            utility_compact: false,
        }
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
fn chip(
    id: &'static str,
    glyph: &str,
    s: Option<SharedString>,
    fg: Hsla,
    t: &Theme,
    focus: &Entity<focus::ControlFocus>,
    cx: &App,
) -> Div {
    div().child(
        focus::shadow_ring(pill(id, t), focus, Theme::RADIUS_FULL, px(2.), cx)
            .when(id == "choose-project", |chip| {
                chip.role(gpui::Role::Button)
                    .aria_label("Choose project")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFolder), cx))
                    .on_key_down(open_folder_key)
            })
            .child(rounded_clip(
                px(13.),
                shapes::Corner::Round,
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .gap(px(6.))
                    .child(icon(glyph, 16., fg))
                    .children(s.map(|s| label(s, fg))),
            )),
    )
}

/// `Composer utility bar` (`_rail_1hrlq` attached above the composer): project and
/// environment.
fn utility_bar(
    home: &HomeMain,
    owner: Entity<HomeMain>,
    t: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let project = home.project.clone();
    let compact = home.utility_compact;
    let project_focus = focus::handle("choose-project", 0, window, cx);
    let work_focus = focus::handle("work-in", 0, window, cx);
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
            // `horizontal-scroll-fade-mask` scroller (`-m-0.5`, `p-0.5`): its contents fade over
            // 1rem at both ends (the scroll timeline is inactive) in a `saveLayer` of the page
            // layer, so they rasterize on the page's tiles.
            .child(
                div()
                    .py(px(3.))
                    .relative()
                    .child(
                        div()
                            .mx(px(-2.))
                            .p(px(2.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .saved_layer()
                            .child(chip(
                                "choose-project",
                                "folder",
                                (!compact)
                                    .then(|| project.unwrap_or_else(|| "Choose project".into())),
                                t.text_tertiary,
                                t,
                                &project_focus,
                                cx,
                            ))
                            .child(chip(
                                "work-in",
                                "laptop",
                                (!compact).then(|| "This computer".into()),
                                t.text,
                                t,
                                &work_focus,
                                cx,
                            ))
                            .child(shapes::x_fade(px(16.), px(16.))),
                    )
                    .child(
                        canvas(
                            move |bounds, window, cx| {
                                let compact = bounds.size.width <= px(475.);
                                let changed = owner.update(cx, |home, _| {
                                    std::mem::replace(&mut home.utility_compact, compact) != compact
                                });
                                if changed {
                                    let owner = owner.clone();
                                    window.defer(cx, move |_, cx| {
                                        owner.update(cx, |_, cx| cx.notify())
                                    });
                                }
                            },
                            |_, (), _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(1.)),
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
            .child(icon("codex-logo", 56., Hsla { a: 0.3, ..t.text }))
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
                    .child(text(
                        self.project.as_ref().map_or_else(
                            || "What should we build?".to_owned(),
                            |project| format!("What should we build in {project}?"),
                        ),
                        28.,
                        33.6,
                        400.,
                        t.text,
                    )),
            );
        let column = div()
            .w_full()
            .max_w(px(768.))
            .px(px(16.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .child(div().h(px(8.)))
            .child(utility_bar(self, cx.entity(), &t, window, cx))
            .child(self.composer.clone());
        // The page column under the `home-mode-toggle` row is the file drop target.
        // macOS overlay scrollbars reserve no horizontal gutter.
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
                    .child(hero),
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
