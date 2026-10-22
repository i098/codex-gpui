#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::{Root, Route, SidePanel};
use bridge::Event;
use common::{Fx, GREETING, exchange, script};
use gpui::AppContext as _;
use serde_json::json;
use std::time::Duration;

#[test]
fn proxy() {
    common::proxy_child();
}

fn proof(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_COMMAND_DESTINATIONS_EVIDENCE") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        fx.shot()?.save(directory.join(name))?;
    }
    Ok(())
}

fn file_notifications(fx: &mut Fx, session: &str, query: &str, path: &std::path::Path) {
    fx.root.update(&mut fx.cx, |root, cx| root.on_event(Event::Notification {
        method: "fuzzyFileSearch/sessionUpdated".into(),
        params: json!({"sessionId": session, "query": query, "files": [{"root": "/", "path": path}]}),
    }, cx));
    fx.cx.run_until_parked();
}

#[test]
fn file_search_ignores_old_queries_and_opens_the_selected_file() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("target.txt");
    std::fs::write(&target, "Native file destination proof\n")?;
    let other_directory = directory.path().join("other");
    std::fs::create_dir(&other_directory)?;
    let other = other_directory.join("target.txt");
    std::fs::write(&other, "Another file with the same name\n")?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(cwd) = line.pointer_mut("/msg/result/thread/cwd")
            && let Some(path) = cwd.as_str()
        {
            *cwd = format!("{}/", path.trim_end_matches('/')).into();
        }
    }
    for method in [
        "fuzzyFileSearch/sessionStart",
        "fuzzyFileSearch/sessionUpdate",
        "fuzzyFileSearch/sessionStop",
    ] {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-p")?;
    proof(&mut fx, "native-files-empty.png")?;
    let session = fx.read(|root, cx| {
        root.command_menu()
            .unwrap()
            .read(cx)
            .file_session()
            .unwrap()
            .to_owned()
    });
    fx.type_text("target")?;
    fx.advance(Duration::from_millis(250))?;
    fx.wait("the file index starts", |fx| {
        fx.read(|root, cx| root.command_menu().unwrap().read(cx).file_session_started())
    })?;
    file_notifications(&mut fx, &session, "old-query", &target);
    file_notifications(&mut fx, "another-window", "target", &target);
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_some() && root.file_view().is_none()));
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: "fuzzyFileSearch/sessionUpdated".into(),
                params: json!({"sessionId": session, "query": "target", "files": [
                    {"root": "/", "path": other},
                    {"root": "/", "path": target},
                ]}),
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
    fx.keys("down")?;
    proof(&mut fx, "native-files-result.png")?;
    fx.keys("enter")?;
    fx.wait("the selected file opens", |fx| {
        fx.read(|root, cx| {
            root.side_panel() == Some(SidePanel::File)
                && root
                    .file_view()
                    .is_some_and(|file| file.read(cx).path == target.to_string_lossy().as_ref())
        })
    })?;
    fx.advance(Duration::from_millis(300))?;
    proof(&mut fx, "native-file-open.png")?;
    fx.keys("cmd-p")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    Ok(())
}

#[test]
fn absolute_file_search_rejects_directories_before_opening_a_file() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("absolute.txt");
    std::fs::write(&target, "Absolute file path proof\n")?;
    let lines = script(None, |_| {})?;
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-p")?;
    fx.type_text(directory.path().to_str().context("directory path")?)?;
    fx.advance(Duration::from_millis(500))?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_some() && root.file_view().is_none()));
    fx.keys("cmd-a")?;
    fx.type_text(target.to_str().context("file path")?)?;
    fx.advance(Duration::from_millis(500))?;
    fx.keys("enter")?;
    fx.wait("the absolute file opens", |fx| {
        fx.read(|root, cx| {
            root.file_view()
                .is_some_and(|file| file.read(cx).name() == "absolute.txt")
        })
    })?;
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    Ok(())
}

