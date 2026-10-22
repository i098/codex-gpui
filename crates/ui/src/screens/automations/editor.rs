use crate::{
    composer::{Composer, Dismiss, ModifiedSubmit},
    icon::icon,
    text,
    tooltip::{Side, tooltip},
};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, Role,
    ScrollAnchor, ScrollHandle, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use theme::{ActiveTheme as _, Theme};

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

#[derive(Clone, Debug)]
pub struct Draft {
    pub name: String,
    pub prompt: String,
    pub rrule: String,
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
}

pub struct Editor {
    fields: [Entity<Composer>; 7],
    controls: [FocusHandle; 15],
    scroll: ScrollHandle,
    field_anchors: [ScrollAnchor; 7],
    control_anchors: [ScrollAnchor; 11],
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
}

impl EventEmitter<Outcome> for Editor {}
impl Focusable for Editor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.fields[NAME].focus_handle(cx)
    }
}

impl Editor {
    pub fn new(item: Option<&Value>, cx: &mut Context<Self>) -> Self {
        let values = [
            item.and_then(|v| v["name"].as_str()).unwrap_or(""),
            item.and_then(|v| v["prompt"].as_str()).unwrap_or(""),
            "09:00",
            "1",
            "1",
            "1",
            "0",
        ];
        let placeholders = [
            "Scheduled task title",
            "Describe what ChatGPT should do",
            "HH:MM",
            "1",
            "1, 15",
            "1",
            "0",
        ];
        let fields = std::array::from_fn(|i| {
            cx.new(|cx| Composer::settings_field(placeholders[i], values[i], i == PROMPT, cx))
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

    fn load_rule(&mut self, rule: &str, cx: &mut Context<Self>) {
        let parts: std::collections::HashMap<_, _> = rule
            .trim_start_matches("RRULE:")
            .split(';')
            .filter_map(|s| s.split_once('='))
            .collect();
        let frequency = parts.get("FREQ").copied().unwrap_or("");
        self.raw_rule = uneditable_rule(rule, &parts, frequency);
        self.frequency = ["HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"]
            .iter()
            .position(|s| *s == frequency)
            .unwrap_or(1);
        self.minutes = frequency == "MINUTELY";
        let interval = parts.get("INTERVAL").copied().unwrap_or("1");
        let weekdays = parts.get("BYDAY").copied().unwrap_or("MO");
        self.weekdays = std::array::from_fn(|i| weekdays.split(',').any(|day| day == DAYS[i]));
        self.mode = Some(if self.raw_rule {
            5
        } else {
            rule_mode(frequency, &parts, interval, weekdays)
        });
        for (index, value) in [
            (INTERVAL, interval.to_owned()),
            (
                TIME,
                format!(
                    "{:02}:{:02}",
                    parts
                        .get("BYHOUR")
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(9),
                    parts
                        .get("BYMINUTE")
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(0)
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

    pub fn draft(&self, cx: &App) -> Option<Draft> {
        let name = self.value(NAME, cx);
        let prompt = self.value(PROMPT, cx);
        if name.is_empty() || prompt.is_empty() {
            return None;
        }
        Some(Draft {
            name: name.into(),
            prompt: prompt.into(),
            rrule: self.rule(cx)?,
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
        if mode == 0 {
            let interval = self.number(INTERVAL, 1, 999, cx)?;
            return Some(format!(
                "FREQ={};INTERVAL={interval}{}",
                if self.minutes { "MINUTELY" } else { "HOURLY" },
                if self.minutes { "" } else { ";BYMINUTE=0" }
            ));
        }
        let (frequency, interval) = if mode == 5 {
            (self.frequency, self.number(INTERVAL, 1, 999, cx)?)
        } else {
            (preset_frequency(mode)?, 1)
        };
        let mut rule = format!(
            "FREQ={};INTERVAL={interval}",
            ["HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"][frequency],
        );
        rule.push_str(&self.day_parts(frequency, mode, cx)?);
        rule.push_str(&self.time_part(frequency, cx)?);
        Some(rule)
    }

    /// `BYDAY`, `BYMONTHDAY` and `BYMONTH` for a weekly, monthly or yearly rule.
    fn day_parts(&self, frequency: usize, mode: usize, cx: &App) -> Option<String> {
        let mut parts = String::new();
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
            parts.push_str(&format!(";BYDAY={}", days.join(",")));
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
            parts.push_str(&format!(
                ";BYMONTHDAY={}",
                days.split(',').map(str::trim).collect::<Vec<_>>().join(",")
            ));
        }
        if frequency == 4 {
            parts.push_str(&format!(";BYMONTH={}", self.number(MONTH, 1, 12, cx)?));
        }
        Some(parts)
    }

    /// `BYMINUTE` for an hourly rule, else `BYHOUR` and `BYMINUTE` from the time field.
    fn time_part(&self, frequency: usize, cx: &App) -> Option<String> {
        if frequency == 0 {
            return Some(format!(";BYMINUTE={}", self.number(MINUTE, 0, 59, cx)?));
        }
        let (hour, minute) = self.value(TIME, cx).split_once(':')?;
        let hour = hour.parse::<u8>().ok().filter(|h| *h < 24)?;
        let minute = minute.parse::<u8>().ok().filter(|m| *m < 60)?;
        Some(format!(";BYHOUR={hour};BYMINUTE={minute}"))
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
            self.controls[14].focus(window, cx);
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
        }
    }
    fn choose(&mut self, picker: Picker, index: usize, cx: &mut Context<Self>) {
        match picker {
            Picker::Schedule => {
                self.mode = Some(index);
                self.raw_rule = false;
            }
            Picker::Frequency => self.frequency = index,
            Picker::Unit => self.minutes = index == 1,
            Picker::Time => {
                self.fields[TIME].update(cx, |field, cx| field.set_text(CLOCKS[index], cx))
            }
        }
        self.schedule_dirty = true;
        self.menu = None;
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
            let count = Self::options(picker).len();
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
            self.controls[0].clone(),
        ];
        if self.shows_frequency() {
            handles.push(self.controls[1].clone());
        }
        handles.extend(
            self.visible_fields()
                .iter()
                .map(|i| self.fields[*i].focus_handle(cx)),
        );
        if self.mode == Some(0) {
            handles.push(self.controls[2].clone());
        }
        if self.shows_weekdays() {
            handles.extend(self.controls[4..11].iter().cloned());
        }
        if self.visible_fields().contains(&TIME) {
            handles.push(self.controls[3].clone());
        }
        handles.extend([
            self.controls[13].clone(),
            self.controls[14].clone(),
            self.controls[12].clone(),
        ]);
        handles
    }

    fn field(&self, index: usize, label: &'static str, t: &Theme, cx: &App) -> Stateful<Div> {
        div()
            .id(("automation-field-row", index))
            .anchor_scroll(Some(self.field_anchors[index].clone()))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(text(label, 13., 20., 500., t.text_secondary))
            .child(
                div()
                    .id(("automation-field", index))
                    .role(Role::TextInput)
                    .aria_label(label)
                    .aria_value(self.value(index, cx).to_owned())
                    .rounded(px(8.))
                    .track_focus(&self.fields[index].focus_handle(cx))
                    .focus_visible(|d| d.border_2().border_color(gpui::rgb(0x3a83f7)))
                    .child(self.fields[index].clone()),
            )
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
        let selected = Self::options(picker)
            .iter()
            .position(|v| *v == value)
            .unwrap_or(0);
        div()
            .id(("automation-picker", control))
            .anchor_scroll(Some(self.control_anchors[control].clone()))
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&self.controls[control])
            .h(px(36.))
            .px(px(10.))
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .hover(|s| s.bg(t.ghost_hover))
            .child(text(value.to_owned(), 13., 20., 430., t.text))
            .child(icon("chevron-down", 16., t.text_secondary))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !this.saving {
                    this.controls[control].focus(window, cx);
                    this.menu = Some((picker, selected));
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                if !this.saving
                    && this.menu.is_none()
                    && matches!(e.keystroke.key.as_str(), "enter" | "space" | "down")
                {
                    this.menu = Some((picker, selected));
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
    }

    fn schedule(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(text("Choose a schedule", 13., 20., 500., t.text_secondary))
            .child(self.picker(
                Picker::Schedule,
                "Repeat",
                self.mode.map_or("Choose a schedule", |i| MODES[i]),
                0,
                t,
                cx,
            ));
        if self.raw_rule {
            content = content.child(
                text(
                    self.original_rule.clone().unwrap_or_default(),
                    13.,
                    20.,
                    430.,
                    t.text_secondary,
                )
                .whitespace_normal(),
            );
        }
        if self.shows_frequency() {
            content = content.child(self.picker(
                Picker::Frequency,
                "Repeat frequency",
                FREQUENCIES[self.frequency],
                1,
                t,
                cx,
            ));
        }
        for index in self.visible_fields() {
            content = content.child(self.field(index, field_label(index), t, cx));
        }
        if self.mode == Some(0) {
            content = content.child(self.picker(
                Picker::Unit,
                "Interval unit",
                if self.minutes { "minutes" } else { "hours" },
                2,
                t,
                cx,
            ));
        }
        if self.shows_weekdays() {
            content = content.child(self.weekday_toggles(t, cx));
        }
        if self.visible_fields().contains(&TIME) {
            content =
                content.child(self.picker(Picker::Time, "Time presets", "Time presets", 3, t, cx));
        }
        content
    }

    /// The custom rule shows its frequency picker.
    fn shows_frequency(&self) -> bool {
        self.mode == Some(5) && !self.raw_rule
    }

    /// The weekly preset and a weekly custom rule show the weekday toggles.
    fn shows_weekdays(&self) -> bool {
        self.mode == Some(3) || self.mode == Some(5) && self.frequency == 2 && !self.raw_rule
    }

    fn weekday_toggles(&self, t: &Theme, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id("automation-weekdays")
            .role(Role::Group)
            .aria_label("Days of the week")
            .flex()
            .gap(px(4.))
            .children(
                DAYS.iter()
                    .enumerate()
                    .map(|(i, day)| self.weekday_toggle(i, day, t, cx)),
            )
    }

    fn weekday_toggle(
        &self,
        i: usize,
        day: &'static str,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("automation-weekday", i))
            .anchor_scroll(Some(self.control_anchors[4 + i].clone()))
            .role(Role::Button)
            .aria_label(DAY_LABELS[i])
            .aria_toggled(if self.weekdays[i] {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .track_focus(&self.controls[4 + i])
            .size(px(34.))
            .rounded(px(8.))
            .bg(if self.weekdays[i] {
                t.text_alpha_10
            } else {
                gpui::transparent_black()
            })
            .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
            .flex()
            .items_center()
            .justify_center()
            .child(text(day, 12., 20., 430., t.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.saving {
                    this.weekdays[i] = !this.weekdays[i];
                    this.schedule_dirty = true;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                if !this.saving && matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    this.weekdays[i] = !this.weekdays[i];
                    this.schedule_dirty = true;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
    }

    fn button(
        &self,
        control: usize,
        label: &'static str,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("automation-editor-button", control))
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&self.controls[control])
            .h(px(32.))
            .px(px(12.))
            .rounded(px(8.))
            .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
            .flex()
            .items_center()
            .justify_center()
            .child(text(label, 13., 20., 500., t.text))
            .on_click(cx.listener(move |this, _, window, cx| {
                if control == 14 {
                    this.submit(window, cx);
                } else if !this.saving {
                    cx.emit(Outcome::Dismiss);
                }
            }))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    if control == 14 {
                        this.submit(window, cx);
                    } else if !this.saving {
                        cx.emit(Outcome::Dismiss);
                    }
                    cx.stop_propagation();
                }
            }))
    }

    fn title(&self) -> &'static str {
        if self.editing {
            "Edit scheduled task"
        } else {
            "Create scheduled task"
        }
    }

    /// The Create or Save button; its tooltip names the first missing field.
    fn save_button(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let valid = !self.saving
            && !self.value(NAME, cx).is_empty()
            && !self.value(PROMPT, cx).is_empty()
            && self.rule(cx).is_some();
        tooltip(
            self.button(14, self.save_label(), t, cx)
                .bg(t.text_alpha_10)
                .aria_disabled(!valid)
                .opacity(if valid { 1. } else { 0.4 }),
            if valid {
                "Save scheduled task"
            } else {
                self.missing_reason(cx)
            },
            None,
            Side::Top,
        )
    }

    fn save_label(&self) -> &'static str {
        if self.saving {
            "Saving…"
        } else if self.editing {
            "Save"
        } else {
            "Create"
        }
    }

    fn missing_reason(&self, cx: &App) -> &'static str {
        if self.value(NAME, cx).is_empty() {
            "Enter a name"
        } else if self.value(PROMPT, cx).is_empty() {
            "Enter a prompt"
        } else if self.mode.is_none() {
            "Choose a schedule"
        } else {
            "Enter valid schedule fields"
        }
    }

    /// The open picker's options over the form; a click outside closes it.
    fn picker_menu(&self, picker: Picker, selected: usize, t: &Theme, cx: &Context<Self>) -> Div {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .id("automation-editor-picker-menu")
                    .absolute()
                    .right(px(24.))
                    .bottom(px(64.))
                    .w(px(220.))
                    .rounded(px(10.))
                    .py(px(4.))
                    .bg(t.menu_background)
                    .border_1()
                    .border_color(t.border)
                    .role(Role::Menu)
                    .children(Self::options(picker).iter().enumerate().map(|(i, label)| {
                        div()
                            .id(("automation-editor-option", i))
                            .role(Role::MenuItem)
                            .aria_label(*label)
                            .h(px(30.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .bg(if selected == i {
                                t.list_hover
                            } else {
                                gpui::transparent_black()
                            })
                            .hover(|s| s.bg(t.list_hover))
                            .child(text(*label, 13., 20., 430., t.text))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(picker, i, cx)))
                    })),
            )
    }
}

fn field_label(index: usize) -> &'static str {
    match index {
        TIME => "Time (local, HH:MM)",
        INTERVAL => "Every",
        MONTH_DAYS => "On days (1-31)",
        MONTH => "Month (1-12)",
        MINUTE => "Minute past the hour",
        _ => "Schedule rule",
    }
}

/// The `FREQUENCIES` index that a daily, weekday, weekly or monthly preset saves.
fn preset_frequency(mode: usize) -> Option<usize> {
    match mode {
        1 => Some(1),
        2 | 3 => Some(2),
        4 => Some(3),
        _ => None,
    }
}

/// The RRULE fields the editor shows for `frequency`.
fn editable_fields(frequency: &str) -> &'static [&'static str] {
    match frequency {
        "MINUTELY" => &[],
        "HOURLY" => &["BYMINUTE"],
        "DAILY" => &["BYHOUR", "BYMINUTE"],
        "WEEKLY" => &["BYDAY", "BYHOUR", "BYMINUTE"],
        "MONTHLY" => &["BYMONTHDAY", "BYHOUR", "BYMINUTE"],
        "YEARLY" => &["BYMONTH", "BYMONTHDAY", "BYHOUR", "BYMINUTE"],
        _ => &[],
    }
}

/// Whether `rule` has parts the editor cannot show; the editor then keeps it verbatim.
fn uneditable_rule(
    rule: &str,
    parts: &std::collections::HashMap<&str, &str>,
    frequency: &str,
) -> bool {
    let rule_fields = editable_fields(frequency);
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

/// The `MODES` entry that shows an editable rule.
fn rule_mode(
    frequency: &str,
    parts: &std::collections::HashMap<&str, &str>,
    interval: &str,
    weekdays: &str,
) -> usize {
    if frequency == "MINUTELY" || frequency == "HOURLY" && parts.get("BYMINUTE") == Some(&"0") {
        0
    } else if frequency == "HOURLY" || interval != "1" || frequency == "YEARLY" {
        5
    } else {
        match frequency {
            "DAILY" => 1,
            "WEEKLY" if weekdays == "MO,TU,WE,TH,FR" => 2,
            "WEEKLY" => 3,
            "MONTHLY" => 4,
            _ => 5,
        }
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let mut form = div()
            .id("automation-editor-form")
            .role(Role::Dialog)
            .aria_label(self.title())
            .relative()
            .w(px(640.).min(window.viewport_size().width * 0.92))
            .max_h(window.viewport_size().height * 0.9)
            .p(px(24.))
            .rounded(px(20.))
            .bg(t.surface_elevated_secondary)
            .border_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .gap(px(16.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            // Text fields bind Cmd-Enter to `ModifiedSubmit`; the binding runs before key listeners.
            .capture_action(cx.listener(|this, _: &ModifiedSubmit, window, cx| {
                this.submit(window, cx);
                cx.stop_propagation();
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                if key.key == "enter" && (key.modifiers.platform || key.modifiers.control) {
                    this.submit(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(text(self.title(), 16., 24., 590., t.text))
                    .child(self.button(12, "Close", t, cx)),
            )
            .child(
                div()
                    .id("automation-editor-fields")
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(self.field(NAME, "Name", t, cx))
                    .child(self.field(PROMPT, "Prompt", t, cx))
                    .child(self.schedule(t, cx)),
            )
            .when_some(self.error.as_ref(), |d, error| {
                d.child(text(error.clone(), 13., 20., 430., t.text_error).whitespace_normal())
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.button(13, "Cancel", t, cx))
                    .child(self.save_button(t, cx)),
            );
        if let Some((picker, selected)) = self.menu {
            form = form.child(self.picker_menu(picker, selected, t, cx));
        }
        if self.saving {
            form = form.child(div().absolute().inset_0().occlude());
        }
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(t.dialog_backdrop)
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if !this.saving {
                        cx.emit(Outcome::Dismiss);
                    }
                }),
            )
            .child(form)
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "up" if this.open => this.selected = (this.selected + 2) % 3,
                    "down" if this.open => this.selected = (this.selected + 1) % 3,
                    "enter" | "space" if this.open => this.select(window, cx),
                    "enter" | "space" | "down" => this.open = true,
                    "escape" | "tab" if this.open => this.open = false,
                    _ => return,
                }
                cx.notify();
                cx.stop_propagation();
            }))
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
