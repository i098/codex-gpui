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

#[test]
fn popup_typing_does_not_reach_the_composer() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    fx.type_text("draft")?;
    for at in [(1030., 790.), (522.5, 790.)] {
        fx.click(at)?;
        let composer = fx.composer();
        ensure!(
            fx.cx.update_window(fx.window, |_, window, cx| composer
                .read(cx)
                .menu_state(window)
                .0)?,
            "the popup did not open"
        );
        fx.type_text("x")?;
        ensure!(fx.composer_text()? == "draft", "the popup captured text");
        ensure!(!fx.composer_focused()?, "the popup lost focus");
        fx.keys("escape")?;
    }
    fx.click((800., 200.))?;
    fx.type_text("a")?;
    ensure!(fx.composer_text()? == "drafta", "content typing was lost");
    ensure!(
        fx.composer_focused()?,
        "content typing did not focus the editor"
    );
    Ok(())
}

#[test]
fn thread_content_typing_preserves_editing_and_popup_isolation() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    let image = fx.shot()?;
    let y = image.height() * 65 / 100;
    let center = image.width() / 2;
    let color = image.get_pixel(center, y);
    let mut left = center;
    let mut right = center;
    while left > 0 && image.get_pixel(left - 1, y) == color {
        left -= 1;
    }
    while right + 1 < image.width() && image.get_pixel(right + 1, y) == color {
        right += 1;
    }
    ensure!(right - left > image.width() / 2, "no blank transcript span");
    let scale = image.width() as f32 / fx.size.0;
    fx.click(((left + right) as f32 / (2. * scale), y as f32 / scale))?;
    fx.type_text("a")?;
    ensure!(fx.composer_text()? == "a", "thread content typing was lost");
    ensure!(
        fx.composer_focused()?,
        "thread typing did not focus the editor"
    );
    fx.keys("left")?;
    fx.type_text("b")?;
    ensure!(
        fx.composer_text()? == "ba",
        "thread editing changed the cursor position"
    );
    for at in [(1030., 790.), (522.5, 790.)] {
        fx.click(at)?;
        let composer = fx.composer();
        ensure!(
            fx.cx.update_window(fx.window, |_, window, cx| composer
                .read(cx)
                .menu_state(window)
                .0)?,
            "the thread popup did not open"
        );
        fx.type_text("x")?;
        ensure!(
            fx.composer_text()? == "ba",
            "thread popup input reached the draft"
        );
        ensure!(!fx.composer_focused()?, "the thread popup lost focus");
        fx.keys("escape")?;
    }
    Ok(())
}
