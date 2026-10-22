#[path = "usage_model.rs"]
pub mod model;
pub use model::*;

#[path = "usage_analytics.rs"]
pub mod analytics;

use super::{Layer, deco, on_card, probe};
use crate::screens::computer_use::Load;
use crate::{
    composer::{Composer, popover},
    focus, text,
};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Div, Entity, Global,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window as AppWindow, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    History(i64),
    Analytics(analytics::Query),
    Reset(Option<String>),
    SaveReload(ReloadDraft),
    DisableReload,
    RequestMore(String),
    SaveAppLimits(BTreeMap<String, u8>),
    SetAppCreditPreference(bool),
    SetAgentResetEnabled(bool),
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
    pub analytics: analytics::Snapshot,
    pub initial_analytics: bool,
    pub monthly: Load<Arc<Value>>,
    pub request_policy: Load<Arc<Value>>,
    pub requests: Load<Arc<Value>>,
    pub app_limits: Load<Arc<Value>>,
    pub app_credit_preference: Load<Arc<Value>>,
    pub agent_reset_enabled: bool,
    pub plan_origin: Option<String>,
    pub reference_time: Option<i64>,
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
    AppLimitMenu(String),
    AppLimitValue(String, u8),
    AppLimitToggle(String, u8),
    SaveAppLimits,
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
            let analytics_view = cx.new(analytics::Analytics::new);
            Usage {
                layer: Layer::default(),
                analytics: cx.global::<State>().initial_analytics,
                analytics_view,
                scroll: gpui::ScrollHandle::new(),
                reload_open: false,
                disable_confirmation: false,
                fields,
                request_open: false,
                confirm_reset: None,
                reset_history: false,
                field_error: None,
                seen_revision: 0,
                app_limit_values: BTreeMap::new(),
                app_limit_menu: None,
                number_features: gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)])),
            }
        })
        .into_any_element()
}

