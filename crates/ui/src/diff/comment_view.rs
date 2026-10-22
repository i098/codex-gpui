use super::comments::{Comment, Position, Side};
use super::*;
use crate::composer::{Composer, dictation::Completion};
use gpui::{Focusable as _, MouseMoveEvent, Subscription};
use std::{collections::HashSet, num::NonZeroU32};

struct Entry {
    key: String,
    comment: Comment,
    model: bool,
    collapsed: bool,
}

struct Draft {
    position: Position,
    hunk: Option<String>,
    editing: Option<String>,
    editor: Entity<Composer>,
    save_after_dictation: bool,
    _completion: Subscription,
    _observe: Subscription,
}

#[derive(Clone, Copy)]
struct Selection {
    file: usize,
    start: (Side, u32),
    end: (Side, u32),
}

#[derive(Default)]
pub(super) struct Annotations {
    enabled: bool,
    entries: Vec<Entry>,
    dismissed: HashSet<String>,
    draft: Option<Draft>,
    selection: Option<Selection>,
    selecting: bool,
}

impl Annotations {
    pub(super) fn models(&mut self, turns: &[Turn], cwd: &str, roots: &[String]) {
        for turn in turns
            .iter()
            .filter(|turn| turn.status == bridge::types::TurnStatus::Completed)
        {
            for item in &turn.items {
                if let ThreadItem::AgentMessage { text, phase, .. } = item {
                    if phase.as_deref() == Some("commentary") {
                        continue;
                    }
                    for comment in comments::model_comments(text, cwd, roots) {
                        self.add_model(comment);
                    }
                }
            }
        }
    }

    fn add_model(&mut self, comment: Comment) {
        let key = comment.key();
        if self.entries.iter().any(|entry| entry.key == key) {
            return;
        }
        let collapsed = comment
            .extra
            .get("isResolved")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.entries.push(Entry {
            key,
            comment,
            model: true,
            collapsed,
        });
    }

    pub(super) fn intersects(&self, dir: &str, name: &str, hunk: &Hunk) -> bool {
        self.entries
            .iter()
            .filter(|entry| !self.dismissed.contains(&entry.key))
            .map(|entry| &entry.comment.position)
            .chain(self.draft.iter().map(|draft| &draft.position))
            .any(|position| {
                if position.path.strip_suffix(name) != Some(dir) {
                    return false;
                }
                let start = position.start_line.unwrap_or(position.line).get();
                hunk.lines.iter().any(|line| {
                    let number = match position.side {
                        Side::Right if line.kind != Kind::Removed => line.number,
                        Side::Left if line.kind != Kind::Added => line.old_number,
                        _ => 0,
                    };
                    number >= start && number <= position.line.get()
                })
            })
    }
}

impl DiffPanel {
    pub fn set_comment_context(
        &mut self,
        user: Vec<Comment>,
        model: Vec<Comment>,
        dismissed: HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        self.annotations.enabled = true;
        self.annotations.dismissed = dismissed;
        for comment in model {
            self.annotations.add_model(comment);
        }
        self.set_user_comments(user, cx);
        if self.hide_imports {
            self.refresh_filtered_view();
        }
        cx.notify();
    }

    pub fn set_user_comments(&mut self, comments: Vec<Comment>, cx: &mut Context<Self>) {
        if self
            .annotations
            .entries
            .iter()
            .filter(|entry| !entry.model)
            .map(|entry| &entry.comment)
            .eq(comments.iter())
        {
            return;
        }
        self.annotations.entries.retain(|entry| entry.model);
        self.annotations
            .entries
            .extend(comments.into_iter().map(|comment| Entry {
                key: comment.key(),
                comment,
                model: false,
                collapsed: false,
            }));
        if self.hide_imports {
            self.refresh_filtered_view();
        }
        cx.notify();
    }

