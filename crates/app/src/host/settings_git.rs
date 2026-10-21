use super::*;

const PR_WATCH_MARKER: &str =
    "This is a heartbeat turn. Continue automatically fixing this pull request.";

impl LocalHost {
    pub(super) fn worktree_roots(&self) -> Result<Vec<PathBuf>> {
        let settings = crate::host_settings::Settings::load(self.host_settings_path.clone())?;
        let custom = settings
            .read_all()
            .get("git-worktree-root")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let mut roots = Vec::new();
        if let Some(custom) = custom {
            let path = PathBuf::from(custom);
            ensure!(path.is_absolute(), "Worktree root must be absolute");
            roots.push(path);
        }
        let legacy = self.home.join("worktrees");
        if !roots.contains(&legacy) {
            roots.push(legacy);
        }
        Ok(roots)
    }

    pub(super) fn refresh_worktree_upstream(&self, cwd: &Path) -> Result<()> {
        let settings = crate::host_settings::Settings::load(self.host_settings_path.clone())?;
        if settings
            .read_all()
            .get("worktree-upstream-refresh-mode")
            .and_then(Value::as_str)
            == Some("best-effort")
            && let Err(error) = git::refresh_upstream(cwd)
        {
            eprintln!("codex-gpui: upstream refresh: {error:#}");
        }
        Ok(())
    }

    pub(super) fn managed_worktree(
        &self,
        cwd: &Path,
        start: Option<&str>,
        environment: Option<&Path>,
    ) -> Result<Option<git::Worktree>> {
        self.refresh_worktree_upstream(cwd)?;
        let key = format!("local:{}", cwd.to_string_lossy().trim_end_matches('/'));
        let state_path = self.home.join(".codex-global-state.json");
        let state: Value = match fs::read(state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(error.into()),
        };
        let selected = state["electron-persisted-atom-state"]["local-env-selections-by-workspace"]
            [&key]
            .as_str()
            .filter(|path| *path != "__none__")
            .map(Path::new);
        git::create_worktree(
            &self.worktree_roots()?[0],
            cwd,
            start,
            environment.or(selected),
        )
    }

    pub(super) fn select_environment(&self, root: &str, path: &str) -> Result<Value> {
        if git::select_worktree_environment(Path::new(root), path)? {
            return Ok(json!({"selected": true}));
        }
        let key = format!("local:{}", root.trim_end_matches('/'));
        self.update_global_state(|state| {
            if state["electron-persisted-atom-state"].is_null() {
                state["electron-persisted-atom-state"] = json!({});
            }
            let atoms = &mut state["electron-persisted-atom-state"];
            if atoms["local-env-selections-by-workspace"].is_null() {
                atoms["local-env-selections-by-workspace"] = json!({});
            }
            atoms["local-env-selections-by-workspace"][&key] = json!(path);
            Ok(true)
        })?;
        Ok(json!({"selected": true}))
    }

    pub(super) fn worktrees(&self) -> Result<Value> {
        let mut result = git::managed_worktrees(&self.worktree_roots()?)?;
        result["defaultRoot"] = json!(self.home.join("worktrees"));
        Ok(result)
    }

    pub(super) fn delete_worktree(&self, path: &str) -> Result<Value> {
        git::remove_managed_worktree(Path::new(path), &self.worktree_roots()?)?;
        Ok(json!({"success": true}))
    }

    pub(super) fn update_pr_watchers(&self, preferences: &Value) -> Result<Value> {
        let instructions = preferences["instructions"]
            .as_str()
            .context("Missing PR watch instructions")?;
        let auto_merge = preferences["autoMerge"]
            .as_bool()
            .context("Missing PR auto-merge preference")?;
        let mut database = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let _transaction = database
            .as_mut()
            .map(|database| {
                database.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            })
            .transpose()?;
        let mut updated = 0;
        for item in self.automations()?["items"]
            .as_array()
            .context("Invalid automation list")?
        {
            if item["kind"] != "heartbeat" || item["status"] != "ACTIVE" {
                continue;
            }
            let Some(prompt) = item["prompt"].as_str() else {
                continue;
            };
            let Some((index, _)) = prompt.match_indices(PR_WATCH_MARKER).find(|(index, _)| {
                let end = index + PR_WATCH_MARKER.len();
                (*index == 0 || prompt.as_bytes()[index - 1] == b'\n')
                    && (end == prompt.len() || prompt.as_bytes()[end] == b'\n')
            }) else {
                continue;
            };
            let before = &prompt[..index + PR_WATCH_MARKER.len()];
            let replacement = format!("{before}\n{}", watch_instructions(auto_merge, instructions));
            if replacement == prompt {
                continue;
            }
            let id = item["id"].as_str().context("Missing automation id")?;
            let path = self
                .home
                .join("automations")
                .join(id)
                .join("automation.toml");
            let mut definition: toml::Table = toml::from_str(&fs::read_to_string(&path)?)?;
            definition.insert("prompt".into(), toml::Value::String(replacement));
            definition.insert(
                "updated_at".into(),
                toml::Value::Integer(chrono::Utc::now().timestamp_millis()),
            );
            self.write_automation(id, &toml::to_string(&definition)?)?;
            updated += 1;
        }
        Ok(json!({"updated": updated}))
    }
}

fn watch_instructions(auto_merge: bool, instructions: &str) -> String {
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
    let instructions = instructions.trim();
    if !instructions.is_empty() {
        lines.extend([
            "",
            "Custom user instructions (these override conflicting defaults):",
            instructions,
        ]);
    }
    lines.join("\n")
}
