use gpui::{
    AnimationExt as _, Context, Div, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::*, px,
};
use theme::{ActiveTheme as _, Theme};

use super::ThreadView;
use crate::text;

static RETRY_NUMERALS: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));

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
            .px(px(8.))
            .py(px(2.))
            .rounded(Theme::RADIUS_FULL)
            .border_1()
            .border_color(gpui::rgba(0x00000000))
            .cursor_pointer()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(text(label, 13., 18., 430., t.text)),
        &focus,
        Theme::RADIUS_FULL,
        px(2.),
        cx,
    )
}

impl Render for History {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut notices = div().flex().flex_col().gap(px(8.));
        if let Some(error) = &self.error {
            let retry = retry_button("history-retry", "Retry", window, cx)
                .debug_selector(|| "history-retry".into())
                .aria_label(format!("Retry loading messages: {error}"))
                .hover(|style| style.bg(t.ghost_hover))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(RetryHistory)))
                .on_key_down(cx.listener(|_, e: &gpui::KeyDownEvent, _, cx| {
                    if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                        cx.emit(RetryHistory);
                        cx.stop_propagation();
                    }
                }));
            notices = notices.child(
                div()
                    .debug_selector(|| "history-failure".into())
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pl(px(12.))
                    .pr(px(8.))
                    .py(px(8.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(Theme::RADIUS_3XL),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.surface),
                        shadows: vec![
                            crate::composer::shadow(t.border_strong, 0., 0., 0.5, false),
                            crate::composer::shadow(gpui::black().opacity(0.05), 0., 2., 0., false),
                            crate::composer::shadow(gpui::black().opacity(0.02), 4., 6., 0., false),
                        ],
                        ..Default::default()
                    }))
                    .child(text("Couldn't load messages", 13., 21.125, 430., t.text).flex_1())
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .border_l_1()
                            .border_color(t.border)
                            .pl(px(12.))
                            .child(crate::tooltip::tooltip(
                                retry,
                                error.clone(),
                                None,
                                crate::tooltip::Side::Top,
                            )),
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
            .pt(px(32.))
            .pb(px(114.))
            .bg(t.surface)
            .when(self.error.is_none(), |d| {
                d.child(
                    div()
                        .id("history-loading")
                        .debug_selector(|| "history-loading".into())
                        .role(gpui::Role::Status)
                        .aria_label("Loading task")
                        .size(px(28.))
                        .child(
                            crate::icon::icon("sidebar-cmd-spinner", 28., t.text_secondary)
                                .own_layer(None)
                                .with_animation(
                                    "history-loading",
                                    gpui::Animation::new(std::time::Duration::from_secs(1))
                                        .repeat_synced(),
                                    |icon, progress| {
                                        icon.with_transformation(gpui::Transformation::rotate(
                                            gpui::radians(progress * std::f32::consts::TAU),
                                        ))
                                    },
                                ),
                        ),
                )
            })
            .when(self.error.is_some(), |history| {
                history.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .child(
                            gpui::canvas(
                                move |bounds, window, _| {
                                    super::gradient_image(
                                        &[(t.surface, 0.), (gpui::transparent_black(), 1.)],
                                        t.surface,
                                        (
                                            f32::from(bounds.size.width),
                                            f32::from(bounds.size.height),
                                        ),
                                        (0., f32::from(bounds.top()) - 44.),
                                        true,
                                        window.scale_factor(),
                                    )
                                },
                                |bounds, image, window, _| {
                                    window
                                        .paint_image(
                                            bounds,
                                            bounds,
                                            gpui::Corners::default(),
                                            image,
                                            0,
                                            false,
                                        )
                                        .ok();
                                },
                            )
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(-27.))
                            .h(px(32.))
                            .top_layer(),
                        )
                        .child(
                            div()
                                .relative()
                                .w_full()
                                .pt(px(5.))
                                .pb(px(126.))
                                .layer()
                                .cc_tiles()
                                .window_space_recording()
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(5.))
                                        .left_0()
                                        .right_0()
                                        .bottom_0()
                                        .child(shapes::decoration(shapes::Decoration {
                                            fill: Some(t.surface),
                                            ..Default::default()
                                        })),
                                )
                                .child(
                                    div()
                                        .mx_auto()
                                        .w_full()
                                        .max_w(super::CONTENT_MAX_WIDTH)
                                        .px(px(16.))
                                        .child(notices),
                                ),
                        ),
                )
            })
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
        let retry = retry_button(format!("retry-turn-{turn}"), "Try again", window, cx)
            .debug_selector(|| format!("retry-turn-{turn}"))
            .aria_label(format!("Try again: {error}"))
            .bg(t.text_alpha_5)
            .hover(|style| style.bg(t.text_alpha_10))
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
            }));
        div()
            .debug_selector(|| format!("render-failure-{turn}"))
            .relative()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .px(px(17.))
            .py(px(13.))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(Theme::RADIUS_LG),
                fill: Some(t.surface),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(text(
                "This turn couldn't render",
                13.,
                f32::from(Theme::TEXT_SM.line_height),
                500.,
                t.text,
            ))
            .child(crate::tooltip::tooltip(
                retry,
                error.to_owned(),
                None,
                crate::tooltip::Side::Top,
            ))
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
            summary =
                summary.child(crate::icon::icon("toast-error", 16., t.text_alpha_60).runtime());
        } else if retrying {
            summary = summary.child(crate::icon::icon("thread-reconnect", 16., t.text_alpha_60));
        }
        let color = if usage {
            t.text_secondary
        } else {
            t.text_alpha_60
        };
        let label = if let Some((prefix, count)) = title
            .rsplit_once(' ')
            .filter(|(_, count)| retrying && count.contains('/'))
        {
            let prefix = format!("{prefix} ");
            let prefix_width =
                px((f32::from(super::width(window, &prefix, 14., 430., None)) * 64.).ceil() / 64.);
            let font = gpui::Font {
                weight: gpui::FontWeight(430.),
                ..gpui::font(crate::SANS)
            };
            let font_id = window.text_system().resolve_font(&font);
            let digit_width = window
                .text_system()
                .ch_advance(font_id, px(14.))
                .expect("The interface font must contain a zero glyph");
            let digit_width = px((f32::from(digit_width) * 64.).round() / 64.);
            let (digits, limit) = count.split_once('/').unwrap();
            div()
                .flex()
                .child(
                    div()
                        .w(prefix_width)
                        .flex_none()
                        .child(text(prefix, 14., 21., 430., color)),
                )
                .child(
                    div()
                        .flex()
                        .layer()
                        .children(digits.chars().map(|digit| {
                            div()
                                .relative()
                                .w(digit_width)
                                .h(px(21.))
                                .flex_none()
                                .overflow_hidden()
                                .snap_origin()
                                .layer()
                                .child(
                                    text(digit.to_string(), 14., 21., 430., color)
                                        .font_features(RETRY_NUMERALS.clone()),
                                )
                        }))
                        .child(
                            text(format!("/{limit}"), 14., 21., 430., color)
                                .font_features(RETRY_NUMERALS.clone()),
                        ),
                )
        } else {
            div().child(text(title.clone(), 14., 21., 430., color))
        };
        summary = summary.child(label.flex_shrink_1().min_w(px(0.)));
        if usage {
            summary = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().h(px(1.)).bg(t.border))
                .child(summary)
                .child(div().flex_1().h(px(1.)).bg(t.border));
        }
        let mut notice = div()
            .id(format!("turn-error-{turn}-{index}"))
            .debug_selector(|| format!("turn-error-{turn}-{index}"))
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