    fn user_comments(&self) -> Vec<Comment> {
        self.annotations
            .entries
            .iter()
            .filter(|entry| !entry.model)
            .map(|entry| entry.comment.clone())
            .collect()
    }

    fn publish_comments(&self, cx: &mut Context<Self>) {
        cx.emit(DiffRequest::CommentsChanged(self.user_comments()));
    }

    pub fn comment_draft(&self) -> Option<&Entity<Composer>> {
        self.annotations.draft.as_ref().map(|draft| &draft.editor)
    }

    pub(super) fn comments_enabled(&self) -> bool {
        self.annotations.enabled
    }

    pub(super) fn context_comment(
        &mut self,
        file: usize,
        side: Side,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shown) = self.files.get(file) else {
            return;
        };
        let same = |position: &Position| {
            position.side == side
                && position.line.get() == line
                && position.path.strip_suffix(shown.name.as_ref()) == Some(shown.dir.as_ref())
        };
        if self
            .annotations
            .entries
            .iter()
            .filter(|entry| !self.annotations.dismissed.contains(&entry.key))
            .any(|entry| same(&entry.comment.position))
            || self
                .annotations
                .draft
                .as_ref()
                .is_some_and(|draft| same(&draft.position))
        {
            return;
        }
        self.annotations.selection = None;
        self.open_comment(file, side, line, None, window, cx);
    }

    pub(super) fn end_gutter_selection(&mut self) {
        self.annotations.selecting = false;
    }

    pub(super) fn comment_gutter(
        &self,
        file: usize,
        side: Side,
        line: u32,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.annotations.enabled || line == 0 {
            return None;
        }
        let t = cx.theme();
        let fill = t.text;
        let hover_fill = t.surface_secondary.blend(t.text.opacity(0.88));
        let drafting = self.annotations.draft.as_ref().is_some_and(|draft| {
            let shown = &self.files[file];
            draft.position.side == side
                && draft.position.line.get() == line
                && draft.position.path.strip_suffix(shown.name.as_ref()) == Some(shown.dir.as_ref())
        });
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .left(px(22.))
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                let point = (side, line);
                                let start = if event.modifiers.shift {
                                    this.annotations
                                        .selection
                                        .filter(|s| s.file == file)
                                        .map_or(point, |s| s.start)
                                } else {
                                    point
                                };
                                this.annotations.selection = Some(Selection {
                                    file,
                                    start,
                                    end: point,
                                });
                                this.annotations.selecting = true;
                                if let Some(focus) = &this.focus {
                                    window.focus(focus, cx);
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                            if this.annotations.selecting
                                && event.pressed_button == Some(MouseButton::Left)
                                && let Some(selection) = &mut this.annotations.selection
                                && selection.file == file
                            {
                                selection.end = (side, line);
                                cx.notify();
                            }
                        })),
                )
                .child(
                    div()
                        .size(px(CODE_LINE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui::canvas(
                                |bounds, window, _| {
                                    window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
                                },
                                move |bounds, hitbox, window, cx| {
                                    shapes::paint(
                                        &Decoration {
                                            radius: Corners::all(px(4.)),
                                            fill: Some(if hitbox.is_hovered(window) {
                                                hover_fill
                                            } else {
                                                fill
                                            }),
                                            ..Default::default()
                                        },
                                        None,
                                        bounds,
                                        window,
                                        cx,
                                    );
                                },
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .child(icon("diff-comment-add", 16., t.surface_secondary))
                        .id(SharedString::from(format!(
                            "comment-gutter-{file}-{side:?}-{line}"
                        )))
                        .absolute()
                        .right_0()
                        .top_0()
                        .opacity(0.)
                        .when(drafting, |d| d.opacity(1.))
                        .group_hover("diff-comment-line", |d| d.opacity(1.))
                        .focus(|d| d.opacity(1.))
                        .role(gpui::Role::Button)
                        .aria_label("Add a comment")
                        .tab_index(0)
                        .cursor_pointer()
                        .on_hover(|_, window, _| window.refresh())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_comment(file, side, line, None, window, cx)
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.open_comment(file, side, line, None, window, cx);
                                cx.stop_propagation();
                            }
                        })),
                )
                .into_any_element(),
        )
    }

    fn open_comment(
        &mut self,
        file: usize,
        side: Side,
        line: u32,
        editing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.annotations.enabled || line == 0 {
            return;
        }
        self.cancel_context_request();
        let Some(file_diff) = self.files.get(file) else {
            return;
        };
        let selected = self.annotations.selection.filter(|selection| {
            selection.file == file && self.comment_selection_contains(file, side, line)
        });
        let (mut start, mut end) = (
            selected.map_or((side, line), |s| s.start),
            selected.map_or((side, line), |s| s.end),
        );
        if start.0 == end.0 && start.1 > end.1 {
            std::mem::swap(&mut start, &mut end);
        }
        let path = format!("{}{}", file_diff.dir, file_diff.name);
        let mut position = Position {
            path,
            side: end.0,
            line: NonZeroU32::new(end.1).unwrap(),
            start_line: (start != end).then(|| NonZeroU32::new(start.1).unwrap()),
            start_side: (start.0 != end.0).then_some(start.0),
        };
        let mut hunk = file_diff
            .hunks
            .iter()
            .find(|hunk| {
                hunk.lines.iter().any(|entry| {
                    (if side == Side::Left {
                        entry.old_number
                    } else {
                        entry.number
                    }) == line
                })
            })
            .map(|hunk| {
                let mut text = String::new();
                for entry in &hunk.lines {
                    text.push(match entry.kind {
                        Kind::Added => '+',
                        Kind::Removed => '-',
                        Kind::Context => ' ',
                    });
                    text.push_str(&entry.text);
                    text.push('\n');
                }
                text
            });
        let initial = editing
            .as_ref()
            .and_then(|key| {
                self.annotations
                    .entries
                    .iter()
                    .find(|entry| &entry.key == key)
            })
            .map(|entry| {
                position = entry.comment.position.clone();
                hunk = entry.comment.local_diff_hunk.clone();
                entry.comment.text()
            })
            .unwrap_or_default();
        let weak = cx.entity().downgrade();
        if let Some(previous) = self.annotations.draft.take() {
            previous
                .editor
                .update(cx, |editor, cx| editor.cancel_dictation(cx));
        }
        let editor = cx.new(|cx| {
            Composer::comment_editor(
                &initial,
                move |window, cx| {
                    let _ = weak.update(cx, |panel, cx| panel.close_comment(window, cx));
                },
                window,
                cx,
            )
        });
        self.annotations.selection = Some(Selection {
            file,
            start: (
                position.start_side.unwrap_or(position.side),
                position.start_line.unwrap_or(position.line).get(),
            ),
            end: (position.side, position.line.get()),
        });
        let observe = cx.observe(&editor, |_, _, cx| cx.notify());
        let completion = cx.subscribe_in(&editor, window, Self::dictation_completed);
        self.annotations.draft = Some(Draft {
            position,
            hunk,
            editing,
            editor,
            save_after_dictation: false,
            _completion: completion,
            _observe: observe,
        });
        self.annotations.selecting = false;
        if self.hide_imports {
            self.refresh_filtered_view();
        }
        cx.notify();
    }

    fn focus_comment(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = &self.annotations.draft {
            let focus = draft.editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            cx.stop_propagation();
        }
    }

    fn dictate_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = &self.annotations.draft {
            draft
                .editor
                .update(cx, |editor, cx| editor.dictate(window, cx));
        }
    }

    fn dictation_completed(
        &mut self,
        editor: &Entity<Composer>,
        completion: &Completion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = &mut self.annotations.draft else {
            return;
        };
        if draft.editor.entity_id() != editor.entity_id() {
            return;
        }
        let save = std::mem::take(&mut draft.save_after_dictation);
        if save && *completion == Completion::Succeeded {
            self.save_comment(window, cx);
        } else {
            cx.notify();
        }
    }

    fn abort_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.annotations.draft.take() {
            draft
                .editor
                .update(cx, |editor, cx| editor.cancel_dictation(cx));
        }
        self.close_comment(window, cx);
    }

    fn close_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.annotations.draft = None;
        self.annotations.selection = None;
        if self.hide_imports {
            self.refresh_filtered_view();
        }
        if let Some(focus) = &self.focus {
            window.focus(focus, cx);
        }
        cx.notify();
    }

    fn save_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.annotations.draft else {
            return;
        };
        if draft.save_after_dictation {
            return;
        }
        if draft.editor.read(cx).has_active_dictation() {
            draft.save_after_dictation = true;
            draft
                .editor
                .update(cx, |editor, cx| editor.finish_dictation(window, cx));
            cx.notify();
            return;
        }
        let text = draft.editor.read(cx).field_text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let draft = self.annotations.draft.take().unwrap();
        if let Some(key) = &draft.editing {
            self.annotations.entries.retain(|entry| &entry.key != key);
        }
        let comment = Comment::new(
            draft.position,
            text,
            draft.hunk,
            Some(self.file_root().to_owned()),
        );
        self.annotations.entries.push(Entry {
            key: comment.key(),
            comment,
            model: false,
            collapsed: false,
        });
        self.annotations.selection = None;
        if self.hide_imports {
            self.refresh_filtered_view();
        }
        self.publish_comments(cx);
        if let Some(focus) = &self.focus {
            window.focus(focus, cx);
        }
        cx.notify();
    }

    fn delete_comment(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.annotations
            .entries
            .retain(|entry| entry.model || entry.key != key);
        self.abort_comment(window, cx);
        self.publish_comments(cx);
    }

    fn dismiss_comment(&mut self, key: String, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DiffRequest::DismissModelComment(key));
    }

    pub fn hide_model_comment(&mut self, key: String, cx: &mut Context<Self>) {
        self.annotations.dismissed.insert(key);
        if self.hide_imports {
            self.refresh_filtered_view();
        }
        cx.notify();
    }

    pub(super) fn comment_selection_contains(&self, file: usize, side: Side, line: u32) -> bool {
        self.annotations.selection.is_some_and(|selection| {
            selection.file == file
                && (if selection.start.0 == selection.end.0 {
                    side == selection.start.0
                        && (selection.start.1.min(selection.end.1)
                            ..=selection.start.1.max(selection.end.1))
                            .contains(&line)
                } else {
                    side == selection.start.0 && line >= selection.start.1
                        || side == selection.end.0 && line <= selection.end.1
                })
        })
    }

    pub(super) fn comment_rows(
        &self,
        file: usize,
        side: Side,
        line: u32,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<(gpui::AnyElement, Hsla)> {
        let file_diff = &self.files[file];
        let matches = |position: &Position| {
            position.side == side
                && position.line.get() == line
                && position.path.strip_suffix(file_diff.name.as_ref())
                    == Some(file_diff.dir.as_ref())
        };
        let mut rows = Vec::new();
        let t = cx.theme();
        let annotation_background: Hsla = rgb(if t.appearance == Appearance::Dark {
            0x202020
        } else {
            0xfbfbfb
        })
        .into();
        // ponytail: scan the small annotation list; index by file and line if review threads grow large.
        for entry in self.annotations.entries.iter().filter(|entry| {
            matches(&entry.comment.position) && !self.annotations.dismissed.contains(&entry.key)
        }) {
            if self
                .annotations
                .draft
                .as_ref()
                .is_some_and(|draft| draft.editing.as_ref() == Some(&entry.key))
            {
                continue;
            }
            if !entry.model {
                rows.push((
                    local_comment(entry, file, cx).into_any_element(),
                    annotation_background,
                ));
                continue;
            }
            let key = entry.key.clone();
            let toggle_key = key.clone();
            let dismiss_key = key.clone();
            let comment_text = entry.comment.text();
            let label = format!(
                "Review comment on {} {}",
                if entry.comment.position.start_line.is_some() {
                    "lines"
                } else {
                    "line"
                },
                entry.comment.position.display_range()
            );
            let buttons = div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(comment_button(
                    if entry.collapsed {
                        "Expand"
                    } else {
                        "Collapse"
                    },
                    false,
                    t,
                    cx,
                    move |this, _, cx| {
                        if let Some(entry) = this
                            .annotations
                            .entries
                            .iter_mut()
                            .find(|entry| entry.key == toggle_key)
                        {
                            entry.collapsed = !entry.collapsed;
                        }
                        cx.notify();
                    },
                ))
                .child(comment_button(
                    "Dismiss comment",
                    false,
                    t,
                    cx,
                    move |this, window, cx| this.dismiss_comment(dismiss_key.clone(), window, cx),
                ));
            let mut card = div()
                .id(SharedString::from(format!("diff-comment-{key}")))
                .w_full()
                .min_w_0()
                .font_family(SANS)
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .rounded(px(12.5))
                .border_1()
                .border_color(over(t.border.opacity(0.24), t.surface))
                .bg(over(t.surface_secondary.opacity(0.92), t.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .id("comment-author")
                                        .relative()
                                        .size(px(24.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .role(gpui::Role::Image)
                                        .aria_label("Codex avatar")
                                        .child(shapes::decoration(Decoration {
                                            radius: Corners::all(px(9999.)),
                                            fill: Some(over(
                                                t.surface_secondary.opacity(0.6),
                                                over(t.surface_secondary.opacity(0.92), t.surface),
                                            )),
                                            border: Some((
                                                px(1.),
                                                over(
                                                    t.border.opacity(0.2),
                                                    over(
                                                        t.surface_secondary.opacity(0.92),
                                                        t.surface,
                                                    ),
                                                ),
                                            )),
                                            ..Default::default()
                                        }))
                                        .child(icon("diff-codex", 16., t.text)),
                                )
                                .child(text("Codex", 13., 18.571_428, 500., t.text)),
                        )
                        .child(buttons),
                );
            if !entry.collapsed {
                card = card
                    .child(crate::screens::thread::view::render_markdown(
                        &comment_text,
                        window,
                        t,
                    ))
                    .child(text(label, 12., 16., 400., t.text_tertiary));
            } else {
                card = card.child(
                    text(
                        comment_text.lines().next().unwrap_or("").to_owned(),
                        13.,
                        18.571_428,
                        400.,
                        t.text_secondary,
                    )
                    .truncate(),
                );
            }
            rows.push((
                div()
                    .w_full()
                    .max_w(px(768.))
                    .p(px(6.))
                    .child(card)
                    .into_any_element(),
                annotation_background,
            ));
        }
        if let Some(draft) = &self.annotations.draft
            && matches(&draft.position)
        {
            let editing = draft.editing.is_some();
            let enabled = (!draft.editor.read(cx).field_text().trim().is_empty()
                || draft.editor.read(cx).has_active_dictation())
                && !draft.save_after_dictation;
            let dictation_label = draft.editor.read(cx).dictation_label();
            rows.push((
                div()
                    .w_full()
                    .max_w(px(768.))
                    .p(px(6.))
                    .font_family(SANS)
                    .child(
                        local_comment_frame(&draft.position, t, |frame| {
                            frame
                                .child(div().p(px(12.)).child(draft.editor.clone()))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(8.))
                                        .px(px(12.))
                                        .py(px(8.))
                                        .child(div().when_some(draft.editing.clone(), |d, key| {
                                            d.child(comment_button(
                                                "Delete",
                                                false,
                                                t,
                                                cx,
                                                move |this, window, cx| {
                                                    this.delete_comment(&key, window, cx)
                                                },
                                            ))
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.))
                                                .child(
                                                    crate::composer::pill(
                                                        "diff-comment-dictate",
                                                        t,
                                                    )
                                                    .size(px(24.))
                                                    .justify_center()
                                                    .role(gpui::Role::Button)
                                                    .aria_label(dictation_label)
                                                    .aria_disabled(
                                                        dictation_label == "Transcribing",
                                                    )
                                                    .tab_index(0)
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.dictate_comment(window, cx)
                                                    }))
                                                    .on_key_down(cx.listener(
                                                        |this, event: &KeyDownEvent, window, cx| {
                                                            if matches!(
                                                                event.keystroke.key.as_str(),
                                                                "enter" | "space"
                                                            ) {
                                                                this.dictate_comment(window, cx);
                                                                cx.stop_propagation();
                                                            }
                                                        },
                                                    ))
                                                    .child(if dictation_label == "Dictate" {
                                                        icon("mic", 16., t.text).into_any_element()
                                                    } else {
                                                        div()
                                                            .size(px(8.8))
                                                            .rounded(px(1.))
                                                            .bg(t.text)
                                                            .into_any_element()
                                                    }),
                                                )
                                                .child(comment_button(
                                                    "Cancel",
                                                    false,
                                                    t,
                                                    cx,
                                                    Self::abort_comment,
                                                ))
                                                .child(
                                                    comment_button(
                                                        if editing { "Save" } else { "Comment" },
                                                        true,
                                                        t,
                                                        cx,
                                                        Self::save_comment,
                                                    )
                                                    .aria_disabled(!enabled)
                                                    .tab_stop(enabled)
                                                    .when(!enabled, |d| d.opacity(0.5)),
                                                ),
                                        ),
                                )
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.focus_comment(window, cx)),
                        ),
                    )
                    .into_any_element(),
                if editing {
                    annotation_background
                } else {
                    t.surface
                },
            ));
        }
        rows
    }
}

