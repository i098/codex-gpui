#![cfg(target_os = "macos")]
mod common;

fn open_settings(fx: &mut common::Fx, section: &str) -> anyhow::Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text(section)?;
    let index = fx
        .read(|root, cx| {
            root.command_menu()
                .unwrap()
                .read(cx)
                .describe()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|(text, _)| {
                    text.split(" | ").next().unwrap().replace(['[', ']'], "") == section
                })
        })
        .expect("settings command");
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    assert_eq!(fx.route(), app::Route::Settings);
    fx.advance(std::time::Duration::from_millis(500))?;
    Ok(())
}

fn capture(
    fx: &mut common::Fx,
    out: &str,
    name: &str,
    original: Option<&str>,
) -> anyhow::Result<()> {
    let image = fx.reference_shot()?;
    image.save(format!("{out}/{name}.png"))?;
    if let (Some(original), Ok(refs)) = (original, std::env::var("CGX_GIT_REFS")) {
        let metrics = parity::diff_ref(
            std::path::Path::new(&refs),
            &std::path::Path::new(out).join("pairs"),
            original,
            "dark",
            &image,
        )?;
        std::fs::write(
            format!("{out}/{name}-counts.json"),
            serde_json::to_vec_pretty(&metrics)?,
        )?;
    }
    Ok(())
}

#[test]
fn git_settings_native() -> anyhow::Result<()> {
    use app::host_settings::Settings;
    use common::*;
    use serde_json::Value;
    use std::time::Duration;
    let out = std::env::var("CGX_GIT_EVIDENCE")
        .unwrap_or_else(|_| format!("/tmp/cgx-git-settings-{}", std::process::id()));
    std::fs::create_dir_all(&out)?;
    let path =
        std::path::Path::new(&out).join(format!("host-settings-{}.json", std::process::id()));
    let mut fx = Fx::new()?;
    fx.cx
        .update_window(fx.window, |_, window, _| window.activate_window())?;
    fx.cx
        .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
    open_settings(&mut fx, "Git")?;
    fx.scroll((200., 500.), -377.)?;
    fx.advance(Duration::from_millis(500))?;
    capture(&mut fx, &out, "git", Some("git"))?;
    fx.click((1040., 218.))?;
    fx.keys("cmd-a")?;
    fx.type_text("me/")?;
    fx.click((800., 140.))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-branch-prefix"], "me/");
    fx.click((1140., 609.))?;
    fx.click((800., 680.))?;
    fx.type_text("Wait for checks.")?;
    fx.advance(Duration::from_millis(3100))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-pr-watch-auto-merge"], true);
    assert_eq!(saved["git-pr-watch-instructions"], "Wait for checks.");
    fx.scroll((1050., 700.), -600.)?;
    fx.advance(Duration::from_millis(500))?;
    fx.click((800., 480.))?;
    fx.type_text("Use a short title.")?;
    fx.advance(Duration::from_millis(2000))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert!(saved.get("git-commit-instructions").is_none());
    fx.keys("enter")?;
    fx.type_text("Explain the change.")?;
    fx.advance(Duration::from_millis(2900))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert!(saved.get("git-commit-instructions").is_none());
    fx.advance(Duration::from_millis(200))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(
        saved["git-commit-instructions"],
        "Use a short title.\nExplain the change."
    );
    let previous = std::fs::read(&path)?;
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    fx.click((800., 710.))?;
    fx.type_text("Explain the test plan.")?;
    fx.advance(Duration::from_millis(3100))?;
    assert!(!fx.cx.update(|cx| {
        cx.global::<Settings>()
            .read_all()
            .contains_key("git-pr-instructions")
    }));
    std::fs::remove_dir(&path)?;
    std::fs::write(&path, previous)?;
    fx.keys("cmd-s")?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-pr-instructions"], "Explain the test plan.");
    fx.advance(Duration::from_millis(3500))?;
    capture(&mut fx, &out, "git-instructions", None)?;
    fx.click(BACK)?;
    fx.cx
        .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
    open_settings(&mut fx, "Git")?;
    fx.click((800., 680.))?;
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some("Wait for checks.")
    );
    fx.keys("cmd-a")?;
    fx.type_text("Keep watching.")?;
    fx.cx.update(|cx| cx.shutdown());
    fx.cx.run_until_parked();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-pr-watch-instructions"], "Keep watching.");
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn worktree_preferences_require_confirmation_and_preserve_the_saved_limit() -> anyhow::Result<()> {
    use anyhow::Context as _;
    use app::host_settings::Settings;
    use common::*;
    use serde_json::{Value, json};
    use std::time::Duration;
    use ui::screens::{
        computer_use::Load,
        settings::{environments, worktrees},
    };
    let out = std::env::var("CGX_GIT_EVIDENCE")
        .unwrap_or_else(|_| format!("/tmp/cgx-git-settings-{}", std::process::id()));
    std::fs::create_dir_all(&out)?;
    let path =
        std::path::Path::new(&out).join(format!("worktree-settings-{}.json", std::process::id()));
    let mut fx = Fx::new()?;
    fx.cx
        .update_window(fx.window, |_, window, _| window.activate_window())?;
    fx.cx.update(|cx| {
        cx.set_global(Settings::load(path.clone()).unwrap());
        cx.set_global(worktrees::State {
            default_root: "/tmp/cgx-mac/home/worktrees".into(),
            worktrees: Load::Loading,
            ..Default::default()
        });
        cx.set_global(environments::State {
            projects: Load::Ready(Vec::new().into()),
            ..Default::default()
        });
    });
    open_settings(&mut fx, "Worktrees")?;
    fx.scroll((200., 500.), -408.)?;
    fx.advance(Duration::from_millis(500))?;
    capture(&mut fx, &out, "worktrees-loading", Some("worktrees"))?;
    fx.click((560., 199.))?;
    capture(&mut fx, &out, "environments-empty", Some("environments"))?;
    fx.click((478., 225.))?;
    fx.advance(Duration::ZERO)?;
    fx.click((1140., 434.))?;
    capture(&mut fx, &out, "disable-cleanup", Some("disable-cleanup"))?;
    assert_ne!(
        fx.cx.update(|cx| cx
            .global::<Settings>()
            .read_all()
            .get("worktree-auto-cleanup-enabled")
            .cloned()),
        Some(json!(false))
    );
    fx.keys("escape")?;
    fx.advance(Duration::ZERO)?;
    fx.click((1090., 519.))?;
    fx.advance(Duration::ZERO)?;
    fx.keys("cmd-a")?;
    fx.type_text("7")?;
    fx.keys("enter")?;
    fx.advance(Duration::ZERO)?;
    let saved: Value = serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("read saved worktree limit at {}", path.display()))?,
    )?;
    assert_eq!(saved["worktree-keep-count"], 7);
    fx.click((1140., 434.))?;
    fx.advance(Duration::ZERO)?;
    fx.keys("tab enter")?;
    fx.advance(Duration::ZERO)?;
    let saved: Value = serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("read disabled cleanup setting at {}", path.display()))?,
    )?;
    assert_eq!(saved["worktree-auto-cleanup-enabled"], false);
    assert_eq!(saved["worktree-keep-count"], 7);
    capture(&mut fx, &out, "cleanup-disabled", None)?;
    std::fs::remove_file(path)?;
    Ok(())
}
