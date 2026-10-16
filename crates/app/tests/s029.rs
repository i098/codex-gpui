mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::{Fx, script};
use image::RgbaImage;
use serde_json::Value;
use ui::dialog::Kind as DialogKind;

#[test]
fn proxy() {
    common::proxy_child();
}

fn center(bbox: &Value) -> Result<(f32, f32)> {
    let x = bbox["x"].as_f64().context("x")? + bbox["w"].as_f64().context("w")? / 2.;
    let y = bbox["y"].as_f64().context("y")? + bbox["h"].as_f64().context("h")? / 2.;
    Ok((x as f32, y as f32))
}

/// The recorded tabs as the port's `(label, selected)`.
fn recorded_tabs(expected: &Value) -> Vec<(String, bool)> {
    expected["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            (
                t["label"].as_str().unwrap_or_default().to_owned(),
                t["selected"] == true,
            )
        })
        .collect()
}

fn port_tabs(fx: &Fx) -> Vec<(String, bool)> {
    fx.read(|root, _| {
        root.side_tabs()
            .iter()
            .map(|side| {
                let label = match side {
                    SidePanel::Changes => "Changes",
                    SidePanel::Terminal => {
                        "python3 -c 'from src.math import subtract; print(subtract(5, 3))'"
                    }
                };
                (label.to_owned(), root.side_panel() == Some(*side))
            })
            .collect()
    })
}

/// A tab surface px (y 9: inside the 32 px tab, above its text) of tab `bbox`.
fn surface_px(image: &RgbaImage, bbox: &Value) -> Result<[u8; 4]> {
    let scale = image.width() / 1280;
    let x = bbox["x"].as_u64().context("tab x")? as u32 + 200;
    Ok(image.get_pixel(x * scale, 9 * scale).0)
}

/// The recorded surface fills, checked against the port's px: the selected tab (and a hovered
/// one) paints `rgba(255, 255, 255, 0.08)` over the titlebar, an unselected one nothing.
fn check_surfaces(fx: &mut Fx, expected: &Value, hovered: Option<usize>) -> Result<()> {
    let image = fx.shot()?;
    let n = expected["tabs"].as_array().map_or(0, Vec::len);
    image.save(std::env::temp_dir().join(format!(
        "cgx-s029-port-{n}{}.png",
        hovered.map_or("", |_| "-hover")
    )))?;
    let scale = image.width() / 1280;
    // Empty strip px between `New tab` and `Enter full view`.
    let bare = image.get_pixel(1190 * scale, 9 * scale).0;
    for (ix, tab) in expected["tabs"]
        .as_array()
        .context("tabs")?
        .iter()
        .enumerate()
    {
        let px = surface_px(&image, &tab["bbox"])?;
        let fill = tab["surface"]["backgroundColor"].as_str().context("fill")?;
        let filled = fill == "rgba(255, 255, 255, 0.08)";
        ensure!(
            filled == (tab["selected"] == true || hovered == Some(ix)),
            "recording: tab {ix} fill {fill}"
        );
        if filled {
            ensure!(
                px[0] > bare[0] + 10,
                "tab {ix} must paint its surface: {px:?} over {bare:?}"
            );
        } else {
            ensure!(px == bare, "tab {ix} must stay bare: {px:?} vs {bare:?}");
        }
    }
    Ok(())
}

#[test]
fn panel_tabs_replay() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S029 rendering requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s029.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let mut hovered = None;
    for step in recording["steps"].as_array().context("steps")? {
        let expected = &step["expect"];
        match step["type"].as_str() {
            Some("click") => {
                let at = if let Some(id) = step["waitThread"].as_str() {
                    let at = fx.at("home", step["target"]["text"].as_str().context("row")?)?;
                    fx.click(at)?;
                    fx.wait_thread(id)?;
                    continue;
                } else if let Some(reference) = step["portReference"].as_object() {
                    fx.at(
                        reference["screen"].as_str().context("reference screen")?,
                        reference["text"].as_str().context("reference text")?,
                    )?
                } else if step["at"].is_object() {
                    point(&step["at"])?
                } else {
                    center(&step["target"]["bbox"])?
                };
                fx.click(at)?;
                hovered = None;
                fx.hover((1270., 820.))?;
            }
            Some("hover") => {
                fx.hover(point(&step["at"])?)?;
                hovered = Some(0);
            }
            Some("bridge") => {
                let view = fx.read(|r, _| r.thread().cloned()).context("no thread")?;
                fx.cx.update(|cx| {
                    view.update(cx, |v, cx| {
                        for event in ui::screens::terminal::running_command_replay() {
                            v.apply(&event, cx);
                        }
                    })
                });
                fx.cx.run_until_parked();
            }
            Some("key") => {
                let windows = fx.cx.update(|cx| cx.windows().len());
                fx.keys(step["portKey"].as_str().context("port key")?)?;
                if expected["windowOpen"] == true {
                    ensure!(
                        fx.cx.update(|cx| cx.windows().len()) == windows
                            && fx.read(|r, _| r.dialog().is_none()),
                        "⌘W with a tab must close the tab, not the window"
                    );
                } else {
                    ensure!(
                        fx.read(|r, cx| r
                            .dialog()
                            .is_some_and(|d| d.read(cx).kind() == DialogKind::CloseWindow)),
                        "⌘W without a tab must close the window"
                    );
                }
            }
            other => anyhow::bail!("unknown step: {other:?}"),
        }
        if expected.get("tabs").is_some() {
            ensure!(
                port_tabs(&fx) == recorded_tabs(expected),
                "{:?}: {:?} vs recorded {:?}",
                step["target"]["text"],
                port_tabs(&fx),
                recorded_tabs(expected)
            );
            check_surfaces(&mut fx, expected, hovered)?;
        }
        if let Some(panel) = expected["tabpanel"].as_str() {
            let active = port_tabs(&fx).into_iter().find(|(_, selected)| *selected);
            ensure!(
                active.as_ref().is_some_and(|(label, _)| label == panel),
                "the panel must show the selected tab {panel}: {active:?}"
            );
        }
    }
    ensure!(
        port_tabs(&fx).is_empty() && fx.shell(|s| s.new_tab),
        "closing the last tab closes the panel"
    );
    Ok(())
}

fn point(at: &Value) -> Result<(f32, f32)> {
    Ok((
        at["x"].as_f64().context("x")? as f32,
        at["y"].as_f64().context("y")? as f32,
    ))
}
