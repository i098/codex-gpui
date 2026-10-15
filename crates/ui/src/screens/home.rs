//! `home` parity screen: hero, composer utility bar and the Home composer, laid out as
//! refs/home (CSS px).

use super::composer::fixture_composer;
use crate::composer::{Composer, label, pill};
use crate::{Shell, icon::icon, rounded_clip, text};
use gpui::{
    AnyView, App, AppContext as _, Context, Corners, Div, Entity, Hsla, IntoElement, ParentElement,
    Render, Styled, Window, div, px,
};
use theme::{ActiveTheme as _, Theme};

pub struct HomeMain {
    pub composer: Entity<Composer>,
}

impl HomeMain {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            composer: fixture_composer(window, cx, Composer::home),
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

/// `Choose project` / `This computer` chip of the utility bar. The pill is `overflow: hidden`:
/// Skia draws its icon under that AA clip (its padding box; light has no fitted images).
fn chip(id: &'static str, glyph: &str, s: &'static str, fg: Hsla, t: &Theme) -> Div {
    div().child(
        pill(id, t).child(rounded_clip(
            px(13.),
            shapes::Corner::Round,
            div()
                .h_full()
                .flex()
                .items_center()
                .px(px(8.))
                .gap(px(6.))
                .child(icon(glyph, 16., fg))
                .child(label(s, fg)),
        )),
    )
}

/// `Composer utility bar` (`_rail_1hrlq` attached above the composer): project and
/// environment.
fn utility_bar(t: &Theme) -> Div {
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
                div().py(px(3.)).child(
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
                            "Choose project",
                            t.text_tertiary,
                            t,
                        ))
                        .child(chip("work-in", "laptop", "This computer", t.text, t))
                        .child(shapes::x_fade(px(16.), px(16.))),
                ),
            ),
    )
}

impl Render for HomeMain {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(24.))
            .child(icon("codex-logo", 56., Hsla { a: 0.3, ..t.text }))
            .child(text("What should we build?", 28., 33.6, 400., t.text));
        let column = div()
            .w_full()
            .max_w(px(768.))
            .px(px(16.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .child(div().h(px(8.)))
            .child(utility_bar(&t))
            .child(self.composer.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            // `home-mode-toggle` toolbar row.
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(px(10.))
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
                    ),
            )
    }
}
