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
            host.fetch(&Request::post("automation-create", Some(params)))
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

#[cfg(target_os = "macos")]
mod editor {
    use super::common::{Fx, RAIL_SCHEDULED};
    use anyhow::{Result, ensure};
    use app::host::{Host, LocalHost, Request, json_body};
    use std::{fs, sync::Arc, time::Duration};
    use ui::screens::automations::{CreateAutomation, CreationMode, EditAutomation};

    #[test]
    fn escape_closes_the_picker_before_discarding_the_editor() -> Result<()> {
        let home = tempfile::tempdir()?;
        let mut fx = Fx::new()?;
        fx.root.update(&mut fx.cx, |root, cx| {
            root.set_host(Arc::new(LocalHost::new(home.path())), cx)
        });
        fx.click(RAIL_SCHEDULED)?;
        fx.wait("initial task list", |fx| {
            fx.read(|root, _| root.automations().is_some())
        })?;
        for focus in 0..3 {
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_action(
                    Box::new(CreateAutomation {
                        mode: CreationMode::Manual,
                    }),
                    cx,
                )
            })?;
            fx.advance(Duration::from_millis(20))?;
            fx.type_text("Discard this draft")?;
            if focus > 0 {
                fx.keys("tab")?;
                fx.type_text("Do not save this prompt.")?;
            }
            if focus == 2 {
                fx.keys("tab enter down enter enter escape")?;
                ensure!(fx.read(|root, _| root.automation_editor().is_some()));
            }
            fx.keys("escape")?;
            fx.wait("Escape discards the editor", |fx| {
                fx.read(|root, _| root.automation_editor().is_none())
            })?;
        }
        ensure!(!home.path().join("automations").exists());
        Ok(())
    }

    #[test]
    fn manual_task_keeps_text_schedule_and_draft_through_save_failure_and_reload() -> Result<()> {
        let home = tempfile::tempdir()?;
        let mut fx = Fx::new()?;
        fx.root.update(&mut fx.cx, |root, cx| {
            root.set_host(Arc::new(LocalHost::new(home.path())), cx)
        });
        fx.click(RAIL_SCHEDULED)?;
        fx.wait("initial automation read", |fx| {
            fx.read(|root, _| root.automations().is_some())
        })?;
        fx.click((310., 110.))?;
        fx.click((310., 110.))?;
        fx.keys("enter")?;
        ensure!(fx.route() == app::Route::Scheduled);
        ensure!(fx.read(|root, _| root.dialog().is_none() && root.automation_editor().is_none()));
        if let Ok(dir) = std::env::var("CGX_AUTOMATION_EDITOR_SHOTS") {
            fs::create_dir_all(&dir)?;
            fx.shot()?.save(format!("{dir}/create-menu.png"))?;
        }
        fx.keys("down down enter")?;
        fx.advance(Duration::from_millis(20))?;
        ensure!(fx.read(|root, cx| {
            root.automation_editor()
                .unwrap()
                .read(cx)
                .draft(cx)
                .is_none()
        }));
        fx.type_text("Weekly review")?;
        fx.keys("tab")?;
        fx.type_text("Review local changes.")?;
        fx.keys("enter")?;
        fx.type_text("Include risks.")?;
        fx.keys("tab enter down down down enter")?;
        fx.keys("tab cmd-a")?;
        fx.type_text("18:35")?;
        let draft = fx.read(|root, cx| {
            root.automation_editor()
                .unwrap()
                .read(cx)
                .draft(cx)
                .unwrap()
        });
        ensure!(draft.name == "Weekly review");
        ensure!(draft.prompt == "Review local changes.\nInclude risks.");
        ensure!(draft.rrule == "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO;BYHOUR=18;BYMINUTE=35");
        if let Ok(dir) = std::env::var("CGX_AUTOMATION_EDITOR_SHOTS") {
            fs::create_dir_all(&dir)?;
            fx.shot()?.save(format!("{dir}/manual-weekly.png"))?;
        }
        fs::write(home.path().join("automations"), "owned write failure")?;
        fx.keys("cmd-enter")?;
        fx.wait("failed write retains the editor", |fx| {
            fx.read(|root, cx| {
                root.automation_editor()
                    .is_some_and(|view| !view.read(cx).saving())
            })
        })?;
        ensure!(fx.read(|root, cx| {
            root.automation_editor()
                .unwrap()
                .read(cx)
                .draft(cx)
                .is_some_and(|draft| draft.prompt == "Review local changes.\nInclude risks.")
        }));
        fs::remove_file(home.path().join("automations"))?;
        fx.keys("cmd-enter")?;
        fx.wait("manual create completes", |fx| {
            fx.read(|root, _| root.automation_editor().is_none())
        })?;
        let host = LocalHost::new(home.path());
        let saved =
            json_body(&host.fetch(&Request::post("list-automations", None))?)?["items"][0].clone();
        let id = saved["id"].as_str().unwrap().to_owned();
        ensure!(saved["name"] == "Weekly review");
        ensure!(saved["rrule"] == draft.rrule);
        fx.wait("created item reload", |fx| {
            fx.read(|root, _| {
                root.automations()
                    .is_some_and(|v| v.as_ref().is_ok_and(|v| v["items"][0]["id"] == id))
            })
        })?;
        fx.cx.update_window(fx.window, |_, window, cx| {
            window.dispatch_action(Box::new(EditAutomation { id: id.clone() }), cx);
        })?;
        fx.advance(Duration::from_millis(20))?;
        fx.keys("cmd-a")?;
        fx.type_text("Renamed review")?;
        fx.keys("cmd-enter")?;
        fx.wait("edited task saves", |fx| {
            fx.read(|root, _| root.automation_editor().is_none())
        })?;
        let reloaded = json_body(
            &LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?,
        )?;
        ensure!(reloaded["items"][0]["name"] == "Renamed review");
        ensure!(reloaded["items"][0]["prompt"] == saved["prompt"]);
        ensure!(reloaded["items"][0]["rrule"] == saved["rrule"]);
        ensure!(reloaded["items"][0]["createdAt"] == saved["createdAt"]);
        if let Ok(dir) = std::env::var("CGX_AUTOMATION_EDITOR_SHOTS") {
            fx.shot()?.save(format!("{dir}/saved-task.png"))?;
        }
        for pointer in [true, false] {
            fx.click(RAIL_SCHEDULED)?;
            fx.click((310., 110.))?;
            if pointer {
                fx.click((214., 148.))?;
            } else {
                fx.keys("down enter")?;
            }
            ensure!(fx.route() == app::Route::Home);
            ensure!(
                fx.read(|root, _| root.dialog().is_none() && root.automation_editor().is_none())
            );
            ensure!(fx.read(|root, cx| root.composer(cx).read(cx).field_text() ==
            "Let's set up a scheduled task together. First, explain how scheduled tasks work in ChatGPT. Then interview me to figure out what I need scheduled and when it should run."));
        }
        println!(
            "Native automation editor: Name, multiline Prompt, Weekly at 18:35, failed-write draft, create, edit and fresh-host reload passed."
        );
        Ok(())
    }

    #[test]
    fn schedule_controls_save_interval_presets_weekdays_month_days_and_custom_frequencies()
    -> Result<()> {
        let home = tempfile::tempdir()?;
        let mut fx = Fx::new()?;
        fx.root.update(&mut fx.cx, |root, cx| {
            root.set_host(Arc::new(LocalHost::new(home.path())), cx)
        });
        fx.click(RAIL_SCHEDULED)?;
        fx.wait("initial task list", |fx| {
            fx.read(|root, _| root.automations().is_some())
        })?;
        let rules = [
            "FREQ=MINUTELY;INTERVAL=2",
            "FREQ=DAILY;INTERVAL=1;BYHOUR=18;BYMINUTE=0",
            "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,TU,WE,TH,FR;BYHOUR=9;BYMINUTE=0",
            "FREQ=WEEKLY;INTERVAL=1;BYDAY=TU;BYHOUR=9;BYMINUTE=0",
            "FREQ=MONTHLY;INTERVAL=1;BYMONTHDAY=1,15;BYHOUR=9;BYMINUTE=0",
            "FREQ=YEARLY;INTERVAL=2;BYMONTHDAY=20;BYMONTH=10;BYHOUR=18;BYMINUTE=35",
            "FREQ=HOURLY;INTERVAL=2;BYMINUTE=35",
        ];
        for (index, expected) in rules.into_iter().enumerate() {
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_action(
                    Box::new(CreateAutomation {
                        mode: CreationMode::Manual,
                    }),
                    cx,
                )
            })?;
            fx.advance(Duration::from_millis(20))?;
            let name = format!("Schedule variant {index}");
            fx.type_text(&name)?;
            fx.keys("tab")?;
            fx.type_text("Review local changes.")?;
            fx.keys("tab enter")?;
            for _ in 0..index.min(5) {
                fx.keys("down")?;
            }
            fx.keys("enter")?;
            match index {
                0 => {
                    fx.keys("tab cmd-a")?;
                    fx.type_text("2")?;
                    fx.keys("tab enter down enter")?;
                    fx.keys("shift-tab cmd-a")?;
                    fx.type_text("0")?;
                    fx.keys("shift-tab enter down enter")?;
                    ensure!(
                        fx.read(|root, cx| root
                            .automation_editor()
                            .unwrap()
                            .read(cx)
                            .draft(cx)
                            .is_some_and(|draft| draft
                                .rrule
                                .starts_with("FREQ=DAILY;INTERVAL=1;"))),
                        "Daily inherited the hidden invalid interval"
                    );
                    fx.keys("enter up enter tab cmd-a")?;
                    fx.type_text("2")?;
                }
                1 => {
                    fx.keys("tab tab enter down down enter")?;
                }
                3 => {
                    fx.keys("tab tab space")?;
                    ensure!(fx.read(|root, cx| {
                        root.automation_editor()
                            .unwrap()
                            .read(cx)
                            .draft(cx)
                            .is_none()
                    }));
                    fx.keys("tab space")?;
                }
                4 => {
                    fx.keys("tab cmd-a")?;
                    fx.type_text("1,15")?;
                }
                5 => {
                    fx.keys("tab enter down down down enter tab cmd-a")?;
                    fx.type_text("2")?;
                    fx.keys("tab cmd-a")?;
                    fx.type_text("20")?;
                    fx.keys("tab cmd-a")?;
                    fx.type_text("10")?;
                    fx.keys("tab cmd-a")?;
                    fx.type_text("18:35")?;
                }
                6 => {
                    fx.keys("tab enter up enter tab cmd-a")?;
                    fx.type_text("2")?;
                    fx.keys("tab cmd-a")?;
                    fx.type_text("35")?;
                }
                _ => {}
            }
            let draft = fx.read(|root, cx| {
                root.automation_editor()
                    .unwrap()
                    .read(cx)
                    .draft(cx)
                    .unwrap()
            });
            ensure!(draft.rrule == expected, "variant {index}: {}", draft.rrule);
            if let Ok(dir) = std::env::var("CGX_AUTOMATION_EDITOR_SHOTS") {
                fs::create_dir_all(&dir)?;
                fx.shot()?.save(format!("{dir}/schedule-{index}.png"))?;
            }
            fx.keys("cmd-enter")?;
            fx.wait("variant saves", |fx| {
                fx.read(|root, _| root.automation_editor().is_none())
            })?;
            let loaded = json_body(
                &LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?,
            )?;
            let saved = loaded["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == name)
                .unwrap();
            ensure!(saved["rrule"] == expected);
            if index == 6 {
                let id = saved["id"].as_str().unwrap().to_owned();
                fx.cx.update_window(fx.window, |_, window, cx| {
                    window.dispatch_action(Box::new(EditAutomation { id: id.clone() }), cx);
                })?;
                fx.advance(Duration::from_millis(20))?;
                fx.keys("tab tab tab tab cmd-a")?;
                fx.type_text("3")?;
                fx.keys("cmd-enter")?;
                fx.wait("hourly edit saves", |fx| {
                    fx.read(|root, _| root.automation_editor().is_none())
                })?;
                let edited = json_body(
                    &LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?,
                )?;
                let item = edited["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item["id"] == id)
                    .unwrap();
                ensure!(
                    item["rrule"] == "FREQ=HOURLY;INTERVAL=3;BYMINUTE=35",
                    "Hourly edit changed the minute offset"
                );
            }
        }
        println!(
            "Native schedule controls: interval units, Evening preset, Weekdays, empty weekday rejection, Tuesday, month days, Yearly and Hourly all persisted."
        );
        Ok(())
    }
}
