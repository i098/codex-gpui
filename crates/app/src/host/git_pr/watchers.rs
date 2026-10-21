use crate::host::{LocalHost, device_id};
use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use std::fs;

const HEADER: &str = "## Pull request fix automation:";
const HEARTBEAT: &str =
    "This is a heartbeat turn. Continue automatically fixing this pull request.";

fn behavior(auto_merge: bool, custom: &str) -> String {
    let mut lines = vec![
        "The following behavior is the default. Explicit custom user instructions below override conflicting defaults, but never bypass safety, permissions, or access restrictions.",
        "Inspect the latest PR state with `gh`, including mergeability and all current checks.",
        "By default, fix only failing checks caused by this PR and merge conflicts with its base branch. Do not change code for unrelated failures, infrastructure outages, or flakes unless the custom user instructions explicitly authorize broader remediation.",
        "Start from logs and annotations before changing code. Use installed CI skills or tools for external providers such as Buildkite, and report the exact missing credential or access if blocked.",
        "Do not modify the configured checkout, switch its branch, clean it, reset it, or commit from it.",
        "Inspect from the configured checkout, then create an isolated git worktree only when a code change or conflict resolution is required. Make every mutation in that isolated worktree so the user's active checkout is not disturbed.",
        "Keep changes minimal and relevant to the authorized task. Run the narrowest useful verification, commit, and push only to the PR branch unless custom instructions explicitly authorize a separate fix PR.",
        if auto_merge {
            "Once all required checks pass and the PR is mergeable, merge it using the user's custom instructions or the repository's merge workflow. If merging fails, diagnose the failure, update the branch when needed, retry the merge workflow, and continue until the PR is merged or closed."
        } else {
            "Do not merge the pull request unless the custom user instructions explicitly request it; otherwise, the user controls merging separately."
        },
        "Re-check live GitHub state instead of trusting prior turns.",
        "If checks are still pending, finish the useful work for this turn without sleeping; the heartbeat will check again in 10 minutes.",
        if auto_merge {
            "When the PR is merged or closed, pause this heartbeat automation with the automation update tool before your final response. Do not stop merely because the checks pass."
        } else {
            "When the PR is green and conflict-free, merged, or closed, pause this heartbeat automation with the automation update tool before your final response, unless the custom user instructions explicitly require continuing until it merges."
        },
        "If progress requires user input or unavailable credentials, ask one concise question in this thread, report the exact blocker, and pause this heartbeat automation. The user can reply here and resume it when ready.",
        "Do not create or suggest another automation.",
    ];
    let custom = custom.trim();
    if !custom.is_empty() {
        lines.extend([
            "",
            "Custom user instructions (these override conflicting defaults):",
            custom,
        ]);
    }
    lines.join("\n")
}

fn metadata(pr: &Value) -> Result<String> {
    let number = pr["number"]
        .as_u64()
        .filter(|number| *number > 0)
        .context("missing pull request number")?;
    let url = pr["url"].as_str().context("missing pull request URL")?;
    let head = pr["headBranch"].as_str().context("missing head branch")?;
    let base = pr["baseBranch"].as_str().context("missing base branch")?;
    let repository = pr["repo"].as_str().unwrap_or("the current repository");
    Ok(format!(
        "Repository: {repository}\nPull request: #{number}\nPull request URL: {url}\nBranch: {head} -> {base}"
    ))
}

fn prompt(pr: &Value, auto_merge: bool, custom: &str) -> Result<String> {
    Ok(format!(
        "{HEADER}\n{}\n\n{HEARTBEAT}\n{}",
        metadata(pr)?,
        behavior(auto_merge, custom)
    ))
}

fn matches(prompt: &str, url: &str) -> bool {
    let mut lines = prompt.lines().skip_while(|line| *line != HEADER);
    lines.next();
    let expected = format!("Pull request URL: {url}");
    lines
        .take_while(|line| !line.is_empty())
        .any(|line| line == expected)
}

