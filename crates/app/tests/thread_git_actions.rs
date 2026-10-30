mod common;

use anyhow::{Result, ensure};
use app::host::LocalHost;
use common::{Fx, GREETING, script};
use std::{fs, path::Path, process::Command, sync::Arc};
use ui::dialog::Kind;

#[test]
fn proxy() {
    common::proxy_child();
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[test]
fn failed_pr_creation_retains_the_description_and_local_changes() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path();
    git(root, &["init", "-q", "-b", "feature"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native proof"])?;
    fs::write(root.join("tracked"), "base\n")?;
    git(root, &["add", "tracked"])?;
    git(root, &["commit", "-qm", "Base"])?;
    let original_head = git(root, &["rev-parse", "HEAD"])?;
    fs::write(root.join("tracked"), "pending\n")?;
    let mut recording = script(None, |_| {})?;
    for line in &mut recording {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let home = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(home.path()));
    #[cfg(target_os = "macos")]
    let mut fx = Fx::captured("dark", (1280., 832.), recording)?;
    #[cfg(not(target_os = "macos"))]
    let mut fx = Fx::scripted(recording)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click((1217., 22.))?;
    fx.click((1242., 106.))?;
    fx.wait("Git menu loaded", |fx| {
        fx.read(|root, cx| {
            let view = root.thread().unwrap().read(cx);
            view.git_actions_open()
                && view
                    .git_action_rows()
                    .iter()
                    .all(|(_, reason)| !reason.is_some_and(|reason| reason.starts_with("Loading")))
        })
    })?;
    #[cfg(target_os = "macos")]
    let evidence = {
        let path = std::env::var_os("CGX_THREAD_GIT_EVIDENCE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("cgx-thread-git"));
        fs::create_dir_all(&path)?;
        fx.shot()?.save(path.join("git-menu.png"))?;
        path
    };
    fx.keys("home down down down down enter")?;
    fx.wait("Create PR dialog", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|dialog| dialog.read(cx).kind() == Kind::CreatePr)
        })
    })?;
    fx.type_text("Keep this description after a failed request.")?;
    fx.keys("shift-tab cmd-a")?;
    fx.type_text("Native pull request")?;
    fx.keys("tab tab")?;
    fx.type_text("main")?;
    #[cfg(target_os = "macos")]
    fx.shot()?.save(evidence.join("create-pr.png"))?;
    fx.keys("tab tab tab tab enter")?;
    fx.wait("Failed request retains the dialog", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|dialog| !dialog.read(cx).saving())
        })
    })?;
    ensure!(
        fx.read(|root, cx| root.dialog().unwrap().read(cx).query().to_owned())
            == "Keep this description after a failed request."
    );
    ensure!(git(root, &["rev-parse", "HEAD"])? == original_head);
    ensure!(fs::read_to_string(root.join("tracked"))? == "pending\n");
    #[cfg(target_os = "macos")]
    fx.shot()?.save(evidence.join("create-pr-failure.png"))?;
    fx.keys("escape")?;
    ensure!(fx.composer_focused()?);
    #[cfg(target_os = "macos")]
    {
        let thread = fx.read(|root, _| root.thread().unwrap().clone());
        thread.update(&mut fx.cx, |view, cx| view.set_worktree_missing(true, cx));
        fx.shot()?.save(evidence.join("restore-banner.png"))?;
        thread.update(&mut fx.cx, |view, cx| {
            view.set_worktree_missing(false, cx);
            view.set_worktree_status(Some("Checkout created; setup failed"), cx);
        });
        fx.shot()?.save(evidence.join("setup-failure.png"))?;
    }
    Ok(())
}
