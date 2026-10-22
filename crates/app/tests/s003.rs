//! S003 replays the original sidebar states and its last-section paging behavior.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, ensure};
use bridge::Event;
use common::{Fx, ROOT, SUBTRACT, changed, exchange, proxy_child, script};
use serde_json::{Value, json};
use std::{fs, time::Duration};

fn recording() -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s003.json"
    ))?)?)
}

fn point(v: &Value) -> (f32, f32) {
    (
        v["x"].as_f64().unwrap() as f32,
        v["y"].as_f64().unwrap() as f32,
    )
}

fn target(step: &Value) -> (f32, f32) {
    if step["at"].is_object() {
        return point(&step["at"]);
    }
    let b = &step["target"]["bbox"];
    let (x, y) = point(b);
    (
        x + b["w"].as_f64().unwrap() as f32 / 2.,
        y + b["h"].as_f64().unwrap() as f32 / 2.,
    )
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn projects_and_row_status_follow_the_original() -> Result<()> {
    let rec = recording()?;
    let outcome = &rec["result"]["outcome"];
    let mut lines = script(None, |_| {})?;
    let list = lines
        .iter()
        .find(|line| line["msg"]["result"]["data"][0]["preview"].is_string())
        .expect("thread/list");
    let list = list["msg"]["result"].clone();
    lines.extend(exchange("thread/list", list));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.hover((1270., 820.))?;
    let rest = fx.shot()?;
    fx.hover((150., 336.))?;
    fx.advance(Duration::from_millis(600))?;
    let hovered = fx.shot()?;
    fx.hover((1270., 820.))?;
    for (i, step) in rec["steps"].as_array().unwrap().iter().enumerate() {
        match step["type"].as_str().unwrap() {
            "click" => fx.click(target(step))?,
            "hover" => fx.hover(target(step))?,
            "bridge" => {
                fx.root.update(&mut fx.cx, |r, cx| {
                    r.on_event(
                        Event::Notification {
                            method: step["method"].as_str().unwrap().into(),
                            params: step["params"].clone(),
                        },
                        cx,
                    )
                });
            }
            other => panic!("unexpected S003 step {other}"),
        }
        fx.advance(Duration::from_millis(step["waitMs"].as_u64().unwrap_or(0)))?;
        let image = fx.shot()?;
        if i <= 1 {
            let original = if i == 0 {
                &outcome["projectsCollapsed"]
            } else {
                &outcome["projectsExpanded"]
            };
            let collapsed = fx.read(|r, cx| r.sidebar().read(cx).projects_collapsed());
            assert_eq!(!collapsed, original["expanded"] == "true");
            let y = original["recents"]["y"].as_u64().unwrap() as u32;
            let crop = |img: &image::RgbaImage, y| {
                image::imageops::crop_imm(img, 128, y * 2, 360, 50).to_image()
            };
            assert_eq!(
                crop(&image, y),
                crop(&rest, 259),
                "Recents did not move with the Projects child list"
            );
        }
        if i == 2 || i == 3 {
            ensure!(
                fx.read(|r, cx| r.sidebar().read(cx).is_running(SUBTRACT)),
                "active thread lacks a running state"
            );
            ensure!(
                changed(&rest, &image, [304., 326., 20., 20.]) > 0,
                "Working spinner is absent"
            );
            image.save(format!("{}/s003-running.png", env!("CARGO_TARGET_TMPDIR")))?;
        }
        if i == 4 || i == 8 {
            image.save(format!(
                "{}/s003-hover-{i}.png",
                env!("CARGO_TARGET_TMPDIR")
            ))?;
            hovered.save(format!(
                "{}/s003-hover-before.png",
                env!("CARGO_TARGET_TMPDIR")
            ))?;
            ensure!(
                changed(&hovered, &image, [304., 326., 20., 20.]) == 0,
                "The status indicator remains over the hover actions at step {i}"
            );
        }
        if i == 7 {
            ensure!(
                !fx.read(|r, cx| r.sidebar().read(cx).is_running(SUBTRACT)),
                "idle thread keeps its spinner"
            );
            ensure!(
                fx.read(|r, cx| r.sidebar().read(cx).unread.iter().any(|id| id == SUBTRACT)),
                "completed background turn is not unread"
            );
            assert_eq!(
                &image.get_pixel(628, 672).0[..3],
                &[58, 131, 247],
                "Unread dot color differs from the recording"
            );
            image.save(format!("{}/s003-unread.png", env!("CARGO_TARGET_TMPDIR")))?;
        }
    }
    fx.click((150., 336.))?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(
        !fx.read(|r, cx| r.sidebar().read(cx).unread.iter().any(|id| id == SUBTRACT)),
        "Opening a thread did not clear its dot"
    );
    fx.shot()?
        .save(format!("{}/s003-row.png", env!("CARGO_TARGET_TMPDIR")))?;
    Ok(())
}

fn paged_script() -> Result<Vec<Value>> {
    let mut lines = script(None, |_| {})?;
    let response = lines
        .iter_mut()
        .find_map(|line| {
            line.pointer_mut("/msg/result/data").filter(|data| {
                data.as_array()
                    .is_some_and(|rows| rows.first().is_some_and(|t| t["preview"].is_string()))
            })
        })
        .expect("thread/list data");
    let template = response[0].clone();
    let threads: Vec<_> = (1..=40)
        .map(|n| {
            let mut thread = template.clone();
            thread["id"] = json!(format!("0199a003-0000-7000-8000-0000000000{n:02}"));
            thread["name"] = json!(format!("S003 thread {n:02}"));
            thread
        })
        .collect();
    *response = json!(threads[..10]);
    for line in &mut lines {
        if line
            .pointer("/msg/result/data")
            .is_some_and(|data| data == &json!(threads[..10]))
        {
            line["msg"]["result"]["nextCursor"] = json!("s003:10");
        }
    }
    for start in [10, 20, 30] {
        let mut page = exchange(
            "thread/list",
            json!({
                "data": threads[start..start + 10],
                "nextCursor": if start == 30 { None } else { Some(format!("s003:{}", start + 10)) },
            }),
        );
        page[0]["msg"]["params"] = json!({"cursor": format!("s003:{start}"), "limit": 10});
        lines.extend(page);
    }
    Ok(lines)
}

#[test]
fn recents_pages_by_ten_when_its_end_enters_the_view() -> Result<()> {
    let rec = recording()?;
    let paging = &rec["paging"];
    let mut fx = Fx::captured("dark", (1280., 832.), paged_script()?)?;
    let shown = |fx: &Fx| fx.read(|r, cx| r.sidebar().read(cx).recents_shown());
    let rest = paging["result"]["rest"]["rows"].as_array().unwrap().len();
    fx.wait("initial Recents page", |fx| {
        fx.shot().unwrap();
        shown(fx) == rest
    })?;
    for step in paging["steps"].as_array().unwrap() {
        for _ in 0..step["times"].as_u64().unwrap_or(1) {
            fx.scroll(target(step), -step["deltaY"].as_f64().unwrap() as f32)?;
            fx.advance(Duration::from_millis(200))?;
        }
        fx.advance(Duration::from_millis(step["waitMs"].as_u64().unwrap_or(0)))?;
    }
    let after = paging["result"]["afterScroll"]["rows"]
        .as_array()
        .unwrap()
        .len();
    fx.wait("Recents page after scroll", |fx| {
        fx.shot().unwrap();
        shown(fx) == after
    })?;
    let cursors: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "thread/list" && r["params"]["cursor"].is_string())
        .map(|r| r["params"].clone())
        .collect();
    assert_eq!(
        cursors,
        vec![
            json!({"cursor":"s003:10","limit":10}),
            json!({"cursor":"s003:20","limit":10}),
            json!({"cursor":"s003:30","limit":10}),
        ]
    );
    assert_eq!(fx.read(|r, _| r.recents()).len(), 40);
    fx.shot()?
        .save(format!("{}/s003-paging.png", env!("CARGO_TARGET_TMPDIR")))?;
    Ok(())
}

