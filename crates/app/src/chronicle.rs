//! Settings ▸ Computer history's service: the original main process's `chronicle` service (`rS`)
//! and its recorder controller (`Fie`). The recorder is the signed Computer Use service, driven
//! by `ComputerUseIPCSkysight*Request` Apple Events through [`Host::skysight`]; the history is
//! the memory-summary files it writes under `$CODEX_HOME/memories/extensions/skysight/resources`.
use crate::{
    Root,
    host::Host,
    personalization::{read_memory, write_memory},
};
use anyhow::{Context as _, Result, bail};
use bridge::Bridge;
use gpui::{App, AppContext as _, WeakEntity};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use ui::screens::settings::{
    computer_history::{Reply, Service},
    personalization::MemoryChange,
};

/// Start and resume wait for macOS permission prompts (`FFe`); every other call uses `c7`.
const LONG_TIMEOUT: u32 = 115;
const TIMEOUT: u32 = 15;
const PLUGIN: &str = "computer-history@openai-bundled";

/// The activation state machine (`RI`): `idle`, `starting`, `waiting_for_permissions`, `failed`.
static ACTIVATION: Mutex<&str> = Mutex::new("idle");
/// Serializes recorder operations, as the controller's `runSerialized` does.
static CONTROL: Mutex<()> = Mutex::new(());

pub(crate) fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(Service {
        dispatch: Some(Arc::new(move |method, params, reply: Reply, cx| {
            root.update(cx, |root, cx| {
                let bridge = root.bridge.clone();
                let host = root.host.clone();
                let task = cx
                    .background_spawn(async move { call(&bridge, host.as_ref(), method, &params) });
                cx.spawn(async move |_, cx| {
                    let result = task.await;
                    if let Err(error) = &result {
                        eprintln!("codex-gpui: chronicle {method}: {error:#}");
                    }
                    cx.update(|cx| reply(result, cx));
                })
                .detach();
            })
            .ok();
        })),
    });
}

fn call(bridge: &Bridge, host: &dyn Host, method: &str, params: &Value) -> Result<Value> {
    match method {
        "availability" => Ok(json!({
            "service": host.skysight_available(),
            "allowed": allowed(bridge),
        })),
        "getState" => state(bridge, host),
        "setEnabled" => {
            if params["enabled"] == true {
                enable(bridge, host)
            } else {
                disable(bridge, host)
            }
        }
        "retryActivation" => {
            if configured(bridge)? {
                let _ = reconcile(bridge, host);
            }
            state(bridge, host)
        }
        "pause" => {
            let _guard = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
            let reply = skysight(
                host,
                "ComputerUseIPCSkysightPauseRequest",
                json!({}),
                TIMEOUT,
            )?;
            derive(true, &reply)
        }
        "resume" => {
            ensure_allowed(bridge)?;
            let _guard = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
            let status = skysight(
                host,
                "ComputerUseIPCSkysightStatusRequest",
                json!({}),
                TIMEOUT,
            )?;
            let reply = match status["state"].as_str() {
                Some("stopped") => skysight(
                    host,
                    "ComputerUseIPCSkysightStartRequest",
                    json!({}),
                    LONG_TIMEOUT,
                )?,
                Some("running") => status,
                _ => skysight(
                    host,
                    "ComputerUseIPCSkysightResumeRequest",
                    json!({}),
                    LONG_TIMEOUT,
                )?,
            };
            derive(configured(bridge)?, &reply)
        }
        "getSettings" => skysight(
            host,
            "ComputerUseIPCSkysightGetSettingsRequest",
            json!({}),
            TIMEOUT,
        ),
        "updateSettings" => skysight(
            host,
            "ComputerUseIPCSkysightUpdateSettingsRequest",
            json!({ "settings": params["settings"] }),
            TIMEOUT,
        ),
        "clearHistory" => {
            let mut request = json!({ "scope": params["scope"] });
            if let Some(interval) = params["interval"].as_object() {
                // The service counts seconds from 2001-01-01 (`l7`).
                let seconds = |key: &str| {
                    interval
                        .get(key)
                        .and_then(Value::as_f64)
                        .unwrap_or_default()
                        / 1000.
                        - 978_307_200.
                };
                request["interval"] =
                    json!({ "start": seconds("startMs"), "end": seconds("endMs") });
            }
            skysight(
                host,
                "ComputerUseIPCSkysightClearHistoryRequest",
                request,
                TIMEOUT,
            )
        }
        "listHistory" => Ok(Value::Array(list_history(&resources(), now_ms())?)),
        "listApplications" => list_applications(),
        "readMemories" => memories(bridge),
        "enableMemories" => {
            let read = read_memory(bridge)?;
            let write = write_memory(bridge, &read.settings, MemoryChange::Enabled(true));
            if let Some(error) = write.error {
                bail!("{error}");
            }
            memories(bridge)
        }
        "reveal" => reveal(params["path"].as_str().context("missing path")?),
        _ => bail!("unknown Computer history request {method}"),
    }
}

