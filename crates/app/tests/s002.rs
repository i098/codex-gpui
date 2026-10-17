//! S002: the Recents row context menu (RS-090) and its `Rename chat` dialog (RS-091),
//! replayed from refs/interactions/s002.json on the port. The original shows a native menu
//! (`electronBridge.showContextMenu`) that the recorder answered with `nativeMenuPick`; the
//! port draws the menu, so the pick is a click on its `Rename` row.

mod common;

use anyhow::{Context as _, Result, ensure};
use app::Route;
use common::{Fx, ROOT, changed, proxy_child, script};
use serde_json::{Value, json};
use std::{fs, time::Duration};
use ui::dialog::Kind as DialogKind;

#[test]
fn proxy() {
    proxy_child();
}

fn at(step: &Value) -> (f32, f32) {
    let p = |k: &str| step["at"][k].as_f64().unwrap() as f32;
    (p("x"), p("y"))
}

fn center(bbox: &Value) -> (f32, f32) {
    let f = |k: &str| bbox[k].as_f64().unwrap() as f32;
    (f("x") + f("w") / 2., f("y") + f("h") / 2.)
}

/// The recorded items without their icons (the port's `context_menu` shape).
fn without_icons(v: &Value) -> Value {
    match v {
        Value::Object(m) => m
            .iter()
            .filter(|(k, _)| *k != "icon")
            .map(|(k, v)| (k.clone(), without_icons(v)))
            .collect(),
        Value::Array(a) => a.iter().map(without_icons).collect(),
        v => v.clone(),
    }
}

fn menu(fx: &Fx) -> Option<Value> {
    fx.read(|root, cx| root.sidebar().read(cx).context_menu())
}

fn menu_at(fx: &Fx) -> Option<(f32, f32)> {
    fx.read(|root, cx| {
        let p = root.sidebar().read(cx).context_menu_at()?;
        Some((f32::from(p.x), f32::from(p.y)))
    })
}

/// `(kind, focused control, title, all selected)` of the open dialog.
fn dialog(fx: &Fx) -> Option<(DialogKind, String, String, bool)> {
    fx.read(|root, cx| {
        let d = root.dialog()?.read(cx);
        Some((
            d.kind(),
            d.focused_control().to_owned(),
            d.query().to_owned(),
            d.all_selected(),
        ))
    })
}

fn dialog_focused(fx: &mut Fx) -> Result<bool> {
    let dialog = fx.read(|root, _| root.dialog().cloned());
    fx.cx.update_window(fx.window, |_, window, cx| {
        dialog.is_some_and(|d| gpui::Focusable::focus_handle(&d, cx).is_focused(window))
    })
}

/// Center of menu row `index`: the popover pads 4 px, rows are 28.5625 px, separators 9 px.
fn row_center(items: &[Value], (x, y): (f32, f32), index: usize) -> (f32, f32) {
    let above: f32 = items[..index]
        .iter()
        .map(|i| {
            if i["type"] == "separator" {
                9.
            } else {
                28.5625
            }
        })
        .sum();
    (x + 4. + 110., y + 4. + above + 28.5625 / 2.)
}

/// The most common colour inside the CSS-px `bbox` of a 2x shot.
fn dominant(shot: &image::RgbaImage, bbox: &Value) -> [u8; 3] {
    let n = |k: &str| (bbox[k].as_f64().unwrap() * 2.) as u32;
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    for y in n("y")..n("y") + n("h") {
        for x in n("x")..n("x") + n("w") {
            let p = shot.get_pixel(x, y).0;
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|&(_, n)| n).unwrap().0
}

fn name_set_requests(fx: &mut Fx) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "thread/name/set")
        .map(|r| r["params"].clone())
        .collect())
}

fn recorded_params(requests: &Value) -> Vec<Value> {
    requests
        .as_array()
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str(r["params"].as_str().unwrap()).unwrap())
        .collect()
}

