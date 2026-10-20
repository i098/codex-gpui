use crate::{composer::popover, focus, icon::icon, text};
use gpui::{
    Anchor, AnyElement, App, ClickEvent, Context, Div, ElementId, EventEmitter, FocusHandle,
    Focusable, Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    Render, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, anchored, deferred,
    div, point, prelude::FluentBuilder as _, px,
};
use std::time::{Duration, Instant};
use theme::{ActiveTheme as _, Theme};

#[derive(PartialEq, Eq)]
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
    disabled: bool,
    menu_width: f32,
    row_padding: f32,
    menu_offset: f32,
    menu_margin: f32,
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
            disabled: false,
            menu_width: 280.,
            row_padding: 6.,
            menu_offset: 32.,
            menu_margin: 0.,
        }
    }

    pub fn menu_width(mut self, width: f32) -> Self {
        self.menu_width = width;
        self
    }

    pub fn row_padding(mut self, padding: f32) -> Self {
        self.row_padding = padding;
        self
    }

    pub fn menu_offset(mut self, offset: f32) -> Self {
        self.menu_offset = offset;
        self
    }

    pub fn menu_margin(mut self, margin: f32) -> Self {
        self.menu_margin = margin;
        self
    }

    pub fn set_choices(&mut self, choices: Vec<Choice>, cx: &mut Context<Self>) {
        if self.choices == choices {
            return;
        }
        if self.choices.len() != choices.len()
            || self
                .choices
                .iter()
                .zip(&choices)
                .any(|(old, new)| old.value != new.value)
        {
            self.items = choices
                .iter()
                .map(|choice| {
                    self.choices
                        .iter()
                        .position(|old| old.value == choice.value)
                        .map(|index| self.items[index].clone())
                        .unwrap_or_else(|| cx.focus_handle())
                })
                .collect();
            self.search.clear();
        }
        self.choices = choices;
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
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
        if !self.disabled && !choice.disabled {
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
        if key.modifiers.platform
            || key.modifiers.control
            || key.modifiers.alt
            || (self.disabled && !matches!(key.key.as_str(), "escape" | "tab"))
        {
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
        let mut trigger = select_control(
            text(format!("{count} selected"), 13., 18., 430., t.text),
            None,
            self.disabled,
            if self.open {
                t.ghost_hover
            } else {
                t.primary_soft_alpha
            },
            &t,
            icon("chevron-down", 14., t.text_tertiary).when(self.open, |icon| icon.runtime()),
        )
        .id("settings-multiselect-trigger")
        .role(gpui::Role::Button)
        .aria_label(self.label.clone())
        .aria_value(format!("{count} selected"))
        .aria_disabled(self.disabled)
        .track_focus(&self.trigger)
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
        .capture_key_down(cx.listener(Self::key))
        .on_click(cx.listener(|this, _, window, cx| {
            if this.disabled {
                return;
            }
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
                    .aria_disabled(self.disabled || choice.disabled)
                    .track_focus(&self.items[index])
                    .rounded(px(15.))
                    .px(px(8.))
                    .py(px(self.row_padding))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(self.disabled || choice.disabled, |d| d.opacity(0.5))
                    .when(!self.disabled && !choice.disabled, |d| {
                        d.hover(|s| s.bg(t.menu_item_hover))
                    })
                    .when(self.items[index].is_focused(window), |d| {
                        d.bg(t.menu_item_hover)
                    })
                    .child(text(choice.label.clone(), 13., 18.571_428, 430., t.text).flex_1())
                    .child(
                        div().size(px(16.)).children(
                            choice
                                .checked
                                .then(|| icon("menu-check", 16., t.text).runtime()),
                        ),
                    )
                    .capture_key_down(cx.listener(Self::key))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.disabled && !this.choices[index].disabled {
                            this.items[index].focus(window, cx);
                            this.toggle(index, cx);
                        }
                    }))
            });
            trigger = trigger.child(deferred(
                div().absolute().top_0().right_0().child(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(self.menu_offset)))
                        .child(
                            div()
                                .id("settings-multiselect-menu")
                                .p(px(self.menu_margin))
                                .snap_origin()
                                .role(gpui::Role::Menu)
                                .aria_label(self.label.clone())
                                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                    this.menu_hovered = *hovered
                                }))
                                .child(popover(&t, self.menu_width, |d| d.children(rows))),
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

pub fn select_control(
    label: Div,
    leading: Option<AnyElement>,
    disabled: bool,
    fill: Hsla,
    t: &Theme,
    chevron: impl IntoElement,
) -> Div {
    div()
        .h(px(28.))
        .px(px(13.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(4.))
        .relative()
        .child(shapes::decoration(shapes::Decoration {
            radius: gpui::Corners::all(px(12.5)),
            corner: shapes::Corner::Squircle,
            fill: Some(fill),
            border: Some((px(1.), t.border)),
            ..Default::default()
        }))
        .when(disabled, |d| d.opacity(0.4))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .children(leading)
                .child(label),
        )
        .child(chevron)
}

pub fn section_header(title: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .h(px(46.))
        .pb(px(6.))
        .flex()
        .items_center()
        .child(text(title, 14., 21., 500., t.text))
}

pub fn external_action_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    enabled: bool,
    action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let title = title.into();
    let t = cx.theme();
    let control = div()
        .id(id.clone())
        .role(gpui::Role::Button)
        .aria_label(title.clone())
        .aria_disabled(!enabled)
        .relative()
        .h(px(28.))
        .px(px(9.))
        .flex()
        .items_center()
        .gap(px(4.))
        .child(text(title, 13., 18., 430., t.text_tertiary))
        .child(icon("arrow-up-right", 14., t.text_tertiary).runtime());
    if !enabled {
        return control.opacity(0.4);
    }
    let state = focus::handle(id, 0, window, cx);
    let handle = state.read(cx).handle.clone();
    focus::ring(control, &state, px(12.5), px(2.), cx)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            handle.focus(window, cx)
        })
        .on_click(action)
}
