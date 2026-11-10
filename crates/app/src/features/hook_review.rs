//! The composer's `Review hooks` button and dialog (`ComposerHookReview`, `HookReviewDialog`).
//! `hooks/list` of the composer's folder runs when a chat opens and when it sends a turn; enabled,
//! unmanaged hooks whose `trustStatus` is `untrusted` or `modified` cannot run until approved.
//! Approval writes `hooks.state.<key>.trusted_hash` through the Settings > Hooks trust path.
use crate::*;
use ui::composer::ReviewHooks;
use ui::screens::settings::integrations::needs_review;

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(
        cx.listener(|this, _: &ReviewHooks, window, cx| this.open_hook_review(window, cx)),
    )
}

/// The hooks of `listed` (a `hooks/list` result) that `cwd`'s composer offers for review.
fn hooks_to_review(listed: &Value, cwd: &str) -> Vec<Value> {
    let mut hooks: Vec<Value> = Vec::new();
    for hook in listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["cwd"] == cwd)
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
        .filter(|hook| hook["enabled"] == true && hook["isManaged"] != true && needs_review(hook))
    {
        if !hooks.iter().any(|seen| seen["key"] == hook["key"]) {
            hooks.push(hook.clone());
        }
    }
    hooks
}

impl Root {
    /// Lists `cwd`'s hooks and shows the composer's `Review hooks` button while any need approval.
    pub(crate) fn refresh_hook_review(
        &mut self,
        cwd: String,
        composer: Entity<ui::composer::Composer>,
        cx: &mut Context<Self>,
    ) {
        if self.bridge.is_acp() {
            return;
        }
        let bridge = self.bridge.clone();
        let roots = json!({ "cwds": [cwd] });
        let list = cx
            .background_spawn(async move { bridge.request::<_, Value>("hooks/list", roots).await });
        cx.spawn(async move |this, cx| {
            let hooks = match list.await {
                Ok(listed) => hooks_to_review(&listed, &cwd),
                Err(error) => {
                    eprintln!("codex-gpui: hooks/list: {error:#}");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                composer.update(cx, |composer, cx| {
                    composer.set_hooks_to_review(hooks.len(), cx)
                });
                this.hook_review = (cwd, hooks);
            })
            .ok();
        })
        .detach();
    }

    /// The button opens the dialog on the hooks it counted.
    fn open_hook_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hooks = self.hook_review.1.clone();
        if hooks.is_empty() {
            return;
        }
        let dialog = cx.new(|cx| Dialog::hook_review(hooks, cx));
        self.show_dialog(dialog, window, cx);
        self.dialog_to_composer = true;
    }

    /// `Allow selected` or `Allow all`: trusts `hooks` (key, hash), then lists the folder again.
    pub(crate) fn trust_reviewed_hooks(
        &mut self,
        dialog: Entity<Dialog>,
        hooks: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let cwd = self.hook_review.0.clone();
        let roots = vec![cwd.clone()];
        let trust = cx.background_spawn(async move {
            crate::settings_integrations::trust_hooks(&bridge, &hooks, &roots).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = trust.await;
            this.update_in(cx, |this, _, cx| {
                let error = result.err().map(|error| {
                    eprintln!("codex-gpui: trust hooks: {error:#}");
                    error.to_string() == settings_integrations::HOOKS_OVERRIDDEN
                });
                if this.dialog.as_ref() == Some(&dialog) {
                    dialog.update(cx, |dialog, cx| dialog.hook_review_finished(error, cx));
                }
                let composer = this.composer(cx);
                this.refresh_hook_review(cwd, composer, cx);
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_keeps_enabled_unmanaged_untrusted_hooks_of_the_folder_once() {
        let hook = |key: &str, trust: &str, enabled: bool, managed: bool| json!({"key": key, "trustStatus": trust, "enabled": enabled, "isManaged": managed});
        let listed = json!({"data": [
            {"cwd": "/p", "hooks": [
                hook("new", "untrusted", true, false),
                hook("changed", "modified", true, false),
                hook("trusted", "trusted", true, false),
                hook("off", "untrusted", false, false),
                hook("managed", "untrusted", true, true),
                hook("new", "untrusted", true, false),
            ]},
            {"cwd": "/other", "hooks": [hook("elsewhere", "untrusted", true, false)]},
        ]});
        let keys: Vec<_> = hooks_to_review(&listed, "/p")
            .iter()
            .map(|hook| hook["key"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(keys, ["new", "changed"]);
    }
}