#[test]
fn row_menu_and_rename_dialog_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s002: the Metal replay requires macOS");
        return Ok(());
    }
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s002.json"))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let steps = rec["steps"].as_array().context("steps")?;
    let result = &rec["result"];
    let items = without_icons(&result["nativeContextMenu"]["items"]);
    let input = &result["renameDialog"]["input"];
    let row = &steps[0]["target"]["bbox"];
    let row_rect = [
        row["x"].as_f64().unwrap() as f32,
        row["y"].as_f64().unwrap() as f32,
        row["w"].as_f64().unwrap() as f32,
        row["h"].as_f64().unwrap() as f32,
    ];
    let rest = fx.shot()?;
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let mut expected_requests = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        match step["type"].as_str() {
            Some("click") if step["button"] == "right" => {
                fx.right_click(at(step))?;
                let got = menu(&fx).context("the right-click opened no menu")?;
                ensure!(got == items, "menu differs:\n{got}\n{items}");
                let anchored = menu_at(&fx).unwrap();
                ensure!(
                    anchored == at(step),
                    "menu anchored at {anchored:?}, the original at {:?}",
                    at(step)
                );
                if index == 0 {
                    fx.shot()?.save(out.join("s002-row-menu.png"))?;
                    // Down lands on `Rename`; Escape closes it, focus unchanged.
                    fx.keys("down")?;
                    fx.keys("escape")?;
                    ensure!(menu(&fx).is_none(), "Escape must close the row menu");
                    continue;
                }
                let pick = step["nativeMenuPick"].as_str().context("pick")?;
                let row = items
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|i| i["id"] == pick)
                    .unwrap();
                fx.click(row_center(items.as_array().unwrap(), anchored, row))?;
                ensure!(menu(&fx).is_none(), "the pick must close the menu");
                let (kind, focused, title, selected) = dialog(&fx).context("no dialog")?;
                ensure!(kind == DialogKind::Rename, "dialog {kind:?}");
                ensure!(
                    focused == input["aria"].as_str().unwrap() && dialog_focused(&mut fx)?,
                    "the title input is not focused: {focused}"
                );
                ensure!(
                    title == input["value"].as_str().unwrap() && selected,
                    "title {title:?} selected {selected}"
                );
                let shot = fx.shot()?;
                if index == 1 {
                    shot.save(out.join("s002-rename-dialog.png"))?;
                }
                let got = dominant(&shot, &input["bbox"]);
                ensure!(
                    got.iter().all(|c| c.abs_diff(45) <= 2),
                    "the title input shows rgb{got:?}, recorded rgba(45, 45, 45, 0.96)"
                );
            }
            Some("click") => {
                fx.click(center(&step["target"]["bbox"]))?;
            }
            Some("key") => {
                fx.keys(&step["key"].as_str().unwrap().to_lowercase())?;
            }
            Some("type") => {
                fx.type_text(step["text"].as_str().unwrap())?;
                let (_, _, title, selected) = dialog(&fx).context("no dialog")?;
                let typed = &result["afterType"]["input"];
                ensure!(
                    title == typed["value"].as_str().unwrap() && !selected,
                    "typed title {title:?}"
                );
            }
            other => anyhow::bail!("unsupported step {other:?}"),
        }
        for (key, outcome) in [
            ("afterEscape", &result["afterEscape"]),
            ("afterEnter", &result["afterEnter"]),
            ("afterSave", &result["afterSave"]),
        ] {
            if outcome["afterStep"] != index {
                continue;
            }
            ensure!(dialog(&fx).is_none(), "{key}: the dialog stayed open");
            if let Some(toast) = outcome["toast"].as_str() {
                fx.wait(toast, |fx| {
                    fx.read(|r, cx| r.toasts(cx)).iter().any(|t| t == toast)
                })?;
                fx.advance(Duration::from_millis(300))?;
                ensure!(
                    changed(&rest, &fx.shot()?, row_rect) == 0,
                    "{key}: the row changed after a refused rename"
                );
            } else {
                // The original's activeElement is the body; the port's body is the root.
                ensure!(
                    fx.cx
                        .update_window(fx.window, |_, window, cx| window.focused(cx).is_some())?
                        && !fx.composer_focused()?,
                    "{key}: focus did not go to the root"
                );
            }
            expected_requests.extend(recorded_params(&outcome["requests"]));
            ensure!(
                name_set_requests(&mut fx)? == expected_requests,
                "{key}: thread/name/set requests {:?}, recorded {expected_requests:?}",
                name_set_requests(&mut fx)?
            );
        }
    }
    ensure!(fx.route() == Route::Home, "route left Home");
    ensure!(
        json!(fx.read(|r, _| r.recents())).as_array().unwrap().len() == 3,
        "Recents changed"
    );
    Ok(())
}
