use super::*;
use crate::screens::diff::panel::comments::Comment;

#[derive(Clone)]
pub struct DiffCommentsChanged(pub Vec<Comment>);

impl gpui::EventEmitter<DiffCommentsChanged> for Composer {}

impl Composer {
    pub fn set_diff_comments(&mut self, comments: Vec<Comment>, cx: &mut Context<Self>) {
        if self.diff_comments != comments {
            self.diff_comments = comments;
            cx.notify();
        }
    }

    pub fn diff_comments(&self) -> &[Comment] {
        &self.diff_comments
    }

    pub fn comment_editor(
        value: &str,
        cancel: impl Fn(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::settings_field("Add a comment…", value, true, cx);
        editor.line_height = px(22.75);
        editor.on_cancel_edit = Some(Rc::new(cancel));
        editor.focus_handle.focus(window, cx);
        editor
    }

    pub(super) fn with_diff_comments(&self, prompt: String) -> String {
        if self.diff_comments.is_empty() {
            return prompt;
        }
        let mut combined = String::from("# Diff comments:\n");
        for (index, comment) in self.diff_comments.iter().enumerate() {
            combined.push_str(&comment.prompt(index + 1));
        }
        combined.push_str("\n## My request:\n");
        combined.push_str(&prompt);
        combined
    }

    pub(super) fn take_diff_comments(&mut self, cx: &mut Context<Self>) {
        if !self.diff_comments.is_empty() {
            self.submitted_diff_comments = std::mem::take(&mut self.diff_comments);
            cx.emit(DiffCommentsChanged(Vec::new()));
        }
    }

    pub(super) fn diff_comment_chips(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .when(!self.diff_comments.is_empty(), |d| {
                d.px(px(12.)).pt(px(12.))
            })
            .children(
                self.diff_comments
                    .iter()
                    .enumerate()
                    .map(|(index, comment)| {
                        let label = format!(
                            "{} {}",
                            comment.position.path,
                            comment.position.display_range()
                        );
                        let remove = move |this: &mut Composer, cx: &mut Context<Composer>| {
                            if index < this.diff_comments.len() {
                                this.diff_comments.remove(index);
                                cx.emit(DiffCommentsChanged(this.diff_comments.clone()));
                                cx.notify();
                            }
                        };
                        div()
                            .id(("diff-comment-chip", index))
                            .rounded(px(8.))
                            .bg(t.list_hover)
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .py(px(4.))
                            .child(text(label, 12., 16., 430., t.text))
                            .child(
                                div()
                                    .id(("remove-diff-comment", index))
                                    .role(gpui::Role::Button)
                                    .aria_label("Remove comment")
                                    .focusable()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| remove(this, cx)))
                                    .on_key_down(cx.listener(
                                        move |this, event: &gpui::KeyDownEvent, _, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                remove(this, cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                                    .child(icon("close", 12., t.text_tertiary)),
                            )
                    }),
            )
    }
}
