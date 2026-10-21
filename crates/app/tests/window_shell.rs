#![cfg(target_os = "macos")]
mod common;

use anyhow::Result;
use common::*;
use std::time::Duration;

#[test]
fn sidebar_peek_waits_for_hover_and_does_not_navigate() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-b")?;
    fx.advance(Duration::from_millis(350))?;
    fx.hover(RAIL_HOME)?;
    fx.advance(Duration::from_millis(99))?;
    assert!(!fx.shell(|shell| shell.is_peeking()));
    fx.advance(Duration::from_millis(2))?;
    assert!(fx.shell(|shell| shell.is_peeking()));
    assert_eq!(fx.route(), app::Route::Home);
    fx.hover((140., 600.))?;
    fx.advance(Duration::from_millis(200))?;
    assert!(fx.shell(|shell| shell.is_peeking()));
    if let Some(directory) = std::env::var_os("CGX_WINDOW_SHELL_PROOF") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory)?;
        fx.frozen_shot()?.save(directory.join("sidebar-peek.png"))?;
    }
    fx.hover((800., 200.))?;
    fx.advance(Duration::from_millis(99))?;
    assert!(fx.shell(|shell| shell.is_peeking()));
    fx.advance(Duration::from_millis(2))?;
    assert!(!fx.shell(|shell| shell.is_peeking()));
    fx.hover(RAIL_HOME)?;
    fx.advance(Duration::from_millis(101))?;
    assert!(fx.shell(|shell| shell.is_peeking()));
    fx.keys("escape")?;
    assert!(!fx.shell(|shell| shell.is_peeking()));
    assert!(!fx.sidebar_open());
    Ok(())
}

#[test]
fn clicking_the_new_tab_trigger_keeps_composer_focus() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.focus_composer()?;
    fx.hover((1258., 22.))?;
    fx.advance(Duration::from_millis(350))?;
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "new-tab")?;
    fx.click((1258., 22.))?;
    assert!(fx.composer_focused()?);
    fx.type_text("The draft keeps focus")?;
    assert_eq!(fx.composer_text()?, "The draft keeps focus");
    Ok(())
}

#[test]
fn a_narrow_sidebar_opens_docked_and_preserves_the_wide_preference() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.resize(600., 832.)?;
    assert!(!fx.sidebar_open());
    fx.keys("cmd-b")?;
    fx.advance(Duration::from_millis(350))?;
    assert!(fx.sidebar_open());
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "narrow-sidebar")?;
    fx.click((500., 100.))?;
    fx.advance(Duration::from_millis(350))?;
    assert!(
        fx.sidebar_open(),
        "a docked sidebar must stay open after an outside press"
    );
    fx.keys("cmd-b")?;
    assert!(!fx.sidebar_open());
    fx.resize(1280., 832.)?;
    assert!(
        fx.sidebar_open(),
        "the narrow layout must not replace the wide-window preference"
    );
    Ok(())
}

#[test]
fn close_shortcut_uses_panel_focus_instead_of_panel_visibility() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.action(Box::new(app::GoToChat(0)))?;
    fx.wait("thread", |fx| fx.thread_id().is_some())?;
    fx.action(Box::new(ui::OpenChangesPanel))?;
    fx.wait("changes", |fx| {
        fx.read(|root, _| root.side_panel().is_some())
    })?;
    fx.focus_composer()?;
    fx.keys("cmd-w")?;
    assert!(fx.read(|root, _| root.dialog().is_some()));
    assert!(fx.read(|root, _| root.side_panel().is_some()));
    fx.keys("escape")?;
    fx.click((1100., 600.))?;
    fx.keys("cmd-w")?;
    assert!(fx.read(|root, _| root.side_panel().is_none()));
    assert!(fx.read(|root, _| root.dialog().is_none()));
    Ok(())
}
