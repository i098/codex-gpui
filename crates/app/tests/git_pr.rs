use anyhow::{Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use serde_json::{Value, json};

#[test]
fn pull_request_updates_reject_cross_host_targets_before_execution() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let request = json!({
        "account": {"hostId": "local", "hostname": "github.com", "login": "owner"},
        "pullRequest": {"hostname": "github.com", "owner": "owner", "repository": "repo", "number": 1},
        "action": "mark-ready",
    });
    for (path, value, expected) in [
        (
            "/account/hostId",
            json!("another-host"),
            "Pull request account does not match the request",
        ),
        (
            "/pullRequest/hostname",
            json!("another.example"),
            "Pull request account does not match the request",
        ),
        (
            "/pullRequest/repository",
            json!("../repo"),
            "invalid GitHub repository",
        ),
    ] {
        let mut changed = request.clone();
        *changed.pointer_mut(path).unwrap() = value;
        let response = json_body(&host.fetch(&Request::post("gh-pr-update", Some(changed)))?)?;
        ensure!(
            response["status"] == "error" && response["error"] == expected,
            "{response}"
        );
    }
    let remote = Request::post(
        "github-request",
        Some(json!({
            "hostConfig": {"id": "another-host"},
            "request": {"method": "gh-pr-update", "params": request},
        })),
    );
    ensure!(host.fetch(&remote).is_err());
    Ok(())
}

#[test]
fn pr_creation_rejects_relative_workspace_before_authentication() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let response: Value = json_body(&host.fetch(&Request::post(
        "gh-pr-create",
        Some(json!({
            "cwd": "relative", "headBranch": "branch",
        })),
    ))?)?;
    ensure!(
        response["status"] == "error"
            && response["error"] == "No workspace available for PR creation"
    );
    Ok(())
}

#[test]
fn malformed_auto_merge_state_cannot_become_a_disable_request() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let result = json_body(&host.fetch(&Request::post("gh-pr-update", Some(json!({
        "account": {"hostId": "local", "hostname": "github.com"},
        "pullRequest": {"hostname": "github.com", "owner": "owner", "repository": "repo", "number": 1},
        "action": "toggle-auto-merge", "enabled": "true",
    }))))?)?;
    assert_eq!(result["status"], "error");
    assert_eq!(result["error"], "missing auto-merge state");
    Ok(())
}
