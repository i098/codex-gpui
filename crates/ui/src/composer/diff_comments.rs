use super::*;
use crate::screens::diff::panel::{OpenFile, comments::Comment};
use gpui::AppContext as _;

const ATTACHMENT_LINE_HEIGHT: f32 = 18.571_428;

#[derive(Clone)]
pub struct DiffCommentsChanged(pub Vec<Comment>);

impl gpui::EventEmitter<DiffCommentsChanged> for Composer {}

impl Composer {
    pub fn set_diff_comments(&mut self, comments: Vec<Comment>, cx: &mut Context<Self>) {
        if self.diff_comments != comments {
            self.diff_comments = comments;
            if self.diff_comments.is_empty() {
                self.close_comments_popup();
            } else if self.menu == Some(Menu::Comments) {
                self.menu_focus.prepare(self.diff_comments.len(), cx);
            }
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
        crate::screens::diff::panel::comments::with_comments(&self.diff_comments, prompt)
    }

    /// `artifact-direct-comment`: the turn that sends `comments` on their own with an empty
    /// request (bundle `Wdt`), keeping the draft. None when the turn cannot start now.
    pub fn comments_submission(&self, comments: &[Comment]) -> Option<Submission> {
        if comments.is_empty()
            || self.blocked.is_some()
            || self.edit_pending
            || (!self.running && self.current_model().is_none())
        {
            return None;
        }
        Some(Submission {
            text: crate::screens::diff::panel::comments::with_comments(comments, String::new()),
            shell_command: false,
            attachments: Vec::new(),
            context_files: Vec::new(),
            model: self
                .current_model()
                .map(|m| m.id.clone())
                .unwrap_or_default(),
            effort: self.effort.clone(),
            permission: self.permission.clone(),
            service_tier: SERVICE_TIERS[self.speed],
            plan: self.plan,
            background: false,
        })
    }

    pub(super) fn take_diff_comments(&mut self, cx: &mut Context<Self>) {
        if !self.diff_comments.is_empty() {
            self.close_comments_popup();
            self.submitted_diff_comments = std::mem::take(&mut self.diff_comments);
            cx.emit(DiffCommentsChanged(Vec::new()));
        }
    }

    pub(super) fn diff_comment_attachment(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let count = self.diff_comments.len();
        let label = format!("{count} comment{}", if count == 1 { "" } else { "s" });
        let open = self.menu == Some(Menu::Comments);
        let surface = |fill| {
            shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(10.)),
                fill: Some(fill),
                border: Some((px(1.), t.border)),
                ..Default::default()
            })
        };
        div()
            .id("diff-comments-attachment")
            .relative()
            .group("diff-comments-attachment")
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                this.hover_comment_attachment(*hovered, window, cx)
            }))
            .child(
                div()
                    .id("diff-comments-preview-trigger")
                    .group("diff-comments-preview-trigger")
                    .relative()
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .aria_expanded(open)
                    .track_focus(&self.comments_focus)
                    .focus_visible(menu::trigger_focus)
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(9.))
                    .py(px(7.))
                    .max_w(px(320.))
                    .rounded(px(10.))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .group_hover("diff-comments-preview-trigger", |s| s.opacity(0.))
                            .child(surface(t.surface_elevated_secondary)),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(0.)
                            .group_hover("diff-comments-preview-trigger", |s| s.opacity(1.))
                            .child(surface(t.surface_elevated)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.comments_close = None;
                        if this.menu != Some(Menu::Comments) {
                            this.toggle_menu(Menu::Comments, window, cx);
                        }
                        this.menu_focus.content.focus(window, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                            this.comments_close = None;
                            if this.menu != Some(Menu::Comments) {
                                this.toggle_menu(Menu::Comments, window, cx);
                            }
                            if let Some(first) = this.menu_focus.items.first() {
                                first.focus(window, cx);
                            }
                            cx.stop_propagation();
                        }
                    }))
                    .child(icon("composer-diff-comments", 14., t.text_tertiary))
                    .child(
                        div().min_w_0().pr(px(4.)).child(
                            text(label, 13., ATTACHMENT_LINE_HEIGHT, 500., t.text).truncate(),
                        ),
                    ),
            )
            .child(
                div()
                    .id("remove-diff-comments")
                    .absolute()
                    .right(px(4.))
                    .top(relative(0.5))
                    .mt(px(-10.))
                    .size(px(20.))
                    .rounded_full()
                    .bg(t.surface_elevated)
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.)
                    .group_hover("diff-comments-attachment", |style| style.opacity(1.))
                    .focus_visible(|style| style.opacity(1.))
                    .role(gpui::Role::Button)
                    .aria_label("Remove comments attachment")
                    .focusable()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clear_diff_comments(cx);
                        cx.stop_propagation();
                    }))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.clear_diff_comments(cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(icon("close", 12., t.text_tertiary)),
            )
            .when(open, |d| {
                d.child(
                    deferred(
                        div()
                            .absolute()
                            .left_0()
                            .bottom(px(ATTACHMENT_LINE_HEIGHT + 18.))
                            .child(self.diff_comments_preview(t, cx)),
                    )
                    .with_priority(2),
                )
            })
    }

    fn diff_comments_preview(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let rows = self
            .diff_comments
            .iter()
            .enumerate()
            .map(|(index, comment)| {
                let path = std::path::Path::new(&comment.position.path);
                let label = comment
                    .local_workspace_root
                    .as_deref()
                    .and_then(|root| path.strip_prefix(root).ok())
                    .and_then(std::path::Path::to_str)
                    .unwrap_or(&comment.position.path)
                    .to_owned();
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id(("diff-comment-file", index))
                                    .role(gpui::Role::Link)
                                    .aria_label(format!("Open {label}"))
                                    .track_focus(&self.menu_focus.items[index])
                                    .focus_visible(menu::trigger_focus)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_comment_attachment(index, window, cx)
                                    }))
                                    .child(text(label, 12., 16., 430., t.text_tertiary)),
                            )
                            .child(text(comment.position.side.marker(), 12., 16., 500., t.text))
                            .child(text(
                                comment.position.range(),
                                12.,
                                16.,
                                430.,
                                t.text_tertiary,
                            )),
                    )
                    .child(text(
                        comment.text(),
                        13.,
                        ATTACHMENT_LINE_HEIGHT,
                        430.,
                        t.text,
                    ))
            });
        div()
            .id("diff-comments-preview")
            .occlude()
            .role(gpui::Role::Dialog)
            .aria_label("Comments")
            .track_focus(&self.menu_focus.content)
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                this.hover_comment_attachment(*hovered, window, cx)
            }))
            .on_key_down(cx.listener(Self::menu_key))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if !this.menu_trigger_hovered && this.menu == Some(Menu::Comments) {
                    this.close_comments_popup();
                    window.blur(cx);
                    cx.notify();
                }
            }))
            .child(popover(t, 384., |d| {
                d.child(
                    div()
                        .id("diff-comments-preview-scroll")
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .children(rows),
                )
            }))
    }

    fn hover_comment_attachment(&mut self, hovered: bool, window: &Window, cx: &mut Context<Self>) {
        self.comments_close = None;
        self.menu_trigger_hovered = hovered;
        if hovered {
            if self.menu != Some(Menu::Comments) {
                self.close_speed();
                self.menu = Some(Menu::Comments);
                self.menu_focus.prepare(self.diff_comments.len(), cx);
                self.menu_focus.trigger = self.comments_focus.clone();
                cx.notify();
            }
        } else if self.menu == Some(Menu::Comments) {
            let handle = window.window_handle();
            self.comments_close = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let _ = cx.update_window(handle, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        if this.menu == Some(Menu::Comments)
                            && this.menu_focus.content.contains_focused(window, cx)
                        {
                            this.comments_focus.focus(window, cx);
                        }
                        this.close_comments_popup();
                        cx.notify();
                    })
                });
            }));
        }
    }

    fn close_comments_popup(&mut self) {
        self.comments_close = None;
        if self.menu == Some(Menu::Comments) {
            self.menu = None;
        }
    }

    fn clear_diff_comments(&mut self, cx: &mut Context<Self>) {
        self.set_diff_comments(Vec::new(), cx);
        cx.emit(DiffCommentsChanged(Vec::new()));
    }

    pub(super) fn open_comment_attachment(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(comment) = self.diff_comments.get(index) else {
            return;
        };
        let action = OpenFile {
            path: comment.position.path.clone(),
            line: Some(comment.position.line.get()),
        };
        self.dismiss(&Dismiss, window, cx);
        window.dispatch_action(Box::new(action), cx);
    }
}
