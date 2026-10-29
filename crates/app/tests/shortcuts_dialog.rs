mod common;

use anyhow::{Result, ensure};
use common::Fx;
use std::time::Duration;
use ui::screens::settings::keyboard_shortcuts::{self as keyboard, Change, KeyboardState};

fn change(fx: &mut Fx, change: Change) -> Result<()> {
    fx.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(change, cx);
    });
    fx.cx.run_until_parked();
    fx.advance(Duration::ZERO)
}

fn titles(fx: &Fx) -> Vec<&'static str> {
    fx.read(|root, cx| root.dialog().unwrap().read(cx).visible_titles())
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    let image = fx.shot()?;
    if let Some(directory) = std::env::var_os("CGX_SHORTCUT_PROOF") {
        std::fs::create_dir_all(&directory)?;
        image.save(std::path::PathBuf::from(directory).join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn the_dialog_shows_only_recorded_commands_that_the_app_binds() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    capture(&mut fx, "shortcuts-open")?;
    let recording: serde_json::Value = serde_json::from_slice(&std::fs::read(format!(
        "{}/refs/interactions/f3b.json",
        common::ROOT
    ))?)?;
    let active = fx
        .cx
        .update(|cx| cx.global::<KeyboardState>().active_commands.clone());
    let bound: Vec<_> = keyboard::commands()
        .iter()
        .filter(|command| active.contains(command.id))
        .map(|command| command.dialog_title())
        .collect();
    let mut expected: Vec<_> = recording["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|section| section["rows"].as_array().unwrap())
        .filter_map(|row| row["title"].as_str())
        .filter(|title| bound.contains(title))
        .collect();
    let mut shown = titles(&fx);
    shown.sort_unstable();
    expected.sort_unstable();
    ensure!(
        shown == expected,
        "dialog {shown:?} != recorded and bound {expected:?}"
    );
    Ok(())
}

#[test]
fn the_open_dialog_follows_persisted_binding_changes() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    fx.type_text("archive")?;
    ensure!(titles(&fx) == ["Archive chat"]);
    change(
        &mut fx,
        Change::Bindings {
            command: "archiveThread".into(),
            keys: Vec::new(),
        },
    )?;
    capture(&mut fx, "archive-unbound")?;
    ensure!(
        titles(&fx).is_empty(),
        "an unbound command remains in the open dialog"
    );
    change(&mut fx, Change::ResetCommand("archiveThread".into()))?;
    capture(&mut fx, "archive-restored")?;
    ensure!(
        titles(&fx) == ["Archive chat"],
        "reset did not restore the open dialog row"
    );
    Ok(())
}

#[test]
fn dialog_search_matches_command_ids_descriptions_and_fuzzy_titles() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    for (query, expected) in [
        ("  ARCHIVETHREAD  ", vec!["Archive chat"]),
        ("outside of any project", vec!["New standalone chat"]),
        ("arch*ve chat", vec!["Archive chat"]),
        ("zzzz-no-shortcut", vec![]),
    ] {
        fx.keys("cmd-a")?;
        fx.type_text(query)?;
        ensure!(
            titles(&fx) == expected,
            "query {query:?}: {:?}",
            titles(&fx)
        );
    }
    Ok(())
}

#[test]
fn home_dialog_lists_the_original_labels_for_layout_and_copy_rows() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    let shown = titles(&fx);
    for title in [
        "New tab in full view",
        "New tab",
        "Cycle workspace layout",
        "Copy conversation path",
        "Copy working directory",
    ] {
        ensure!(shown.contains(&title), "{title} missing: {shown:?}");
    }
    ensure!(!shown.contains(&"Full view") && !shown.contains(&"Switch between Chat and tabs"));
    Ok(())
}
