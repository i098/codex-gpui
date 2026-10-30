mod common;

use anyhow::Result;
use common::Fx;

const MODE_SWITCH: &str = "Switch mode, current mode: Codex";

fn mode(app: &Fx) -> usize {
    app.read(|root, cx| root.sidebar().read(cx).mode())
}

fn mode_menu(app: &Fx) -> bool {
    app.read(|root, cx| root.sidebar().read(cx).mode_menu_open())
}

#[test]
fn mode_switch_menu_picks_with_keys_and_click_outside_closes() -> Result<()> {
    let mut app = Fx::new()?;
    assert_eq!(mode(&app), 1, "Codex is the launch mode");
    let trigger = app.at("home", MODE_SWITCH)?;

    app.click(trigger)?;
    assert!(mode_menu(&app));
    app.capture_evidence("CGX_MODE_MENU_SHOTS", "mode-menu")?;
    // A pointer open highlights no row, so `down` reaches the first one: ChatGPT.
    app.keys("down enter")?;
    assert_eq!(mode(&app), 0);
    assert!(!mode_menu(&app));

    // Picking ChatGPT again on its Work home keeps that home.
    app.keys("ctrl-2")?;
    app.click(trigger)?;
    app.keys("down enter")?;
    assert_eq!(mode(&app), 2);
    app.keys("ctrl-1")?;

    app.click(trigger)?;
    assert!(mode_menu(&app));
    app.keys("escape")?;
    assert!(!mode_menu(&app));
    assert_eq!(mode(&app), 0);

    app.click(trigger)?;
    // A blank point of the page, outside the menu.
    app.click((1000., 150.))?;
    assert!(!mode_menu(&app));

    // Clicking the trigger again closes the menu instead of reopening it.
    app.click(trigger)?;
    app.click(trigger)?;
    assert!(!mode_menu(&app));
    Ok(())
}

#[test]
fn control_digits_switch_mode() -> Result<()> {
    let mut app = Fx::new()?;
    app.keys("ctrl-1")?;
    assert_eq!(mode(&app), 0);
    app.keys("ctrl-2")?;
    assert_eq!(mode(&app), 2);
    app.keys("ctrl-3")?;
    assert_eq!(mode(&app), 1);
    Ok(())
}

fn location_menu(app: &Fx) -> bool {
    app.read(|root, cx| root.home().read(cx).location.open)
}

#[test]
fn work_in_menu_toggles_and_dismisses() -> Result<()> {
    let mut app = Fx::new()?;
    let chip = app.at("home", "This computer")?;

    app.click(chip)?;
    assert!(location_menu(&app));
    app.keys("escape")?;
    assert!(!location_menu(&app));

    app.click(chip)?;
    app.click(chip)?;
    assert!(!location_menu(&app), "second click closes");

    app.click(chip)?;
    app.click((chip.0, chip.1 - 200.))?;
    assert!(!location_menu(&app), "outside click closes");
    Ok(())
}
