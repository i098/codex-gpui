#![cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::host_settings::Settings;
use common::*;
use gpui::Focusable as _;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use ui::{composer::Permission, dialog::Kind};

fn recording() -> Result<Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../refs/interactions/f3c.json"
    ))?)
}

fn fresh_settings(rec: &Value) -> Result<Settings> {
    let mut settings = Settings::default();
    for (key, value) in rec["fixtureState"]
        .as_object()
        .context("recorded initial host state")?
    {
        settings.write(key, value.clone())?;
    }
    Ok(settings)
}

fn announcement(lines: Vec<Value>, settings: Settings) -> Result<Fx> {
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    ensure!(fx.read(|root, cx| {
        root.dialog()
            .is_some_and(|dialog| dialog.read(cx).kind() == Kind::ModelAnnouncement)
    }));
    fx.shot()?;
    Ok(fx)
}

fn replay(fx: &mut Fx, steps: &Value) -> Result<()> {
    for step in steps.as_array().context("recorded steps")? {
        match step["type"].as_str() {
            Some("key") => fx.keys(step["portKey"].as_str().context("recorded key")?)?,
            Some("click") => {
                let at = if step["at"].is_object() {
                    &step["at"]
                } else {
                    &step["target"]
                };
                fx.click((
                    at["x"].as_f64().context("recorded x")? as f32,
                    at["y"].as_f64().context("recorded y")? as f32,
                ))?;
            }
            other => bail!("unsupported recorded input {other:?}"),
        }
        fx.shot()?;
    }
    Ok(())
}

fn evidence_directory() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("cgx-f3c-port-{}", std::process::id()));
    fs::create_dir_all(&path)?;
    Ok(path)
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn unseen_eligible_model_opens_the_recorded_surface() -> Result<()> {
    let rec = recording()?;
    let mut fx = announcement(script(None, |_| {})?, fresh_settings(&rec)?)?;
    let dialog = fx
        .read(|root, _| root.dialog().cloned())
        .context("eligible announcement absent")?;
    ensure!(fx.read(|_, cx| dialog.read(cx).kind() == Kind::ModelAnnouncement));
    let image = fx.shot()?;
    let scale = image.width() as f32 / 1280.;
    ensure!(
        image
            .get_pixel((420. * scale) as u32, (450. * scale) as u32)
            .0
            == [45, 45, 45, 255],
        "announcement body differs from the original opaque surface"
    );
    image.save(evidence_directory()?.join("announcement-open.png"))?;
    Ok(())
}

#[test]
fn announcement_contains_the_recorded_focus_cycle() -> Result<()> {
    let rec = recording()?;
    let mut fx = announcement(script(None, |_| {})?, fresh_settings(&rec)?)?;
    let dialog = fx
        .read(|root, _| root.dialog().cloned())
        .context("eligible announcement absent")?;
    let expected = rec["branches"]["secondary"]["focus"]
        .as_array()
        .context("original focus cycle")?;
    for (index, label) in expected.iter().enumerate() {
        if index != 0 {
            fx.keys(if index == 4 { "shift-tab" } else { "tab" })?;
        }
        fx.cx.update_window(fx.window, |_, window, cx| {
            let dialog = dialog.read(cx);
            ensure!(
                dialog.focus_handle(cx).is_focused(window),
                "announcement focus escaped"
            );
            ensure!(
                dialog.focused_control() == label.as_str().context("original focused control")?
            );
            Ok::<_, anyhow::Error>(())
        })??;
    }
    fx.shot()?
        .save(evidence_directory()?.join("announcement-focus.png"))?;
    Ok(())
}

#[test]
fn dismissals_persist_history_and_do_not_reopen_after_reload() -> Result<()> {
    let rec = recording()?;
    let key = "seen-model-upgrade-list";
    for branch in ["secondary", "escape", "close", "outside"] {
        let path = evidence_directory()?.join(format!("{branch}-host-settings.json"));
        let mut settings = Settings::load(path.clone())?;
        settings.write(key, rec["fixtureState"][key].clone())?;
        let mut fx = announcement(script(None, |_| {})?, settings)?;
        let before = fx.sent()?.len();
        ensure!(
            fx.read(|_, cx| cx.global::<Settings>().read_all()[key] == rec["fixtureState"][key])
        );
        let original_model = fx.read(|root, _| root.config()["model"].clone());
        replay(
            &mut fx,
            if branch == "secondary" {
                &rec["steps"]
            } else {
                &rec["branches"][branch]["steps"]
            },
        )?;
        ensure!(
            fx.read(|root, _| root.dialog().is_none()),
            "{branch}: dialog remains open"
        );
        ensure!(
            !fx.composer_focused()?,
            "{branch}: dismissal focused the composer"
        );
        ensure!(fx.read(|root, _| root.config()["model"] == original_model));
        ensure!(
            fx.sent()?[before..].is_empty(),
            "{branch}: dismissal wrote app-server state"
        );
        let stored: Value = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(
            stored[key] == rec["branches"]["secondary"]["stored"]["value"],
            "{branch}: actual stored history differs from the original"
        );
        drop(fx);
        let restarted = Fx::captured_with_host_settings(
            "dark",
            (1280., 832.),
            script(None, |_| {})?,
            Settings::load(path)?,
        )?;
        ensure!(
            restarted.read(|root, _| root.dialog().is_none()),
            "{branch}: stored history did not survive a fresh app context"
        );
    }
    Ok(())
}

