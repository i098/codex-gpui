mod backdrop;
mod noise;

pub(super) use backdrop::Backdrop;

use super::{Action, Onboarding};
use crate::{MONO, focus, icon::icon, text};
use gpui::{
    Context, Div, InteractiveElement, KeyDownEvent, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, point, prelude::FluentBuilder as _, px, relative, rgb,
};
use theme::{ActiveTheme as _, Theme};

const TOP_CODE: &[&[(&str, u32)]] = &[
    &[
        ("import", 0xf8a6c8),
        (" mongoose, { ", 0),
        ("Schema", 0xb897f4),
        (" } ", 0),
        ("from", 0xf8a6c8),
        (" ", 0),
        ("\"mongoose\"", 0x83d197),
        (";", 0),
    ],
    &[
        ("export", 0xf8a6c8),
        (" ", 0),
        ("const", 0xf8a6c8),
        (" collection = ", 0),
        ("\"Product\"", 0x83d197),
        (";", 0),
    ],
];
const BOTTOM_CODE: &[&[(&str, u32)]] = &[
    &[
        ("const", 0xf8a6c8),
        (" schema = ", 0),
        ("new", 0xf8a6c8),
        (" ", 0),
        ("Schema", 0xb897f4),
        ("(", 0),
    ],
    &[("  {", 0)],
    &[("    ", 0), ("name", 0xf9dc78), (": {", 0)],
    &[
        ("      ", 0),
        ("type", 0xf9dc78),
        (": ", 0),
        ("String", 0xb897f4),
        (",", 0),
    ],
    &[
        ("      ", 0),
        ("required", 0xf9dc78),
        (": ", 0),
        ("true", 0xf1a275),
        (",", 0),
    ],
    &[
        ("      ", 0),
        ("trim", 0xf9dc78),
        (": ", 0),
        ("true", 0xf1a275),
        (",", 0),
    ],
    &[("    },", 0)],
    &[("    ", 0), ("description", 0xf9dc78), (": {", 0)],
];

fn code_block(lines: &'static [&'static [(&'static str, u32)]], t: &Theme) -> Div {
    let radius = gpui::Corners::all(px(20.));
    div()
        .relative()
        .w_full()
        .px(px(21.))
        .py(px(17.))
        .flex()
        .flex_col()
        .child(shapes::decoration(shapes::Decoration {
            radius,
            fill: Some(t.surface),
            ..Default::default()
        }))
        .child(shapes::decoration(shapes::Decoration {
            radius,
            fill: Some(t.text.opacity(0.08)),
            border: Some((px(1.), t.border_subtle)),
            ..Default::default()
        }))
        .children(lines.iter().map(|line| {
            div()
                .h(px(20.))
                .flex()
                .children(line.iter().map(|&(chunk, color)| {
                    text(
                        chunk,
                        12.,
                        20.,
                        430.,
                        if color == 0 {
                            t.text
                        } else {
                            rgb(color).into()
                        },
                    )
                    .font_family(MONO)
                    .whitespace_nowrap()
                }))
        }))
}

fn illustration(index: usize, t: &Theme) -> Div {
    let code = index == 2;
    let mut card = div()
        .relative()
        .w_full()
        .p(px(match index {
            0 => 9.,
            1 => 17.,
            _ => 8.,
        }))
        .flex()
        .flex_col()
        .when(!code, |card| card.gap(px(16.)))
        .child(shapes::decoration(shapes::Decoration {
            radius: gpui::Corners::all(px(if code { 15. } else { 20. })),
            fill: Some(t.surface_elevated_secondary),
            border: (!code).then_some((px(1.), t.border)),
            shadows: vec![shapes::Shadow {
                color: gpui::black().opacity(if code { 0.12 } else { 0.19 }),
                offset: point(px(0.), px(if code { 8. } else { 16. })),
                blur: px(if code { 16. } else { 32. }),
                spread: px(if code { -4. } else { -8. }),
                inset: false,
            }],
            ..Default::default()
        }));
    match index {
        0 => card
            .child(
                text(
                    "Ask Codex to do anything",
                    16.,
                    24.,
                    430.,
                    t.text.opacity(0.498),
                )
                .pl(px(8.))
                .pt(px(8.))
                .opacity(0.4),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .size(px(28.))
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: gpui::Corners::all(Theme::RADIUS_FULL),
                                corner: shapes::Corner::Round,
                                border: Some((px(1.), t.border)),
                                ..Default::default()
                            }))
                            .child(icon("plus", 18., t.text)),
                    )
                    .child(
                        div()
                            .size(px(32.))
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: gpui::Corners::all(Theme::RADIUS_FULL),
                                corner: shapes::Corner::Round,
                                fill: Some(t.button_primary_solid),
                                ..Default::default()
                            }))
                            .child(icon("composer-arrow-up", 20., t.surface_elevated_secondary)),
                    ),
            ),
        1 => {
            card = card
                .items_center()
                .child(icon("first-run-cloud", 32., t.text));
            for (index, (title, subtitle, added, removed)) in [
                (
                    "Explain repository to a new designer",
                    "openai/agi · Oct 12",
                    "",
                    "",
                ),
                ("Fix an onboarding bug", "openai/agi · Oct 9", "+2", "-20"),
                (
                    "Create a darkmode theme",
                    "openai/codex · Oct 8",
                    "+249",
                    "-123",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                card = card.child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .child(icon(
                            if index == 0 {
                                "first-run-spinner"
                            } else {
                                "first-run-check"
                            },
                            16.,
                            t.text,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(text(title, 13., 18.5714, 500., t.text))
                                .child(text(subtitle, 13., 18.5714, 500., t.text.opacity(0.5))),
                        )
                        .when(index != 0, |row| {
                            row.child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(text(added, 13., 18.5714, 500., rgb(0x00a240).into()))
                                    .child(text(removed, 13., 18.5714, 500., rgb(0xe02e2a).into())),
                            )
                        }),
                );
            }
            card
        }
        _ => card
            .child(code_block(TOP_CODE, t))
            .child(
                div().h(px(42.)).px(px(16.)).flex().items_center().child(
                    text(
                        "// TODO: IMPLEMENT SCHEMA",
                        12.,
                        16.,
                        430.,
                        t.text.opacity(0.498),
                    )
                    .font_family(MONO)
                    .font_features(textsys::letter_spacing(2.4)),
                ),
            )
            .child(code_block(BOTTOM_CODE, t)),
    }
}

