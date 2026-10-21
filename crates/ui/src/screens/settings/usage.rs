#[path = "usage_model.rs"]
pub mod model;
pub use model::*;

use super::{Layer, deco, on_card, probe};
use crate::screens::computer_use::Load;
use crate::{composer::Composer, focus, text};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Div, Entity, Global,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window as AppWindow, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    History(i64),
    Reset(Option<String>),
    SaveReload(ReloadDraft),
    DisableReload,
    RequestMore(String),
    OpenUrl(String),
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub access: Access,
    pub plan: String,
    pub limits: Load<Arc<Limits>>,
    pub reload: Load<Arc<Reload>>,
    pub reload_pricing: Load<Arc<Value>>,
    pub has_saved_payment_method: bool,
    pub reset_history: Load<Arc<Value>>,
    pub history: Load<Option<Arc<BalanceHistory>>>,
    pub analytics: Load<Arc<Value>>,
    pub monthly: Load<Arc<Value>>,
    pub request_policy: Load<Arc<Value>>,
    pub requests: Load<Arc<Value>>,
    pub page_open: bool,
    pub unavailable: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub revision: u64,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

#[derive(Clone)]
enum Control {
    Action(Action),
    Tab(bool),
    EditReload,
    CloseReload,
    SaveReload,
    ConfirmDisable,
    RequestForm(bool),
    SubmitRequest,
    Reset(Option<String>),
    ResetHistory(bool),
}

pub fn render(window: &mut AppWindow, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("usage-settings", cx, |_, cx| {
            cx.observe_global::<State>(|_: &mut Usage, cx| cx.notify())
                .detach();
            cx.default_global::<State>().page_open = true;
            cx.on_release(|_: &mut Usage, cx| {
                if cx.has_global::<State>() {
                    cx.update_global::<State, _>(|state, _| state.page_open = false);
                }
            })
            .detach();
            if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
                dispatch(Action::Refresh, cx);
            }
            let fields = std::array::from_fn(|index| {
                let placeholder = match index {
                    0 => "125",
                    1 => "250",
                    2 => "No limit",
                    _ => "Tell your admin why you need more usage",
                };
                let field = cx.new(|cx| Composer::settings_field(placeholder, "", index == 3, cx));
                cx.observe(&field, |this: &mut Usage, _, cx| {
                    this.field_error = None;
                    cx.notify();
                })
                .detach();
                field
            });
            Usage {
                layer: Layer::default(),
                analytics: false,
                reload_open: false,
                disable_confirmation: false,
                fields,
                request_open: false,
                confirm_reset: None,
                reset_history: false,
                field_error: None,
                seen_revision: 0,
            }
        })
        .into_any_element()
}

struct Usage {
    layer: Layer,
    analytics: bool,
    reload_open: bool,
    disable_confirmation: bool,
    fields: [Entity<Composer>; 4],
    request_open: bool,
    confirm_reset: Option<Option<String>>,
    reset_history: bool,
    field_error: Option<String>,
    seen_revision: u64,
}

