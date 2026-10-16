use super::*;

/// Right-aligned user message: `bg-user-message`, 22 px squircle, at most 70% wide.
pub(super) fn user_bubble(message: &str, t: &Theme) -> Div {
    div().flex().flex_col().items_end().pb(px(30.)).child(
        div()
            .relative()
            .max_w(relative(0.7))
            .py(px(10.))
            .px(px(16.))
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(22.)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.user_message_background),
                ..Default::default()
            }))
            .child(
                div()
                    .text_size(px(14.))
                    .line_height(px(22.75))
                    .font_weight(FontWeight(430.))
                    .text_color(white())
                    .child(message.to_owned()),
            ),
    )
}

/// A row that wraps its word boxes like an inline formatting context. Chromium lets a line's
/// trailing space hang past the wrap width, and each word box carries its trailing space.
/// ponytail: a fixed 4 px allowance (one 14 px space) instead of a hanging space.
pub(super) fn wrap_row() -> Div {
    div().flex().flex_wrap().mr(px(-4.))
}

/// `Worked for 17s ›` over a hairline (collapsed turn disclosure); `open`: the chevron points
/// down.
pub(super) fn disclosure(label: String, open: bool, window: &Window, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div().h(px(22.3)).pt(px(1.3)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(measured(
                        window,
                        label,
                        14.,
                        21.,
                        430.,
                        t.text_tertiary,
                        None,
                    ))
                    .child(icon(
                        if open {
                            "chevron-down"
                        } else {
                            "thread-chevron-right"
                        },
                        14.,
                        t.text_tertiary,
                    )),
            ),
        )
        .child(
            div()
                .pt(px(4.))
                .child(div().h(px(1.)).bg(over(t.border, t.surface))),
        )
}

/// `Edited math.py +4 -0` card with `Undo` and `View changes` for the file changes of turn
/// `turn_id`.
pub(super) fn diff_card(
    turn_id: &str,
    items: &[ThreadItem],
    window: &Window,
    t: &Theme,
) -> Option<Div> {
    let changes: Vec<_> = items
        .iter()
        .flat_map(|i| match i {
            ThreadItem::FileChange { changes, .. } => changes.as_slice(),
            _ => &[],
        })
        .collect();
    let first = changes.first()?;
    let lines = |prefix: char| {
        changes
            .iter()
            .flat_map(|c| c.diff.lines())
            .filter(|l| l.starts_with(prefix) && !l.starts_with("+++") && !l.starts_with("---"))
            .count()
    };
    let mut files: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    files.dedup();
    let title = match files.len() {
        1 => format!(
            "Edited {}",
            first.path.rsplit('/').next().unwrap_or(&first.path)
        ),
        n => format!("Edited {n} files"),
    };
    let tones = tones(t);
    let card_bg = tones.card;
    // A 1 px border box (`px-2 border`): the padding stands in for the border, which the
    // decoration paints under the label.
    let button = |label: &'static str, decoration: Option<gpui::AnyElement>| {
        div()
            .relative()
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(9.))
            .children(decoration)
            .child(measured(window, label, 13., 18., 430., t.text, None))
    };
    Some(
        div()
            .relative()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(Theme::RADIUS_LG),
                corner: shapes::Corner::Squircle,
                fill: Some(card_bg),
                // `box-shadow: 0 0 0 0.5px` ring.
                shadows: vec![shadow(t.border_strong, 0., 0., 0.5, false)],
                ..Default::default()
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(12.))
                    .child(
                        div()
                            .relative()
                            .size(px(40.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(Theme::RADIUS_LG),
                                corner: shapes::Corner::Squircle,
                                // Translucent, onto the card's fill in the scroller's layer.
                                fill: Some(Hsla {
                                    a: 0.92,
                                    ..t.surface_secondary
                                }),
                                ..Default::default()
                            }))
                            .child(icon("thread-file-diff", 24., t.text_secondary)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text(title, 14., 21., 500., t.text).text_ellipsis())
                            .child(div().h(px(19.5)).pt(px(3.)).child(
                                crate::screens::diff::panel::stats(lines('+'), lines('-'), 13., t),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(8.))
                            .child({
                                let turn_id = turn_id.to_owned();
                                button("Undo", None)
                                    .child(icon("thread-undo", 14., t.text))
                                    .id("undo-turn")
                                    .on_click(move |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(RevertTurn(turn_id.clone())),
                                            cx,
                                        )
                                    })
                            })
                            // Blink's background bleed avoidance paints the fill and the
                            // translucent border into one transparency layer (shapes' clip
                            // layer op), rasterized in the scroller's layer like the card.
                            .child(
                                button(
                                    "View changes",
                                    Some(
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .child(shapes::decoration(shapes::Decoration {
                                                radius: Corners::all(Theme::RADIUS_LG),
                                                corner: shapes::Corner::Squircle,
                                                fill: Some(tones.secondary_button),
                                                border: Some((px(1.), t.border)),
                                                ..Default::default()
                                            }))
                                            .into_any_element(),
                                    ),
                                )
                                .id("view-changes")
                                .on_click(crate::dispatch::<crate::OpenChangesPanel>),
                            ),
                    ),
            ),
    )
}

/// A turn's items without a final answer: agent messages and command rows. A running turn
/// names its running commands; otherwise commands fold into `Ran a command`.
pub(super) fn activity(
    items: &[ThreadItem],
    running: bool,
    window: &Window,
    t: &Theme,
    fades: &HashMap<String, Vec<StreamSegment>>,
) -> Div {
    let text60 = t.text_alpha_60;
    let row = |label: String| {
        div()
            .h(px(21.))
            .flex()
            .items_center()
            .gap(px(6.))
            .child(icon("thread-terminal", 16., text60))
            .child(measured(window, label, 14., 21., 430., text60, None))
    };
    let mut col = div().flex().flex_col().gap(px(16.));
    let mut ran = 0;
    let flush = |col: Div, ran: &mut usize| match std::mem::take(ran) {
        0 => col,
        1 => col.child(row("Ran a command".into())),
        _ => col.child(row("Ran commands".into())),
    };
    for item in items {
        match item {
            ThreadItem::CommandExecution {
                command, status, ..
            } => {
                if running && status == "inProgress" {
                    col = flush(col, &mut ran).child(row(command.clone()));
                } else {
                    ran += 1;
                }
            }
            ThreadItem::AgentMessage { id, text } if !text.is_empty() => {
                let segments = fades.get(id).map(Vec::as_slice).unwrap_or_default();
                col = flush(col, &mut ran).child(markdown(text, window, t, segments));
            }
            _ => {}
        }
    }
    flush(col, &mut ran)
}
