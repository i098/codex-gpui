#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn account_read_failure_shows_login_without_starting_authentication() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let id = lines
        .iter()
        .find(|line| line["dir"] == "send" && line["msg"]["method"] == "account/read")
        .context("recorded account read")?["msg"]["id"]
        .clone();
    let reply = lines
        .iter_mut()
        .find(|line| line["dir"] == "recv" && line["msg"]["id"] == id)
        .context("recorded account reply")?;
    reply["msg"].as_object_mut().unwrap().remove("result");
    reply["msg"]["error"] = json!({"code": -32603, "message": "Owned account fixture failure"});
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    ensure!(fx.read(|root, _| root.signed_out()));
    for key in ["cmd-n", "cmd-k", "cmd-o", "cmd-1", "cmd-[", "cmd-]"] {
        fx.keys(key)?;
    }
    ensure!(fx.read(|root, _| root.signed_out()
        && root.command_menu().is_none()
        && root.thread().is_none()));
    ensure!(!fx.cx.did_prompt_for_paths());
    ensure!(fx.sent()?.iter().all(|m| !matches!(
        m["method"].as_str(),
        Some("thread/read" | "thread/start" | "account/login/start" | "account/logout")
    )));
    Ok(())
}

#[test]
fn home_controls_follow_available_column_width_and_keep_the_folder_action() -> Result<()> {
    for (width, probe_x, folder_x, labels_visible) in [
        (600., 190., 112., true),
        (767., 480., 401., false),
        (1280., 500., 474., true),
    ] {
        let mut fx = Fx::with("dark", (width, 832.), None, |_| {})?;
        fx.advance(std::time::Duration::ZERO)?;
        let image = fx.reference_shot()?;
        let scale = image.width() as f32 / width;
        let bright = |[x, y, w, h]: [f32; 4]| {
            ((y * scale) as u32..((y + h) * scale) as u32)
                .flat_map(|y| ((x * scale) as u32..((x + w) * scale) as u32).map(move |x| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y).0[0] > 80)
                .count()
        };
        ensure!(
            (bright([probe_x, 681., 100., 28.]) > 0) == labels_visible,
            "{width}px utility labels differ"
        );
        if width == 600. {
            ensure!(
                bright([155., 776., 75., 28.]) > 0,
                "permission label collapsed too soon"
            );
            ensure!(
                bright([360., 776., 90., 28.]) > 50,
                "model label collapsed too soon"
            );
        }
        fx.click((folder_x, 695.))?;
        ensure!(fx.cx.did_prompt_for_paths());
        fx.cx.simulate_path_prompt_response(|_| None);
        fx.cx.run_until_parked();
        ensure!(fx.route() == app::Route::Home);
    }
    Ok(())
}

#[test]
fn missing_optional_account_metadata_keeps_the_draft_and_recovers_navigation() -> Result<()> {
    use app::host::{CloudHost, LocalHost};
    use std::sync::Arc;

    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.type_text("Keep this draft")?;
    let home = tempfile::tempdir()?;
    let host = Arc::new(CloudHost::new(
        LocalHost::new(home.path()),
        Box::new(|_| Ok(None)),
        "http://127.0.0.1:9",
    ));
    fx.cx
        .update(|cx| fx.root.update(cx, |root, cx| root.set_host(host, cx)));
    fx.wait("optional account metadata failure", |fx| {
        fx.read(|_, cx| !ui::HomeAccountAccess::available(cx))
    })?;
    fx.click((26., 158.))?;
    ensure!(fx.route() == app::Route::Home);
    ensure!(!fx.read(|root, _| root.signed_out()));
    ensure!(fx.composer_text()? == "Keep this draft");
    fx.keys("cmd-k")?;
    ensure!(fx.read(|root, _| root.command_menu().is_some()));
    fx.keys("escape")?;

    let host = Arc::new(LocalHost::new(home.path()));
    fx.cx
        .update(|cx| fx.root.update(cx, |root, cx| root.set_host(host, cx)));
    fx.wait("optional account metadata recovery", |fx| {
        fx.read(|_, cx| ui::HomeAccountAccess::available(cx))
    })?;
    ensure!(fx.composer_text()? == "Keep this draft");
    fx.click((26., 158.))?;
    ensure!(fx.route() == app::Route::Scheduled);
    ensure!(fx.sent()?.iter().all(|m| !matches!(
        m["method"].as_str(),
        Some("thread/start" | "account/login/start" | "account/logout")
    )));
    Ok(())
}
