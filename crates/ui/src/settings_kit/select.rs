use super::Choice;
use crate::{
    composer::{Composer, popover},
    text,
};
use gpui::{
    Anchor, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme as _, Theme};

pub struct Selected(pub SharedString);

pub struct Select {
    value: SharedString,
    choices: Vec<Choice>,
    trigger: FocusHandle,
    items: Vec<FocusHandle>,
    search: Option<Entity<Composer>>,
    scroll: ScrollHandle,
    open: bool,
    menu_hovered: bool,
    highlighted: Option<usize>,
    default_action: bool,
    loading: bool,
    disabled: bool,
    error: Option<SharedString>,
    query: String,
    search_labels: Vec<String>,
    has_aliases: bool,
    icons: Option<fn(&str, &Theme) -> Option<gpui::AnyElement>>,
    details: std::collections::HashMap<&'static str, &'static str>,
    width: f32,
    /// A flattened submenu's title over the options after a pinned default.
    heading: Option<&'static str>,
}

impl EventEmitter<Selected> for Select {}
impl Focusable for Select {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.trigger.clone()
    }
}

impl Select {
    pub fn new(
        value: impl Into<SharedString>,
        choices: Vec<Choice>,
        search_placeholder: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = search_placeholder
            .map(|placeholder| cx.new(|cx| Composer::settings_search(placeholder, cx)));
        if let Some(search) = &search {
            cx.observe(search, |this, search, cx| {
                this.query = search.read(cx).field_text().trim().to_lowercase();
                if this.open {
                    this.highlighted = this.first_match();
                }
                cx.notify();
            })
            .detach();
        }
        Self {
            value: value.into(),
            items: choices.iter().map(|_| cx.focus_handle()).collect(),
            search_labels: choices
                .iter()
                .map(|choice| choice.label.to_lowercase())
                .collect(),
            choices,
            trigger: cx.focus_handle().tab_stop(true),
            search,
            scroll: ScrollHandle::new(),
            open: false,
            menu_hovered: false,
            highlighted: None,
            default_action: false,
            loading: false,
            disabled: false,
            error: None,
            query: String::new(),
            has_aliases: false,
            icons: None,
            details: std::collections::HashMap::new(),
            width: 240.,
            heading: None,
        }
    }

    pub fn with_icons(mut self, icons: fn(&str, &Theme) -> Option<gpui::AnyElement>) -> Self {
        self.icons = Some(icons);
        self
    }

    pub fn with_details(mut self, details: &'static [(&'static str, &'static str)]) -> Self {
        self.details = details.iter().copied().collect();
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn keep_default_visible(mut self) -> Self {
        self.default_action = true;
        self
    }

    /// Shows the options after the pinned default under `heading`, as a flattened submenu.
    pub fn with_heading(mut self, heading: &'static str) -> Self {
        self.heading = Some(heading);
        self
    }

    pub fn default_value(&self) -> &str {
        self.choices
            .first()
            .map_or("", |choice| choice.value.as_ref())
    }

    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    fn activate_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.items.iter().position(|item| item.is_focused(window));
        if let Some(index) = self.highlighted.or(focused).or_else(|| self.first_match()) {
            self.choose(index, window, cx);
        }
    }

    pub fn set_value(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        let value = value.into();
        if self.value != value {
            self.value = value;
            cx.notify();
        }
    }

