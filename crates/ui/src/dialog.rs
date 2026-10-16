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
    /// `Schedule a task`: the Scheduled sidebar's `New task` create editor (AM-005).
    ScheduleTask,
}
#[derive(Clone, Copy)]
pub enum Outcome {
    Dismiss,
    CloseWindow { do_not_ask_again: bool },
}

pub struct Dialog {
    kind: Kind,
    controls: [FocusHandle; 3],
    selected: usize,
    query: String,
    select_all: bool,
    do_not_ask_again: bool,
    scroll: ScrollHandle,
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
            do_not_ask_again: false,
            scroll: ScrollHandle::new(),
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
            (Kind::ScheduleTask, 0) => "Instructions…",
            (Kind::ScheduleTask, 1) => "Cancel",
            (Kind::ScheduleTask, _) => "Close dialog",
        }
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
            (Kind::CloseWindow, 1) => cx.emit(Outcome::CloseWindow {
                do_not_ask_again: self.do_not_ask_again,
            }),
            (Kind::CloseWindow, 2) => {
                self.do_not_ask_again = !self.do_not_ask_again;
                cx.notify();
            }
            _ => cx.emit(Outcome::Dismiss),
        }
    }
    fn insert(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.kind != Kind::Shortcuts || self.selected != 0 {
            return;
        }
        if self.select_all {
            self.query.clear();
            self.select_all = false;
        }
        self.query.extend(value.chars().filter(|c| !c.is_control()));
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match k.key.as_str() {
            "escape" => cx.emit(Outcome::Dismiss),
            "tab" => {
                let count = if self.kind == Kind::Shortcuts { 2 } else { 3 };
                self.selected =
                    (self.selected + if k.modifiers.shift { count - 1 } else { 1 }) % count;
                self.controls[self.selected].focus(window, cx);
                cx.notify();
            }
            "enter" | "space" if self.kind == Kind::CloseWindow || self.selected != 0 => {
                self.activate(cx)
            }
            "backspace" if self.kind == Kind::Shortcuts && self.selected == 0 => {
                if self.select_all {
                    self.query.clear();
                    self.select_all = false;
                } else {
                    self.query.pop();
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
                if primary { t.menu_background } else { t.text },
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.controls[index].focus(window, cx);
                this.activate(cx);
            }))
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
    /// `Schedule a task` (editor-dialog chunk): instructions editor, the `Repeat` / `Time`
    /// field group, `Advanced`, `Cancel` and the disabled `Create`. Only the open and close
    /// paths of AM-005 live here; the fields' menus and typing are other rows.
    fn schedule_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let label = t.text.opacity(0.65);
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
                    .track_focus(&self.controls[0])
                    .mt(px(20.))
                    .h(px(122.))
                    .rounded(px(20.))
                    .border_1()
                    .border_color(t.border)
                    .px(px(18.))
                    .py(px(16.))
                    .cursor_text()
                    .child(text("Instructions…", 14., 20., 400., t.text_tertiary))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
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
                        field("Repeat", "Daily".into(), 32.).relative().child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left(px(16.))
                                .right(px(16.))
                                .h(px(1.))
                                .bg(t.border),
                        ),
                    )
                    .child(field("Time", schedule_time().into(), 115.)),
            )
            .child(
                div().mt(px(16.)).h(px(28.)).flex().child(
                    div()
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .child(text("Advanced", 14., 18., 400., label))
                        .child(icon("chevron-down", 20., label).ml(px(4.))),
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
                            .aria_disabled(true)
                            .w(px(79.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .bg(t.text)
                            .border_1()
                            .border_color(t.border)
                            .opacity(0.4)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text("Create", 14., 20., 500., t.surface_elevated_secondary)),
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
        };
        let width = width.min(f32::from(viewport.width) * 0.92);
        let height = height.min(f32::from(viewport.height) * 0.92);
        let content = match self.kind {
            Kind::Shortcuts => self.shortcuts_content(&visible, t, cx),
            Kind::CloseWindow => self.close_content(t, cx),
            Kind::ScheduleTask => self.schedule_content(t, cx),
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
                        if this.select_all {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.query.clone(),
                            ));
                        }
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Cut, _, cx| {
                        if this.select_all {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(std::mem::take(
                                &mut this.query,
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
