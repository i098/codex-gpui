//! Mobile setup rows 3, 5, 6 and 7 of docs/flows.md: `Allow` enables remote control, the waiting
//! step shows the pairing code, a failed pairing start keeps the step with its error, and a
//! claimed code opens the connected step and records the setup.

mod common;

use anyhow::{Result, ensure};
use common::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use ui::screens::connections::ConnectionsState;
use ui::screens::mobile::SetupStep;

fn status(state: &str) -> serde_json::Value {
    let environment = (state != "disabled").then_some("env");
    json!({"status": state, "serverName": "mac", "installationId": "inst", "environmentId": environment})
}

fn clients(ids: &[&str]) -> serde_json::Value {
    json!({"data": ids.iter().map(|id| json!({"clientId": id, "displayName": "Phone"})).collect::<Vec<_>>(), "nextCursor": null})
}

/// The app on the mobile page with remote control off, `exchanges` after the readback.
fn open(exchanges: Vec<[serde_json::Value; 2]>) -> Result<(Fx, tempfile::TempDir)> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("remoteControl/status/read", status("disabled")));
    lines.extend(exchanges.into_iter().flatten());
    let mut fx = Fx::scripted(lines)?;
    let home = tempfile::tempdir()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(app::host::LocalHost::new(home.path())), cx)
    });
    fx.action(Box::new(ui::OpenMobileSetup))?;
    fx.cx.run_until_parked();
    let page = fx.read(|root, _| root.mobile_page().unwrap().clone());
    fx.cx
        .update(|cx| page.update(cx, |page, cx| page.set_step(SetupStep::AllowHost, cx)));
    Ok((fx, home))
}

fn step(fx: &Fx) -> SetupStep {
    fx.read(|root, cx| root.mobile_page().unwrap().read(cx).step())
}

fn press_allow(fx: &mut Fx) -> Result<()> {
    // The centered `Allow` button of the left half at 1280x832.
    fx.click((320., 524.))?;
    Ok(())
}

#[test]
fn allow_enables_remote_control_and_a_claimed_code_connects() -> Result<()> {
    let (mut fx, home) = open(vec![
        exchange("remoteControl/enable", status("connected")),
        exchange("remoteControl/status/read", status("connected")),
        exchange("remoteControl/client/list", clients(&[])),
        exchange(
            "remoteControl/pairing/start",
            json!({"environmentId": "env", "expiresAt": 4_000_000_000_i64, "pairingCode": "QR-CODE", "manualPairingCode": "ABC-123"}),
        ),
        exchange("remoteControl/pairing/status", json!({"claimed": true})),
        exchange("remoteControl/client/list", clients(&["phone"])),
    ])?;
    ensure!(step(&fx) == SetupStep::AllowHost);
    press_allow(&mut fx)?;
    fx.wait("waiting step", |fx| step(fx) == SetupStep::Waiting)?;
    ensure!(step(&fx) == SetupStep::Waiting, "step {:?}", step(&fx));
    fx.wait("pairing code", |fx| {
        fx.read(|_, cx| cx.global::<ConnectionsState>().pairing.is_some())
    })?;
    let (pairing, error) = fx.read(|_, cx| {
        let state = cx.global::<ConnectionsState>();
        (state.pairing.clone(), state.pairing_error.clone())
    });
    ensure!(pairing.is_some_and(|p| p.manual_pairing_code.as_deref() == Some("ABC-123")));
    ensure!(error.is_none());
    // The poll sleeps one second of the test clock between reads.
    fx.wait("connected step", |fx| {
        fx.advance(Duration::from_secs(1)).is_ok() && step(fx) == SetupStep::Connected
    })?;
    ensure!(step(&fx) == SetupStep::Connected, "step {:?}", step(&fx));
    ensure!(fx.read(|_, cx| cx.global::<ConnectionsState>().setup_complete));
    fx.action(Box::new(ui::screens::mobile::FinishSetup))?;
    ensure!(
        fx.route() == app::Route::Home,
        "Done leaves for {:?}",
        fx.route()
    );
    drop(home);
    Ok(())
}

#[test]
fn a_failed_pairing_start_keeps_the_waiting_step_with_its_error() -> Result<()> {
    let (mut fx, _home) = open(vec![
        exchange("remoteControl/enable", status("connected")),
        exchange("remoteControl/status/read", status("connected")),
        exchange("remoteControl/client/list", clients(&[])),
    ])?;
    press_allow(&mut fx)?;
    fx.wait("waiting step", |fx| step(fx) == SetupStep::Waiting)?;
    ensure!(step(&fx) == SetupStep::Waiting, "step {:?}", step(&fx));
    fx.wait("pairing error", |fx| {
        fx.read(|_, cx| cx.global::<ConnectionsState>().pairing_error.is_some())
    })?;
    ensure!(step(&fx) == SetupStep::Waiting);
    Ok(())
}
