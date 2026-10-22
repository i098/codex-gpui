mod common;

use anyhow::{Result, ensure};
use common::Fx;
use std::time::Duration;
use ui::keyboard::{Change, KeyboardState};

fn change(fx: &mut Fx, change: Change) -> Result<()> {
    fx.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(change, cx);
    });
    fx.cx.run_until_parked();
    // The dialog observes the store on the next frame.
    fx.advance(Duration::ZERO)
}

fn titles(fx: &Fx) -> Vec<&'static str> {
    fx.read(|root, cx| root.dialog().unwrap().read(cx).visible_titles())
}

/// The recorded original dialog rows (`refs/interactions/f3b.json`) whose command this app binds,
/// in recorded order. The original shows a webview command only while a handler is registered;
/// the five commands it leaves out of its Home and thread states (`temporaryChat`,
/// `openReviewTab`, `toggleSidePanel`, `undoAppAction`, `redoAppAction`) are not in the catalog.
fn expected_titles(fx: &mut Fx) -> Result<Vec<String>> {
    let recording: serde_json::Value = serde_json::from_slice(&std::fs::read(format!(
        "{}/refs/interactions/f3b.json",
        env!("CARGO_MANIFEST_DIR").replace("/crates/app", "")
    ))?)?;
    let active = fx
        .cx
        .update(|cx| cx.global::<KeyboardState>().active_commands.clone());
    let bound: Vec<&str> = ui::keyboard::commands()
        .iter()
        .filter(|c| active.contains(c.id))
        .map(ui::keyboard::dialog_title)
        .collect();
    Ok(recording["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|section| section["rows"].as_array().unwrap())
        .filter_map(|row| row["title"].as_str())
        .filter(|title| bound.contains(title))
        .map(str::to_owned)
        .collect())
}

#[test]
fn the_dialog_shows_every_recorded_row_the_app_binds_and_nothing_else() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    let expected = expected_titles(&mut fx)?;
    ensure!(
        expected.len() > 40,
        "the recorded catalog must map to bound commands"
    );
    let shown: Vec<String> = titles(&fx).into_iter().map(str::to_owned).collect();
    ensure!(
        shown == expected,
        "dialog {shown:?} != recorded-and-bound {expected:?}"
    );
    ensure!(
        shown.iter().any(|t| t == "New tab"),
        "toggleSidePanel is the original's New tab"
    );
    for hidden in [
        "New Temporary Chat",
        "Open Changes tab",
        "Undo last action",
        "Redo last action",
    ] {
        ensure!(
            !shown.iter().any(|t| t == hidden),
            "{hidden} is not in the original dialog"
        );
    }
    Ok(())
}

#[test]
fn the_open_shortcuts_dialog_follows_live_bindings() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    ensure!(titles(&fx).contains(&"Archive chat"));
    change(
        &mut fx,
        Change::Bindings {
            command: "archiveThread".into(),
            keys: Vec::new(),
        },
    )?;
    ensure!(
        !titles(&fx).contains(&"Archive chat"),
        "an unbound command leaves the open dialog"
    );
    change(&mut fx, Change::ResetCommand("archiveThread".into()))?;
    ensure!(titles(&fx).contains(&"Archive chat"));
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
