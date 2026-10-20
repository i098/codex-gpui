use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn reviewed_sources_and_patches_change_only_the_selected_repository_content() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let call = |endpoint: &str, mut params: Value| -> Result<Value> {
        params["cwd"] = root.to_string_lossy().into();
        json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)
    };
    let patch = |source: &str, files: Value| json!({"source": source, "files": files});
    ensure!(call("git-changes", json!({}))?["not_repository"] == true);
    fs::write(root.join("keep.txt"), "keep this untracked file\n")?;
    call("git-init-repo", json!({}))?;
    ensure!(root.join(".git").is_dir());
    ensure!(fs::read_to_string(root.join("keep.txt"))? == "keep this untracked file\n");
    ensure!(call("git-init-repo", json!({})).is_err());
    git(root, &["config", "user.name", "Repository proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    let base = (1..=24).map(|n| format!("line {n}\n")).collect::<String>();
    fs::write(root.join("a.txt"), &base)?;
    call("git-stage", json!({"path": "a.txt"}))?;
    call("git-unstage", json!({"path": "a.txt"}))?;
    ensure!(git(root, &["ls-files"])?.is_empty());
    call("git-stage", json!({"path": "a.txt"}))?;
    git(root, &["commit", "-qm", "base"])?;
    git(root, &["branch", "-M", "main"])?;
    let first = git(root, &["rev-parse", "HEAD"])?;
    git(root, &["checkout", "-qb", "feature"])?;
    fs::write(root.join("committed.txt"), "branch change\n")?;
    git(root, &["add", "committed.txt"])?;
    git(root, &["commit", "-qm", "feature change"])?;
    let branch = call(
        "git-changes",
        json!({"reviewSource": "branch", "baseBranch": "main"}),
    )?;
    ensure!(branch["review"][0]["path"] == "committed.txt");
    ensure!(branch["base_branch"] == "main");
    let commit = call(
        "git-changes",
        json!({"reviewSource": "commits", "commit": first.trim()}),
    )?;
    ensure!(commit["review"][0]["path"] == "a.txt");
    ensure!(
        commit["review"][0]["diff"]
            .as_str()
            .context("root commit patch")?
            .contains("+line 24")
    );
    ensure!(
        call(
            "git-changes",
            json!({"reviewSource": "commits", "commit": "--all"})
        )
        .is_err()
    );
    let edited = base
        .replace("line 2\n", "first edit\n")
        .replace("line 23\n", "last edit\n");
    fs::write(root.join("a.txt"), &edited)?;
    let changes = call("git-changes", json!({"reviewSource": "uncommitted"}))?;
    ensure!(
        changes["review"]
            .as_array()
            .context("uncommitted")?
            .iter()
            .any(|f| f["path"] == "a.txt")
    );
    let a = changes["unstaged"]
        .as_array()
        .context("unstaged")?
        .iter()
        .find(|f| f["path"] == "a.txt")
        .context("a patch")?;
    let mut hunk = a.clone();
    hunk["hunk"] = 0.into();
    let changes = call("git-stage", patch("unstaged", json!([hunk])))?;
    let staged = base.replace("line 2\n", "first edit\n");
    ensure!(git(root, &["show", ":a.txt"])? == staged);
    ensure!(fs::read_to_string(root.join("a.txt"))? == edited);
    let mut staged_hunk = changes["staged"][0].clone();
    staged_hunk["hunk"] = 0.into();
    call("git-unstage", patch("staged", json!([staged_hunk])))?;
    ensure!(git(root, &["show", ":a.txt"])? == base);
    ensure!(fs::read_to_string(root.join("a.txt"))? == edited);
    let changes = call("git-changes", json!({}))?;
    let mut revert_hunk = changes["unstaged"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "a.txt")
        .unwrap()
        .clone();
    revert_hunk["source"] = "unstaged".into();
    revert_hunk["hunk"] = 0.into();
    call("git-revert", revert_hunk)?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == base.replace("line 23\n", "last edit\n"));
    let changes = call("git-changes", json!({}))?;
    fs::write(root.join("a.txt"), "newer user work\n")?;
    ensure!(call("git-stage", patch("unstaged", changes["unstaged"].clone())).is_err());
    ensure!(git(root, &["show", ":a.txt"])? == base);
    ensure!(fs::read_to_string(root.join("keep.txt"))? == "keep this untracked file\n");
    let changes = call("git-changes", json!({}))?;
    fs::write(root.join(".git/index.lock"), "owned proof lock")?;
    ensure!(call("git-stage", patch("unstaged", changes["unstaged"].clone())).is_err());
    fs::remove_file(root.join(".git/index.lock"))?;
    let changes = call("git-stage", patch("unstaged", changes["unstaged"].clone()))?;
    ensure!(git(root, &["show", ":a.txt"])? == "newer user work\n");
    ensure!(git(root, &["show", ":keep.txt"])? == "keep this untracked file\n");
    let changes = call("git-unstage", patch("staged", changes["staged"].clone()))?;
    ensure!(git(root, &["show", ":a.txt"])? == base);
    ensure!(fs::read_to_string(root.join("a.txt"))? == "newer user work\n");
    let mut selected = changes["unstaged"].as_array().unwrap().clone();
    selected.retain(|f| f["path"] == "a.txt");
    call("git-revert", patch("unstaged", json!(selected)))?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == base);
    ensure!(fs::read_to_string(root.join("keep.txt"))? == "keep this untracked file\n");
    let changes = call("git-changes", json!({}))?;
    call("git-revert", patch("unstaged", changes["unstaged"].clone()))?;
    ensure!(!root.join("keep.txt").exists());
    fs::write(root.join("a.txt"), &edited)?;
    let changes = call("git-changes", json!({}))?;
    let changes = call("git-stage", patch("unstaged", changes["unstaged"].clone()))?;
    fs::write(root.join("a.txt"), "preserve newer work\n")?;
    let result = call("git-revert", patch("staged", changes["staged"].clone()))?;
    ensure!(result["status"] == "partial-success");
    ensure!(git(root, &["show", ":a.txt"])? == base);
    ensure!(fs::read_to_string(root.join("a.txt"))? == "preserve newer work\n");
    println!(
        "Proved creation, root commit and branch sources, file and hunk actions, section stage/unstage/revert, stale snapshots, index locks, and preservation of newer user work."
    );
    Ok(())
}
