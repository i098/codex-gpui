#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::{host::LocalHost, host_settings::Settings};
use common::{Fx, SUBTRACT};
use serde_json::Value;
use std::{fs, path::Path, process::Command, sync::Arc};

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .menu_items()
                .position(|(text, enabled)| text == label && enabled)
        })
        .ok_or_else(|| anyhow::anyhow!("missing menu item {label}"))?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    fx.wait("branch diff", |fx| {
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).pending())
    })
}

fn open(root: &Path, home: &Path, settings: Settings) -> Result<Fx> {
    let mut script = common::script(None, |_| {})?;
    for row in &mut script {
        if let Some(thread) = row.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let mut fx = Fx::scripted(script)?;
    fx.cx.update(|cx| cx.set_global(settings));
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(LocalHost::new(home)), cx)
    });
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    fx.click((735., 68.))?;
    choose(&mut fx, "Branch")?;
    Ok(fx)
}

fn expanded(fx: &Fx) -> bool {
    fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].open)
}

#[test]
fn branch_viewed_revision_survives_reopen_and_rejects_failed_save() -> Result<()> {
    common::install_panic_hook();
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("repo");
    let home = directory.path().join("home");
    fs::create_dir_all(&root)?;
    fs::create_dir_all(&home)?;
    git(&root, &["init", "-q", "-b", "main"])?;
    git(&root, &["config", "user.name", "Viewed proof"])?;
    git(&root, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(root.join("file.txt"), "base\n")?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-qm", "base"])?;
    git(&root, &["checkout", "-qb", "feature"])?;
    fs::write(root.join("file.txt"), "review me\n")?;
    git(&root, &["commit", "-qam", "change"])?;
    let path = home.join("settings.json");
    let mut settings = Settings::load(path.clone())?;
    settings.write_batch(
        common::recorded_host_settings()?
            .read_all()
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone())),
    )?;
    let mut fx = open(&root, &home, settings)?;
    ensure!(expanded(&fx));
    fx.capture_evidence("CGX_DIFF_VIEWED_EVIDENCE", "branch-unviewed")?;
    fx.click((1208., 111.))?;
    ensure!(!expanded(&fx), "marking must collapse the file");
    let saved: Value = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        saved["review-viewed-file-v1"]
            .as_object()
            .is_some_and(|v| v.len() == 1)
    );
    fx.capture_evidence("CGX_DIFF_VIEWED_EVIDENCE", "branch-viewed")?;
    drop(fx);
    let mut fx = open(&root, &home, Settings::load(path.clone())?)?;
    ensure!(!expanded(&fx), "the saved revision must survive reopening");
    fx.click((1208., 111.))?;
    ensure!(expanded(&fx), "unmarking must expand the file");
    let cleared: Value = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        cleared["review-viewed-file-v1"]
            .as_object()
            .is_some_and(|v| v.is_empty())
    );
    fx.click((1208., 111.))?;
    ensure!(!expanded(&fx), "the file can be marked again");
    fs::write(root.join("file.txt"), "new revision\n")?;
    git(&root, &["commit", "-qam", "revision"])?;
    fx.click((735., 68.))?;
    choose(&mut fx, "Branch")?;
    fx.wait("new revision", |fx| expanded(fx))?;
    fx.capture_evidence("CGX_DIFF_VIEWED_EVIDENCE", "branch-revised")?;
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    fx.click((1208., 111.))?;
    ensure!(expanded(&fx), "a failed save must leave the file expanded");
    Ok(())
}
