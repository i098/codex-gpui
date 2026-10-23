#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use common::Fx;

#[test]
fn edit_menu_preserves_unselected_text_and_plain_paste_is_undoable() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.focus_composer()?;
    fx.composer()
        .update(&mut fx.cx, |composer, cx| composer.set_text("keep π", cx));
    fx.keys("cmd-left")?;
    fx.menu("Edit", "Delete")?;
    assert_eq!(fx.composer_text()?, "keep π");

    fx.keys("cmd-a")?;
    fx.menu("Edit", "Delete")?;
    assert_eq!(fx.composer_text()?, "");
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "keep π");

    fx.cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("replacement λ".into()));
    });
    fx.menu("Edit", "Paste and Match Style")?;
    assert_eq!(fx.composer_text()?, "replacement λ");
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "keep π");
    fx.keys("cmd-alt-shift-v")?;
    assert_eq!(fx.composer_text()?, "replacement λ");
    fx.keys("cmd-z")?;
    fx.keys("cmd-shift-v")?;
    assert_eq!(fx.composer_text()?, "replacement λ");
    Ok(())
}

#[test]
fn full_screen_shortcut_toggles_the_current_window() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.cx
        .update_window(fx.window, |_, window, _| window.activate_window())?;
    let full_screen = |fx: &mut Fx| {
        fx.cx
            .update_window(fx.window, |_, window, _| window.is_fullscreen())
    };
    assert!(!full_screen(&mut fx)?);
    fx.keys("ctrl-cmd-f")?;
    assert!(full_screen(&mut fx)?);
    fx.keys("ctrl-cmd-f")?;
    assert!(!full_screen(&mut fx)?);
    Ok(())
}

#[test]
fn about_reuses_its_window_and_escape_closes_only_about() -> Result<()> {
    let mut fx = Fx::new()?;
    let before = fx.cx.update(|cx| {
        cx.windows()
            .into_iter()
            .map(|window| window.window_id())
            .collect::<std::collections::HashSet<_>>()
    });
    fx.menu("Codex", "About Codex")?;
    let about = fx
        .cx
        .update(|cx| {
            cx.windows()
                .into_iter()
                .find(|window| !before.contains(&window.window_id()))
        })
        .expect("About window");
    let mut expected = before.clone();
    expected.insert(about.window_id());
    fx.menu("Codex", "About Codex")?;
    assert_eq!(
        fx.cx.update(|cx| {
            cx.windows()
                .into_iter()
                .map(|window| window.window_id())
                .collect::<std::collections::HashSet<_>>()
        }),
        expected
    );
    fx.cx.update_window(about, |_, window, cx| {
        window.dispatch_keystroke(gpui::Keystroke::parse("escape").expect("Escape"), cx);
    })?;
    fx.cx.run_until_parked();
    assert_eq!(
        fx.cx.update(|cx| {
            cx.windows()
                .into_iter()
                .map(|window| window.window_id())
                .collect::<std::collections::HashSet<_>>()
        }),
        before
    );
    Ok(())
}

#[test]
fn control_command_f_dispatches_the_full_screen_action() -> Result<()> {
    let mut fx = Fx::new()?;
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let received = calls.clone();
    fx.cx.update(|cx| {
        cx.on_action(move |_: &app::native_menus::ToggleFullScreen, cx| {
            received.set(received.get() + 1);
            cx.propagate();
        });
    });
    fx.keys("ctrl-f")?;
    assert_eq!(calls.get(), 0);
    fx.keys("ctrl-cmd-f")?;
    assert_eq!(calls.get(), 1);
    Ok(())
}

