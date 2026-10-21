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