impl Usage {
    fn dispatch(&self, action: Action, cx: &mut App) {
        if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
            dispatch(action, cx);
        }
    }

    fn control(&mut self, control: Control, cx: &mut Context<Self>) {
        match control {
            Control::Action(action) => self.dispatch(action, cx),
            Control::Tab(analytics) => self.analytics = analytics,
            Control::EditReload => {
                if let Load::Ready(settings) = &cx.global::<State>().reload {
                    let draft = ReloadDraft::from_settings(settings);
                    for (field, value) in
                        self.fields
                            .iter()
                            .zip([draft.threshold, draft.target, draft.monthly])
                    {
                        field.update(cx, |field, cx| field.set_text(&value, cx));
                    }
                    self.reload_open = true;
                    self.disable_confirmation = false;
                    self.field_error = None;
                }
            }
            Control::CloseReload => {
                if !cx.global::<State>().busy {
                    self.reload_open = false;
                }
            }
            Control::SaveReload => {
                let draft = ReloadDraft {
                    threshold: self.fields[0].read(cx).field_text().into(),
                    target: self.fields[1].read(cx).field_text().into(),
                    monthly: self.fields[2].read(cx).field_text().into(),
                };
                let state = cx.global::<State>();
                if let Load::Ready(settings) = &state.reload {
                    match draft.validate(settings, state.access.maximum_credits) {
                        Ok(()) => self.dispatch(Action::SaveReload(draft), cx),
                        Err(error) => self.field_error = Some(error.into()),
                    }
                }
            }
            Control::ConfirmDisable => {
                if self.disable_confirmation {
                    self.dispatch(Action::DisableReload, cx);
                } else {
                    self.disable_confirmation = true;
                }
            }
            Control::RequestForm(open) => {
                self.request_open = open;
                self.field_error = None;
                if open {
                    let justification = pending_request(cx.global::<State>())
                        .and_then(|v| v["justification"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    self.fields[3].update(cx, |field, cx| field.set_text(&justification, cx));
                }
            }
            Control::SubmitRequest => {
                let justification = self.fields[3].read(cx).field_text().trim().to_owned();
                if justification.is_empty() {
                    self.field_error = Some("Enter a justification to submit your request".into());
                } else {
                    self.dispatch(Action::RequestMore(justification), cx);
                }
            }
            Control::Reset(id) => {
                if self.confirm_reset.as_ref() == Some(&id) {
                    self.dispatch(Action::Reset(id), cx);
                } else {
                    self.confirm_reset = Some(id);
                }
            }
            Control::ResetHistory(history) => self.reset_history = history,
        }
        cx.notify();
    }

    fn action(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        control: Control,
        disabled: bool,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id = id.into();
        let label = label.into();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key = control.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .relative()
                .h(px(28.))
                .px(px(13.))
                .flex()
                .items_center()
                .justify_center()
                .child(on_card(
                    12.5,
                    shapes::Corner::Squircle,
                    t.text_alpha_5,
                    None,
                ))
                .child(text(label, 13., 18., 430., t.text))
                .when(disabled, |d| d.opacity(0.6)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                this.control(control.clone(), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.control(key.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn status(
        &self,
        message: &str,
        retry: Option<Action>,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
        let row = div()
            .p(px(16.))
            .flex()
            .items_center()
            .justify_between()
            .child(text(
                message.to_owned(),
                13.,
                19.,
                430.,
                cx.theme().text_secondary,
            ));
        match retry {
            Some(action) => row.child(self.action(
                "usage-retry",
                "Retry",
                Control::Action(action),
                false,
                window,
                cx,
            )),
            None => row,
        }
    }

    fn plan(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Option<Div> {
        let name = match state.plan.as_str() {
            "free" => "ChatGPT Free",
            "go" => "ChatGPT Go",
            "plus" => "ChatGPT Plus",
            "prolite" => "ChatGPT Pro 100",
            "pro" => "ChatGPT Pro 200",
            "promax" => "ChatGPT Pro 500",
            _ => return None,
        };
        let control = self.action(
            "usage-view-plans",
            "View plans",
            Control::Action(Action::OpenUrl("https://chatgpt.com/#pricing".into())),
            false,
            window,
            cx,
        );
        Some(self.group(
            "Your plan",
            None,
            row(name, None, control, cx.theme()),
            cx.theme(),
        ))
    }

    fn limits(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut content = div().flex().flex_col();
        match &state.limits {
            Load::Loading => {
                content = content.child(self.status("Loading usage settings…", None, window, cx))
            }
            Load::Failed(_) => {
                content = content.child(self.status(
                    "Could not load usage settings.",
                    Some(Action::Refresh),
                    window,
                    cx,
                ))
            }
            Load::Ready(limits) => {
                let values: Vec<&Limit> = if limits.rate_limits_by_limit_id.is_empty() {
                    vec![&limits.rate_limits]
                } else {
                    limits.rate_limits_by_limit_id.values().collect()
                };
                for limit in values {
                    if let Some(name) = &limit.limit_name {
                        content = content.child(
                            text(name.clone(), 14., 21., 500., t.text)
                                .px(px(16.))
                                .pt(px(12.)),
                        );
                    }
                    for (index, limit) in limit
                        .primary
                        .iter()
                        .chain(limit.secondary.iter())
                        .enumerate()
                    {
                        content = content.child(self.meter(limit, index, &t));
                    }
                }
            }
        }
        if state.access.enterprise {
            content = content.child(self.enterprise(state, window, cx));
        }
        self.group("Plan limits", Some("Shared across Codex, Work, Workspace Agents, and ChatGPT for Excel. Chat conversations are not included."), content, &t)
    }

    fn meter(&self, limit: &model::Window, index: usize, t: &Theme) -> Div {
        let remaining = limit.remaining();
        let label = limit.label();
        let reset = limit.resets_at.map(countdown).unwrap_or_default();
        div()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text(label, 13., 19., 500., t.text))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(text(reset, 12., 16., 430., t.text_secondary))
                            .child(text(
                                format!("{remaining:.0}% left"),
                                12.,
                                16.,
                                430.,
                                t.text_secondary,
                            )),
                    )
                    .child(
                        div()
                            .id(("usage-meter", index))
                            .role(gpui::Role::ProgressIndicator)
                            .aria_label(label)
                            .aria_value(format!("{remaining:.0}% left"))
                            .relative()
                            .h(px(6.))
                            .w_full()
                            .child(on_card(9999., shapes::Corner::Round, t.text_alpha_10, None))
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(remaining as f32 / 100.))
                                    .child(on_card(
                                        9999.,
                                        shapes::Corner::Round,
                                        if remaining <= 10. {
                                            t.text_warning
                                        } else {
                                            t.text_accent
                                        },
                                        None,
                                    )),
                            ),
                    ),
            )
    }

    fn enterprise(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col();
        let monthly = match &state.monthly {
            Load::Ready(value) => value,
            Load::Loading => return self.status("Loading usage settings…", None, window, cx),
            Load::Failed(_) => {
                return self.status(
                    "Could not load usage settings.",
                    Some(Action::Refresh),
                    window,
                    cx,
                );
            }
        };
        let limit = &monthly["effective_monthly_limit"];
        let number = |value: &Value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.parse::<f64>().ok())
                .filter(|number| number.is_finite())
        };
        let used = number(&monthly["current_month_usage"]);
        let amount = if limit["limit_mode"] == "unlimited_platform_max" {
            None
        } else if limit["limit_amount"]["unit"] == monthly["balance_unit"] {
            number(&limit["limit_amount"]["amount"])
        } else {
            number(&limit["limit"])
        };
        if let (Some(amount), Some(used)) = (amount, used) {
            let usage = model::Window {
                used_percent: if amount == 0. {
                    100.
                } else {
                    (used / amount * 100.).clamp(0., 100.)
                },
                window_duration_mins: Some(43200),
                resets_at: None,
            };
            content = content.child(self.meter(&usage, 100, cx.theme())).child(
                text(
                    format!("{used} of {amount} credits used"),
                    13.,
                    19.,
                    430.,
                    cx.theme().text_secondary,
                )
                .px(px(16.)),
            );
        } else {
            content = content.child(text(
                "Your admin hasn’t set a usage limit",
                13.,
                19.,
                430.,
                cx.theme().text_secondary,
            ));
            if let Some(used) = used {
                content = content.child(text(
                    format!("Your usage this month: {used} credits"),
                    13.,
                    19.,
                    430.,
                    cx.theme().text_secondary,
                ));
            }
        }
        if limit["enforcement_mode"] != "HARD_CAP" {
            return content;
        }
        let policy = match &state.request_policy {
            Load::Ready(value) => value,
            Load::Loading => {
                return content.child(self.status(
                    "Loading usage request settings…",
                    None,
                    window,
                    cx,
                ));
            }
            Load::Failed(_) => {
                return content.child(self.status(
                    "Couldn’t load usage request settings",
                    Some(Action::Refresh),
                    window,
                    cx,
                ));
            }
        };
        if policy["kind"] == "custom" {
            if let Some(instructions) = policy["instructions"].as_str() {
                content = content.child(
                    text(
                        instructions.to_owned(),
                        13.,
                        19.,
                        430.,
                        cx.theme().text_secondary,
                    )
                    .whitespace_normal(),
                );
            }
            if let Some(url) = policy["request_url"]
                .as_str()
                .filter(|url| url.starts_with("https://"))
            {
                content = content.child(self.action(
                    "usage-custom-request",
                    "Request more usage",
                    Control::Action(Action::OpenUrl(url.into())),
                    false,
                    window,
                    cx,
                ));
            }
        } else if policy["kind"] == "openai_native" {
            match &state.requests {
                Load::Loading => {
                    return content.child(self.status(
                        "Loading usage request settings…",
                        None,
                        window,
                        cx,
                    ));
                }
                Load::Failed(_) => {
                    return content.child(self.status(
                        "Couldn’t load usage request settings",
                        Some(Action::Refresh),
                        window,
                        cx,
                    ));
                }
                Load::Ready(_) => {}
            }
            let pending = pending_request(state).is_some();
            if pending {
                content = content.child(text(
                    "Your request is pending admin review.",
                    13.,
                    19.,
                    430.,
                    cx.theme().text_secondary,
                ));
            }
            content = content.child(self.action(
                "usage-request",
                if pending {
                    "Update pending request"
                } else {
                    "Request more usage"
                },
                Control::RequestForm(true),
                state.busy,
                window,
                cx,
            ));
            if self.request_open {
                content = content
                    .child(text("Justification", 13., 19., 500., cx.theme().text))
                    .child(self.input(3, "Justification", cx))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(self.action(
                                "usage-request-cancel",
                                "Cancel",
                                Control::RequestForm(false),
                                state.busy,
                                window,
                                cx,
                            ))
                            .child(self.action(
                                "usage-request-submit",
                                if pending {
                                    "Update request"
                                } else {
                                    "Submit request"
                                },
                                Control::SubmitRequest,
                                state.busy,
                                window,
                                cx,
                            )),
                    );
            }
        }
        content.p(px(16.)).gap(px(8.))
    }

    fn credits(
        &self,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if !state.access.view_credits {
            return None;
        }
        let credits = match &state.limits {
            Load::Ready(limits) => limits.rate_limits.credits.as_ref(),
            _ => None,
        };
        let balance = match credits {
            Some(credits) if credits.unlimited => "Unlimited credit".to_owned(),
            Some(credits) => credits
                .balance
                .as_ref()
                .map(|balance| format!("{balance} credits remaining"))
                .unwrap_or_else(|| "Credit remaining unavailable".into()),
            None => "Credit remaining unavailable".into(),
        };
        let control = if state.access.manage_credits {
            self.action(
                "usage-add-credits",
                "Add more",
                Control::Action(Action::OpenUrl(
                    "https://chatgpt.com/settings/usage?credit_modal=true".into(),
                )),
                false,
                window,
                cx,
            )
        } else {
            div().into_any_element()
        };
        let mut content = div().flex().flex_col().child(row(
            &balance,
            Some("Current balance"),
            control,
            cx.theme(),
        ));
        if state.access.manage_credits {
            content = content.child(match &state.reload {
                Load::Loading => {
                    self.status("Loading automatic reload settings…", None, window, cx)
                }
                Load::Failed(_) => self.status(
                    "Could not load automatic reload settings",
                    Some(Action::Refresh),
                    window,
                    cx,
                ),
                Load::Ready(reload) => {
                    let control = self.action(
                        "usage-edit-reload",
                        if reload.is_enabled {
                            "Manage auto-reload"
                        } else {
                            "Set up auto-reload"
                        },
                        Control::EditReload,
                        state.busy,
                        window,
                        cx,
                    );
                    row("Automatic reload", None, control, cx.theme())
                }
            });
        }
        Some(self.group("Credits", Some(if state.access.manage_credits {
            "Buy credits or turn on automatic reload to continue using Work and Codex when you reach usage limits."
        } else { "Your remaining credits" }), content, cx.theme()))
    }

    fn resets(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col();
        match &state.limits {
            Load::Loading => {
                content =
                    content.child(self.status("Loading usage limit resets…", None, window, cx))
            }
            Load::Failed(_) => {
                content = content.child(self.status(
                    "Couldn’t load usage limit resets",
                    Some(Action::Refresh),
                    window,
                    cx,
                ))
            }
            Load::Ready(limits) => {
                let Some(resets) = &limits.rate_limit_reset_credits else {
                    return self.group(
                        "Usage limit resets",
                        Some("Use a reset to restore your 5-hour limit, weekly limit, or both"),
                        self.status(
                            "Couldn’t load usage limit resets",
                            Some(Action::Refresh),
                            window,
                            cx,
                        ),
                        cx.theme(),
                    );
                };
                if resets.history_enabled {
                    content = content.child(
                        div()
                            .p(px(16.))
                            .flex()
                            .gap(px(8.))
                            .child(self.action(
                                "usage-resets-available",
                                format!("Available {}", resets.available_count),
                                Control::ResetHistory(false),
                                false,
                                window,
                                cx,
                            ))
                            .child(self.action(
                                "usage-resets-history",
                                "History",
                                Control::ResetHistory(true),
                                false,
                                window,
                                cx,
                            )),
                    );
                }
                if self.reset_history && resets.history_enabled {
                    content = content.child(match &state.reset_history {
                        Load::Loading => {
                            self.status("Loading banked reset history…", None, window, cx)
                        }
                        Load::Failed(_) => self.status(
                            "Couldn’t load banked reset history",
                            Some(Action::Refresh),
                            window,
                            cx,
                        ),
                        Load::Ready(value) => {
                            let mut rows = div().flex().flex_col();
                            for event in value["events"].as_array().into_iter().flatten() {
                                let label = if event["kind"] == "granted" {
                                    "Reset received"
                                } else {
                                    "Reset used"
                                };
                                let time = event["occurred_at"].as_str().unwrap_or_default();
                                rows = rows.child(row(
                                    label,
                                    None,
                                    text(
                                        time.to_owned(),
                                        12.,
                                        16.,
                                        430.,
                                        cx.theme().text_secondary,
                                    )
                                    .into_any_element(),
                                    cx.theme(),
                                ));
                            }
                            if value["events"].as_array().is_none_or(Vec::is_empty) {
                                rows = rows.child(self.status(
                                    "No banked reset activity in the past 30 days",
                                    None,
                                    window,
                                    cx,
                                ));
                            }
                            rows
                        }
                    });
                } else if resets.available_count == 0 {
                    content = content.child(self.status("No resets available", None, window, cx));
                } else {
                    let available: Vec<_> = resets
                        .credits
                        .iter()
                        .filter(|credit| credit.status == "available")
                        .collect();
                    if available.is_empty() {
                        let control = self.action(
                            "usage-reset-auto",
                            if self.confirm_reset == Some(None) {
                                "Confirm"
                            } else {
                                "Use reset"
                            },
                            Control::Reset(None),
                            state.busy,
                            window,
                            cx,
                        );
                        content = content.child(row("Usage reset", None, control, cx.theme()));
                    }
                    for credit in available {
                        let id = Some(credit.id.clone());
                        let label = if self.confirm_reset.as_ref() == Some(&id) {
                            "Confirm"
                        } else {
                            "Use reset"
                        };
                        let control = self.action(
                            format!("usage-reset-{}", credit.id),
                            label,
                            Control::Reset(id),
                            state.busy,
                            window,
                            cx,
                        );
                        let expiry = credit.expires_at.map(|at| format!("Expires {}", date(at)));
                        content = content.child(row(
                            credit.title.as_deref().unwrap_or("Usage reset"),
                            expiry.as_deref(),
                            control,
                            cx.theme(),
                        ));
                    }
                }
            }
        }
        self.group(
            "Usage limit resets",
            Some("Use a reset to restore your 5-hour limit, weekly limit, or both"),
            content,
            cx.theme(),
        )
    }

    fn history(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col().gap(px(12.));
        let mut controls = div().flex().gap(px(8.));
        if let Load::Ready(limits) = &state.limits {
            for limit in limits
                .rate_limits
                .primary
                .iter()
                .chain(limits.rate_limits.secondary.iter())
            {
                if let Some(minutes @ (300 | 10080)) = limit.window_duration_mins {
                    controls = controls.child(self.action(
                        format!("usage-history-{minutes}"),
                        if minutes == 300 { "5-hour" } else { "Weekly" },
                        Control::Action(Action::History(minutes)),
                        false,
                        window,
                        cx,
                    ));
                }
            }
        }
        content = content.child(controls);
        content = content.child(match &state.history {
            Load::Loading => self.status("Loading…", None, window, cx),
            Load::Failed(_) => self.status(
                "Could not load usage settings.",
                Some(Action::History(10080)),
                window,
                cx,
            ),
            Load::Ready(None) => {
                self.status("Balance history isn’t available yet", None, window, cx)
            }
            Load::Ready(Some(history)) if history.points.is_empty() => {
                self.status("Balance history isn’t available yet", None, window, cx)
            }
            Load::Ready(Some(history)) => {
                match history.samples(chrono::Utc::now().timestamp_millis()) {
                    Err(error) => self.status(&error, Some(Action::History(10080)), window, cx),
                    Ok(samples) => {
                        let mut chart = div().h(px(220.)).flex().items_end().gap(px(2.));
                        let first = samples
                            .first()
                            .map(|sample| sample.timestamp)
                            .unwrap_or_default();
                        for sample in samples {
                            chart = chart.child(
                                div().flex_1().h_full().flex().items_end().child(
                                    match sample.remaining {
                                        Some(remaining) => div()
                                            .w_full()
                                            .h(gpui::relative(remaining as f32 / 100.))
                                            .bg(cx.theme().text_accent),
                                        None => div().w_full().h_full().bg(cx.theme().text_alpha_5),
                                    },
                                ),
                            );
                        }
                        div()
                            .child(chart)
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(text(
                                        date(first / 1000),
                                        12.,
                                        16.,
                                        430.,
                                        cx.theme().text_secondary,
                                    ))
                                    .child(text("Now", 12., 16., 430., cx.theme().text_secondary)),
                            )
                            .child(text(
                                format!(
                                    "History through {}",
                                    history.data_as_of.as_deref().unwrap_or_else(|| history
                                        .points
                                        .last()
                                        .map(|p| p.observed_at.as_str())
                                        .unwrap_or_default())
                                ),
                                12.,
                                16.,
                                430.,
                                cx.theme().text_secondary,
                            ))
                    }
                }
            }
        });
        self.group(
            "Recent plan usage",
            Some("How your activity draws down your allowance"),
            content.p(px(16.)),
            cx.theme(),
        )
    }

    fn input(&self, field: usize, label: &'static str, cx: &Context<Self>) -> AnyElement {
        div()
            .id(("usage-input", field))
            .role(gpui::Role::TextInput)
            .aria_label(label)
            .aria_value(self.fields[field].read(cx).field_text().to_owned())
            .w_full()
            .child(self.fields[field].clone())
            .into_any_element()
    }

    fn reload_dialog(
        &self,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let enabled = matches!(&state.reload, Load::Ready(settings) if settings.is_enabled);
        let mut body = div()
            .relative()
            .w(px(536.))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(on_card(
                20.,
                shapes::Corner::Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(text("Auto-reload credits", 20., 26., 500., t.text));
        if self.disable_confirmation {
            body = body.child(text("Turn auto-reload off", 16., 24., 500., t.text));
        } else {
            for (field, label, helper) in [
                (
                    0,
                    "Minimum balance",
                    "Automatically purchase credits when my balance drops below this amount",
                ),
                (
                    1,
                    "Target balance",
                    "Bring my balance back to this amount if I hit my minimum",
                ),
                (
                    2,
                    "Maximum monthly spend (optional)",
                    "Limit the total number of credits purchased through auto-reload each month. Leave blank for no limit.",
                ),
            ] {
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(text(label, 14., 20., 500., t.text))
                        .child(self.input(field, label, cx))
                        .child(text(helper, 13., 18., 430., t.text_secondary).whitespace_normal()),
                );
            }
            body = body.child(div().flex().flex_col().gap(px(4.))
                .child(text("Your card will be automatically charged", 14., 20., 500., t.text))
                .child(text("Enabling auto-reload will charge your card every time you drop below your minimum balance. You can disable it in settings at any time.", 13., 19., 430., t.text_secondary).whitespace_normal()));
            if !enabled {
                match &state.reload_pricing {
                    Load::Loading => {
                        body = body.child(text("Loading price", 13., 19., 430., t.text_secondary))
                    }
                    Load::Failed(_) => {
                        body = body.child(text(
                            "Could not load credit pricing. Try again to continue.",
                            13.,
                            19.,
                            430.,
                            t.text_error,
                        ))
                    }
                    Load::Ready(_) => {}
                }
            }
        }
        if let Some(error) = self.field_error.as_ref().or(state.error.as_ref()) {
            body =
                body.child(text(error.clone(), 13., 19., 430., t.text_error).whitespace_normal());
        }
        let mut actions = div().flex().justify_end().gap(px(8.));
        if enabled {
            actions = actions.child(self.action(
                "usage-disable-reload",
                if self.disable_confirmation {
                    "Turn off"
                } else {
                    "Turn auto-reload off"
                },
                Control::ConfirmDisable,
                state.busy,
                window,
                cx,
            ));
        }
        actions = actions.child(self.action(
            "usage-cancel-reload",
            "Cancel",
            Control::CloseReload,
            state.busy,
            window,
            cx,
        ));
        if !self.disable_confirmation {
            actions = actions.child(self.action(
                "usage-save-reload",
                if enabled {
                    "Save"
                } else {
                    "Turn on auto-reload"
                },
                Control::SaveReload,
                state.busy || (!enabled && !matches!(state.reload_pricing, Load::Ready(_))),
                window,
                cx,
            ));
        }
        body = body.child(actions);
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.4))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.control(Control::CloseReload, cx);
                    cx.stop_propagation();
                }
            }))
            .child(body)
            .into_any_element()
    }

    fn group(
        &self,
        title: &'static str,
        subtitle: Option<&'static str>,
        content: Div,
        t: &Theme,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text(title, 14., 21., 500., t.text))
                    .children(
                        subtitle
                            .map(|s| text(s, 13., 19., 430., t.text_secondary).whitespace_normal()),
                    ),
            )
            .child(
                div()
                    .relative()
                    .p(px(1.))
                    .child(deco(
                        &self.layer,
                        20.,
                        shapes::Corner::Squircle,
                        t.background_panel,
                        Some(t.border),
                    ))
                    .child(content),
            )
    }
}