fn local_comment_frame(position: &Position, t: &Theme, content: impl FnOnce(Div) -> Div) -> Div {
    let title = format!(
        "Local comment on {} {}",
        if position.start_line.is_some() {
            "lines"
        } else {
            "line"
        },
        position.display_range(),
    );
    let frame = div()
        .relative()
        .rounded(px(12.5))
        .flex()
        .flex_col()
        .font_family(SANS)
        .child(shapes::decoration(Decoration {
            radius: Corners::all(px(12.5)),
            // `rounded-lg`: `corner-shape: var(--codex-corner-shape)`, `superellipse(1.5)`.
            corner: shapes::Corner::Squircle,
            fill: Some(if t.appearance == Appearance::Dark {
                gpui::white().opacity(0.03)
            } else {
                t.primary_soft_alpha
            }),
            shadows: vec![
                shadow(t.border_strong, 0., 0., 0.5, false),
                shadow(black().opacity(0.04), 3., 7.5, 0., false),
                shadow(black().opacity(0.05), 0., 20., 0., false),
            ],
            ..Default::default()
        }));
    let body = div().flex().flex_col().child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(12.))
            .pt(px(8.))
            .pb(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .id("comment-author")
                            .size(px(24.))
                            .flex_none()
                            // Skia's anti-aliased circle, as Chromium paints `rounded-full`.
                            .child(shapes::decoration(Decoration {
                                radius: Corners::all(px(12.)),
                                fill: Some(rgb(0xff7419).into()),
                                ..Default::default()
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .role(gpui::Role::Image)
                            .aria_label("Your avatar")
                            .child(text("U", 12., 16., 400., t.text)),
                    )
                    .child(text("You", 12., 16., 430., t.text_tertiary)),
            )
            .child(text(title, 12., 16., 430., t.text_tertiary)),
    );
    frame.child(crate::rounded_clip(
        px(12.5),
        shapes::Corner::Squircle,
        content(body),
    ))
}