/// `g7`: retries transient Apple-Event failures and a not-yet-enabled service.
fn skysight(host: &dyn Host, request_type: &str, payload: Value, timeout: u32) -> Result<Value> {
    const TRANSIENT: [u64; 5] = [50, 100, 200, 400, 800];
    const NOT_ENABLED: [u64; 5] = [100, 250, 500, 1000, 2000];
    let (mut transient, mut not_enabled) = (0, 0);
    loop {
        let error = match host.skysight(request_type, &payload, timeout) {
            Ok(reply) => return Ok(reply),
            Err(error) => error,
        };
        let message = error.to_string();
        let delay = if ["-600 (", "-609 (", "-1708 (", "-1717 ("]
            .iter()
            .any(|code| message.starts_with(&format!("AESendMessage failed with {code}")))
        {
            transient += 1;
            TRANSIENT.get(transient - 1)
        } else if message
            .to_lowercase()
            .contains("computer history is not enabled for this user")
        {
            not_enabled += 1;
            NOT_ENABLED.get(not_enabled - 1)
        } else {
            None
        };
        match delay {
            Some(ms) => std::thread::sleep(Duration::from_millis(*ms)),
            None => return Err(error),
        }
    }
}

/// The service's permission answer in an Apple-Event error: `-10009` denied, `-10014` pending.
fn permission(error: &anyhow::Error) -> Option<&'static str> {
    error.chain().find_map(|cause| {
        let code = cause
            .to_string()
            .strip_prefix("Codex Computer Use Apple Event error ")?
            .split(':')
            .next()?
            .parse::<i64>()
            .ok()?;
        match code {
            -10009 => Some("denied"),
            -10014 => Some("pending"),
            _ => None,
        }
    })
}

fn set_activation(state: &'static str) {
    *ACTIVATION.lock().unwrap_or_else(|e| e.into_inner()) = state;
}

/// `#t`: the activation state only matters while enabled with the recorder stopped.
fn derive(enabled: bool, reply: &Value) -> Result<Value> {
    let recorder = reply["state"].as_str().context("invalid recorder status")?;
    let activation = if !enabled || recorder != "stopped" {
        "idle"
    } else {
        *ACTIVATION.lock().unwrap_or_else(|e| e.into_inner())
    };
    Ok(json!({ "enabled": enabled, "recorderState": recorder, "activationState": activation }))
}

fn state(bridge: &Bridge, host: &dyn Host) -> Result<Value> {
    let status = skysight(
        host,
        "ComputerUseIPCSkysightStatusRequest",
        json!({}),
        TIMEOUT,
    )?;
    derive(configured(bridge)?, &status)
}

/// `isChronicleFeatureConfigured`: allowed, with `features.memories` and `features.chronicle` on.
fn configured(bridge: &Bridge) -> Result<bool> {
    let config = request(bridge, "config/read", json!({ "includeLayers": false }))?;
    let features = &config["config"]["features"];
    Ok(allowed(bridge) && features["memories"] == true && features["chronicle"] == true)
}