impl Render for Usage {
    fn render(&mut self, window: &mut AppWindow, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        if state.revision != self.seen_revision {
            self.seen_revision = state.revision;
            if state.notice.is_some() {
                self.reload_open = false;
                self.request_open = false;
                self.confirm_reset = None;
            }
        }
        let t = cx.theme().clone();
        let mut groups = div()
            .w_full()
            .max_w(px(768.))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(40.));
        if let Some(error) = &state.error {
            groups =
                groups.child(text(error.clone(), 13., 19., 430., t.text_error).whitespace_normal());
        }
        if let Some(notice) = &state.notice {
            groups = groups.child(text(notice.clone(), 13., 19., 430., t.text));
        }
        if !state.access.visible {
            let message = match &state.limits {
                Load::Failed(message) => message.as_str(),
                _ => "Loading usage settings…",
            };
            groups = groups.child(self.status(
                message,
                (!state.unavailable).then_some(Action::Refresh),
                window,
                cx,
            ));
        } else if self.analytics && state.access.analytics {
            groups = groups.child(match &state.analytics {
                Load::Ready(value) => {
                    let summary = &value["summary"];
                    let mut data = div().flex().flex_col().gap(px(16.));
                    for (key, label) in [
                        ("lifetimeTokens", "Lifetime tokens"),
                        ("peakDailyTokens", "Peak daily tokens"),
                        ("longestRunningTurnSec", "Longest running turn"),
                        ("currentStreakDays", "Current streak"),
                        ("longestStreakDays", "Longest streak"),
                    ] {
                        if let Some(value) = summary[key].as_u64() {
                            data = data.child(text(
                                format!("{label}: {value}"),
                                14.,
                                21.,
                                430.,
                                t.text,
                            ));
                        }
                    }
                    data
                }
                Load::Loading => self.status("Loading usage settings…", None, window, cx),
                Load::Failed(_) => self.status(
                    "Could not load usage settings.",
                    Some(Action::Refresh),
                    window,
                    cx,
                ),
            });
        } else {
            if let Some(plan) = self.plan(&state, window, cx) {
                groups = groups.child(plan);
            }
            groups = groups.child(self.limits(&state, window, cx));
            if state.access.balance_history {
                groups = groups.child(self.history(&state, window, cx));
            }
            if let Some(credits) = self.credits(&state, window, cx) {
                groups = groups.child(credits);
            }
            if !state.access.enterprise {
                groups = groups.child(self.resets(&state, window, cx));
            }
        }
        let mut title = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .py(px(12.))
            .min_h(px(60.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text("Usage & billing", 28., 33.6, 500., t.text));
        if state.access.analytics {
            title = title.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(self.action(
                        "usage-overview",
                        "Overview",
                        Control::Tab(false),
                        false,
                        window,
                        cx,
                    ))
                    .child(self.action(
                        "usage-analytics",
                        "Analytics",
                        Control::Tab(true),
                        false,
                        window,
                        cx,
                    )),
            );
        }
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(focus::tab)
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("usage-settings-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
                            .pt(px(16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(title)
                            .child(groups),
                    ),
            )
            .when(self.reload_open, |d| {
                d.child(self.reload_dialog(&state, window, cx))
            })
    }
}

