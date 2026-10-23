use crate::{
    composer::{Composer, Dismiss, ModifiedSubmit},
    icon::icon,
    text,
    tooltip::{Side, tooltip},
};
use bridge::types::{Model, Thread};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, Role,
    ScrollAnchor, ScrollHandle, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::collections::HashMap;
use theme::{ActiveTheme as _, Theme};

mod execution;
use execution::Execution;

const MODES: &[&str] = &[
    "Interval", "Daily", "Weekdays", "Weekly", "Monthly", "Custom",
];
const FREQUENCIES: &[&str] = &["Hourly", "Daily", "Weekly", "Monthly", "Yearly"];
const DAYS: &[&str] = &["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
const DAY_LABELS: &[&str] = &[
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const TIMES: &[&str] = &["Morning", "Afternoon", "Evening", "Night"];
const CLOCKS: &[&str] = &["09:00", "14:00", "18:00", "22:00"];
const NAME: usize = 0;
const PROMPT: usize = 1;
const TIME: usize = 2;
const INTERVAL: usize = 3;
const MONTH_DAYS: usize = 4;
const MONTH: usize = 5;
const MINUTE: usize = 6;
const ADVANCED: usize = 5;
const CLOSE: usize = 6;
const CANCEL: usize = 7;
const SAVE: usize = 8;
const TRIGGER: usize = 9;
const NOTIFICATIONS: usize = 10;
const NEW_CHAT: usize = 11;
const AUTO_ARCHIVE: usize = 12;
const PROJECT: usize = 13;
const MODEL: usize = 14;
const EFFORT: usize = 15;
const THREAD: usize = 16;
static FORM_BODY_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| textsys::letter_spacing(-0.15));
static FORM_TITLE_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| textsys::letter_spacing(-0.32));
static FORM_TABULAR_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| {
        let mut features = FORM_BODY_FEATURES.0.as_ref().clone();
        features.push(("tnum".into(), 1));
        gpui::FontFeatures(std::sync::Arc::new(features))
    });

#[derive(Clone, Debug)]
pub struct Draft {
    pub name: String,
    pub prompt: String,
    pub rrule: String,
    pub execution: Value,
}

pub enum Outcome {
    Dismiss,
    Save(Draft),
}

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Schedule,
    Frequency,
    Unit,
    Time,
    Trigger,
    Days,
    Notifications,
    Project,
    Model,
    Effort,
    Thread,
}

pub struct Editor {
    fields: [Entity<Composer>; 7],
    controls: [FocusHandle; 17],
    scroll: ScrollHandle,
    field_anchors: [ScrollAnchor; 7],
    control_anchors: [ScrollAnchor; 17],
    mode: Option<usize>,
    frequency: usize,
    minutes: bool,
    weekdays: [bool; 7],
    menu: Option<(Picker, usize)>,
    original_rule: Option<String>,
    schedule_dirty: bool,
    raw_rule: bool,
    editing: bool,
    saving: bool,
    error: Option<String>,
    execution: Execution,
    advanced: bool,
    zone: String,
    day_label: SharedString,
}

impl EventEmitter<Outcome> for Editor {}
impl Focusable for Editor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.fields[NAME].focus_handle(cx)
    }
}

impl Editor {
    pub fn new(item: Option<&Value>, cx: &mut Context<Self>) -> Self {
        let zone = crate::dialog::local_zone();
        let default_time = clock_label(9, 0, &zone);
        let values = [
            item.and_then(|v| v["name"].as_str()).unwrap_or(""),
            item.and_then(|v| v["prompt"].as_str()).unwrap_or(""),
            &default_time,
            "1",
            "1",
            "1",
            "0",
        ];
        let placeholders = ["Task name", "Instructions…", "Time", "1", "1, 15", "1", "0"];
        let fields = std::array::from_fn(|i| {
            cx.new(|cx| {
                if i <= PROMPT {
                    Composer::form_field(placeholders[i], values[i], i == PROMPT, cx)
                } else {
                    Composer::inline_field(placeholders[i], values[i], cx)
                }
            })
        });
        let scroll = ScrollHandle::new();
        let mut this = Self {
            fields,
            controls: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            field_anchors: std::array::from_fn(|_| ScrollAnchor::for_handle(scroll.clone())),
            control_anchors: std::array::from_fn(|_| ScrollAnchor::for_handle(scroll.clone())),
            scroll,
            mode: None,
            frequency: 1,
            minutes: false,
            weekdays: [true, false, false, false, false, false, false],
            menu: None,
            original_rule: item.and_then(|v| v["rrule"].as_str()).map(str::to_owned),
            schedule_dirty: false,
            raw_rule: false,
            editing: item.is_some_and(|item| item["id"].is_string()),
            saving: false,
            error: None,
            execution: Execution::new(item),
            advanced: false,
            zone,
            day_label: "Monday".into(),
        };
        if let Some(rule) = this.original_rule.clone() {
            this.load_rule(&rule, cx);
        }
        for (i, field) in this.fields.iter().enumerate() {
            let mut previous = field.read(cx).field_text().to_owned();
            cx.observe(field, move |this: &mut Self, field, cx| {
                let value = field.read(cx).field_text();
                if previous != value {
                    previous = value.to_owned();
                    this.schedule_dirty |= i >= TIME;
                    this.error = None;
                }
                cx.notify();
            })
            .detach();
        }
        this
    }

