#[cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use app::host_settings::Settings;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn call(host: &LocalHost, endpoint: &str, params: Value) -> Result<Value> {
    json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[test]
fn environment_conflicts_inheritance_selection_and_worktree_snapshots() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let project = dir.path().join("project");
    fs::create_dir_all(&project)?;
    git(&project, &["init", "-q"])?;
    git(&project, &["config", "user.name", "Fixture"])?;
    git(
        &project,
        &["config", "user.email", "fixture@example.invalid"],
    )?;
    fs::write(project.join("tracked.txt"), "original\n")?;
    git(&project, &["add", "."])?;
    git(&project, &["commit", "-qm", "initial"])?;
    git(&project, &["branch", "-M", "main"])?;
    let initial = git(&project, &["rev-parse", "HEAD"])?;
    let origin = dir.path().join("origin.git");
    fs::create_dir(&origin)?;
    git(&origin, &["init", "--bare", "-q"])?;
    git(
        &project,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().context("origin path")?,
        ],
    )?;
    git(&project, &["push", "-qu", "origin", "main"])?;
    fs::write(project.join("upstream.txt"), "fetched before creation\n")?;
    git(&project, &["add", "upstream.txt"])?;
    git(&project, &["commit", "-qm", "upstream change"])?;
    git(&project, &["push", "-q", "origin", "main"])?;
    git(&project, &["reset", "--hard", &initial])?;
    git(
        &project,
        &["update-ref", "refs/remotes/origin/main", &initial],
    )?;
    git(
        &project,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    )?;
    let settings_path = dir.path().join("settings.json");
    let mut settings = Settings::load(settings_path.clone())?;
    let worktrees = dir.path().join("custom-worktrees");
    let host = LocalHost::new(&home).with_settings(Some(settings.clone()));
    settings.write("git-worktree-root", json!(worktrees))?;
    settings.write("worktree-upstream-refresh-mode", json!("best-effort"))?;
    let config = project.join(".codex/environments/local.toml");
    let raw = "version = 1\nname = 'local'\n[setup]\nscript = 'printf configured > setup-result'\n[[actions]]\nname = 'Run'\ncommand = 'printf action'\nicon = 'run'\nplatform = 'linux'\n";
    let saved = call(
        &host,
        "local-environment-config-save",
        json!({"configPath": config, "expectedRevision": null, "raw": raw}),
    )?;
    let stale = saved["revision"].clone();
    let read = call(
        &host,
        "local-environment-config",
        json!({"configPath": config}),
    )?;
    ensure!(read["environment"]["environment"]["actions"][0]["command"] == "printf action");
    let nested = project.join("child");
    fs::create_dir(&nested)?;
    let inherited = call(
        &host,
        "local-environments",
        json!({"workspaceRoot": nested}),
    )?;
    ensure!(inherited["environments"][0]["configPath"] == json!(config));
    let changed = raw.replace("printf action", "printf updated");
    let saved = call(
        &host,
        "local-environment-config-save",
        json!({"configPath": config, "expectedRevision": stale, "raw": changed}),
    )?;
    ensure!(saved["type"] == "success");
    let conflict = call(
        &host,
        "local-environment-config-save",
        json!({"configPath": config, "expectedRevision": stale, "raw": raw}),
    )?;
    ensure!(conflict["type"] == "conflict");
    ensure!(fs::read_to_string(&config)? == changed);
    ensure!(call(&host, "local-environment-config-save", json!({"configPath": config, "expectedRevision": saved["revision"], "raw": "version = 1\nname = 'bad'\n[setup]\nscript = ''\n[[actions]]\nname = 'Run'\ncommand = 'run'\nplatform = 'macos'\n"})).is_err());
    call(
        &host,
        "local-environment-select",
        json!({"workspaceRoot": project, "configPath": config}),
    )?;
    let created = call(&host, "create-worktree", json!({"cwd": project}))?;
    let root = Path::new(
        created["worktreeGitRoot"]
            .as_str()
            .context("created root")?,
    );
    ensure!(root.starts_with(&worktrees));
    ensure!(fs::read_to_string(root.join("setup-result"))? == "configured");
    ensure!(
        fs::read_to_string(root.join("upstream.txt"))
            .context("worktree must start from the refreshed upstream")?
            == "fetched before creation\n"
    );
    call(
        &host,
        "set-worktree-owner-thread",
        json!({"worktree": root, "conversationId": "fixture-thread"}),
    )?;
    let catalog = call(&host, "codex-worktrees", json!({}))?;
    ensure!(catalog["worktrees"][0]["dir"] == json!(root));
    ensure!(call(&host, "worktree-delete", json!({"worktree": project})).is_err());
    let nested_repository = root.join("nested");
    fs::create_dir(&nested_repository)?;
    git(&nested_repository, &["init", "-q"])?;
    ensure!(call(&host, "worktree-delete", json!({"worktree": root})).is_err());
    ensure!(root.exists());
    fs::remove_dir_all(&nested_repository)?;
    fs::write(root.join("tracked.txt"), "changed\n")?;
    fs::write(root.join("untracked.txt"), "preserved\n")?;
    call(&host, "worktree-delete", json!({"worktree": root}))?;
    ensure!(!root.exists());
    let reference = git(
        &project,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/codex/snapshots",
        ],
    )?;
    ensure!(git(&project, &["show", &format!("{reference}:tracked.txt")])? == "changed");
    ensure!(git(&project, &["show", &format!("{reference}:untracked.txt")])? == "preserved");
    ensure!(fs::read_to_string(project.join("tracked.txt"))? == "original\n");
    Ok(())
}

