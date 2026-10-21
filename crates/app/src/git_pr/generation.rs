use crate::{Generation, GitRun, Root};
use gpui::{AsyncWindowContext, WeakEntity};
use serde_json::{Value, json};
use std::{sync::LazyLock, time::Duration};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Commit,
    PullRequest,
    CommitPullRequest,
}

impl Kind {
    fn source(self) -> &'static str {
        match self {
            Self::Commit => "commit_message",
            Self::PullRequest => "pull_request_message",
            Self::CommitPullRequest => "commit_pull_request_message",
        }
    }

    fn limits(self) -> (usize, Duration) {
        match self {
            Self::Commit => (20_000, Duration::from_secs(30)),
            Self::PullRequest => (30_000, Duration::from_secs(45)),
            Self::CommitPullRequest => (40_000, Duration::from_secs(45)),
        }
    }

    fn fields(self) -> &'static [(&'static str, usize, usize)] {
        match self {
            Self::Commit => &[("message", 8, 4000)],
            Self::PullRequest => &[("title", 8, 120), ("body", 12, 30_000)],
            Self::CommitPullRequest => &[
                ("message", 8, 4000),
                ("title", 8, 120),
                ("body", 12, 30_000),
            ],
        }
    }

    fn schema(self) -> &'static Value {
        static SCHEMAS: LazyLock<[Value; 3]> = LazyLock::new(|| {
            [Kind::Commit, Kind::PullRequest, Kind::CommitPullRequest].map(|kind| {
                let mut properties = serde_json::Map::new();
                for &(field, min, max) in kind.fields() {
                    let mut property = json!({"type": "string", "minLength": min, "maxLength": max});
                    if field == "message" {
                        property["pattern"] = json!("^[^\\r\\n]{1,72}(?:\\r?\\n[\\s\\S]*)?$");
                    }
                    properties.insert(field.to_owned(), property);
                }
                json!({"type": "object", "properties": properties, "required": kind.fields().iter().map(|(name, _, _)| name).collect::<Vec<_>>(), "additionalProperties": false})
            })
        });
        &SCHEMAS[match self {
            Self::Commit => 0,
            Self::PullRequest => 1,
            Self::CommitPullRequest => 2,
        }]
    }

    fn parse(self, raw: Option<&str>) -> Option<Value> {
        let value: Value = serde_json::from_str(raw?.trim()).ok()?;
        let mut parsed = serde_json::Map::new();
        for &(field, min, max) in self.fields() {
            let text = value[field].as_str()?;
            if !(min..=max).contains(&text.encode_utf16().count()) {
                return None;
            }
            if field == "message" {
                let subject = text
                    .split_once('\n')
                    .map_or(text, |(first, _)| first.strip_suffix('\r').unwrap_or(first));
                if !(1..=72).contains(&subject.encode_utf16().count()) || subject.contains('\r') {
                    return None;
                }
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            parsed.insert(field.to_owned(), json!(trimmed));
        }
        Some(Value::Object(parsed))
    }

    fn instructions(self, context: &str) -> String {
        match self {
            Self::Commit => crate::generation_instructions(context),
            Self::PullRequest => [
                "You are a helpful assistant. Generate a pull request title and body.",
                "Write the result into the structured response fields title and body.",
                "Make 0 tool calls.",
                "If context includes pull request instructions, follow them even when they conflict with the default rules below.",
                "Language rules:",
                "- Match the primary language of the supplied context; default to English.",
                "- Translate standard section headings such as Summary and Testing when writing in another language.",
                "Fallback PR title rules:",
                "- title must contain only the PR title, not JSON, field labels, or body content.",
                "- Use an imperative or action-oriented phrasing first.",
                "- Keep the title under 120 characters.",
                "- No trailing punctuation.",
                "Body rules:",
                "- body must contain only the PR body, not JSON, field labels, the title, or a full PR draft.",
                "- Do not repeat, restate, or label the title inside body.",
                "- Keep the body concise and scannable.",
                "- Keep the body under 30000 characters.",
                "- Use Markdown with short bullets.",
                "- Include a Summary section and a Testing section.",
                "- In Testing, describe meaningful validation at a high level, such as new unit or integration tests, or local UI testing with Playwright.",
                "- Do not paste command transcripts. For routine checks, summarize the result, for example: lint and formatting passed.",
                "- If tests were not run, say \"Not run (not requested)\".",
                "- If context includes pull request instructions, apply them to the title/body content only.",
                "", "Context:", context,
            ].join("\n"),
            Self::CommitPullRequest => [
                "Using the supplied commit and pull request contexts below, generate one git commit message plus one pull request title and body.",
                "Write the result into the structured response fields message, title, and body.",
                "message must contain plain commit-message text only, not JSON, field labels, or code fences.",
                "Make 0 tool calls.",
                "If context includes pull request instructions, follow them even when they conflict with the default pull request rules below.",
                "Custom commit instructions for message content and formatting apply to message only and override the fallback commit message rules below.",
                "Commit message bounds:",
                "- Keep the complete message under 4000 characters.",
                "- Keep the subject under 72 characters.",
                "Fallback commit message rules:",
                "- Generate a concise single-line subject.",
                "- Use an imperative verb first.",
                "- Do not add a scope prefix unless the context already clearly uses one.",
                "- Do not include markdown, quotes, or trailing punctuation.",
                "Pull request language rules:",
                "- Match the primary language of the supplied context; default to English.",
                "- Translate standard section headings such as Summary and Testing when writing in another language.",
                "Fallback PR title rules:",
                "- title must contain only the PR title, not JSON, field labels, or body content.",
                "- Use an imperative or action-oriented phrasing first.",
                "- Keep title under 120 characters.",
                "- No trailing punctuation.",
                "Pull request body rules:",
                "- body must contain only the PR body, not JSON, field labels, the title, or a full PR draft.",
                "- Do not repeat, restate, or label the title inside body.",
                "- Keep the body concise and scannable.",
                "- Keep the body under 30000 characters.",
                "- Use Markdown with short bullets.",
                "- Include a Summary section and a Testing section.",
                "- In Testing, describe meaningful validation at a high level, such as new unit or integration tests, or local UI testing with Playwright.",
                "- Do not paste command transcripts. For routine checks, summarize the result, for example: lint and formatting passed.",
                "- If tests were not run, say \"Not run (not requested)\".",
                "- If context includes pull request instructions, apply them to title/body only.",
                "", "Commit and pull request context:", context,
            ].join("\n"),
        }
    }
}

