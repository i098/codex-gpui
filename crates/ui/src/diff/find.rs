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

/// The `Find in file…` / `Search diff…` popover: query, prev/next, `N / M results`, close.
pub(super) struct FindBar<'a> {
    pub label: &'static str,
    pub placeholder: &'static str,
    pub query: &'a str,
    /// The selected match among the `shown` highlighted ones; `total` counts every match.
    pub active: usize,
    pub shown: usize,
    pub total: usize,
    pub scope: Option<Div>,
    pub top: Pixels,
}

pub(super) fn find_bar<V: 'static>(
    bar: FindBar,
    close: fn(&mut V, &mut Context<V>),
    step: fn(&mut V, bool, &mut Context<V>),
    cx: &Context<V>,
) -> impl IntoElement + use<V> {
    let t = cx.theme();
    let query = bar.query;
    let count = format!(
        "{} / {} results",
        if bar.shown == 0 { 0 } else { bar.active + 1 },
        bar.total
    );
    let button = |id, name, label, size, color| {
        div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .child(icon(name, 16., color))
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .cursor_pointer()
            .hover(|style| style.bg(t.text_alpha_10))
    };
    let find_shadow = shadow(black().opacity(0.12), 8., 16., -4., false);
    let popover = div()
        .w(px(340.))
        .h(px(81.))
        .rounded(px(20.))
        .p(px(0.5))
        .child(shapes::decoration(Decoration {
            fill: Some(t.surface_secondary),
            border: Some((px(0.5), t.border)),
            radius: Corners::all(px(20.)),
            shadows: vec![find_shadow],
            ..Default::default()
        }))
        .flex()
        .flex_col()
        .font_family(SANS)
        // `overflow-hidden rounded-[20px]` with a 0.5px border: the content sits in an AA
        // rounded clip (padding box, radius 19.5) that changes the icons' edge pixels.
        .child(crate::rounded_clip(
            px(19.5),
            shapes::Corner::Round,
            div()
                .flex_1()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .pl(px(16.))
                                .gap(px(8.))
                                .child(icon("diff-find-search", 16., t.text))
                                .child(
                                    div()
                                        .id("diff-find-input")
                                        .flex_1()
                                        .cursor_text()
                                        .role(gpui::Role::TextInput)
                                        .aria_label(bar.label)
                                        .aria_placeholder(bar.placeholder)
                                        .aria_value(query.to_owned())
                                        .aria_active_descendant()
                                        .child(text(
                                            if query.is_empty() {
                                                bar.placeholder.to_owned()
                                            } else {
                                                query.to_owned()
                                            },
                                            14.,
                                            24.,
                                            430.,
                                            if query.is_empty() {
                                                t.text_tertiary
                                            } else {
                                                t.text
                                            },
                                        )),
                                ),
                        )
                        .children(bar.scope)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .pr(px(16.))
                                .child(div().mx(px(8.)).w(px(1.)).h(px(16.)).bg(t.border))
                                .child(
                                    button(
                                        "diff-find-close",
                                        "diff-find-close",
                                        "Close find",
                                        24.,
                                        t.text,
                                    )
                                    .mx(px(-2.))
                                    .on_click(cx.listener(move |this, _, _, cx| close(this, cx))),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .px(px(16.))
                        .gap(px(12.))
                        .border_t_1()
                        .border_color(t.border)
                        .child(
                            button(
                                "diff-find-previous",
                                "diff-find-previous",
                                "Previous result",
                                16.,
                                t.text_tertiary,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| step(this, true, cx))),
                        )
                        .child(
                            button(
                                "diff-find-next",
                                "diff-find-next",
                                "Next result",
                                16.,
                                t.text_tertiary,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| step(this, false, cx))),
                        )
                        .child(div().flex_1())
                        .child(
                            text(count, 14., 24., 430., t.text_tertiary)
                                .self_start()
                                .mt(px(7.)),
                        ),
                ),
        ));
    // Include the shadow's three-sigma raster extent in the composited layer.
    let outset = find_shadow.blur * 1.5 + find_shadow.spread;
    deferred(
        div()
            .absolute()
            .top(bar.top - (outset - find_shadow.offset.y))
            .right(px(12.) - outset)
            .w(px(340.) + outset * 2.)
            .h(px(81.) + outset * 2.)
            .pt(outset - find_shadow.offset.y)
            .pl(outset)
            .top_layer()
            .child(popover),
    )
    .with_priority(3)
}

/// Highlighted matches past this many are only counted, as the diff caps its own.
const FILE_FIND_LIMIT: usize = 1000;

