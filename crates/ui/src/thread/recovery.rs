use gpui::{Context, Div, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px};
use theme::{ActiveTheme as _, Theme};

use super::ThreadView;
use crate::text;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageLimitNotice {
    #[default]
    RetryLater,
    Upgrade,
    UpgradeOrAddCredits,
    AddCredits,
}

impl UsageLimitNotice {
    pub fn for_plan(plan: Option<&str>) -> Self {
        match plan {
            Some("free" | "go") => Self::Upgrade,
            Some("plus" | "prolite") => Self::UpgradeOrAddCredits,
            Some("pro" | "promax") => Self::AddCredits,
            _ => Self::RetryLater,
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::RetryLater => "You've hit your usage limit. Try again later.",
            Self::Upgrade => {
                "You've hit your usage limit. Upgrade your plan to continue, or try again later."
            }
            Self::UpgradeOrAddCredits => {
                "You've hit your usage limit. Upgrade your plan or add credits to continue, or try again later."
            }
            Self::AddCredits => {
                "You've hit your usage limit. Add credits to continue, or try again later."
            }
        }
    }
}

pub struct RetryHistory;

pub struct History {
    error: Option<String>,
}

impl gpui::EventEmitter<RetryHistory> for History {}

impl History {
    pub fn loading() -> Self {
        Self { error: None }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
}

pub(super) fn retry_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::Stateful<Div> {
    let id = id.into();
    let focus = crate::focus::handle(id.clone(), 0, window, cx);
    let t = cx.theme();
    crate::focus::ring(
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .relative()
            .flex_none()
            .px(px(12.))
            .py(px(6.))
            .rounded(Theme::RADIUS_SM)
            .bg(t.surface_secondary)
            .cursor_pointer()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(text(label, 14., 20., 500., t.text)),
        &focus,
        Theme::RADIUS_SM,
        px(2.),
        cx,
    )
}

impl Render for History {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut notices = div().flex().flex_col().gap(px(8.));
        if let Some(error) = &self.error {
            notices = notices.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(Theme::RADIUS_LG)
                    .border_1()
                    .border_color(t.border)
                    .px(px(12.))
                    .py(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(text("Couldn't load messages", 14., 20., 500., t.text))
                            .child(text(error.clone(), 13., 20., 400., t.text_secondary)),
                    )
                    .child(
                        retry_button("history-retry", "Retry", window, cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RetryHistory)))
                            .on_key_down(cx.listener(|_, e: &gpui::KeyDownEvent, _, cx| {
                                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                                    cx.emit(RetryHistory);
                                    cx.stop_propagation();
                                }
                            })),
                    ),
            );
        }
        div()
            .id("thread-history")
            .size_full()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.surface)
            .when(self.error.is_none(), |d| {
                d.child(text("Loading task…", 14., 20., 400., t.text_secondary))
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(16.))
                    .mx_auto()
                    .w_full()
                    .max_w(super::CONTENT_MAX_WIDTH)
                    .px(px(16.))
                    .child(notices),
            )
    }
}

impl ThreadView {
    pub fn set_usage_limit_notice(&mut self, notice: UsageLimitNotice, cx: &mut Context<Self>) {
        self.usage_limit_notice = notice;
        cx.notify();
    }

    pub(super) fn render_failure(
        &self,
        turn: &str,
        error: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let id = turn.to_owned();
        let keyboard_id = id.clone();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .rounded(Theme::RADIUS_LG)
            .border_1()
            .border_color(t.border)
            .px(px(16.))
            .py(px(12.))
            .child(text("This turn couldn't render", 14., 20., 500., t.text))
            .child(text(error.to_owned(), 13., 20., 400., t.text_secondary))
            .child(
                retry_button(format!("retry-turn-{turn}"), "Try again", window, cx)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.render_errors.borrow_mut().remove(&id);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(move |view, e: &gpui::KeyDownEvent, _, cx| {
                        if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                            view.render_errors.borrow_mut().remove(&keyboard_id);
                            cx.notify();
                            cx.stop_propagation();
                        }
                    })),
            )
    }
    pub fn turn_error_messages(&self, turn: &str) -> Vec<String> {
        self.turns
            .iter()
            .find(|t| t.id == turn)
            .into_iter()
            .flat_map(|turn| self.turn_failures(turn))
            .map(|(error, retrying)| error_title(error, retrying, self.usage_limit_notice))
            .collect()
    }

    pub fn render_error(&self, turn: &str) -> Option<String> {
        self.render_errors.borrow().get(turn).cloned()
    }

    fn turn_failures<'a>(
        &'a self,
        turn: &'a bridge::types::Turn,
    ) -> impl Iterator<Item = (&'a bridge::types::TurnError, bool)> + 'a {
        let errors = self.stream_errors.get(&turn.id);
        let terminal = turn
            .error
            .as_ref()
            .filter(|_| errors.is_none_or(|errors| errors.iter().all(|(_, retrying)| *retrying)));
        errors
            .into_iter()
            .flatten()
            .map(|(error, retrying)| (error, *retrying))
            .chain(terminal.map(|error| (error, false)))
    }

    pub(super) fn turn_errors(
        &self,
        turn: &bridge::types::Turn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let mut errors = self.turn_failures(turn).peekable();
        errors.peek()?;
        Some(
            div().flex().flex_col().children(
                errors
                    .enumerate()
                    .map(|(index, error)| self.error_notice((&turn.id, index), error, window, cx)),
            ),
        )
    }

    fn error_notice(
        &self,
        (turn, index): (&str, usize),
        (error, retrying): (&bridge::types::TurnError, bool),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let title = error_title(error, retrying, self.usage_limit_notice);
        let details = match (
            (title != error.message).then_some(error.message.as_str()),
            error.additional_details.as_deref(),
        ) {
            (Some(message), Some(details)) => format!("{message}\n{details}"),
            (Some(message), None) => message.to_owned(),
            (None, Some(details)) => details.to_owned(),
            (None, None) => String::new(),
        };
        let t = cx.theme().clone();
        let usage = !retrying
            && error
                .codex_error_info
                .as_ref()
                .is_some_and(|info| info == "usageLimitExceeded");
        let mut summary = div().flex().items_center().gap(px(6.));
        if usage {
            summary = summary
                .gap(px(8.))
                .child(div().flex_1().min_w(px(12.)).h(px(1.)).bg(t.border))
                .child(crate::icon::icon("toast-error", 16., t.text_secondary));
        } else if retrying {
            summary = summary.child(crate::icon::icon("thread-reconnect", 16., t.text_secondary));
        }
        summary = summary.child(
            text(title.clone(), 14., 20., 400., t.text_secondary)
                .flex_shrink_1()
                .min_w(px(0.)),
        );
        if usage {
            summary = summary.child(div().flex_1().min_w(px(12.)).h(px(1.)).bg(t.border));
        }
        let mut notice = div()
            .id(format!("turn-error-{turn}-{index}"))
            .mt(px(16.))
            .flex()
            .flex_col()
            .gap(px(4.));
        if details.is_empty() {
            return notice.child(summary);
        }
        let id = format!("error-details-{turn}-{index}");
        let focus = crate::focus::handle(id.clone(), 0, window, cx);
        let open = self.expanded.contains(&id);
        let keyboard_id = id.clone();
        let summary = crate::focus::ring(
            summary
                .id(id.clone())
                .relative()
                .role(gpui::Role::Button)
                .aria_label(title)
                .cursor_pointer(),
            &focus,
            Theme::RADIUS_SM,
            px(2.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |view, _, _, cx| {
            if !view.expanded.remove(&id) {
                view.expanded.insert(id.clone());
            }
            cx.notify();
        }))
        .on_key_down(cx.listener(move |view, e: &gpui::KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                if !view.expanded.remove(&keyboard_id) {
                    view.expanded.insert(keyboard_id.clone());
                }
                cx.notify();
                cx.stop_propagation();
            }
        }));
        notice = notice.child(summary);
        if open {
            notice = notice.child(text(details, 14., 20., 400., cx.theme().text_secondary));
        }
        notice
    }
}

