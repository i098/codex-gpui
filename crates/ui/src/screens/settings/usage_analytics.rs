use super::{Action, State, heading};
use crate::{focus, icon::icon, screens::computer_use::Load, text};
use chrono::Datelike as _;
use gpui::{
    AnyElement, App, Context, Div, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = usage, no_json)]
pub struct OpenChat(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Usage,
    Credits,
    Plugins,
    Skills,
    Messages,
    Plan,
    Chats,
}

#[derive(Clone, Copy, Debug)]
pub struct Query {
    pub metric: Metric,
    pub days: i64,
    pub offset_days: i64,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub usage: Load<Arc<Value>>,
    pub credits: Load<Arc<Value>>,
    pub plugins: Load<Arc<Value>>,
    pub skills: Load<Arc<Value>>,
    pub messages: Load<Arc<Value>>,
    pub plan: Load<Arc<Value>>,
    pub chats: Load<Arc<Value>>,
    pub token_usage: Load<Arc<Value>>,
}

impl Snapshot {
    pub fn set(&mut self, metric: Metric, value: Load<Arc<Value>>) {
        *match metric {
            Metric::Usage => &mut self.usage,
            Metric::Credits => &mut self.credits,
            Metric::Plugins => &mut self.plugins,
            Metric::Skills => &mut self.skills,
            Metric::Messages => &mut self.messages,
            Metric::Plan => &mut self.plan,
            Metric::Chats => &mut self.chats,
        } = value;
    }
}

#[derive(Clone, Copy, Debug)]
enum Control {
    Period(Metric, i64),
    Move(i64),
    Menu(Metric),
    Group(Metric, &'static str),
    Retry(Metric),
    ShowChats,
}

pub(super) struct Analytics {
    usage_days: i64,
    credits_days: i64,
    tools_days: i64,
    messages_days: i64,
    offset_days: i64,
    usage_group: &'static str,
    credits_group: &'static str,
    messages_group: &'static str,
    menu: Option<Metric>,
    show_chats: bool,
}

impl Analytics {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<State>(|_, cx| cx.notify()).detach();
        Self {
            usage_days: 7,
            credits_days: 7,
            tools_days: 7,
            messages_days: 7,
            offset_days: 0,
            usage_group: "feature",
            credits_group: "product",
            messages_group: "model",
            menu: None,
            show_chats: false,
        }
    }
    fn days(&self, metric: Metric) -> i64 {
        match metric {
            Metric::Usage => self.usage_days,
            Metric::Credits => self.credits_days,
            Metric::Plugins | Metric::Skills => self.tools_days,
            Metric::Messages => self.messages_days,
            Metric::Plan => 7,
            Metric::Chats => 30,
        }
    }

    fn query(&self, metric: Metric) -> Query {
        Query {
            metric,
            days: self.days(metric),
            offset_days: if metric == Metric::Usage {
                self.offset_days
            } else {
                0
            },
        }
    }

    fn fetch(&self, metric: Metric, cx: &mut App) {
        if let Some(dispatch) = cx.global::<State>().dispatch.clone() {
            dispatch(Action::Analytics(self.query(metric)), cx);
        }
    }

    fn select_period(&mut self, metric: Metric, days: i64, cx: &mut Context<Self>) {
        match metric {
            Metric::Usage => {
                self.usage_days = days;
                self.offset_days = 0;
            }
            Metric::Credits => self.credits_days = days,
            Metric::Plugins | Metric::Skills => self.tools_days = days,
            Metric::Messages => self.messages_days = days,
            _ => return,
        }
        self.fetch(metric, cx);
        cx.notify();
    }

    fn activate(&mut self, control: Control, cx: &mut Context<Self>) {
        match control {
            Control::Period(metric, days) => return self.select_period(metric, days, cx),
            Control::Move(direction) => {
                self.offset_days = (self.offset_days + direction * self.usage_days).max(0);
                self.fetch(Metric::Usage, cx);
            }
            Control::Menu(metric) => self.menu = (self.menu != Some(metric)).then_some(metric),
            Control::Group(metric, group) => {
                match metric {
                    Metric::Usage => self.usage_group = group,
                    Metric::Credits => self.credits_group = group,
                    Metric::Messages => self.messages_group = group,
                    _ => {}
                }
                self.menu = None;
            }
            Control::Retry(metric) => self.fetch(metric, cx),
            Control::ShowChats => self.show_chats = !self.show_chats,
        }
        cx.notify();
    }

