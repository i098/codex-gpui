#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use std::{cell::Cell, rc::Rc, time::Duration};
use ui::updater::{InstallUpdate, Release, Status, Updates};

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
                ..Default::default()
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
            ..Default::default()
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

#[test]
fn sidebar_banner_follows_the_update_until_dismissed() -> Result<()> {
    let mut fx = Fx::new()?;
    let installs = Rc::new(Cell::new(0));
    fx.cx.update(|cx| {
        let installs = installs.clone();
        cx.on_action(move |_: &InstallUpdate, _| installs.set(installs.get() + 1));
    });
    let release = Release {
        title: "26.1100.1".into(),
        notes: Some("https://updates.example/notes.html".into()),
    };
    for (name, status, visible, restart) in [
        ("idle", Status::Idle, false, false),
        ("checking", Status::Checking, false, false),
        ("downloading", Status::Downloading(Some(42)), true, false),
        ("installing", Status::Installing, true, false),
        ("ready", Status::Ready, true, true),
    ] {
        fx.cx.update(|cx| {
            cx.set_global(Updates {
                available: true,
                status,
                release: Some(release.clone()),
                ..Default::default()
            });
            cx.refresh_windows();
        });
        fx.advance(Duration::from_millis(20))?;
        ensure!(
            fx.cx.debug_bounds(fx.window, "update-banner")?.is_some() == visible,
            "{name}: incorrect banner visibility"
        );
        ensure!(
            fx.cx
                .debug_bounds(fx.window, "update-banner-restart")?
                .is_some()
                == restart,
            "{name}: Restart to update only on a staged update"
        );
        if visible {
            ensure!(
                fx.cx
                    .debug_bounds(fx.window, "update-banner-notes")?
                    .is_some(),
                "{name}: release notes link"
            );
        }
        if let Some(output) = std::env::var_os("CGX_UPDATER_SHOTS") {
            fx.shot()?
                .save(std::path::Path::new(&output).join(format!("banner-{name}.png")))?;
        }
    }
    let center = fx
        .cx
        .debug_bounds(fx.window, "update-banner-restart")?
        .context("restart")?
        .center();
    fx.click((center.x.into(), center.y.into()))?;
    ensure!(installs.get() == 1, "Restart to update installs");
    let center = fx
        .cx
        .debug_bounds(fx.window, "update-banner-dismiss")?
        .context("dismiss")?
        .center();
    fx.click((center.x.into(), center.y.into()))?;
    fx.advance(Duration::from_millis(20))?;
    ensure!(
        fx.cx.debug_bounds(fx.window, "update-banner")?.is_none(),
        "a dismissed release stays hidden"
    );
    fx.cx.update(|cx| {
        cx.global_mut::<Updates>().release = Some(Release {
            title: "26.1200.0".into(),
            notes: None,
        });
        cx.refresh_windows();
    });
    fx.advance(Duration::from_millis(20))?;
    ensure!(
        fx.cx.debug_bounds(fx.window, "update-banner")?.is_some(),
        "a newer release shows again"
    );
    Ok(())
}
