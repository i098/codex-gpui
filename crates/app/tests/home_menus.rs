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
    app.keys("up enter")?;
    assert_eq!(mode(&app), 0);
    assert!(!mode_menu(&app));

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

fn project_menu(app: &Fx) -> bool {
    app.read(|root, cx| root.home().read(cx).projects.open)
}

fn project(app: &Fx) -> Option<String> {
    app.read(|root, cx| root.home().read(cx).project.as_ref().map(|p| p.to_string()))
}

#[test]
fn choose_project_menu_searches_picks_and_clears() -> Result<()> {
    let mut app = Fx::new()?;
    let composer = app.read(|root, cx| root.home().read(cx).composer.clone());
    app.cx.update(|cx| {
        composer.update(cx, |composer, _| {
            composer.set_slash_projects(vec![
                ("alpha".into(), "/tmp/alpha".into()),
                ("beta".into(), "/tmp/beta".into()),
            ])
        })
    });
    let chip = app.at("home", "Choose project")?;

    app.click(chip)?;
    assert!(project_menu(&app));
    app.keys("escape")?;
    assert!(!project_menu(&app));

    // The search field has focus: a query without matches leaves Enter with nothing to pick.
    app.click(chip)?;
    app.type_text("zzz")?;
    app.keys("enter")?;
    assert_eq!(project(&app), None);
    assert!(
        project_menu(&app),
        "Enter without a match keeps the menu open"
    );
    app.keys("escape")?;
    assert!(!project_menu(&app));

    // Reopening clears the query; Enter picks the first match.
    app.click(chip)?;
    app.type_text("bet")?;
    app.keys("enter")?;
    assert!(!project_menu(&app));
    assert_eq!(project(&app).as_deref(), Some("beta"));

    app.action(Box::new(ui::screens::home::ClearProject))?;
    assert_eq!(project(&app), None);
    Ok(())
}
