//! S022: the thread composer's placeholder while a turn runs (CP-002), undo and redo
//! (CP-010), Option+Arrow word movement (CP-013), word and line deletes (CP-014), select all,
//! copy, cut and paste (CP-018) and the editor's right-click menu (SH-126), replayed from
//! refs/interactions/s022.json on the port.

mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::Event;
use common::{Fx, ROOT, SUBTRACT, exchange, proxy_child, script};
use gpui::{EntityInputHandler as _, Focusable as _};
use serde_json::{Value, json};
use std::{fs, path::Path, thread, time::Duration};

#[test]
fn proxy() {
    proxy_child();
}

/// The editor's text, selection anchor and focus (character offsets) and whether it is focused,
/// as the recording's `editor` entries hold them.
fn editor(fx: &mut Fx) -> Result<Value> {
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |c, cx| {
            let text = c
                .text_for_range(0..usize::MAX, &mut None, window, cx)
                .unwrap_or_default();
            let sel = c
                .selected_text_range(false, window, cx)
                .context("selection")?;
            let (anchor, focus) = if sel.reversed {
                (sel.range.end, sel.range.start)
            } else {
                (sel.range.start, sel.range.end)
            };
            let focused = c.focus_handle(cx).is_focused(window);
            Ok(json!({"text": text, "anchor": anchor, "focus": focus, "focused": focused}))
        })
    })?
}

fn menu(fx: &Fx) -> Vec<(String, String, bool)> {
    fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .context_menu_items()
            .map(|(id, label, enabled)| (id.to_owned(), label.to_owned(), enabled))
            .collect()
    })
}

fn menu_at(fx: &Fx) -> Option<(f32, f32)> {
    fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .context_menu_at()
            .map(|p| (f32::from(p.x), f32::from(p.y)))
    })
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn point(v: &Value) -> (f32, f32) {
    (
        v["x"].as_f64().unwrap() as f32,
        v["y"].as_f64().unwrap() as f32,
    )
}

fn notify(fx: &mut Fx, step: &Value) -> Result<()> {
    let event = Event::Notification {
        method: step["method"].as_str().context("method")?.into(),
        params: step["params"].clone(),
    };
    fx.root
        .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn composer_editing_matches_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s022: the Metal replay requires macOS");
        return Ok(());
    }
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s022.json"))?)?;
    let steps = rec["steps"].as_array().context("steps")?;
    let dom = rec["result"]["dom"].as_array().context("dom")?;
    let menus = rec["result"]["contextMenus"]
        .as_array()
        .context("contextMenus")?;
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/resume", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (index, step) in steps.iter().enumerate() {
        match step["type"].as_str() {
            Some("click") if step["waitThread"].is_string() => {
                fx.click(fx.at("home", "Add a subtract")?)?;
                fx.wait_thread(SUBTRACT)?;
                ensure!(fx.thread_id().as_deref() == Some(SUBTRACT));
            }
            Some("click") => {
                let b = &step["target"]["bbox"];
                let (x, y) = point(b);
                fx.click((
                    x + b["w"].as_f64().unwrap() as f32 / 2.,
                    y + b["h"].as_f64().unwrap() as f32 / 2.,
                ))?;
            }
            Some("bridge") => {
                notify(&mut fx, step)?;
                let running = step["method"] == "turn/started";
                fx.wait("turn state", |fx| {
                    fx.read(|root, cx| {
                        (root.composer(cx).read(cx).send_label() == "Stop") == running
                    })
                })?;
            }
            // Typing right after an edit joins its undo group (500 ms): the recording paused.
            Some("type") => {
                thread::sleep(Duration::from_millis(600));
                fx.type_text(step["text"].as_str().context("text")?)?;
            }
            Some("key") => fx.keys(step["portKey"].as_str().context("portKey")?)?,
            Some("context-menu") => {
                let expected = menus
                    .iter()
                    .find(|m| m["afterStep"] == index)
                    .context("recorded context menu")?;
                let before = fx.sent()?.len();
                let at = point(&step["at"]);
                fx.right_click(at)?;
                ensure!(expected["shown"] == true);
                let items: Vec<(String, String, bool)> = expected["menu"]["items"]
                    .as_array()
                    .context("items")?
                    .iter()
                    .map(|item| {
                        (
                            item["id"].as_str().unwrap_or("separator").to_owned(),
                            item["label"].as_str().unwrap_or("").to_owned(),
                            item["enabled"] == true,
                        )
                    })
                    .collect();
                ensure!(
                    menu(&fx) == items,
                    "step {index}: menu {:?}, the original {items:?}",
                    menu(&fx)
                );
                let anchored = menu_at(&fx).context("menu position")?;
                ensure!(
                    anchored == at,
                    "menu anchored at {anchored:?}, clicked at {at:?}"
                );
                // The drawn menu holds keyboard focus while open (the native one leaves the
                // DOM focus in the editor); it returns to the editor on close, checked below.
                let mut state = editor(&mut fx)?;
                state["focused"] = true.into();
                ensure!(
                    state == expected["editor"],
                    "step {index}: editor {state} after the right-click, the original {}",
                    expected["editor"]
                );
                fx.shot()?
                    .save(out.join(format!("s022-menu-{index}.png")))?;
                let has_selection = items.iter().any(|item| item.0 == "lookUpSelection");
                let empty = expected["editor"]["text"] == "";
                if has_selection {
                    // Keyboard: Down skips the separator; Copy is the fourth enabled row.
                    fx.keys("down down down down enter")?;
                    ensure!(menu(&fx).is_empty(), "Enter must close the menu");
                    ensure!(
                        clipboard(&mut fx).as_deref() == Some("one two three"),
                        "Copy wrote {:?}",
                        clipboard(&mut fx)
                    );
                } else if empty {
                    // Down skips the disabled Cut and Copy: the first stop is Paste.
                    fx.keys("down enter")?;
                    ensure!(menu(&fx).is_empty(), "Paste must close the menu");
                    ensure!(
                        fx.composer_text()? == "one two three",
                        "Paste put {:?}",
                        fx.composer_text()?
                    );
                } else {
                    fx.keys("down escape")?;
                    ensure!(menu(&fx).is_empty(), "Escape must close the menu");
                }
                ensure!(fx.composer_focused()?, "focus must return to the editor");
                ensure!(
                    fx.sent()?.len() == before,
                    "the editor menu sent app-server requests"
                );
            }
            other => anyhow::bail!("unsupported recorded step: {other:?}"),
        }
        let Some(expected) = dom.iter().find(|d| d["afterStep"] == index) else {
            continue;
        };
        if expected["placeholder"].is_string() {
            let (placeholder, label) = fx.read(|root, cx| {
                let c = root.composer(cx).read(cx);
                (c.placeholder().to_owned(), c.send_label())
            });
            ensure!(
                placeholder == expected["placeholder"],
                "{}: placeholder {placeholder:?}, the original {}",
                expected["state"],
                expected["placeholder"]
            );
            ensure!(
                (label == "Stop") == expected["stop"].is_object(),
                "{}: button {label}, the original shows Stop: {}",
                expected["state"],
                expected["stop"].is_object()
            );
        } else {
            ensure!(
                editor(&mut fx)? == expected["editor"],
                "step {index} ({}): editor {}, the original {}",
                step["key"].as_str().or(step["text"].as_str()).unwrap_or(""),
                editor(&mut fx)?,
                expected["editor"]
            );
        }
    }
    Ok(())
}