pub(crate) async fn generate(
    this: &WeakEntity<Root>,
    cx: &mut AsyncWindowContext,
    run: &GitRun,
    kind: Kind,
    context: &str,
) -> Result<Option<Value>, String> {
    let context = context.trim();
    if context.is_empty() {
        return Ok(None);
    }
    let (limit, duration) = kind.limits();
    let mut units = 0;
    let prompt = context
        .chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= limit
        })
        .collect::<String>();
    let started: Value = run.bridge.request("thread/start", json!({
        "model": run.model, "modelProvider": null, "allowProviderModelFallback": true,
        "cwd": run.cwd, "approvalPolicy": "never", "permissions": ":read-only", "runtimeWorkspaceRoots": [],
        "config": {"features.enable_fanout": false, "features.multi_agent": false, "features.multi_agent_v2": false, "web_search": "disabled", "mcp_servers.codex_app": {"enabled": false, "command": ""}, "model_reasoning_effort": "low"},
        "personality": null, "ephemeral": true, "threadSource": kind.source(),
        "experimentalRawEvents": false, "dynamicTools": null, "serviceTier": null,
    })).await.map_err(|error| format!("{error:#}"))?;
    let thread = started["thread"]["id"]
        .as_str()
        .ok_or("the app-server started no thread")?
        .to_owned();
    let (done, answer) = async_channel::bounded(1);
    let registration = this.update(cx, |this, _| {
        this.generation = Some(Generation {
            thread: thread.clone(),
            text: None,
            done: done.clone(),
        });
    });
    if let Err(error) = registration {
        run.bridge
            .request::<_, Value>("thread/unsubscribe", json!({"threadId": thread}))
            .await
            .ok();
        return Err(error.to_string());
    }
    let executor = cx.background_executor().clone();
    let timeout = executor.spawn({
        let (executor, done) = (executor.clone(), done.clone());
        async move {
            executor.timer(duration).await;
            done.try_send(Err("Timed out waiting for structured result.".into()))
                .ok();
        }
    });
    let turn = run.bridge.request::<_, Value>("turn/start", json!({
        "threadId": thread,
        "input": [{"type": "text", "text": kind.instructions(&prompt), "text_elements": []}],
        "outputSchema": kind.schema(),
    })).await;
    let result = match turn {
        Ok(_) => answer
            .recv()
            .await
            .unwrap_or_else(|_| Err("the app-server closed".into())),
        Err(error) => Err(format!("{error:#}")),
    };
    drop(timeout);
    this.update(cx, |this, _| this.generation = None).ok();
    run.bridge
        .request::<_, Value>("thread/unsubscribe", json!({"threadId": thread}))
        .await
        .ok();
    Ok(kind.parse(result?.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_messages_reject_empty_or_out_of_bounds_fields() {
        let valid = json!({"message": "Change behavior", "title": "Change behavior", "body": "## Summary\nFix the behavior."});
        assert!(
            Kind::CommitPullRequest
                .parse(Some(&valid.to_string()))
                .is_some()
        );
        for field in ["message", "title", "body"] {
            let mut invalid = valid.clone();
            invalid[field] = json!(" ");
            assert!(
                Kind::CommitPullRequest
                    .parse(Some(&invalid.to_string()))
                    .is_none()
            );
        }
        let mut invalid = valid;
        invalid["message"] = json!("x".repeat(73));
        assert!(
            Kind::CommitPullRequest
                .parse(Some(&invalid.to_string()))
                .is_none()
        );
    }
}