    pub fn set_options(
        &mut self,
        models: &[Model],
        threads: &[Thread],
        projects: Option<&[crate::screens::sidebar::Project]>,
        config: &Value,
        cwd: &str,
        cx: &mut Context<Self>,
    ) {
        let (models_changed, projects_changed, threads_changed) = self
            .execution
            .set_options(models, threads, projects, config, cwd);
        if self.menu.is_some_and(|(picker, _)| match picker {
            Picker::Model | Picker::Effort => models_changed,
            Picker::Project => projects_changed,
            Picker::Thread => threads_changed,
            _ => false,
        }) {
            self.menu = None;
        }
        cx.notify();
    }

    fn load_rule(&mut self, rule: &str, cx: &mut Context<Self>) {
        let parts: HashMap<_, _> = rule
            .trim_start_matches("RRULE:")
            .split(';')
            .filter_map(|s| s.split_once('='))
            .collect();
        let frequency = parts.get("FREQ").copied().unwrap_or("");
        self.raw_rule = Self::has_uneditable_fields(rule, &parts);
        self.frequency = ["HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"]
            .iter()
            .position(|s| *s == frequency)
            .unwrap_or(1);
        self.minutes = frequency == "MINUTELY";
        let interval = parts.get("INTERVAL").copied().unwrap_or("1");
        let weekdays = parts.get("BYDAY").copied().unwrap_or("MO");
        self.weekdays = std::array::from_fn(|i| weekdays.split(',').any(|day| day == DAYS[i]));
        self.update_day_label();
        self.mode = Some(self.mode_for_rule(&parts));
        for (index, value) in [
            (INTERVAL, interval.to_owned()),
            (
                TIME,
                clock_label(
                    parts
                        .get("BYHOUR")
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(9),
                    parts
                        .get("BYMINUTE")
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(0),
                    &self.zone,
                ),
            ),
            (
                MONTH_DAYS,
                parts.get("BYMONTHDAY").copied().unwrap_or("1").to_owned(),
            ),
            (
                MONTH,
                parts.get("BYMONTH").copied().unwrap_or("1").to_owned(),
            ),
            (
                MINUTE,
                parts.get("BYMINUTE").copied().unwrap_or("0").to_owned(),
            ),
        ] {
            self.fields[index].update(cx, |field, cx| field.set_text(&value, cx));
        }
    }

    fn has_uneditable_fields(rule: &str, parts: &HashMap<&str, &str>) -> bool {
        let frequency = parts.get("FREQ").copied().unwrap_or("");
        let rule_fields: &[&str] = match frequency {
            "MINUTELY" => &[],
            "HOURLY" => &["BYMINUTE"],
            "DAILY" => &["BYHOUR", "BYMINUTE"],
            "WEEKLY" => &["BYDAY", "BYHOUR", "BYMINUTE"],
            "MONTHLY" => &["BYMONTHDAY", "BYHOUR", "BYMINUTE"],
            "YEARLY" => &["BYMONTH", "BYMONTHDAY", "BYHOUR", "BYMINUTE"],
            _ => &[],
        };
        // Keep uneditable fields verbatim; BYSECOND=0 can still change heartbeat timing.
        rule.contains('\n')
            || parts
                .keys()
                .any(|key| !["FREQ", "INTERVAL"].contains(key) && !rule_fields.contains(key))
            || rule_fields.iter().any(|key| !parts.contains_key(key))
            || !["MINUTELY", "HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"].contains(&frequency)
            || parts
                .get("BYDAY")
                .is_some_and(|days| days.split(',').any(|day| !DAYS.contains(&day)))
            || ["BYHOUR", "BYMINUTE", "BYMONTH"]
                .iter()
                .any(|key| parts.get(key).is_some_and(|s| s.contains(',')))
    }

    fn mode_for_rule(&self, parts: &HashMap<&str, &str>) -> usize {
        let frequency = parts.get("FREQ").copied().unwrap_or("");
        let interval = parts.get("INTERVAL").copied().unwrap_or("1");
        let weekdays = parts.get("BYDAY").copied().unwrap_or("MO");
        if self.raw_rule {
            return 5;
        }
        if self.minutes || frequency == "HOURLY" && parts.get("BYMINUTE") == Some(&"0") {
            return 0;
        }
        if frequency == "HOURLY" || interval != "1" || frequency == "YEARLY" {
            return 5;
        }
        match frequency {
            "DAILY" => 1,
            "WEEKLY" if weekdays == "MO,TU,WE,TH,FR" => 2,
            "WEEKLY" => 3,
            "MONTHLY" => 4,
            _ => 5,
        }
    }

