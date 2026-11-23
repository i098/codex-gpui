use super::device_id;
use anyhow::{Context as _, Result, ensure};
use bridge::{Bridge, Event};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    future::Future,
    path::Path,
    pin::pin,
    sync::Arc,
    task::{Context as TaskContext, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};

/// Buffered process output. A set capture-limit flag means the corresponding stream is incomplete.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_cap_reached: bool,
    pub stderr_cap_reached: bool,
}

/// Executes trusted UI commands through the app-server, without a Codex sandbox.
/// Never pass model-provided commands. Spawn and connection errors remain errors.
/// Callers must check the exit code and capture-limit flags before using the output.
pub fn execute(
    bridge: &Arc<Bridge>,
    cwd: &Path,
    command: &[String],
    timeout_ms: u64,
) -> Result<Output> {
    ensure!(cwd.is_absolute(), "process cwd must be absolute");
    ensure!(!command.is_empty(), "process command is empty");
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let events = bridge.subscribe();
    let handle = format!("git-pr:{}", device_id());
    let spawned = wait_until(
        bridge.request::<_, Value>(
            "process/spawn",
            json!({
                "processHandle": handle,
                "command": command,
                "cwd": cwd,
                "env": {"GIT_TERMINAL_PROMPT": "0", "GH_PROMPT_DISABLED": "1"},
                "streamStdin": false,
                "streamStdoutStderr": false,
                "timeoutMs": timeout_ms,
            }),
        ),
        deadline,
    );
    match spawned {
        Some(result) => {
            result?;
        }
        None => {
            cancel(bridge, &handle);
            anyhow::bail!("Timed out waiting for app-server process start");
        }
    }
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            cancel(bridge, &handle);
            anyhow::bail!("Timed out waiting for app-server process exit");
        }
        let event = events.recv_timeout(remaining);
        match event {
            Ok(Event::Notification { method, params })
                if method == "process/exited" && params["processHandle"] == handle =>
            {
                return serde_json::from_value(params).context("invalid process exit response");
            }
            Ok(_) => {}
            Err(error) => {
                cancel(bridge, &handle);
                return Err(error).context("App-server closed or process command timed out");
            }
        }
    }
}

pub(super) fn repository_command(
    host: &super::LocalHost,
    cwd: &Path,
    args: &[String],
) -> Result<String> {
    let bridge = host
        .bridge
        .as_ref()
        .context("Repository commands require an app-server connection")?;
    let output = execute(bridge, cwd, args, 120_000)?;
    ensure!(
        !output.stdout_cap_reached && !output.stderr_cap_reached,
        "Repository command output is incomplete"
    );
    ensure!(output.exit_code == 0, "{}", output.stderr.trim());
    Ok(output.stdout.trim().to_owned())
}

fn gitlab_project(remote: &str) -> Option<String> {
    let url = if remote.contains("://") {
        url::Url::parse(remote).ok()?
    } else {
        let (host, path) = remote.split_once(':')?;
        url::Url::parse(&format!("ssh://{host}/{path}")).ok()?
    };
    if !matches!(url.scheme(), "http" | "https" | "ssh")
        || !url.host_str()?.eq_ignore_ascii_case("gitlab.com")
    {
        return None;
    }
    let path = url.path().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if !path.contains('/')
        || !path.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        })
    {
        return None;
    }
    Some(format!("https://gitlab.com/{path}"))
}

fn gitlab_repository(cwd: &Path, status: &Value) -> Option<String> {
    let remote = status["remote"].as_str()?;
    let output = super::git::command(cwd, &["remote", "get-url", "--", remote])
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    gitlab_project(str::from_utf8(&output.stdout).ok()?.trim())
}