impl LocalHost {
    pub(in crate::host) fn start_pr_watcher(&self, params: &Value) -> Result<Value> {
        self.database(rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?
            .context("automation database is unavailable")?;
        let bridge = self
            .bridge
            .as_ref()
            .context("The app server is not connected")?;
        let pr = &params["pullRequest"];
        let auto_merge = params["autoMerge"] == true;
        let custom = params["instructions"].as_str().unwrap_or_default();
        let next_prompt = prompt(pr, auto_merge, custom)?;
        let url = pr["url"].as_str().context("missing pull request URL")?;
        let items = self.automations()?;
        let existing = items["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| {
                item["kind"] == "heartbeat"
                    && item["status"] != "DELETED"
                    && item["prompt"]
                        .as_str()
                        .is_some_and(|prompt| matches(prompt, url))
            });
        let new = existing.is_none();
        let id = existing
            .and_then(|item| item["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(device_id);
        let target = if let Some(target) = existing.and_then(|item| item["targetThreadId"].as_str())
        {
            target.to_owned()
        } else {
            let parent = params["parentThreadId"]
                .as_str()
                .context("Automatic fix requires a parent conversation")?;
            let fork = pollster::block_on(bridge.request::<_, Value>(
                "thread/fork",
                json!({
                    "threadId": parent, "threadSource": "pull_request_fix_automation"
                }),
            ))?;
            fork["thread"]["id"]
                .as_str()
                .context("Automatic fix child thread could not be created")?
                .to_owned()
        };
        pollster::block_on(bridge.request::<_, Value>(
            "thread/read",
            json!({"threadId": target, "includeTurns": false}),
        ))?;
        let setup_error = params["setupError"].as_str();
        let status = if setup_error.is_some() {
            "PAUSED"
        } else {
            "ACTIVE"
        };
        let unchanged =
            existing.is_some_and(|item| item["status"] == status && item["prompt"] == next_prompt);
        let item = if unchanged {
            existing.unwrap().clone()
        } else {
            let path = self
                .home
                .join("automations")
                .join(&id)
                .join("automation.toml");
            let previous = if new {
                None
            } else {
                Some(fs::read_to_string(&path)?)
            };
            let now = chrono::Utc::now().timestamp_millis();
            let mut table: toml::Table = if let Some(previous) = &previous {
                toml::from_str(previous)?
            } else {
                let number = pr["number"]
                    .as_u64()
                    .context("missing pull request number")?;
                toml::Value::try_from(json!({
                    "version": 1, "id": id, "name": format!("Auto-fix PR #{number}"),
                    "kind": "heartbeat", "target_thread_id": target,
                    "rrule": "RRULE:FREQ=MINUTELY;INTERVAL=10", "created_at": now,
                }))?
                .as_table()
                .cloned()
                .context("invalid heartbeat definition")?
            };
            table.insert("prompt".into(), next_prompt.clone().into());
            table.insert("updated_at".into(), now.into());
            table.insert("status".into(), status.into());
            self.write_automation(&id, &toml::to_string(&table)?)?;
            match self.update_automation(&json!({"id": id, "status": status})) {
                Ok(mut result) => result["item"].take(),
                Err(error) => {
                    if let Some(previous) = previous {
                        self.write_automation(&id, &previous)?;
                    } else {
                        let _ = fs::remove_file(&path);
                        let _ = fs::remove_dir(path.parent().unwrap());
                    }
                    return Err(error);
                }
            }
        };
        if let Some(error) = setup_error {
            return Ok(json!({"automation": item, "runError": error}));
        }
        let text = if new {
            format!(
                "Begin automatically fixing PR #{}.\nA 10-minute heartbeat is attached to this thread and will continue checking the pull request until the task is complete or blocked.\n\n{}\n{}",
                pr["number"],
                metadata(pr)?,
                behavior(auto_merge, custom)
            )
        } else {
            next_prompt
        };
        let run = pollster::block_on(async {
            bridge
                .request::<_, Value>("thread/resume", json!({"threadId": target}))
                .await?;
            bridge
                .request::<_, Value>(
                    "turn/start",
                    json!({
                        "threadId": target, "turnTrigger": "pull_request_fix_setup",
                        "input": [{"type": "text", "text": text, "text_elements": []}]
                    }),
                )
                .await
        });
        Ok(json!({"automation": item, "runError": run.err().map(|error| error.to_string())}))
    }

    pub(in crate::host) fn refresh_pr_watchers(&self, params: &Value) -> Result<Value> {
        let auto_merge = params["autoMerge"] == true;
        let custom = params["instructions"].as_str().unwrap_or_default();
        let automations = self.automations()?;
        let mut updated = 0;
        for item in automations["items"].as_array().into_iter().flatten() {
            if item["kind"] != "heartbeat" || item["status"] != "ACTIVE" {
                continue;
            }
            let Some(prompt) = item["prompt"].as_str() else {
                continue;
            };
            let lines = prompt.lines().collect::<Vec<_>>();
            let Some(at) = lines.iter().position(|line| *line == HEARTBEAT) else {
                continue;
            };
            let next = format!(
                "{}\n{}",
                lines[..=at].join("\n"),
                behavior(auto_merge, custom)
            );
            if next == prompt {
                continue;
            }
            let id = item["id"].as_str().context("missing automation id")?;
            let path = self
                .home
                .join("automations")
                .join(id)
                .join("automation.toml");
            let previous = fs::read_to_string(&path)?;
            let mut definition: toml::Table = toml::from_str(&previous)?;
            definition.insert("prompt".into(), next.into());
            definition.insert(
                "updated_at".into(),
                chrono::Utc::now().timestamp_millis().into(),
            );
            self.write_automation(id, &toml::to_string(&definition)?)?;
            if let Err(error) = self.update_automation(&json!({"id": id, "status": "ACTIVE"})) {
                self.write_automation(id, &previous)?;
                return Err(error);
            }
            updated += 1;
        }
        Ok(json!({"updated": updated}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_identity_uses_only_the_metadata_section() {
        let target = "https://github.com/example/repo/pull/42";
        let other = "https://github.com/example/repo/pull/4";
        let metadata = format!("{HEADER}\nPull request URL: {target}\n\nPull request URL: {other}");
        assert!(matches(&metadata, target));
        assert!(!matches(&metadata, other));
        assert!(!matches(&format!("Pull request URL: {target}"), target));
    }
}
