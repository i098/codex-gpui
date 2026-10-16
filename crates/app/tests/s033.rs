//! Replays `refs/interactions/s033.json` (SH-158: Escape closes the open menu and where
//! focus goes; HC-033: a printable key typed outside an editable goes to the composer) and
//! drives Edit ▸ Cut, Copy and Paste on the focused composer (SH-098: not recordable in
//! route1, the menu lives in Electron's main process).

mod common;

use anyhow::{Result, ensure};
use common::Fx;
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

#[test]
fn escape_and_type_anywhere_match_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s033.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let before = fx.sent()?.len();
    for (i, step) in recording["steps"].as_array().unwrap().iter().enumerate() {
        match step["type"].as_str().unwrap() {
            "click" => fx.click((
                step["at"]["x"].as_f64().unwrap() as f32,
                step["at"]["y"].as_f64().unwrap() as f32,
            ))?,
            "key" => match step["key"].as_str().unwrap() {
                "Escape" => fx.keys("escape")?,
                " " => fx.keys("space")?,
                key => fx.type_text(key)?,
            },
            other => anyhow::bail!("unsupported recorded step {other}"),
        }
        let expected = &recording["result"]["dom"][i];
        let composer = fx.composer();
        let (menu_open, _, trigger_focused) = fx.cx.update_window(fx.window, |_, window, cx| {
            composer.read(cx).menu_state(window)
        })?;
        ensure!(
            menu_open == expected["menuOpen"].as_bool().unwrap(),
            "step {i}: menu open {menu_open}"
        );
        ensure!(
            fx.composer_text()? == expected["editorText"].as_str().unwrap(),
            "step {i}: editor text {:?}",
            fx.composer_text()?
        );
        // Escape with nothing open blurs the original's editor (CP-111, S024): not an S033 row.
        if step["note"]
            .as_str()
            .is_some_and(|note| note.contains("not asserted"))
        {
            continue;
        }
        let active = &expected["active"];
        ensure!(
            fx.composer_focused()? == active["editor"].as_bool().unwrap(),
            "step {i}: composer focus {}",
            fx.composer_focused()?
        );
        let on_trigger = active["aria"] == "Change permissions";
        ensure!(
            trigger_focused == on_trigger,
            "step {i}: permissions trigger focus {trigger_focused}"
        );
    }
    ensure!(
        fx.sent()?[before..] == *recording["result"]["requests"].as_array().unwrap(),
        "the steps sent an app-server request"
    );

    // SH-098: the Edit menu's native roles act on the focused composer's text.
    fx.keys("cmd-a")?;
    fx.menu("Edit", "Cut")?;
    ensure!(fx.composer_text()?.is_empty(), "Cut left the text");
    ensure!(
        clipboard(&mut fx).as_deref() == Some("abc"),
        "Cut wrote nothing"
    );
    fx.menu("Edit", "Paste")?;
    ensure!(fx.composer_text()? == "abc", "Paste did not insert");
    fx.cx
        .update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("stale".into())));
    fx.keys("cmd-a")?;
    fx.menu("Edit", "Copy")?;
    ensure!(
        clipboard(&mut fx).as_deref() == Some("abc"),
        "Copy wrote nothing"
    );
    ensure!(fx.composer_text()? == "abc", "Copy changed the text");
    Ok(())
}
