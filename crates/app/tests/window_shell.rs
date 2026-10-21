#![cfg(target_os = "macos")]
mod common;

use anyhow::Result;
use common::*;
use std::time::Duration;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn fullscreen_moves_history_controls_and_restores_them_on_exit() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.cx.update_window(fx.window, |_, window, _| {
        window.toggle_fullscreen();
        window.refresh();
    })?;
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "fullscreen-rest")?;
    fx.cx.update_window(fx.window, |_, window, _| {
        window.toggle_fullscreen();
        window.refresh();
    })?;
    fx.keys("cmd-,")?;
    assert_eq!(fx.route(), app::Route::Settings);
    fx.cx.update_window(fx.window, |_, window, _| {
        window.toggle_fullscreen();
        window.refresh();
    })?;
    fx.advance(Duration::ZERO)?;
    fx.click((18., 22.))?;
    assert_eq!(fx.route(), app::Route::Home);
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "fullscreen")?;
    fx.cx.update_window(fx.window, |_, window, _| {
        window.toggle_fullscreen();
        window.refresh();
    })?;
    fx.advance(Duration::ZERO)?;
    fx.click(FORWARD)?;
    assert_eq!(fx.route(), app::Route::Settings);
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "windowed-restored")?;
    Ok(())
}

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
    fx.hover((140., 200.))?;
    fx.advance(Duration::from_millis(200))?;
    assert!(fx.shell(|shell| shell.is_peeking()));
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "sidebar-peek")?;
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
fn a_narrow_sidebar_floats_and_an_outside_press_closes_it() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.resize(600., 832.)?;
    assert!(!fx.sidebar_open());
    fx.keys("cmd-b")?;
    fx.advance(Duration::from_millis(350))?;
    assert!(fx.sidebar_open());
    fx.capture_evidence("CGX_WINDOW_SHELL_PROOF", "narrow-sidebar")?;
    fx.click((500., 100.))?;
    fx.advance(Duration::from_millis(350))?;
    assert!(!fx.sidebar_open());
    fx.resize(1280., 832.)?;
    assert!(
        fx.sidebar_open(),
        "the narrow overlay must not replace the wide-window preference"
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