    pub fn set_choices(&mut self, choices: Vec<Choice>, cx: &mut Context<Self>) {
        self.items = choices.iter().map(|_| cx.focus_handle()).collect();
        self.search_labels = choices
            .iter()
            .map(|choice| choice.label.to_lowercase())
            .collect();
        self.has_aliases = false;
        self.choices = choices;
        self.loading = false;
        self.highlighted = self.open.then(|| self.first_match()).flatten();
        self.error = None;
        cx.notify();
    }

    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        self.error = None;
        cx.notify();
    }

    /// A disabled select keeps its value but ignores pointer and keyboard input.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            self.open &= !disabled;
            cx.notify();
        }
    }

    pub fn set_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.loading = false;
        self.error = Some(error.into());
        cx.notify();
    }

    pub fn search_aliases(
        &mut self,
        build: impl FnOnce() -> std::collections::HashMap<String, String>,
    ) {
        if self.has_aliases {
            return;
        }
        let aliases = build();
        for (choice, label) in self.choices.iter().zip(&mut self.search_labels) {
            if let Some(alias) = aliases.get(choice.value.as_ref()) {
                label.push('\0');
                label.push_str(&alias.to_lowercase());
            }
        }
        self.has_aliases = true;
    }

    fn matches(&self, index: usize) -> bool {
        self.default_action && index == 0 || self.search_labels[index].contains(&self.query)
    }

    fn first_match(&self) -> Option<usize> {
        (0..self.choices.len()).find(|&index| {
            !(self.default_action && index == 0)
                && !self.choices[index].disabled
                && self.matches(index)
        })
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.menu_hovered = false;
        self.highlighted = None;
        self.trigger.focus(window, cx);
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.set_text("", cx));
        }
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choices.get(index) else {
            return;
        };
        if self.loading || self.disabled || choice.disabled {
            return;
        }
        self.value = choice.value.clone();
        cx.emit(Selected(self.value.clone()));
        self.close(window, cx);
    }

    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .highlighted
            .or_else(|| self.items.iter().position(|item| item.is_focused(window)));
        let next = if forward {
            (current.map_or(0, |index| index + 1)..self.items.len())
                .find(|&index| !self.choices[index].disabled && self.matches(index))
        } else {
            (0..current.unwrap_or(self.items.len()))
                .rev()
                .find(|&index| !self.choices[index].disabled && self.matches(index))
        };
        if let Some(index) = next {
            self.highlighted = Some(index);
            if self.search.is_none() {
                self.items[index].focus(window, cx);
            }
            let scroll_index = if self.default_action {
                index.saturating_sub(1)
            } else {
                index
            };
            self.scroll.scroll_to_item(scroll_index);
            cx.notify();
        }
    }

    fn open(&mut self, keyboard: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        if let Some(search) = &self.search {
            self.highlighted = self.first_match();
            search.focus_handle(cx).focus(window, cx);
        } else if keyboard {
            self.move_focus(true, window, cx);
        } else {
            self.trigger.focus(window, cx);
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.platform || key.modifiers.control || key.modifiers.alt {
            return;
        }
        if !self.open {
            if self.disabled {
                return;
            }
            if matches!(key.key.as_str(), "enter" | "space" | "up" | "down") {
                self.open(true, window, cx);
            } else {
                return;
            }
        } else if !self.menu_key(key, window, cx) {
            return;
        }
        cx.stop_propagation();
    }

    fn menu_key(
        &mut self,
        key: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match key.key.as_str() {
            "escape" => self.close(window, cx),
            "down" => self.move_focus(true, window, cx),
            "up" => self.move_focus(false, window, cx),
            "home" | "end" if self.search.is_none() => {
                self.trigger.focus(window, cx);
                self.highlighted = None;
                self.move_focus(key.key == "home", window, cx);
            }
            "enter" => self.activate_focused(window, cx),
            "space"
                if self
                    .search
                    .as_ref()
                    .is_none_or(|search| !search.focus_handle(cx).is_focused(window)) =>
            {
                self.choose_focused(window, cx);
            }
            "tab" => self.tab_out(key.modifiers.shift, window, cx),
            _ => return false,
        }
        true
    }

    fn choose_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.items.iter().position(|item| item.is_focused(window)) {
            self.choose(index, window, cx);
        }
    }

    fn tab_out(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        if backwards {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
    }

    fn option(
        &self,
        index: usize,
        choice: &Choice,
        window: &Window,
        t: &Theme,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let detail = self.details.get(choice.value.as_ref()).copied();
        let mut leading = self.icons.and_then(|icon| icon(&choice.value, t));
        let inline_icon = if detail.is_some() {
            leading.take()
        } else {
            None
        };
        let highlighted = self.highlighted == Some(index) || self.items[index].is_focused(window);
        let row = div()
            .id(("select-option", index))
            .role(gpui::Role::MenuItemRadio)
            .aria_toggled(if choice.value == self.value {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .aria_label(choice.label.clone())
            .aria_disabled(choice.disabled)
            .track_focus(&self.items[index])
            .px(px(8.))
            .py(px(5.))
            .rounded(px(15.))
            .flex()
            .items_center()
            .gap(px(6.))
            .when(choice.disabled, |d| d.opacity(0.5))
            .when(self.default_action && index == 0, |row| row.opacity(0.75))
            .when(!choice.disabled, |d| d.hover(|s| s.bg(t.menu_item_hover)))
            .when(highlighted, |d| d.bg(t.menu_item_hover))
            .children(leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .when(detail.is_some(), |row| row.pt(px(1.281_25)))
                            .children(inline_icon)
                            .child(
                                text(choice.label.clone(), 13., 18.571_428, 430., t.text)
                                    .min_w_0()
                                    .truncate(),
                            ),
                    )
                    .children(
                        detail
                            .map(|detail| text(detail, 12., 14., 430., t.text_tertiary).truncate()),
                    ),
            )
            .child(
                div()
                    .size(px(16.))
                    .opacity(if highlighted { 1. } else { 0.75 })
                    .children(
                        (choice.value == self.value)
                            .then(|| crate::icon::icon("menu-check", 16., t.text)),
                    ),
            )
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.choose(index, window, cx)))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this
                    .choices
                    .get(index)
                    .is_some_and(|choice| !choice.disabled)
                    && this.highlighted != Some(index)
                {
                    this.highlighted = Some(index);
                    cx.notify();
                }
            }));
        if let Some(detail) = detail {
            crate::tooltip::tooltip(row, detail, None, crate::tooltip::Side::Top).into_any_element()
        } else {
            row.into_any_element()
        }
    }
}

