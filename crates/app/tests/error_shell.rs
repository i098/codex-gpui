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
        // Dark recovery surface (#000000), not the page surface.
        let shot = fx.shot()?;
        let corner = shot.get_pixel(8, 8);
        ensure!(corner.0[0] < 8, "recovery background: {corner:?}");
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
        // The blossom's center is hollow between the petals; probe the region.
        let loader = fx.shot()?;
        let (w, h) = (loader.width(), loader.height());
        let corner = loader.get_pixel(8, 8).0[0];
        let ink = (w / 2 - 120..w / 2 + 120)
            .step_by(4)
            .flat_map(|x| (h / 2 - 120..h / 2 + 120).step_by(4).map(move |y| (x, y)))
            .map(|(x, y)| loader.get_pixel(x, y).0[0])
            .max()
            .unwrap_or(0);
        ensure!(
            ink.abs_diff(corner) > 60,
            "blossom visible near center: ink={ink} corner={corner}"
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

/// Pixel pairs against the original error-shell refs (`refs/error-shell`, dark
/// 1280x832, captured through `tools/route1/exp-recovery.mjs`).
#[test]
#[cfg(target_os = "macos")]
fn error_shell_matches_the_original() -> Result<()> {
    let refs = std::path::Path::new(common::ROOT).join("refs");
    let out = std::env::temp_dir().join("cgx-error-shell-parity");
    let mut fx = Fx::crashed()?;
    fx.wait("crash page", |fx| fx.read(|root, _| root.crashed()))?;
    fx.advance(Duration::ZERO)?;
    let metrics = parity::diff_ref(&refs, &out, "error-shell/crash-page", "dark", &fx.shot()?)?;
    // Everything is bit-exact except the Try-again pill's corner arcs: GPUI's
    // rounded-corner coverage differs from Skia's (302 px measured, stable; a
    // y nudge scan found no position that improves it).
    ensure!(
        metrics.counts.diff_px_outside_text <= 320 && metrics.counts.text_diff_ratio <= 0.001,
        "crash page: outside={} text={:.3}%",
        metrics.counts.diff_px_outside_text,
        metrics.counts.text_diff_ratio * 100.
    );
    let mut fx = Fx::loading()?;
    ensure!(
        !fx.read(|root, _| root.ready()),
        "loader is visible pre-ready"
    );
    let metrics = parity::diff_ref(&refs, &out, "error-shell/loader", "dark", &fx.shot()?)?;
    // The blossom is a polychrome sprite; main's atlas-tile sample clamp
    // (26ba40dc) moves its soft edge samples by one gray level (460 px
    // measured, all off by exactly 1 of 255).
    ensure!(
        metrics.counts.diff_px_outside_text <= 480 && metrics.counts.text_diff_ratio <= 0.001,
        "loader: outside={} text={:.3}%",
        metrics.counts.diff_px_outside_text,
        metrics.counts.text_diff_ratio * 100.
    );
    Ok(())
}
