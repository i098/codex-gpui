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
    }

    pub(super) fn change_diff_comments(&mut self, comments: &[Comment], cx: &mut Context<Self>) {
        let Some(thread) = self
            .thread
            .as_ref()
            .map(|thread| thread.read(cx).id().to_owned())
        else {
            return;
        };
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
        self.thread_composer.update(cx, |composer, cx| {
            composer.set_diff_comments(comments.to_vec(), cx)
        });
        if let Some(panel) = &self.diff {
            panel.update(cx, |panel, cx| {
                panel.set_user_comments(comments.to_vec(), cx)
            });
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
}