#[test]
fn failed_page_stops_loading_and_can_retry_the_same_cursor() -> Result<()> {
    let mut lines = paged_script()?;
    let page = lines
        .iter()
        .position(|line| line["msg"]["result"]["nextCursor"] == "s003:20")
        .unwrap()
        - 1;
    let mut refused = lines[page..page + 2].to_vec();
    refused[0]["msg"]["id"] = json!(99_999);
    refused[1]["msg"]["id"] = json!(99_999);
    refused[1]["msg"].as_object_mut().unwrap().remove("result");
    refused[1]["msg"]["error"] = json!({"code":-1,"message":"offline"});
    lines.splice(page..page, refused);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.wait("load error", |fx| {
        fx.shot().unwrap();
        fx.read(|r, cx| r.sidebar().read(cx).load_failed())
    })?;
    assert_eq!(fx.read(|r, _| r.recents()).len(), 10);
    fx.advance(Duration::from_secs(1))?;
    let cursors = |fx: &mut Fx| -> Result<Vec<Value>> {
        Ok(fx
            .sent()?
            .into_iter()
            .filter(|r| r["method"] == "thread/list" && r["params"]["cursor"].is_string())
            .collect())
    };
    assert_eq!(
        cursors(&mut fx)?.len(),
        1,
        "The failed page retried without a click"
    );
    fx.click((190., 612.))?;
    fx.wait("retry page", |fx| {
        fx.shot().unwrap();
        fx.read(|r, cx| r.sidebar().read(cx).recents_shown()) == 20
    })?;
    let sent = cursors(&mut fx)?;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["params"], sent[1]["params"]);
    ensure!(
        !fx.read(|r, cx| r.sidebar().read(cx).load_failed()),
        "The retry kept its error state"
    );
    Ok(())
}

