//! A Recents row menu's thread actions (`refs/interactions/s002.json`, the original's `jJo`):
//! `Section`, `Fork`, `Copy`, `Open in new window` and `Permanently delete` on the row's
//! thread, open or not.

use crate::*;
use bridge::types::{ThreadItem, TurnStatus, UserInput};
use gpui::ClipboardItem;
use ui::confirmation::{Confirmation, Outcome as ConfirmationOutcome};
use ui::screens::sidebar::{ThreadCommand, ThreadMenuAction};

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, action: &ThreadMenuAction, window, cx| {
        let id = action.thread.clone();
        match &action.command {
            ThreadCommand::MoveToSection(section) => this.move_to_section(id, section.clone(), cx),
            ThreadCommand::NewSection => this.new_section_for(id, window, cx),
            ThreadCommand::Fork => this.fork_thread(id, 0, window, cx),
            ThreadCommand::CopyDeeplink => {
                cx.write_to_clipboard(ClipboardItem::new_string(format!("codex://threads/{id}")));
            }
            ThreadCommand::CopyMarkdown => this.copy_markdown(id, cx),
            ThreadCommand::CopyWorkingDirectory => {
                if let Some(cwd) = this
                    .threads
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.cwd.clone())
                {
                    this.copy_working_directory(id, cwd, window, cx);
                }
            }
            ThreadCommand::OpenInNewWindow => this.open_thread_window(id, cx),
            ThreadCommand::Delete => this.confirm_delete(id, window, cx),
        }
    }))
}

