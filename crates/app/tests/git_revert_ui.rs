#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::{host::LocalHost, host_settings::Settings};
use common::{Fx, GREETING, proxy_child, recorded_host_settings, script};
use serde_json::json;
use std::{fs, path::Path, process::Command, sync::Arc};
use ui::dialog::Kind;

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .menu_items()
                .filter(|(_, enabled)| *enabled)
                .position(|(text, _)| text == label)
        })
        .with_context(|| format!("enabled menu item {label}"))?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

fn loaded(fx: &mut Fx) -> Result<()> {
    fx.wait("repository changes", |fx| {
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).pending())
    })
}

fn source(fx: &mut Fx, label: &str) -> Result<()> {
    fx.hover((1000., 22.))?;
    fx.advance(ui::toast::LIFETIME + std::time::Duration::from_secs(1))?;
    fx.click((735., 70.))?;
    choose(fx, label)?;
    loaded(fx)
}

fn file_revert(fx: &mut Fx) -> Result<()> {
    fx.click((1258., 110.))?;
    choose(fx, "Revert file")
}

fn confirmation(fx: &Fx) -> Result<()> {
    ensure!(
        fx.read(|root, cx| root.dialog().is_some_and(|dialog| {
            let dialog = dialog.read(cx);
            dialog.kind() == Kind::RevertChanges && dialog.focused_control() == "Confirm"
        })),
        "revert confirmation must focus Confirm"
    );
    Ok(())
}

fn toast(fx: &Fx, expected: &str) -> Result<()> {
    ensure!(
        fx.read(|root, cx| root
            .toasts(cx)
            .first()
            .is_some_and(|title| title == expected)),
        "missing result: {expected}"
    );
    Ok(())
}