#[test]
fn an_all_pinned_page_still_loads_older_recents() -> Result<()> {
    let mut lines = paged_script()?;
    for line in &mut lines {
        if line["msg"]["result"]["nextCursor"] == "s003:10" {
            for thread in line["msg"]["result"]["data"].as_array_mut().unwrap() {
                thread["section"] = json!({"id": ui::screens::sidebar::PINNED_SECTION_ID});
            }
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.wait("older Recents after pinned page", |fx| {
        fx.shot().unwrap();
        fx.read(|r, cx| r.sidebar().read(cx).recents_shown()) == 10
    })?;
    assert_eq!(fx.read(|r, _| r.recents()).len(), 20);
    let pages: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "thread/list" && r["params"]["cursor"].is_string())
        .collect();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["params"], json!({"cursor":"s003:10","limit":10}));
    Ok(())
}

#[test]
fn pinning_a_sibling_does_not_restart_a_finished_spinner() -> Result<()> {
    let mut lines = script(None, |line| {
        if let Some(threads) = line
            .pointer_mut("/msg/result/data")
            .and_then(Value::as_array_mut)
        {
            for thread in threads {
                if thread["id"] == SUBTRACT {
                    thread["status"] = json!({"type":"active","activeFlags":[]});
                }
            }
        }
    })?;
    lines.extend(exchange("thread/section/move", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    ensure!(
        fx.read(|r, cx| r.sidebar().read(cx).is_running(SUBTRACT)),
        "The initial active status lacks a spinner"
    );
    fx.root.update(&mut fx.cx, |r, cx| {
        r.on_event(
            Event::Notification {
                method: "thread/status/changed".into(),
                params: json!({"threadId": SUBTRACT, "status":{"type":"idle"}}),
            },
            cx,
        )
    });
    fx.hover((150., 305.))?;
    fx.advance(Duration::from_millis(150))?;
    ensure!(
        !fx.read(|r, cx| r.sidebar().read(cx).is_running(SUBTRACT)),
        "The idle event did not stop the spinner"
    );
    fx.click((286., 305.))?;
    fx.wait("sibling pin", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|r| r["method"] == "thread/section/move"))
    })?;
    ensure!(
        !fx.read(|r, cx| r.sidebar().read(cx).is_running(SUBTRACT)),
        "Pinning restored the finished spinner"
    );
    Ok(())
}

