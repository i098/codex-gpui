//! Settings ▸ Mini & Pets through the real `Root`: pick a pet, open Customize, and pick
//! `None`, which hides the preview as the original does.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;

#[test]
fn proxy() {
    common::proxy_child();
}

fn click(fx: &mut Fx, selector: &str) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, selector)?
        .with_context(|| format!("{selector} is not on screen"))?;
    let center = bounds.center();
    fx.click((f32::from(center.x), f32::from(center.y)))
}

fn visible(fx: &mut Fx, selector: &str) -> bool {
    fx.cx
        .debug_bounds(fx.window, selector)
        .is_ok_and(|b| b.is_some())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_MINI_PETS_EVIDENCE", name)
}

#[test]
fn choose_customize_and_hide() -> Result<()> {
    let mut fx = Fx::new()?;
    let link = app::deep_link::parse("codex://pets/install").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("pet grid", |fx| visible(fx, "pets-tile-dewey"))?;
    ensure!(
        visible(&mut fx, "pets-customize"),
        "the default pet shows its preview"
    );
    shot(&mut fx, "page")?;

    click(&mut fx, "pets-tile-dewey")?;
    click(&mut fx, "pets-customize")?;
    fx.wait("Customize dialog", |fx| {
        visible(fx, "pets-customize-dialog")
    })?;
    shot(&mut fx, "customize")?;
    fx.keys("escape")?;
    fx.wait("dialog closed", |fx| !visible(fx, "pets-customize-dialog"))?;

    click(&mut fx, "pets-tile-none")?;
    fx.wait("preview hidden", |fx| !visible(fx, "pets-customize"))?;
    shot(&mut fx, "none")?;
    Ok(())
}
