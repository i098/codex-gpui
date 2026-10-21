#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::host::LocalHost;
use common::{Fx, SUBTRACT, script};
use std::{fs, path::Path, process::Command, sync::Arc};

#[test]
fn proxy() {
    common::proxy_child();
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn pr_fields_keep_the_chat_draft_and_block_an_unavailable_provider() -> Result<()> {
    let root = tempfile::tempdir()?;
    let remote = tempfile::tempdir()?;
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"])?;
    git(root.path(), &["init", "-q", "-b", "main"])?;
    git(root.path(), &["config", "user.name", "PR field proof"])?;
    git(
        root.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    fs::write(root.path().join("tracked"), "base\n")?;
    git(root.path(), &["add", "tracked"])?;
    git(root.path(), &["commit", "-qm", "Base"])?;
    git(
        root.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    )?;
    git(root.path(), &["push", "-q", "-u", "origin", "main"])?;
    git(root.path(), &["remote", "set-head", "origin", "main"])?;
    git(root.path(), &["checkout", "-qb", "feature"])?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.path().to_string_lossy().into_owned().into();
        }
    }
    let home = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(home.path()));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.composer().update(&mut fx.cx, |composer, cx| {
        composer.set_text("Keep this chat draft", cx)
    });
    fx.keys("cmd-k")?;
    fx.type_text("Create PR")?;
    fx.keys("enter")?;
    fx.wait("unavailable provider", |fx| {
        fx.read(|root, cx| {
            root.pull_request_form().is_some_and(|form| {
                form.read(cx).blocker(cx).as_deref() == Some("Pull request creation unavailable")
            })
        })
    })?;
    fx.type_text("Change the behavior")?;
    assert_eq!(
        fx.read(|root, cx| root
            .pull_request_form()
            .unwrap()
            .read(cx)
            .title_field()
            .read(cx)
            .field_text()
            .to_owned()),
        "Change the behavior"
    );
    fx.keys("tab")?;
    fx.type_text("Describe the change")?;
    assert_eq!(
        fx.read(|root, cx| root
            .pull_request_form()
            .unwrap()
            .read(cx)
            .body_field()
            .read(cx)
            .field_text()
            .to_owned()),
        "Describe the change"
    );
    fx.keys("cmd-enter")?;
    ensure!(fx.read(|root, _| root.pull_request_form().is_some()));
    assert_eq!(
        fx.read(|root, cx| root
            .pull_request_form()
            .unwrap()
            .read(cx)
            .body_field()
            .read(cx)
            .field_text()
            .to_owned()),
        "Describe the change"
    );
    fx.keys("escape")?;
    ensure!(fx.read(|root, _| root.pull_request_form().is_none()));
    assert_eq!(fx.composer_text()?, "Keep this chat draft");
    ensure!(fx.composer_focused()?);
    Ok(())
}