impl FileView {
    /// `⌘F` in the file tab: opens `Find in file…` with its query selected. `false` when the
    /// tab is not focused or shows no lines.
    pub fn open_find(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self
            .focus
            .as_ref()
            .is_some_and(|focus| focus.contains_focused(window, cx))
            || self.line_count == 0
        {
            return false;
        }
        self.go_to_line = None;
        self.find.get_or_insert_default().selected = true;
        self.search_file();
        cx.notify();
        true
    }

    pub fn close_find(&mut self, cx: &mut Context<Self>) {
        self.find = None;
        self.find_matches.clear();
        self.find_total = 0;
        cx.notify();
    }

    /// `⌘G` / `⇧⌘G`, Enter / Shift-Enter: the next or previous match, wrapping. `false`
    /// while the bar is closed.
    pub fn next_find(&mut self, previous: bool, cx: &mut Context<Self>) -> bool {
        if self.find.is_none() {
            return false;
        }
        let n = self.find_matches.len();
        if n > 0 {
            self.find_active = (self.find_active + if previous { n - 1 } else { 1 }) % n;
            self.reveal_find();
            cx.notify();
        }
        true
    }

    /// The matched `(line index, byte range)`s of the query, case-insensitive.
    pub fn find_matches(&self) -> &[(usize, Range<usize>)] {
        &self.find_matches
    }

    pub(super) fn search_file(&mut self) {
        self.find_matches.clear();
        self.find_total = 0;
        let (Some(find), Some(Ok(contents))) = (&self.find, &self.contents) else {
            return;
        };
        let query = find.text.to_lowercase();
        for (index, line) in controls::lines(contents).enumerate() {
            let (ranges, total) = crate::screens::thread::view::find_ranges(
                line,
                &query,
                FILE_FIND_LIMIT - self.find_matches.len(),
            );
            self.find_matches
                .extend(ranges.into_iter().map(|range| (index, range)));
            self.find_total += total;
        }
        self.find_active = self
            .find_active
            .min(self.find_matches.len().saturating_sub(1));
        self.reveal_find();
    }

    fn reveal_find(&mut self) {
        if let Some((line, _)) = self.find_matches.get(self.find_active) {
            let top = line.saturating_sub(2) as f32 * FILE_LINE_HEIGHT;
            self.scroll.set_offset(gpui::point(px(0.), -px(top)));
        }
    }

    /// The open bar's keys: Escape closes, Enter steps, typing edits the query.
    pub(super) fn find_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(find) = &mut self.find else {
            return false;
        };
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" => {
                self.close_find(cx);
                return true;
            }
            "enter" => {
                self.next_find(key.modifiers.shift, cx);
                return true;
            }
            _ if !find.key(event) => return false,
            _ => {}
        }
        self.find_active = 0;
        self.search_file();
        cx.notify();
        true
    }

    pub(super) fn file_find_bar(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        find_bar(
            FindBar {
                label: "Find in file",
                placeholder: "Find in file…",
                query: self.find.as_ref().map_or("", |find| find.text.as_str()),
                active: self.find_active,
                shown: self.find_matches.len(),
                total: self.find_total,
                scope: None,
                top: px(56.),
            },
            Self::close_find,
            |this, previous, cx| {
                this.next_find(previous, cx);
            },
            cx,
        )
    }

    /// The match highlights of line `index`: the selected match orange, the others yellow.
    pub(super) fn find_highlights(
        &self,
        index: usize,
    ) -> impl Iterator<Item = (Range<usize>, gpui::HighlightStyle)> + '_ {
        let start = self.find_matches.partition_point(|(line, _)| *line < index);
        self.find_matches[start..]
            .iter()
            .enumerate()
            .take_while(move |(_, (line, _))| *line == index)
            .map(move |(offset, (_, range))| {
                (
                    range.clone(),
                    gpui::HighlightStyle {
                        background_color: Some(
                            rgb(if start + offset == self.find_active {
                                0xfb6a22
                            } else {
                                0xffd240
                            })
                            .into(),
                        ),
                        color: Some(gpui::white()),
                        ..Default::default()
                    },
                )
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_find_matches_every_line_case_insensitively() {
        let mut view = FileView::new("/w", "/w/a.txt".into(), None);
        view.contents = Some(Ok("Foo foo\nbar\nxFOO".into()));
        view.find = Some(controls::TextField {
            text: "foo".into(),
            selected: false,
        });
        view.search_file();
        assert_eq!(view.find_matches(), [(0, 0..3), (0, 4..7), (2, 1..4)]);
        assert_eq!(view.find_total, 3);
        view.find_active = 2;
        let line: Vec<_> = view.find_highlights(2).map(|(range, _)| range).collect();
        assert_eq!((line.len(), line[0].clone()), (1, 1..4));
        assert_eq!(view.find_highlights(1).count(), 0);
    }
}
