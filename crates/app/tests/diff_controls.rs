#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;
use anyhow::{Context as _, Result, ensure};
use app::host::LocalHost;
use bridge::Event;
use common::{Fx, SUBTRACT, script};
use gpui::{AppContext as _, px, size};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use ui::screens::diff::panel::{DiffPanel, RepositoryChanges};

const PATCH: &str = "diff --git a/src/math.py b/src/math.py\n--- a/src/math.py\n+++ b/src/math.py\n@@ -1,2 +1,2 @@\n def add(a, b):\n-    return a - b\n+    return a + b\n";
fn out() -> PathBuf {
    std::env::var_os("CGX_DIFF_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-diff-controls"))
}
fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.reference_shot()?
        .save(out().join(format!("native-{name}.png")))?;
    println!("capture native-{name}.png");
    Ok(())
}
fn notify(fx: &mut Fx, method: &str, params: Value) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: method.into(),
                params,
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
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
fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .menu_items()
                .position(|(name, enabled)| enabled && name == label)
        })
        .context("menu item")?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}
fn loaded(fx: &mut Fx) -> Result<()> {
    fx.wait("repository read", |fx| {
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).pending())
    })
}

#[test]
fn header_wrap_retry_and_line_navigation_use_native_input() -> Result<()> {
    fs::create_dir_all(out())?;
    let repo = tempfile::tempdir_in(out())?;
    git(repo.path(), &["init", "-q"])?;
    git(repo.path(), &["config", "user.name", "Diff proof"])?;
    git(
        repo.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    fs::write(repo.path().join("a.txt"), "base\n")?;
    git(repo.path(), &["add", "a.txt"])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    let cwd = repo.path().to_string_lossy().into_owned();
    let mut recording = script(None, |_| {})?;
    for line in &mut recording {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = cwd.clone().into();
        }
    }
    let mut fx = Fx::scripted(recording)?;
    let home = tempfile::tempdir_in(out())?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(LocalHost::new(home.path())), cx)
    });
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    let turn = fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .turns()
            .last()
            .unwrap()
            .id
            .clone()
    });
    notify(
        &mut fx,
        "turn/diff/updated",
        json!({"threadId":SUBTRACT,"turnId":turn,"diff":PATCH}),
    );
    shot(&mut fx, "panel")?;
    fx.click((1252., 68.))?;
    shot(&mut fx, "tree")?;
    fx.click((1150., 110.))?;
    fx.type_text("not-present")?;
    shot(&mut fx, "filter-empty")?;
    fx.keys("cmd-a cmd-c")?;
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref()
            == Some("not-present"),
        "the file filter must own Copy while it has focus"
    );
    fx.cx
        .update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("math".into())));
    fx.keys("cmd-v")?;
    shot(&mut fx, "filter-match")?;
    fx.click((1252., 68.))?;
    fx.resize(1440., 900.)?;
    fx.click((1314., 68.))?;
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).wraps_lines()),
        "header wrap did not activate"
    );
    shot(&mut fx, "wrap")?;
    fx.click((1382., 68.))?;
    fx.click((1382., 68.))?;
    shot(&mut fx, "split")?;
    fx.resize(1280., 832.)?;
    fx.click((735., 68.))?;
    choose(&mut fx, "Unstaged")?;
    loaded(&mut fx)?;
    shot(&mut fx, "empty")?;
    let panel = fx.read(|root, _| root.diff_panel().unwrap().clone());
    panel.update(&mut fx.cx, |panel, cx| panel.begin_repository_request(cx));
    shot(&mut fx, "loading")?;
    panel.update(&mut fx.cx, |panel, cx| {
        panel.set_repository(
            serde_json::from_value::<RepositoryChanges>(
                json!({"root":cwd,"unstaged":[],"staged":[]}),
            )
            .map_err(|e| e.to_string()),
            cx,
        )
    });
    fs::write(repo.path().join("a.txt"), "changed\n")?;
    fx.click((735., 68.))?;
    choose(&mut fx, "Refresh")?;
    loaded(&mut fx)?;
    fs::rename(
        repo.path().join(".git/index"),
        repo.path().join(".git/index-owned-proof"),
    )?;
    fs::write(repo.path().join(".git/index"), "owned proof invalid index")?;
    fx.click((735., 68.))?;
    choose(&mut fx, "Refresh")?;
    loaded(&mut fx)?;
    ensure!(
        fx.read(|root, cx| root
            .diff_panel()
            .unwrap()
            .read(cx)
            .repository_error()
            .is_some()),
        "real read failure missing"
    );
    let retained = fx.reference_shot()?;
    retained.save(out().join("native-retry.png"))?;
    let scale = retained.width() / 1280;
    let visible_addition = (150..350).any(|y| {
        (1100..1200).all(|x| {
            let pixel = retained.get_pixel(x * scale, y * scale).0;
            pixel[1] > pixel[0] && pixel[1] > pixel[2]
        })
    });
    ensure!(
        visible_addition,
        "a failed read must keep the displayed changes visible"
    );
    fs::rename(
        repo.path().join(".git/index-owned-proof"),
        repo.path().join(".git/index"),
    )?;
    fx.click((1232., 120.))?;
    loaded(&mut fx)?;
    ensure!(
        fx.read(|root, cx| root
            .diff_panel()
            .unwrap()
            .read(cx)
            .repository_error()
            .is_none()),
        "Retry did not read the restored repository"
    );
    println!("real read failure and Retry recovered");

    let standalone = fx.cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        cx.new(|_| DiffPanel::from_patch(&cwd, PATCH))
    })?;
    fx.window = standalone.into();
    fx.size = (1280., 832.);
    shot(&mut fx, "standalone-auto-1280")?;
    fx.click((1183., 22.))?;
    shot(&mut fx, "standalone-unified")?;
    fx.click((1219., 22.))?;
    shot(&mut fx, "standalone-split")?;
    fx.click((1255., 22.))?;
    fx.resize(767., 832.)?;
    shot(&mut fx, "standalone-auto-767")?;
    fx.resize(600., 832.)?;
    shot(&mut fx, "standalone-auto-600")?;
    fx.resize(1280., 832.)?;
    let large = format!(
        "diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -0,0 +1 @@\n+{}\n",
        "x".repeat(10_001)
    );
    let big = fx.cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        cx.new(|_| DiffPanel::from_patch(&cwd, &large))
    })?;
    fx.window = big.into();
    shot(&mut fx, "too-large")?;
    let invalid = "diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -broken +1 @@\n+new\n";
    let bad = fx.cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        cx.new(|_| DiffPanel::from_patch(&cwd, invalid))
    })?;
    fx.window = bad.into();
    shot(&mut fx, "render-failure")?;
    let file = fx.cx.open_window(size(px(600.), px(832.)), |_, cx| {
        cx.new(|cx| {
            let mut file =
                ui::screens::diff::panel::FileView::new(&cwd, format!("{cwd}/a.py"), None);
            file.set_contents(Err("owned file is unavailable".into()), cx);
            file
        })
    })?;
    let view = fx.cx.read_window(&file, |view, _| view)?;
    fx.window = file.into();
    fx.size = (600., 832.);
    fx.click((200., 100.))?;
    fx.keys("cmd-l")?;
    fx.type_text("17")?;
    fx.keys("enter")?;
    ensure!(
        view.read_with(&fx.cx, |view, _| view.line.is_none()),
        "an unavailable file must not accept line navigation"
    );
    shot(&mut fx, "line-unavailable")?;
    view.update(&mut fx.cx, |file, cx| {
        file.set_contents(Ok((1..=100).map(|n| format!("line {n}\n")).collect()), cx);
    });
    fx.keys("cmd-l")?;
    shot(&mut fx, "go-to-line")?;
    fx.type_text("-2")?;
    fx.keys("enter")?;
    ensure!(
        view.read_with(&fx.cx, |view, _| view.line == Some(100)),
        "negative line navigation differs from the original"
    );
    shot(&mut fx, "line-100")?;
    println!("native Diff controls and boundary frames verified");
    Ok(())
}
