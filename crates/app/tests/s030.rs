//! DT-056 (S030): a drag on the chat/panel divider resizes the side panel live, both panes
//! keep 320 px, and the width stays after the release, as the original recorded in
//! refs/interactions/s030.json (edge `left`: the panel width is the width at press minus the
//! pointer travel; the stored width is clamped to the window's maximum, so the recording
//! shrinks the panel first).
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::*;
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px,
};
use serde_json::Value;
use ui::screens::diff::panel::panel_width;

#[test]
fn proxy() {
    proxy_child();
}

/// The port's panel width, as its layout reads it.
fn width(fx: &mut Fx) -> f32 {
    fx.cx
        .update_window(fx.window, |_, window, cx| {
            f32::from(panel_width(window, cx))
        })
        .expect("window")
}

fn pointer_x(at: &Value) -> Result<f32> {
    Ok(at["x"].as_f64().context("x")? as f32)
}

/// Presses the divider, moves `dx` in 10 steps sampling the width halfway, releases.
fn drag(fx: &mut Fx, dx: f32) -> Result<f32> {
    // Inside the divider's 16 px hit area on the panel side of the border.
    let (x, y) = (fx.size.0 - 4. - width(fx) + 8., 400.);
    fx.hover((x, y))?;
    fx.input(PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    }))?;
    let mut mid = 0.;
    for i in 1..=10 {
        fx.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(x + dx * i as f32 / 10.), px(y)),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        }))?;
        if i == 5 {
            mid = width(fx);
        }
    }
    fx.input(PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: point(px(x + dx), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
    }))?;
    Ok(mid)
}

#[test]
fn divider_drag_resizes_the_panel() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s030: the Metal replay requires macOS");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s030.json"))?;
    let steps = rec["steps"].as_array().context("steps")?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    // Steps 0 and 1: the Recents row, then `View changes`.
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.click(fx.at("thread", "View changes")?)?;
    ensure!(
        fx.read(|r, _| r.side_panel()) == Some(SidePanel::Changes),
        "{:?}",
        fx.read(|r, _| r.side_panel())
    );
    fx.hover((1270., 820.))?;
    let rest = width(&mut fx);
    let start = &steps[2]["expect"]["start"]["separator"];
    let (min, max) = (
        start["min"].as_f64().context("min")? as f32,
        start["max"].as_f64().context("max")? as f32,
    );
    // The original opens at its maximum (`aria-valuenow` = `aria-valuemax`): the window
    // leaves the chat its 320 px. The port's main panel counts its 0.5 px hairline in the
    // width, so its maximum is 0.5 px wider.
    ensure!(rest == max + 0.5, "panel at rest {rest}, original {max}");
    let before = fx.shot()?;

    // Steps 2 and 3: the drags; the width follows the pointer live and stays after release.
    for step in &steps[2..] {
        let dx = pointer_x(&step["to"])? - pointer_x(&step["from"])?;
        let was = width(&mut fx);
        let mid = drag(&mut fx, dx)?;
        let now = width(&mut fx);
        let expected = step["expect"]["after"]["panel"]["width"]
            .as_f64()
            .context("width")? as f32;
        let delta = expected
            - step["expect"]["start"]["separator"]["now"]
                .as_f64()
                .context("now")? as f32;
        ensure!(
            mid == was - dx / 2.,
            "live width {mid} halfway, from {was} by {dx}"
        );
        ensure!(
            now == was + delta,
            "width {now} after release, from {was} by {delta}"
        );
    }
    let after = fx.shot()?;
    after.save(format!("{}/s030-port.png", env!("CARGO_TARGET_TMPDIR")))?;
    // The panel's left border moved 100 px right: the chat now paints there.
    let moved = changed(&before, &after, [1280. - 4. - rest, 44., 100., 784.]);
    ensure!(moved > 20_000, "panel region changed by only {moved} px");

    // Both panes keep 320 px: a far drag right stops at the recorded minimum, a far drag
    // left at the maximum (the original then overflows into the full view, DT-061).
    drag(&mut fx, 600.)?;
    ensure!(width(&mut fx) == min, "{} vs min {min}", width(&mut fx));
    drag(&mut fx, -600.)?;
    ensure!(width(&mut fx) == rest, "{} vs max {rest}", width(&mut fx));
    println!("s030: the divider resizes the panel live, {rest} -> {min} .. {rest}");
    Ok(())
}