fn local_comment(entry: &Entry, file: usize, cx: &Context<DiffPanel>) -> Div {
    let t = cx.theme();
    let key = entry.key.clone();
    let delete_key = key.clone();
    let keyboard_key = key.clone();
    let side = entry.comment.position.side;
    let line = entry.comment.position.line.get();
    let card = local_comment_frame(&entry.comment.position, t, |frame| {
        frame
            .child(
                div()
                    .p(px(12.))
                    .child(text(entry.comment.text(), 14., 22.75, 430., t.text)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .px(px(12.))
                    .py(px(8.))
                    .child(comment_button(
                        "Delete",
                        false,
                        t,
                        cx,
                        move |this, window, cx| this.delete_comment(&delete_key, window, cx),
                    )),
            )
    })
    .id(SharedString::from(format!("diff-comment-{key}")))
    .cursor_pointer()
    .role(gpui::Role::Button)
    .aria_label("Edit comment")
    .tab_index(0)
    .on_click(cx.listener(move |this, _, window, cx| {
        this.open_comment(file, side, line, Some(key.clone()), window, cx);
    }))
    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            this.open_comment(file, side, line, Some(keyboard_key.clone()), window, cx);
            cx.stop_propagation();
        }
    }));
    div().w_full().max_w(px(768.)).p(px(6.)).child(card)
}