fn sidebar_shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(path) = std::env::var_os("CGX_SIDEBAR_PROOF_DIR") {
        fs::create_dir_all(&path)?;
        let image = fx.shot()?;
        image.save(std::path::PathBuf::from(path).join(format!(
            "{name}-{}-{}x{}.png",
            fx.theme, fx.size.0, fx.size.1
        )))?;
    }
    Ok(())
}

#[test]
fn startup_chat_failure_keeps_home_available_and_retry_restores_chats() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let index = lines
        .iter()
        .position(|line| line["msg"]["result"]["data"][0]["preview"].is_string())
        .expect("thread/list")
        - 1;
    lines[index]["msg"]["params"] = json!({});
    let mut added = lines[index + 1]["msg"]["result"]["data"][0].clone();
    added["id"] = json!("created-after-list-failure");
    added["name"] = json!("New chat after list failure");
    let mut older = added.clone();
    older["id"] = json!("recovered-older-chat");
    lines[index + 1]["msg"]["result"]["nextCursor"] = json!("recovered-next");
    let mut next = exchange("thread/list", json!({"data": [older], "nextCursor": null}));
    next[0]["msg"]["params"] = json!({"cursor": "recovered-next", "limit": 10});
    lines.extend(next);
    let mut refused = lines[index..index + 2].to_vec();
    refused[0]["msg"]["id"] = json!(99_997);
    refused[1]["msg"]["id"] = json!(99_997);
    refused[1]["msg"].as_object_mut().unwrap().remove("result");
    refused[1]["msg"]["error"] = json!({"code": -1, "message": "offline"});
    lines.splice(index..index, refused);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    assert_eq!(fx.route(), app::Route::Home);
    assert!(fx.read(|r, _| r.ready()));
    assert!(fx.read(|r, cx| r.sidebar().read(cx).load_failed()));
    sidebar_shot(&mut fx, "chats-error")?;
    fx.advance(Duration::from_secs(1))?;
    assert!(
        fx.read(|r, cx| r.sidebar().read(cx).load_failed()),
        "Section loading must not clear the failed chat list"
    );
    assert_eq!(
        fx.sent()?
            .iter()
            .filter(|r| r["method"] == "thread/list")
            .count(),
        1
    );
    fx.root.update(&mut fx.cx, |r, cx| {
        r.on_event(
            Event::Notification {
                method: "thread/started".into(),
                params: json!({"thread": added}),
            },
            cx,
        )
    });
    fx.click((90., 368.))?;
    fx.wait("older chats after recovery", |fx| {
        fx.shot().unwrap();
        fx.read(|r, _| r.recents().iter().any(|id| id == "recovered-older-chat"))
    })?;
    assert!(!fx.read(|r, cx| r.sidebar().read(cx).load_failed()));
    fx.click((150., 336.))?;
    fx.wait_thread(SUBTRACT)?;
    Ok(())
}

#[test]
fn background_refresh_failure_keeps_the_loaded_chats_usable() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines
        .iter_mut()
        .find(|line| line["msg"]["method"] == "thread/list")
        .expect("initial list request")["msg"]["params"] = json!({});
    let mut failed = exchange("thread/list", Value::Null);
    failed[1]["msg"].as_object_mut().unwrap().remove("result");
    failed[1]["msg"]["error"] = json!({"code": -1, "message": "refresh offline"});
    lines.extend(failed);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root.update(&mut fx.cx, |r, cx| {
        r.on_event(Event::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId": SUBTRACT, "turn": {"id": "finished", "status": "completed"}}),
        }, cx);
    });
    assert!(
        fx.read(
            |r, cx| r.sidebar().read(cx).threads_state == ui::screens::sidebar::ListState::Ready
        )
    );
    fx.wait("background chat refresh", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().filter(|r| r["method"] == "thread/list").count() == 2)
    })?;
    fx.advance(Duration::from_millis(200))?;
    fx.click((150., 336.))?;
    fx.wait_thread(SUBTRACT)?;
    assert!(!fx.read(|r, cx| r.sidebar().read(cx).load_failed()));
    Ok(())
}

