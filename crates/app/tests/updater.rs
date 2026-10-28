#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use std::{cell::Cell, rc::Rc, time::Duration};
use ui::updater::{InstallUpdate, Status, Updates};

#[test]
fn rail_requires_a_ready_update_and_an_enabled_policy() -> Result<()> {
    let mut fx = Fx::new()?;
    let installs = Rc::new(Cell::new(0));
    fx.cx.update(|cx| {
        let installs = installs.clone();
        cx.on_action(move |_: &InstallUpdate, _| installs.set(installs.get() + 1));
    });
    for (name, status, available, managed, visible) in [
        ("no-feed", Status::Ready, false, false, false),
        ("idle", Status::Idle, true, false, false),
        ("checking", Status::Checking, true, false, false),
        (
            "downloading",
            Status::Downloading(Some(50)),
            true,
            false,
            true,
        ),
        ("installing", Status::Installing, true, false, true),
        ("ready", Status::Ready, true, false, true),
        ("managed", Status::Ready, true, true, false),
    ] {
        fx.cx.update(|cx| {
            cx.set_global(Updates {
                available,
                managed,
                status,
            });
            cx.refresh_windows();
        });
        fx.advance(Duration::from_millis(20))?;
        let bounds = fx.cx.debug_bounds(fx.window, "rail-update")?;
        ensure!(
            bounds.is_some() == visible,
            "{name}: incorrect rail visibility"
        );
        let before = installs.get();
        if let Some(bounds) = bounds {
            let center = bounds.center();
            fx.click((center.x.into(), center.y.into()))?;
        }
        ensure!(
            installs.get() == before + usize::from(visible && status == Status::Ready),
            "{name}: only a ready update may request installation"
        );
        if let Some(output) = std::env::var_os("CGX_UPDATER_SHOTS") {
            fx.shot()?
                .save(std::path::Path::new(&output).join(format!("{name}.png")))?;
        }
    }
    fx.cx.update(|cx| {
        cx.set_global(Updates {
            available: true,
            managed: false,
            status: Status::Ready,
        });
        cx.refresh_windows();
    });
    fx.advance(Duration::from_millis(20))?;
    let center = fx
        .cx
        .debug_bounds(fx.window, "rail-update")?
        .context("ready update")?
        .center();
    fx.click((center.x.into(), center.y.into()))?;
    let before = installs.get();
    fx.keys("enter")?;
    ensure!(
        installs.get() == before + 1,
        "keyboard installation must remain available"
    );
    Ok(())
}