fn gitlab_create_args(
    cwd: &Path,
    repository: &str,
    branch: &str,
    params: &Value,
) -> Result<Vec<String>> {
    let nonempty = |field: &str| {
        params[field]
            .as_str()
            .filter(|value| !value.trim().is_empty())
    };
    let (title, body) = (nonempty("title"), nonempty("body"));
    let commit = if title.is_none() || body.is_none() {
        let output = super::git::command(cwd, &["log", "-1", "--format=%s%x00%b"])
            .output()
            .context("Read the latest Git commit")?;
        ensure!(output.status.success(), "Cannot read the latest Git commit");
        String::from_utf8(output.stdout).context("Git commit text is not UTF-8")?
    } else {
        String::new()
    };
    let (subject, message) = commit.split_once('\0').unwrap_or_default();
    let description = body.unwrap_or(message.trim_end());
    // A bare dash tells glab to open an editor.
    let description = if description == "-" {
        "-\n"
    } else {
        description
    };
    let mut args = vec![
        "glab".into(),
        "mr".into(),
        "create".into(),
        "--yes".into(),
        "--push=false".into(),
        format!("--repo={repository}"),
        format!("--source-branch={branch}"),
        format!("--title={}", title.unwrap_or(subject)),
        format!("--description={description}"),
    ];
    if let Some(base) = nonempty("base") {
        args.push(format!("--target-branch={base}"));
    }
    if params["draft"] == true {
        args.push("--draft".into());
    }
    Ok(args)
}

fn gitlab_merge_request_url(repository: &str, output: &str) -> Result<Value> {
    let iid = output
        .strip_prefix(repository)
        .and_then(|path| path.strip_prefix("/-/merge_requests/"))
        .and_then(|iid| iid.parse::<u64>().ok())
        .filter(|iid| *iid > 0);
    ensure!(iid.is_some(), "GitLab did not return a merge request URL");
    Ok(json!({"url": output}))
}

fn existing_merge_request(
    host: &super::LocalHost,
    cwd: &Path,
    repository: &str,
    branch: &str,
) -> Result<Value> {
    let output = repository_command(
        host,
        cwd,
        &[
            "glab".into(),
            "mr".into(),
            "list".into(),
            format!("--repo={repository}"),
            format!("--source-branch={branch}"),
            "--output=json".into(),
        ],
    )?;
    let rows: Vec<Value> = serde_json::from_str(&output)?;
    let Some(row) = rows.into_iter().find(|row| {
        row["source_project_id"].as_u64().is_some()
            && row["source_project_id"] == row["target_project_id"]
    }) else {
        return Ok(Value::Null);
    };
    gitlab_merge_request_url(repository, row["web_url"].as_str().unwrap_or_default())
}

pub(super) fn summary(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let status = super::git::fetch("git-commit-status", params)?;
    if let Some(repository) = gitlab_repository(cwd, &status) {
        if params["creationProvider"] == true {
            let project = repository.trim_start_matches("https://gitlab.com/");
            let info = repository_command(
                host,
                cwd,
                &[
                    "glab".into(),
                    "api".into(),
                    "--hostname=gitlab.com".into(),
                    format!("projects/{}", project.replace('/', "%2F")),
                ],
            )
            .ok()
            .and_then(|output| serde_json::from_str::<Value>(&output).ok());
            let base = info
                .as_ref()
                .and_then(|info| info["default_branch"].as_str());
            return Ok(json!({"gitLab": true, "baseBranch": base}));
        }
        return Ok(json!({"gitLab": true}));
    }
    if params["creationProvider"] == true {
        return Ok(Value::Null);
    }
    github_summary(host, cwd, &status)
}

fn github_summary(host: &super::LocalHost, cwd: &Path, status: &Value) -> Result<Value> {
    let branch = status["branch"]
        .as_str()
        .context("Create a branch before opening a pull request")?;
    let output = repository_command(
        host,
        cwd,
        &[
            "gh".into(),
            "pr".into(),
            "list".into(),
            "--head".into(),
            branch.into(),
            "--state".into(),
            "open".into(),
            "--json".into(),
            "number,url,title,baseRefName,headRefName,isDraft".into(),
        ],
    )?;
    let rows: Vec<Value> = serde_json::from_str(&output)?;
    Ok(rows.into_iter().next().unwrap_or(Value::Null))
}