    pub fn saving(&self) -> bool {
        self.saving
    }
    pub fn failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(message);
        cx.notify();
    }
    fn value<'a>(&'a self, index: usize, cx: &'a App) -> &'a str {
        self.fields[index].read(cx).field_text().trim()
    }
    fn number(&self, index: usize, min: u16, max: u16, cx: &App) -> Option<u16> {
        self.value(index, cx)
            .parse::<u16>()
            .ok()
            .filter(|n| (min..=max).contains(n))
    }

    fn fields_error(&self, cx: &App) -> Option<&'static str> {
        for (index, limit, missing, too_long) in [
            (NAME, 250, "Enter a name", "Name is too long"),
            (PROMPT, 100_000, "Enter a prompt", "Prompt is too long"),
        ] {
            let value = self.value(index, cx);
            if value.is_empty() {
                return Some(missing);
            }
            if value.chars().nth(limit).is_some() {
                return Some(too_long);
            }
        }
        None
    }

    pub fn draft(&self, cx: &App) -> Option<Draft> {
        let name = self.value(NAME, cx);
        let prompt = self.value(PROMPT, cx);
        if self.fields_error(cx).is_some() || !self.execution.valid() {
            return None;
        }
        Some(Draft {
            name: name.into(),
            prompt: prompt.into(),
            rrule: self.rule(cx)?,
            execution: if self.editing {
                self.execution.changes()
            } else {
                self.execution.creation()
            },
        })
    }

    fn rule(&self, cx: &App) -> Option<String> {
        if !self.schedule_dirty
            && let Some(rule) = &self.original_rule
        {
            return Some(rule.clone());
        }
        let mode = self.mode?;
        if self.raw_rule {
            return self.original_rule.clone();
        }
        let interval = if matches!(mode, 0 | 5) {
            self.number(INTERVAL, 1, 999, cx)?
        } else {
            1
        };
        if mode == 0 {
            return Some(format!(
                "FREQ={};INTERVAL={interval}{}",
                if self.minutes { "MINUTELY" } else { "HOURLY" },
                if self.minutes { "" } else { ";BYMINUTE=0" }
            ));
        }
        self.calendar_rule(mode, interval, cx)
    }

    fn calendar_rule(&self, mode: usize, interval: u16, cx: &App) -> Option<String> {
        let frequency = if mode == 5 {
            self.frequency
        } else {
            *[1, 2, 2, 3].get(mode.checked_sub(1)?)?
        };
        let mut rule = format!(
            "FREQ={};INTERVAL={}",
            ["HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"][frequency],
            if mode == 5 { interval } else { 1 }
        );
        if frequency == 2 {
            let days: Vec<_> = DAYS
                .iter()
                .enumerate()
                .filter(|(i, _)| if mode == 2 { *i < 5 } else { self.weekdays[*i] })
                .map(|(_, day)| *day)
                .collect();
            if days.is_empty() {
                return None;
            }
            rule.push_str(&format!(";BYDAY={}", days.join(",")));
        }
        if frequency >= 3 {
            let days = self.value(MONTH_DAYS, cx);
            if days.is_empty()
                || !days
                    .split(',')
                    .all(|s| s.trim().parse::<u8>().is_ok_and(|n| (1..=31).contains(&n)))
            {
                return None;
            }
            rule.push_str(&format!(
                ";BYMONTHDAY={}",
                days.split(',').map(str::trim).collect::<Vec<_>>().join(",")
            ));
        }
        if frequency == 4 {
            rule.push_str(&format!(";BYMONTH={}", self.number(MONTH, 1, 12, cx)?));
        }
        if frequency == 0 {
            rule.push_str(&format!(";BYMINUTE={}", self.number(MINUTE, 0, 59, cx)?));
        } else {
            let (hour, minute) = clock_parts(self.value(TIME, cx), &self.zone)?;
            rule.push_str(&format!(";BYHOUR={hour};BYMINUTE={minute}"));
        }
        Some(rule)
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.menu.take().is_none() {
            cx.emit(Outcome::Dismiss);
        }
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if let Some(draft) = self.draft(cx) {
            self.saving = true;
            self.menu = None;
            self.controls[SAVE].focus(window, cx);
            cx.emit(Outcome::Save(draft));
            cx.notify();
        }
    }

    fn options(picker: Picker) -> &'static [&'static str] {
        match picker {
            Picker::Schedule => MODES,
            Picker::Frequency => FREQUENCIES,
            Picker::Unit => &["hours", "minutes"],
            Picker::Time => TIMES,
            Picker::Trigger => &["Schedule", "Date and time"],
            Picker::Days => DAY_LABELS,
            Picker::Notifications => &["All runs", "Failed runs only"],
            _ => &[],
        }
    }

    fn option_count(&self, picker: Picker) -> usize {
        match picker {
            Picker::Model => self.execution.model_count(),
            Picker::Effort => self.execution.effort_count(),
            Picker::Project => self.execution.project_count(),
            Picker::Thread => self.execution.thread_count(),
            _ => Self::options(picker).len(),
        }
    }

    fn option_label(&self, picker: Picker, index: usize) -> SharedString {
        match picker {
            Picker::Model => self.execution.model_label(index),
            Picker::Effort => self.execution.effort_label(index),
            Picker::Project => self.execution.project_label(index),
            Picker::Notifications => self.execution.notification_label(index).into(),
            Picker::Thread => self.execution.thread_label(index),
            _ => Self::options(picker)[index].into(),
        }
    }

    fn option_enabled(&self, picker: Picker, index: usize) -> bool {
        !matches!((picker, index), (Picker::Trigger, 1) | (Picker::Thread, 0))
    }

    fn update_day_label(&mut self) {
        let selected: Vec<_> = self
            .weekdays
            .iter()
            .enumerate()
            .filter_map(|(i, selected)| selected.then_some(i))
            .collect();
        self.day_label = match selected.as_slice() {
            [] => "Select days".into(),
            [day] => DAY_LABELS[*day].into(),
            _ => selected
                .iter()
                .map(|day| &DAY_LABELS[*day][..3])
                .collect::<Vec<_>>()
                .join(", ")
                .into(),
        };
    }
    fn choose(&mut self, picker: Picker, index: usize, cx: &mut Context<Self>) {
        if !self.option_enabled(picker, index) {
            return;
        }
        match picker {
            Picker::Schedule => {
                self.mode = Some(index);
                self.raw_rule = false;
            }
            Picker::Frequency => self.frequency = index,
            Picker::Unit => self.minutes = index == 1,
            Picker::Time => {
                let (hour, minute) = clock_parts(CLOCKS[index], &self.zone).unwrap();
                let value = clock_label(hour, minute, &self.zone);
                self.fields[TIME].update(cx, |field, cx| field.set_text(&value, cx));
            }
            Picker::Trigger => {
                if self.mode.is_some() {
                    self.menu = None;
                    cx.notify();
                    return;
                }
                self.mode = Some(1);
            }
            Picker::Days => {
                self.weekdays[index] = !self.weekdays[index];
                self.update_day_label();
            }
            Picker::Notifications => self.execution.select_notification(index),
            Picker::Project => self.execution.select_project(index),
            Picker::Model => self.execution.select_model(index),
            Picker::Effort => self.execution.select_effort(index),
            Picker::Thread => self.execution.select_thread(index),
        }
        self.schedule_dirty |= matches!(
            picker,
            Picker::Schedule
                | Picker::Frequency
                | Picker::Unit
                | Picker::Time
                | Picker::Days
                | Picker::Trigger
        );
        if picker != Picker::Days {
            self.menu = None;
        }
        self.error = None;
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if self.saving {
            cx.stop_propagation();
            return;
        }
        if let Some((picker, index)) = self.menu {
            let count = self.option_count(picker);
            match key.key.as_str() {
                "up" => self.menu = Some((picker, (index + count - 1) % count)),
                "down" => self.menu = Some((picker, (index + 1) % count)),
                "enter" | "space" => self.choose(picker, index, cx),
                "escape" | "tab" => self.menu = None,
                _ => return,
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        match key.key.as_str() {
            "escape" => self.dismiss(cx),
            "tab" => {
                let handles = self.tab_order(cx);
                let current = handles
                    .iter()
                    .position(|h| h.is_focused(window))
                    .unwrap_or(0);
                let next = &handles[(current
                    + if key.modifiers.shift {
                        handles.len() - 1
                    } else {
                        1
                    })
                    % handles.len()];
                next.focus(window, cx);
                let anchor = self
                    .fields
                    .iter()
                    .position(|field| field.focus_handle(cx) == *next)
                    .map(|i| &self.field_anchors[i])
                    .or_else(|| {
                        self.controls[..self.control_anchors.len()]
                            .iter()
                            .position(|control| control == next)
                            .filter(|index| !matches!(*index, 12..=14))
                            .map(|i| &self.control_anchors[i])
                    });
                if let Some(anchor) = anchor {
                    anchor.scroll_to(window, cx);
                    window.on_next_frame(|window, _| window.refresh());
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn visible_fields(&self) -> Vec<usize> {
        if self.raw_rule {
            return vec![];
        }
        match self.mode {
            None => vec![],
            Some(0) => vec![INTERVAL],
            Some(4) => vec![MONTH_DAYS, TIME],
            Some(5) => {
                let mut fields = vec![INTERVAL];
                if self.frequency >= 3 {
                    fields.push(MONTH_DAYS);
                }
                if self.frequency == 4 {
                    fields.push(MONTH);
                }
                fields.push(if self.frequency == 0 { MINUTE } else { TIME });
                fields
            }
            _ => vec![TIME],
        }
    }
    fn tab_order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut handles = vec![
            self.fields[NAME].focus_handle(cx),
            self.fields[PROMPT].focus_handle(cx),
            self.controls[TRIGGER].clone(),
            self.controls[0].clone(),
        ];
        if !self.raw_rule {
            if self.mode == Some(5) {
                handles.push(self.controls[1].clone());
            }
            let fields = self.visible_fields();
            let days = self.mode == Some(3) || self.mode == Some(5) && self.frequency == 2;
            for &index in &fields {
                if index == TIME && days {
                    handles.push(self.controls[4].clone());
                }
                handles.push(self.fields[index].focus_handle(cx));
            }
            if self.mode == Some(0) {
                handles.push(self.controls[2].clone());
            }
            if fields.contains(&TIME) {
                handles.push(self.controls[3].clone());
            }
        }
        handles.push(self.controls[ADVANCED].clone());
        self.append_advanced_controls(&mut handles);
        if !self.editing {
            handles.push(self.controls[CANCEL].clone());
        }
        handles.extend([self.controls[SAVE].clone(), self.controls[CLOSE].clone()]);
        handles
    }

    fn append_advanced_controls(&self, handles: &mut Vec<FocusHandle>) {
        if !self.advanced {
            return;
        }
        handles.push(self.controls[NOTIFICATIONS].clone());
        handles.push(self.controls[NEW_CHAT].clone());
        if self.execution.new_chat() {
            handles.extend([
                self.controls[AUTO_ARCHIVE].clone(),
                self.controls[PROJECT].clone(),
            ]);
            if self.execution.model_count() > 0 {
                handles.push(self.controls[MODEL].clone());
            }
            if self.execution.effort_count() > 0 {
                handles.push(self.controls[EFFORT].clone());
            }
        } else {
            handles.push(self.controls[THREAD].clone());
        }
    }

    fn row(label: &'static str, control: impl IntoElement, t: &Theme, divided: bool) -> Div {
        div()
            .relative()
            .h(px(48.))
            .px(px(20.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .when(divided, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-1.))
                        .left(px(16.))
                        .right(px(16.))
                        .h(px(1.))
                        .bg(t.border),
                )
            })
            .child(text(label, 14., 20., 400., t.text))
            .child(control)
    }

    fn input(&self, index: usize, label: &'static str, cx: &App) -> Stateful<Div> {
        div()
            .id(("automation-field", index))
            .anchor_scroll(Some(self.field_anchors[index].clone()))
            .role(Role::TextInput)
            .aria_label(label)
            .aria_value(self.value(index, cx).to_owned())
            .track_focus(&self.fields[index].focus_handle(cx))
            .w_full()
            .child(self.fields[index].clone())
    }

    fn field(&self, index: usize, t: &Theme, cx: &App) -> Div {
        let label = match index {
            INTERVAL => "Every",
            MONTH_DAYS => "On",
            MONTH => "Month",
            MINUTE => "Minute",
            _ => "Schedule",
        };
        Self::row(
            label,
            div()
                .w(px(160.))
                .flex_none()
                .child(self.input(index, label, cx)),
            t,
            true,
        )
    }

    fn popup(&self, picker: Picker, t: &Theme, cx: &Context<Self>) -> Stateful<Div> {
        let selected = self.menu.map_or(0, |(_, index)| index);
        div()
            .id("automation-editor-picker-menu")
            .role(Role::Menu)
            .w(px(if picker == Picker::Trigger {
                320.
            } else {
                240.
            }))
            .max_h(px(360.))
            .overflow_y_scroll()
            .rounded(px(10.))
            .py(px(4.))
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .children((0..self.option_count(picker)).map(|index| {
                let label = self.option_label(picker, index);
                let enabled = self.option_enabled(picker, index);
                div()
                    .id(("automation-editor-option", index))
                    .role(Role::MenuItem)
                    .aria_label(label.clone())
                    .aria_disabled(!enabled)
                    .min_h(px(32.))
                    .px(px(12.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .bg(if selected == index {
                        t.list_hover
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|s| s.bg(t.list_hover))
                    .opacity(if enabled { 1. } else { 0.5 })
                    .child(text(label, 14., 20., 400., t.text))
                    .when(picker == Picker::Days, |d| {
                        d.aria_toggled(if self.weekdays[index] {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                    })
                    .when(picker == Picker::Trigger && index == 1, |d| {
                        d.child(
                            text(
                                "Turn off Run on this computer to use this schedule",
                                12.,
                                16.,
                                400.,
                                t.text_secondary,
                            )
                            .whitespace_normal(),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose(picker, index, cx);
                        cx.stop_propagation();
                    }))
            }))
    }

    fn toggle_control(&mut self, control: usize, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        match control {
            ADVANCED => self.advanced = !self.advanced,
            NEW_CHAT => self.execution.toggle_new_chat(),
            AUTO_ARCHIVE => self.execution.toggle_archive(),
            _ => return,
        }
        self.menu = None;
        cx.notify();
    }

    fn toggle(
        &self,
        control: usize,
        label: &'static str,
        on: bool,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        crate::screens::settings::switch(
            if control == NEW_CHAT {
                "automation-new-chat"
            } else {
                "automation-auto-archive"
            },
            on,
            false,
            t,
        )
        .anchor_scroll(Some(self.control_anchors[control].clone()))
        .role(Role::Button)
        .aria_label(label)
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .track_focus(&self.controls[control])
        .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_control(control, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.toggle_control(control, cx);
                cx.stop_propagation();
            }
        }))
    }

    fn advanced(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let card = |row: Div| {
            row.px(px(21.))
                .py(px(1.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(20.)),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pt(px(12.))
            .child(card(Self::row(
                "Notifications",
                self.picker(
                    Picker::Notifications,
                    "Notifications",
                    self.execution.notification_value(),
                    NOTIFICATIONS,
                    t,
                    cx,
                ),
                t,
                false,
            )))
            .child(card(Self::row(
                "Start each run in new chat",
                self.toggle(
                    NEW_CHAT,
                    "Start each run in new chat",
                    self.execution.new_chat(),
                    t,
                    cx,
                ),
                t,
                false,
            )))
            .when(self.execution.new_chat(), |d| {
                d.child(card(Self::row(
                    "Archive successful runs",
                    self.toggle(
                        AUTO_ARCHIVE,
                        "Archive successful runs",
                        self.execution.values["autoArchive"] == true,
                        t,
                        cx,
                    ),
                    t,
                    false,
                )))
                .child(card(Self::row(
                    "Project",
                    self.picker(
                        Picker::Project,
                        "Project",
                        self.execution.project_value().as_ref(),
                        PROJECT,
                        t,
                        cx,
                    ),
                    t,
                    false,
                )))
                .child(card(Self::row(
                    "Model",
                    self.picker(
                        Picker::Model,
                        "Model",
                        self.execution.model_value().as_ref(),
                        MODEL,
                        t,
                        cx,
                    ),
                    t,
                    false,
                )))
                .child(card(Self::row(
                    "Effort",
                    self.picker(
                        Picker::Effort,
                        "Effort",
                        self.execution.effort_value().as_ref(),
                        EFFORT,
                        t,
                        cx,
                    ),
                    t,
                    false,
                )))
            })
            .when(!self.execution.new_chat(), |d| {
                d.child(card(Self::row(
                    "Runs in",
                    self.picker(
                        Picker::Thread,
                        "Runs in",
                        self.execution.thread_value().as_ref(),
                        THREAD,
                        t,
                        cx,
                    ),
                    t,
                    false,
                )))
            })
    }

    fn picker(
        &self,
        picker: Picker,
        label: &'static str,
        value: &str,
        control: usize,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let disabled = self.option_count(picker) == 0;
        let selected = (0..self.option_count(picker))
            .position(|index| self.option_label(picker, index).as_ref() == value)
            .unwrap_or(0);
        let setting = matches!(
            picker,
            Picker::Notifications | Picker::Project | Picker::Thread
        );
        let color = Self::picker_color(picker, t);
        let (indicator, gap) = Self::picker_icon(picker, color, t);
        div()
            .id(("automation-picker", control))
            .anchor_scroll(Some(self.control_anchors[control].clone()))
            .relative()
            .role(Role::Button)
            .aria_label(label)
            .aria_expanded(self.menu.is_some_and(|(open, _)| open == picker))
            .aria_disabled(disabled)
            .opacity(if disabled { 0.5 } else { 1. })
            .track_focus(&self.controls[control])
            .h(px(if setting { 28. } else { 40. }))
            .when(picker == Picker::Time, |d| d.w(px(20.)).flex_none())
            .rounded(px(10.))
            .when(setting, |d| d.px(px(7.)).rounded(px(9999.)))
            .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(gap))
            .hover(|s| s.bg(t.ghost_hover))
            .when(!value.is_empty(), |d| {
                d.child(
                    text(
                        value.to_owned(),
                        14.,
                        if setting { 18. } else { 20. },
                        400.,
                        color,
                    )
                    .when(picker == Picker::Model, |d| {
                        d.font_features(FORM_TABULAR_FEATURES.clone())
                    }),
                )
            })
            .when(picker == Picker::Trigger, |d| {
                d.child(icon("automation-scheduled", 16., color))
            })
            .child(indicator)
            .on_click(cx.listener(move |this, _, window, cx| {
                if !this.saving && !disabled {
                    this.controls[control].focus(window, cx);
                    this.menu = Some((picker, selected));
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if !this.saving
                    && !disabled
                    && this.menu.is_none()
                    && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down")
                {
                    this.menu = Some((picker, selected));
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .when(self.menu.is_some_and(|(open, _)| open == picker), |d| {
                d.child(
                    gpui::deferred(
                        div().absolute().right_0().top(px(44.)).child(
                            gpui::anchored()
                                .anchor(gpui::Anchor::TopRight)
                                .snap_to_window_with_margin(px(8.))
                                .child(self.popup(picker, t, cx)),
                        ),
                    )
                    .with_priority(3),
                )
            })
    }

    fn picker_color(picker: Picker, t: &Theme) -> gpui::Hsla {
        match picker {
            Picker::Notifications | Picker::Project | Picker::Thread => t.text,
            Picker::Trigger | Picker::Effort => t.text_tertiary,
            _ => t.text_secondary,
        }
    }

    fn picker_icon(picker: Picker, color: gpui::Hsla, t: &Theme) -> (crate::icon::Icon, f32) {
        let (name, size, gap, color) = match picker {
            Picker::Notifications | Picker::Project | Picker::Thread => {
                ("chevron-down", 14., 4., t.text_tertiary)
            }
            Picker::Trigger => ("chevron-down-medium", 16., 6., t.text_tertiary),
            Picker::Model | Picker::Effort => ("chevron-down-medium", 16., 8., t.text_tertiary),
            _ => ("automation-chevron-down", 20., 8., color),
        };
        (icon(name, size, color), gap)
    }

    fn schedule(&self, t: &Theme, window: &Window, cx: &Context<Self>) -> Div {
        let mut content = div()
            .relative()
            .p(px(1.))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(20.)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.surface_elevated_secondary.opacity(0.96)),
                border: Some((px(1.), t.border_strong)),
                ..Default::default()
            }))
            .flex()
            .flex_col()
            .child(
                Self::row(
                    "Trigger",
                    self.picker(Picker::Trigger, "Trigger", "Schedule", TRIGGER, t, cx),
                    t,
                    false,
                )
                .id("automation-trigger-row")
                .on_click(cx.listener(|this, _, window, cx| {
                    if !this.saving {
                        this.controls[TRIGGER].focus(window, cx);
                        this.menu = Some((Picker::Trigger, 0));
                        cx.notify();
                    }
                })),
            )
            .child(Self::row(
                "Frequency",
                self.picker(
                    Picker::Schedule,
                    "Repeat schedule",
                    self.mode.map_or("Choose", |index| MODES[index]),
                    0,
                    t,
                    cx,
                ),
                t,
                true,
            ));
        if self.raw_rule {
            return content.child(
                div().px(px(20.)).py(px(12.)).child(
                    text(
                        self.original_rule.clone().unwrap_or_default(),
                        13.,
                        20.,
                        400.,
                        t.text_secondary,
                    )
                    .whitespace_normal(),
                ),
            );
        }
        if self.mode == Some(5) {
            content = content.child(Self::row(
                "Repeat",
                self.picker(
                    Picker::Frequency,
                    "Repeat frequency",
                    FREQUENCIES[self.frequency],
                    1,
                    t,
                    cx,
                ),
                t,
                true,
            ));
        }
        let days = self.mode == Some(3) || self.mode == Some(5) && self.frequency == 2;
        for index in self.visible_fields() {
            if index == TIME {
                let value = self.value(TIME, cx);
                let mut font = gpui::font(crate::SANS);
                font.features = FORM_TABULAR_FEATURES.clone();
                let time_width = window
                    .text_system()
                    .shape_line(
                        value.to_owned().into(),
                        px(14.),
                        &[gpui::TextRun {
                            len: value.len(),
                            font,
                            color: t.text_secondary,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    )
                    .width
                    .max(px(48.))
                    .min(px(180.));
                let time_width = px(f32::from(time_width).ceil());
                if days {
                    content = content.child(Self::row(
                        "On",
                        self.picker(
                            Picker::Days,
                            "Days of the week",
                            self.day_label.as_ref(),
                            4,
                            t,
                            cx,
                        ),
                        t,
                        true,
                    ));
                }
                content = content.child(Self::row(
                    "Time",
                    div()
                        .w(time_width + px(28.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(time_width)
                                .flex_none()
                                .font_features(FORM_TABULAR_FEATURES.clone())
                                .child(self.input(TIME, "Time", cx)),
                        )
                        .child(self.picker(Picker::Time, "Time presets", "", 3, t, cx)),
                    t,
                    true,
                ));
            } else {
                content = content.child(self.field(index, t, cx));
            }
        }
        if self.mode == Some(0) {
            content = content.child(Self::row(
                "Unit",
                self.picker(
                    Picker::Unit,
                    "Interval unit",
                    if self.minutes { "minutes" } else { "hours" },
                    2,
                    t,
                    cx,
                ),
                t,
                true,
            ));
        }
        content
    }

    fn button(
        &self,
        control: usize,
        label: &'static str,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let (height, padding, radius, gap) = match control {
            ADVANCED => (28., 8., 10., 8.),
            CLOSE => (32., 0., 10., 6.),
            _ => (36., 16., 9999., 6.),
        };
        let (weight, color) = if control == SAVE {
            (500., t.surface_elevated_secondary)
        } else {
            (400., t.text_secondary)
        };
        div()
            .id(("automation-editor-button", control))
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&self.controls[control])
            .h(px(height))
            .px(px(padding))
            .rounded(px(radius))
            .when(control == CLOSE, |d| d.w(px(32.)))
            .when(control == SAVE, |d| {
                d.px(px(17.)).child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(9999.)),
                    fill: Some(t.text),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }))
            })
            .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(gap))
            .when(control != CLOSE, |d| {
                d.child(
                    text(label, 14., 20., weight, color)
                        .when(control == ADVANCED, |d| d.w(px(58.87)).flex_none()),
                )
            })
            .when(control == CLOSE, |d| {
                d.child(icon("close", 20., t.text_tertiary))
            })
            .when(control == ADVANCED, |d| {
                d.aria_expanded(self.advanced).child(icon(
                    if self.advanced {
                        "automation-chevron-up"
                    } else {
                        "automation-chevron-down"
                    },
                    20.,
                    t.text_secondary,
                ))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_button(control, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate_button(control, window, cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn activate_button(&mut self, control: usize, window: &mut Window, cx: &mut Context<Self>) {
        if control == ADVANCED {
            self.toggle_control(control, cx);
        } else if control == SAVE {
            self.submit(window, cx);
        } else if !self.saving {
            cx.emit(Outcome::Dismiss);
        }
    }

    fn footer(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let field_error = self.fields_error(cx);
        let valid = !self.saving
            && field_error.is_none()
            && self.execution.valid()
            && self.rule(cx).is_some();
        let reason = if let Some(error) = field_error {
            error
        } else if !self.execution.valid() {
            "Select a chat"
        } else {
            "Enter valid schedule fields"
        };
        div()
            .flex()
            .justify_end()
            .gap(px(12.))
            .pt(px(20.))
            .when(!self.editing, |d| {
                d.child(self.button(CANCEL, "Cancel", t, cx))
            })
            .child(tooltip(
                self.button(
                    SAVE,
                    match (self.saving, self.editing) {
                        (true, _) => "Saving…",
                        (false, true) => "Save",
                        (false, false) => "Create",
                    },
                    t,
                    cx,
                )
                .aria_disabled(!valid)
                .opacity(if valid { 1. } else { 0.4 }),
                if valid { "Save scheduled task" } else { reason },
                None,
                Side::Top,
            ))
    }
}

fn clock_label(hour: u8, minute: u8, zone: &str) -> String {
    if hour >= 24 || minute >= 60 {
        return format!("{hour}:{minute:02}");
    }
    format!(
        "{}:{minute:02} {} {zone}",
        if hour.is_multiple_of(12) {
            12
        } else {
            hour % 12
        },
        if hour < 12 { "AM" } else { "PM" }
    )
}

fn clock_parts(value: &str, zone: &str) -> Option<(u8, u8)> {
    let mut parts = value.split_whitespace();
    let (hour, minute) = parts.next()?.split_once(':')?;
    if minute.len() != 2 {
        return None;
    }
    let mut hour = hour.parse::<u8>().ok()?;
    let minute = minute.parse::<u8>().ok().filter(|value| *value < 60)?;
    if let Some(period) = parts.next() {
        if !(1..=12).contains(&hour) || !matches!(period, "AM" | "PM" | "am" | "pm") {
            return None;
        }
        hour = hour % 12
            + if period.eq_ignore_ascii_case("pm") {
                12
            } else {
                0
            };
        if let Some(label) = parts.next()
            && label != zone
        {
            return None;
        }
    }
    (hour < 24 && parts.next().is_none()).then_some((hour, minute))
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let title = if self.editing {
            "Edit task"
        } else {
            "Schedule a task"
        };
        let mut form = div()
            .id("automation-editor-form")
            .role(Role::Dialog)
            .aria_label(title)
            .relative()
            .w(px(680.).min(window.viewport_size().width * 0.92))
            .max_h(window.viewport_size().height * 0.85)
            .p(px(20.))
            .flex()
            .flex_col()
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(30.)),
                fill: Some(t.surface_elevated_secondary),
                shadows: vec![
                    shapes::Shadow {
                        color: t.border,
                        offset: gpui::point(px(0.), px(0.)),
                        blur: px(0.),
                        spread: px(0.5),
                        inset: false,
                    },
                    shapes::Shadow {
                        color: gpui::black().opacity(0.1),
                        offset: gpui::point(px(0.), px(8.)),
                        blur: px(10.),
                        spread: px(-6.),
                        inset: false,
                    },
                    shapes::Shadow {
                        color: gpui::black().opacity(0.1),
                        offset: gpui::point(px(0.), px(20.)),
                        blur: px(25.),
                        spread: px(-5.),
                        inset: false,
                    },
                ],
                ..Default::default()
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter"
                    && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                {
                    this.submit(window, cx);
                    cx.stop_propagation();
                }
            }))
            // Name and Prompt bind cmd-enter to `ModifiedSubmit`, which wins over key listeners.
            .capture_action(cx.listener(|this, _: &ModifiedSubmit, window, cx| {
                this.submit(window, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .relative()
                    .h(px(26.))
                    .mb(px(20.))
                    .child(
                        text(title, 16., 26., 590., t.text)
                            .font_features(FORM_TITLE_FEATURES.clone()),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(-6.))
                            .top(px(-4.))
                            .child(self.button(CLOSE, "Close dialog", t, cx)),
                    ),
            )
            .child(
                div()
                    .id("automation-editor-fields")
                    .font_features(FORM_BODY_FEATURES.clone())
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(self.advanced, |d| d.top_layer())
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(self.input(NAME, "Name", cx))
                    .child(self.input(PROMPT, "Prompt", cx))
                    .child(self.schedule(t, window, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(self.button(ADVANCED, "Advanced", t, cx))
                            .when(self.advanced, |d| d.child(self.advanced(t, cx).w_full())),
                    ),
            )
            .when_some(self.error.as_ref(), |d, error| {
                d.child(text(error.clone(), 13., 20., 400., t.text_error).whitespace_normal())
            })
            .child(self.footer(t, cx));
        if self.menu.is_some() {
            form = form.child(div().absolute().inset_0().occlude().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                    cx.stop_propagation();
                }),
            ));
        }
        if self.saving {
            form = form.child(div().absolute().inset_0().occlude());
        }
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(window.viewport_size().height * 0.1)
            .child(shapes::backdrop_blur(
                px(1.),
                shapes::Fill::Color,
                false,
                shapes::Decoration {
                    fill: Some(gpui::black().opacity(0.5)),
                    ..Default::default()
                },
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dismiss(cx);
                }),
            )
            // Chromium composites the dialog as a layer of its own, whose tiles start at its
            // shadows' ink (1.5 x blur + spread, moved by the offset): `0 20px 25px -5px`
            // reaches 32.5 px left and right, 12.5 px above and 52.5 px below the box. The
            // original's tile seams and squircle border raster match this origin.
            .child(
                div()
                    .top_layer()
                    .px(px(32.5))
                    .pt(px(12.5))
                    .pb(px(52.5))
                    .mx(px(-32.5))
                    .mt(px(-12.5))
                    .mb(px(-52.5))
                    .child(form),
            )
    }
}

pub struct CreateMenu {
    open: bool,
    selected: usize,
    focus: FocusHandle,
    button_hovered: bool,
}

impl CreateMenu {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            open: false,
            selected: 0,
            focus: cx.focus_handle().tab_stop(true),
            button_hovered: false,
        }
    }
    fn select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        let mode = [
            super::CreationMode::Codex,
            super::CreationMode::ChatGPT,
            super::CreationMode::Manual,
        ][self.selected];
        window.dispatch_action(Box::new(super::CreateAutomation { mode }), cx);
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "up" if self.open => self.selected = (self.selected + 2) % 3,
            "down" if self.open => self.selected = (self.selected + 1) % 3,
            "enter" | "space" if self.open => self.select(window, cx),
            "enter" | "space" | "down" => self.open = true,
            "escape" | "tab" if self.open => self.open = false,
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for CreateMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let labels = [
            "Create with Codex",
            "Create with ChatGPT",
            "Set up manually",
        ];
        let menu = div()
            .id("automation-create-options")
            .occlude()
            .role(Role::Menu)
            .aria_label("Create scheduled task options")
            .w(px(220.))
            .rounded(px(10.))
            .py(px(4.))
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.button_hovered {
                    this.open = false;
                    cx.notify();
                }
            }))
            .children(labels.into_iter().enumerate().map(|(i, label)| {
                div()
                    .id(("automation-create-option", i))
                    .role(Role::MenuItem)
                    .aria_label(label)
                    .h(px(30.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .bg(if self.selected == i {
                        t.list_hover
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|s| s.bg(t.list_hover))
                    .child(text(label, 13., 20., 430., t.text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected = i;
                        this.select(window, cx);
                    }))
            }));
        let visible = self.open || self.focus.is_focused(window);
        div()
            .id("automation-create-menu")
            .relative()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::key))
            .child(
                div()
                    .id("automation-create-trigger")
                    .role(Role::Button)
                    .aria_label("Create scheduled task options")
                    .aria_expanded(self.open)
                    .size(px(28.))
                    .rounded(px(8.))
                    .opacity(if visible { 1. } else { 0. })
                    .group_hover("new-task", |s| s.opacity(1.))
                    .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
                    .hover(|s| s.bg(t.ghost_hover))
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        this.button_hovered = *hovered;
                    }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("chevron-down", 16., t.text_secondary))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.focus.focus(window, cx);
                        this.open = !this.open;
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .when(self.open, |d| {
                d.child(
                    gpui::deferred(
                        div().absolute().right(px(0.)).top(px(32.)).child(
                            gpui::anchored()
                                .anchor(gpui::Anchor::TopRight)
                                .snap_to_window_with_margin(px(7.))
                                .child(menu),
                        ),
                    )
                    .with_priority(2),
                )
            })
    }
}
