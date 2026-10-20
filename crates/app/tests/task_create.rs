use anyhow::Result;
use app::host::{Host, LocalHost, Request};
use serde_json::json;

#[test]
fn invalid_local_schedules_leave_storage_unchanged() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let valid = json!({"name": "Review", "prompt": "Review local changes", "rrule": "FREQ=DAILY;BYHOUR=9;BYMINUTE=0", "cwds": [home.path()]});
    for (key, value) in [
        ("prompt", json!(" \n ")),
        ("rrule", json!("FREQ=DAILY;BYHOUR=24;BYMINUTE=0")),
        ("rrule", json!("FREQ=WEEKLY;BYDAY=XX;BYHOUR=9;BYMINUTE=0")),
        ("rrule", json!("FREQ=DAILY;BYHOUR=9;BYMINUTE=60")),
        ("cwds", json!(["relative"])),
        ("hostId", json!("remote")),
    ] {
        let mut params = valid.clone();
        params[key] = value;
        assert!(
            host.fetch(&Request::post("create-automation", Some(params)))
                .is_err()
        );
        assert!(!home.path().join("automations").exists());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn editable_local_schedule_survives_failure_and_reload() -> Result<()> {
    use anyhow::ensure;
    use app::host::json_body;
    use common::{Fx, RAIL_SCHEDULED};
    use std::{fs, sync::Arc};
    let home = tempfile::tempdir()?;
    let mut fx = Fx::new()?;
    fx.cx.update(|cx| {
        fx.root.update(cx, |root, cx| {
            root.set_host(Arc::new(LocalHost::new(home.path())), cx)
        })
    });
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("empty local list", |fx| {
        fx.read(|r, _| r.automations().is_some_and(Result::is_ok))
    })?;
    fx.click((130., 110.))?;
    fx.click((920., 467.))?;
    ensure!(fx.read(|r, _| r.dialog().is_some()));
    ensure!(!home.path().join("automations").exists());
    fx.click((500., 190.))?;
    fx.type_text("abcd")?;
    fx.keys("left left")?;
    fx.type_text("X")?;
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).query() == "abXcd"));
    fx.keys("shift-left")?;
    fx.type_text("Y")?;
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).query() == "abYcd"));
    fx.keys("cmd-a")?;
    fx.type_text("Line 0")?;
    for i in 1..10 {
        fx.keys("enter")?;
        fx.type_text(&format!("Line {i}"))?;
    }
    let bottom = fx.shot()?;
    fx.scroll((500., 190.), 500.)?;
    let top = fx.shot()?;
    ensure!(bottom != top, "long instructions must scroll");
    fx.keys("end")?;
    let restored = fx.shot()?;
    ensure!(restored != top, "the cursor line must return into view");
    fx.keys("cmd-a")?;
    fx.type_text("Review local changes")?;
    fx.keys("enter")?;
    fx.type_text("Include risks and tests.")?;
    let prompt = "Review local changes\nInclude risks and tests.";
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).query() == prompt));
    fx.click((900., 314.))?;
    fx.keys("down down down down down enter")?;
    fx.click((900., 362.))?;
    fx.type_text("25:90")?;
    fx.click((920., 467.))?;
    ensure!(!home.path().join("automations").exists());
    fx.click((900., 362.))?;
    fx.type_text("14:35")?;
    if let Ok(output) = std::env::var("CGX_TASK_CREATE_SHOTS") {
        fs::create_dir_all(&output)?;
        fx.shot()?.save(format!("{output}/after-editor.png"))?;
    }
    fs::write(home.path().join("automations"), "owned failure fixture")?;
    fx.click((920., 467.))?;
    fx.wait("failed create retains draft", |fx| {
        fx.read(|r, cx| r.dialog().is_some_and(|d| !d.read(cx).saving()))
    })?;
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).query() == prompt));
    fs::remove_file(home.path().join("automations"))?;
    fx.click((920., 467.))?;
    fx.wait("create completes", |fx| {
        fx.read(|r, _| r.dialog().is_none())
    })?;
    fx.wait("saved task reload", |fx| {
        fx.read(|r, _| {
            r.automations().is_some_and(|v| {
                v.as_ref()
                    .is_ok_and(|v| v["items"].as_array().is_some_and(|v| v.len() == 1))
            })
        })
    })?;
    drop(fx);

    let host = LocalHost::new(home.path());
    let loaded = json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    let item = &loaded["items"][0];
    ensure!(item["prompt"] == prompt);
    ensure!(item["rrule"] == "FREQ=WEEKLY;BYDAY=FR;BYHOUR=14;BYMINUTE=35");
    ensure!(item["status"] == "ACTIVE");
    let mut fx = Fx::new()?;
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root, cx| root.set_host(Arc::new(host), cx))
    });
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("new root reloads saved task", |fx| {
        fx.read(|r, _| {
            r.automations()
                .is_some_and(|v| v.as_ref().is_ok_and(|v| v["items"] == loaded["items"]))
        })
    })?;
    fx.click((150., 190.))?;
    fx.wait("saved task opens", |fx| {
        matches!(fx.route(), app::Route::Automation(_))
    })?;
    if let Ok(output) = std::env::var("CGX_TASK_CREATE_SHOTS") {
        fx.shot()?.save(format!("{output}/after-reload.png"))?;
    }
    println!(
        "Native local creation: edited instructions, Friday 14:35, invalid time rejected, failed write retained draft, one task survived a new Root and LocalHost."
    );
    Ok(())
}