struct Usage {
    layer: Layer,
    analytics: bool,
    analytics_view: Entity<analytics::Analytics>,
    scroll: gpui::ScrollHandle,
    reload_open: bool,
    disable_confirmation: bool,
    fields: [Entity<Composer>; 4],
    request_open: bool,
    confirm_reset: Option<Option<String>>,
    reset_history: bool,
    field_error: Option<String>,
    seen_revision: u64,
    app_limit_values: BTreeMap<String, (u8, u8)>,
    app_limit_menu: Option<String>,
    number_features: gpui::FontFeatures,
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
            Control::EditReload
            | Control::CloseReload
            | Control::SaveReload
            | Control::ConfirmDisable => self.reload_control(control, cx),
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
            Control::AppLimitMenu(_)
            | Control::AppLimitValue(..)
            | Control::AppLimitToggle(..)
            | Control::SaveAppLimits => self.app_limit_control(control, cx),
        }
        cx.notify();
    }

    fn reload_control(&mut self, control: Control, cx: &mut Context<Self>) {
        match control {
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
            _ => unreachable!(),
        }
    }

    fn app_limit_control(&mut self, control: Control, cx: &mut Context<Self>) {
        match control {
            Control::AppLimitMenu(id) => {
                self.app_limit_menu = (self.app_limit_menu.as_ref() != Some(&id)).then_some(id);
            }
            Control::AppLimitValue(id, value) => {
                self.app_limit_values.entry(id).or_insert((value, 10)).0 = value;
                self.app_limit_menu = None;
            }
            Control::AppLimitToggle(id, current) => {
                let entry = self.app_limit_values.entry(id).or_insert((current, 10));
                if current == 0 {
                    entry.0 = entry.1;
                } else {
                    *entry = (0, current);
                }
            }
            Control::SaveAppLimits => {
                if let Load::Ready(apps) = &cx.global::<State>().app_limits {
                    let values = apps["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|app| {
                            let id = app["id"].as_str()?;
                            let value =
                                self.app_limit_values.get(id).map(|entry| entry.0).or_else(
                                    || {
                                        app["allowed_usage_percent"]
                                            .as_u64()
                                            .and_then(|value| u8::try_from(value).ok())
                                    },
                                )?;
                            Some((id.to_owned(), value))
                        })
                        .collect();
                    self.dispatch(Action::SaveAppLimits(values), cx);
                }
            }
            _ => unreachable!(),
        }
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
        let selected = !matches!(&control, Control::Tab(value) if *value != self.analytics);
        let tab = matches!(&control, Control::Tab(_));
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .relative()
                .h(px(28.))
                .px(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .child(on_card(
                    12.5,
                    shapes::Corner::Squircle,
                    if selected {
                        t.text_alpha_5
                    } else {
                        gpui::transparent_black()
                    },
                    None,
                ))
                .child(text(
                    label,
                    13.,
                    18.,
                    430.,
                    if tab && !selected {
                        t.text_tertiary
                    } else {
                        t.text
                    },
                ))
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
            Control::Action(Action::OpenUrl(
                if state.access.logged_in_pricing {
                    "https://chatgpt.com/#pricing"
                } else {
                    "https://chatgpt.com/pricing/"
                }
                .into(),
            )),
            false,
            window,
            cx,
        );
        if state.plan == "free" {
            return Some(self.group(
                "Your plan",
                None,
                row(name, None, control, cx.theme()),
                cx.theme(),
            ));
        }
        let (prefix, link, suffix, url) = match state.plan_origin.as_deref() {
            Some("ios") => (
                "Your subscription is managed through your Apple account. You'll need to ",
                "cancel via iOS",
                "",
                "https://help.openai.com/en/articles/7905690-how-to-cancel-your-apple-subscription-for-chatgpt-in-the-chatgpt-ios-app",
            ),
            Some("android") => (
                "Your subscription is managed through your Google Play account. You'll need to ",
                "cancel via Android",
                "",
                "https://help.openai.com/en/articles/8258076-how-to-cancel-a-subscription-in-the-chatgpt-android-app",
            ),
            _ => (
                "Your subscription is managed through ChatGPT. Go to ",
                "Settings > Billing on Web",
                " to manage or cancel your plan",
                "https://chatgpt.com/#settings/Billing",
            ),
        };
        let subtitle = div()
            .flex()
            .child(text(prefix, 13., 18., 400., cx.theme().text_secondary))
            .child(
                text(link, 13., 18., 400., cx.theme().text_accent)
                    .id("usage-manage-billing")
                    .on_click(move |_, _, cx| cx.open_url(url)),
            )
            .child(text(suffix, 13., 18., 400., cx.theme().text_secondary));
        Some(
            div()
                .flex()
                .flex_col()
                .child(heading(
                    "Your plan",
                    Some(subtitle.into_any_element()),
                    16.,
                    cx.theme(),
                ))
                .child(self.card(row(name, None, control, cx.theme()), cx.theme())),
        )
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
                        content = content.child(self.meter(limit, index, state.reference_time, &t));
                    }
                }
            }
        }
        if state.access.enterprise {
            content = content.child(self.enterprise(state, window, cx));
        }
        let panel = shapes::decoration(shapes::Decoration {
            radius: gpui::Corners::all(px(20.)),
            corner: shapes::Corner::Squircle,
            fill: Some(t.background_panel),
            shadows: vec![shapes::Shadow {
                color: t.border_subtle,
                offset: gpui::point(px(0.), px(0.)),
                blur: px(0.),
                spread: px(1.),
                inset: true,
            }],
            ..Default::default()
        });
        div().flex().flex_col()
            .child(heading("Plan limits", Some(text("Shared across Codex, Work, Workspace Agents, and ChatGPT for Excel. Chat conversations are not included.", 13., 18., 400., t.text_secondary).into_any_element()), 16., &t))
            .child(div().relative().child(panel).child(content))
    }

    fn meter(&self, limit: &model::Window, index: usize, now: Option<i64>, t: &Theme) -> Div {
        let remaining = limit.remaining();
        let label = limit.label();
        let reset = limit
            .resets_at
            .map(|at| countdown(at, now))
            .unwrap_or_default();
        div()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(text(label, 13., 18.571_428, 500., t.text))
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
                            .child(
                                text(
                                    format!("{remaining:.0}% left"),
                                    12.,
                                    16.,
                                    430.,
                                    t.text_secondary,
                                )
                                .font_features(self.number_features.clone()),
                            ),
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
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: gpui::Corners {
                                            top_left: px(3.),
                                            bottom_left: px(3.),
                                            top_right: px(if remaining >= 100. { 3. } else { 0. }),
                                            bottom_right: px(if remaining >= 100. {
                                                3.
                                            } else {
                                                0.
                                            }),
                                        },
                                        corner: shapes::Corner::Round,
                                        fill: Some(if remaining <= 10. {
                                            t.text_warning
                                        } else {
                                            t.composer_primary
                                        }),
                                        ..Default::default()
                                    })),
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
            content = content
                .child(self.meter(&usage, 100, state.reference_time, cx.theme()))
                .child(
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
        self.enterprise_requests(content, state, window, cx)
    }

    fn enterprise_requests(
        &self,
        content: Div,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
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
        self.request_controls(content, state, policy, window, cx)
    }

    fn request_controls(
        &self,
        mut content: Div,
        state: &State,
        policy: &Value,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
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
                    "https://chatgpt.com/codex/settings/usage?credit_modal=true".into(),
                )),
                false,
                window,
                cx,
            )
        } else {
            div().into_any_element()
        };
        let content = div().flex().flex_col().child(
            div()
                .p(px(16.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(text(balance, 13., 18.571_428, 430., cx.theme().text))
                        .child(text(
                            "Current balance",
                            13.,
                            18.571_428,
                            430.,
                            cx.theme().text_secondary,
                        )),
                )
                .child(control),
        );
        Some(self.credit_content(content, state, window, cx))
    }

    fn credit_content(
        &self,
        mut content: Div,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
        if state.access.manage_credits && state.has_saved_payment_method {
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
        self.group("Credits", Some(if state.access.manage_credits {
            if state.has_saved_payment_method { "Buy credits or turn on automatic reload to continue using Work and Codex when you reach usage limits." } else { "Buy credits to continue using Work and Codex when you reach usage limits" }
        } else { "Your remaining credits" }), content, cx.theme())
    }

    fn switch(
        &self,
        (id, label): (&'static str, impl Into<SharedString>),
        checked: bool,
        control: Control,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let handle = focus::handle(label.clone(), 0, window, cx);
        let key = control.clone();
        let disabled = state.busy;
        focus::ring(
            super::switch(id, checked, disabled, cx.theme())
                .role(gpui::Role::CheckBox)
                .aria_label(label)
                .aria_toggled(if checked {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                }),
            &handle,
            px(10.),
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

    fn agent_reset(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let control = self.switch(
            ("usage-agent-reset", "Allow Codex to use resets"),
            state.agent_reset_enabled,
            Control::Action(Action::SetAgentResetEnabled(!state.agent_reset_enabled)),
            state,
            window,
            cx,
        );
        self.card(row("Allow Codex to use resets",
            Some("Give codex the ability to use a reset when you explicitly request it during a conversation, and your current limit has 10% or less remaining. Resets are never used automatically."),
            control, cx.theme()), cx.theme())
    }

    fn app_limits(
        &self,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let t = cx.theme().clone();
        let mut rows = div().flex().flex_col();
        let mut dirty = false;
        match &state.app_limits {
            Load::Ready(apps) => {
                let items = apps["items"].as_array()?;
                if items.is_empty() {
                    return None;
                }
                for app in items {
                    let id = app["id"].as_str()?;
                    let name = app["name"].as_str()?;
                    let original = u8::try_from(app["allowed_usage_percent"].as_u64()?).ok()?;
                    let value = self
                        .app_limit_values
                        .get(id)
                        .map_or(original, |entry| entry.0);
                    dirty |= value != original;
                    let mut selector = div().relative().child(self.action(
                        format!("usage-app-limit-{id}"),
                        format!("{}%", value.max(10)),
                        Control::AppLimitMenu(id.into()),
                        state.busy || value == 0,
                        window,
                        cx,
                    ));
                    if self.app_limit_menu.as_deref() == Some(id) {
                        let mut items = Vec::new();
                        for value in (10..=100).step_by(10) {
                            items.push(self.action(
                                format!("usage-app-choice-{id}-{value}"),
                                format!("{value}%"),
                                Control::AppLimitValue(id.into(), value),
                                state.busy,
                                window,
                                cx,
                            ));
                        }
                        selector = selector
                            .child(gpui::deferred(
                                div()
                                    .absolute()
                                    .top(px(28.))
                                    .right_0()
                                    .occlude()
                                    .child(popover(&t, 120., |menu| menu.children(items))),
                            ))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.app_limit_menu = None;
                                cx.notify();
                            }));
                    }
                    let toggle = self.switch(
                        (
                            "usage-app-enabled",
                            format!("Allow {name} to use plan limits"),
                        ),
                        value > 0,
                        Control::AppLimitToggle(id.into(), value),
                        state,
                        window,
                        cx,
                    );
                    rows = rows.child(
                        row_content(
                            name,
                            None,
                            div()
                                .flex()
                                .gap(px(8.))
                                .items_center()
                                .child(selector)
                                .child(toggle)
                                .into_any_element(),
                            &t,
                        )
                        .id(format!("usage-app-{id}")),
                    );
                }
            }
            Load::Loading => {
                rows = rows.child(self.status("Loading app limits…", None, window, cx))
            }
            Load::Failed(_) => {
                let retry = self.action(
                    "usage-app-limits-retry",
                    "Retry",
                    Control::Action(Action::Refresh),
                    state.busy,
                    window,
                    cx,
                );
                rows = rows.child(row("Couldn't load app limits", None, retry, &t));
            }
        }
        let header = heading(
            "App limits",
            Some(
                text(
                    "Set limits for each app’s use of your ChatGPT plan and credits.",
                    13.,
                    18.,
                    400.,
                    t.text_secondary,
                )
                .into_any_element(),
            ),
            14.,
            &t,
        );
        let header = if matches!(state.app_limits, Load::Ready(_)) {
            div()
                .flex()
                .justify_between()
                .child(header)
                .child(self.action(
                    "usage-save-app-limits",
                    "Save",
                    Control::SaveAppLimits,
                    state.busy || !dirty,
                    window,
                    cx,
                ))
        } else {
            header
        };
        let cards = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(self.card(rows, &t));
        let cards = self.app_credit_cards(cards, t, state, window, cx);
        Some(div().flex().flex_col().child(header).child(cards))
    }

    fn app_credit_cards(
        &self,
        mut cards: Div,
        t: Theme,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
        if state.access.view_credits
            && matches!(&state.app_limits, Load::Ready(apps) if !apps.is_null())
        {
            let row = match &state.app_credit_preference {
                Load::Ready(preference) => {
                    let checked =
                        preference["settings"]["can_consume_personal_credits_chatpass"] == true;
                    let toggle = self.switch(
                        (
                            "usage-app-credits",
                            "Allow other apps to use credits after reaching your usage limit",
                        ),
                        checked,
                        Control::Action(Action::SetAppCreditPreference(!checked)),
                        state,
                        window,
                        cx,
                    );
                    row(
                        "Allow other apps to use credits after reaching your usage limit",
                        Some("Applies to apps you’ve allowed to use your ChatGPT plan."),
                        toggle,
                        &t,
                    )
                }
                Load::Loading => self.status("Loading credit preference…", None, window, cx),
                Load::Failed(_) => self.status(
                    "Couldn’t load credit preference",
                    Some(Action::Refresh),
                    window,
                    cx,
                ),
            };
            cards = cards.child(self.card(row, &t));
        }
        cards
    }

    fn reset_failure(&self, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let control = self.action(
            "usage-reset-retry",
            "Retry",
            Control::Action(Action::Refresh),
            false,
            window,
            cx,
        );
        row(
            "Couldn’t load usage limit resets",
            None,
            control,
            cx.theme(),
        )
    }

    fn resets(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col();
        match &state.limits {
            Load::Loading => {
                content =
                    content.child(self.status("Loading usage limit resets…", None, window, cx))
            }
            Load::Failed(_) => {
                content = content.child(self.reset_failure(window, cx));
            }
            Load::Ready(limits) => {
                let Some(resets) = &limits.rate_limit_reset_credits else {
                    let error = self.reset_failure(window, cx);
                    return self.group(
                        "Usage limit resets",
                        Some("Use a reset to restore your 5-hour limit, weekly limit, or both"),
                        error,
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
                    content = content.child(self.reset_activity(state, window, cx));
                } else {
                    content = self.available_resets(content, resets, state, window, cx);
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

    fn reset_activity(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        match &state.reset_history {
            Load::Loading => self.status("Loading banked reset history…", None, window, cx),
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
                        text(time.to_owned(), 12., 16., 430., cx.theme().text_secondary)
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
        }
    }

    fn available_resets(
        &self,
        mut content: Div,
        resets: &ResetCredits,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
        if resets.available_count == 0 {
            return content.child(self.status("No resets available", None, window, cx));
        }
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
        content
    }

    fn history_controls(
        &self,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
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
        controls
    }

    fn history(&self, state: &State, window: &mut AppWindow, cx: &mut Context<Self>) -> Div {
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(self.history_controls(state, window, cx));
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
        self.reload_overlay(body, enabled, state, window, cx)
    }

    fn reload_overlay(
        &self,
        mut body: Div,
        enabled: bool,
        state: &State,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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

    fn groups(
        &self,
        state: &State,
        t: &Theme,
        window: &mut AppWindow,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut groups = div().w_full().pt(px(20.)).flex().flex_col().gap(px(40.));
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
            groups = groups.child(self.analytics_view.clone());
        } else {
            if let Some(plan) = self.plan(state, window, cx) {
                groups = groups.child(plan);
            }
            groups = groups.child(self.limits(state, window, cx));
            if state.access.balance_history {
                groups = groups.child(self.history(state, window, cx));
            }
            if let Some(credits) = self.credits(state, window, cx) {
                groups = groups.child(credits);
            }
            if state.access.app_limits
                && let Some(apps) = self.app_limits(state, window, cx)
            {
                groups = groups.child(apps);
            }
            if !state.access.enterprise {
                groups = groups.child(self.agent_reset(state, window, cx));
                groups = groups.child(self.resets(state, window, cx));
            }
        }
        groups
    }

    fn group(
        &self,
        title: &'static str,
        subtitle: Option<&'static str>,
        content: Div,
        t: &Theme,
    ) -> Div {
        let subtitle = subtitle.map(|s| {
            text(s, 13., 18., 400., t.text_secondary)
                .whitespace_normal()
                .into_any_element()
        });
        div()
            .flex()
            .flex_col()
            .child(heading(title, subtitle, 16., t))
            .child(self.card(content, t))
    }

    fn card(&self, content: Div, t: &Theme) -> Div {
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
            .child(content)
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
                self.app_limit_values.clear();
                self.app_limit_menu = None;
            }
        }
        let t = cx.theme().clone();
        let groups = self.groups(&state, &t, window, cx);
        let title = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .py(px(12.))
            .min_h(px(60.))
            .flex()
            .flex_col()
            .justify_center()
            .child(text("Usage & billing", 28., 33.6, 500., t.text));
        let mut body = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .pb(px(20.))
            .flex()
            .flex_col();
        if state.access.analytics {
            let offset = (-self.scroll.offset().y).min(self.scroll.max_offset().y);
            let shift = (offset - px(77.)).max(px(0.));
            let tabs = div()
                .relative()
                .top(shift)
                .top_layer()
                .child(on_card(0., shapes::Corner::Round, t.surface, None))
                .pt(px(20.))
                .pb(px(8.))
                .flex()
                .gap(px(2.))
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
                ))
                .child(gpui::deferred(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(-32.))
                        .h(px(32.))
                        .bg(gpui::linear_gradient(
                            180.,
                            gpui::linear_color_stop(t.surface, 0.),
                            gpui::linear_color_stop(t.surface.opacity(0.), 1.),
                        )),
                ));
            body = body.child(gpui::deferred(tabs));
        }
        body = body.child(groups);
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
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
                            .pt(px(16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .justify_center()
                                    .border_b_1()
                                    .border_color(gpui::transparent_black())
                                    .child(title),
                            )
                            .child(body),
                    ),
            )
            .when(self.reload_open, |d| {
                d.child(self.reload_dialog(&state, window, cx))
            })
    }
}