#[test]
fn a_detached_chat_keeps_its_backend_after_the_first_window_closes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("detached.txt");
    std::fs::write(&target, "Shared file history proof\n")?;
    let mut lines = script(None, |_| {})?;
    lines.extend(script(None, |_| {})?);
    let mut preview_search = exchange("thread/search", json!({"data": []}));
    preview_search[0]["msg"]["params"] = json!({
        "cursor": null, "limit": 9, "sortKey": "recency_at", "archived": false,
        "sourceKinds": [], "searchTerm": "Open in new window",
    });
    lines.extend(preview_search);
    let mut stream_search = exchange("thread/search", json!({"data": []}));
    stream_search[0]["msg"]["params"] = json!({
        "cursor": null, "limit": 9, "sortKey": "recency_at", "archived": false,
        "sourceKinds": [], "searchTerm": "detached-stream-proof",
    });
    lines.extend(stream_search);
    lines.push(json!({"dir": "recv", "msg": {
        "method": "turn/started",
        "params": {"threadId": GREETING, "turn": {"id": "detached-live", "items": [], "status": "inProgress"}}
    }}));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-p")?;
    fx.type_text(target.to_str().context("shared file path")?)?;
    fx.advance(Duration::from_millis(500))?;
    fx.keys("enter")?;
    fx.wait("the first window opens the file", |fx| {
        fx.read(|root, cx| {
            root.file_view()
                .is_some_and(|file| file.read(cx).name() == "detached.txt")
        })
    })?;
    let first_window = fx.window;
    fx.keys("cmd-k")?;
    fx.type_text("Open in new window")?;
    fx.keys("enter")?;
    let windows = fx.cx.update(|cx| cx.windows());
    let second = windows
        .into_iter()
        .find(|window| *window != first_window)
        .context("second native window")?
        .downcast::<Root>()
        .context("second window root")?;
    let root = fx.cx.read_window(&second, |root, _| root)?;
    fx.root = root;
    fx.window = second.into();
    fx.wait("detached chat startup", |fx| {
        fx.read(|root, _| root.ready())
    })?;
    fx.wait_thread(GREETING)?;
    fx.cx
        .update_window(first_window, |_, window, _| window.remove_window())?;
    fx.keys("cmd-p")?;
    fx.keys("enter")?;
    fx.wait("the remaining window shares recent files", |fx| {
        fx.read(|root, cx| {
            root.file_view()
                .is_some_and(|file| file.read(cx).name() == "detached.txt")
        })
    })?;
    fx.keys("cmd-k")?;
    fx.type_text("detached-stream-proof")?;
    fx.advance(Duration::from_millis(250))?;
    fx.wait("the remaining window receives backend events", |fx| {
        fx.read(|root, cx| {
            root.thread().is_some_and(|thread| {
                thread
                    .read(cx)
                    .turns()
                    .iter()
                    .any(|turn| turn.id == "detached-live")
            })
        })
    })?;
    fx.keys("escape")?;
    proof(&mut fx, "native-detached-chat.png")?;
    let sent = fx.sent()?;
    ensure!(
        sent.iter()
            .filter(|request| request["method"] == "initialize")
            .count()
            == 1
    );
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    Ok(())
}

#[test]
fn file_search_retries_start_and_releases_its_index_for_a_dialog() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let mut rejected = exchange("fuzzyFileSearch/sessionStart", json!({}));
    rejected[1]["msg"].as_object_mut().unwrap().remove("result");
    rejected[1]["msg"]["error"] = json!({"code": -32000, "message": "index unavailable"});
    lines.extend(rejected);
    for method in [
        "fuzzyFileSearch/sessionStart",
        "fuzzyFileSearch/sessionUpdate",
        "fuzzyFileSearch/sessionStop",
    ] {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-p")?;
    fx.type_text("retry")?;
    fx.advance(Duration::from_millis(250))?;
    fx.wait("retry starts the index and searches", |fx| {
        fx.keys("enter").unwrap();
        let sent = fx.sent().unwrap();
        sent.iter()
            .filter(|request| request["method"] == "fuzzyFileSearch/sessionStart")
            .count()
            == 2
            && sent.iter().any(|request| {
                request["method"] == "fuzzyFileSearch/sessionUpdate"
                    && request["params"]["query"] == "retry"
            })
    })?;
    fx.keys("cmd-/")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none() && root.dialog().is_some()));
    fx.wait("the dialog releases the file index", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|request| request["method"] == "fuzzyFileSearch/sessionStop")
    })?;
    fx.keys("escape")?;
    fx.keys("cmd-p")?;
    fx.keys("escape")?;
    ensure!(
        fx.sent()?
            .iter()
            .filter(|request| request["method"] == "fuzzyFileSearch/sessionStart")
            .count()
            == 2
    );
    Ok(())
}

#[test]
fn a_failed_file_read_does_not_enter_recent_files() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing.txt");
    let mut lines = script(None, |_| {})?;
    for method in [
        "fuzzyFileSearch/sessionStart",
        "fuzzyFileSearch/sessionUpdate",
        "fuzzyFileSearch/sessionStop",
    ] {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-p")?;
    fx.type_text("missing")?;
    fx.advance(Duration::from_millis(250))?;
    fx.wait("the file index starts", |fx| {
        fx.read(|root, cx| root.command_menu().unwrap().read(cx).file_session_started())
    })?;
    let session = fx.read(|root, cx| {
        root.command_menu()
            .unwrap()
            .read(cx)
            .file_session()
            .unwrap()
            .to_owned()
    });
    file_notifications(&mut fx, &session, "missing", &missing);
    fx.keys("enter")?;
    fx.wait("the file read fails", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title == "Could not open file")
        })
    })?;
    fx.keys("cmd-p")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_some()));
    Ok(())
}