fn row(title: &str, detail: Option<&str>, control: AnyElement, t: &Theme) -> Div {
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
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(text(title.to_owned(), 13., 19., 500., t.text))
                .children(detail.map(|s| {
                    text(s.to_owned(), 12., 16., 430., t.text_secondary).whitespace_normal()
                })),
        )
        .child(control)
}

pub fn pending_request(state: &State) -> Option<&Value> {
    let Load::Ready(requests) = &state.requests else {
        return None;
    };
    pending_request_in(requests)
}

pub fn pending_request_in(requests: &Value) -> Option<&Value> {
    requests["items"].as_array()?.iter().find(|request| {
        request["status"] == "pending"
            && request["payloads"].as_array().is_some_and(|payloads| {
                payloads.iter().any(|payload| {
                    payload["kind"] == "spend_limit"
                        && payload["target"]["source"] == "account_user_spend_controls"
                        && payload["target"]["source_id"] == "account_user.credit_limits"
                })
            })
    })
}

fn date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%b %-d, %Y %-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

fn countdown(timestamp: i64) -> String {
    let minutes = ((timestamp - chrono::Utc::now().timestamp()).max(0) / 60) as u64;
    match minutes {
        0 => "Resets now".into(),
        1..=59 => format!("Resets in {minutes}m"),
        60..=1439 => format!("Resets in {}h {}m", minutes / 60, minutes % 60),
        _ => format!("Resets in {}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}