#[test]
fn primary_success_updates_the_model_and_remembers_the_announcement() -> Result<()> {
    let rec = recording()?;
    let original = &rec["branches"]["primarySuccess"];
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "config/batchWrite",
        original["wireResponse"]["result"].clone(),
    ));
    let mut fx = announcement(lines, fresh_settings(&rec)?)?;
    let before = fx.sent()?.len();
    replay(&mut fx, &original["steps"])?;
    fx.wait("the recorded model change", |fx| {
        fx.read(|root, _| {
            root.config()["model"] == rec["announcement"]["model"]
                && root.config()["model_reasoning_effort"] == rec["announcement"]["reasoningEffort"]
        })
    })?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(fx.read(
        |_, cx| cx.global::<Settings>().read_all()["seen-model-upgrade-list"]
            == original["stored"]["seen-model-upgrade-list"]
    ));
    let expected: Vec<_> = original["requests"]
        .as_array()
        .context("original model write")?
        .iter()
        .map(|request| json!({"method": request["method"], "params": request["params"]}))
        .collect();
    ensure!(
        fx.sent()?[before..] == expected,
        "primary model write differs from the original"
    );
    fx.shot()?
        .save(evidence_directory()?.join("announcement-primary-success.png"))?;
    Ok(())
}

#[test]
fn primary_failure_keeps_the_model_and_still_remembers_the_announcement() -> Result<()> {
    let rec = recording()?;
    let original = &rec["branches"]["primaryReadOnly"];
    let mut lines = script(None, |_| {})?;
    let mut reply = exchange("config/batchWrite", Value::Null);
    reply[1]["msg"]
        .as_object_mut()
        .context("recorded reply")?
        .remove("result");
    reply[1]["msg"]["error"] = original["responses"][0]["error"].clone();
    lines.extend(reply);
    let mut fx = announcement(lines, fresh_settings(&rec)?)?;
    let original_model = fx.read(|root, _| root.config()["model"].clone());
    let before = fx.sent()?.len();
    replay(&mut fx, &original["steps"])?;
    let toast = original["toast"]
        .as_str()
        .context("original failure message")?;
    fx.wait("the recorded model failure", |fx| {
        fx.read(|root, cx| root.toasts(cx).iter().any(|title| title.as_ref() == toast))
    })?;
    ensure!(fx.read(|root, _| root.dialog().is_none() && root.config()["model"] == original_model));
    ensure!(fx.read(
        |_, cx| cx.global::<Settings>().read_all()["seen-model-upgrade-list"]
            == original["stored"]["value"]
    ));
    let expected: Vec<_> = original["requests"]
        .as_array()
        .context("original failed write")?
        .iter()
        .map(|request| json!({"method": request["method"], "params": request["params"]}))
        .collect();
    ensure!(
        fx.sent()?[before..] == expected,
        "failed model write differs from the original"
    );
    fx.shot()?
        .save(evidence_directory()?.join("announcement-primary-failure.png"))?;
    Ok(())
}

#[test]
fn selected_or_default_announced_model_persists_without_a_dialog() -> Result<()> {
    let rec = recording()?;
    let key = "seen-model-upgrade-list";
    for (index, model) in [Some(&rec["announcement"]["model"]), None]
        .into_iter()
        .enumerate()
    {
        let mut lines = script(None, |_| {})?;
        let config = lines
            .iter_mut()
            .find_map(|line| line.pointer_mut("/msg/result/config")?.as_object_mut())
            .context("recorded startup config")?;
        if let Some(model) = model {
            config.insert("model".into(), model.clone());
        } else {
            config.remove("model");
        }
        let path = evidence_directory()?.join(format!("selected-{index}-host-settings.json"));
        let mut settings = Settings::load(path.clone())?;
        settings.write(key, rec["fixtureState"][key].clone())?;
        let fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
        ensure!(fx.read(|root, _| root.dialog().is_none()));
        let stored: Value = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            stored[key] == rec["branches"]["secondary"]["stored"]["value"],
            "the selected or default model was not persisted"
        );
    }
    Ok(())
}

#[test]
fn changing_models_preserves_an_explicit_approval_choice() -> Result<()> {
    let rec = recording()?;
    let mut lines = script(None, |_| {})?;
    let config = lines
        .iter_mut()
        .find_map(|line| line.pointer_mut("/msg/result/config")?.as_object_mut())
        .context("recorded startup config")?;
    config.insert("approval_policy".into(), "never".into());
    config.insert("sandbox_mode".into(), "danger-full-access".into());
    lines.extend(exchange(
        "config/batchWrite",
        rec["branches"]["primarySuccess"]["wireResponse"]["result"].clone(),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let composer = fx.composer();
    ensure!(
        composer.read_with(&fx.cx, |composer, _| composer.permission()) == Permission::FullAccess
    );
    fx.shot()?;
    fx.click(fx.at("home", "Custom")?)?;
    fx.keys("down enter")?;
    ensure!(composer.read_with(&fx.cx, |composer, _| composer.permission()) == Permission::Ask);
    fx.click(fx.at("home", "GPT-6 Astra")?)?;
    fx.keys("down enter")?;
    fx.keys("down enter")?;
    fx.wait("the model write", |fx| {
        fx.read(|root, _| root.config()["model"] == rec["announcement"]["model"])
    })?;
    ensure!(
        composer.read_with(&fx.cx, |composer, _| composer.permission()) == Permission::Ask,
        "changing the model restored a broader approval mode"
    );
    Ok(())
}
