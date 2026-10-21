use super::generation::{self, Kind};
use crate::*;
use ui::screens::git_pr::{CreatePullRequest, Event as FormEvent, Repository, Submission};

impl Root {
    pub fn pull_request_form(&self) -> Option<&Entity<CreatePullRequest>> {
        self.pull_request_form.as_ref()
    }

    pub(crate) fn update_pr_watch_preferences(&self, cx: &mut Context<Self>) {
        let Some(settings) = cx.try_global::<host_settings::Settings>() else {
            return;
        };
        let values = settings.read_all();
        let params = json!({
            "autoMerge": values.get("git-pr-watch-auto-merge").and_then(Value::as_bool).unwrap_or(false),
            "instructions": values.get("git-pr-watch-instructions").and_then(Value::as_str).unwrap_or_default(),
        });
        let host = self.host.clone();
        let task = cx.spawn(async move |_, cx| -> Result<()> {
            cx.background_executor()
                .spawn(
                    async move { git_call(&*host, "git-pr-watch-preferences", params).map(|_| ()) },
                )
                .await
        });
        log_err(task, cx);
    }

    pub(crate) fn open_pull_request(
        &mut self,
        draft: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_busy.is_some() {
            return;
        }
        let Some(view) = &self.thread else {
            return;
        };
        let thread = view.read(cx).id().to_owned();
        let cwd = view.read(cx).cwd().to_owned();
        let suggested = branch_name(
            &git_setting(cx, "git-branch-prefix").unwrap_or_else(|| "codex/".into()),
            &view.read(cx).header_title(),
        );
        let instructions = git_setting(cx, "git-pr-instructions").unwrap_or_default();
        let run = Arc::new(GitRun {
            host: self.host.clone(),
            bridge: self.bridge.clone(),
            model: generation_model(&self.models),
            cwd,
            instructions: git_setting(cx, "git-commit-instructions").unwrap_or_default(),
            attribution: self.commit_attribution.clone(),
        });
        let form = cx.new(|cx| CreatePullRequest::new(draft, cx));
        let workflow = run.clone();
        cx.subscribe_in(&form, window, move |this, form, event, window, cx| {
            if this.pull_request_form.as_ref() != Some(form) {
                return;
            }
            this.pull_request_form = None;
            window.focus(&this.composer(cx).focus_handle(cx), cx);
            match event {
                FormEvent::Dismiss => {}
                FormEvent::Open(url) => cx.open_url(url),
                FormEvent::Submit(submission) => this.create_pull_request(
                    workflow.clone(),
                    thread.clone(),
                    instructions.clone(),
                    submission.clone(),
                    window,
                    cx,
                ),
            }
            cx.notify();
        })
        .detach();
        window.focus(&form.read(cx).focus_handle(cx), cx);
        self.pull_request_form = Some(form.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = load_repository(&run, &suggested, cx).await;
            this.update_in(cx, |this, window, cx| {
                if this.pull_request_form.as_ref() != Some(&form) {
                    return;
                }
                form.update(cx, |form, cx| form.set_repository(result, cx));
                window.focus(&form.read(cx).focus_handle(cx), cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn create_pull_request(
        &mut self,
        run: Arc<GitRun>,
        thread: String,
        instructions: String,
        submission: Submission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_busy.is_some() {
            return;
        }
        self.set_git_phase(Some("Generating messages…"), cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = create(&this, cx, &run, &thread, &instructions, submission).await;
            this.update(cx, |this, cx| {
                this.set_git_phase(None, cx);
                match result {
                    Ok(created) => {
                        if !created.web {
                            let mut toast =
                                Toast::success(format!("Created PR for {}", created.branch));
                            if let Some(url) = created.url {
                                toast = toast.link("PR", url);
                            }
                            this.toast(toast, cx);
                        }
                        if created.attachment_failed {
                            this.toast(
                                Toast::danger(
                                    "Pull request created, but could not attach it to this task",
                                ),
                                cx,
                            );
                        }
                        if created.branch_failed {
                            this.toast(Toast::danger("Failed to set branch"), cx);
                        }
                    }
                    Err(failed) => this.finish_git_workflow(failed, &run.cwd, cx),
                }
                this.refresh_git_actions(cx);
            })
            .ok();
        })
        .detach();
    }
}

async fn load_repository(
    run: &GitRun,
    suggested: &str,
    cx: &AsyncWindowContext,
) -> Result<Repository, String> {
    let context = run
        .call(cx, "git-pr-context", json!({"cwd": run.cwd}))
        .await?;
    let branch = context["branch"].as_str().map(str::to_owned);
    let base = context["defaultBranch"].as_str().map(str::to_owned);
    let mut blocker = if base.is_none() {
        Some("Default branch information unavailable".to_owned())
    } else if branch.is_some() && branch == base {
        Some("Checkout a feature branch before creating a PR".to_owned())
    } else {
        None
    };
    let status = run
        .call(
            cx,
            "gh-cli-status",
            json!({"hostId": "local", "cwd": run.cwd}),
        )
        .await;
    let available = match status {
        Ok(status)
            if !status["isInstalled"].is_boolean() || !status["isAuthenticated"].is_boolean() =>
        {
            blocker = Some("Pull request creation unavailable".into());
            false
        }
        Ok(status) if status["isInstalled"] != true => {
            blocker = Some("Install GitHub CLI (gh) to create PRs".into());
            false
        }
        Ok(status) if status["isAuthenticated"] != true => {
            blocker = Some("Authenticate GitHub CLI: run `gh auth login`".into());
            false
        }
        Ok(_) => true,
        Err(_) => {
            blocker = Some("Pull request creation unavailable".into());
            false
        }
    };
    let mut existing_pr = None;
    if available && let Some(branch) = &branch {
        match run
            .call(
                cx,
                "gh-pr-local-state",
                json!({"cwd": run.cwd, "headBranch": branch}),
            )
            .await
        {
            Ok(pr) => {
                if pr["state"]
                    .as_str()
                    .is_some_and(|state| state.eq_ignore_ascii_case("OPEN"))
                {
                    existing_pr = pr["url"].as_str().map(str::to_owned);
                }
            }
            Err(_) => blocker = Some("Pull request creation unavailable".into()),
        }
    }
    Ok(Repository {
        name: PathBuf::from(context["root"].as_str().unwrap_or(&run.cwd))
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        new_branch: branch.is_none().then(|| suggested.to_owned()),
        branch,
        base,
        branches: context["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|branch| branch.as_str().map(str::to_owned))
            .collect(),
        additions: context["all"]["additions"].as_u64().unwrap_or_default() as usize,
        deletions: context["all"]["deletions"].as_u64().unwrap_or_default() as usize,
        existing_pr,
        blocker,
    })
}

struct Created {
    branch: String,
    url: Option<String>,
    web: bool,
    attachment_failed: bool,
    branch_failed: bool,
}

fn failure(title: &str, error: impl Into<String>) -> GitOutcome {
    GitOutcome::failed(title, Some(error.into()))
}

async fn phase(this: &WeakEntity<Root>, cx: &mut AsyncWindowContext, phase: &'static str) {
    this.update(cx, |this, cx| this.set_git_phase(Some(phase), cx))
        .ok();
}

async fn create(
    this: &WeakEntity<Root>,
    cx: &mut AsyncWindowContext,
    run: &GitRun,
    thread: &str,
    instructions: &str,
    submission: Submission,
) -> Result<Created, GitOutcome> {
    let context = run
        .call(cx, "git-pr-context", json!({"cwd": run.cwd}))
        .await
        .map_err(|error| failure("Failed to create pull request", error))?;
    let branch = submission
        .new_branch
        .as_deref()
        .or(context["branch"].as_str())
        .ok_or_else(|| {
            failure(
                "Failed to create pull request",
                "Branch information unavailable",
            )
        })?;
    let base = context["defaultBranch"].as_str().ok_or_else(|| {
        failure(
            "Failed to create pull request",
            "Default branch information unavailable",
        )
    })?;
    if branch == base {
        return Err(failure(
            "Failed to create pull request",
            "Checkout a feature branch before creating a PR",
        ));
    }
    let commit = submission.include_local_changes
        && context["all"]["files"].as_u64().unwrap_or_default() > 0;
    let push = commit
        || submission.new_branch.is_some()
        || context["commitsAhead"].as_u64().unwrap_or_default() > 0
        || context["upstream"].is_null();
    let pr_context = pull_request_context(&context, branch, base, instructions);
    let generated = if commit {
        let diff = run
            .call(
                cx,
                "git-commit-diff",
                json!({"cwd": run.cwd, "includeUnstaged": true}),
            )
            .await
            .map_err(|error| {
                failure("Failed to generate commit and pull request messages", error)
            })?;
        let prompt = format!(
            "Commit message context:\n{}\n\nPull request context:\n{pr_context}",
            generation_prompt(&run.instructions, diff["unifiedDiff"].as_str())
        );
        generation::generate(this, cx, run, Kind::CommitPullRequest, &prompt)
            .await
            .map_err(|error| failure("Failed to generate commit and pull request messages", error))?
            .ok_or_else(|| {
                failure(
                    "Couldn't generate commit and pull request messages",
                    "The app-server returned no structured result",
                )
            })?
    } else if submission.title.trim().is_empty() || submission.body.trim().is_empty() {
        generation::generate(this, cx, run, Kind::PullRequest, &pr_context)
            .await
            .map_err(|error| failure("Failed to generate pull request title and body", error))?
            .ok_or_else(|| {
                failure(
                    "Failed to generate pull request title and body",
                    "The app-server returned no structured result",
                )
            })?
    } else {
        json!({})
    };
    let title = if submission.title.trim().is_empty() {
        generated["title"].as_str().unwrap_or_default()
    } else {
        submission.title.trim()
    };
    let body = if submission.body.trim().is_empty() {
        generated["body"].as_str().unwrap_or_default()
    } else {
        submission.body.trim()
    };
    if let Some(branch) = &submission.new_branch {
        phase(this, cx, "Creating branch…").await;
        let result = run
            .call(
                cx,
                "git-checkout",
                json!({"cwd": run.cwd, "newBranch": branch}),
            )
            .await
            .map_err(|error| failure("Failed to create branch", error))?;
        git_result(&result)?;
    }
    if commit {
        phase(this, cx, "Committing…").await;
        let attribution = super::attribution(run, cx).await;
        let result = run.call(cx, "git-commit", json!({"cwd": run.cwd, "message": generated["message"], "includeUnstaged": true, "commitAttribution": attribution})).await.map_err(|error| failure("Failed to commit changes", error))?;
        if result["errorType"] != "nothing-to-commit" {
            git_result(&result)?;
        }
    }
    if push {
        phase(this, cx, "Pushing changes…").await;
        let result = run
            .call(cx, "git-push", json!({"cwd": run.cwd}))
            .await
            .map_err(|error| failure("Failed to push changes", error))?;
        git_result(&result)?;
    }
    phase(this, cx, "Creating PR…").await;
    let result = run.call(cx, "gh-pr-create", json!({"hostId": "local", "cwd": run.cwd, "headBranch": branch, "baseBranch": base, "isDraft": submission.draft, "openInBrowser": submission.open_in_browser, "titleOverride": title, "bodyOverride": body})).await.map_err(|error| failure("Failed to create pull request", error))?;
    if result["status"] != "success" {
        return Err(failure(
            "Failed to create pull request",
            result["error"]
                .as_str()
                .unwrap_or("GitHub returned an invalid result"),
        ));
    }
    let attachment_failed = if !submission.open_in_browser
        && let Some(url) = result["url"].as_str()
    {
        attach(
            run,
            thread,
            branch,
            context["root"].as_str().unwrap_or(&run.cwd),
            url,
        )
        .await
        .is_err()
    } else {
        false
    };
    let branch_failed = !submission.open_in_browser
        && run
            .bridge
            .request::<_, Value>(
                "thread/metadata/update",
                json!({"threadId": thread, "gitInfo": {"branch": branch}}),
            )
            .await
            .is_err();
    Ok(Created {
        branch: branch.to_owned(),
        url: result["url"].as_str().map(str::to_owned),
        web: submission.open_in_browser,
        attachment_failed,
        branch_failed,
    })
}

fn pull_request_context(context: &Value, head: &str, base: &str, instructions: &str) -> String {
    let mut parts = vec![format!("Branches:\n- Head: {head}\n- Base: {base}")];
    let instructions = instructions.trim();
    if !instructions.is_empty() {
        let suffix = if instructions.matches("```").count().is_multiple_of(2) {
            ""
        } else {
            "\n```"
        };
        parts.push(format!("Pull request instructions (apply these to the title/body content only):\n{instructions}{suffix}"));
    }
    let diff = context["branchDiff"].as_str().unwrap_or_default().trim();
    let changes = if diff.is_empty() || diff.lines().count() > 1000 {
        let files = context["branchFiles"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut lines = files
            .iter()
            .take(100)
            .filter_map(Value::as_str)
            .map(|path| format!("- {path}"))
            .collect::<Vec<_>>();
        if files.len() > 100 {
            lines.push(format!("…and {} more", files.len() - 100));
        }
        if lines.is_empty() {
            "- (no files listed)".to_owned()
        } else {
            lines.join("\n")
        }
    } else {
        diff.to_owned()
    };
    parts.push(format!("Changes:\n{changes}"));
    parts.join("\n\n")
}

async fn attach(run: &GitRun, thread: &str, branch: &str, root: &str, address: &str) -> Result<()> {
    let url = url::Url::parse(address)?;
    let parts = url
        .path_segments()
        .context("invalid pull request URL")?
        .collect::<Vec<_>>();
    ensure!(
        parts.len() == 4 && parts[2] == "pull",
        "invalid pull request URL"
    );
    let host = url
        .host_str()
        .context("pull request has no host")?
        .to_lowercase();
    let number = parts[3].parse::<u64>()?;
    let identity = json!([
        host,
        parts[0].to_lowercase(),
        parts[1].to_lowercase(),
        number
    ])
    .to_string();
    run.bridge.request::<_, Value>("thread/attachment/add", json!({"threadId": thread, "attachmentType": "pull_request", "identityKey": identity, "payload": {"url": address, "root": root, "headBranch": branch}})).await?;
    Ok(())
}
