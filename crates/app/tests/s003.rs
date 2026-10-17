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
