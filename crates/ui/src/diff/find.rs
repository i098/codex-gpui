use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub(super) struct FindScroll {
    pub(super) target: Rc<RefCell<Option<(gpui::TextLayout, usize)>>>,
    pub(super) pending: Rc<Cell<bool>>,
}

impl FindScroll {
    pub(super) fn track(&self, line: &Line, active: usize, layout: &gpui::TextLayout) {
        if let Some(range) = active
            .checked_sub(line.match_start)
            .and_then(|index| line.matches.get(index))
        {
            *self.target.borrow_mut() = Some((layout.clone(), range.start));
        }
    }

    pub(super) fn probe(&self, scroll: ScrollHandle, view: gpui::EntityId) -> impl IntoElement {
        let target = self.target.clone();
        let pending = self.pending.clone();
        gpui::canvas(
            |_, _, _| {},
            move |_, (), window, cx| {
                if !pending.replace(false) {
                    return;
                }
                let Some((layout, index)) = target.borrow().clone() else {
                    return;
                };
                let Some(position) = layout.position_for_index(index) else {
                    return;
                };
                let viewport = scroll.bounds();
                let center = (viewport.top() + viewport.bottom()) / 2.;
                let mut offset = scroll.offset();
                offset.y = (offset.y + center - position.y - layout.line_height() / 2.)
                    .clamp(-scroll.max_offset().y, px(0.));
                if offset != scroll.offset() {
                    scroll.set_offset(offset);
                    window.defer(cx, move |_, cx| cx.notify(view));
                }
            },
        )
        .absolute()
        .size_full()
    }
}

impl DiffPanel {
    pub fn set_find_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query = Some(query.to_owned());
        self.active_match = 0;
        self.selected_query = true;
        self.search_changed();
        if let Some(focus) = &self.focus {
            window.focus(focus, cx);
        }
        cx.notify();
    }

    pub fn close_find(&mut self, cx: &mut Context<Self>) {
        self.query = None;
        self.search_changed();
        cx.notify();
    }

    pub fn next_find(&mut self, previous: bool, cx: &mut Context<Self>) -> bool {
        if self.query.is_none() {
            return false;
        }
        if self.matches > 0 {
            self.active_match =
                (self.active_match + if previous { self.matches - 1 } else { 1 }) % self.matches;
            self.find_scroll.pending.set(true);
            cx.notify();
        }
        true
    }
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct FindInChat(pub String);

#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct FindInDiff(pub String);

pub(crate) fn find_scope_buttons(query: &str, diffs: bool, t: &Theme) -> Div {
    let query = SharedString::from(query.to_owned());
    div().h(px(44.)).flex().items_center().gap(px(8.)).children(
        [
            (false, "Search chat", "diff-find-chat"),
            (true, "Search diffs", "diff-tab-changes"),
        ]
        .into_iter()
        .map(|(diff, label, icon)| {
            let action: Box<dyn Action> = if diff {
                Box::new(FindInDiff(query.to_string()))
            } else {
                Box::new(FindInChat(query.to_string()))
            };
            let key_action = action.boxed_clone();
            round_icon(icon, 24., t)
                .id(label)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_toggled(if diff == diffs {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .focusable()
                .cursor_pointer()
                .when(diff == diffs, |d| d.bg(t.text_alpha_10))
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                .on_key_down(move |event, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        window.dispatch_action(key_action.boxed_clone(), cx);
                        cx.stop_propagation();
                    }
                })
        }),
    )
}