/// `configRequirements` allows Chronicle unless it sets it to `false`; a failed read allows it.
fn allowed(bridge: &Bridge) -> bool {
    request(bridge, "configRequirements/read", json!({}))
        .map(|read| read["requirements"]["featureRequirements"]["chronicle"] != false)
        .unwrap_or(true)
}

fn ensure_allowed(bridge: &Bridge) -> Result<()> {
    anyhow::ensure!(
        allowed(bridge),
        "Computer History is disabled by your organization"
    );
    Ok(())
}

fn write_chronicle(bridge: &Bridge, enabled: bool) -> Result<()> {
    request(
        bridge,
        "config/value/write",
        json!({
            "keyPath": "features.chronicle", "value": enabled,
            "mergeStrategy": "upsert", "expectedVersion": null
        }),
    )
    .map(|_| ())
}

/// `reconcileComputerHistoryPluginInstallation`: the bundled Computer History plugin is
/// installed while the feature is on and removed when it is turned off.
fn reconcile_plugin(bridge: &Bridge, installed: bool) -> Result<()> {
    let list = request(bridge, "plugin/list", json!({ "cwds": [] }))?;
    let found = list["marketplaces"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|market| {
            let plugin = market["plugins"]
                .as_array()?
                .iter()
                .find(|p| p["id"] == PLUGIN)?;
            Some((market["path"].clone(), plugin["installed"] == true))
        });
    match found {
        Some((path, false)) if installed => request(
            bridge,
            "plugin/install",
            json!({ "marketplacePath": path, "pluginName": "computer-history" }),
        )
        .map(|_| ()),
        Some((_, true)) if !installed => {
            request(bridge, "plugin/uninstall", json!({ "pluginId": PLUGIN })).map(|_| ())
        }
        _ => Ok(()),
    }
}

/// `setEnabled(true)`: start the recorder, then save `features.chronicle`. A pending permission
/// keeps it on and waiting; a denied one turns it back off.
fn enable(bridge: &Bridge, host: &dyn Host) -> Result<Value> {
    ensure_allowed(bridge)?;
    let _guard = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
    set_activation("starting");
    if let Err(error) = reconcile_plugin(bridge, true) {
        eprintln!("codex-gpui: install the Computer History plugin: {error:#}");
    }
    match skysight(
        host,
        "ComputerUseIPCSkysightStartRequest",
        json!({}),
        LONG_TIMEOUT,
    ) {
        Ok(reply) => {
            write_chronicle(bridge, true)?;
            set_activation(if reply["state"] == "stopped" {
                "failed"
            } else {
                "idle"
            });
            derive(true, &reply)
        }
        Err(error) => match permission(&error) {
            Some("pending") => {
                write_chronicle(bridge, true)?;
                set_activation("waiting_for_permissions");
                derive(true, &json!({ "state": "stopped" }))
            }
            Some(_) => {
                rollback(bridge, host);
                set_activation("idle");
                derive(false, &json!({ "state": "stopped" }))
            }
            None => {
                rollback(bridge, host);
                set_activation("failed");
                Err(error)
            }
        },
    }
}

fn rollback(bridge: &Bridge, host: &dyn Host) {
    let stop = skysight(
        host,
        "ComputerUseIPCSkysightStopRequest",
        json!({}),
        TIMEOUT,
    );
    let write = write_chronicle(bridge, false);
    for error in [stop.err(), write.err()].into_iter().flatten() {
        eprintln!("codex-gpui: Chronicle rollback: {error:#}");
    }
}