#[test]
fn summary_menu_tracks_the_active_thread_and_toggles_from_composer_focus() -> Result<()> {
    let mut fx = Fx::new()?;
    let target = fx.at("home", "Add a subtract")?;
    fx.click(target)?;
    fx.wait_thread("01a13de9-6fff-78e1-84fc-ebb3a2dc873f")?;
    let thread = fx
        .read(|root, _| root.thread().cloned())
        .expect("open thread");
    fx.focus_composer()?;
    let available = |fx: &mut Fx| {
        fx.cx.update_window(fx.window, |_, window, cx| {
            window.is_action_available(&app::native_menus::TogglePinnedSummary, cx)
        })
    };

    assert!(available(&mut fx)?);
    fx.menu("View", "Toggle Pinned Summary")?;
    assert!(fx.cx.update(|cx| thread.read(cx).summary_open));
    fx.menu("View", "Toggle Pinned Summary")?;
    assert!(!fx.cx.update(|cx| thread.read(cx).summary_open));

    let previous_route = fx.route();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(app::NextChat), cx);
    })?;
    assert_ne!(fx.route(), previous_route);
    assert_eq!(
        fx.thread_id().as_deref(),
        Some("01a13de9-6fff-78e1-84fc-ebb3a2dc873f")
    );
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(app::native_menus::TogglePinnedSummary), cx);
        window.draw(cx).clear(cx);
    })?;
    assert!(!fx.cx.update(|cx| thread.read(cx).summary_open));
    assert!(!available(&mut fx)?);

    fx.keys("cmd-n")?;
    assert!(!available(&mut fx)?);
    fx.menu("View", "Toggle Pinned Summary")?;
    assert!(!fx.cx.update(|cx| thread.read(cx).summary_open));
    Ok(())
}

#[test]
fn workspace_navigation_reads_files_without_changing_them() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let root = workspace.path().to_string_lossy().into_owned();
    std::fs::create_dir(workspace.path().join("src"))?;
    let math = workspace.path().join("src/math.py");
    std::fs::write(&math, "def subtract(a, b):\n    return a - b\n")?;
    let expected = std::fs::read(&math)?;
    let script = common::script(None, |_| {})?
        .into_iter()
        .map(|line| serde_json::from_str(&line.to_string().replace(common::CWD, &root)))
        .collect::<serde_json::Result<Vec<_>>>()?;
    let mut fx = Fx::scripted(script)?;
    fx.action(Box::new(app::GoToRecentChat(1)))?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.action(Box::new(ui::screens::diff::panel::OpenFile {
        path: math.to_string_lossy().into_owned(),
        line: None,
    }))?;
    fx.wait("the file read", |fx| {
        fx.read(|root, cx| {
            root.file_view()
                .is_some_and(|file| file.read(cx).contents.as_ref().is_some_and(Result::is_ok))
        })
    })?;
    let file = fx.read(|root, _| root.file_view().cloned()).unwrap();
    fx.menu("View", "Toggle navigation panel")?;
    fx.wait("the workspace root", |fx| {
        fx.read(|_, cx| file.read(cx).visible_file_paths().any(|path| path == "src"))
    })?;
    assert!(!fx.read(|_, cx| {
        file.read(cx)
            .visible_file_paths()
            .any(|path| path == "src/math.py")
    }));
    fx.cx.update_window(fx.window, |_, window, cx| {
        file.update(cx, |file, cx| file.focus_file_tree(window, cx));
    })?;
    fx.keys("home enter")?;
    fx.wait("the expanded workspace directory", |fx| {
        fx.read(|_, cx| {
            file.read(cx)
                .visible_file_paths()
                .any(|path| path == "src/math.py")
        })
    })?;
    assert!(!fx.read(|_, cx| file.read(cx).is_preview()));
    fx.menu("View", "Toggle navigation panel")?;
    assert!(!fx.read(|_, cx| file.read(cx).file_tree_open()));
    fx.menu("View", "Toggle navigation panel")?;
    assert!(fx.read(|_, cx| file.read(cx).file_tree_open()));
    let opened = fx.read(|_, cx| file.read(cx).path.clone());
    for path in [
        "../outside.txt".to_owned(),
        workspace.path().join("outside.txt").display().to_string(),
    ] {
        file.update(&mut fx.cx, |_, cx| {
            cx.emit(ui::screens::diff::panel::FileTreeRequest::Open(path));
        });
        fx.cx.run_until_parked();
        assert_eq!(fx.read(|_, cx| file.read(cx).path.clone()), opened);
    }
    assert_eq!(std::fs::read(math)?, expected);
    Ok(())
}