fn open(root: &Path, settings: Settings) -> Result<Fx> {
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    let host = Arc::new(LocalHost::new(root.join(".git/proof-profile")));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-b")?;
    source(&mut fx, "Unstaged")?;
    Ok(fx)
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn confirmed_reverts_preserve_unselected_changes_and_report_partial_results() -> Result<()> {
    let scratch = std::env::var_os("CGX_GIT_REVERT_UI_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-git-revert-ui"));
    fs::create_dir_all(&scratch)?;
    let repo = tempfile::tempdir_in(&scratch)?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native proof"])?;
    let base = (1..=24)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let edited = base
        .replace("line 2\n", "first edit\n")
        .replace("line 23\n", "last edit\n");
    fs::write(root.join("a.txt"), &base)?;
    git(root, &["add", "a.txt"])?;
    git(root, &["commit", "-qm", "base"])?;
    fs::write(root.join("a.txt"), &edited)?;
    let settings_path = scratch.join(format!("host-settings-{}.json", std::process::id()));
    let mut settings = Settings::load(settings_path.clone())?;
    for (key, value) in recorded_host_settings()?.read_all() {
        settings.write(key, value.clone())?;
    }
    let mut fx = open(root, settings)?;
    fx.shot()?.save(scratch.join("after-controls.png"))?;
    fx.resize(1440., 900.)?;
    fx.shot()?.save(scratch.join("after-controls-1440.png"))?;
    fx.resize(1280., 832.)?;
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    fx.shot()?.save(scratch.join("after-confirmation.png"))?;
    fx.keys("tab space tab enter")?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(
        fs::read_to_string(root.join("a.txt"))? == edited,
        "Cancel changed the file"
    );
    ensure!(
        git(root, &["show", ":a.txt"])? == base,
        "Cancel changed the index"
    );
    ensure!(
        Settings::load(settings_path.clone())?
            .read_all()
            .get("skip-revert-confirmation")
            .is_none(),
        "Cancel saved Don't ask again"
    );
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    ensure!(fx.read(|root, cx| !root.dialog().unwrap().read(cx).do_not_ask_again()));
    fx.keys("escape")?;
    ensure!(
        fs::read_to_string(root.join("a.txt"))? == edited,
        "Escape changed the file"
    );
    fx.click((1210., 110.))?;
    confirmation(&fx)?;
    fx.click((650., 453.))?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(
        fs::read_to_string(root.join("a.txt"))? == edited,
        "pointer Cancel changed the file"
    );
    fx.click((1240., 145.))?;
    confirmation(&fx)?;
    fx.keys("enter")?;
    loaded(&mut fx)?;
    let remaining = base.replace("line 23\n", "last edit\n");
    let actual = fs::read_to_string(root.join("a.txt"))?;
    ensure!(
        actual == remaining,
        "hunk revert result {actual:?}; toasts: {:?}",
        fx.read(|root, cx| root.toasts(cx))
    );
    ensure!(git(root, &["show", ":a.txt"])? == base);
    toast(&fx, "Reverted hunk 1 in a.txt")?;
    fx.shot()?.save(scratch.join("after-hunk.png"))?;

    git(root, &["add", "a.txt"])?;
    source(&mut fx, "Staged")?;
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    fx.click((766., 453.))?;
    loaded(&mut fx)?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == base);
    ensure!(git(root, &["show", ":a.txt"])? == base);
    toast(&fx, "Reverted a.txt")?;

    fs::write(root.join("a.txt"), &edited)?;
    git(root, &["add", "a.txt"])?;
    source(&mut fx, "Staged")?;
    fx.click((1240., 145.))?;
    confirmation(&fx)?;
    fx.keys("enter")?;
    loaded(&mut fx)?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == remaining);
    ensure!(git(root, &["show", ":a.txt"])? == remaining);
    toast(&fx, "Reverted hunk 1 in a.txt")?;

    source(&mut fx, "Staged")?;
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    fs::write(root.join(".git/index.lock"), "owned proof lock")?;
    fx.keys("enter")?;
    loaded(&mut fx)?;
    toast(&fx, "Failed to revert a.txt")?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == remaining);
    ensure!(git(root, &["show", ":a.txt"])? == remaining);
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].name == "a.txt"),
        "failure removed the visible changes"
    );
    fx.shot()?.save(scratch.join("after-failure.png"))?;
    fs::remove_file(root.join(".git/index.lock"))?;

    source(&mut fx, "Staged")?;
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    let newer = remaining.replace("line 12\n", "newer worktree edit\n");
    fs::write(root.join("a.txt"), &newer)?;
    fx.keys("enter")?;
    loaded(&mut fx)?;
    ensure!(git(root, &["show", ":a.txt"])? == base);
    ensure!(fs::read_to_string(root.join("a.txt"))? == newer);
    toast(&fx, "Partially reverted a.txt")?;
    fx.shot()?.save(scratch.join("after-partial.png"))?;

    source(&mut fx, "Unstaged")?;
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    let stale = format!("{newer}later edit\n");
    fs::write(root.join("a.txt"), &stale)?;
    fx.keys("enter")?;
    loaded(&mut fx)?;
    toast(&fx, "Failed to revert a.txt")?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == stale);
    ensure!(git(root, &["show", ":a.txt"])? == base);

    source(&mut fx, "Unstaged")?;
    file_revert(&mut fx)?;
    confirmation(&fx)?;
    fx.keys("tab space shift-tab enter")?;
    loaded(&mut fx)?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == base);
    ensure!(
        Settings::load(settings_path.clone())?.read_all()["skip-revert-confirmation"]
            == json!(true)
    );
    drop(fx);
    fs::write(root.join("a.txt"), &edited)?;
    let mut fx = open(root, Settings::load(settings_path)?)?;
    file_revert(&mut fx)?;
    loaded(&mut fx)?;
    ensure!(
        fx.read(|root, _| root.dialog().is_none()),
        "saved preference did not skip confirmation"
    );
    ensure!(fs::read_to_string(root.join("a.txt"))? == base);
    ensure!(git(root, &["show", ":a.txt"])? == base);
    toast(&fx, "Reverted a.txt")?;
    println!(
        "PASS: native pointer and keyboard, Cancel and Escape, preference reload, file and hunk reverts, staged index and disk, lock and stale failures retain changes, index-only partial result"
    );
    Ok(())
}

