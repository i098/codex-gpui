#![cfg(target_os = "macos")]
mod common;

fn open_git(fx: &mut common::Fx) -> anyhow::Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text("Git")?;
    let index = fx
        .read(|root, cx| {
            root.command_menu()
                .unwrap()
                .read(cx)
                .describe()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|(text, _)| {
                    text.split(" | ").next().unwrap().replace(['[', ']'], "") == "Git"
                })
        })
        .expect("Git command");
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    assert_eq!(fx.route(), app::Route::Settings);
    fx.advance(std::time::Duration::from_millis(500))?;
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
    fx.cx.run_until_parked();
    fx.cx
        .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
    open_git(&mut fx)?;
    fx.click((1040., 330.))?;
    fx.keys("cmd-a")?;
    fx.type_text("me/")?;
    fx.keys("enter")?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-branch-prefix"], "me/");
    fx.click((1140., 466.))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-always-force-push"], true);
    fx.click((1140., 265.))?;
    fx.click((1140., 519.))?;
    fx.click((1110., 410.))?;
    fx.shot()?.save(format!("{out}/merge-menu.png"))?;
    fx.click((1090., 475.))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-review-mode"], "last-turn-only");
    assert_eq!(saved["git-create-pull-request-as-draft"], false);
    assert_eq!(saved["git-pull-request-merge-method"], "squash");
    fx.click((800., 710.))?;
    fx.type_text("Use a short title.")?;
    fx.keys("enter")?;
    fx.type_text("Explain the change.")?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(
        saved["git-commit-instructions"],
        "Use a short title.\nExplain the change."
    );
    fx.click(BACK)?;
    fx.cx
        .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
    open_git(&mut fx)?;
    fx.click((1040., 330.))?;
    fx.keys("cmd-a cmd-c")?;
    let displayed = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(displayed.as_deref(), Some("me/"));
    fx.click((800., 710.))?;
    fx.keys("cmd-a cmd-c")?;
    let displayed = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(
        displayed.as_deref(),
        Some("Use a short title.\nExplain the change.")
    );
    fx.click((800., 140.))?;
    fx.advance(Duration::from_millis(5500))?;
    fx.shot()?.save(format!("{out}/after.png"))?;
    fx.scroll((1050., 700.), -700.)?;
    fx.advance(Duration::from_millis(500))?;
    let previous = std::fs::read(&path)?;
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    fx.click((800., 425.))?;
    fx.type_text("Explain the test plan.")?;
    assert!(path.is_dir());
    assert!(!fx.cx.update(|cx| {
        cx.global::<Settings>()
            .read_all()
            .contains_key("git-pr-instructions")
    }));
    std::fs::remove_dir(&path)?;
    std::fs::write(&path, previous)?;
    fx.click((1128., 494.))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-pr-instructions"], "Explain the test plan.");
    fx.click((1140., 643.))?;
    fx.click((800., 710.))?;
    fx.type_text("Wait for checks.")?;
    let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["git-pr-watch-auto-merge"], true);
    assert_eq!(saved["git-pr-watch-instructions"], "Wait for checks.");
    fx.advance(Duration::from_millis(5500))?;
    fx.shot()?.save(format!("{out}/after-instructions.png"))?;
    std::fs::copy(&path, std::path::Path::new(&out).join("host-settings.json"))?;
    std::fs::remove_file(path)?;
    println!("Git settings: native edits persisted and reopened fields read back from disk.");
    Ok(())
}
