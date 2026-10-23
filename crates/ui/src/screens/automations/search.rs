use crate::{Copy, Cut, Paste, SelectAll, focus, text};
use gpui::{
    App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Point, Render, Role, ShapedLine, SharedString,
    StatefulInteractiveElement, Styled, TextRun, UTF16Selection, Window, canvas, div, fill, point,
    prelude::FluentBuilder as _, px,
};
use std::ops::Range;
use theme::ActiveTheme as _;

fn utf16(value: &str, offset: usize) -> usize {
    value[..offset].encode_utf16().count()
}

fn utf8(value: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, ch) in value.char_indices() {
        if units + ch.len_utf16() > offset {
            return index;
        }
        units += ch.len_utf16();
    }
    value.len()
}

pub struct SearchInput {
    pub value: String,
    focus: FocusHandle,
    pub return_focus: Option<FocusHandle>,
    cursor: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    line: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll: Pixels,
}

pub enum SearchEvent {
    Changed,
    Dismiss,
}
impl EventEmitter<SearchEvent> for SearchInput {}
impl Focusable for SearchInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SearchInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            value: String::new(),
            focus: cx.focus_handle().tab_stop(true),
            return_focus: None,
            cursor: 0,
            anchor: 0,
            marked: None,
            line: None,
            bounds: None,
            scroll: px(0.),
        }
    }

    pub fn set_value(&mut self, value: String, cx: &mut Context<Self>) {
        self.cursor = value.len();
        self.anchor = self.cursor;
        self.value = value;
        self.marked = None;
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    fn bytes(&self, range: Range<usize>) -> Range<usize> {
        utf8(&self.value, range.start)..utf8(&self.value, range.end)
    }
    fn units(&self, range: Range<usize>) -> Range<usize> {
        utf16(&self.value, range.start)..utf16(&self.value, range.end)
    }

    fn replace(&mut self, range: Range<usize>, value: &str, cx: &mut Context<Self>) {
        let value = value.replace(['\r', '\n'], " ");
        self.value.replace_range(range.clone(), &value);
        self.cursor = range.start + value.len();
        self.anchor = self.cursor;
        self.marked = None;
        cx.emit(SearchEvent::Changed);
        cx.notify();
    }

    fn copy(&self, cx: &mut Context<Self>) {
        if !self.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.value[self.selection()].to_owned(),
            ));
        }
    }

    fn index(&self, position: Point<Pixels>) -> usize {
        match (&self.line, self.bounds) {
            (Some(line), Some(bounds)) => {
                line.closest_index_for_x(position.x - bounds.origin.x + self.scroll)
            }
            _ => self.value.len(),
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" => {
                self.set_value(String::new(), cx);
                cx.emit(SearchEvent::Dismiss);
                if let Some(focus) = &self.return_focus {
                    focus.focus(window, cx);
                }
            }
            "tab" => {
                focus::tab(event, window, cx);
                return;
            }
            "left" | "right" | "home" | "end" | "backspace" | "delete" => {
                self.apply_edit_key(key, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn apply_edit_key(&mut self, key: &gpui::Keystroke, cx: &mut Context<Self>) {
        let range = self.selection();
        let back = matches!(key.key.as_str(), "left" | "backspace" | "home");
        let boundary = self.cursor_boundary(key, back);
        if matches!(key.key.as_str(), "backspace" | "delete") {
            let range = if range.is_empty() {
                boundary.min(self.cursor)..boundary.max(self.cursor)
            } else {
                range
            };
            self.replace(range, "", cx);
        } else {
            self.cursor = if !key.modifiers.shift && !range.is_empty() {
                if back { range.start } else { range.end }
            } else {
                boundary
            };
            if !key.modifiers.shift {
                self.anchor = self.cursor;
            }
        }
    }

    fn cursor_boundary(&self, key: &gpui::Keystroke, back: bool) -> usize {
        if key.modifiers.platform || matches!(key.key.as_str(), "home" | "end") {
            if back { 0 } else { self.value.len() }
        } else if back {
            self.value[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        } else {
            self.cursor
                + self.value[self.cursor..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
        }
    }
}

impl EntityInputHandler for SearchInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.bytes(range);
        *actual = Some(self.units(range.clone()));
        Some(self.value[range].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.units(self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.bytes(range);
        self.anchor = range.start;
        self.cursor = range.end;
        cx.notify();
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(utf16(&self.value, self.value.len()))
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| self.units(range))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_and_mark_text_in_range(range, value, None, window, cx);
        self.marked = None;
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        value: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| self.bytes(range))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let start = range.start;
        self.replace(range, value, cx);
        self.marked = (!value.is_empty()).then_some(start..self.cursor);
        if let Some(selection) = selection {
            let start = utf16(&self.value, start);
            let range = self.bytes(start + selection.start..start + selection.end);
            self.anchor = range.start;
            self.cursor = range.end;
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.bytes(range);
        let line = self.line.as_ref()?;
        Some(Bounds::from_corners(
            bounds.origin + point(line.x_for_index(range.start) - self.scroll, px(0.)),
            bounds.origin + point(line.x_for_index(range.end) - self.scroll, px(20.)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(utf16(&self.value, self.index(point)))
    }
}

impl Render for SearchInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let value: SharedString = self.value.clone().into();
        let mut font = window.text_style().font();
        font.weight = FontWeight(430.);
        self.line = Some(window.text_system().shape_line(
            value.clone(),
            px(14.),
            &[TextRun {
                len: value.len(),
                font,
                color: t.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        ));
        if let (Some(line), Some(bounds)) = (&self.line, self.bounds) {
            let caret = line.x_for_index(self.cursor);
            self.scroll = self
                .scroll
                .min(caret)
                .max(caret - bounds.size.width + px(1.))
                .max(px(0.));
        }
        let entity = cx.entity();
        let clear_focus = focus::handle("clear-task-search", 0, window, cx);
        div()
            .id("task-search-input")
            .role(Role::TextInput)
            .aria_label("Search scheduled tasks")
            .aria_value(value.clone())
            .aria_placeholder("Search scheduled tasks")
            .track_focus(&self.focus)
            .relative()
            .mb(px(8.))
            .px(px(12.))
            .h(px(36.))
            .rounded_full()
            .bg(t.list_hover)
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_text()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.cursor = this.index(event.position);
                    if !event.modifiers.shift {
                        this.anchor = this.cursor;
                    }
                    this.focus.focus(window, cx);
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.anchor = 0;
                this.cursor = this.value.len();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                this.copy(cx);
                this.replace(this.selection(), "", cx);
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(value) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    this.replace(this.selection(), &value, cx);
                }
            }))
            .child(crate::icon::icon("search", 16., t.text_secondary))
            .child(
                div()
                    .relative()
                    .h(px(18.))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        text(
                            if value.is_empty() {
                                "Search scheduled tasks".into()
                            } else {
                                value
                            },
                            14.,
                            18.,
                            430.,
                            if self.value.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .relative()
                        .left(-self.scroll),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, (), window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.bounds = Some(bounds);
                                    window.handle_input(
                                        &this.focus,
                                        ElementInputHandler::new(bounds, entity.clone()),
                                        cx,
                                    );
                                    if this.focus.is_focused(window) {
                                        let line = this.line.as_ref().unwrap();
                                        let x = |index| line.x_for_index(index) - this.scroll;
                                        let range = this.selection();
                                        if !range.is_empty() {
                                            window.paint_quad(fill(
                                                Bounds::from_corners(
                                                    bounds.origin + point(x(range.start), px(0.)),
                                                    bounds.origin + point(x(range.end), px(18.)),
                                                ),
                                                t.text_selection,
                                            ));
                                        }
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                bounds.origin + point(x(this.cursor), px(2.)),
                                                gpui::size(px(1.), px(14.)),
                                            ),
                                            t.text,
                                        ));
                                    }
                                });
                            },
                        )
                        .absolute()
                        .inset_0(),
                    ),
            )
            .when(!self.value.is_empty(), |d| {
                d.child(focus::ring(
                    div()
                        .id("clear-task-search")
                        .role(Role::Button)
                        .aria_label("Clear search")
                        .relative()
                        .size(px(14.))
                        .flex_none()
                        .rounded_full()
                        .bg(t.text_secondary)
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.replace(0..this.value.len(), "", cx);
                            this.focus.focus(window, cx);
                            cx.stop_propagation();
                        }))
                        .child(crate::icon::icon("close", 8., t.surface)),
                    &clear_focus,
                    px(7.),
                    px(2.),
                    cx,
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unicode_selection_offsets_round_trip_and_clamp_surrogates() {
        let value = "a😀é";
        for offset in value
            .char_indices()
            .map(|(index, _)| index)
            .chain([value.len()])
        {
            assert_eq!(super::utf8(value, super::utf16(value, offset)), offset);
        }
        assert_eq!(super::utf8(value, 2), 1);
        assert_eq!(super::utf8(value, usize::MAX), value.len());
    }
}
