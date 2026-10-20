#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use app::Route;
use common::*;
use serde_json::{Value, json};
use std::fs;
use ui::screens::sidebar::sections::{SectionAction, SectionCommand};

#[test]
fn proxy() {
    proxy_child();
}

fn fixture() -> Result<(Vec<Value>, String)> {
    let rec: Value = serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s003_sections.json"
    ))?)?;
    let state = rec["fixture"]["seed"]["value"]
        .as_object()
        .context("section accounts")?
        .values()
        .next()
        .context("section state")?;
    let section = &state["sections"][0];
    let id = section["id"].as_str().context("section id")?.to_owned();
    let mut lines = script(None, |_| {})?;
    let mut template = recorded_thread(GREETING)?;
    template["turns"] = json!([]);
    let data: Vec<Value> = section["itemKeys"]
        .as_array()
        .context("section members")?
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let mut thread = template.clone();
            thread["id"] = key
                .as_str()
                .unwrap()
                .strip_prefix("codex:thread:local:")
                .unwrap()
                .into();
            thread["name"] = format!("S003 thread {:02}", index + 1).into();
            thread["section"] = json!({"id": id});
            thread
        })
        .collect();
    let mut sections = exchange(
        "threadSection/list",
        json!({"data": [{"id": id, "name": "Review"}], "nextCursor": null}),
    );
    sections[0]["msg"]["params"] = json!({"cursor": null, "limit": 100});
    lines.extend(sections);
    let mut members = exchange("thread/list", json!({"data": data, "nextCursor": null}));
    members[0]["msg"]["params"] = json!({
        "cursor": null, "limit": 100, "sectionId": id,
        "sortKey": "section_position", "modelProviders": [], "useStateDbOnly": true
    });
    lines.extend(members);
    lines.extend(exchange("threadSection/update", json!({})));
    lines.extend(exchange(
        "threadSection/create",
        json!({"section": {"id": "created-section", "name": "New section"}}),
    ));
    lines.extend(exchange("threadSection/delete", json!({})));
    lines.extend(exchange("thread/section/move", json!({})));
    lines.extend(exchange("thread/section/move", json!({})));
    for thread_id in ["section-chat", "ordinary-chat"] {
        let mut thread = template.clone();
        thread["id"] = thread_id.into();
        thread["section"] = Value::Null;
        lines.extend(exchange(
            "thread/start",
            json!({"cwd": thread["cwd"], "model": "gpt-6.1-sol", "thread": thread}),
        ));
        lines.extend(exchange(
            "turn/start",
            json!({"turn": {
                "id": format!("turn-{thread_id}"), "items": [], "status": "completed", "error": null
            }}),
        ));
    }
    for _ in 0..20 {
        lines.extend(exchange("thread/archive", json!({})));
    }
    Ok((lines, id))
}