/// `reconcileEnabled`: start the recorder when it is stopped.
fn reconcile(bridge: &Bridge, host: &dyn Host) -> Result<Value> {
    let _guard = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
    set_activation("starting");
    if let Err(error) = reconcile_plugin(bridge, true) {
        eprintln!("codex-gpui: install the Computer History plugin: {error:#}");
    }
    let result = skysight(
        host,
        "ComputerUseIPCSkysightStatusRequest",
        json!({}),
        TIMEOUT,
    )
    .and_then(|status| {
        if status["state"] == "stopped" {
            skysight(
                host,
                "ComputerUseIPCSkysightStartRequest",
                json!({}),
                LONG_TIMEOUT,
            )
        } else {
            Ok(status)
        }
    });
    match &result {
        Ok(reply) => set_activation(if reply["state"] == "stopped" {
            "failed"
        } else {
            "idle"
        }),
        Err(error) => match permission(error) {
            Some("pending") => set_activation("waiting_for_permissions"),
            Some(_) => {
                // `disableAfterPermissionsDenied`.
                set_activation(if write_chronicle(bridge, false).is_ok() {
                    "idle"
                } else {
                    "failed"
                });
            }
            None => set_activation("failed"),
        },
    }
    result
}

/// `setEnabled(false)`: stop the recorder (it must close its segment), then save the flag.
fn disable(bridge: &Bridge, host: &dyn Host) -> Result<Value> {
    let _guard = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
    let reply = skysight(
        host,
        "ComputerUseIPCSkysightStopRequest",
        json!({}),
        TIMEOUT,
    )?;
    anyhow::ensure!(
        reply["state"] == "stopped"
            && reply["currentSegmentEventsPath"].is_null()
            && reply["currentSegmentMetadataPath"].is_null(),
        "Chronicle stopped but did not close its current segment"
    );
    if let Err(error) = reconcile_plugin(bridge, false) {
        eprintln!("codex-gpui: remove the Computer History plugin: {error:#}");
    }
    write_chronicle(bridge, false)?;
    derive(false, &reply)
}

fn memories(bridge: &Bridge) -> Result<Value> {
    let read = read_memory(bridge)?;
    let settings = read.settings;
    Ok(json!({
        "available": settings.available,
        "featureEnabled": settings.feature_enabled,
        "generate": settings.generate,
        "use": settings.use_memories,
    }))
}

fn request(bridge: &Bridge, method: &'static str, params: Value) -> Result<Value> {
    pollster::block_on(bridge.request(method, params))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn resources() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
        })
        .join("memories/extensions/skysight/resources")
}

/// `jme`: `YYYY-MM-DDTHH-mm-ss-<zone>-(10min|6h)-memory-summary.md`, started at a UTC time.
fn summary_interval(name: &str) -> Option<(i64, i64, &'static str)> {
    let rest = name.strip_suffix("-memory-summary.md")?;
    let (rest, level, length) = if let Some(rest) = rest.strip_suffix("-10min") {
        (rest, "ten_minute", 600_000)
    } else {
        (rest.strip_suffix("-6h")?, "six_hour", 21_600_000)
    };
    let (stamp, zone) = rest.get(..19).zip(rest.get(19..))?;
    if !zone.starts_with('-') || zone.len() < 2 || zone[1..].contains('-') {
        return None;
    }
    let start = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H-%M-%S")
        .ok()?
        .and_utc()
        .timestamp_millis();
    Some((start, start + length, level))
}

