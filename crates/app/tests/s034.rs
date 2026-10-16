use app::host_settings::Settings;
#[cfg(target_os = "macos")]
use serde_json::Value;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "s034-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn host_write_persists_and_failed_write_keeps_the_previous_value() -> anyhow::Result<()> {
    let dir = scratch();
    let path = dir.join("settings.json");
    fs::write(&path, r#"{"unrelated":"kept"}"#)?;
    let mut settings = Settings::load(path.clone())?;
    settings.write("mac-menu-bar-enabled", json!(false))?;
    let saved = Settings::load(path.clone())?;
    assert_eq!(saved.read_all()["mac-menu-bar-enabled"], false);
    assert_eq!(saved.read_all()["unrelated"], "kept");
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    assert!(settings.write("mac-menu-bar-enabled", json!(true)).is_err());
    assert_eq!(settings.read_all()["mac-menu-bar-enabled"], false);
    fs::remove_dir_all(dir)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod common;

#[test]
#[cfg(target_os = "macos")]
fn proxy() {
    common::proxy_child();
}

#[test]
#[cfg(target_os = "macos")]
fn recorded_switch_writes_and_back_navigation() -> anyhow::Result<()> {
    use app::Route;
    use common::*;

    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s034.json"))?;
    let dir = scratch();
    let path = dir.join("settings.json");
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.cx
        .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
    let before = fx.sent()?.len();
    let mut writes = rec["result"]["hostRequests"].as_array().unwrap().iter();
    for (index, step) in rec["steps"].as_array().unwrap().iter().enumerate() {
        if step["type"] == "key" {
            fx.keys(step["portKey"].as_str().unwrap())?;
        } else {
            let bbox = &step["target"]["bbox"];
            fx.click((
                (bbox["x"].as_f64().unwrap() + bbox["w"].as_f64().unwrap() / 2.) as f32,
                (bbox["y"].as_f64().unwrap() + bbox["h"].as_f64().unwrap() / 2.) as f32,
            ))?;
        }
        if step["target"]["text"] == "Show ChatGPT in the menu bar" {
            let request = writes.next().unwrap();
            assert_eq!(request["svc"], "settings");
            assert_eq!(request["method"], "write");
            let saved: Value = serde_json::from_slice(&fs::read(&path)?)?;
            let key = request["args"][0].as_str().unwrap();
            assert_eq!(
                saved[key], request["args"][1],
                "settings.write at step {index}"
            );
        }
        for dom in rec["result"]["dom"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["afterStep"] == index)
        {
            assert_eq!(fx.route(), Route::Settings);
            let checked = fx.cx.update(|cx| {
                cx.global::<Settings>()
                    .read_all()
                    .get("mac-menu-bar-enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            });
            assert_eq!(checked, dom["checked"] == "true", "checked at step {index}");
            let shot = fx.shot()?;
            let center = fx.at("settings", dom["aria"].as_str().unwrap())?;
            let bbox = &dom["bbox"];
            let left = center.0 - bbox["w"].as_f64().unwrap() as f32 / 2.;
            let top = center.1 - bbox["h"].as_f64().unwrap() as f32 / 2.;
            for sample in dom["samples"].as_array().unwrap() {
                // Preserve the canonical Mac layout; route1 on Linux places this switch 4.25 px left.
                let x = (left + sample["at"][0].as_f64().unwrap() as f32
                    - bbox["x"].as_f64().unwrap() as f32)
                    * shot.width() as f32
                    / 1280.;
                let y = (top + sample["at"][1].as_f64().unwrap() as f32
                    - bbox["y"].as_f64().unwrap() as f32)
                    * shot.height() as f32
                    / 832.;
                let expected: Vec<u8> = serde_json::from_value(sample["rgb"].clone())?;
                assert_eq!(
                    &shot.get_pixel(x as u32, y as u32).0[..3],
                    expected,
                    "switch pixel at step {index}"
                );
            }
            shot.save(dir.join(format!("step-{index}.png")))?;
        }
        if step["target"]["text"] == "Back" {
            assert_eq!(fx.route(), Route::Home);
            fx.cx
                .update(|cx| cx.set_global(Settings::load(path.clone()).unwrap()));
        }
    }
    assert!(writes.next().is_none());
    assert!(
        fx.sent()?[before..]
            .iter()
            .all(|r| r["method"] != "config/value/write"),
        "host settings must not be sent to the app-server"
    );
    println!(
        "S034: recorded off/on pixels, settings.write values, reload, and Back passed; screenshots: {}",
        dir.display()
    );
    Ok(())
}