fn row(title: &str, detail: Option<&str>, control: AnyElement, t: &Theme) -> Div {
    row_content(
        title,
        detail,
        div()
            .min_w(px(160.))
            .flex()
            .items_center()
            .justify_end()
            .child(control)
            .into_any_element(),
        t,
    )
}

fn row_content(title: &str, detail: Option<&str>, control: AnyElement, t: &Theme) -> Div {
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
                .flex_col()
                .gap(px(2.))
                .child(text(title.to_owned(), 13., 18.571_428, 500., t.text))
                .children(detail.map(|s| {
                    text(s.to_owned(), 12., 16., 430., t.text_secondary).whitespace_normal()
                })),
        )
        .child(control)
}

fn heading(title: &'static str, subtitle: Option<AnyElement>, size: f32, t: &Theme) -> Div {
    div()
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(text(
            title,
            size,
            if size == 16. { 24.888_89 } else { 21. },
            500.,
            t.text,
        ))
        .children(subtitle)
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

fn countdown(timestamp: i64, now: Option<i64>) -> String {
    let minutes =
        ((timestamp - now.unwrap_or_else(|| chrono::Utc::now().timestamp())).max(0) / 60) as u64;
    match minutes {
        0 => "Resets now".into(),
        1..=59 => format!("Resets in {minutes}m"),
        60..=1439 => format!("Resets in {}h {}m", minutes / 60, minutes % 60),
        _ => format!("Resets in {}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}
