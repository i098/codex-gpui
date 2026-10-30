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
        .context("GitHub requires an app-server connection")?;
    let output = execute(bridge, cwd, args, 120_000)?;
    ensure!(
        !output.stdout_cap_reached && !output.stderr_cap_reached,
        "GitHub output is incomplete"
    );
    ensure!(output.exit_code == 0, "{}", output.stderr.trim());
    Ok(output.stdout.trim().to_owned())
}

pub(super) fn summary(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let branch = super::git::fetch("git-commit-status", params)?["branch"]
        .as_str()
        .context("Create a branch before opening a pull request")?
        .to_owned();
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
    let existing = summary(host, params)?;
    if !existing.is_null() && params["commitChanges"] != true {
        return Ok(existing);
    }
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let status = super::git::fetch("git-commit-status", params)?;
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
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
        print(json.dumps({"method":"process/exited","params":{"processHandle":message["params"]["processHandle"],"exitCode":0,"stdout":sys.argv[1],"stderr":"","stdoutCapReached":False,"stderrCapReached":False}}), flush=True)
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