impl Root {
    /// `Copy as Markdown`: `thread/read` with its turns, rendered as the original's
    /// `conversation-markdown` does, then `Copied conversation as Markdown`.
    fn copy_markdown(&mut self, id: String, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let read = bridge
                .request::<_, ThreadReadResponse>(
                    "thread/read",
                    json!({"threadId": id, "includeTurns": true}),
                )
                .await;
            this.update(cx, |this, cx| match &read {
                Ok(read) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(markdown(&read.thread)));
                    this.toast(Toast::success("Copied conversation as Markdown"), cx);
                }
                Err(_) => this.toast(Toast::danger("Failed to copy conversation as Markdown"), cx),
            })?;
            read.map(drop)
        });
        log_err(task, cx);
    }

    /// `Permanently delete`: `Delete task?`, then `thread/delete`; the row leaves the sidebar.
    fn confirm_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcut_state.confirmation.is_some() {
            return;
        }
        let confirmation = cx.new(|cx| {
            Confirmation::new(
                "Delete task?",
                "This permanently deletes the task and its subagent tasks. This cannot be undone.",
                "Delete",
                window,
                cx,
            )
            .destructive()
        });
        cx.subscribe_in(
            &confirmation,
            window,
            move |this, _, outcome, window, cx| {
                this.shortcut_state.confirmation = None;
                if *outcome == ConfirmationOutcome::Confirm {
                    this.delete_thread(id.clone(), window, cx);
                }
                cx.notify();
            },
        )
        .detach();
        self.shortcut_state.confirmation = Some(confirmation);
        cx.notify();
    }

    fn delete_thread(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = bridge
                .request::<_, Value>("thread/delete", json!({"threadId": id}))
                .await;
            this.update_in(cx, |this, window, cx| {
                if result.is_err() {
                    this.toast(Toast::danger("Couldn't delete the task. Try again."), cx);
                    return;
                }
                if this.state.route == Route::Thread(id.clone()) {
                    this.new_chat(window, cx);
                }
                storage::thread_deleted(this, &id, cx);
                this.toast(Toast::success("Task deleted"), cx);
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }
}

/// The original's `fe`: `# {title}`, then each turn's user text, its work as a collapsed
/// `N previous messages` quote, and its final answer.
/// shortcut: work items render one by one (no exploration grouping), paths stay absolute and
/// there is no length budget; port the rest of `conversation-markdown` if a copy needs it.
fn markdown(thread: &Thread) -> String {
    let title = thread.name.as_deref().unwrap_or(&thread.preview);
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if title.is_empty() {
        "Codex conversation".to_owned()
    } else {
        title.replace('#', "\\#")
    };
    let mut parts = vec![format!("# {title}")];
    for turn in &thread.turns {
        let finished = turn.status != TurnStatus::InProgress;
        let answer = finished
            .then(|| {
                turn.items
                    .iter()
                    .rposition(|item| matches!(item, ThreadItem::AgentMessage { .. }))
            })
            .flatten();
        let mut blocks = Vec::new();
        let mut work = Vec::new();
        for (index, item) in turn.items.iter().enumerate() {
            if let ThreadItem::UserMessage { content, .. } = item {
                let text: Vec<&str> = content
                    .iter()
                    .filter_map(|input| match input {
                        UserInput::Text { text } => Some(text.as_str()),
                        UserInput::Other => None,
                    })
                    .collect();
                push(&mut blocks, escape_details(text.join("\n").trim()));
            } else if Some(index) != answer {
                push(&mut work, item_markdown(item, finished));
            }
        }
        if !work.is_empty() {
            let noun = if work.len() == 1 {
                "message"
            } else {
                "messages"
            };
            let quoted = work
                .join("\n\n")
                .trim()
                .lines()
                .map(|line| {
                    if line.is_empty() {
                        ">".to_owned()
                    } else {
                        format!("> {line}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(format!(
                "<details><summary>{} previous {noun}</summary>\n\n{quoted}\n</details>",
                work.len()
            ));
        }
        if let Some(ThreadItem::AgentMessage { text, .. }) = answer.map(|i| &turn.items[i]) {
            push(&mut blocks, escape_details(text.trim()));
        }
        if !blocks.is_empty() {
            parts.push(blocks.join("\n\n"));
        }
    }
    format!("{}\n", parts.join("\n\n").trim_end())
}

/// The original's `D` for the item kinds this app server sends.
fn item_markdown(item: &ThreadItem, finished: bool) -> String {
    match item {
        ThreadItem::AgentMessage { text, .. } => escape_details(text.trim()),
        ThreadItem::Plan { text, .. } => {
            let text = escape_details(text).trim().to_owned();
            if text.is_empty() {
                text
            } else {
                format!("Plan\n\n{text}")
            }
        }
        ThreadItem::CommandExecution {
            command,
            status,
            aggregated_output,
            exit_code,
            ..
        } if finished => {
            let mut body = vec![fence("bash", &format!("$ {command}"))];
            if let Some(output) = aggregated_output
                .as_deref()
                .filter(|o| !o.trim().is_empty())
            {
                body.push(fence("text", output));
            }
            body.push(match (status.as_str(), exit_code) {
                (_, Some(0)) => "Success".to_owned(),
                (_, Some(code)) => format!("Failed with exit code {code}"),
                ("completed", None) => "Success".to_owned(),
                ("interrupted", None) => "Stopped".to_owned(),
                _ => "Running".to_owned(),
            });
            details(&format!("Ran {}", code(command)), &body.join("\n\n"))
        }
        ThreadItem::FileChange { changes, .. } => changes
            .iter()
            .filter(|change| !change.diff.trim().is_empty())
            .map(|change| {
                let verb = match change.kind["type"].as_str() {
                    Some("add") => "Created",
                    Some("delete") => "Deleted",
                    _ => "Edited",
                };
                let (mut added, mut removed) = (0, 0);
                for line in change.diff.lines() {
                    if line.starts_with('+') && !line.starts_with("+++") {
                        added += 1;
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        removed += 1;
                    }
                }
                details(
                    &format!("{verb} {} +{added} -{removed}", code(&change.path)),
                    &fence("diff", &change.diff),
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ThreadItem::McpToolCall {
            server,
            tool,
            arguments,
            ..
        } => format!(
            "MCP tool call\n\n{}\n\n{}",
            escape_details(&format!("{server}.{tool}")),
            fence(
                "json",
                &serde_json::to_string_pretty(arguments).unwrap_or_default()
            )
        ),
        _ => String::new(),
    }
}

fn push(blocks: &mut Vec<String>, block: String) {
    if !block.trim().is_empty() {
        blocks.push(block);
    }
}

fn details(summary: &str, body: &str) -> String {
    format!(
        "<details><summary>{summary}</summary>\n\n{}\n\n</details>",
        body.trim()
    )
}

/// A fence one backtick longer than the longest run inside (at least three).
fn fence(lang: &str, text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let ticks = "`".repeat((longest + 1).max(3));
    format!("{ticks}{lang}\n{}\n{ticks}", text.trim_end())
}

fn code(text: &str) -> String {
    format!("<code>{}</code>", html_escape(text))
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `<details>` tags inside a message would break the export's own blocks.
fn escape_details(text: &str) -> String {
    let re = regex::Regex::new(r"(?i)</?details(?:\s[^>]*)?>").expect("details pattern");
    re.replace_all(&text.replace("\r\n", "\n"), |m: &regex::Captures| {
        html_escape(&m[0])
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_renders_turns_like_the_original() {
        let thread: Thread = serde_json::from_value(json!({
            "id": "t", "preview": "Fix #1", "name": null, "model": null, "reasoningEffort": null,
            "modelProvider": "openai", "cwd": "/w", "path": null, "createdAt": 0, "updatedAt": 0,
            "section": null, "status": {"type": "idle"}, "gitInfo": null,
            "turns": [{
                "id": "u", "status": "completed", "error": null, "startedAt": null,
                "completedAt": null, "durationMs": null,
                "items": [
                    {"type": "userMessage", "id": "1", "content": [{"type": "text", "text": "Hi <details>"}]},
                    {"type": "agentMessage", "id": "2", "text": "Looking"},
                    {"type": "commandExecution", "id": "3", "command": "ls", "cwd": "/w",
                     "status": "completed", "aggregatedOutput": "a\n", "exitCode": 0, "durationMs": 1},
                    {"type": "agentMessage", "id": "4", "text": "Done"}
                ]
            }]
        }))
        .expect("thread");
        assert_eq!(
            markdown(&thread),
            "# Fix \\#1\n\nHi &lt;details&gt;\n\n<details><summary>2 previous messages</summary>\n\n\
             > Looking\n>\n> <details><summary>Ran <code>ls</code></summary>\n>\n> ```bash\n> $ ls\n> ```\n>\n\
             > ```text\n> a\n> ```\n>\n> Success\n>\n> </details>\n</details>\n\nDone\n"
        );
    }
}
