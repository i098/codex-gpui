//! S010: the selected transcript menu, using the original native rows and S011 selection.
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use gpui::ClipboardItem;
use serde_json::Value;

#[test]
fn proxy() {
    proxy_child();
}

fn at(value: &Value) -> (f32, f32) {
    (
        value["x"].as_f64().unwrap() as f32,
        value["y"].as_f64().unwrap() as f32,
    )
}

fn selection(fx: &mut Fx) -> Result<Option<String>> {
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.read(cx)
            .thread()
            .and_then(|thread| thread.read(cx).selected_text(window, cx))
    })
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn seed_clipboard(fx: &mut Fx) {
    fx.cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("s010 clipboard sentinel".into()));
    });
}

fn menu(fx: &Fx) -> Vec<(String, String, bool)> {
    fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .context_menu_items()
            .map(|(id, label, enabled)| (id.to_owned(), label.to_owned(), enabled))
            .collect()
    })
}

#[test]
fn selected_text_menu_preserves_selection_and_clipboard() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s010: the Metal replay requires macOS");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s010.json"))?;
    let result = &recording["result"];
    let steps = &recording["steps"];
    let expected = result["selection"].as_str().context("selection")?;
    let expected_rows: Vec<(String, String, bool)> = result["menu"]["items"]
        .as_array()
        .context("native menu items")?
        .iter()
        .map(|item| {
            (
                item["id"].as_str().unwrap_or("separator").to_owned(),
                item["label"].as_str().unwrap_or("").to_owned(),
                item["enabled"] == true,
            )
        })
        .collect();
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "In two sentences")?)?;
    fx.wait_thread(EXPLAIN)?;
    fx.drag(at(&steps[1]["from"]), at(&steps[1]["to"]), 12)?;
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    let traffic = fx.sent()?.len();
    seed_clipboard(&mut fx);
    let pointer = at(&steps[2]["at"]);
    fx.right_click(pointer)?;
    assert_eq!(menu(&fx), expected_rows);
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    assert_eq!(
        clipboard(&mut fx).as_deref(),
        Some("s010 clipboard sentinel")
    );
    let anchor = fx.read(|root, cx| root.thread().unwrap().read(cx).context_menu_at());
    let anchor = anchor.context("selected menu anchor")?;
    assert_eq!((f32::from(anchor.x), f32::from(anchor.y)), pointer);
    fx.shot()?.save(format!(
        "{}/s010-selected-menu.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;

    fx.keys("down")?;
    fx.keys("escape")?;
    ensure!(menu(&fx).is_empty(), "Escape left the menu open");
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    assert_eq!(
        clipboard(&mut fx).as_deref(),
        Some("s010 clipboard sentinel")
    );
    fx.keys("cmd-c")?;
    assert_eq!(clipboard(&mut fx).as_deref(), result["clipboard"].as_str());

    seed_clipboard(&mut fx);
    fx.right_click(pointer)?;
    fx.keys("cmd-c")?;
    ensure!(menu(&fx).is_empty(), "Cmd+C left the menu open");
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    assert_eq!(clipboard(&mut fx).as_deref(), result["clipboard"].as_str());

    seed_clipboard(&mut fx);

    fx.right_click(pointer)?;
    // The original inserts a 9 px separator after both Look Up and Search with Google.
    fx.click((
        pointer.0 + 114.,
        pointer.1 + 4. + 2. * (28.5625 + 9.) + 14.28125,
    ))?;
    ensure!(menu(&fx).is_empty(), "Copy left the menu open");
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    assert_eq!(clipboard(&mut fx).as_deref(), result["clipboard"].as_str());

    fx.right_click(pointer)?;
    fx.keys("home")?;
    fx.keys("down")?;
    fx.keys("enter")?;
    ensure!(menu(&fx).is_empty(), "Search left the menu open");
    assert_eq!(
        fx.cx.take_opened_url().as_deref(),
        result["googleUrl"].as_str()
    );
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    assert_eq!(clipboard(&mut fx).as_deref(), result["clipboard"].as_str());

    // A headless window has no AppKit view. The adapter must report that error, not open an app.
    fx.right_click(pointer)?;
    fx.keys("home")?;
    fx.keys("enter")?;
    ensure!(menu(&fx).is_empty(), "Look Up left the menu open");
    assert_eq!(fx.cx.take_opened_url(), None);
    assert_eq!(selection(&mut fx)?.as_deref(), Some(expected));
    assert_eq!(clipboard(&mut fx).as_deref(), result["clipboard"].as_str());

    let backwards = &result["backwards"];
    fx.click(at(&result["collapse"]))?;
    fx.drag(
        at(&backwards["drag"]["from"]),
        at(&backwards["drag"]["to"]),
        12,
    )?;
    assert_eq!(
        selection(&mut fx)?.as_deref(),
        backwards["selection"].as_str()
    );
    fx.right_click((600., pointer.1))?;
    fx.keys("end")?;
    fx.keys("enter")?;
    assert_eq!(
        clipboard(&mut fx).as_deref(),
        backwards["clipboard"].as_str()
    );
    assert_eq!(
        selection(&mut fx)?.as_deref(),
        backwards["selection"].as_str()
    );

    fx.click(at(&result["collapse"]))?;
    assert_eq!(selection(&mut fx)?, None);
    fx.right_click(pointer)?;
    ensure!(
        menu(&fx).is_empty(),
        "Plain text opened a menu without a selection"
    );
    assert_eq!(
        fx.sent()?.len(),
        traffic,
        "The text menu sent app-server requests"
    );
    println!(
        "s010: original native rows, selection, clipboard, search, dismissal, and traffic match"
    );
    Ok(())
}
