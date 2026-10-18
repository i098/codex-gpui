//! Modal surfaces recorded in `refs/interactions/f3b.json`; the Scheduled sidebar's `New task`
//! create dialog in `refs/interactions/s036.json`.
use crate::{Copy, Cut, Paste, SelectAll, icon::icon, motion::ButtonVisual, text};
use gpui::{
    App, Context, Corners, Div, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render, ScrollHandle, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, point,
    prelude::FluentBuilder as _, px, rgba,
};
use theme::{ActiveTheme as _, Theme};

type Shortcut = (&'static str, &'static str, &'static [&'static str]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Shortcuts,
    CloseWindow,
    FullAccess,
    /// `Schedule a task`: the Scheduled sidebar's `New task` create editor (AM-005).
    ScheduleTask,
    /// `Rename chat` from a Recents row's menu (RS-091, refs/interactions/s002.json): the
    /// `Chat title` input, prefilled and selected, `Cancel`, `Save` and `Close dialog`.
    Rename,
}
#[derive(Clone)]
pub enum Outcome {
    Dismiss,
    FullAccess,
    CloseWindow {
        do_not_ask_again: bool,
    },
    /// `Save` or Enter in `Rename chat`: `thread/name/set` of `thread` to `name`.
    Rename {
        thread: String,
        name: String,
    },
    ScheduleTask {
        prompt: String,
        rrule: String,
    },
}

pub struct Dialog {
    kind: Kind,
    controls: [FocusHandle; 6],
    selected: usize,
    /// The text of the search, chat title, or task instructions.
    query: String,
    select_all: bool,
    instruction_cursor: usize,
    instruction_anchor: usize,
    do_not_ask_again: bool,
    scroll: ScrollHandle,
    /// `Rename`: the thread id.
    thread: String,
    schedule_time: String,
    repeat: usize,
    repeat_open: bool,
    saving: bool,
    error: Option<String>,
}
impl EventEmitter<Outcome> for Dialog {}
impl Focusable for Dialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.controls[self.selected].clone()
    }
}
impl Dialog {
    pub fn new(kind: Kind, cx: &mut Context<Self>) -> Self {
        Self {
            kind,
            controls: std::array::from_fn(|_| cx.focus_handle()),
            selected: 0,
            query: String::new(),
            select_all: false,
            instruction_cursor: 0,
            instruction_anchor: 0,
            do_not_ask_again: false,
            scroll: ScrollHandle::new(),
            thread: String::new(),
            schedule_time: "09:00".into(),
            repeat: 0,
            repeat_open: false,
            saving: false,
            error: None,
        }
    }
    /// `Rename chat` for `thread`, its input prefilled with `title` and all of it selected.
    pub fn rename(thread: String, title: String, cx: &mut Context<Self>) -> Self {
        Self {
            query: title,
            select_all: true,
            thread,
            ..Self::new(Kind::Rename, cx)
        }
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn do_not_ask_again(&self) -> bool {
        self.do_not_ask_again
    }
    pub fn focused_control(&self) -> &'static str {
        match (self.kind, self.selected) {
            (Kind::Shortcuts, 0) => "Search shortcuts",
            (Kind::Shortcuts, _) => "Close dialog",
            (Kind::CloseWindow, 0) => "Cancel",
            (Kind::CloseWindow, 1) => "Close Window",
            (Kind::CloseWindow, _) => "Do not ask me again",
            (Kind::FullAccess, 0) => "Cancel",
            (Kind::FullAccess, _) => "Confirm",
            (Kind::ScheduleTask, 0) => "Instructions…",
            (Kind::ScheduleTask, 1) => "Cancel",
            (Kind::ScheduleTask, 2) => "Close dialog",
            (Kind::ScheduleTask, 3) => "Repeat",
            (Kind::ScheduleTask, 4) => "Time",
            (Kind::ScheduleTask, _) => "Create",
            (Kind::Rename, 0) => "Chat title",
            (Kind::Rename, 1) => "Save",
            (Kind::Rename, _) => "Cancel",
        }
    }
    /// The whole query is selected (typing replaces it).
    pub fn all_selected(&self) -> bool {
        self.select_all
    }
    fn editing(&self) -> bool {
        (matches!(
            self.kind,
            Kind::Shortcuts | Kind::Rename | Kind::ScheduleTask
        ) && self.selected == 0)
            || (self.kind == Kind::ScheduleTask && self.selected == 4)
    }

    fn input(&mut self) -> &mut String {
        if self.kind == Kind::ScheduleTask && self.selected == 4 {
            &mut self.schedule_time
        } else {
            &mut self.query
        }
    }