impl Onboarding {
    fn legal_link(
        &self,
        id: &'static str,
        label: &'static str,
        url: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let handle = focus::handle(id, 0, window, cx);
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Link)
                .cursor_pointer()
                .on_click(move |_, _, cx| cx.open_url(url))
                .on_key_down(move |event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "enter" {
                        cx.open_url(url);
                        cx.stop_propagation();
                    }
                })
                .child(text(label, 14., 21., 430., cx.theme().text.opacity(0.498)).underline()),
            &handle,
            px(2.),
            px(2.),
            cx,
        )
    }

    fn legal_copy(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let description = t.text.opacity(0.498);
        let docs = self.legal_link(
            "first-run-docs",
            "Codex docs",
            "https://developers.openai.com/codex/ide",
            window,
            cx,
        );
        let preferences = self.legal_link(
            "first-run-data-controls",
            "training data preferences",
            "https://chatgpt.com/#settings/DataControls",
            window,
            cx,
        );
        let copy = [
            div()
                .flex()
                .flex_wrap()
                .child(text(
                    "For more details, see the ",
                    14.,
                    21.,
                    430.,
                    description,
                ))
                .child(docs),
            div().child(
                text(
                    "Review the code it writes and commands it runs",
                    14.,
                    21.,
                    430.,
                    description,
                )
                .whitespace_normal(),
            ),
            div()
                .flex()
                .flex_wrap()
                .child(text(
                    "Uses your plan’s rate limits and ",
                    14.,
                    21.,
                    430.,
                    description,
                ))
                .child(preferences),
        ];
        let rows = [
            (
                "first-run-autonomy",
                "Decide how much autonomy you want to grant",
            ),
            ("first-run-info", "Codex can make mistakes"),
            ("first-run-account", "Powered by your ChatGPT account"),
        ];
        div()
            .pt(px(12.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(
                rows.into_iter()
                    .zip(copy)
                    .map(|((asset, title), description)| {
                        div()
                            .flex()
                            .items_start()
                            .gap(px(12.))
                            .child(
                                div()
                                    .mt(px(2.))
                                    .child(icon(asset, 20., t.text.opacity(0.8))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(text(title, 13., 18.5714, 500., t.text))
                                    .child(description),
                            )
                    }),
            )
    }

    pub(super) fn first_run_view(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let backdrop = self
            .backdrop
            .as_mut()
            .map(|backdrop| backdrop.render(window, cx));
        let width = (f32::from(window.viewport_size().width) - 88.).clamp(0., 512.);
        let titles = [
            "Codex in your IDE",
            "Hand off to Codex in the cloud",
            "Turn TODOs into Codex tasks",
        ];
        let descriptions = [
            "Codex navigates, edits, runs commands, and executes tests directly in your repo. Powered by your ChatGPT account.",
            "Send tasks to Codex to run in the background so you can stay focused and move faster.",
            "Write a TODO comment and convert it into a Codex task with a single click.",
        ];
        let copy = if index < 3 {
            text(descriptions[index], 14., 21., 430., t.text.opacity(0.498))
                .w(px(384.))
                .max_w_full()
                .whitespace_normal()
                .text_center()
        } else {
            self.legal_copy(window, cx)
        };
        let footer = div()
            .absolute()
            .bottom(px(40.))
            .left(relative(0.5))
            .ml(px(-width / 2.))
            .w(px(width))
            .px(px(24.))
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .mb(px(8.))
                    .flex()
                    .justify_center()
                    .when(index < 3, |row| {
                        row.child(
                            text(titles[index], 14., 17.5, 500., t.text)
                                .w(px(384.))
                                .max_w_full()
                                .text_center(),
                        )
                    }),
            )
            .child(
                div()
                    .w_full()
                    .px(px(8.))
                    .flex()
                    .justify_center()
                    .child(copy),
            )
            .child(
                div()
                    .w_full()
                    .mt(px(40.))
                    .px(px(8.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w(px(400.))
                            .max_w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .when(self.chatgpt, |row| {
                                row.child(self.button(
                                    "first-run-back",
                                    "Back",
                                    Action::Back,
                                    false,
                                    window,
                                    cx,
                                ))
                            })
                            .child(self.button(
                                "first-run-next",
                                if self.chatgpt { "Next" } else { "Continue" },
                                Action::Continue,
                                true,
                                window,
                                cx,
                            )),
                    ),
            );
        div()
            .relative()
            .size_full()
            .px(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .children(backdrop)
            .when(
                index < 3 && f32::from(window.viewport_size().height) > 500.,
                |page| {
                    let wide = f32::from(window.viewport_size().width) > 560.;
                    page.child(
                        div()
                            .w(px(if wide { 560. } else { 320. }))
                            .h(px(if wide { 320. } else { 240. }))
                            .child(illustration(index, &t)),
                    )
                },
            )
            .child(footer)
    }
}
