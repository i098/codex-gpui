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
    let index = fx
        .read(|root, cx| {
            root.command_menu()
                .unwrap()
                .read(cx)
                .describe()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|(text, _)| {
                    text.split(" | ").next().unwrap().replace(['[', ']'], "") == "Configuration"
                })
        })
        .expect("Configuration command");
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    fx.wait("global configuration read", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .filter(|request| request["method"] == "configRequirements/read")
            .count()
            >= request_count
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
    lines.extend(exchange(
        "configRequirements/read",
        json!({"requirements": null}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    evidence(&mut fx, "configuration")?;
    fx.click((1100., 278.))?;
    evidence(&mut fx, "approval-menu")?;
    fx.keys("enter")?;
    fx.wait("approval choice", |fx| {
        fx.read(|root, _| root.config()["approval_policy"] == "on-request")
    })?;
    fx.click((1100., 339.))?;
    fx.keys("down")?;
    evidence(&mut fx, "sandbox-menu")?;
    fx.keys("enter")?;
    fx.wait("sandbox choice", |fx| {
        fx.read(|root, _| root.config()["sandbox_mode"] == "workspace-write")
    })?;
    fx.keys("cmd-[")?;
    open_configuration(&mut fx, 2)?;
    evidence(&mut fx, "reopened")?;
    fx.read(|root, cx| {
        let page = root.settings_page().unwrap().read(cx);
        assert_eq!(page.choice("config.approval_policy", ""), "on-request");
        assert_eq!(page.choice("config.sandbox_mode", ""), "workspace-write");
    });
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "config/batchWrite")
        .collect();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0]["params"]["edits"][0]["keyPath"],
        "approval_policy"
    );
    assert_eq!(writes[1]["params"]["edits"][0]["keyPath"], "sandbox_mode");
    assert_eq!(writes[1]["params"]["edits"][0]["value"], "workspace-write");
    assert!(
        writes
            .iter()
            .all(|request| request["params"]["filePath"].is_null())
    );
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
    fx.click((1100., 278.))?;
    fx.keys("enter down enter")?;
    fx.click((1100., 339.))?;
    fx.keys("enter down enter")?;
    assert!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "config/batchWrite")
    );
    assert_eq!(
        fx.read(|root, _| root.config()["sandbox_mode"].clone()),
        "read-only"
    );
    evidence(&mut fx, "managed")?;
    Ok(())
}

#[test]
fn failed_permission_write_keeps_the_saved_value() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/read", global_config()));
    let mut failed = exchange("config/batchWrite", Value::Null);
    failed[1]["msg"].as_object_mut().unwrap().remove("result");
    failed[1]["msg"]["error"] =
        json!({"code": -32000, "message": "owned fixture cannot write config.toml"});
    lines.extend(failed);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    fx.click((1100., 278.))?;
    fx.keys("enter")?;
    fx.wait("configuration save failure", |fx| {
        fx.read(|root, cx| {
            root.settings_page()
                .unwrap()
                .read(cx)
                .configuration_error()
                .is_some_and(|error| error.contains("Couldn't update this setting"))
        })
    })?;
    assert_eq!(
        fx.read(|root, _| root.config()["approval_policy"].clone()),
        "never"
    );
    fx.read(|root, cx| {
        let page = root.settings_page().unwrap().read(cx);
        assert_eq!(page.choice("config.approval_policy", ""), "never");
    });
    evidence(&mut fx, "failed-write")?;
    Ok(())
}

#[test]
fn access_choices_save_and_reopen() -> Result<()> {
    let mut config = global_config();
    config["config"]["sandbox_mode"] = json!("workspace-write");
    config["config"]["sandbox_workspace_write"] = json!({"network_access": false});
    config["config"]["web_search"] = Value::Null;
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/read", config.clone()));
    for _ in 0..4 {
        lines.extend(exchange("config/batchWrite", json!({"status": "ok"})));
    }
    config["config"]["sandbox_workspace_write"]["network_access"] = json!(true);
    config["config"]["web_search"] = json!("live");
    lines.extend(exchange("config/read", config));
    lines.extend(exchange(
        "configRequirements/read",
        json!({"requirements": null}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    evidence(&mut fx, "access-configuration")?;
    fx.click((1140., 400.))?;
    fx.wait("network access saved", |fx| {
        fx.read(|root, _| root.config()["sandbox_workspace_write"]["network_access"] == true)
    })?;
    for (key, expected) in [("space", false), ("enter", true)] {
        fx.click((1100., 461.))?;
        fx.keys("escape shift-tab")?;
        fx.keys(key)?;
        fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key)?,
        }))?;
        fx.wait("keyboard network access saved", |fx| {
            fx.read(|root, _| {
                root.config()["sandbox_workspace_write"]["network_access"] == expected
            })
        })?;
    }
    fx.click((1100., 461.))?;
    evidence(&mut fx, "web-search-menu")?;
    fx.keys("end enter")?;
    fx.wait("live search saved", |fx| {
        fx.read(|root, _| root.config()["web_search"] == "live")
    })?;
    fx.keys("cmd-[")?;
    open_configuration(&mut fx, 2)?;
    fx.read(|root, cx| {
        let page = root.settings_page().unwrap().read(cx);
        assert_eq!(page.choice("config.web_search", ""), "live");
    });
    evidence(&mut fx, "access-reopened")?;
    Ok(())
}

#[test]
fn managed_access_does_not_accept_pointer_or_keyboard_changes() -> Result<()> {
    let mut config = global_config();
    config["config"]["sandbox_mode"] = json!("workspace-write");
    config["config"]["sandbox_workspace_write"] = json!({"network_access": false});
    config["config"]["web_search"] = json!("cached");
    for key in ["sandbox_workspace_write", "web_search"] {
        config["origins"][key] = json!({"name": {"type": "mdm"}});
    }
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/read", config));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    fx.click((1140., 400.))?;
    fx.keys("space enter")?;
    fx.click((1100., 461.))?;
    fx.keys("end enter")?;
    assert!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "config/batchWrite")
    );
    fx.read(|root, cx| {
        assert_eq!(
            root.settings_page()
                .unwrap()
                .read(cx)
                .choice("config.web_search", ""),
            "cached"
        );
    });
    evidence(&mut fx, "managed-access")?;
    Ok(())
}

#[test]
fn proxy() {
    proxy_child();
}
