use super::*;
use crate::confirmation::Outcome;
use gpui::AppContext as _;

impl Composer {
    /// Enables the local thread's confirmed shell action.
    pub fn enable_shell_commands(&mut self) {
        self.shell_commands = true;
    }

    pub(super) fn shell_disabled_reason(&self) -> Option<&'static str> {
        if !self.shell_commands {
            return None;
        }
        let command = self.content.strip_prefix('!')?;
        if self.permission != Permission::FullAccess {
            Some("Shell commands require Full access because they run outside the sandbox.")
        } else if [
            self.pending_attachments > 0,
            !self.attachments.is_empty(),
            !self.mentions.is_empty(),
            !self.diff_comments.is_empty(),
        ]
        .contains(&true)
        {
            Some("Remove attachments, mentions, and diff comments before running a shell command.")
        } else if command.trim().is_empty() {
            Some("Enter a shell command after !.")
        } else {
            None
        }
    }

    pub(super) fn confirm_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shell_disabled_reason().is_some() {
            return;
        }
        self.cancel_dictation(cx);
        let content = self.content.clone();
        let description = format!(
            "This command runs with full access, outside the sandbox, in this chat's workspace.\n\n{}",
            &content[1..]
        );
        let dialog = cx.new(|cx| {
            Confirmation::new("Run shell command?", description, "Run command", window, cx)
                .destructive()
        });
        cx.subscribe_in(&dialog, window, move |this, _, outcome, window, cx| {
            this.blocked = None;
            if *outcome == Outcome::Confirm
                && this.content == content
                && this.shell_disabled_reason().is_none()
            {
                this.finish_send_draft(false, window, cx);
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        })
        .detach();
        self.blocked = Some(dialog);
        cx.notify();
    }
}
