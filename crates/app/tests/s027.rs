mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

fn point(value: &Value) -> Result<(f32, f32)> {
    Ok((
        value["x"].as_f64().context("x")? as f32,
        value["y"].as_f64().context("y")? as f32,
    ))
}

fn center(value: &Value) -> Result<(f32, f32)> {
    let (x, y) = point(value)?;
    Ok((
        x + value["w"].as_f64().context("width")? as f32 / 2.,
        y + value["h"].as_f64().context("height")? as f32 / 2.,
    ))
}

fn menu(fx: &Fx) -> Vec<(&'static str, bool)> {
    fx.read(|root, cx| root.diff_panel().unwrap().read(cx).menu_items().collect())
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let items = menu(fx);
    let index = items
        .iter()
        .filter(|(_, enabled)| *enabled)
        .position(|(text, _)| *text == label)
        .context("enabled menu item")?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[test]
fn diff_selection_and_hover_replay() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S027 requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s027.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    fx.click(fx.at("sidebar", "Add a subtract")?)?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.click(fx.at("thread", "View changes")?)?;
    let requests_before = fx.sent()?.len();
    let steps = recording["steps"].as_array().context("steps")?;
    let header = steps
        .iter()
        .find(|s| s["target"]["text"] == "File actions")
        .context("file actions")?;
    let trigger = center(&header["target"]["bbox"])?;
    fx.click(trigger)?;
    let expected: Vec<_> = header["expect"]["menu"]
        .as_array()
        .context("header menu")?
        .iter()
        .map(|item| {
            (
                item["text"].as_str().unwrap(),
                item["text"] != "Open in editor",
            )
        })
        .collect();
    ensure!(menu(&fx) == expected, "header menu: {:?}", menu(&fx));
    fx.shot()?
        .save(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("s027-header-menu.png"))?;
    let copy = steps
        .iter()
        .find(|s| s["target"]["text"] == "Copy path")
        .context("copy path")?;
    fx.click(center(&copy["target"]["bbox"])?)?;
    ensure!(
        clipboard(&mut fx).as_deref() == copy["expect"]["clipboard"].as_str(),
        "header path copy"
    );
    ensure!(menu(&fx).is_empty(), "copy must close the menu");
    fx.click(trigger)?;
    fx.keys("down down enter")?;
    ensure!(
        !fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].open),
        "collapse skips the disabled editor item"
    );
    fx.click(trigger)?;
    choose(&mut fx, "Expand file")?;
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].open),
        "expand file"
    );
    fx.right_click((800., 229.))?;
    ensure!(
        menu(&fx)[0] == ("Copy selection", false),
        "copy needs a selection"
    );
    fx.keys("escape")?;
    ensure!(menu(&fx).is_empty(), "Escape must close the menu");
    let hover = steps
        .iter()
        .find(|s| s["type"] == "hover")
        .context("hover")?;
    ensure!(
        hover["expect"]["hover"]["before"] == hover["expect"]["hover"]["after"],
        "the original hover changed"
    );
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.hover(point(&hover["portAt"])?)?;
    let after = fx.shot()?;
    ensure!(
        common::changed(&before, &after, [662., 218., 614., 22.]) == 0,
        "diff hover must not change the row"
    );
    let drag = steps.iter().find(|s| s["type"] == "drag").context("drag")?;
    fx.drag(point(&drag["portFrom"])?, point(&drag["portTo"])?, 10)?;
    let selected = fx.read(|root, cx| root.diff_panel().unwrap().read(cx).selected_text());
    ensure!(
        selected == drag["expect"]["selection"].as_str().context("selection")?,
        "selected text: {selected:?}"
    );
    let key = steps.iter().find(|s| s["type"] == "key").context("copy")?;
    fx.keys(key["portKey"].as_str().context("port key")?)?;
    let copied = clipboard(&mut fx);
    ensure!(
        copied.as_deref() == key["expect"]["clipboard"].as_str(),
        "clipboard: {copied:?}"
    );
    fx.shot()?
        .save(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("s027-selection.png"))?;
    fx.drag(point(&drag["portTo"])?, point(&drag["portFrom"])?, 10)?;
    fx.keys("cmd-c")?;
    let reverse = clipboard(&mut fx);
    ensure!(
        reverse == copied,
        "reverse selection must copy the same text"
    );
    let context = steps
        .iter()
        .find(|s| s["type"] == "context-menu")
        .context("context menu")?;
    fx.right_click(point(&context["portAt"])?)?;
    let expected: Vec<_> = context["expect"]["items"]
        .as_array()
        .context("context items")?
        .iter()
        .filter(|item| {
            matches!(
                item["id"].as_str(),
                Some("copy-selection" | "copy-path" | "copy-relative-path" | "toggle-wrap")
            )
        })
        .map(|item| {
            (
                item["label"].as_str().unwrap(),
                item["enabled"].as_bool().unwrap(),
            )
        })
        .collect();
    ensure!(menu(&fx) == expected, "context menu: {:?}", menu(&fx));
    fx.shot()?
        .save(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("s027-context-menu.png"))?;
    choose(&mut fx, "Copy selection")?;
    ensure!(
        clipboard(&mut fx).as_deref()
            == recording["selectionNative"]["clipboard"][0]["args"][0].as_str(),
        "native selection copy"
    );
    for action in recording["contextActions"]
        .as_array()
        .context("context actions")?
    {
        fx.right_click(point(&action["portAt"])?)?;
        let label = match action["id"].as_str().context("action id")? {
            "copy-path" => "Copy path",
            "copy-relative-path" => "Copy relative path",
            "toggle-wrap" => "Toggle line wrap",
            other => anyhow::bail!("unknown menu action {other}"),
        };
        choose(&mut fx, label)?;
        if let Some(text) = action["expect"]["clipboard"][0]["args"][0].as_str() {
            ensure!(
                clipboard(&mut fx).as_deref() == Some(text),
                "{label} clipboard"
            );
        } else {
            ensure!(
                action["expect"]["wrapMode"] == "wrap",
                "original wrap state"
            );
            ensure!(
                fx.read(|root, cx| root.diff_panel().unwrap().read(cx).wraps_lines()),
                "wrap must turn on"
            );
        }
    }
    let whitespace = &recording["whitespaceSelection"];
    fx.drag(
        point(&whitespace["portFrom"])?,
        point(&whitespace["portTo"])?,
        10,
    )?;
    fx.keys("cmd-c")?;
    ensure!(
        clipboard(&mut fx).as_deref() == whitespace["keyboardClipboard"].as_str(),
        "keyboard copy preserves whitespace"
    );
    fx.right_click((800., 164.))?;
    choose(&mut fx, "Copy selection")?;
    ensure!(
        clipboard(&mut fx).as_deref() == whitespace["menuClipboard"].as_str(),
        "native menu copy trims whitespace"
    );
    fx.right_click((800., 229.))?;
    fx.click((640., 22.))?;
    ensure!(menu(&fx).is_empty(), "outside click must close the menu");
    let sent = fx.sent()?;
    ensure!(
        serde_json::to_value(&sent[requests_before..])? == recording["result"]["requests"],
        "diff actions must match the recorded app-server traffic"
    );
    let thread = common::recorded_thread(common::SUBTRACT)?;
    let turn = thread["turns"]
        .as_array()
        .context("turns")?
        .last()
        .context("last turn")?;
    let long_line = format!("    {}end", "abcdefghij ".repeat(12));
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(bridge::Event::Notification {
            method: "turn/diff/updated".into(),
            params: serde_json::json!({
                "threadId": common::SUBTRACT,
                "turnId": turn["id"],
                "diff": format!("diff --git a/src/math.py b/src/math.py\n--- a/src/math.py\n+++ b/src/math.py\n@@ -0,0 +1 @@\n+{long_line}\n"),
            }),
        }, cx);
    });
    fx.cx.run_until_parked();
    fx.drag((714., 143.), (1270., 174.), 10)?;
    fx.keys("cmd-c")?;
    fx.shot()?.save(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("s027-wrapped-selection.png"),
    )?;
    let wrapped = clipboard(&mut fx);
    ensure!(
        wrapped.as_deref() == Some(long_line.as_str()),
        "wrapped text: {wrapped:?}"
    );
    fx.click((1000., 300.))?;
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).selected_text())
            .is_empty(),
        "background click clears the selection"
    );
    fx.keys("cmd-c")?;
    ensure!(
        clipboard(&mut fx) == wrapped,
        "copy with no selection leaves the clipboard unchanged"
    );
    Ok(())
}
