use super::*;

/// `Fork chat from here` under a turn: a new chat with the thread up to that turn (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct ForkThread(pub String);

/// A file change card's `Undo`: revert the thread to before that turn (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct RevertTurn(pub String);

/// Copy, Read aloud, Rate response, Fork chat from here: shown under the latest turn; earlier
/// turns (group `group`) show them on hover.
/// ponytail: Read aloud (ChatGPT voice playback) and Rate response (feedback menu) have no
/// handler; wire them with a voice backend and the feedback menu.
pub(super) fn action_row(
    last: bool,
    group: &SharedString,
    turn_id: &str,
    answer: &str,
    t: &Theme,
) -> Div {
    let action = |name: &'static str| {
        let color = t.text_tertiary;
        // `Icon` folds its own opacity into the color, so the hover sets the color.
        let icon = if last {
            icon(name, 16., color)
        } else {
            icon(name, 16., color.opacity(0.))
                .group_hover(group.clone(), move |s| s.text_color(color))
        };
        div()
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(Theme::RADIUS_MD)
            .child(icon)
    };
    let (answer, turn_id) = (answer.to_owned(), turn_id.to_owned());
    div()
        .mt(px(6.))
        .ml(px(-4.))
        .h(px(26.))
        .flex()
        .gap(px(2.))
        .child(
            action("thread-copy")
                .id("copy-response")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(answer.clone()))
                }),
        )
        .child(action("thread-read-aloud"))
        .child(action("thread-rate"))
        .child(
            action("thread-fork")
                .id("fork-thread")
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(ForkThread(turn_id.clone())), cx)
                }),
        )
}
