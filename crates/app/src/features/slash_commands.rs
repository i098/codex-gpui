//! Composer slash commands that call the app-server directly: `/status`, `/init`, `/side`,
//! `/review-mode`, `/personality`, `/goal`, and the Home-only `/worktree`.

use crate::*;
use ui::composer::Goal;
use ui::screens::hotkey::Mode;

/// The original's `/init` prompt (the Codex bundle's `Oit`).
const INIT_PROMPT: &str = "Generate a file named AGENTS.md that serves as a contributor guide for this repository.
Your goal is to produce a clear, concise, and well-structured document with descriptive headings and actionable explanations for each section.
Follow the outline below, but adapt as needed — add sections if relevant, and omit those that do not apply to this project.

Document Requirements

- Title the document \"Repository Guidelines\".
- Use Markdown headings (#, ##, etc.) for structure.
- Keep the document concise. 200-400 words is optimal.
- Keep explanations short, direct, and specific to this repository.
- Provide examples where helpful (commands, directory paths, naming patterns).
- Maintain a professional, instructional tone.

Recommended Sections

Project Structure & Module Organization

- Outline the project structure, including where the source code, tests, and assets are located.

Build, Test, and Development Commands

- List key commands for building, testing, and running locally (e.g., npm test, make build).
- Briefly explain what each command does.

Coding Style & Naming Conventions

- Specify indentation rules, language-specific style preferences, and naming patterns.
- Include any formatting or linting tools used.

Testing Guidelines

- Identify testing frameworks and coverage requirements.
- State test naming conventions and how to run tests.

Commit & Pull Request Guidelines

- Summarize commit message conventions found in the project’s Git history.
- Outline pull request requirements (descriptions, linked issues, screenshots, etc.).

(Optional) Add other sections if relevant, such as Security & Configuration Tips, Architecture Overview, or Agent-Specific Instructions.
";

/// `Used 40% of the 5h limit, resets in 2h 10m`-style text of one `account/rateLimits/read`
/// window, or `None` without a percentage.
fn limit_line(window: &Value) -> Option<String> {
    let used = window["usedPercent"].as_f64()?;
    let name = match window["windowDurationMins"].as_u64() {
        Some(300) => "5h".to_owned(),
        Some(10080) => "weekly".to_owned(),
        Some(mins) if mins % 60 == 0 => format!("{}h", mins / 60),
        Some(mins) => format!("{mins}m"),
        None => "usage".to_owned(),
    };
    Some(format!("{name} limit {}% used", used.round()))
}

/// The `/status` text: chat id, context usage and each rate limit window.
fn status_text(
    id: &str,
    usage: Option<ui::composer::ContextUsage>,
    limits: Option<&Value>,
) -> String {
    let mut lines = vec![format!("Chat ID: {id}")];
    lines.push(match usage {
        Some(usage) => format!(
            "Context: {}% used ({}% left)",
            usage.percent(),
            100 - usage.percent()
        ),
        None => "Context: not reported yet".into(),
    });
    let windows = ["primary", "secondary"]
        .iter()
        .filter_map(|key| limit_line(&limits?["rateLimits"][key]))
        .collect::<Vec<_>>();
    lines.push(if windows.is_empty() {
        "Rate limits: unavailable".into()
    } else {
        format!("Rate limits: {}", windows.join(", "))
    });
    lines.join("\n")
}

impl Root {
    /// The open chat's view, or a toast naming what `action` needs.
    fn command_thread(
        &mut self,
        action: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ThreadView>> {
        let view = self
            .thread
            .clone()
            .filter(|_| matches!(self.state.route, Route::Thread(_)));
        if view.is_none() {
            self.toast(Toast::danger(format!("Open a chat before {action}")), cx);
        }
        view
    }

    fn turn_running(view: &Entity<ThreadView>, cx: &App) -> bool {
        view.read(cx)
            .turns()
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress)
    }

    /// `/status`: the chat id, context window use and rate limits, as one toast.
    pub(crate) fn show_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.command_thread("showing its status", cx) else {
            return;
        };
        let id = view.read(cx).id().to_owned();
        let usage = view.read(cx).context_usage();
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let limits = bridge
                .request::<_, Value>("account/rateLimits/read", json!({}))
                .await
                .ok();
            this.update(cx, |this, cx| {
                this.toast(Toast::info(status_text(&id, usage, limits.as_ref())), cx)
            })
        });
        log_err(task, cx);
    }

    /// `/init`: sends the create-AGENTS.md prompt unless the chat's folder has one.
    pub(crate) fn init_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.command_thread("running /init", cx) else {
            return;
        };
        if Self::turn_running(&view, cx) {
            self.toast(
                Toast::danger("Wait for the current turn before running /init"),
                cx,
            );
            return;
        }
        let (id, cwd) = (
            view.read(cx).id().to_owned(),
            view.read(cx).cwd().to_owned(),
        );
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let exists = cx
                .background_spawn(
                    async move { std::path::Path::new(&cwd).join("AGENTS.md").exists() },
                )
                .await;
            if exists {
                this.update(cx, |this, cx| {
                    this.toast(
                        Toast::info(
                            "AGENTS.md already exists here. Skipping /init to avoid overwriting it",
                        ),
                        cx,
                    )
                })?;
                return Ok(());
            }
            let sent = async {
                resume_if(resume, &bridge, &this, &id, cx).await?;
                bridge
                    .request::<_, Value>(
                        "turn/start",
                        json!({"threadId": id,
                            "input": [{"type": "text", "text": INIT_PROMPT, "text_elements": []}]}),
                    )
                    .await
                    .map(drop)
            }
            .await;
            if sent.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Failed to run /init"), cx)
                })?;
            }
            sent
        });
        log_err(task, cx);
    }

    /// `/side`: a new empty side chat forked from the open chat.
    pub(crate) fn start_side_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.command_thread("starting a side chat", cx) else {
            return;
        };
        let source = view.read(cx).id().to_owned();
        let resume = !self.loaded.contains(&source);
        let title: SharedString = match self.side_chats.len() {
            0 => "Side chat".into(),
            n => format!("Side chat {}", n + 1).into(),
        };
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let started = async {
                resume_if(resume, &bridge, &this, &source, cx).await?;
                let params = json!({
                    "threadId": source,
                    "ephemeral": true,
                    "threadSource": "user",
                    "developerInstructions": SIDE_CHAT_INSTRUCTIONS,
                    "excludeTurns": true,
                });
                let (thread, settings) = routes::forked_thread(
                    bridge.request::<_, Value>("thread/fork", params).await?,
                )?;
                let fork_id = thread.id.clone();
                let key = this.update_in(cx, |this, window, cx| {
                    this.add_side_chat(thread, settings, title, window, cx)
                })?;
                let boundary = json!([{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": SIDE_CHAT_BOUNDARY }],
                }]);
                let injected = bridge
                    .request::<_, Value>(
                        "thread/inject_items",
                        json!({ "threadId": fork_id, "items": boundary }),
                    )
                    .await;
                if injected.is_err() {
                    this.update_in(cx, |this, window, cx| {
                        this.close_side(SidePanel::SideChat(key), window, cx)
                    })?;
                }
                injected.map(drop)
            }
            .await;
            if started.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger(SIDE_CHAT_FAILED), cx)
                })?;
            }
            started
        });
        log_err(task, cx);
    }

    /// `/review-mode` opened: reads the chat folder's local branches into `composer`.
    pub(crate) fn load_review_branches(
        &mut self,
        composer: Entity<Composer>,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self
            .thread
            .as_ref()
            .map(|view| view.read(cx).cwd().to_owned())
        else {
            return;
        };
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            git_call(&*host, "git-commit-status", json!({"cwd": cwd}))
        });
        cx.spawn(async move |_, cx| {
            let branches: Vec<String> = read
                .await
                .ok()
                .and_then(|status| {
                    status["branches"].as_array().map(|names| {
                        names
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                })
                .unwrap_or_default();
            composer
                .update(cx, |composer, cx| {
                    composer.set_review_branches(branches, cx)
                })
                .ok();
        })
        .detach();
    }

    /// `review/start` on the open chat for the working tree (`None`) or against `branch`.
    pub(crate) fn start_review(
        &mut self,
        branch: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.command_thread("starting a review", cx) else {
            return;
        };
        if Self::turn_running(&view, cx) {
            self.toast(
                Toast::danger("Wait for the current turn before starting a review"),
                cx,
            );
            return;
        }
        let id = view.read(cx).id().to_owned();
        let resume = !self.loaded.contains(&id);
        let target = match branch {
            None => json!({"type": "uncommittedChanges"}),
            Some(branch) => json!({"type": "baseBranch", "branch": branch}),
        };
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let started = async {
                resume_if(resume, &bridge, &this, &id, cx).await?;
                bridge
                    .request::<_, Value>(
                        "review/start",
                        json!({"threadId": id, "target": target, "delivery": "inline"}),
                    )
                    .await
                    .map(drop)
            }
            .await;
            if started.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Failed to start review"), cx)
                })?;
            }
            started
        });
        log_err(task, cx);
    }

    /// `/personality`: writes the `personality` setting for the chats that follow.
    pub(crate) fn set_personality(&mut self, name: &'static str, cx: &mut Context<Self>) {
        self.write_config(
            "config/batchWrite",
            json!({
                "edits": [{"keyPath": "personality", "value": name, "mergeStrategy": "upsert"}],
                "filePath": null, "expectedVersion": null, "reloadUserConfig": true,
            }),
            cx,
            move |config| config["personality"] = name.into(),
        );
        let label = if name == "friendly" {
            "Friendly"
        } else {
            "Pragmatic"
        };
        self.toast(Toast::success(format!("Personality set to {label}")), cx);
    }

    /// `/goal <objective>` sets the open chat's goal; without text it shows the current one.
    pub(crate) fn set_goal(
        &mut self,
        objective: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.command_thread("setting a goal", cx) else {
            return;
        };
        if objective.is_empty() {
            let message = view.read(cx).goal().map_or_else(
                || "Type /goal followed by what Codex should keep working on".to_owned(),
                Goal::label,
            );
            self.toast(Toast::info(message), cx);
            return;
        }
        let id = view.read(cx).id().to_owned();
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let set = async {
                resume_if(resume, &bridge, &this, &id, cx).await?;
                bridge
                    .request::<_, Value>(
                        "thread/goal/set",
                        json!({"threadId": id, "objective": objective, "status": "active"}),
                    )
                    .await
            }
            .await;
            this.update(cx, |this, cx| match &set {
                Ok(response) => {
                    view.update(cx, |view, cx| {
                        view.set_goal(Goal::from_value(&response["goal"]), cx)
                    });
                    this.sync_goal(cx);
                }
                Err(_) => this.toast(Toast::danger("Failed to set goal"), cx),
            })?;
            set.map(drop)
        });
        log_err(task, cx);
    }

    /// The goal pill's click: `thread/goal/clear` on the open chat.
    pub(crate) fn clear_goal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.thread.clone() else {
            return;
        };
        let id = view.read(cx).id().to_owned();
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let cleared = bridge
                .request::<_, Value>("thread/goal/clear", json!({"threadId": id}))
                .await;
            this.update(cx, |this, cx| match &cleared {
                Ok(_) => {
                    view.update(cx, |view, cx| view.set_goal(None, cx));
                    this.sync_goal(cx);
                }
                Err(_) => this.toast(Toast::danger("Failed to clear goal"), cx),
            })?;
            cleared.map(drop)
        });
        log_err(task, cx);
    }

    /// Reads the shown chat's goal (`thread/goal/get`); a server without goals leaves none.
    pub(crate) fn load_goal(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.thread.clone() else {
            return;
        };
        let id = view.read(cx).id().to_owned();
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let Ok(response) = bridge
                .request::<_, Value>("thread/goal/get", json!({"threadId": id}))
                .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                view.update(cx, |view, cx| {
                    view.set_goal(Goal::from_value(&response["goal"]), cx)
                });
                this.sync_goal(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Copies each open chat's goal to its composer.
    pub(crate) fn sync_goal(&self, cx: &mut Context<Self>) {
        let side = self
            .side_chats
            .iter()
            .map(|chat| (&chat.view, &chat.composer));
        for (view, composer) in self.thread.iter().zip([&self.thread_composer]).chain(side) {
            let goal = view.read(cx).goal().cloned();
            composer.update(cx, |composer, cx| composer.set_goal(goal, cx));
        }
    }

    /// After a `thread/goal/*` notification: refreshes the pill and announces an achieved goal.
    pub(crate) fn goal_notification(
        &mut self,
        method: &str,
        params: &Value,
        cx: &mut Context<Self>,
    ) {
        self.sync_goal(cx);
        if method == "thread/goal/updated"
            && Goal::from_value(&params["goal"]).is_some_and(|goal| goal.achieved())
        {
            self.toast(Toast::success("Goal achieved"), cx);
        }
    }

    /// `/worktree`: flips whether the next Home chat runs in a new worktree.
    pub(crate) fn toggle_worktree_run(&mut self, cx: &mut Context<Self>) {
        let outcome = self.home.update(cx, |home, cx| {
            if home.project.is_none() || home.location.worktree_available != Some(true) {
                return None;
            }
            home.location.mode = match home.location.mode {
                Mode::Worktree => Mode::Local,
                _ => Mode::Worktree,
            };
            cx.notify();
            Some(home.location.mode == Mode::Worktree)
        });
        let message = match outcome {
            Some(true) => Toast::info("The next chat runs in a new worktree"),
            Some(false) => Toast::info("The next chat runs in the project folder"),
            None => Toast::danger("Choose a Git project to run chats in a worktree"),
        };
        self.toast(message, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lists_the_known_windows_only() {
        let limits = json!({"rateLimits": {
            "primary": {"usedPercent": 40.4, "windowDurationMins": 300},
            "secondary": null,
        }});
        let text = status_text("abc", None, Some(&limits));
        assert_eq!(
            text,
            "Chat ID: abc\nContext: not reported yet\nRate limits: 5h limit 40% used"
        );
        assert!(status_text("abc", None, None).ends_with("Rate limits: unavailable"));
    }
}
