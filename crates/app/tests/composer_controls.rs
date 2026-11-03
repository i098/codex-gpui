//! Composer footer focus order and tooltips, from `refs/interactions/f1c.json` steps 29-39
//! and the original's `composer.*.tooltip` messages. f1c checks the ring pixels on macOS.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use gpui::PlatformWindow as _;
use std::time::Duration;

/// The footer controls in Tab order: `aria-label` in `refs/home`, tooltip, and shortcut.
const CONTROLS: [(&str, &str, Option<&str>); 5] = [
    ("Add files and more", "Add files and more", Some("@")),
    ("Change permissions", "Change permissions", None),
    ("GPT-6 Astra", "Select effort", Some("⌃⇧M")),
    ("Dictate", "Dictate", None),
    ("Start new voice chat", "Start new voice chat", None),
];

fn shown(fx: &mut Fx) -> Option<(String, Option<String>, f32, f32)> {
    fx.cx.update(|cx| {
        ui::tooltip::shown(cx).map(|tip| {
            (
                tip.label.to_string(),
                tip.shortcut.as_ref().map(|s| s.to_string()),
                f32::from(tip.bounds.center().x),
                f32::from(tip.bounds.bottom()),
            )
        })
    })
}

fn menu_open(fx: &mut Fx) -> Result<bool> {
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.read(cx).menu_state(window).0
    })
}

/// Presses `key`, then renders the focus event and the tooltip frame it requests.
fn press(fx: &mut Fx, key: &str) -> Result<()> {
    fx.keys(key)?;
    fx.advance(Duration::ZERO)?;
    fx.advance(Duration::ZERO)
}

#[test]
fn tab_reaches_each_footer_control_and_opens_its_tooltip() -> Result<()> {
    let mut fx = Fx::new()?;
    // TestWindow has no OS window; focus events need its simulated active status.
    fx.cx.test_window(fx.window).activate();
    fx.click(fx.at("home", "Do anything")?)?;
    ensure!(fx.composer_focused()?, "the click did not focus the editor");
    for (step, (control, label, shortcut)) in CONTROLS.iter().enumerate() {
        press(&mut fx, "tab")?;
        let (got, keys, x, bottom) =
            shown(&mut fx).with_context(|| format!("Tab {step}: no tooltip on {control}"))?;
        ensure!(
            got == *label && keys.as_deref() == *shortcut,
            "Tab {step}: {control} shows {got:?} {keys:?}"
        );
        let (cx, cy) = fx.at("home", control)?;
        ensure!(
            (x - cx).abs() <= 40. && bottom <= cy - 14.,
            "Tab {step}: {control} tooltip at x {x}, bottom {bottom}"
        );
    }
    for (step, (control, label, _)) in CONTROLS.iter().rev().skip(1).enumerate() {
        press(&mut fx, "shift-tab")?;
        let got = shown(&mut fx).map(|tip| tip.0);
        ensure!(
            got.as_deref() == Some(*label),
            "Shift-Tab {step}: expected {control}, tooltip {got:?}"
        );
    }
    press(&mut fx, "shift-tab")?;
    ensure!(
        fx.composer_focused()?,
        "Shift-Tab did not return to the editor"
    );
    ensure!(shown(&mut fx).is_none(), "the editor kept a tooltip");
    Ok(())
}

#[test]
fn hover_opens_each_footer_tooltip_after_the_delay() -> Result<()> {
    for (control, label, shortcut) in CONTROLS {
        let mut fx = Fx::new()?;
        fx.hover(fx.at("home", control)?)?;
        fx.advance(Duration::from_millis(199))?;
        ensure!(shown(&mut fx).is_none(), "{control}: opened before 200 ms");
        fx.advance(Duration::from_millis(1))?;
        let (got, keys, ..) = shown(&mut fx).with_context(|| format!("{control}: no tooltip"))?;
        ensure!(
            got == label && keys.as_deref() == shortcut,
            "{control}: {got:?} {keys:?}"
        );
        fx.hover((640., 300.))?;
        ensure!(
            shown(&mut fx).is_none(),
            "{control}: leave kept the tooltip"
        );
    }
    Ok(())
}

#[test]
fn menu_triggers_close_on_a_second_click_without_a_tooltip() -> Result<()> {
    for control in ["Change permissions", "GPT-6 Astra"] {
        let mut fx = Fx::new()?;
        let at = fx.at("home", control)?;
        fx.click(at)?;
        ensure!(menu_open(&mut fx)?, "{control}: the click opened no menu");
        fx.advance(Duration::from_millis(300))?;
        ensure!(
            shown(&mut fx).is_none(),
            "{control}: tooltip over the open menu"
        );
        fx.click(at)?;
        ensure!(
            !menu_open(&mut fx)?,
            "{control}: the second click reopened the menu"
        );
    }
    Ok(())
}
