//! Error shell (SH-145, SH-147): the crash page when a spawned app-server dies, and
//! the startup loader until the first answers arrive.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use std::time::Duration;
use ui::screens::recovery::TryAgain;

/// The crash page shows once the spawned app-server exits.
#[test]
fn crash_page_shows_when_the_app_server_dies() -> Result<()> {
    let mut fx = Fx::crashed()?;
    fx.wait("crash page", |fx| fx.read(|root, _| root.crashed()))?;
    fx.advance(Duration::ZERO)?;
    #[cfg(target_os = "macos")]
    {
        // Dark recovery surface (#141414), not the page surface (#303030+).
        let corner = fx.shot()?.get_pixel(8, 8);
        ensure!(corner.0[0] < 32, "recovery background: {corner:?}");
    }
    Ok(())
}

/// "Try again" spawns a new app-server; when it dies too, the crash page returns.
#[test]
fn try_again_respawns_the_app_server() -> Result<()> {
    // The respawn resolves `CODEX_BIN` at spawn time; the wrapper records the spawn
    // and dies at once, so the crash page returns.
    let dir = tempfile::tempdir()?;
    let probe = dir.path().join("spawned");
    let wrapper = dir.path().join("codex-false");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\necho x >> {}\nexec false\n", probe.display()),
    )?;
    std::fs::set_permissions(
        &wrapper,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )?;
    // `connect` runs the app-server in the fixture cwd; it must exist to spawn.
    std::fs::create_dir_all(common::CWD).ok();
    unsafe { std::env::set_var("CODEX_BIN", &wrapper) };
    let mut fx = Fx::crashed()?;
    fx.wait("crash page", |fx| fx.read(|root, _| root.crashed()))?;
    // The action handler lives in the rendered crash page.
    fx.advance(Duration::ZERO)?;
    fx.action(Box::new(TryAgain))?;
    fx.wait("crash page again", |fx| fx.read(|root, _| root.crashed()))?;
    let spawns = std::fs::read_to_string(&probe)
        .with_context(|| format!("spawn probe {}", probe.display()))?;
    let spawns = spawns.lines().filter(|l| !l.is_empty()).count();
    ensure!(spawns == 1, "restart spawned {spawns} app-servers");
    Ok(())
}

/// Until `initialize` answers, only the 56 px blossom on the window background shows.
#[test]
fn loader_shows_until_the_app_server_answers() -> Result<()> {
    let mut fx = Fx::loading()?;
    ensure!(
        !fx.read(|root, _| root.ready()),
        "loader is visible pre-ready"
    );
    #[cfg(target_os = "macos")]
    {
        let loader = fx.shot()?;
        let (w, h) = (loader.width(), loader.height());
        let center = loader.get_pixel(w / 2, h / 2);
        let corner = loader.get_pixel(8, 8);
        ensure!(
            center.0[0].abs_diff(corner.0[0]) > 60,
            "blossom visible at center: {center:?} vs {corner:?}"
        );
    }
    fx.wait("ready", |fx| fx.read(|root, _| root.ready()))?;
    #[cfg(target_os = "macos")]
    {
        let app = fx.shot()?;
        let center = app.get_pixel(app.width() / 2, app.height() / 2);
        ensure!(center.0[0] < 100, "loader replaced by the app: {center:?}");
    }
    Ok(())
}