fn error_title(
    error: &bridge::types::TurnError,
    retrying: bool,
    usage: UsageLimitNotice,
) -> String {
    let info = error.codex_error_info.as_ref();
    if retrying {
        let busy = info.is_some_and(|info| {
            info == "serverOverloaded"
                || info["responseStreamDisconnected"]["httpStatusCode"] == 429
        });
        let progress = error
            .message
            .trim()
            .strip_prefix("Reconnecting")
            .map(|s| s.strip_prefix("...").unwrap_or(s).trim())
            .filter(|s| {
                s.split_once('/').is_some_and(|(attempt, max)| {
                    attempt.parse::<u64>().is_ok() && max.parse::<u64>().is_ok()
                })
            });
        return match (busy, progress) {
            (true, Some(progress)) => format!("Server is busy, reconnecting {progress}"),
            (true, None) => "Server is busy, reconnecting".to_owned(),
            (false, Some(progress)) => format!("Reconnecting {progress}"),
            (false, None) => error.message.clone(),
        };
    }
    if info.is_some_and(|info| info == "usageLimitExceeded") {
        usage.message().to_owned()
    } else {
        error.message.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{UsageLimitNotice, error_title};
    use bridge::types::TurnError;
    use serde_json::{Value, json};

    #[test]
    fn reconnect_and_terminal_errors_preserve_the_source_contract() {
        let title = |message: &str, info: Value, retrying| {
            error_title(
                &TurnError {
                    message: message.into(),
                    codex_error_info: Some(info),
                    additional_details: None,
                },
                retrying,
                UsageLimitNotice::RetryLater,
            )
        };
        for message in ["Reconnecting... 2/5", "Reconnecting 2/5"] {
            assert_eq!(title(message, Value::Null, true), "Reconnecting 2/5");
        }
        assert_eq!(
            title(
                "Reconnecting... 2/5",
                json!({"responseStreamDisconnected":{"httpStatusCode":429}}),
                true
            ),
            "Server is busy, reconnecting 2/5"
        );
        assert_eq!(
            title("Connection closed", json!("serverOverloaded"), true),
            "Server is busy, reconnecting"
        );
        assert_eq!(
            title("Connection closed", Value::Null, true),
            "Connection closed"
        );
        assert_eq!(
            title("Reconnecting x/5", Value::Null, true),
            "Reconnecting x/5"
        );
        assert_eq!(
            title("Quota exhausted", json!("usageLimitExceeded"), false),
            "You've hit your usage limit. Try again later."
        );
        assert_eq!(
            title("Source failure", Value::Null, false),
            "Source failure"
        );
    }

    #[test]
    fn account_plans_select_supported_usage_advice() {
        for plan in ["free", "go"] {
            assert_eq!(
                UsageLimitNotice::for_plan(Some(plan)),
                UsageLimitNotice::Upgrade
            );
        }
        for plan in ["plus", "prolite"] {
            assert_eq!(
                UsageLimitNotice::for_plan(Some(plan)),
                UsageLimitNotice::UpgradeOrAddCredits
            );
        }
        for plan in ["pro", "promax"] {
            assert_eq!(
                UsageLimitNotice::for_plan(Some(plan)),
                UsageLimitNotice::AddCredits
            );
        }
        for plan in [None, Some("unknown")] {
            assert_eq!(
                UsageLimitNotice::for_plan(plan),
                UsageLimitNotice::RetryLater
            );
        }
    }
}