#[test]
fn activity_does_not_page_past_the_week() -> Result<()> {
    let mut lines = paged_script()?;
    for line in &mut lines {
        if let Some(threads) = line
            .pointer_mut("/msg/result/data")
            .and_then(Value::as_array_mut)
        {
            for thread in threads {
                if thread["preview"].is_string() {
                    thread["updatedAt"] = json!(1_791_000_000);
                }
            }
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.wait("normal sidebar page", |fx| {
        fx.shot().unwrap();
        fx.read(|r, _| r.recents().len()) >= 20
    })?;
    fx.click((282., 70.))?;
    let sent = fx
        .sent()?
        .iter()
        .filter(|r| r["method"] == "thread/list")
        .count();
    for _ in 0..3 {
        fx.advance(Duration::from_millis(200))?;
    }
    assert_eq!(
        fx.sent()?
            .iter()
            .filter(|r| r["method"] == "thread/list")
            .count(),
        sent
    );
    Ok(())
}

#[test]
fn project_load_failure_retries_the_owned_state_file() -> Result<()> {
    let profile = tempfile::tempdir()?;
    let project = profile.path().join("project");
    fs::create_dir(&project)?;
    let file = profile.path().join(".codex-global-state.json");
    fs::write(&file, "{invalid")?;
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |r, cx| {
        r.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(profile.path())),
            cx,
        );
    });
    fx.wait("project load error", |fx| {
        fx.read(|r, cx| {
            r.sidebar().read(cx).projects_state == ui::screens::sidebar::ListState::Failed
        })
    })?;
    sidebar_shot(&mut fx, "projects-error")?;
    fs::write(
        &file,
        serde_json::to_vec(&json!({
            "local-projects": {"owned": {
                "id": "owned", "name": "Recovered project", "rootPaths": [project],
                "createdAt": 1, "updatedAt": 2
            }},
            "project-order": ["owned"]
        }))?,
    )?;
    fx.click((90., 251.))?;
    fx.wait("project retry", |fx| {
        fx.read(|r, cx| {
            r.sidebar()
                .read(cx)
                .projects()
                .iter()
                .any(|p| p.id == "owned")
        })
    })?;
    assert_eq!(
        fx.read(|r, cx| r.sidebar().read(cx).projects()[0].label.to_string()),
        "Recovered project"
    );
    assert_eq!(
        fx.read(|r, cx| r.sidebar().read(cx).projects()[0]
            .root
            .as_deref()
            .map(str::to_owned)),
        Some(project.to_string_lossy().into_owned())
    );
    Ok(())
}

#[test]
fn activity_opens_running_chats_and_shortcut_restores_the_sidebar() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click((282., 70.))?;
    for theme in ["dark", "light"] {
        fx.action(Box::new(ui::SetTheme(theme)))?;
        fx.theme = theme;
        for size in [(1280., 832.), (1440., 900.)] {
            fx.resize(size.0, size.1)?;
            fx.hover((size.0 * 0.6, 6.))?;
            assert!(fx.read(|r, cx| r.sidebar().read(cx).activity_clock.is_some()));
            sidebar_shot(&mut fx, "activity-empty")?;
            fx.hover((150., 300.))?;
            sidebar_shot(&mut fx, "activity-hover")?;
        }
    }
    fx.action(Box::new(ui::SetTheme("dark")))?;
    fx.theme = "dark";
    fx.resize(1280., 832.)?;
    fx.hover((768., 6.))?;
    fx.root.update(&mut fx.cx, |r, cx| {
        r.on_event(Event::Notification {
            method: "thread/status/changed".into(),
            params: json!({"threadId": SUBTRACT, "status": {"type": "active", "activeFlags": []}}),
        }, cx)
    });
    fx.advance(Duration::from_millis(300))?;
    sidebar_shot(&mut fx, "activity-running")?;
    fx.click((150., 214.))?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-u")?;
    assert!(fx.read(|r, cx| r.sidebar().read(cx).activity_clock.is_none()));
    fx.keys("alt-cmd-u")?;
    assert!(fx.read(|r, cx| r.sidebar().read(cx).activity_clock.is_some()));
    fx.keys("alt-cmd-u")?;
    assert_eq!(fx.route(), app::Route::Thread(SUBTRACT.into()));
    Ok(())
}