#[test]
fn source_selection_and_section_actions_reach_the_real_repository() -> Result<()> {
    let scratch = std::env::var_os("CGX_GIT_REVIEW_UI_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-git-review-ui"));
    fs::create_dir_all(&scratch)?;
    let repo = tempfile::tempdir_in(&scratch)?;
    let root = repo.path();
    fs::write(root.join("a.txt"), "first file\n")?;
    fs::write(root.join("b.txt"), "second file\n")?;
    let mut fx = open(root, recorded_host_settings()?)?;
    source(&mut fx, "Last Turn")?;
    fx.click((1050., 110.))?;
    fx.click((735., 70.))?;
    fx.shot()?
        .save(scratch.join("native-source-last-turn.png"))?;
    choose(&mut fx, "Unstaged")?;
    loaded(&mut fx)?;
    fx.shot()?.save(scratch.join("native-create.png"))?;
    fx.click((965., 169.))?;
    loaded(&mut fx)?;
    ensure!(
        root.join(".git").is_dir(),
        "Create git repository did not initialize the folder"
    );
    ensure!(fs::read_to_string(root.join("a.txt"))? == "first file\n");
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native proof"])?;
    git(root, &["add", "a.txt", "b.txt"])?;
    git(root, &["commit", "-qm", "base"])?;
    git(root, &["branch", "-M", "main"])?;
    fs::write(root.join("a.txt"), "first edited\n")?;
    fs::write(root.join("b.txt"), "second edited\n")?;
    source(&mut fx, "Unstaged")?;
    fx.click((735., 70.))?;
    fx.shot()?.save(scratch.join("native-source.png"))?;
    choose(&mut fx, "Uncommitted")?;
    loaded(&mut fx)?;
    fx.shot()?.save(scratch.join("native-section.png"))?;
    fx.click((968., 798.))?;
    loaded(&mut fx)?;
    ensure!(git(root, &["show", ":a.txt"])? == "first edited\n");
    ensure!(git(root, &["show", ":b.txt"])? == "second edited\n");
    source(&mut fx, "Staged")?;
    fx.click((968., 798.))?;
    loaded(&mut fx)?;
    ensure!(git(root, &["show", ":a.txt"])? == "first file\n");
    ensure!(fs::read_to_string(root.join("a.txt"))? == "first edited\n");
    source(&mut fx, "Unstaged")?;
    fx.click((919., 798.))?;
    confirmation(&fx)?;
    fx.keys("escape")?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == "first edited\n");
    fx.click((919., 798.))?;
    confirmation(&fx)?;
    fx.keys("enter")?;
    loaded(&mut fx)?;
    ensure!(fs::read_to_string(root.join("a.txt"))? == "first file\n");
    ensure!(fs::read_to_string(root.join("b.txt"))? == "second file\n");
    fs::write(root.join("a.txt"), "staged but absent from the review\n")?;
    git(root, &["add", "a.txt"])?;
    fs::write(root.join("a.txt"), "first file\n")?;
    source(&mut fx, "Uncommitted")?;
    ensure!(fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files.is_empty()));
    fx.click((919., 798.))?;
    ensure!(
        fx.read(|root, _| root.dialog().is_none()),
        "Uncommitted offered a revert for a file absent from the review"
    );
    ensure!(fs::read_to_string(root.join("a.txt"))? == "first file\n");
    ensure!(git(root, &["show", ":a.txt"])? == "staged but absent from the review\n");
    source(&mut fx, "Branch")?;
    choose(&mut fx, "main")?;
    loaded(&mut fx)?;
    ensure!(fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files.is_empty()));
    source(&mut fx, "Committed")?;
    fx.keys("home enter")?;
    loaded(&mut fx)?;
    ensure!(fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .files
            .iter()
            .any(|f| f.name == "a.txt")
    }));
    fx.shot()?.save(scratch.join("native-commit.png"))?;
    println!(
        "PASS: native Create git repository, source choices, section stage/unstage, Revert all Cancel/Confirm, and real index/disk effects"
    );
    Ok(())
}