    fn button(
        &self,
        label: impl Into<SharedString>,
        control: Control,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let id = SharedString::from(format!("usage-analytics-{control:?}"));
        let handle = focus::handle(id.clone(), 0, window, cx);
        let disabled = cx.global::<State>().busy;
        let t = cx.theme();
        let selected =
            !matches!(control, Control::Period(metric, days) if self.days(metric) != days);
        let period = matches!(control, Control::Period(..));
        let menu = matches!(control, Control::Menu(..));
        let compact = compact || period;
        let (height, font_size, line_height) = if compact {
            (24., 12., 16.)
        } else {
            (28., 13., 18.)
        };
        let (padding, fill, weight) = match control {
            Control::Period(..) => (8., t.surface, 500.),
            Control::Menu(..) => (13., t.text.opacity(0.03), 430.),
            _ => (9., t.text_alpha_5, 430.),
        };
        let content = div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .h(px(height))
            .px(px(padding))
            .relative()
            .flex()
            .gap(px(4.))
            .items_center()
            .justify_center()
            .when(selected, |d| {
                d.child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(12.5)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(fill),
                    border: menu.then_some((px(1.), t.border)),
                    shadows: if period {
                        vec![shapes::Shadow {
                            color: gpui::black().opacity(0.08),
                            offset: gpui::point(px(0.), px(1.)),
                            blur: px(2.),
                            spread: px(-1.),
                            inset: false,
                        }]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                }))
            })
            .child(text(
                label,
                font_size,
                line_height,
                weight,
                if selected { t.text } else { t.text_secondary },
            ))
            .when(menu, |d| {
                d.child(icon("chevron-down", 14., t.text_tertiary).runtime())
            })
            .when(disabled, |d| d.opacity(0.4));
        focus::ring(content, &handle, px(12.5), px(2.), cx)
            .on_click(cx.listener(move |this, _, _, cx| {
                if !disabled {
                    this.activate(control, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate(control, cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }

    fn period(&self, metric: Metric, window: &mut Window, cx: &mut Context<Self>) -> Div {
        div()
            .relative()
            .grid()
            .grid_cols(2)
            .gap(px(2.))
            .p(px(2.))
            .child(super::super::on_card(
                10.,
                shapes::Corner::Round,
                cx.theme().surface_secondary.opacity(0.92),
                None,
            ))
            .children([7, 30].map(|days| {
                self.button(
                    format!("{days}d"),
                    Control::Period(metric, days),
                    false,
                    window,
                    cx,
                )
            }))
    }

    fn breakdown(&self, metric: Metric, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let group = match metric {
            Metric::Usage => self.usage_group,
            Metric::Credits => self.credits_group,
            _ => self.messages_group,
        };
        let label = format!(
            "By {}",
            if group == "turn-start" {
                "turn start"
            } else {
                group
            }
        );
        let mut result =
            div()
                .relative()
                .child(self.button(label, Control::Menu(metric), false, window, cx));
        if self.menu == Some(metric) {
            let options: &[&str] = match metric {
                Metric::Usage => &["feature", "model", "surface", "turn-start"],
                Metric::Credits => &["product", "model", "speed"],
                _ => &["model", "surface"],
            };
            let children: Vec<_> = options
                .iter()
                .map(|&group| {
                    self.button(
                        format!("By {group}"),
                        Control::Group(metric, group),
                        false,
                        window,
                        cx,
                    )
                })
                .collect();
            result = result
                .child(gpui::deferred(
                    div().absolute().right_0().top(px(28.)).occlude().child(
                        crate::composer::popover(cx.theme(), 160., |d| {
                            d.flex().flex_col().children(children)
                        }),
                    ),
                ))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }));
        }
        result
    }

    fn card(&self, content: Div, t: &Theme) -> Div {
        div()
            .relative()
            .p(px(1.))
            .child(super::super::on_card(
                20.,
                shapes::Corner::Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(content)
    }

    fn chart(
        &self,
        metric: Metric,
        data: &Load<Arc<Value>>,
        state: &State,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let group = match metric {
            Metric::Usage => self.usage_group,
            Metric::Credits => self.credits_group,
            _ => self.messages_group,
        };
        let mut body = div()
            .px(px(16.))
            .pt(px(14.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.));
        if metric == Metric::Usage {
            body = body.child(self.period_navigation(state, &t, window, cx));
        } else {
            let label = match metric {
                Metric::Credits => "Credits used",
                Metric::Plugins => "Plugins called",
                Metric::Skills => "Skills used",
                _ => "Messages",
            };
            body = body.child(text(label, 12., 16., 430., t.text));
        }
        let content = match data {
            Load::Loading => div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "Loading usage data…",
                    13.,
                    18.571_428,
                    430.,
                    t.text_tertiary,
                )),
            Load::Failed(_) => div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(text(
                    "Could not load usage data",
                    13.,
                    18.571_428,
                    430.,
                    t.text_tertiary,
                ))
                .child(self.button("Retry", Control::Retry(metric), false, window, cx)),
            Load::Ready(value) => self.bars(metric, value, group, &t),
        };
        self.card(body.child(content), &t)
    }

    fn period_navigation(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut arrows = div().flex();
        for (direction, glyph, label) in [
            (-1, "usage-chevron-right", "Next period"),
            (1, "chevron-left", "Previous period"),
        ]
        .into_iter()
        .rev()
        {
            let disabled = (direction == -1 && self.offset_days == 0) || state.busy;
            let handle = focus::handle(label, 0, window, cx);
            arrows = arrows.child(
                focus::ring(
                    div()
                        .id(label)
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .size(px(32.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(disabled, |d| d.opacity(0.4))
                        .child(icon(glyph, 16., t.text).runtime()),
                    &handle,
                    px(12.5),
                    px(0.),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !disabled {
                        this.activate(Control::Move(direction), cx);
                    }
                }))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, _, cx| {
                        if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.activate(Control::Move(direction), cx);
                            cx.stop_propagation();
                        }
                    },
                )),
            );
        }
        div()
            .h(px(32.))
            .mb(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .child(text(
                date_range(state.reference_time, self.usage_days, self.offset_days),
                12.,
                16.,
                430.,
                t.text,
            ))
            .child(arrows)
    }

    fn bars(&self, metric: Metric, value: &Value, group: &str, t: &Theme) -> Div {
        if metric == Metric::Usage
            && value["data"]
                .as_array()
                .is_some_and(|data| data.iter().any(|row| row["attribution"].is_null()))
        {
            return div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "Usage breakdown is unavailable for this period",
                    13.,
                    19.,
                    430.,
                    t.text_tertiary,
                ));
        }
        let Some(points) = series(value, metric, group) else {
            return div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "Usage data is unavailable",
                    13.,
                    19.,
                    430.,
                    t.text_tertiary,
                ));
        };
        if points.is_empty() {
            return div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "No usage recorded during this period",
                    13.,
                    19.,
                    430.,
                    t.text_tertiary,
                ));
        }
        let maximum = points
            .iter()
            .map(|(_, values)| values.values().sum::<f64>())
            .fold(0., f64::max);
        if maximum <= 0. {
            return div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "No usage recorded during this period",
                    13.,
                    19.,
                    430.,
                    t.text_tertiary,
                ));
        }
        let mut bars = div().h(px(236.)).flex().items_end().gap(px(8.));
        for (date, values) in points {
            let mut bar = div().flex_1().h_full().flex().flex_col().justify_end();
            for (index, (_, amount)) in values.into_iter().enumerate() {
                let color = [t.text_accent, t.text_success, t.text_warning][index % 3];
                bar = bar.child(
                    div()
                        .w_full()
                        .h(gpui::relative(
                            (amount / maximum).clamp(0., 1.) as f32 * 0.8,
                        ))
                        .bg(color),
                );
            }
            bars = bars.child(bar.child(text(date, 10., 16., 430., t.text_secondary)));
        }
        div().h(px(260.)).child(bars)
    }

    fn status_row(
        &self,
        label: &'static str,
        metric: Metric,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let control = self.button("Retry", Control::Retry(metric), compact, window, cx);
        super::row(label, None, control, cx.theme())
    }

    fn chats(&self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let content = match &state.analytics.chats {
            Load::Loading => div().p(px(16.)).child(text(
                "Loading chat usage…",
                13.,
                18.571_428,
                430.,
                t.text_secondary,
            )),
            Load::Failed(_) => {
                self.status_row("Couldn't load chat usage", Metric::Chats, true, window, cx)
            }
            Load::Ready(value) => {
                let rows = value["rows"].as_array();
                if rows.is_none_or(Vec::is_empty) {
                    div().p(px(16.)).child(text(
                        "No recent chats on this device",
                        13.,
                        18.571_428,
                        430.,
                        t.text_secondary,
                    ))
                } else {
                    let mut table = div().flex().flex_col();
                    for row in rows.into_iter().flatten().take(if self.show_chats {
                        usize::MAX
                    } else {
                        5
                    }) {
                        let name = row["thread"]["name"]
                            .as_str()
                            .filter(|name| !name.is_empty())
                            .unwrap_or("Untitled chat");
                        let usage = &row["usage"];
                        let Some(id) = row["thread"]["id"].as_str() else {
                            continue;
                        };
                        let id = id.to_owned();
                        let key_id = id.clone();
                        let handle = focus::handle(format!("usage-chat-{id}"), 0, window, cx);
                        let amount = usage["balance_usage_credits"]
                            .as_f64()
                            .map(|amount| amount.to_string())
                            .unwrap_or_else(|| "Not available".into());
                        let item = focus::ring(
                            div()
                                .id(format!("usage-chat-{id}"))
                                .px(px(16.))
                                .py(px(8.))
                                .role(gpui::Role::Button)
                                .aria_label(name.to_owned())
                                .flex()
                                .justify_between()
                                .child(text(name.to_owned(), 13., 20., 430., t.text))
                                .child(text(amount, 13., 20., 430., t.text_secondary)),
                            &handle,
                            px(0.),
                            px(0.),
                            cx,
                        )
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(OpenChat(id.clone())), cx)
                        })
                        .on_key_down(move |event, window, cx| {
                            if event.keystroke.key == "enter" {
                                window.dispatch_action(Box::new(OpenChat(key_id.clone())), cx);
                            }
                        });
                        table = table.child(item);
                    }
                    if rows.is_some_and(|rows| rows.len() > 5) {
                        table = table.child(self.button(
                            if self.show_chats {
                                "Show less"
                            } else {
                                "Show more"
                            },
                            Control::ShowChats,
                            true,
                            window,
                            cx,
                        ));
                    }
                    table
                }
            }
        };
        div()
            .flex()
            .flex_col()
            .child(heading(
                "Top chats",
                Some(
                    text(
                        "Compare each chat’s plan and credit usage",
                        13.,
                        18.,
                        400.,
                        t.text_secondary,
                    )
                    .into_any_element(),
                ),
                16.,
                &t,
            ))
            .child(self.card(content, &t))
    }

    fn plan_history(&self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut columns = div().flex().gap(px(24.));
        for (minutes, label) in [(300, "5-hour limits"), (10080, "Weekly limits")] {
            let content = match &state.analytics.plan {
                Load::Failed(_) => self.status_row(
                    "Couldn't load limit history",
                    Metric::Plan,
                    true,
                    window,
                    cx,
                ),
                Load::Loading => div().p(px(16.)).child(text(
                    "Loading limit history…",
                    13.,
                    19.,
                    430.,
                    t.text_secondary,
                )),
                Load::Ready(value) if value.is_null() => div().p(px(16.)).child(text(
                    "Limit history isn't available yet",
                    13.,
                    19.,
                    430.,
                    t.text_secondary,
                )),
                Load::Ready(value) => {
                    let periods: Vec<_> = value["periods"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|period| period["window_minutes"] == minutes)
                        .collect();
                    let mut rows = div().flex().flex_col();
                    if periods.is_empty() {
                        rows = rows.child(
                            text(
                                "No limit periods in this date range",
                                13.,
                                19.,
                                430.,
                                t.text_secondary,
                            )
                            .p(px(16.)),
                        );
                    }
                    for period in periods {
                        let start = period["starts_at"].as_str().unwrap_or_default();
                        let end = period["ends_at"].as_str().unwrap_or_default();
                        rows = rows.child(
                            div()
                                .p(px(12.))
                                .child(text(format!("{start} – {end}"), 12., 16., 430., t.text))
                                .child(text(
                                    period["used_basis_points"]
                                        .as_f64()
                                        .map(|used| format!("{:.1}% of limit used", used / 100.))
                                        .unwrap_or_else(|| "Not available".into()),
                                    12.,
                                    16.,
                                    430.,
                                    t.text_secondary,
                                )),
                        );
                    }
                    rows
                }
            };
            columns = columns.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(text(label, 13., 19., 500., t.text))
                    .child(self.card(content, &t)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .h(px(34.))
                    .child(text("Plan usage history", 14., 21., 500., t.text)),
            )
            .child(columns)
    }
}