/// `---` YAML frontmatter: `title`, `description`, `applications` and an optional `suggestion`.
/// shortcut: reads the YAML subset the recorder writes (scalars, a flow or block list, one
/// nested map); upgrade to a YAML parser if summaries start using other forms.
fn frontmatter(text: &str) -> Option<Value> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut yaml = json!({});
    let mut parent: Option<String> = None;
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with(char::is_whitespace);
        let line = line.trim();
        if let Some(item) = line.strip_prefix("- ") {
            let key = parent.as_deref()?;
            if !yaml[key].is_array() {
                yaml[key] = json!([]);
            }
            yaml[key].as_array_mut()?.push(unquote(item).into());
            continue;
        }
        let (key, value) = line.split_once(':')?;
        let (key, value) = (key.trim(), value.trim());
        if indented && let Some(parent) = &parent {
            yaml[parent.as_str()][key] = unquote(value).into();
        } else if value.is_empty() {
            parent = Some(key.to_owned());
            yaml[key] = json!({});
        } else if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            parent = None;
            yaml[key] = list
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| Value::from(unquote(item)))
                .collect();
        } else {
            parent = None;
            yaml[key] = unquote(value).into();
        }
    }
    if !closed {
        return None;
    }
    if yaml["applications"] == json!({}) {
        yaml["applications"] = json!([]);
    }
    let title = yaml["title"].as_str()?;
    let description = yaml["description"].as_str()?;
    let applications: Vec<&str> = yaml["applications"]
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<_>>()?;
    let suggestion = (|| {
        let s = &yaml["suggestion"];
        let kind = s["type"]
            .as_str()
            .filter(|t| matches!(*t, "skill" | "automation"))?;
        let name = s["name"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty())?;
        let description = s["description"]
            .as_str()
            .map(str::trim)
            .filter(|d| !d.is_empty())?;
        Some(json!({ "type": kind, "name": name, "description": description }))
    })();
    Some(json!({
        "title": title, "description": description,
        "applications": applications, "suggestion": suggestion,
    }))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let quoted = value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')));
    if quoted {
        &value[1..value.len() - 1]
    } else {
        value
    }
    .to_owned()
}

/// `Pme`/`Nw`: six-hour summaries, then ten-minute ones they do not cover, newest first. A
/// suggestion shows for two days.
fn list_history(dir: &Path, now: i64) -> Result<Vec<Value>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((start, end, level)) = summary_interval(&name) else {
            continue;
        };
        let Some(mut item) = std::fs::read_to_string(entry.path())
            .ok()
            .and_then(|text| frontmatter(&text))
        else {
            continue;
        };
        if now - start >= 172_800_000 {
            item["suggestion"] = Value::Null;
        }
        item["id"] = name.clone().into();
        item["interval"] = json!({ "startMs": start, "endMs": end });
        item["summaryLevel"] = level.into();
        item["memoryFilePath"] = entry.path().to_string_lossy().into_owned().into();
        items.push(item);
    }
    let six: Vec<(i64, i64)> = items
        .iter()
        .filter(|i| i["summaryLevel"] == "six_hour")
        .map(|i| (ms(i, "startMs"), ms(i, "endMs")))
        .collect();
    items.retain(|item| {
        item["summaryLevel"] == "six_hour" || {
            let (start, end) = (ms(item, "startMs"), ms(item, "endMs"));
            !six.iter().any(|(s, e)| *s <= start && end <= *e)
        }
    });
    items.sort_by_key(|item| std::cmp::Reverse(ms(item, "startMs")));
    Ok(items)
}

fn ms(item: &Value, key: &str) -> i64 {
    item["interval"][key].as_i64().unwrap_or_default()
}

/// `Lme`: installed apps by usage, without system agents. shortcut: no icons (the original reads
/// them through a native addon); upgrade when an app icon reader exists.
fn list_applications() -> Result<Value> {
    const HIDDEN: [&str; 9] = [
        "com.openai.sky.CUAService",
        "com.apple.WindowManager",
        "com.apple.controlcenter",
        "com.apple.dock",
        "com.apple.notificationcenterui",
        "com.apple.UserNotificationCenter",
        "com.apple.LocalAuthentication.UIAgent",
        "com.apple.LocalAuthenticationRemoteService",
        "com.apple.SecurityAgent",
    ];
    let apps = crate::host::native_desktop_apps(&json!({ "order": "usage" }))?;
    Ok(apps
        .into_iter()
        .filter(|app| !HIDDEN.contains(&app.bundle_id.as_str()))
        .map(|app| json!({ "bundleIdentifier": app.bundle_id, "displayName": app.display_name }))
        .collect())
}