pub(super) fn create(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let status = super::git::fetch("git-commit-status", params)?;
    let branch = status["branch"]
        .as_str()
        .context("Create a branch before opening a pull request")?;
    let gitlab = gitlab_repository(cwd, &status);
    ensure!(
        gitlab.is_some() == (params["gitLab"] == true),
        "The Git remote changed. Close and reopen the form."
    );
    let existing = if let Some(repository) = &gitlab {
        existing_merge_request(host, cwd, repository, branch)?
    } else {
        github_summary(host, cwd, &status)?
    };
    if !existing.is_null() && params["commitChanges"] != true {
        return Ok(existing);
    }
    if status["all"]["files"].as_u64().unwrap_or_default() > 0 {
        ensure!(
            params["commitChanges"] == true,
            "Commit local changes first, or select Commit and push local changes"
        );
        let committed = super::git::fetch(
            "git-commit",
            &json!({
                "cwd": cwd, "includeUnstaged": true,
                "message": params["title"].as_str().filter(|title| !title.trim().is_empty()).unwrap_or("Update working changes"),
            }),
        )?;
        ensure!(committed["status"] == "success", "{}", committed["error"]);
    }
    let pushed = super::git::fetch("git-push", &json!({"cwd": cwd}))?;
    ensure!(pushed["status"] == "success", "{}", pushed["error"]);
    if !existing.is_null() {
        return Ok(existing);
    }
    if let Some(repository) = &gitlab {
        let args = gitlab_create_args(cwd, repository, branch, params)?;
        let output = repository_command(host, cwd, &args)?;
        return gitlab_merge_request_url(repository, &output);
    }
    let mut args = vec!["gh".into(), "pr".into(), "create".into(), "--fill".into()];
    for (field, flag) in [("title", "--title"), ("body", "--body"), ("base", "--base")] {
        if let Some(value) = params[field]
            .as_str()
            .filter(|value| !value.trim().is_empty())
        {
            args.extend([flag.into(), value.into()]);
        }
    }
    if params["draft"] == true {
        args.push("--draft".into());
    }
    let url = repository_command(host, cwd, &args)?;
    Ok(json!({"url": url}))
}

pub(super) fn compare(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let repo: Value = serde_json::from_str(&repository_command(
        host,
        cwd,
        &[
            "gh".into(),
            "repo".into(),
            "view".into(),
            "--json".into(),
            "url,defaultBranchRef".into(),
        ],
    )?)?;
    let status = super::git::fetch("git-commit-status", params)?;
    let head = status["branch"]
        .as_str()
        .context("Create a branch before comparing")?;
    let base = repo["defaultBranchRef"]["name"]
        .as_str()
        .context("Default branch is unavailable")?;
    let mut url = url::Url::parse(
        repo["url"]
            .as_str()
            .context("Repository URL is unavailable")?,
    )?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid repository URL"))?
        .push("compare")
        .push(&format!("{base}...{head}"));
    Ok(json!({"url": url.as_str()}))
}

pub(super) fn repair(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let details = repository_command(
        host,
        cwd,
        &[
            "gh".into(),
            "pr".into(),
            "view".into(),
            "--json".into(),
            "url,title,body,comments,reviews,statusCheckRollup,mergeable".into(),
        ],
    )?;
    Ok(serde_json::from_str(&details)?)
}

struct NotifyThread(Thread);

impl Wake for NotifyThread {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn wait_until<F: Future>(future: F, deadline: Instant) -> Option<F::Output> {
    let waker = Waker::from(Arc::new(NotifyThread(thread::current())));
    let mut context = TaskContext::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return Some(value);
        }
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
    }
}