impl Render for Analytics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.global::<State>().clone();
        let t = cx.theme().clone();
        let history = div().flex().flex_col()
            .child(heading("Usage history", Some(text("See how plan usage and credits were consumed in Work, Codex, and other agentic tasks. Conversations in Chat are not included.", 13., 18., 400., t.text_secondary).whitespace_normal().into_any_element()), 16., &t))
            .child(div().flex().flex_col().gap(px(12.))
                .child(div().flex().justify_between().items_center().child(self.period(Metric::Usage, window, cx)).child(self.breakdown(Metric::Usage, window, cx)))
                .child(self.chart(Metric::Usage, &state.analytics.usage, &state, window, cx)));
        let credits = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(heading(
                        "Credits history",
                        Some(
                            text(
                                "Track credits usage over time",
                                13.,
                                18.,
                                400.,
                                t.text_secondary,
                            )
                            .into_any_element(),
                        ),
                        16.,
                        &t,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(self.period(Metric::Credits, window, cx))
                            .child(self.breakdown(Metric::Credits, window, cx)),
                    ),
            )
            .child(self.chart(
                Metric::Credits,
                &state.analytics.credits,
                &state,
                window,
                cx,
            ));
        let tools = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(heading(
                        "Tool activity",
                        Some(
                            text(
                                "See which plugins and skills were used over time",
                                13.,
                                18.,
                                400.,
                                t.text_secondary,
                            )
                            .into_any_element(),
                        ),
                        16.,
                        &t,
                    ))
                    .child(self.period(Metric::Plugins, window, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(self.chart(
                        Metric::Plugins,
                        &state.analytics.plugins,
                        &state,
                        window,
                        cx,
                    ))
                    .child(self.chart(Metric::Skills, &state.analytics.skills, &state, window, cx)),
            );
        let messages = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(heading(
                        "Messages",
                        Some(
                            text(
                                "Track the number of messages sent over time",
                                13.,
                                18.,
                                400.,
                                t.text_secondary,
                            )
                            .into_any_element(),
                        ),
                        16.,
                        &t,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(self.period(Metric::Messages, window, cx))
                            .child(self.breakdown(Metric::Messages, window, cx)),
                    ),
            )
            .child(self.chart(
                Metric::Messages,
                &state.analytics.messages,
                &state,
                window,
                cx,
            ));
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(history)
            .child(self.chats(&state, window, cx))
            .child(self.plan_history(&state, window, cx))
            .child(credits)
            .child(tools)
            .child(messages)
    }
}

