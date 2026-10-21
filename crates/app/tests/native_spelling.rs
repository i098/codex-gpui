#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result};
use common::Fx;

fn open_correction(fx: &mut Fx) -> Result<usize> {
    fx.composer()
        .update(&mut fx.cx, |composer, cx| composer.set_text("teh", cx));
    fx.focus_composer()?;
    fx.shot()?;
    fx.right_click((456., 738.))?;
    fx.read(|root, cx| {
        let rows = root
            .composer(cx)
            .read(cx)
            .context_menu_items()
            .map(|(id, label, enabled)| (id.to_owned(), label.to_owned(), enabled))
            .collect::<Vec<_>>();
        let index = rows
            .iter()
            .position(|(id, label, enabled)| {
                id == "dictionarySuggestions" && label == "the" && *enabled
            })
            .with_context(|| format!("system dictionary has no correction: {rows:?}"))?;
        Ok(rows[..index]
            .iter()
            .filter(|(_, _, enabled)| *enabled)
            .count())
    })
}

fn choose_correction(fx: &mut Fx, preceding: usize) -> Result<()> {
    fx.keys("home")?;
    for _ in 0..preceding {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[test]
fn spelling_correction_is_undoable_and_rejects_a_stale_word() -> Result<()> {
    let mut fx = Fx::new()?;
    let preceding = open_correction(&mut fx)?;
    choose_correction(&mut fx, preceding)?;
    assert_eq!(fx.composer_text()?, "the");
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "teh");

    let preceding = open_correction(&mut fx)?;
    fx.composer()
        .update(&mut fx.cx, |composer, cx| composer.set_text("safe", cx));
    choose_correction(&mut fx, preceding)?;
    assert_eq!(fx.composer_text()?, "safe");
    Ok(())
}