fn source_fixture() -> Result<Fx> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-b")?;
    let composer = fx.composer();
    composer.update(&mut fx.cx, |composer, cx| {
        composer.set_models(&[], &json!({}), cx)
    });
    Ok(fx)
}

#[test]
fn inline_code_wraps_across_lines() -> Result<()> {
    let mut fx = source_fixture()?;
    fx.click((1050., 110.))?;
    fx.click((735., 70.))?;
    let image = fx.reference_shot()?;
    let mut bands = 0;
    let mut in_band = false;
    for y in 600..708 {
        let background = (680..1320)
            .filter(|&x| image.get_pixel(x, y).0 == [48, 48, 48, 255])
            .count()
            >= 20;
        if background && !in_band {
            bands += 1;
        }
        in_band = background;
    }
    ensure!(
        bands == 2,
        "inline code has {bands} background rows, expected two"
    );
    if let Some(scratch) = std::env::var_os("CGX_DIFF_PARITY_EVIDENCE") {
        let scratch = std::path::PathBuf::from(scratch);
        fs::create_dir_all(&scratch)?;
        image.save(scratch.join("native.png"))?;
    }
    Ok(())
}

#[test]
fn missing_model_blocks_new_submission_but_preserves_running_actions() -> Result<()> {
    let mut fx = source_fixture()?;
    fx.click((500., 740.))?;
    fx.type_text("Do not send without a model.")?;
    let composer = fx.composer();
    let submissions = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = submissions.clone();
    composer.update(&mut fx.cx, |composer, cx| {
        composer.set_models(&[], &json!({}), cx);
        composer.on_submit(move |_, _, _| observed.set(observed.get() + 1));
    });
    fx.keys("enter")?;
    ensure!(submissions.get() == 0, "Enter submitted without a model");
    ensure!(
        fx.read(|_, cx| composer.read(cx).field_text() == "Do not send without a model."),
        "disabled submission discarded the draft"
    );
    fx.click((622., 790.))?;
    ensure!(
        submissions.get() == 0,
        "disabled Send accepted a submission"
    );
    composer.update(&mut fx.cx, |composer, cx| composer.set_running(true, cx));
    fx.click((500., 740.))?;
    fx.keys("enter")?;
    ensure!(
        submissions.get() == 1,
        "running-turn submission was blocked"
    );
    Ok(())
}

#[test]
fn control_opacity_matches_the_original_mac_renderer() -> Result<()> {
    let mut fx = source_fixture()?;
    fx.click((1050., 110.))?;
    fx.click((735., 70.))?;
    let image = fx.reference_shot()?;
    if let Some(scratch) = std::env::var_os("CGX_DIFF_PARITY_EVIDENCE") {
        let scratch = std::path::PathBuf::from(scratch);
        fs::create_dir_all(&scratch)?;
        image.save(scratch.join("alpha.png"))?;
    }
    // These device pixels sample the original Mac controls at 50%, 40%, and 75% opacity.
    for (x, y, level) in [
        (1238, 1552, 70),
        (908, 1573, 100),
        (1714, 195, 181),
        (1252, 1571, 153),
        (1247, 1572, 75),
    ] {
        let actual = image.get_pixel(x, y).0;
        ensure!(
            actual == [level, level, level, 255],
            "control opacity at ({x}, {y}): {actual:?}, expected {level}"
        );
    }
    Ok(())
}
