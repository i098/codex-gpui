use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub(super) struct FindScroll {
    pub(super) target: Rc<RefCell<Option<(gpui::TextLayout, usize)>>>,
    pub(super) pending: Rc<Cell<bool>>,
    /// Typing a query keeps the scroll while any match is on screen, as Chromium's find does.
    pub(super) typing: Rc<Cell<bool>>,
    pub(super) lines: Rc<RefCell<Vec<gpui::TextLayout>>>,
}

impl FindScroll {
    pub(super) fn track(&self, line: &Line, active: usize, layout: &gpui::TextLayout) {
        if !line.matches.is_empty() {
            self.lines.borrow_mut().push(layout.clone());
        }
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
        let typing = self.typing.clone();
        let lines = self.lines.clone();
        gpui::canvas(
            |_, _, _| {},
            move |_, (), window, cx| {
                if !pending.replace(false) {
                    return;
                }
                let Some((layout, index)) = target.borrow().clone() else {
                    return;
                };
                let viewport = scroll.bounds();
                if typing.replace(false)
                    && lines.borrow().iter().any(|line| {
                        line.position_for_index(0)
                            .is_some_and(|p| p.y >= viewport.top() && p.y < viewport.bottom())
                    })
                {
                    return;
                }
                let Some(position) = layout.position_for_index(index) else {
                    return;
                };
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
        self.find_scroll.pending.set(self.matches > 0);
        self.find_scroll.typing.set(false);
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
            self.reveal_find_match();
            self.find_scroll.pending.set(true);
            self.find_scroll.typing.set(false);
            cx.notify();
        }
        true
    }

    pub(super) fn reveal_find_match(&mut self) {
        if !self.capped() {
            return;
        }
        if let Some(index) = self.files.iter().position(|file| {
            file.hunks.iter().any(|hunk| {
                hunk.lines.iter().any(|line| {
                    (line.match_start..line.match_start + line.matches.len())
                        .contains(&self.active_match)
                })
            })
        }) && self.active_file != index
        {
            self.active_file = index;
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
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
            div()
                .size(px(24.))
                .mx(px(-2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                // The fitted header icon has a different raster position.
                .child(
                    crate::icon::icon(
                        icon,
                        16.,
                        if diff == diffs {
                            t.text
                        } else {
                            t.text_tertiary
                        },
                    )
                    .runtime(),
                )
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
                .hover(|style| style.bg(t.text_alpha_10))
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
