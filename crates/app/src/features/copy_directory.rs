//! `copyWorkingDirectory` / `copyConversationPath` (the original runs one function for both:
//! `Gda`, refs: data/cgx-fam-menus-ide/clipboard-contracts-048.json): for an app server that
//! supports `threadAttachments`, refresh the thread's attachments and copy the newest worktree's
//! workspace root (minus worktrees with a pending archive), else the cwd. A failed refresh
//! copies nothing.

use crate::*;
use gpui::ClipboardItem;

actions!(app, [CopyWorkingDirectory]);

/// The app server that first answers `thread/attachment/list` (`threadAttachments`).
const MIN_VERSION: &str = "0.155.0-alpha.2";

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &CopyWorkingDirectory, window, cx| {
        this.copy_working_directory(window, cx)
    }))
}

impl Root {
    fn copy_working_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Route::Thread(id), Some(thread)) = (self.state.route.clone(), self.thread.as_ref())
        else {
            return;
        };
        let cwd = thread.read(cx).cwd().to_owned();
        let (bridge, host) = (self.bridge.clone(), self.host.clone());
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let root = async {
                let agent = bridge
                    .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                    .await?;
                if bridge.is_acp() || !supports_thread_attachments(&agent.user_agent) {
                    return Ok(None);
                }
                let mut attachments = Vec::new();
                let mut cursor = Value::Null;
                loop {
                    let page: Value = bridge
                        .request(
                            "thread/attachment/list",
                            json!({ "threadId": id, "cursor": cursor, "limit": 100 }),
                        )
                        .await?;
                    attachments.extend(page["data"].as_array().cloned().unwrap_or_default());
                    cursor = page["nextCursor"].clone();
                    if cursor.is_null() {
                        break;
                    }
                }
                let thread_id = id.clone();
                let pending = cx
                    .background_spawn(async move { host.pending_worktree_archives(&thread_id) })
                    .await?;
                Ok::<_, anyhow::Error>(latest_workspace_root(&attachments, &pending))
            }
            .await;
            this.update(cx, |this, cx| match root {
                Ok(root) => {
                    let text = root.unwrap_or(cwd);
                    if !text.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.toast(Toast::success("Copied working directory"), cx);
                    }
                }
                Err(_) => this.toast(Toast::danger("Unable to copy working directory"), cx),
            })
        });
        log_err(task, cx);
    }
}

/// `jM(version, "threadAttachments")`: `0.0.0` is the original's everything-supported sentinel;
/// an unparsable version supports nothing. The app-server version is the first `/`-separated
/// token of the initialize `userAgent` (`/^.+?\/(?<version>\S+)/`).
fn supports_thread_attachments(user_agent: &str) -> bool {
    let Some(version) = user_agent
        .split_once('/')
        .filter(|(name, _)| !name.is_empty())
        .and_then(|(_, rest)| rest.split_whitespace().next())
    else {
        return false;
    };
    static MINIMUM: std::sync::LazyLock<semver::Version> =
        std::sync::LazyLock::new(|| semver::Version::parse(MIN_VERSION).expect("valid version"));
    version == "0.0.0" || semver::Version::parse(version).is_ok_and(|version| version >= *MINIMUM)
}

/// `u2`'s select over the attachment pages: worktree attachments whose `identityKey` is their
/// root, minus the roots of pending archives (`AUr`); the newest `createdAt` wins and a later
/// equal one beats an earlier one. Its `workspaceRoot` is the text to copy.
fn latest_workspace_root(
    attachments: &[Value],
    pending: &[crate::host::PendingArchive],
) -> Option<String> {
    let mut latest: Option<(i64, &str)> = None;
    for attachment in attachments {
        let payload = &attachment["payload"];
        let (Some(root), Some(workspace), Some(key), Some(created)) = (
            payload["root"].as_str().filter(|s| !s.is_empty()),
            payload["workspaceRoot"].as_str().filter(|s| !s.is_empty()),
            attachment["identityKey"].as_str(),
            attachment["createdAt"].as_i64(),
        ) else {
            continue;
        };
        if attachment["attachmentType"] != "worktree"
            || !same_path(key, root)
            || pending.iter().any(|archive| same_path(&archive.root, root))
        {
            continue;
        }
        if latest.is_none_or(|(newest, _)| created >= newest) {
            latest = Some((created, workspace));
        }
    }
    latest.map(|(_, workspace)| workspace.to_owned())
}

/// The original's path identity `WM` for the macOS and Linux paths this app sees: trailing
/// slashes do not change a location.
// shortcut: no Windows drive, UNC or WSL spellings; port the rest of `WM` if the app ships on Windows.
fn same_path(a: &str, b: &str) -> bool {
    fn trim(path: &str) -> &str {
        match path.trim_end_matches('/') {
            "" if path.starts_with('/') => "/",
            trimmed => trimmed,
        }
    }
    trim(a) == trim(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::PendingArchive;

    fn worktree(root: &str, workspace: &str, created: i64) -> Value {
        json!({ "attachmentType": "worktree", "identityKey": root, "createdAt": created,
            "payload": { "root": root, "workspaceRoot": workspace } })
    }

    #[test]
    fn the_gate_follows_the_original_version_rule() {
        for (agent, supported) in [
            ("codex_cli_rs/0.155.0-alpha.2 (Mac OS)", true),
            ("codex_cli_rs/0.155.0-alpha.1", false),
            ("codex_cli_rs/0.155.0-alpha.10", true),
            ("codex_cli_rs/0.155.0", true),
            ("codex_cli_rs/0.154.9", false),
            ("codex_cli_rs/1.0.0+build.5", true),
            ("codex_cli_rs/0.0.0", true),
            ("codex_cli_rs/dev", false),
            ("no-version", false),
            ("/0.200.0", false),
        ] {
            assert_eq!(supports_thread_attachments(agent), supported, "{agent}");
        }
    }

    #[test]
    fn the_newest_unarchived_worktree_supplies_the_workspace_root() {
        let attachments = [
            worktree("/w/a", "/w/a/app", 10),
            worktree("/w/b", "/w/b/app", 30),
            worktree("/w/c", "/w/c/app", 30),
            json!({ "attachmentType": "pull_request", "identityKey": "x", "createdAt": 99 }),
            worktree("/w/d", "/w/d/app", 50),
        ];
        let none: [PendingArchive; 0] = [];
        let archived = [PendingArchive {
            root: "/w/d/".into(),
        }];
        assert_eq!(
            latest_workspace_root(&attachments, &none).as_deref(),
            Some("/w/d/app")
        );
        assert_eq!(
            latest_workspace_root(&attachments, &archived).as_deref(),
            Some("/w/c/app")
        );
        assert_eq!(latest_workspace_root(&[], &none), None);
    }
}
