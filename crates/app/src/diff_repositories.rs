use crate::host::Host;
use anyhow::Result;
use bridge::Bridge;
use gpui::{App, Entity};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::diff::panel::DiffPanel;
use ui::screens::diff::panel::ReviewRepository;

/// With `remote`, the repository is read through that host's `command/exec`; otherwise locally.
pub(crate) fn load(
    panel: Entity<DiffPanel>,
    host: Arc<dyn Host>,
    remote: Option<Arc<Bridge>>,
    thread: String,
    cwd: String,
    cx: &mut App,
) {
    if panel.read(cx).last_turn_only() {
        return;
    }
    let target = panel.downgrade();
    let request = cx.background_executor().spawn(async move {
        match remote {
            Some(bridge) => remote_repositories(&bridge, &cwd).await,
            None => host.review_repositories(&thread, &cwd),
        }
    });
    cx.spawn(async move |cx| match request.await {
        Ok(repositories) => {
            let _ = target.update(cx, |panel, cx| panel.set_repositories(repositories, cx));
        }
        Err(error) => eprintln!("codex-gpui: cannot read review repositories: {error:#}"),
    })
    .detach();
}

async fn remote_repositories(bridge: &Bridge, cwd: &str) -> Result<Vec<ReviewRepository>> {
    let params = json!({
        "command": ["git", "-C", cwd, "rev-parse", "--show-toplevel"],
        "cwd": cwd,
    });
    let response: Value = bridge.request("command/exec", params).await?;
    Ok(toplevel_repository(cwd, &response).into_iter().collect())
}

/// The repository a `git rev-parse --show-toplevel` run printed; none when git exited non-zero.
fn toplevel_repository(cwd: &str, response: &Value) -> Option<ReviewRepository> {
    if response["exitCode"] != 0 {
        return None;
    }
    let root = response["stdout"].as_str()?.trim();
    let name = root.rsplit('/').find(|part| !part.is_empty())?;
    Some(ReviewRepository {
        root: root.to_owned(),
        cwd: cwd.to_owned(),
        name: name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toplevel_names_the_repository_and_a_failed_git_names_none() {
        let ok = json!({"exitCode": 0, "stdout": "/srv/work/app\n", "stderr": ""});
        let repository = toplevel_repository("/srv/work/app/src", &ok).unwrap();
        assert_eq!(
            (
                repository.root.as_str(),
                repository.name.as_str(),
                repository.cwd.as_str()
            ),
            ("/srv/work/app", "app", "/srv/work/app/src")
        );
        let failed = json!({"exitCode": 128, "stdout": "", "stderr": "not a git repository"});
        assert!(toplevel_repository("/tmp", &failed).is_none());
    }
}