#[test]
fn watcher_preferences_update_only_active_pr_heartbeats() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let dir = tempfile::tempdir()?;
    let host = LocalHost::new(dir.path());
    let prompt = "## Pull request fix automation:\nPull request URL: https://example.invalid/repo/pull/1\n\nThis is a heartbeat turn. Continue automatically fixing this pull request.\nOld instructions";
    let unrelated = "Explain the sentence This is a heartbeat turn. Continue automatically fixing this pull request. without creating a watch task.";
    for (id, status, prompt) in [
        ("active", "ACTIVE", prompt),
        ("paused", "PAUSED", prompt),
        ("owned", "ACTIVE", prompt),
        ("unrelated", "ACTIVE", unrelated),
    ] {
        host.write_automation(id, &format!("version = 1\nid = {id:?}\nname = 'Monitor'\nprompt = {prompt:?}\nstatus = {status:?}\nkind = 'heartbeat'\ntarget_thread_id = 'fixture-thread'\nrrule = 'FREQ=MINUTELY;INTERVAL=10'\ncreated_at = 1\nupdated_at = 1\n"))?;
    }
    fs::create_dir(dir.path().join("sqlite"))?;
    let database = rusqlite::Connection::open(dir.path().join("sqlite/codex.db"))?;
    database.execute_batch(
        "CREATE TABLE automations (
        id TEXT PRIMARY KEY, name TEXT, prompt TEXT, next_run_at INTEGER, next_run_nominal_at INTEGER,
        last_run_at INTEGER, rrule TEXT, status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
        INSERT INTO automations VALUES ('active', 'Monitor', '', NULL, NULL, NULL, 'FREQ=MINUTELY;INTERVAL=10', 'ACTIVE', NULL, NULL, NULL);
        INSERT INTO automations VALUES ('owned', 'Monitor', '', NULL, NULL, NULL, 'FREQ=MINUTELY;INTERVAL=10', 'ACTIVE', 'account', 'user', 'installation');",
    )?;
    let owned_path = dir.path().join("automations/owned/automation.toml");
    let owned_before = fs::read(&owned_path)?;
    let paused_path = dir.path().join("automations/paused/automation.toml");
    let paused_before = fs::read(&paused_path)?;
    let unrelated_path = dir.path().join("automations/unrelated/automation.toml");
    let unrelated_before = fs::read(&unrelated_path)?;
    let response = call(
        &host,
        "git-pr-watch-update",
        json!({"autoMerge": true, "instructions": "Keep watching."}),
    )?;
    ensure!(response["updated"] == 1);
    let definition: toml::Value = toml::from_str(&fs::read_to_string(
        dir.path().join("automations/active/automation.toml"),
    )?)?;
    let result = definition["prompt"].as_str().context("watcher prompt")?;
    ensure!(result.starts_with(
        "## Pull request fix automation:\nPull request URL: https://example.invalid/repo/pull/1\n\n"
    ));
    ensure!(result.contains("continue until the PR is merged or closed"));
    ensure!(result.ends_with("Keep watching."));
    ensure!(!result.contains("Old instructions"));
    ensure!(fs::read(paused_path)? == paused_before);
    ensure!(fs::read(unrelated_path)? == unrelated_before);
    ensure!(fs::read(owned_path)? == owned_before);
    Ok(())
}

#[test]
fn deleting_one_worktree_keeps_sibling_reservations_visible() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let directory = tempfile::tempdir()?;
    let project = directory.path().join("project");
    fs::create_dir(&project)?;
    git(&project, &["init", "-q"])?;
    git(&project, &["config", "user.name", "Fixture"])?;
    git(
        &project,
        &["config", "user.email", "fixture@example.invalid"],
    )?;
    git(&project, &["commit", "--allow-empty", "-qm", "initial"])?;
    let managed = directory.path().join("worktrees");
    let reservation = managed.join("shared-reservation");
    fs::create_dir_all(&reservation)?;
    let marker = reservation.join(".codex-worktree-name");
    fs::write(&marker, "")?;
    let first = reservation.join("first");
    let second = reservation.join("second");
    for path in [&first, &second] {
        git(
            &project,
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().context("worktree path")?,
                "HEAD",
            ],
        )?;
    }
    let mut settings = Settings::default();
    settings.write("git-worktree-root", json!(managed))?;
    let host = LocalHost::new(directory.path().join("home")).with_settings(Some(settings));
    call(&host, "worktree-delete", json!({"worktree": first}))?;
    let catalog = call(&host, "codex-worktrees", json!({}))?;
    ensure!(catalog["worktrees"][0]["dir"] == json!(second));
    call(&host, "worktree-delete", json!({"worktree": second}))?;
    ensure!(!reservation.exists());
    Ok(())
}