    fn schedule_rule(&self) -> Option<String> {
        let (hour, minute) = self.schedule_time.split_once(':')?;
        if hour.len() != 2 || minute.len() != 2 {
            return None;
        }
        let hour: u8 = hour.parse().ok()?;
        let minute: u8 = minute.parse().ok()?;
        if hour > 23 || minute > 59 {
            return None;
        }
        let frequency = if self.repeat == 0 {
            "FREQ=DAILY".to_owned()
        } else {
            format!("FREQ=WEEKLY;BYDAY={}", REPEAT[self.repeat].1)
        };
        Some(format!("{frequency};BYHOUR={hour};BYMINUTE={minute}"))
    }

    pub fn schedule_result(&mut self, error: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(error);
        cx.notify();
    }

    pub fn saving(&self) -> bool {
        self.saving
    }
    fn visible(&self) -> impl Iterator<Item = &'static Shortcut> + '_ {
        let query = self.query.trim().as_bytes();
        SHORTCUTS.iter().filter(move |(_, title, _)| {
            query.is_empty()
                || title
                    .as_bytes()
                    .windows(query.len())
                    .any(|s| s.eq_ignore_ascii_case(query))
        })
    }
    pub fn visible_titles(&self) -> Vec<&'static str> {
        self.visible().map(|(_, title, _)| *title).collect()
    }
    fn activate(&mut self, cx: &mut Context<Self>) {
        match (self.kind, self.selected) {
            (Kind::ScheduleTask, _) if self.saving => {}
            (Kind::ScheduleTask, 3) => {
                self.repeat_open = !self.repeat_open;
                cx.notify();
            }
            (Kind::ScheduleTask, 5) => {
                if let Some(rrule) = self
                    .schedule_rule()
                    .filter(|_| !self.query.trim().is_empty())
                {
                    self.saving = true;
                    self.error = None;
                    cx.emit(Outcome::ScheduleTask {
                        prompt: self.query.trim().to_owned(),
                        rrule,
                    });
                    cx.notify();
                }
            }
            (Kind::ScheduleTask, 0 | 4) => {}
            (Kind::FullAccess, 1) => cx.emit(Outcome::FullAccess),
            (Kind::CloseWindow, 1) => cx.emit(Outcome::CloseWindow {
                do_not_ask_again: self.do_not_ask_again,
            }),
            (Kind::CloseWindow, 2) => {
                self.do_not_ask_again = !self.do_not_ask_again;
                cx.notify();
            }
            (Kind::Rename, 0 | 1) => cx.emit(Outcome::Rename {
                thread: self.thread.clone(),
                name: self.query.clone(),
            }),
            _ => cx.emit(Outcome::Dismiss),
        }
    }
    fn instruction_selection(&self) -> std::ops::Range<usize> {
        if self.select_all {
            0..self.query.len()
        } else {
            self.instruction_anchor.min(self.instruction_cursor)
                ..self.instruction_anchor.max(self.instruction_cursor)
        }
    }

    fn replace_instructions(&mut self, value: &str, cx: &mut Context<Self>) {
        let range = self.instruction_selection();
        self.instruction_cursor = range.start + value.len();
        self.query.replace_range(range, value);
        self.instruction_anchor = self.instruction_cursor;
        self.select_all = false;
        self.reveal_instruction_cursor();
        cx.notify();
    }

    fn reveal_instruction_cursor(&self) {
        self.scroll.scroll_to_item(
            self.query[..self.instruction_cursor]
                .bytes()
                .filter(|b| *b == b'\n')
                .count(),
        );
    }
    fn insert(&mut self, value: &str, cx: &mut Context<Self>) {
        if !self.editing() || self.saving {
            return;
        }
        if self.kind == Kind::ScheduleTask && self.selected == 0 {
            let value: String = value
                .chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .collect();
            self.replace_instructions(&value, cx);
            self.error = None;
            return;
        }
        if self.select_all {
            self.input().clear();
            self.select_all = false;
        }
        let multiline = self.kind == Kind::ScheduleTask && self.selected == 0;
        self.input().extend(
            value
                .chars()
                .filter(|c| !c.is_control() || (multiline && *c == '\n')),
        );
        self.error = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if self.kind == Kind::ScheduleTask && self.saving {
            cx.stop_propagation();
            return;
        }
        if self.repeat_open {
            match k.key.as_str() {
                "up" => self.repeat = (self.repeat + REPEAT.len() - 1) % REPEAT.len(),
                "down" => self.repeat = (self.repeat + 1) % REPEAT.len(),
                "enter" | "space" | "escape" | "tab" => self.repeat_open = false,
                _ => {}
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        match k.key.as_str() {
            "left" | "right" | "home" | "end" | "backspace" | "delete"
                if self.kind == Kind::ScheduleTask && self.selected == 0 =>
            {
                let range = self.instruction_selection();
                let cursor = self.instruction_cursor;
                let next = match k.key.as_str() {
                    "home" => 0,
                    "end" => self.query.len(),
                    "left" | "backspace" => self.query[..cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i),
                    _ => {
                        cursor
                            + self.query[cursor..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8)
                    }
                };
                if matches!(k.key.as_str(), "backspace" | "delete") {
                    if range.is_empty() {
                        self.instruction_anchor = next;
                    }
                    self.replace_instructions("", cx);
                } else {
                    self.instruction_cursor = if !k.modifiers.shift && !range.is_empty() {
                        if k.key == "left" {
                            range.start
                        } else {
                            range.end
                        }
                    } else {
                        next
                    };
                    if !k.modifiers.shift {
                        self.instruction_anchor = self.instruction_cursor;
                    }
                    self.select_all = false;
                    self.reveal_instruction_cursor();
                    cx.notify();
                }
            }
            "escape" => cx.emit(Outcome::Dismiss),
            "tab" => {
                let count = match self.kind {
                    Kind::Shortcuts | Kind::FullAccess => 2,
                    Kind::ScheduleTask => 6,
                    _ => 3,
                };
                let order: &[usize] = if self.kind == Kind::ScheduleTask {
                    &[0, 3, 4, 5, 1, 2]
                } else {
                    &[0, 1, 2][..count]
                };
                let current = order.iter().position(|i| *i == self.selected).unwrap_or(0);
                self.selected =
                    order[(current + if k.modifiers.shift { count - 1 } else { 1 }) % count];
                self.controls[self.selected].focus(window, cx);
                cx.notify();
                self.select_all = false;
            }
            // Enter in the title input submits the form; Space types.
            "enter" if self.kind == Kind::Rename && self.selected == 0 => self.activate(cx),
            "enter" if self.kind == Kind::ScheduleTask && self.selected == 0 => {
                self.insert("\n", cx)
            }
            "enter" | "space"
                if matches!(self.kind, Kind::CloseWindow | Kind::FullAccess)
                    || (self.selected != 0 && !self.editing()) =>
            {
                self.activate(cx)
            }
            "backspace" if self.editing() && !self.saving => {
                if self.select_all {
                    self.input().clear();
                    self.select_all = false;
                } else {
                    self.input().pop();
                }
                self.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
            _ if !k.modifiers.platform && !k.modifiers.control => {
                if let Some(value) = &k.key_char {
                    self.insert(value, cx);
                }
            }
            _ => {}
        }
        cx.stop_propagation();
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        width: f32,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let primary = index == 1;
        div()
            .id(("dialog-button", index))
            .role(gpui::Role::Button)
            .aria_label(label)
            .track_focus(&self.controls[index])
            .w(px(width))
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(12.5))
            .bg(if primary { t.text } else { t.list_hover })
            .when(self.kind == Kind::FullAccess, |d| {
                d.h(px(36.)).rounded(px(18.)).bg(if primary {
                    t.text_error.opacity(0.1)
                } else {
                    t.list_hover
                })
            })
            .border_2()
            .border_color(if self.selected == index {
                rgba(0x3a83f7ff)
            } else {
                rgba(0)
            })
            .hover(|s| s.opacity(0.85))
            .child(text(
                label,
                14.,
                18.,
                430.,
                if self.kind == Kind::FullAccess && primary {
                    t.text_error
                } else if primary {
                    t.menu_background
                } else {
                    t.text
                },
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.controls[index].focus(window, cx);
                this.activate(cx);
            }))
    }
    fn full_access_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let mut capabilities = div()
            .p(px(12.))
            .rounded(px(16.))
            .bg(t.list_hover)
            .flex()
            .flex_col()
            .gap(px(12.));
        for (title, description) in [
            (
                "Files and folders",
                "Read, create, modify, upload, or delete files anywhere on this computer",
            ),
            (
                "Terminal commands",
                "Run commands, install software, and change system settings",
            ),
            (
                "Internet and connected apps",
                "Access websites, send data, and use enabled plugins",
            ),
        ] {
            capabilities = capabilities.child(
                div()
                    .flex()
                    .flex_col()
                    .child(text(title, 13., 18., 500., t.text))
                    .child(text(description, 13., 18., 400., t.text_secondary).whitespace_normal()),
            );
        }
        div().relative().size_full().p(px(20.)).flex().flex_col().gap(px(12.))
            .child(text("Turn on Full Access?", 20., 28., 600., t.text))
            .child(text("Codex will be able to run commands, use the internet, and create and edit files anywhere on this computer without your permission. This includes but is not limited to:", 14., 20., 400., t.text_secondary).whitespace_normal())
            .child(capabilities)
            .child(text("This comes with risks like loss or exposure of sensitive data and prompt injection. You can turn this off.", 14., 20., 400., t.text_secondary).whitespace_normal())
            .child(div().absolute().bottom(px(20.)).right(px(20.)).flex().gap(px(12.))
                .child(self.button(0, "Cancel", 88., t, cx))
                .child(self.button(1, "Confirm", 115., t, cx)))
    }

    fn close_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(
                text(
                    "Are you sure you want to close the window?",
                    20.,
                    28.,
                    600.,
                    t.text,
                )
                .whitespace_normal()
                .h(px(56.)),
            )
            .child(
                div()
                    .id("close-do-not-ask")
                    .role(gpui::Role::CheckBox)
                    .aria_label("Do not ask me again")
                    .aria_toggled(if self.do_not_ask_again {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .track_focus(&self.controls[2])
                    .mt(px(12.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(t.border)
                            .when(self.selected == 2, |d| d.border_color(rgba(0x3a83f7ff)))
                            .when(self.do_not_ask_again, |d| {
                                d.bg(t.text)
                                    .child(text("✓", 12., 12., 600., t.menu_background))
                            }),
                    )
                    .child(text("Do not ask me again", 13., 19., 430., t.text))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 2;
                        this.controls[2].focus(window, cx);
                        this.activate(cx);
                    })),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.button(0, "Cancel", 80., t, cx))
                    .child(self.button(1, "Close Window", 128., t, cx)),
            )
    }
    /// The local task editor keeps the draft until the host confirms the write.
    fn schedule_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let label = t.text.opacity(0.65);
        let valid = !self.query.trim().is_empty() && self.schedule_rule().is_some() && !self.saving;
        let field = |name: &'static str, value: SharedString, width: f32| {
            div()
                .h(px(48.))
                .px(px(20.))
                .flex()
                .items_center()
                .justify_between()
                .child(text(name, 14., 20., 400., t.text))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            text(value, 14., 20., 400., label)
                                .w(px(width))
                                .flex()
                                .justify_end(),
                        )
                        .child(icon("chevron-down", 20., label).ml(px(8.))),
                )
        };
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text("Schedule a task", 16., 26., 590., t.text))
            .child(
                div()
                    .id("schedule-instructions")
                    .role(gpui::Role::TextInput)
                    .aria_label("Instructions…")
                    .aria_value(self.query.clone())
                    .track_focus(&self.controls[0])
                    .mt(px(20.))
                    .h(px(122.))
                    .rounded(px(20.))
                    .border_1()
                    .border_color(t.border)
                    .px(px(18.))
                    .py(px(16.))
                    .cursor_text()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(
                        self.query
                            .split('\n')
                            .scan(0usize, |start, line| {
                                let offset = *start;
                                *start += line.len() + 1;
                                Some((offset, line))
                            })
                            .map(|(start, line)| {
                                let selection = self.instruction_selection();
                                let selected = selection.start.max(start)
                                    ..selection.end.min(start + line.len());
                                let input = gpui::StyledText::new(
                                    if self.query.is_empty() {
                                        "Instructions…"
                                    } else if line.is_empty() {
                                        " "
                                    } else {
                                        line
                                    }
                                    .to_owned(),
                                )
                                .with_highlights(
                                    (selected.start < selected.end).then(|| {
                                        (
                                            (selected.start - start)..(selected.end - start),
                                            gpui::HighlightStyle {
                                                background_color: Some(t.text_selection),
                                                ..Default::default()
                                            },
                                        )
                                    }),
                                );
                                let layout = input.layout().clone();
                                let caret = (self.selected == 0
                                    && selection.is_empty()
                                    && self.instruction_cursor >= start
                                    && self.instruction_cursor <= start + line.len())
                                .then_some(self.instruction_cursor.saturating_sub(start));
                                let color = t.text;
                                div()
                                    .relative()
                                    .min_h(px(20.))
                                    .text_size(px(14.))
                                    .line_height(px(20.))
                                    .text_color(if self.query.is_empty() {
                                        t.text_tertiary
                                    } else {
                                        t.text
                                    })
                                    .whitespace_normal()
                                    .child(input)
                                    .child(
                                        gpui::canvas(
                                            |_, _, _| (),
                                            move |_, (), window, _| {
                                                if let Some(cursor) = caret
                                                    && let Some(position) =
                                                        layout.position_for_index(cursor)
                                                {
                                                    window.paint_quad(gpui::fill(
                                                        gpui::Bounds::new(
                                                            position,
                                                            gpui::size(px(1.), px(18.)),
                                                        ),
                                                        color,
                                                    ));
                                                }
                                            },
                                        )
                                        .absolute()
                                        .inset_0(),
                                    )
                            }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
                        this.select_all = false;
                        this.controls[0].focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .mt(px(16.))
                    .h(px(98.))
                    .rounded(px(20.))
                    .border_1()
                    .border_color(t.border_strong)
                    .bg(t.surface_elevated_secondary.opacity(0.96))
                    .flex()
                    .flex_col()
                    .child(
                        field("Repeat", REPEAT[self.repeat].0.into(), 120.)
                            .id("schedule-repeat")
                            .role(gpui::Role::Button)
                            .aria_label("Repeat")
                            .track_focus(&self.controls[3])
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selected = 3;
                                this.controls[3].focus(window, cx);
                                this.activate(cx);
                            }))
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .left(px(16.))
                                    .right(px(16.))
                                    .h(px(1.))
                                    .bg(t.border),
                            ),
                    )
                    .child(
                        field(
                            "Time",
                            if self.selected == 4 || self.schedule_time != "09:00" {
                                format!("{} (local)", self.schedule_time).into()
                            } else {
                                schedule_time().into()
                            },
                            150.,
                        )
                        .id("schedule-time")
                        .role(gpui::Role::TextInput)
                        .aria_label("Time (24-hour HH:MM, local)")
                        .aria_value(self.schedule_time.clone())
                        .track_focus(&self.controls[4])
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.selected = 4;
                            this.select_all = true;
                            this.controls[4].focus(window, cx);
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div().mt(px(16.)).h(px(28.)).flex().child(
                    div()
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .child(text(
                            self.error
                                .as_deref()
                                .unwrap_or(if self.schedule_rule().is_none() {
                                    "Enter a local time in HH:MM format."
                                } else {
                                    "Local schedule"
                                })
                                .to_owned(),
                            14.,
                            18.,
                            400.,
                            if self.error.is_some() || self.schedule_rule().is_none() {
                                t.text_error
                            } else {
                                label
                            },
                        )),
                ),
            )
            .child(
                div()
                    .mt(px(20.))
                    .h(px(36.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("schedule-cancel")
                            .role(gpui::Role::Button)
                            .aria_label("Cancel")
                            .track_focus(&self.controls[1])
                            .w(px(80.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .bg(t.text_alpha_5)
                            .hover(|s| s.bg(t.text_alpha_10))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text("Cancel", 14., 20., 500., t.text))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
                    )
                    .child(
                        div()
                            .id("schedule-create")
                            .role(gpui::Role::Button)
                            .aria_label("Create")
                            .aria_disabled(!valid)
                            .track_focus(&self.controls[5])
                            .w(px(79.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .bg(t.text)
                            .border_1()
                            .border_color(t.border)
                            .opacity(if valid { 1. } else { 0.4 })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text(
                                if self.saving { "Creating…" } else { "Create" },
                                14.,
                                20.,
                                500.,
                                t.surface_elevated_secondary,
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selected = 5;
                                this.controls[5].focus(window, cx);
                                this.activate(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("schedule-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .track_focus(&self.controls[2])
                    .absolute()
                    .right(px(14.))
                    .top(px(16.))
                    .size(px(32.))
                    .child(ButtonVisual {
                        glyph: icon("close", 20., t.text_tertiary),
                        radius: px(16.),
                        hover: t.ghost_hover,
                        foreground: t.text,
                        selected: false,
                    })
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
            .when(self.repeat_open, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(20.))
                        .top(px(145.))
                        .w(px(190.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.menu_background)
                        .py(px(4.))
                        .children(REPEAT.iter().enumerate().map(|(index, (label, _))| {
                            div()
                                .id(("schedule-repeat-option", index))
                                .role(gpui::Role::MenuItem)
                                .aria_label(*label)
                                .h(px(28.))
                                .px(px(12.))
                                .flex()
                                .items_center()
                                .when(self.repeat == index, |d| d.bg(t.list_hover))
                                .hover(|s| s.bg(t.list_hover))
                                .child(text(*label, 14., 20., 400., t.text))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.repeat = index;
                                    this.repeat_open = false;
                                    cx.notify();
                                }))
                        })),
                )
            })
    }
    /// `Rename chat` (420 x 185, p-20): heading 20/28, hint 14/21 at 50 %, the 36 px
    /// `Chat title` input (13 px, 1 px ring, blue when focused), `Cancel` and `Save`, and
    /// `Close dialog` 24 px at the top right.
    fn rename_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let focused = self.selected == 0;
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text("Rename chat", 20., 28., 600., t.text))
            .child(
                text(
                    "Keep it short and recognizable",
                    14.,
                    21.,
                    430.,
                    t.text.opacity(0.498),
                )
                .mt(px(4.)),
            )
            .child(
                div()
                    .id("rename-title")
                    .role(gpui::Role::TextInput)
                    .aria_label("Chat title")
                    .aria_value(self.query.clone())
                    .aria_placeholder("Add a title…")
                    .track_focus(&self.controls[0])
                    .mt(px(12.))
                    .h(px(36.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if focused {
                        rgba(0x3a83f7ff).into()
                    } else {
                        t.border_strong
                    })
                    .bg(t.surface_elevated_secondary.opacity(0.96))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .cursor_text()
                    .child(
                        text(
                            if self.query.is_empty() {
                                "Add a title…".into()
                            } else {
                                self.query.clone()
                            },
                            13.,
                            18.5714,
                            430.,
                            if self.query.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .when(self.select_all && !self.query.is_empty(), |d| {
                            d.bg(rgba(0x3a83f766))
                        }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
                        this.controls[0].focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.button(2, "Cancel", 80., t, cx))
                    .child(self.button(1, "Save", 66., t, cx)),
            )
            .child(
                div()
                    .id("rename-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .absolute()
                    .right(px(16.))
                    .top(px(16.))
                    .size(px(24.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(t.ghost_hover))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("close", 16., t.text.opacity(0.8)))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
    }
    fn shortcuts_content(&self, visible: &[&Shortcut], t: &Theme, cx: &Context<Self>) -> Div {
        let mut list = div().flex().flex_col().pr(px(22.));
        let mut heading = "";
        for (index, &&(section, title, keys)) in visible.iter().enumerate() {
            if section != heading {
                list = list.child(
                    div()
                        .h(px(46.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .child(text(section, 14., 20., 500., t.text)),
                );
                heading = section;
            }
            list = list.child(
                div()
                    .h(px(30.))
                    .when(
                        visible.get(index + 1).is_some_and(|next| next.0 == section),
                        |d| d.mb(px(6.)),
                    )
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .opacity(0.75)
                    .child(text(title, 14., 18., 400., t.text))
                    .child(div().flex().gap(px(8.)).children(keys.iter().map(|key| {
                        div()
                            .rounded(px(6.))
                            .px(px(6.))
                            .py(px(2.))
                            .bg(t.list_hover)
                            .child(text(*key, 12., 12., 400., t.text))
                    }))),
            );
        }
        if visible.is_empty() {
            list = list
                .child(text("No matching shortcuts", 14., 20., 400., t.text_secondary).py(px(24.)));
        }
        div()
            .p(px(20.))
            .size_full()
            .flex()
            .flex_col()
            .child(text("Keyboard shortcuts", 20., 28., 600., t.text).flex_shrink_0())
            .child(
                div()
                    .id("shortcuts-search")
                    .role(gpui::Role::TextInput)
                    .aria_label("Search keyboard shortcuts")
                    .aria_value(self.query.clone())
                    .aria_placeholder("Search shortcuts")
                    .track_focus(&self.controls[0])
                    .mt(px(12.))
                    .h(px(32.))
                    .flex_shrink_0()
                    .rounded(px(999.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.menu_background)
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .overflow_hidden()
                    .child(icon("search", 16., t.text_secondary))
                    .child(
                        text(
                            if self.query.is_empty() {
                                "Search shortcuts".into()
                            } else {
                                self.query.clone()
                            },
                            14.,
                            18.,
                            400.,
                            if self.query.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .when(self.select_all, |d| d.bg(rgba(0x3a83f766))),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
                        this.controls[0].focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("shortcuts-list")
                    .mt(px(12.))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(list),
            )
            .child(
                div()
                    .id("shortcuts-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .track_focus(&self.controls[1])
                    .absolute()
                    .right(px(16.))
                    .top(px(16.))
                    .size(px(24.))
                    .rounded(px(6.))
                    .when(self.selected == 1, |d| {
                        d.border_2().border_color(rgba(0x3a83f7ff))
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text("×", 20., 24., 400., t.text_secondary))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
    }
}

const REPEAT: [(&str, &str); 8] = [
    ("Daily", ""),
    ("Mondays", "MO"),
    ("Tuesdays", "TU"),
    ("Wednesdays", "WE"),
    ("Thursdays", "TH"),
    ("Fridays", "FR"),
    ("Saturdays", "SA"),
    ("Sundays", "SU"),
];

/// The editor's default `Time`: 9:00 AM in the local zone, named like the original's
/// `Intl` short zone name (`PT`, `GMT+2`).
/// ponytail: US zones by offset, else `GMT±N`; a tz database lookup if other names matter.
fn schedule_time() -> String {
    let hours = chrono::Local::now().offset().local_minus_utc() / 3600;
    let zone = match hours {
        -8 | -7 => "PT".into(),
        -6 => "MT".into(),
        -5 => "CT".into(),
        -4 => "ET".into(),
        0 => "GMT".into(),
        h => format!("GMT{h:+}"),
    };
    format!("9:00 AM {zone}")
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let viewport = window.viewport_size();
        let visible: Vec<_> = if self.kind == Kind::Shortcuts {
            self.visible().collect()
        } else {
            Vec::new()
        };
        let mut previous = "";
        let mut sections = 0;
        for &&(section, _, _) in &visible {
            if previous != section {
                sections += 1;
                previous = section;
            }
        }
        let shortcuts_height =
            (124. + visible.len() as f32 * 36. + sections as f32 * 40.).clamp(520., 560.);
        let schedule = self.kind == Kind::ScheduleTask;
        let (width, height): (f32, f32) = match self.kind {
            Kind::Shortcuts => (600., shortcuts_height),
            Kind::CloseWindow => (420., 171.),
            Kind::ScheduleTask => (680., 422.),
            Kind::Rename => (420., 185.),
            Kind::FullAccess => (520., 420.),
        };
        let width = width.min(f32::from(viewport.width) * 0.92);
        let height = height.min(f32::from(viewport.height) * 0.92);
        let content = match self.kind {
            Kind::Shortcuts => self.shortcuts_content(&visible, t, cx),
            Kind::CloseWindow => self.close_content(t, cx),
            Kind::ScheduleTask => self.schedule_content(t, cx),
            Kind::Rename => self.rename_content(t, cx),
            Kind::FullAccess => self.full_access_content(t, cx),
        };
        // `codex-dialog-overlay`: the schedule dialog's `_dimmedOverlay` is rgba(0,0,0,.5) with
        // blur(1px); the blur is not drawn (flat fills are unchanged by it).
        let top = if schedule {
            83.
        } else {
            (f32::from(viewport.height) - height) / 2.
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(if schedule {
                t.dialog_backdrop
            } else {
                rgba(0x00000022).into()
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss)),
            )
            .child(
                div()
                    .id("modal-dialog")
                    .role(gpui::Role::Dialog)
                    .aria_label(match self.kind {
                        Kind::Shortcuts => "Keyboard shortcuts",
                        Kind::CloseWindow => "Are you sure you want to close the window?",
                        Kind::ScheduleTask => "Schedule a task",
                        Kind::Rename => "Rename chat",
                        Kind::FullAccess => "Turn on Full Access?",
                    })
                    .on_key_down(cx.listener(Self::key))
                    .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                        this.select_all = true;
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Paste, _, cx| {
                        if let Some(s) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            this.insert(&s, cx);
                        }
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Copy, _, cx| {
                        if this.kind == Kind::ScheduleTask && this.selected == 0 {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.query[this.instruction_selection()].to_owned(),
                            ));
                            cx.stop_propagation();
                            return;
                        }
                        if this.select_all && this.editing() {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.input().clone(),
                            ));
                        }
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Cut, _, cx| {
                        if this.kind == Kind::ScheduleTask && this.selected == 0 && !this.saving {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.query[this.instruction_selection()].to_owned(),
                            ));
                            this.replace_instructions("", cx);
                            cx.stop_propagation();
                            return;
                        }
                        if this.select_all && this.editing() && !this.saving {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(std::mem::take(
                                this.input(),
                            )));
                            this.select_all = false;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left(px((f32::from(viewport.width) - width) / 2.))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(if schedule { 30. } else { 24. }))
                    .overflow_hidden()
                    .map(|d| {
                        if !schedule {
                            d.child(shapes::backdrop_blur(
                                px(24.),
                                shapes::Fill::Color,
                                false,
                                shapes::Decoration {
                                    radius: Corners::all(px(24.)),
                                    fill: Some(gpui::Hsla {
                                        a: 0.9,
                                        ..t.menu_background
                                    }),
                                    shadows: vec![
                                        crate::composer::shadow(t.border, 0., 0., 0.5, false),
                                        crate::composer::shadow(
                                            rgba(0x0000001a).into(),
                                            4.,
                                            8.,
                                            -2.,
                                            false,
                                        ),
                                    ],
                                    ..Default::default()
                                },
                            ))
                        } else {
                            d.bg(t.surface_elevated_secondary)
                        }
                    })
                    .child(content),
            )
    }
}

const SHORTCUTS: &[(&str, &str, &[&str])] = &[
    ("Chat", "New chat", &["⌘N", "⇧⌘O"]),
    ("Chat", "Archive chat", &["⇧⌘A"]),
    ("Chat", "New standalone chat", &["⌥⌘O"]),
    ("Chat", "Toggle pin", &["⌥⌘P"]),
    ("Navigation", "Find", &["⌘F"]),
    ("Navigation", "Focus tab 1", &["⌘1"]),
    ("Navigation", "Focus tab 2", &["⌘2"]),
    ("Navigation", "Focus tab 3", &["⌘3"]),
    ("Navigation", "Focus tab 4", &["⌘4"]),
    ("Navigation", "Focus tab 5", &["⌘5"]),
    ("Navigation", "Focus tab 6", &["⌘6"]),
    ("Navigation", "Focus tab 7", &["⌘7"]),
    ("Navigation", "Focus tab 8", &["⌘8"]),
    ("Navigation", "Focus tab 9", &["⌘9"]),
    ("Navigation", "Back", &["⌘[", "Mouse Back"]),
    ("Navigation", "Forward", &["⌘]", "Mouse Forward"]),
    ("Navigation", "Next recently viewed chat", &["⌃Tab"]),
    ("Navigation", "Next tab", &["⌃Tab", "⇧⌘]", "⌥⌘→"]),
    ("Navigation", "Next chat", &["⇧⌘]", "⌥⌘→"]),
    ("Navigation", "Next chat needing attention", &["⌥⌘A"]),
    ("Navigation", "Open your dot", &["⌘."]),
    ("Navigation", "Previous recently viewed chat", &["⌃⇧Tab"]),
    ("Navigation", "Previous tab", &["⌃⇧Tab", "⇧⌘[", "⌥⌘←"]),
    ("Navigation", "Previous chat", &["⇧⌘[", "⌥⌘←"]),
    ("Navigation", "Go to recent chat 1", &["⌥⌘1"]),
    ("Navigation", "Go to recent chat 2", &["⌥⌘2"]),
    ("Navigation", "Go to recent chat 3", &["⌥⌘3"]),
    ("Navigation", "Go to recent chat 4", &["⌥⌘4", "⇧⌘9"]),
    ("Navigation", "Go to recent chat 5", &["⌥⌘5", "⇧⌘8"]),
    ("Navigation", "Go to recent chat 6", &["⌥⌘6"]),
    ("Navigation", "Toggle activity view", &["⌥⌘U"]),
    ("Panels", "Open browser tab", &["⌘T"]),
    ("Panels", "Reopen closed tab", &["⇧⌘T"]),
    ("Panels", "New tab in full view", &["⇧⌘F"]),
    ("Panels", "Cycle workspace layout", &["⇧⌘B"]),
    ("Panels", "Toggle bottom panel", &["⌘J"]),
    ("Panels", "Toggle navigation panel", &["⇧⌘E"]),
    ("Panels", "Toggle sidebar", &["⇧⌘S", "⌘B"]),
    ("Panels", "New tab", &["⌥⌘B"]),
    ("Panels", "Open terminal", &["⌃`"]),
    ("Project", "Open folder", &["⌘O"]),
    ("App", "Clear all unreads", &["⇧Esc"]),
    ("App", "Show Mini", &["⌥Space"]),
    ("App", "Settings", &["⌘,"]),
    ("General", "Close other tabs", &["⌥⌘W"]),
    ("General", "Close Tab", &["⌘W"]),
    ("General", "Close", &["⌘W"]),
    ("General", "Select effort", &["⌃⇧M"]),
    ("General", "Open project picker", &["⌥⇧⌘O"]),
    ("General", "Open recent models", &["⌃⌘M"]),
    ("General", "Toggle voice chat", &["⌃⇧V"]),
    ("General", "Send message in background", &["⌘⏎"]),
    ("General", "Copy conversation path", &["⌥⇧⌘C"]),
    ("General", "Copy deeplink", &["⌥⌘L"]),
    ("General", "Copy working directory", &["⇧⌘C"]),
    ("General", "Force Reload Browser Page", &["⇧⌘R"]),
    ("General", "Voice in Mini", &["⌥V"]),
    ("General", "Open command menu", &["⌘K", "⇧⌘P"]),
    ("General", "Reload Browser Page", &["⌘R"]),
    ("General", "Rename chat", &["⌥⌘R"]),
    ("General", "Screenshot selection to current chat", &["⌃⇧4"]),
    ("General", "Screenshot selection to new chat", &["⌃⌥⇧4"]),
    ("General", "Search Files…", &["⌘P"]),
    ("General", "Show keyboard shortcuts", &["⌘/"]),
    ("General", "Go to chat 1", &["⌃1"]),
    ("General", "Go to chat 2", &["⌃2"]),
    ("General", "Go to chat 3", &["⌃3"]),
    ("General", "Go to chat 4", &["⌃4"]),
    ("General", "Go to chat 5", &["⌃5"]),
    ("General", "Go to chat 6", &["⌃6"]),
    ("General", "Go to chat 7", &["⌃7"]),
    ("General", "Go to chat 8", &["⌃8"]),
    ("General", "Go to chat 9", &["⌃9"]),
    ("General", "Toggle Notch", &["⌥C"]),
];
