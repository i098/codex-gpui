//! `Code review` (`/review-mode`), the CLI's `/review`: the original's submenu rows
//! `Review uncommitted changes` and, under `Review against a base branch`, the chat's branches,
//! plus the CLI's `Review a commit` picker. Opening it emits [`ReviewEvent::LoadTargets`]; a pick
//! emits [`ReviewEvent::Start`] with the `review/start` target. Escape sends nothing.
use super::slash::{SubAction, SubItem};
use super::*;

pub enum ReviewEvent {
    /// The submenu opened: read the chat's branches and commits for
    /// [`Composer::set_review_targets`].
    LoadTargets,
    /// Run `review/start` on the open chat with this `target`.
    Start(Value),
}

impl gpui::EventEmitter<ReviewEvent> for Composer {}

const BASE: &str = "Review against a base branch";
const COMMIT: &str = "Review a commit";

impl Composer {
    /// The `git-review-targets` answer (`branches`, `commits`), or `Err` when it failed.
    pub fn set_review_targets(&mut self, targets: Result<Value, ()>, cx: &mut Context<Self>) {
        self.slash.review = Some(targets);
        let items = self.review_items();
        if let Some(sub) = self
            .slash
            .sub
            .as_mut()
            .filter(|sub| sub.id == "review-mode")
        {
            sub.items = items;
            self.refresh_sub(cx);
        }
    }

    pub(super) fn review_items(&self) -> Vec<SubItem> {
        let row = |title: &str, description: &str, icon, heading, action| SubItem {
            title: title.into(),
            description: description.into(),
            icon,
            heading,
            action,
        };
        let mut items = vec![row(
            "Review uncommitted changes",
            "",
            "code-review",
            None,
            SubAction::Review(json!({"type": "uncommittedChanges"})),
        )];
        let targets = match &self.slash.review {
            None => Err("Loading branches…"),
            Some(Err(())) => Err("Unable to load branches"),
            Some(Ok(targets)) => Ok(targets),
        };
        let targets = match targets {
            Ok(targets) => targets,
            Err(status) => {
                items.push(row(status, "", "branch", Some(BASE), SubAction::Inactive));
                return items;
            }
        };
        let list = |key: &str| targets[key].as_array().cloned().unwrap_or_default();
        items.extend(
            list("branches")
                .iter()
                .filter_map(Value::as_str)
                .map(|branch| {
                    row(
                        branch,
                        "",
                        "branch",
                        Some(BASE),
                        SubAction::Review(json!({"type": "baseBranch", "branch": branch})),
                    )
                }),
        );
        items.extend(list("commits").iter().filter_map(|commit| {
            let sha = commit["sha"].as_str()?;
            let subject = commit["subject"].as_str()?;
            Some(row(
                subject,
                sha.get(..7).unwrap_or(sha),
                "commit-branch",
                Some(COMMIT),
                SubAction::Review(json!({"type": "commit", "sha": sha, "title": subject})),
            ))
        }));
        items
    }
}