#[test]
fn activity_options_change_the_visible_list_without_navigation() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click((282., 70.))?;
    fx.click((314., 184.))?;
    fx.keys("enter")?;
    assert_eq!(fx.route(), app::Route::Home);
    fx.click((150., 214.))?;
    fx.wait_thread(common::GREETING)?;
    Ok(())
}

#[test]
fn activity_mark_read_keeps_the_snapshot_until_clear_read() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |r, cx| {
        r.sidebar().update(cx, |s, cx| s.set_unread(SUBTRACT, cx))
    });
    fx.click((282., 70.))?;
    fx.click((314., 184.))?;
    fx.keys("down down down enter")?;
    assert!(!fx.read(|r, cx| r.sidebar().read(cx).unread.iter().any(|id| id == SUBTRACT)));
    fx.click((314., 184.))?;
    fx.click((150., 297.))?;
    fx.wait_thread(common::GREETING)?;
    Ok(())
}

struct ArchivePrompt {
    focus: gpui::FocusHandle,
}

impl gpui::Focusable for ArchivePrompt {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

impl gpui::EventEmitter<gpui::PromptResponse> for ArchivePrompt {}

impl gpui::Render for ArchivePrompt {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::InteractiveElement as _;
        gpui::div().id("archive-prompt").track_focus(&self.focus)
    }
}

#[test]
fn activity_archive_requires_confirmation_and_keeps_recent_chats() -> Result<()> {
    use gpui::AppContext as _;
    use std::{cell::RefCell, rc::Rc};
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/archive", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let prompt = Rc::new(RefCell::new(None));
    let opened = prompt.clone();
    fx.cx.update(|cx| {
        cx.set_prompt_builder(move |_, _, _, _, handle, window, cx| {
            let view = cx.new(|cx| ArchivePrompt {
                focus: cx.focus_handle(),
            });
            *opened.borrow_mut() = Some(view.clone());
            handle.with_view(view, window, cx)
        });
    });
    fx.root.update(&mut fx.cx, |r, cx| {
        r.sidebar().update(cx, |s, cx| s.set_unread(SUBTRACT, cx))
    });
    fx.click((282., 70.))?;
    for choice in [0, 1] {
        fx.click((314., 184.))?;
        fx.keys("down down down down enter")?;
        fx.wait("archive confirmation", |_| prompt.borrow().is_some())?;
        let view = prompt.borrow_mut().take().unwrap();
        view.update(&mut fx.cx, |_, cx| cx.emit(gpui::PromptResponse(choice)));
        if choice == 0 {
            fx.advance(Duration::from_millis(100))?;
            assert!(fx.read(|r, _| r.recents().iter().any(|id| id == SUBTRACT)));
            assert!(
                !fx.sent()?
                    .iter()
                    .any(|request| request["method"] == "thread/archive")
            );
        }
    }
    fx.wait("confirmed priority archive", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|r| r["method"] == "thread/archive"))
    })?;
    let chats = fx.read(|r, _| r.recents());
    assert!(!chats.iter().any(|id| id == SUBTRACT));
    assert!(chats.iter().any(|id| id == common::GREETING));
    assert!(chats.iter().any(|id| id == common::EXPLAIN));
    Ok(())
}
