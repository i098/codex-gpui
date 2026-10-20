use crate::{focus, text};
use gpui::{
    App, Div, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, img, px,
};
use std::sync::Arc;
use theme::ActiveTheme as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The original locked-use gates, supplied by feature and account requirements.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedUsePolicy {
    pub locked_use_enabled: bool,
    pub slingshot_enabled: bool,
    pub remote_control_available: Option<bool>,
    pub remote_control_access_required: bool,
    pub allow_locked_computer_use: Option<bool>,
}

impl LockedUsePolicy {
    pub fn allowed(&self) -> bool {
        self.locked_use_enabled
            && self.slingshot_enabled
            && self.remote_control_available != Some(false)
            && !self.remote_control_access_required
            && self.allow_locked_computer_use != Some(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedUseState {
    pub enabled: Option<bool>,
    #[serde(rename = "computerIconDataURL")]
    pub computer_icon_data_url: Option<String>,
    #[serde(rename = "lockIconDataURL")]
    pub lock_icon_data_url: Option<String>,
}

impl LockedUseState {
    pub fn visible(&self, policy: &LockedUsePolicy) -> bool {
        policy.allowed() && self.enabled.is_some()
    }
}

pub type LockedUseDispatch = Arc<dyn Fn(bool, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct NativeSettingsState {
    pub policy: LockedUsePolicy,
    pub locked_use: LockedUseState,
    pub pending: bool,
    pub error: Option<String>,
    pub dispatch: Option<LockedUseDispatch>,
}

impl Global for NativeSettingsState {}

fn change_locked_use(enabled: bool, cx: &mut App) {
    let Some(state) = cx.try_global::<NativeSettingsState>() else {
        return;
    };
    if state.pending || !state.locked_use.visible(&state.policy) {
        return;
    }
    if let Some(dispatch) = state.dispatch.clone() {
        dispatch(enabled, cx);
    }
}

pub fn locked_use_row(window: &mut Window, cx: &mut App) -> Option<Div> {
    let state = cx.try_global::<NativeSettingsState>()?;
    if !state.locked_use.visible(&state.policy) {
        return None;
    }
    let enabled = state.locked_use.enabled?;
    let disabled = state.pending || state.dispatch.is_none();
    let error = state.error.clone();
    let icons = state
        .locked_use
        .computer_icon_data_url
        .clone()
        .zip(state.locked_use.lock_icon_data_url.clone());
    let icons = icons.and_then(|(computer, lock)| {
        crate::screens::computer_use::data_image(
            &computer,
            "locked-use-computer-icon".into(),
            window,
            cx,
        )
        .zip(crate::screens::computer_use::data_image(
            &lock,
            "locked-use-lock-icon".into(),
            window,
            cx,
        ))
    });
    let leading = match icons {
        Some((computer, lock)) => Some(
            div()
                .w(px(40.))
                .h(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .w(px(40.3))
                        .h(px(40.95))
                        .child(
                            img(computer)
                                .absolute()
                                .left(px(2.6))
                                .top_0()
                                .w(px(34.45))
                                .h(px(34.45)),
                        )
                        .child(
                            img(lock)
                                .absolute()
                                .right_0()
                                .bottom(px(3.9))
                                .w(px(17.55))
                                .h(px(21.45)),
                        ),
                )
                .into_any_element(),
        ),
        _ => None,
    };
    let t = cx.theme().clone();
    let handle = focus::handle("locked-use-enabled", 0, window, cx);
    let control = focus::ring(
        crate::screens::settings::switch("locked-use-enabled", enabled, disabled, &t),
        &handle,
        px(10.),
        px(2.),
        cx,
    )
    .on_click(move |_, _, cx| {
        if !disabled {
            change_locked_use(!enabled, cx);
        }
    })
    .on_key_down(move |event: &KeyDownEvent, _, cx| {
        if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
            change_locked_use(!enabled, cx);
            cx.stop_propagation();
        }
    })
    .into_any_element();
    let help = "https://developers.openai.com/codex/app/computer-use#locked-use";
    let help_handle = focus::handle("locked-use-help", 0, window, cx);
    let link = focus::ring(
        div()
            .id("locked-use-help")
            .child(text("Learn more", 12., 16., 430., t.text_accent)),
        &help_handle,
        px(2.),
        px(1.),
        cx,
    )
    .on_click(move |_, _, cx| cx.open_url(help))
    .on_key_down(move |event: &KeyDownEvent, _, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            cx.open_url(help);
            cx.stop_propagation();
        }
    });
    let detail = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .child(text(
                    "Let ChatGPT use your Mac when it's locked. ",
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                ))
                .child(link),
        )
        .children(error.map(|error| text(error, 12., 16., 430., t.text_error)));
    Some(
        div()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .children(leading)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(text("Locked use", 13., 18.571_428, 500., t.text))
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .w(px(160.))
                    .max_w(gpui::relative(0.4))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child(control),
            ),
    )
}

/// One request owns all updates until completion, failure, or cancellation.
#[derive(Clone, Debug, Default)]
pub struct Appshot {
    pub request_id: String,
    pub app: Option<Value>,
    pub ax_text: Option<String>,
    pub screenshot: Option<Value>,
    pub completed: bool,
    pub failure_reason: Option<String>,
}

impl Appshot {
    pub fn new(request_id: String) -> Self {
        Self {
            request_id,
            ..Self::default()
        }
    }

    pub fn update(&mut self, request_id: &str, update: &Value) -> Result<(), &'static str> {
        if self.request_id != request_id || self.completed {
            return Err("capture request is stale");
        }
        match update["type"].as_str() {
            Some("metadata") if update["app"]["bundleIdentifier"].is_string() => {
                self.app = Some(update["app"].clone());
            }
            Some("axText") if update["text"].is_string() => {
                self.ax_text = update["text"].as_str().map(str::to_owned);
            }
            Some("screenshot") => self.screenshot = Some(update.clone()),
            Some("completed") => self.completed = true,
            Some("failed") => {
                self.completed = true;
                self.failure_reason = Some(
                    update["failureReason"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("native_service_failed")
                        .to_owned(),
                );
            }
            _ => return Err("invalid native capture update"),
        }
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.completed = true;
    }
}