fn reveal(path: &str) -> Result<Value> {
    anyhow::ensure!(cfg!(target_os = "macos"), "Reveal in Finder requires macOS");
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .status()?;
    anyhow::ensure!(status.success(), "Finder could not reveal {path}");
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_names_follow_the_original_pattern() {
        assert_eq!(
            summary_interval("2026-11-04T09-10-00-0800-10min-memory-summary.md"),
            Some((1_793_783_400_000, 1_793_784_000_000, "ten_minute"))
        );
        assert_eq!(
            summary_interval("2026-11-04T06-00-00-UTC-6h-memory-summary.md").map(|i| i.2),
            Some("six_hour")
        );
        for bad in [
            "2026-11-04T09-10-00-a-b-10min-memory-summary.md",
            "2026-11-04T09-10-00-10min-memory-summary.md",
            "2026-13-04T09-10-00-z-6h-memory-summary.md",
            "notes.md",
        ] {
            assert_eq!(summary_interval(bad), None, "{bad}");
        }
    }

    #[test]
    fn history_drops_covered_ten_minute_summaries_and_old_suggestions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let write = |name: &str, extra: &str| {
            std::fs::write(
                dir.path().join(name),
                format!(
                    "---\ntitle: {name}\ndescription: d\napplications: [com.apple.Safari]\n{extra}---\nbody"
                ),
            )
        };
        let suggestion = "suggestion:\n  type: skill\n  name: Budget\n  description: Draft it\n";
        write("2026-11-04T06-00-00-z-6h-memory-summary.md", "")?;
        write("2026-11-04T07-00-00-z-10min-memory-summary.md", "")?;
        write("2026-11-04T13-00-00-z-10min-memory-summary.md", suggestion)?;
        write("2026-11-01T13-00-00-z-10min-memory-summary.md", suggestion)?;
        std::fs::write(
            dir.path()
                .join("2026-11-04T14-00-00-z-10min-memory-summary.md"),
            "no frontmatter",
        )?;
        let now = summary_interval("2026-11-04T14-00-00-z-10min-memory-summary.md")
            .unwrap()
            .0;
        let items = list_history(dir.path(), now)?;
        let ids: Vec<_> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            [
                "2026-11-04T13-00-00-z-10min-memory-summary.md",
                "2026-11-04T06-00-00-z-6h-memory-summary.md",
                "2026-11-01T13-00-00-z-10min-memory-summary.md",
            ]
        );
        assert_eq!(items[0]["suggestion"]["name"], "Budget");
        assert!(items[2]["suggestion"].is_null());
        assert_eq!(items[0]["applications"], json!(["com.apple.Safari"]));
        Ok(())
    }

    #[test]
    fn frontmatter_reads_block_lists_quotes_and_rejects_missing_fields() {
        let text = "---\ntitle: \"Budget: Q3\"\ndescription: 'Sheets'\napplications:\n  - com.google.Chrome\n  - com.tinyspeck.slackmacgap\nsuggestion:\n  type: automation\n  name: Weekly\n  description: Send it\n---\n";
        let item = frontmatter(text).unwrap();
        assert_eq!(item["title"], "Budget: Q3");
        assert_eq!(item["description"], "Sheets");
        assert_eq!(
            item["applications"],
            json!(["com.google.Chrome", "com.tinyspeck.slackmacgap"])
        );
        assert_eq!(item["suggestion"]["type"], "automation");
        assert!(frontmatter("---\ntitle: t\napplications: []\n---\n").is_none());
        assert!(frontmatter("---\ntitle: t\ndescription: d\napplications: []\n").is_none());
    }

    #[test]
    fn permission_codes_come_from_the_service_error() {
        let pending = anyhow::anyhow!("Codex Computer Use Apple Event error -10014: pending");
        let denied = anyhow::anyhow!("Codex Computer Use Apple Event error -10009: no");
        let other = anyhow::anyhow!("AESendMessage failed with -600 (x)");
        assert_eq!(permission(&pending), Some("pending"));
        assert_eq!(permission(&denied), Some("denied"));
        assert_eq!(permission(&other), None);
    }
}
