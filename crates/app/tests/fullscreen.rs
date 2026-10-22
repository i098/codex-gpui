#![cfg(target_os = "macos")]
mod common;

use anyhow::Result;
use common::{FORWARD, Fx};
use std::time::Duration;

#[test]
fn fullscreen_controls_move_and_restore_without_losing_history() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.cx.update_window(fx.window, |_, window, _| {
        window.toggle_fullscreen();
        window.refresh();
    })?;
    fx.capture_evidence("CGX_FULLSCREEN_PROOF", "fullscreen-rest")?;
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
    fx.click((22., 22.))?;
    assert_eq!(fx.route(), app::Route::Home);
    fx.capture_evidence("CGX_FULLSCREEN_PROOF", "fullscreen")?;
    fx.cx.update_window(fx.window, |_, window, _| {
        window.toggle_fullscreen();
        window.refresh();
    })?;
    fx.advance(Duration::ZERO)?;
    fx.click(FORWARD)?;
    assert_eq!(fx.route(), app::Route::Settings);
    fx.capture_evidence("CGX_FULLSCREEN_PROOF", "windowed-restored")?;
    Ok(())
}
