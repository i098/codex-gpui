use crate::{focus, text};
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, Window, actions, div, px,
};
use theme::{ActiveTheme as _, Theme};

actions!(browser_use, [ReturnHome]);

pub fn handoff(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.theme().clone();
    let handle = focus::handle("browser-handoff-return-home", 0, window, cx);
    let button = div()
        .id("browser-handoff-return-home")
        .role(gpui::Role::Button)
        .aria_label("Return home")
        .track_focus(&handle.read(cx).handle)
        .h(px(40.))
        .px(px(16.))
        .rounded(Theme::RADIUS_LG)
        .bg(theme.button_secondary)
        .hover(move |style| style.bg(theme.button_secondary_hover))
        .flex()
        .items_center()
        .justify_center()
        .child(text("Return home", 14., 20., 500., theme.text))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ReturnHome), cx))
        .on_key_down(|event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.dispatch_action(Box::new(ReturnHome), cx);
                cx.stop_propagation();
            }
        });
    div()
        .id("browser-handoff")
        .size_full()
        .bg(theme.surface)
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .p(px(16.))
                .flex()
                .justify_end()
                .child(focus::ring(button, &handle, Theme::RADIUS_LG, px(2.), cx)),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "This computer link is unavailable",
                    14.,
                    21.,
                    430.,
                    theme.text_secondary,
                )),
        )
        .into_any_element()
}

pub fn settings(cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(div().h(px(46.)).flex_none())
        .child(
            div()
                .id("browser-settings-page")
                .flex_1()
                .overflow_y_scroll()
                .child(
                    div().pt(px(16.)).pb(px(1.)).child(
                        div().w_full().max_w(px(768.)).mx_auto().px(px(20.)).child(
                            div()
                                .py(px(12.))
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(text("Browser", 28., 33.6, 500., theme.text))
                                .child(text(
                                    "Manage your Browser Use preferences and site access.",
                                    14.,
                                    24.,
                                    430.,
                                    theme.text_secondary,
                                )),
                        ),
                    ),
                )
                .child(div().w_full().max_w(px(768.)).mx_auto().p(px(20.)).child(
                    div().p(px(16.)).child(text(
                        "In-app browser plugin unavailable",
                        13.,
                        18.571_428,
                        430.,
                        theme.text_secondary,
                    )),
                )),
        )
        .into_any_element()
}