fn cancel(bridge: &Bridge, handle: &str) {
    let request = bridge.request::<_, Value>("process/kill", json!({"processHandle": handle}));
    let mut request = pin!(request);
    let _ = request
        .as_mut()
        .poll(&mut TaskContext::from_waker(Waker::noop()));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn gitlab_project_uses_the_remote_host_and_excludes_credentials() {
        for remote in [
            "https://user:secret@gitlab.com/team/subgroup/project.git",
            "git@gitlab.com:team/subgroup/project.git",
            "ssh://git@gitlab.com/team/subgroup/project.git",
        ] {
            assert_eq!(
                gitlab_project(remote).as_deref(),
                Some("https://gitlab.com/team/subgroup/project")
            );
        }
        for remote in [
            "https://gitlab.com.example.org/team/project",
            "https://gitlab.com@example.org/team/project",
            "file:///gitlab.com/team/project",
            "git@gitlab.com:project",
        ] {
            assert!(gitlab_project(remote).is_none(), "{remote}");
        }
    }

    #[test]
    fn merge_request_form_values_do_not_override_cli_flags() -> Result<()> {
        let args = gitlab_create_args(
            Path::new("."),
            "https://gitlab.com/team/project",
            "feature",
            &json!({"title": "--draft", "body": "line one\nline two", "base": "main"}),
        )?;
        assert!(args.iter().any(|arg| arg == "--title=--draft"));
        assert!(
            args.iter()
                .any(|arg| arg == "--description=line one\nline two")
        );
        assert!(args.iter().any(|arg| arg == "--target-branch=main"));
        assert!(!args.iter().any(|arg| arg == "--fill" || arg == "--draft"));
        let (temp, _host) = pr_fixture("[]")?;
        let draft = gitlab_create_args(
            temp.path(),
            "https://gitlab.com/team/project",
            "feature",
            &json!({"draft": true}),
        )?;
        for flag in [
            "--draft",
            "--title=Initial",
            "--description=",
            "--push=false",
        ] {
            assert!(draft.iter().any(|arg| arg == flag));
        }
        Ok(())
    }

    fn pr_fixture(existing: &str) -> Result<(tempfile::TempDir, super::super::LocalHost)> {
        let temp = tempfile::tempdir()?;
        for args in [
            &["init", "-b", "feature"][..],
            &["config", "user.name", "Test"],
            &["config", "user.email", "test@example.com"],
        ] {
            ensure!(
                Command::new("git")
                    .current_dir(temp.path())
                    .args(args)
                    .output()?
                    .status
                    .success(),
                "Git setup failed"
            );
        }
        std::fs::write(temp.path().join("tracked"), "original\n")?;
        for args in [&["add", "."][..], &["commit", "-m", "Initial"]] {
            ensure!(
                Command::new("git")
                    .current_dir(temp.path())
                    .args(args)
                    .output()?
                    .status
                    .success(),
                "Git commit failed"
            );
        }
        const SERVER: &str = r#"
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if message.get("method") == "initialize":
        print(json.dumps({"id": message["id"], "result": {"userAgent":"pr-fixture","codexHome":"/tmp","platformOs":"linux"}}), flush=True)
    elif message.get("method") == "process/spawn":
        params = message["params"]
        command = params["command"]
        stdout = sys.argv[1]
        if command[:2] == ["glab", "api"]:
            assert command[2:] == ["--hostname=gitlab.com", "projects/team%2Fproject"], command
            stdout = json.dumps({"default_branch": "release/main"})
        elif command[:3] == ["glab", "mr", "list"]:
            stdout = "[]"
        elif command[:3] == ["glab", "mr", "create"]:
            assert "--draft" in command, command
            assert "--target-branch=release/main" in command, command
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
        print(json.dumps({"method":"process/exited","params":{"processHandle":params["processHandle"],"exitCode":0,"stdout":stdout,"stderr":"","stdoutCapReached":False,"stderrCapReached":False}}), flush=True)
"#;
        let mut command = Command::new("python3");
        command.args(["-u", "-c", SERVER, existing]);
        let (bridge, _) = Bridge::spawn(command)?;
        let bridge = Arc::new(bridge);
        pollster::block_on(bridge.initialize("codex_gpui", "0.1.0"))?;
        let host = super::super::LocalHost::new(temp.path()).with_bridge(bridge);
        Ok((temp, host))
    }

    #[test]
    fn creating_draft_merge_request_passes_draft_to_glab() -> Result<()> {
        let url = "https://gitlab.com/team/project/-/merge_requests/1";
        let (temp, host) = pr_fixture(url)?;
        let remote = tempfile::tempdir()?;
        let remote_path = remote.path().to_str().context("Invalid remote path")?;
        for args in [
            &["init", "--bare", remote_path][..],
            &[
                "remote",
                "add",
                "origin",
                "https://gitlab.com/team/project.git",
            ],
            &["remote", "set-url", "--push", "origin", remote_path],
        ] {
            let output = Command::new("git")
                .current_dir(temp.path())
                .args(args)
                .output()?;
            ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let target = summary(
            &host,
            &json!({"cwd": temp.path(), "creationProvider": true}),
        )?;
        assert_eq!(target["baseBranch"], "release/main");
        let result = create(
            &host,
            &json!({"cwd": temp.path(), "gitLab": true, "draft": true, "base": target["baseBranch"]}),
        )?;
        assert_eq!(result["url"], url);
        Ok(())
    }

    #[test]
    fn existing_pull_request_does_not_commit_local_changes() -> Result<()> {
        let (temp, host) = pr_fixture(r#"[{"url":"https://github.com/example/project/pull/1"}]"#)?;
        std::fs::write(temp.path().join("tracked"), "pending\n")?;
        let before = super::super::git::fetch("git-commit-status", &json!({"cwd": temp.path()}))?;
        let result = create(&host, &json!({"cwd": temp.path(), "title": "Update"}))?;
        assert_eq!(result["url"], "https://github.com/example/project/pull/1");
        let after = super::super::git::fetch("git-commit-status", &json!({"cwd": temp.path()}))?;
        assert_eq!(after["commitsAhead"], before["commitsAhead"]);
        assert_eq!(after["all"], before["all"]);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("tracked"))?,
            "pending\n"
        );
        Ok(())
    }
    #[test]
    fn existing_pull_request_pushes_changes_with_consent() -> Result<()> {
        let (temp, host) = pr_fixture(r#"[{"url":"https://github.com/example/project/pull/1"}]"#)?;
        let remote = tempfile::tempdir()?;
        ensure!(
            Command::new("git")
                .args(["init", "--bare"])
                .arg(remote.path())
                .output()?
                .status
                .success(),
            "Remote setup failed"
        );
        let remote_path = remote.path().to_str().context("Invalid remote path")?;
        for args in [
            &["remote", "add", "origin", remote_path][..],
            &["push", "-u", "origin", "feature"],
        ] {
            ensure!(
                Command::new("git")
                    .current_dir(temp.path())
                    .args(args)
                    .output()?
                    .status
                    .success(),
                "Remote setup failed"
            );
        }
        std::fs::write(temp.path().join("tracked"), "pending\n")?;
        let result = create(
            &host,
            &json!({"cwd": temp.path(), "commitChanges": true, "title": "Update"}),
        )?;
        assert_eq!(result["url"], "https://github.com/example/project/pull/1");
        let pushed = Command::new("git")
            .current_dir(remote.path())
            .args(["show", "feature:tracked"])
            .output()?;
        assert!(pushed.status.success());
        assert_eq!(pushed.stdout, b"pending\n");
        let status = super::super::git::fetch("git-commit-status", &json!({"cwd": temp.path()}))?;
        assert_eq!(status["all"]["files"], 0);
        Ok(())
    }
    #[test]
    fn compare_encodes_branch_names_as_one_path_segment() -> Result<()> {
        let (temp, host) = pr_fixture(
            r#"{"url":"https://github.com/example/project","defaultBranchRef":{"name":"release/v1"}}"#,
        )?;
        ensure!(
            Command::new("git")
                .current_dir(temp.path())
                .args(["checkout", "-b", "topic/one#two"])
                .output()?
                .status
                .success(),
            "Branch setup failed"
        );
        let result = compare(&host, &json!({"cwd": temp.path()}))?;
        assert_eq!(
            result["url"],
            "https://github.com/example/project/compare/release%2Fv1...topic%2Fone%23two"
        );
        Ok(())
    }

    #[test]
    fn creating_pull_request_requires_consent_to_commit_changes() -> Result<()> {
        let (temp, host) = pr_fixture("[]")?;
        std::fs::write(temp.path().join("tracked"), "pending\n")?;
        let error = create(&host, &json!({"cwd": temp.path(), "title": "Update"})).unwrap_err();
        assert!(error.to_string().contains("Commit local changes first"));
        let status = super::super::git::fetch("git-commit-status", &json!({"cwd": temp.path()}))?;
        assert_eq!(status["all"]["files"], 1);
        assert_eq!(status["commitsAhead"], 1);
        Ok(())
    }

    #[test]
    fn deadline_bounds_spawn_and_cleanup_replies() -> anyhow::Result<()> {
        const SERVER: &str = r#"
import json, sys, time
mode = sys.argv[1]
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        result = {"userAgent":"deadline-fixture","codexHome":"/tmp","platformOs":"linux"}
    elif method == "process/spawn":
        if mode == "spawn":
            time.sleep(6)
        result = {}
    elif method == "process/kill":
        time.sleep(6)
        result = {}
    else:
        continue
    print(json.dumps({"id":message["id"],"result":result}), flush=True)
"#;
        for mode in ["spawn", "cleanup"] {
            let mut command = Command::new("python3");
            command.args(["-u", "-c", SERVER, mode]);
            let (bridge, _) = Bridge::spawn(command)?;
            let bridge = Arc::new(bridge);
            pollster::block_on(bridge.initialize("codex_gpui", "0.1.0"))?;
            let start = Instant::now();
            let result = execute(&bridge, Path::new("/tmp"), &["unused".into()], 100);
            let error = result.expect_err("a server without an exit event must time out");
            assert!(
                error.to_string().to_lowercase().contains("timed out"),
                "{error:#}"
            );
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "{mode} exceeded the deadline"
            );
        }
        Ok(())
    }
}
