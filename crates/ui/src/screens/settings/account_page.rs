//! The frame and small controls that the ChatGPT account settings pages share: the 768 px
//! column with the 28 px title, a paragraph, a checkbox, a status line and the dialog layer.
use super::{Layer, on_card, probe};
use crate::{confirmation::Confirmation, icon::icon, text};
use gpui::{
    AnyElement, Div, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    Stateful, Styled, Window, deferred, div, prelude::FluentBuilder as _, px,
};
use theme::Theme;

/// The page: toolbar gap, `title`, the `intro` paragraphs and `content` in a scrolling column,
/// and the open `dialog` over the window.
pub(crate) fn frame(
    id: &'static str,
    title: &'static str,
    intro: Vec<AnyElement>,
    content: impl IntoElement,
    dialog: Option<&Entity<Confirmation>>,
    layer: &Layer,
    window: &Window,
    t: &Theme,
) -> AnyElement {
    let column = div().w_full().max_w(px(768.)).mx_auto().px(px(20.));
    div()
        .size_full()
        .relative()
        .flex()
        .flex_col()
        .child(div().h(px(46.)).flex_none())
        .child(
            div()
                .id(id)
                .flex_1()
                .overflow_y_scroll()
                .child(probe(layer))
                .child(
                    column
                        .pt(px(28.))
                        .pb(px(20.))
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .child(text(title, 28., 33.6, 500., t.text))
                        .children(intro)
                        .child(div().pt(px(16.)).child(content)),
                ),
        )
        .children(dialog.map(|dialog| {
            let viewport = window.viewport_size();
            deferred(
                gpui::anchored()
                    .position(gpui::point(px(0.), px(0.)))
                    .child(
                        div()
                            .relative()
                            .w(viewport.width)
                            .h(viewport.height)
                            .child(dialog.clone()),
                    ),
            )
        }))
        .into_any_element()
}

/// A wrapping 14 px secondary paragraph.
pub(crate) fn paragraph(s: impl Into<SharedString>, t: &Theme) -> AnyElement {
    text(s, 14., 21., 430., t.text_secondary)
        .whitespace_normal()
        .into_any_element()
}

/// A wrapping 13 px status line: an error in the error colour, else secondary.
pub(crate) fn status(s: impl Into<SharedString>, error: bool, t: &Theme) -> Div {
    let color = if error {
        t.text_error
    } else {
        t.text_secondary
    };
    text(s, 13., 18., 430., color).whitespace_normal()
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Primary,
    Secondary,
    /// A secondary button with the error-coloured label (Remove, Delete).
    Danger,
}

/// A pill button, findable by `id` as a debug selector. A disabled button is dimmed.
pub(crate) fn action(
    id: &'static str,
    label: impl Into<SharedString>,
    kind: Kind,
    disabled: bool,
    t: &Theme,
) -> Stateful<Div> {
    let (fill, border, color) = match kind {
        Kind::Primary => (t.button_primary_solid, None, t.text_inverse),
        Kind::Secondary => (t.button_secondary, Some(t.border), t.text),
        Kind::Danger => (t.button_secondary, Some(t.border), t.text_error),
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(28.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .relative()
        .child(on_card(14., shapes::Corner::Squircle, fill, border))
        .child(text(label, 13., 18., 500., color))
        .when(disabled, |b| b.opacity(0.5))
}

/// A 14 px checkbox with a wrapping label.
pub(crate) fn checkbox(
    id: &'static str,
    checked: bool,
    label: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .mt(px(2.))
                .size(px(14.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if checked {
                    t.switch_track_checked
                } else {
                    t.border_strong
                })
                .when(checked, |d| {
                    d.bg(t.switch_track_checked)
                        .child(icon("menu-check", 12., t.text_inverse))
                }),
        )
        .child(
            text(label, 12., 16., 430., t.text_secondary)
                .flex_1()
                .whitespace_normal(),
        )
}
