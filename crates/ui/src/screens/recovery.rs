//! Full-screen error and loading states (SH-145, SH-147): the app-server crash page
//! (`uBn` in the original's app-initial) and the `index.html` startup loader.
//!
//! The route and tab error boundaries (SH-143/144) are React-tree boundaries; this
//! port has no equivalent failure mode, so only the crash page and loader exist.

use crate::{focus, text};
use gpui::Hsla;
use gpui::{
    Action, App, ClickEvent, Div, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    RenderOnce, Role, Stateful, StatefulInteractiveElement, Styled, Window, actions, div, img, px,
    rgba,
};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(
    recovery,
    [
        /// Crash page: spawn a new app-server and resume.
        TryAgain,
    ]
);

/// The crash page: the original's `uBn` error-boundary page at its defaults.
#[derive(IntoElement)]
pub struct Recovery;

impl RenderOnce for Recovery {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme().clone();
        // `--color-surface-recovery`: `--app-color-background-recovery` in light, else
        // `--color-surface-secondary` (the dark surface-under).
        let background = match t.appearance {
            Appearance::Dark => t.surface_secondary,
            Appearance::Light => color(0xf8f7f5ff),
        };
        div()
            .id("recovery")
            .flex()
            .size_full()
            .overflow_y_scroll()
            .bg(background)
            .child(
                div()
                    .mx_auto()
                    .my_auto()
                    .flex()
                    .w_full()
                    .max_w(px(896.))
                    .min_h_full()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .text_center()
                    .p(px(24.))
                    .child(
                        div().flex().flex_1().items_end().child(
                            img("ui/fallen-pet.png")
                                .size(px(160.))
                                .flex_none()
                                .ml(px(-8.)),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .max_w(px(448.))
                            .flex_col()
                            .flex_shrink_0()
                            .gap(px(4.))
                            .child(text("ChatGPT hit a snag", 28., 39.2, 600., t.text))
                            .child(text(
                                "Something went wrong. Try again to continue",
                                14.,
                                21.,
                                400.,
                                t.text_secondary,
                            )),
                    )
                    .child(
                        div().flex().flex_1().flex_col().items_center().child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(12.))
                                .pt(px(16.))
                                .child(button(
                                    "try-again",
                                    "Try again",
                                    &TryAgain,
                                    true,
                                    window,
                                    cx,
                                    &t,
                                )),
                        ),
                    ),
            )
    }
}

/// A theme fill from its sRGB bytes.
fn color(bytes: u32) -> Hsla {
    rgba(bytes).into()
}

/// An `Ir` solid button, `size: large` (36 px, 10 px radius, 12 px gutter, 14/20 w400).
fn button<A: Action + Default + Clone>(
    id: &'static str,
    label: &'static str,
    action: &A,
    primary: bool,
    window: &mut Window,
    cx: &mut App,
    t: &Theme,
) -> Stateful<Div> {
    let dark = matches!(t.appearance, Appearance::Dark);
    // `--color-background-*-solid` and its hover, gray-1000/gray-0 inverted for primary.
    let (background, hover, label_color) = if primary {
        (
            t.button_primary_solid,
            color(if dark { 0xedededff } else { 0x333333ff }),
            color(if dark { 0x0d0d0dff } else { 0xffffffff }),
        )
    } else {
        (
            color(if dark { 0x414141ff } else { 0x5d5d5dff }),
            color(if dark { 0x4f4f4fff } else { 0x414141ff }),
            t.text,
        )
    };
    let action = action.clone();
    let key_action = action.clone();
    let control = focus::handle(id, 0, window, cx);
    let click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        window.dispatch_action(Box::new(action.clone()), cx)
    };
    let key = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
        let key = &event.keystroke;
        if matches!(key.key.as_str(), "enter" | "space")
            && !key.modifiers.platform
            && !key.modifiers.control
        {
            window.dispatch_action(Box::new(key_action.clone()), cx);
            cx.stop_propagation();
        }
    };
    let button = div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .track_focus(&control.read(cx).handle)
        .h(px(36.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .bg(background)
        .hover(|state| state.bg(hover))
        .text_color(label_color)
        .on_click(click)
        .on_key_down(key)
        .child(text(label, 14., 20., 400., label_color));
    focus::ring(button, &control, px(10.), px(2.), cx)
}

/// The startup loader (SH-147): the `index.html` `startup-loader` at rest, its 56 px
/// blossom in the shimmer base color on the window background.
#[derive(IntoElement)]
pub struct StartupLoader;

impl RenderOnce for StartupLoader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        // `--startup-logo-shimmer-base`: the only visible layer once the fade-in settles.
        let blossom = match t.appearance {
            Appearance::Dark => color(0xffffffad),
            Appearance::Light => color(0x0000003d),
        };
        div()
            .id("startup-loader")
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(t.background_surface_under)
            .child(crate::icon::icon("blossom", 56., blossom))
    }
}
