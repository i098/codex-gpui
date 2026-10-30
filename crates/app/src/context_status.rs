//! The composer's context window donut: each open chat's `thread/tokenUsage/updated`, shown
//! while the host setting `show-context-window-usage` (`Show context window usage`) is on.

use crate::{Root, host_settings};
use gpui::Context;
use serde_json::Value;

/// The host setting key behind `Show context window usage`.
pub(crate) const SETTING: &str = "show-context-window-usage";

impl Root {
    /// Copies each open chat's context use to its composer, or clears it with the setting off.
    pub(crate) fn sync_context_usage(&self, cx: &mut Context<Self>) {
        let shown = cx
            .try_global::<host_settings::Settings>()
            .is_some_and(|s| s.read_all().get(SETTING) == Some(&Value::Bool(true)));
        let side = self
            .side_chats
            .iter()
            .map(|chat| (&chat.view, &chat.composer));
        for (view, composer) in self.thread.iter().zip([&self.thread_composer]).chain(side) {
            let usage = shown.then(|| view.read(cx).context_usage()).flatten();
            composer.update(cx, |c, cx| c.set_context_usage(usage, cx));
        }
    }
}