pub fn date_range(now: Option<i64>, days: i64, offset: i64) -> String {
    let end =
        chrono::DateTime::from_timestamp(now.unwrap_or_else(|| chrono::Utc::now().timestamp()), 0)
            .unwrap_or_else(chrono::Utc::now)
            .date_naive()
            - chrono::Duration::days(offset);
    let start = end - chrono::Duration::days(days - 1);
    if start.month() == end.month() && start.year() == end.year() {
        format!("{} – {}", start.format("%b %-d"), end.format("%-d, %Y"))
    } else {
        format!("{} – {}", start.format("%b %-d"), end.format("%b %-d, %Y"))
    }
}

fn attribution_field(metric: Metric, group: &str) -> &'static str {
    match metric {
        Metric::Usage => "attribution",
        Metric::Plugins => "plugin_usage_overviews",
        Metric::Skills => "skill_usage_overviews",
        Metric::Messages if group != "model" => "clients",
        _ => "models",
    }
}

fn attribution<'a>(item: &'a Value, metric: Metric, group: &str) -> Option<(&'a str, f64)> {
    let key = match metric {
        Metric::Usage => match group {
            "model" => item["model"].as_str(),
            "surface" => item["surface"].as_str(),
            "turn-start" => item["turn_trigger"].as_str(),
            _ => item["thread_source"].as_str(),
        },
        Metric::Plugins | Metric::Skills => item["display_name"].as_str(),
        Metric::Messages if group != "model" => item["client_id"].as_str(),
        Metric::Credits if group == "speed" => Some(item["speed"].as_str().unwrap_or("unknown")),
        _ => item["model"].as_str(),
    }
    .unwrap_or("Other");
    let amount = match metric {
        Metric::Usage => item["value"].as_f64()?,
        Metric::Plugins | Metric::Skills => item["invocation_counts"].as_f64()?,
        Metric::Messages => item["turns"].as_f64().unwrap_or(0.),
        _ => item["on_demand_credits"].as_f64().unwrap_or(0.),
    };
    amount.is_finite().then_some((key, amount))
}

