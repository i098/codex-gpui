use anyhow::{Context as _, Result, ensure};
use gpui::{HeadlessAppContext, Modifiers, MouseMoveEvent, PlatformInput, point, px, size};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

#[test]
fn suggestion_hover_matches_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s035: requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s035.json"))?;
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        app::init(cx);
    });
    let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        ui::screens::automations::build(window, cx)
            .downcast::<ui::Shell>()
            .expect("automations fixture shell")
    })?;
    let window = window.into();
    cx.capture_screenshot(window)?;
    for (index, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        let x = step["at"]["x"].as_f64().context("pointer x")? as f32;
        let y = step["at"]["y"].as_f64().context("pointer y")? as f32;
        cx.update_window(window, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(x), px(y)),
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
        })?;
        cx.advance_clock(Duration::from_millis(
            step["waitMs"].as_u64().context("waitMs")?,
        ));
        cx.run_until_parked();
        let image = cx.capture_screenshot(window)?;
        let scale = image.width() as f32 / 1280.;
        for sample in recording["result"]["dom"][index]["pixels"]
            .as_array()
            .context("recorded pixels")?
        {
            let x = sample["x"].as_f64().context("sample x")? as f32;
            let y = sample["y"].as_f64().context("sample y")? as f32;
            let expected: [u8; 3] = serde_json::from_value(sample["rgb"].clone())?;
            let actual = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            ensure!(
                (0..3).all(|channel| actual[channel].abs_diff(expected[channel]) <= 1),
                "step {index}, pixel ({x}, {y}): {actual:?} != {expected:?}"
            );
        }
        let icon = &recording["result"]["dom"][index]["icon"];
        let [x, y, w, h] =
            ["x", "y", "w", "h"].map(|key| icon["bbox"][key].as_f64().expect("icon bounds") as f32);
        for color in icon["colors"].as_array().context("icon colors")? {
            let expected: [u8; 3] = serde_json::from_value(color.clone())?;
            let found = ((y * scale) as u32..((y + h) * scale) as u32).any(|py| {
                ((x * scale) as u32..((x + w) * scale) as u32).any(|px| {
                    let actual = image.get_pixel(px, py).0;
                    (0..3).all(|channel| actual[channel].abs_diff(expected[channel]) <= 1)
                })
            });
            ensure!(
                found,
                "step {index}: icon lost its recorded color {expected:?}"
            );
        }
    }
    println!("s035: hover entry and exit match the card fill, adjacent card, and unchanged icon");
    Ok(())
}