impl Render for Select {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let label = self
            .choices
            .iter()
            .find(|choice| choice.value == self.value)
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| self.value.clone());
        let leading = self.icons.and_then(|icon| icon(&self.value, &t));
        let control = crate::screens::settings::select(
            label,
            leading,
            self.loading || self.disabled,
            self.open,
            &t,
        )
        .id("settings-select-trigger")
        .role(gpui::Role::ComboBox)
        .aria_expanded(self.open)
        .aria_disabled(self.disabled)
        .track_focus(&self.trigger)
        .rounded(px(12.5))
        .focus_visible(crate::focus::trigger_focus)
        .capture_key_down(cx.listener(Self::key))
        .on_click(cx.listener(|this, _, window, cx| {
            if this.disabled {
                return;
            }
            if this.open {
                this.close(window, cx);
            } else {
                this.open(false, window, cx);
            }
        }))
        .on_mouse_down_out(cx.listener(|this, _, window, cx| {
            if this.open && !this.menu_hovered {
                this.close(window, cx);
            }
        }));
        if !self.open {
            return control;
        }
        let mut rows = self
            .choices
            .iter()
            .enumerate()
            .filter(|(i, _)| self.matches(*i))
            .map(|(index, choice)| self.option(index, choice, window, &t, cx))
            .collect::<Vec<_>>();
        let default = (self.default_action && !rows.is_empty()).then(|| rows.remove(0));
        let empty = rows.is_empty();
        control.child(deferred(
            div().absolute().top(px(-2.)).right(px(1.)).child(
                anchored()
                    .anchor(Anchor::TopRight)
                    .offset(point(px(0.), px(32.)))
                    .child(
                        div()
                            .id("settings-select-menu")
                            .capture_action(cx.listener(
                                |this, _: &crate::composer::Down, window, cx| {
                                    this.move_focus(true, window, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .capture_action(cx.listener(
                                |this, _: &crate::composer::Up, window, cx| {
                                    this.move_focus(false, window, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .capture_action(cx.listener(
                                |this, _: &crate::composer::Submit, window, cx| {
                                    this.activate_focused(window, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .capture_action(cx.listener(
                                |this, _: &crate::composer::Dismiss, window, cx| {
                                    this.close(window, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                this.menu_hovered = *hovered
                            }))
                            .child(popover(&t, self.width, |d| {
                                d.children(self.search.clone().map(|search| {
                                    div()
                                        .mb(px(4.))
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .px(px(8.))
                                        // The original row: 5 px padding around an 18.57 px input.
                                        .h(px(28.571_428))
                                        .child(crate::icon::icon("search", 14., t.text_tertiary))
                                        .child(div().flex_1().min_w_0().child(search))
                                }))
                                .children(default)
                                .children(self.heading.filter(|_| !empty).map(|heading| {
                                    div().px(px(8.)).py(px(5.)).child(text(
                                        heading,
                                        12.,
                                        16.,
                                        500.,
                                        t.text_tertiary,
                                    ))
                                }))
                                .child(
                                    div()
                                        .id("settings-select-options")
                                        .max_h(px(320.))
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .children(rows),
                                )
                                .when(self.loading, |d| {
                                    d.child(text("Loading…", 14., 20., 430., t.text_secondary))
                                })
                                .when_some(self.error.clone(), |d, error| {
                                    d.child(text(error, 14., 20., 430., t.text_error))
                                })
                                .when(
                                    empty && !self.loading && self.error.is_none(),
                                    |d| {
                                        d.child(text(
                                            "No results found",
                                            14.,
                                            20.,
                                            430.,
                                            t.text_secondary,
                                        ))
                                    },
                                )
                            })),
                    ),
            ),
        ))
    }
}
