#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use app::Route;
use common::*;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn archive_toast_view_and_undo() -> Result<()> {
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s031.json"))?;
    let mut lines = script(None, |_| {})?;
    let lists: Vec<_> = lines
        .iter()
        .filter(|line| line["dir"] == "send" && line["msg"]["method"] == "thread/list")
        .map(|line| line["msg"]["id"].clone())
        .collect();
    lines.retain(|line| !lists.contains(&line["msg"]["id"]));
    for entries in [&rec["fixtureRpc"], &rec["result"]["rpc"]] {
        for entry in entries.as_array().unwrap() {
            let mut entry = entry.clone();
            entry["msg"]["id"] = json!(100_000 + entry["msg"]["id"].as_u64().unwrap());
            lines.push(entry);
        }
    }
    let id = rec["result"]["requests"][0]["params"]["threadId"]
        .as_str()
        .unwrap();
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    assert_eq!(fx.read(|root, _| root.recents()), [id]);
    let before = fx.sent()?.len();
    for step in rec["steps"].as_array().unwrap() {
        let bbox = &step["target"]["bbox"];
        fx.click((
            (bbox["x"].as_f64().unwrap() + bbox["w"].as_f64().unwrap() / 2.) as f32,
            (bbox["y"].as_f64().unwrap() + bbox["h"].as_f64().unwrap() / 2.) as f32,
        ))?;
        match step["target"]["text"].as_str().unwrap() {
            "Archive chat" => {
                fx.wait("archive toast", |fx| {
                    fx.read(|root, cx| root.toasts(cx)) == ["Archived chat"]
                })?;
                assert!(fx.read(|root, _| root.recents()).is_empty());
            }
            "Undo" => {
                fx.wait("restored row", |fx| {
                    fx.read(|root, _| root.recents()) == [id]
                })?;
                assert!(fx.read(|root, cx| root.toasts(cx)).is_empty());
                assert_eq!(fx.route(), Route::Home);
            }
            "View" => {
                assert_eq!(rec["result"]["dom"][3]["route"], "/archive");
                assert_eq!(fx.route(), Route::Archive);
                assert!(!fx.sidebar_open());
                assert!(!fx.shell(|shell| shell.chat_chrome || shell.new_tab));
                assert_eq!(fx.shell(|shell| shell.selected_rail), None);
                assert!(fx.read(|root, cx| root.toasts(cx)).is_empty());
                fx.wait("archived list", |fx| {
                    fx.read(|root, cx| {
                        root.archived_chats()
                            .is_some_and(|page| page.read(cx).threads.is_some())
                    })
                })?;
                let archived = fx.read(|root, cx| {
                    root.archived_chats()
                        .unwrap()
                        .read(cx)
                        .threads
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(|thread| thread.id.clone())
                        .collect::<Vec<_>>()
                });
                assert_eq!(archived, [id]);
                let dir = PathBuf::from(std::env::var_os("S031_SHOTS").unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                        .join("s031")
                        .into_os_string()
                }));
                fs::create_dir_all(&dir)?;
                fx.shot()?.save(dir.join("port-view.png"))?;
            }
            label => anyhow::bail!("unexpected recorded action {label}"),
        }
    }
    let writes: Vec<_> = fx.sent()?[before..]
        .iter()
        .filter(|message| {
            matches!(
                message["method"].as_str(),
                Some("thread/archive" | "thread/unarchive" | "thread/list")
            )
        })
        .cloned()
        .collect();
    assert_eq!(
        writes,
        rec["result"]["requests"].as_array().unwrap().as_slice()
    );
    fx.keys("cmd-[")?;
    assert_eq!(fx.route(), Route::Home);
    assert!(fx.sidebar_open());
    fx.keys("cmd-,")?;
    assert_eq!(fx.route(), Route::Settings);
    fx.keys("cmd-[")?;
    assert_eq!(fx.route(), Route::Home);
    Ok(())
}
