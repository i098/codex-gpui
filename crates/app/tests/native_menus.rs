#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use common::Fx;

#[test]
fn edit_menu_preserves_unselected_text_and_plain_paste_is_undoable() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.focus_composer()?;
    fx.composer().update(&mut fx.cx, |composer, cx| {
        composer.set_text("keep π", cx)
    });
    fx.keys("cmd-left")?;
    fx.menu("Edit", "Delete")?;
    assert_eq!(fx.composer_text()?, "keep π");

    fx.keys("cmd-a")?;
    fx.menu("Edit", "Delete")?;
    assert_eq!(fx.composer_text()?, "");
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "keep π");

    fx.cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("replacement λ".into()));
    });
    fx.menu("Edit", "Paste and Match Style")?;
    assert_eq!(fx.composer_text()?, "replacement λ");
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "keep π");
    fx.keys("cmd-alt-shift-v")?;
    assert_eq!(fx.composer_text()?, "replacement λ");
    fx.keys("cmd-z")?;
    fx.keys("cmd-shift-v")?;
    assert_eq!(fx.composer_text()?, "replacement λ");
    Ok(())
}
