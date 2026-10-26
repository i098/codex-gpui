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
use theme::{ActiveTheme as _, Appearance};

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
        // `--color-surface-recovery`: `--app-color-background-recovery` (#f8f7f5) in light;
        // the opaque dark surface-under (black) otherwise.
        let (background, title, description, fill, hover, label, border) = match t.appearance {
            Appearance::Dark => (
                color(0x000000ff),
                color(0xdfdfdfff),
                color(0xdfdfdfa6),
                color(0xdfdfdfff),
                color(0xdfdfdfcc),
                color(0x181818ff),
                color(0xffffff14),
            ),
            Appearance::Light => (
                color(0xf8f7f5ff),
                color(0x1a1c1fff),
                color(0x1a1c1fa6),
                color(0x0d0d0dff),
                color(0x0d0d0dcc),
                color(0xffffffff),
                color(0x00000014),
            ),
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
                                // `-translate-x-2` is visual-only; a margin shifts
                                // layout, and the row re-centers half of it back.
                                // -16 nets the original's -8 visual shift.
                                .ml(px(-16.)),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .max_w(px(448.))
                            .flex_col()
                            .flex_shrink_0()
                            .gap(px(12.))
                            // The original centers these runs 0.1875 css px left of
                            // GPUI; the nudge puts the glyphs on the original's
                            // subpixel grid (measured 0.000% text diff).
                            .child(
                                text("ChatGPT hit a snag", 28., 39.2, 600., title).ml(px(-0.1875)),
                            )
                            .child(
                                text(
                                    "Something went wrong. Try again to continue",
                                    14.,
                                    21.,
                                    430.,
                                    description,
                                )
                                .ml(px(-0.1875)),
                            ),
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
                                    (fill, hover, border, label),
                                    window,
                                    cx,
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

/// The `Ir` primary button at rest, `size: large`: a 36 px pill (`9999px` radius,
/// `8px 20px` padding, 14/18 w430) — values probed from the original page.
fn button<A: Action + Default + Clone>(
    id: &'static str,
    label: &'static str,
    action: &A,
    colors: (Hsla, Hsla, Hsla, Hsla),
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let (fill, hover, border, label_color) = colors;

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
        .px(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(fill)
        .hover(|state| state.bg(hover))
        .text_color(label_color)
        .on_click(click)
        .on_key_down(key)
        .child(text(label, 14., 18., 430., label_color));
    focus::ring(button, &control, px(999.), px(2.), cx)
}

/// The startup loader (SH-147): the `index.html` `startup-loader` at rest, its 56 px
/// blossom in the shimmer base color on the window background.
#[derive(IntoElement)]
pub struct StartupLoader;

impl RenderOnce for StartupLoader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        // The `startup-loader` CSS is transparent; the native window's
        // `backgroundColor` shows through (`#202020` dark, `#ffffff` light, from the
        // original main bundle). `--startup-logo-shimmer-base` is the only visible
        // layer once the fade-in settles.
        let (window_background, blossom) = match t.appearance {
            Appearance::Dark => (color(0x202020ff), color(0xffffff94)),
            Appearance::Light => (color(0xffffffff), color(0x00000034)),
        };
        div()
            .id("startup-loader")
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(window_background)
            .child(crate::icon::icon("blossom", 56., blossom))
    }
}