fn comment_button(
    label: &'static str,
    primary: bool,
    t: &Theme,
    cx: &Context<DiffPanel>,
    action: impl Fn(&mut DiffPanel, &mut Window, &mut Context<DiffPanel>) + Clone + 'static,
) -> Stateful<Div> {
    let keyboard = action.clone();
    div()
        .id(SharedString::from(format!("comment-{label}")))
        .role(gpui::Role::Button)
        .aria_label(label)
        .tab_index(0)
        .cursor_pointer()
        .px(px(9.))
        .py(px(1.))
        .h(px(24.))
        // `rounded-button-action`: Skia's `superellipse(1.5)` corners; the 1 px border paints
        // with the fill, inside the padding that keeps its layout width.
        .child(shapes::decoration(Decoration {
            radius: Corners::all(px(12.5)),
            corner: shapes::Corner::Squircle,
            fill: primary.then_some(t.button_primary_solid),
            border: primary.then_some((px(1.), t.border)),
            ..Default::default()
        }))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            action(this, window, cx);
            cx.stop_propagation();
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                keyboard(this, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(text(
            label,
            12.,
            16.,
            430.,
            if primary {
                t.surface_elevated_secondary
            } else {
                t.text_tertiary
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_comments_require_completed_final_output_and_deduplicate() {
        let directive = "::code-comment{title=\"Guard the index\" body=\"Check bounds\" file=\"a.rs\" start=2 end=4}";
        let turn: Turn = serde_json::from_value(serde_json::json!({
            "id":"review", "status":"completed", "error":null,
            "items":[
                {"type":"agentMessage","id":"analysis","phase":"commentary","text":directive},
                {"type":"agentMessage","id":"answer","phase":"final_answer","text":directive}
            ]
        }))
        .unwrap();
        let mut annotations = Annotations::default();
        annotations.models(std::slice::from_ref(&turn), "/repo", &[]);
        annotations.models(std::slice::from_ref(&turn), "/repo", &[]);
        assert_eq!(annotations.entries.len(), 1);
        assert_eq!(annotations.entries[0].comment.position.range(), "2-4");
        let mut incomplete = turn;
        incomplete.status = bridge::types::TurnStatus::InProgress;
        let mut annotations = Annotations::default();
        annotations.models(&[incomplete], "/repo", &[]);
        assert!(annotations.entries.is_empty());
    }
}