#[test]
fn section_options_and_targeted_chat() -> Result<()> {
    let (lines, id) = fixture()?;
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.wait("loaded custom section", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).section_name(&id) == Some("Review"))
    })?;
    let output = std::env::var_os("CGX_SECTION_PROOF").map(std::path::PathBuf::from);
    if let Some(out) = &output {
        fs::create_dir_all(out)?;
    }
    fx.hover((768., 6.))?;
    if let Some(out) = &output {
        fx.shot()?.save(out.join("rest.png"))?;
    }
    fx.click((105., 271.))?;
    fx.hover((768., 6.))?;
    if let Some(out) = &output {
        fx.shot()?.save(out.join("focused.png"))?;
    }
    fx.action(Box::new(SectionAction {
        section: id.clone(),
        command: SectionCommand::Edit,
    }))?;
    ensure!(fx.read(|root, cx| {
        root.dialog()
            .is_some_and(|d| d.read(cx).kind() == ui::dialog::Kind::EditSection)
    }));
    if let Some(out) = &output {
        fx.shot()?.save(out.join("edit.png"))?;
    }
    fx.type_text("Reviews")?;
    fx.keys("enter")?;
    fx.wait("renamed section", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).section_name(&id) == Some("Reviews"))
    })?;
    let section_action = |command| {
        Box::new(SectionAction {
            section: id.clone(),
            command,
        })
    };
    fx.action(section_action(SectionCommand::New))?;
    fx.keys("enter")?;
    fx.wait("created section", |fx| {
        fx.read(|root, cx| {
            root.sidebar().read(cx).section_name("created-section") == Some("New section")
        })
    })?;
    fx.action(Box::new(SectionAction {
        section: "created-section".into(),
        command: SectionCommand::Remove,
    }))?;
    fx.wait("removed empty section", |fx| {
        fx.read(|root, cx| {
            root.sidebar()
                .read(cx)
                .section_name("created-section")
                .is_none()
        })
    })?;
    let unread = fx.read(|root, cx| root.sidebar().read(cx).section_threads(&id)[0].clone());
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |s, cx| {
        s.unread = vec![unread, GREETING.to_owned()];
        cx.notify();
    });
    fx.action(section_action(SectionCommand::MarkRead))?;
    ensure!(fx.read(|root, cx| root.sidebar().read(cx).unread == [GREETING]));
    fx.action(section_action(SectionCommand::Archive))?;
    fx.keys("escape")?;
    ensure!(!fx.sent()?.iter().any(|r| r["method"] == "thread/archive"));
    fx.action(section_action(SectionCommand::NewChat))?;
    ensure!(fx.route() == Route::Home && fx.composer_focused()? && fx.composer_text()?.is_empty());
    fx.type_text("A chat for the review section")?;
    fx.keys("enter")?;
    fx.wait("section chat started", |fx| {
        matches!(fx.route(), Route::Thread(_))
    })?;
    let chat = fx.thread_id().context("new chat id")?;
    ensure!(
        fx.read(|root, cx| root.sidebar().read(cx).section_threads(&id).contains(&chat)),
        "new chat lost its target section"
    );
    let sent = fx.sent()?;
    let start = sent
        .iter()
        .position(|r| r["method"] == "thread/start")
        .context("thread start")?;
    let moved = sent
        .iter()
        .position(|r| {
            r["method"] == "thread/section/move"
                && r["params"]["threadId"] == chat
                && r["params"]["sectionId"] == id
        })
        .context("section assignment")?;
    let turn = sent
        .iter()
        .position(|r| r["method"] == "turn/start")
        .context("turn start")?;
    ensure!(
        start < moved && moved < turn,
        "wrong section assignment order"
    );
    fx.action(Box::new(ui::NewChat))?;
    fx.type_text("A normal chat")?;
    fx.keys("enter")?;
    fx.wait("normal chat started", |fx| {
        matches!(fx.route(), Route::Thread(_))
    })?;
    ensure!(
        fx.sent()?
            .iter()
            .filter(|r| r["method"] == "thread/section/move")
            .count()
            == 1,
        "normal New chat reused section target"
    );
    let pinned = fx.read(|root, cx| root.sidebar().read(cx).section_threads(&id)[0].clone());
    fx.action(Box::new(ui::screens::sidebar::PinThread(
        pinned.clone(),
        true,
    )))?;
    fx.action(section_action(SectionCommand::Archive))?;
    fx.keys("tab enter")?;
    fx.wait("archived section chats", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).section_threads(&id).is_empty())
    })?;
    ensure!(
        fx.route() == Route::Thread("ordinary-chat".to_owned()),
        "section archive changed another chat"
    );
    ensure!(
        !fx.sent()?
            .iter()
            .any(|r| r["method"] == "thread/archive" && r["params"]["threadId"] == pinned),
        "section archive included a pinned chat"
    );
    if let Some(out) = &output {
        fs::write(
            out.join("requests.json"),
            serde_json::to_vec_pretty(&fx.sent()?)?,
        )?;
    }
    Ok(())
}
