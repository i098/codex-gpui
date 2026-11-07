//! Up in an empty composer recalls the last sent prompt, as the original's `prompt-history`.

mod common;

use anyhow::{Result, ensure};
use common::{Fx, script};
use gpui::Focusable as _;

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn up_recalls_the_sent_prompt() -> Result<()> {
    common::skip_unless_macos!("prompt_history");
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    composer.update(&mut fx.cx, |c, _| c.on_submit(|_, _, _| {}));
    fx.type_text("Summarize the repo")?;
    fx.keys("enter")?;
    ensure!(fx.composer_text()?.is_empty(), "the prompt was not sent");
    fx.keys("up")?;
    fx.capture_evidence("CGX_PROMPT_HISTORY_EVIDENCE", "prompt-recalled")?;
    ensure!(
        fx.composer_text()? == "Summarize the repo",
        "Up did not recall the sent prompt"
    );
    fx.keys("down")?;
    ensure!(
        fx.composer_text()?.is_empty(),
        "Down did not clear the recall"
    );
    Ok(())
}
