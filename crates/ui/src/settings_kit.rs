mod select;
pub use select::{Select, Selected};

use crate::{composer::popover, focus, icon::icon, text};
use gpui::{
    Anchor, App, Context, Div, ElementId, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use std::time::{Duration, Instant};
use theme::{ActiveTheme as _, Theme};

pub struct Choice {
    pub value: SharedString,
    pub label: SharedString,
    pub checked: bool,
    pub disabled: bool,
}

pub struct SelectionChanged {
    pub value: SharedString,
    pub checked: bool,
}

pub struct MultiSelect {
    label: SharedString,
    choices: Vec<Choice>,
    trigger: FocusHandle,
    items: Vec<FocusHandle>,
    open: bool,
    menu_hovered: bool,
    search: String,
    last_typed: Option<Instant>,
    /// Shows the checked labels (or this text when none is checked) instead of a count.
    summary_off: Option<SharedString>,
}

impl EventEmitter<SelectionChanged> for MultiSelect {}

impl Focusable for MultiSelect {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.trigger.clone()
    }
}

impl MultiSelect {
    pub fn new(
        label: impl Into<SharedString>,
        choices: Vec<Choice>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            label: label.into(),
            trigger: cx.focus_handle().tab_stop(true),
            items: choices.iter().map(|_| cx.focus_handle()).collect(),
            choices,
            open: false,
            menu_hovered: false,
            search: String::new(),
            last_typed: None,
            summary_off: None,
        }
    }

    /// The trigger lists the checked labels, `, `-joined, or `off` when none is checked.
    pub fn summarize(mut self, off: impl Into<SharedString>) -> Self {
        self.summary_off = Some(off.into());
        self
    }

    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    pub fn set_checked(&mut self, value: &str, checked: bool, cx: &mut Context<Self>) {
        if let Some(choice) = self.choices.iter_mut().find(|choice| choice.value == value)
            && choice.checked != checked
        {
            choice.checked = checked;
            cx.notify();
        }
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        let choice = &mut self.choices[index];
        if !choice.disabled {
            choice.checked = !choice.checked;
            let event = SelectionChanged {
                value: choice.value.clone(),
                checked: choice.checked,
            };
            cx.emit(event);
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.menu_hovered = false;
        self.search.clear();
        self.trigger.focus(window, cx);
        cx.notify();
    }

    fn move_focus(&self, forward: bool, window: &mut Window, cx: &mut App) {
        let current = self.items.iter().position(|item| item.is_focused(window));
        let next = if forward {
            (current.map_or(0, |index| index + 1)..self.items.len())
                .find(|&index| !self.choices[index].disabled)
        } else {
            (0..current.unwrap_or(self.items.len()))
                .rev()
                .find(|&index| !self.choices[index].disabled)
        };
        if let Some(index) = next {
            self.items[index].focus(window, cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.platform || key.modifiers.control || key.modifiers.alt {
            return;
        }
        if !self.open {
            match key.key.as_str() {
                "enter" | "space" | "down" | "up" => {
                    self.open = true;
                    self.move_focus(key.key != "up", window, cx);
                    cx.notify();
                }
                _ => return,
            }
        } else {
            match key.key.as_str() {
                "escape" => self.close(window, cx),
                "tab" => {
                    self.close(window, cx);
                    if key.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                }
                "up" => self.move_focus(false, window, cx),
                "down" => self.move_focus(true, window, cx),
                "home" | "end" => {
                    let first = key.key == "home";
                    self.trigger.focus(window, cx);
                    self.move_focus(first, window, cx);
                }
                "enter" | "space" => {
                    if let Some(index) = self.items.iter().position(|item| item.is_focused(window))
                    {
                        self.toggle(index, cx);
                    }
                }
                _ => {
                    let Some(value) = &key.key_char else {
                        return;
                    };
                    let now = cx.background_executor().now();
                    if self
                        .last_typed
                        .is_none_or(|last| now - last > Duration::from_secs(1))
                    {
                        self.search.clear();
                    }
                    self.search
                        .extend(value.chars().flat_map(char::to_lowercase));
                    self.last_typed = Some(now);
                    if let Some(index) = self.choices.iter().position(|choice| {
                        if choice.disabled {
                            return false;
                        }
                        let mut label = choice.label.chars().flat_map(char::to_lowercase);
                        self.search
                            .chars()
                            .all(|character| label.next() == Some(character))
                    }) {
                        self.items[index].focus(window, cx);
                    }
                }
            }
        }
        cx.stop_propagation();
    }
}

impl Render for MultiSelect {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let count = self.choices.iter().filter(|choice| choice.checked).count();
        let summary: SharedString = match &self.summary_off {
            Some(off) if count == 0 => off.clone(),
            Some(_) => self
                .choices
                .iter()
                .filter(|choice| choice.checked)
                .map(|choice| choice.label.as_ref())
                .collect::<Vec<_>>()
                .join(", ")
                .into(),
            None => format!("{count} selected").into(),
        };
        let mut trigger = div()
            .id("settings-multiselect-trigger")
            .role(gpui::Role::Button)
            .aria_label(self.label.clone())
            .aria_value(format!("{count} selected"))
            .track_focus(&self.trigger)
            .relative()
            .h(px(28.))
            .px(px(13.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(12.5)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.primary_soft_alpha),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .when(
                self.trigger.is_focused(window) && window.last_input_was_keyboard(),
                |d| {
                    d.child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(px(12.5)),
                        border: Some((px(2.), t.focus_ring)),
                        ..Default::default()
                    }))
                },
            )
            .child(text(summary, 13., 18., 430., t.text))
            .child(icon("chevron-down", 14., t.text_tertiary))
            .capture_key_down(cx.listener(Self::key))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.open {
                    this.close(window, cx);
                } else {
                    this.open = true;
                    this.trigger.focus(window, cx);
                    cx.notify();
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if this.open && !this.menu_hovered {
                    this.close(window, cx);
                }
            }));
        if self.open {
            let rows = self.choices.iter().enumerate().map(|(index, choice)| {
                div()
                    .id(("settings-multiselect-option", index))
                    .role(gpui::Role::MenuItemCheckBox)
                    .aria_label(choice.label.clone())
                    .aria_toggled(if choice.checked {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .aria_disabled(choice.disabled)
                    .track_focus(&self.items[index])
                    .rounded(px(15.))
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(choice.disabled, |d| d.opacity(0.5))
                    .when(!choice.disabled, |d| d.hover(|s| s.bg(t.menu_item_hover)))
                    .when(self.items[index].is_focused(window), |d| {
                        d.bg(t.menu_item_hover)
                    })
                    .child(text(choice.label.clone(), 13., 18.571_428, 430., t.text).flex_1())
                    .child(
                        div()
                            .size(px(16.))
                            .children(choice.checked.then(|| icon("menu-check", 16., t.text))),
                    )
                    .capture_key_down(cx.listener(Self::key))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.choices[index].disabled {
                            this.items[index].focus(window, cx);
                            this.toggle(index, cx);
                        }
                    }))
            });
            trigger = trigger.child(deferred(
                div().absolute().top_0().right_0().child(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(32.)))
                        .child(
                            div()
                                .id("settings-multiselect-menu")
                                .role(gpui::Role::Menu)
                                .aria_label(self.label.clone())
                                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                    this.menu_hovered = *hovered
                                }))
                                .child(popover(&t, 280., |d| d.children(rows))),
                        ),
                ),
            ));
        }
        trigger
    }
}

pub fn external_link_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    url: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let title = title.into();
    let url = url.into();
    let focus = focus::handle(id.clone(), 0, window, cx);
    let handle = focus.read(cx).handle.clone();
    let t = cx.theme();
    focus::ring(
        div()
            .id(id)
            .role(gpui::Role::Link)
            .aria_label(title.clone())
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .px(px(16.))
            .py(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(title, 13., 18.571_428, 500., t.text))
                    .children(
                        description
                            .map(|description| text(description, 12., 16., 430., t.text_secondary)),
                    ),
            )
            .child(icon("arrow-up-right", 16., t.text_tertiary)),
        &focus,
        Theme::RADIUS_LG,
        px(0.),
        cx,
    )
    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
        handle.focus(window, cx)
    })
    .on_click(move |_, _, cx| cx.open_url(&url))
}
