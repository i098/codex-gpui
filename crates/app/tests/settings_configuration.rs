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
    fx.wait("workspace dependency state", |fx| {
        fx.read(|root, cx| root.settings_page().unwrap().read(cx).dependencies_ready())
    })?;
    fx.advance(Duration::from_millis(100))?;
    Ok(())
}

fn evidence(fx: &mut Fx, name: &str) -> Result<()> {
    if let Ok(out) = std::env::var("CGX_CONFIGURATION_EVIDENCE") {
        std::fs::create_dir_all(&out)?;
        fx.advance(ui::motion::BASIC)?;
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
    fx.click((500., 278.))?;
    evidence(&mut fx, "scope-menu")?;
    fx.keys("down enter")?;
    fx.click((1100., 397.))?;
    evidence(&mut fx, "sandbox-baseline-menu")?;
    fx.keys("escape")?;
    fx.click((1100., 336.))?;
    evidence(&mut fx, "approval-menu")?;
    fx.keys("enter")?;
    fx.wait("approval choice", |fx| {
        fx.read(|root, _| root.config()["approval_policy"] == "on-request")
    })?;
    fx.click((1100., 397.))?;
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
    fx.click((1100., 336.))?;
    fx.keys("enter down enter")?;
    fx.click((1100., 397.))?;
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
    fx.click((1100., 336.))?;
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
    fx.click((500., 278.))?;
    evidence(&mut fx, "access-scope-menu")?;
    fx.keys("escape")?;
    fx.click((1200., 740.))?;
    fx.scroll((1200., 740.), -2000.)?;
    evidence(&mut fx, "configuration-bottom")?;
    fx.scroll((1200., 740.), 2000.)?;
    fx.click((1140., 458.))?;
    fx.wait("network access saved", |fx| {
        fx.read(|root, _| root.config()["sandbox_workspace_write"]["network_access"] == true)
    })?;
    fx.scroll((1200., 740.), -2000.)?;
    fx.click((1100., 476.))?;
    evidence(&mut fx, "effort-menu")?;
    fx.keys("escape")?;
    fx.click((1200., 740.))?;
    fx.scroll((1200., 740.), 2000.)?;
    for (key, expected) in [("space", false), ("enter", true)] {
        fx.click((1100., 519.))?;
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
    fx.click((1100., 519.))?;
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
    fx.click((1140., 458.))?;
    fx.keys("space enter")?;
    fx.click((1100., 519.))?;
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
fn response_detail_preferences_save_clear_and_reopen() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/read", global_config()));
    for _ in 0..3 {
        lines.extend(exchange("config/batchWrite", json!({"status": "ok"})));
    }
    let mut reloaded = global_config();
    reloaded["config"]["model_reasoning_summary"] = json!("none");
    reloaded["config"]["model_verbosity"] = Value::Null;
    lines.extend(exchange("config/read", reloaded));
    lines.extend(exchange(
        "configRequirements/read",
        json!({"requirements": null}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_configuration(&mut fx, 1)?;
    fx.click((1100., 518.))?;
    fx.keys("end enter")?;
    fx.wait("output detail", |fx| {
        fx.read(|root, _| root.config()["model_verbosity"] == "high")
    })?;
    fx.click((1100., 578.))?;
    fx.keys("end enter")?;
    fx.wait("reasoning summary", |fx| {
        fx.read(|root, _| root.config()["model_reasoning_summary"] == "none")
    })?;
    fx.click((1100., 518.))?;
    fx.keys("home enter")?;
    fx.wait("model default detail", |fx| {
        fx.read(|root, _| root.config()["model_verbosity"].is_null())
    })?;
    fx.keys("cmd-[")?;
    open_configuration(&mut fx, 2)?;
    fx.read(|root, cx| {
        let page = root.settings_page().unwrap().read(cx);
        assert_eq!(page.choice("config.model_verbosity", "default"), "default");
        assert_eq!(page.choice("config.model_reasoning_summary", ""), "none");
    });
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "config/batchWrite")
        .collect();
    assert_eq!(writes.len(), 3);
    assert!(writes[2]["params"]["edits"][0]["value"].is_null());
    Ok(())
}

#[test]
fn reasoning_preferences_persist_and_reset_a_hidden_default() -> Result<()> {
    use app::host_settings::Settings;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("host-settings.json");
    std::fs::write(
        &path,
        serde_json::to_vec(recorded_host_settings()?.read_all())?,
    )?;
    let settings = Settings::load(path.clone())?;
    let mut lines = script(None, |line| {
        if line["dir"] == "recv" && line["msg"]["result"]["config"].is_object() {
            line["msg"]["result"]["config"]["model_reasoning_effort"] = json!("ultra");
        }
        if line["dir"] == "recv" {
            if let Some(models) = line["msg"]["result"]["data"].as_array_mut() {
                for model in models {
                    if model["model"] == "gpt-6-astra"
                        && model.get("supportedReasoningEfforts").is_some()
                    {
                        model["defaultReasoningEffort"] = json!("medium");
                    }
                }
            }
        }
    })?;
    lines.extend(exchange("config/read", global_config()));
    lines.extend(exchange("config/batchWrite", json!({"status": "ok"})));
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    open_configuration(&mut fx, 1)?;
    fx.click((1100., 735.))?;
    fx.keys("end enter")?;
    fx.wait("hidden default effort reset", |fx| {
        fx.read(|root, _| root.config()["model_reasoning_effort"] == "medium")
    })?;
    let saved: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    assert!(
        !saved["enabled-reasoning-efforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|effort| effort == "ultra")
    );
    evidence(&mut fx, "reasoning-efforts-disabled")?;
    Ok(())
}

#[test]
fn open_config_preserves_existing_content_and_creates_missing_file() -> Result<()> {
    use app::host::LocalHost;
    use std::{os::unix::fs::PermissionsExt as _, sync::Arc};
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("profile");
    std::fs::create_dir(&home)?;
    let config_path = home.join("config.toml");
    let content = "approval_policy = \"never\"\n";
    std::fs::write(&config_path, content)?;
    let opener = directory.path().join("opener");
    let opened = directory.path().join("opener.args");
    std::fs::write(&opener, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\n")?;
    std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700))?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["id"] == 1 {
            line["msg"]["result"]["codexHome"] = json!(home);
        }
    }
    lines.extend(exchange("config/read", global_config()));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let host = Arc::new(LocalHost::new(&home).with_opener(opener));
    fx.cx
        .update(|cx| fx.root.update(cx, |root, cx| root.set_host(host, cx)));
    open_configuration(&mut fx, 1)?;
    fx.click((1080., 278.))?;
    fx.wait("existing config opened", |_| {
        std::fs::read_to_string(&opened).is_ok_and(|text| text.contains("config.toml"))
    })?;
    assert_eq!(std::fs::read_to_string(&config_path)?, content);
    let canonical = config_path.canonicalize()?;
    assert_eq!(
        std::fs::read_to_string(&opened)?.trim(),
        canonical.to_str().unwrap()
    );
    std::fs::remove_file(&config_path)?;
    std::fs::remove_file(&opened)?;
    fx.keys("enter")?;
    fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter")?,
    }))?;
    fx.wait("new config opened", |_| {
        std::fs::read_to_string(&opened).is_ok_and(|text| text.contains("config.toml"))
    })?;
    assert_eq!(std::fs::read_to_string(&config_path)?, "");
    Ok(())
}

#[test]
fn proxy() {
    proxy_child();
}
