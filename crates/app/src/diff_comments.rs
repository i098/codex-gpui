use crate::{Root, host_settings::Settings};
use gpui::{App, AppContext as _, Context, Entity, Window};
use serde_json::{Value, json};
use std::collections::HashSet;
use ui::confirmation::{Confirmation, Outcome};
use ui::{
    screens::diff::panel::{DiffPanel, comments::Comment},
    toast::Toast,
};

fn stored_comments(cx: &App, key: &str, thread: &str) -> Vec<Comment> {
    let Some(settings) = cx.try_global::<Settings>() else {
        return Vec::new();
    };
    let values = settings.read_all();
    let value = values.get(key).and_then(|threads| threads.get(thread));
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => serde_json::from_value(value.clone()).unwrap_or_else(|error| {
            eprintln!("codex-gpui: invalid stored diff comments: {error}");
            Vec::new()
        }),
    }
}

impl Root {
    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some()
            || self.diff_confirmation.is_some()
            || self.automation_editor.is_some()
            || self.new_project.is_some()
            || self.feedback.is_some()
    }

    pub fn diff_comment_confirmation(&self) -> Option<&Entity<Confirmation>> {
        self.diff_confirmation.as_ref()
    }

    pub(super) fn confirm_diff_comment(
        &mut self,
        panel: Entity<DiffPanel>,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_dialog() {
            return;
        }
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Dismiss this review comment?",
                "The comment will be hidden from the diff but remain in the task transcript",
                "Dismiss",
                window,
                cx,
            )
            .destructive()
        });
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                this.dismiss_diff_comment(&key, cx);
                panel.update(cx, |panel, cx| panel.hide_model_comment(key.clone(), cx));
            }
            this.diff_confirmation = None;
            cx.notify();
        })
        .detach();
        self.diff_confirmation = Some(dialog);
        cx.notify();
    }

    pub(super) fn configure_diff_comments(
        &mut self,
        panel: &Entity<DiffPanel>,
        thread: &str,
        cx: &mut Context<Self>,
    ) {
        let user = stored_comments(cx, "diff_comments", thread);
        let model = stored_comments(cx, "diff_comments_from_model", thread);
        let prefix = format!("dismissed-model-diff-comments-v3:{thread}:");
        let dismissed: HashSet<String> = cx
            .try_global::<Settings>()
            .map(|settings| {
                settings
                    .read_all()
                    .iter()
                    .filter(|(_, value)| **value == true)
                    .filter_map(|(key, _)| key.strip_prefix(&prefix).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        panel.update(cx, |panel, cx| {
            panel.set_comment_context(user.clone(), model, dismissed, cx)
        });
        self.thread_composer
            .update(cx, |composer, cx| composer.set_diff_comments(user, cx));
        self.sync_browser_annotations(cx);
    }

    pub(super) fn change_diff_comments(&mut self, comments: &[Comment], cx: &mut Context<Self>) {
        let Some(thread) = self
            .thread
            .as_ref()
            .map(|thread| thread.read(cx).id().to_owned())
        else {
            return;
        };
        self.save_diff_comments(thread, comments, cx);
        self.thread_composer.update(cx, |composer, cx| {
            composer.set_diff_comments(comments.to_vec(), cx)
        });
        if let Some(panel) = &self.diff {
            panel.update(cx, |panel, cx| {
                panel.set_user_comments(comments.to_vec(), cx)
            });
        }
        self.sync_browser_annotations(cx);
    }

    fn save_diff_comments(&mut self, thread: String, comments: &[Comment], cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(cx.has_global::<Settings>(), "host settings are unavailable");
            let settings = cx.global_mut::<Settings>();
            let mut threads = match settings.read_all().get("diff_comments") {
                Some(Value::Object(threads)) => threads.clone(),
                None | Some(Value::Null) => Default::default(),
                _ => anyhow::bail!("invalid stored comment map"),
            };
            if comments.is_empty() {
                threads.remove(&thread);
            } else {
                threads.insert(thread, serde_json::to_value(comments)?);
            }
            settings.write("diff_comments", Value::Object(threads))
        })();
        if let Err(error) = result {
            eprintln!("codex-gpui: cannot save diff comment: {error:#}");
            self.toast(Toast::danger("Could not save comment. Try again."), cx);
        }
    }

    pub(super) fn dismiss_diff_comment(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(thread) = self
            .thread
            .as_ref()
            .map(|thread| thread.read(cx).id().to_owned())
        else {
            return;
        };
        let key = format!("dismissed-model-diff-comments-v3:{thread}:{key}");
        if let Err(error) = cx.global_mut::<Settings>().write(&key, json!(true)) {
            eprintln!("codex-gpui: cannot dismiss diff comment: {error:#}");
            self.toast(Toast::danger("Could not dismiss comment. Try again."), cx);
        }
    }

    /// `artifact-direct-comment`: the open chat sends the batch as its own turn; true when it
    /// did, so the batch clears. A turn that cannot start now keeps the batch (bundle
    /// `artifactAnnotationBatch.sendFailed`).
    pub(super) fn send_artifact_annotations(
        &mut self,
        comments: &[Comment],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let submission = self
            .thread
            .is_some()
            .then(|| self.thread_composer.read(cx).comments_submission(comments))
            .flatten();
        let Some(submission) = submission else {
            self.toast(Toast::danger("Could not send requests. Try again."), cx);
            return false;
        };
        // The batch is its own turn: a queued message taken back with Edit keeps its slot.
        let edit = self.queue_edit.take();
        self.send(None, submission, None, window, cx);
        self.queue_edit = edit;
        true
    }

    /// The open chat's browser annotations that no turn has sent yet.
    pub(super) fn pending_browser_annotations(&self, cx: &App) -> usize {
        self.thread_composer
            .read(cx)
            .diff_comments()
            .iter()
            .filter(|comment| comment.is_browser())
            .count()
    }

    /// Every browser tab's annotation toolbar counts the chat's pending annotations.
    fn sync_browser_annotations(&self, cx: &mut App) {
        let pending = self.pending_browser_annotations(cx);
        for (_, tab) in &self.browser_tabs {
            tab.update(cx, |tab, cx| tab.set_pending_annotations(pending, cx));
        }
    }

    pub(super) fn add_browser_annotation(
        &mut self,
        _tab: &Entity<ui::browser::BrowserTab>,
        comment: &Comment,
        cx: &mut Context<Self>,
    ) {
        let Some(_thread) = self.thread.as_ref() else {
            return;
        };
        #[cfg(target_os = "macos")]
        if let Some(capture) = _tab.update(cx, |tab, cx| {
            let only_on_drag = cx.try_global::<Settings>().is_some_and(|store| {
                store
                    .read_all()
                    .get("browser-annotation-screenshots-mode")
                    .and_then(Value::as_str)
                    == Some("necessary")
            });
            tab.annotation_screenshot(comment, only_on_drag, cx)
        }) {
            let thread = _thread.read(cx).id().to_owned();
            self.capture_browser_annotation(thread, comment.clone(), capture, cx);
            return;
        }
        self.append_browser_annotation(comment.clone(), cx);
    }

    fn append_browser_annotation(&mut self, comment: Comment, cx: &mut Context<Self>) {
        let mut pending = self.thread_composer.read(cx).diff_comments().to_vec();
        pending.push(comment);
        self.change_diff_comments(&pending, cx);
    }

    #[cfg(target_os = "macos")]
    fn capture_browser_annotation(
        &self,
        thread: String,
        mut comment: Comment,
        capture: impl Future<Output = Option<Vec<u8>>> + 'static,
        cx: &mut Context<Self>,
    ) {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        self.set_browser_annotation_capture_pending(true, cx);
        cx.spawn(async move |this, cx| {
            let timeout = cx
                .background_executor()
                .timer(std::time::Duration::from_secs(5));
            if let Some(png) = futures_lite::future::or(capture, async {
                timeout.await;
                None
            })
            .await
            {
                let mut url = "data:image/png;base64,".to_owned();
                STANDARD.encode_string(&png, &mut url);
                comment
                    .extra
                    .insert("localBrowserScreenshot".into(), json!({"dataUrl": url}));
            }
            let _ = this.update(cx, |this, cx| {
                this.set_browser_annotation_capture_pending(false, cx);
                if this
                    .thread
                    .as_ref()
                    .is_some_and(|open| open.read(cx).id() == thread)
                {
                    this.append_browser_annotation(comment, cx);
                } else {
                    let mut pending = stored_comments(cx, "diff_comments", &thread);
                    pending.push(comment);
                    this.save_diff_comments(thread, &pending, cx);
                }
            });
        })
        .detach();
    }

    #[cfg(target_os = "macos")]
    fn set_browser_annotation_capture_pending(&self, pending: bool, cx: &mut App) {
        self.thread_composer.update(cx, |composer, cx| {
            composer.set_browser_annotation_capture_pending(pending, cx);
        });
        for (_, tab) in &self.browser_tabs {
            tab.update(cx, |tab, cx| {
                tab.set_annotation_capture_pending(pending, cx);
            });
        }
    }

    /// The annotation toolbar's Send: the pending browser annotations go out as their own
    /// turn and leave the pending list; diff comments stay for the next message.
    pub(super) fn send_browser_annotations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .thread_composer
            .read(cx)
            .browser_annotation_capture_pending()
        {
            return;
        }
        let (browser, rest): (Vec<Comment>, Vec<Comment>) = self
            .thread_composer
            .read(cx)
            .diff_comments()
            .iter()
            .cloned()
            .partition(Comment::is_browser);
        let submission = self
            .thread
            .is_some()
            .then(|| self.thread_composer.read(cx).comments_submission(&browser))
            .flatten();
        let Some(submission) = submission else {
            self.toast(Toast::danger("Could not send requests. Try again."), cx);
            return;
        };
        let edit = self.queue_edit.take();
        self.send(None, submission, None, window, cx);
        self.queue_edit = edit;
        self.change_diff_comments(&rest, cx);
    }

    /// `thread.browser.tweaks.cancelConfirm*`: Discard drops every pending browser annotation.
    pub(super) fn confirm_discard_browser_annotations(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_dialog() {
            return;
        }
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Discard all pending annotations?",
                "All edits will be removed from the page",
                "Discard",
                window,
                cx,
            )
            .destructive()
        });
        cx.subscribe(&dialog, |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                let rest: Vec<Comment> = this
                    .thread_composer
                    .read(cx)
                    .diff_comments()
                    .iter()
                    .filter(|comment| !comment.is_browser())
                    .cloned()
                    .collect();
                this.change_diff_comments(&rest, cx);
            }
            this.diff_confirmation = None;
            this.sync_browser_pages(cx);
            cx.notify();
        })
        .detach();
        // The native page draws over every gpui element, the dialog included.
        for (_, tab) in &self.browser_tabs {
            tab.update(cx, |tab, cx| tab.set_page_hidden(true, cx));
        }
        self.diff_confirmation = Some(dialog);
        cx.notify();
    }
}
