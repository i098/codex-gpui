#![cfg(target_os = "macos")]
mod common;

use anyhow::Result;
use common::*;
use serde_json::{Value, json};
use std::time::Duration;

fn global_config() -> Value {
    json!({
        "config": {"approval_policy": "never", "sandbox_mode": "read-only"},
        "origins": {},
    })
}

fn open_configuration(fx: &mut Fx, request_count: usize) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text("Configuration")?;
    let index = fx.read(|root, cx| {
        root.command_menu().unwrap().read(cx).describe().into_iter()
            .flat_map(|(_, rows)| rows)
            .position(|(text, _)| text.split(" | ").next().unwrap().replace(['[', ']'], "") == "Configuration")
    }).expect("Configuration command");
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    fx.wait("global configuration read", |fx| {
        fx.sent().unwrap().iter().filter(|request| request["method"] == "configRequirements/read").count() >= request_count
    })?;
    fx.sent()?;
    fx.cx.run_until_parked();
    fx.advance(Duration::from_millis(100))?;
    Ok(())
}

fn evidence(fx: &mut Fx, name: &str) -> Result<()> {
    if let Ok(out) = std::env::var("CGX_CONFIGURATION_EVIDENCE") {
        std::fs::create_dir_all(&out)?;
        fx.shot()?.save(format!("{out}/{name}.png"))?;
    }
    Ok(())
}

#[test]
fn permission_choices_save_and_reopen() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/read", global_config()));
    for _ in 0..2 {
        lines.extend(exchange("config/batchWrite", json!({"status": "ok"})));
    }
    let mut reloaded = global_config();
    reloaded["config"]["approval_policy"] = json!("on-request");
    reloaded["config"]["sandbox_mode"] = json!("workspace-write");
    lines.extend(exchange("config/read", reloaded));
    lines.extend(exchange("configRequirements/read", json!({"requirements": null})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    evidence(&mut fx, "configuration")?;
    fx.keys("tab enter")?;
    evidence(&mut fx, "approval-menu")?;
    fx.keys("enter")?;
    fx.wait("approval choice", |fx| fx.read(|root, _| root.config()["approval_policy"] == "on-request"))?;
    fx.keys("tab enter down")?;
    evidence(&mut fx, "sandbox-menu")?;
    fx.keys("enter")?;
    fx.wait("sandbox choice", |fx| fx.read(|root, _| root.config()["sandbox_mode"] == "workspace-write"))?;
    fx.keys("cmd-[")?;
    open_configuration(&mut fx, 2)?;
    evidence(&mut fx, "reopened")?;
    let writes: Vec<_> = fx.sent()?.into_iter().filter(|request| request["method"] == "config/batchWrite").collect();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0]["params"]["edits"][0]["keyPath"], "approval_policy");
    assert_eq!(writes[1]["params"]["edits"][0]["keyPath"], "sandbox_mode");
    assert_eq!(writes[1]["params"]["edits"][0]["value"], "workspace-write");
    assert!(writes.iter().all(|request| request["params"]["filePath"].is_null()));
    Ok(())
}

#[test]
fn managed_permissions_cannot_be_changed() -> Result<()> {
    let mut lines = script(None, |line| {
        if line["dir"] == "recv" && line["msg"]["id"] == 5 {
            line["msg"]["result"]["requirements"] = json!({
                "allowedApprovalPolicies": ["never"],
                "allowedSandboxModes": ["read-only"],
            });
        }
    })?;
    let mut config = global_config();
    for key in ["approval_policy", "sandbox_mode"] {
        config["origins"][key] = json!({"name": {"type": "mdm"}});
    }
    lines.extend(exchange("config/read", config));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    fx.keys("tab enter down enter tab enter down enter")?;
    assert!(!fx.sent()?.iter().any(|request| request["method"] == "config/batchWrite"));
    assert_eq!(fx.read(|root, _| root.config()["sandbox_mode"].clone()), "read-only");
    evidence(&mut fx, "managed")?;
    Ok(())
}

#[test]
fn failed_permission_write_keeps_the_saved_value() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/read", global_config()));
    let mut failed = exchange("config/batchWrite", Value::Null);
    failed[1]["msg"].as_object_mut().unwrap().remove("result");
    failed[1]["msg"]["error"] = json!({"code": -32000, "message": "owned fixture cannot write config.toml"});
    lines.extend(failed);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    fx.keys("tab enter enter")?;
    fx.wait("configuration save failure", |fx| fx.read(|root, cx| root.toasts(cx).iter().any(|title| title.contains("Couldn't update this setting"))))?;
    assert_eq!(fx.read(|root, _| root.config()["approval_policy"].clone()), "never");
    evidence(&mut fx, "failed-write")?;
    Ok(())
}

#[test]
fn proxy() {
    proxy_child();
}