fn series(
    value: &Value,
    metric: Metric,
    group: &str,
) -> Option<Vec<(String, BTreeMap<String, f64>)>> {
    value["data"]
        .as_array()?
        .iter()
        .map(|bucket| {
            let date = bucket["date"].as_str()?.to_owned();
            let mut values = BTreeMap::new();
            if let Some(raw) = bucket["values"].as_object() {
                for (key, amount) in raw {
                    values.insert(key.clone(), amount.as_f64()?);
                }
            }
            if metric == Metric::Credits
                && group == "product"
                && let Some(raw) =
                    bucket["premium_usage_values"]["credit_usage_credits"].as_object()
            {
                for (key, amount) in raw {
                    *values.entry(key.clone()).or_insert(0.) += amount.as_f64()?;
                }
            }
            let field = attribution_field(metric, group);
            if metric == Metric::Usage && bucket[field].is_null() {
                return None;
            }
            for item in bucket[field].as_array().into_iter().flatten() {
                let (key, amount) = attribution(item, metric, group)?;
                *values.entry(key.to_owned()).or_insert(0.) += amount;
            }
            if metric == Metric::Credits && !bucket["credit_amount"].is_null() {
                *values
                    .entry(bucket["product_surface"].as_str().unwrap_or("Other").into())
                    .or_insert(0.) += bucket["credit_amount"].as_f64()?;
            }
            if values
                .values()
                .any(|value| !value.is_finite() || *value < 0.)
            {
                return None;
            }
            Some((date, values))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attribution_grouping_preserves_reported_usage() {
        let data = json!({"data":[{"date":"2026-10-15","attribution":[
            {"model":"a","thread_source":"cli","value":3},
            {"model":"a","thread_source":"desktop","value":5}
        ]}]});
        let models = series(&data, Metric::Usage, "model").unwrap();
        assert_eq!(models[0].1, BTreeMap::from([("a".into(), 8.)]));
        let sources = series(&data, Metric::Usage, "feature").unwrap();
        assert_eq!(
            sources[0].1,
            BTreeMap::from([("cli".into(), 3.), ("desktop".into(), 5.)])
        );
        assert!(
            series(
                &json!({"data":[{"date":"2026-10-15","attribution":null}]}),
                Metric::Usage,
                "model"
            )
            .is_none()
        );
    }

    #[test]
    fn credit_speed_keeps_two_speeds_of_one_model_separate() {
        let data = json!({"data":[{"date":"2026-10-15","models":[
            {"model":"a","speed":"fast","on_demand_credits":3},
            {"model":"a","speed":"standard","on_demand_credits":5},
            {"model":"a","on_demand_credits":2}
        ]}]});
        let speeds = series(&data, Metric::Credits, "speed").unwrap();
        assert_eq!(
            speeds[0].1,
            BTreeMap::from([
                ("fast".into(), 3.),
                ("standard".into(), 5.),
                ("unknown".into(), 2.)
            ])
        );
        let models = series(&data, Metric::Credits, "model").unwrap();
        assert_eq!(models[0].1, BTreeMap::from([("a".into(), 10.)]));
    }
}
