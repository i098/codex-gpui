//! Flows §14.2 row 13: every item of the Recents row context menu (refs/interactions/s002.json)
//! runs its action on the row's thread, picked by keyboard as the menu supports.

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

fn menu(fx: &Fx) -> Option<Value> {
    fx.read(|root, cx| root.sidebar().read(cx).context_menu())
}

/// Opens the row menu at `at` and picks `path` (an item id, then a submenu item id) by keys.
fn pick(fx: &mut Fx, at: (f32, f32), path: &[&str]) -> Result<()> {
    fx.right_click(at)?;
    let mut items =
        menu(fx).with_context(|| format!("{path:?}: the right-click opened no menu"))?;
    for (depth, id) in path.iter().enumerate() {
        let list = items.as_array().context("items")?.clone();
        let index = list
            .iter()
            .position(|item| item["id"] == *id)
            .with_context(|| format!("no {id} in {items}"))?;
        // The top level starts with no active row; a submenu opens on its first row.
        let skip = usize::from(depth > 0);
        let moves = list[..=index]
            .iter()
            .filter(|item| item["type"] != "separator")
            .count()
            - skip;
        for _ in 0..moves {
            fx.keys("down")?;
        }
        if depth + 1 < path.len() {
            fx.keys("right")?;
            items = list[index]["submenu"].clone();
        }
    }
    fx.keys("enter")?;
    ensure!(menu(fx).is_none(), "{path:?} left the menu open");
    Ok(())
}

fn sent(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == method)
        .map(|r| r["params"].clone())
        .collect())
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

#[test]
fn row_menu_items_run_on_the_row_thread() -> Result<()> {
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s002.json"))?)?;
    let step = &rec["steps"][0];
    let row = (
        step["at"]["x"].as_f64().unwrap() as f32,
        step["at"]["y"].as_f64().unwrap() as f32,
    );
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/read",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    lines.extend(exchange(
        "threadSection/create",
        json!({"section": {"id": "created-section", "name": "New section"}}),
    ));
    lines.extend(exchange("thread/section/move", json!({})));
    lines.extend(exchange("thread/section/move", json!({})));
    lines.extend(exchange("threadSection/delete", json!({})));
    lines.extend(exchange("thread/delete", json!({})));
    let mut forked = recorded_thread(GREETING)?;
    forked["id"] = "forked-chat".into();
    lines.extend(exchange("thread/fork", json!({"thread": forked})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;

    pick(&mut fx, row, &["copy-actions", "copy-deeplink"])?;
    ensure!(
        clipboard(&mut fx) == Some(format!("codex://threads/{GREETING}")),
        "Copy deeplink copied {:?}",
        clipboard(&mut fx)
    );

    pick(
        &mut fx,
        row,
        &["copy-actions", "copy-conversation-markdown"],
    )?;
    fx.wait("Copied conversation as Markdown", |fx| {
        fx.read(|r, cx| r.toasts(cx))
            .iter()
            .any(|t| t == "Copied conversation as Markdown")
    })?;
    let markdown = clipboard(&mut fx).context("no Markdown copied")?;
    ensure!(
        markdown.starts_with("# ") && markdown.contains(GREETING_PROMPT),
        "Markdown copy: {markdown}"
    );
    ensure!(
        sent(&mut fx, "thread/read")?
            .iter()
            .any(|p| p["threadId"] == GREETING && p["includeTurns"] == true),
        "Copy as Markdown did not read the row's turns"
    );

    pick(
        &mut fx,
        row,
        &["move-to-custom-section", "new-custom-section"],
    )?;
    ensure!(
        fx.read(|r, _| r.dialog().is_some()),
        "New section… opened no dialog"
    );
    fx.keys("enter")?;
    fx.wait("the row moved into the new section", |fx| {
        fx.read(|r, cx| r.sidebar().read(cx).section_threads("created-section") == [GREETING])
    })?;
    fx.right_click(row)?;
    let submenu = menu(&fx).context("no menu")?[3]["submenu"].clone();
    ensure!(
        submenu[0]
            == json!({"id": "move-to-custom-section:created-section", "label": "New section",
                     "enabled": true, "type": "checkbox", "checked": true})
            && submenu[1]["type"] == "separator"
            && submenu[2]["id"] == "new-custom-section",
        "Section submenu: {submenu}"
    );
    fx.keys("escape")?;
    pick(
        &mut fx,
        row,
        &[
            "move-to-custom-section",
            "move-to-custom-section:created-section",
        ],
    )?;
    fx.wait("the checked section took the row out", |fx| {
        fx.read(|r, cx| {
            r.sidebar()
                .read(cx)
                .section_threads("created-section")
                .is_empty()
        })
    })?;
    let moves = sent(&mut fx, "thread/section/move")?;
    ensure!(
        moves
            == [
                json!({"threadId": GREETING, "sectionId": "created-section", "beforeThreadId": null}),
                json!({"threadId": GREETING, "sectionId": null, "beforeThreadId": null}),
            ],
        "section moves: {moves:?}"
    );
    // Removing the empty section puts the Recents rows back where the recording has them.
    fx.action(Box::new(SectionAction {
        section: "created-section".into(),
        command: SectionCommand::Remove,
    }))?;
    fx.wait("the section left", |fx| {
        fx.read(|r, cx| !r.sidebar().read(cx).has_sections())
    })?;

    let windows = fx.cx.update(|cx| cx.windows().len());
    pick(&mut fx, row, &["open-in-new-window"])?;
    ensure!(
        fx.cx.update(|cx| cx.windows().len()) == windows + 1,
        "Open in new window opened no window"
    );

    pick(&mut fx, row, &["delete-thread"])?;
    fx.keys("escape")?;
    ensure!(
        sent(&mut fx, "thread/delete")?.is_empty(),
        "Cancel deleted the chat"
    );
    let before = fx.read(|r, _| r.recents());
    pick(&mut fx, row, &["delete-thread"])?;
    fx.keys("tab")?;
    fx.keys("enter")?;
    fx.wait("Task deleted", |fx| {
        fx.read(|r, cx| r.toasts(cx))
            .iter()
            .any(|t| t == "Task deleted")
    })?;
    let deleted = sent(&mut fx, "thread/delete")?;
    ensure!(deleted.len() == 1, "thread/delete requests: {deleted:?}");
    let id = deleted[0]["threadId"].as_str().context("threadId")?;
    ensure!(
        before.iter().any(|t| t == id) && !fx.read(|r, _| r.recents()).iter().any(|t| t == id),
        "the deleted row {id} stayed in Recents"
    );

    pick(&mut fx, row, &["fork", "fork-into-local"])?;
    fx.wait("the fork opened", |fx| {
        fx.route() == Route::Thread("forked-chat".into())
    })?;
    ensure!(
        sent(&mut fx, "thread/fork")?.len() == 1,
        "Fork chat sent no thread/fork"
    );
    Ok(())
}

#[test]
fn copy_conversation_path_uses_the_thread_read_path() -> Result<()> {
    const PATH: &str = "/tmp/codex/sessions/rollout.jsonl";
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["result"]["thread"]["id"] == GREETING {
            line["msg"]["result"]["thread"]["path"] = PATH.into();
        }
    }
    let mut read = exchange("thread/read", json!({"thread": recorded_thread(GREETING)?}));
    read[0]["msg"]["params"] = json!({"threadId": GREETING, "includeTurns": false});
    read[1]["msg"]["result"]["thread"]["path"] = PATH.into();
    lines.extend(read);

    let mut fx = Fx::scripted(lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-alt-shift-c")?;
    fx.wait("Copied conversation path", |fx| {
        fx.read(|root, cx| root.toasts(cx))
            .iter()
            .any(|toast| toast == "Copied conversation path")
    })?;
    ensure!(clipboard(&mut fx).as_deref() == Some(PATH));
    ensure!(
        sent(&mut fx, "thread/read")?
            .iter()
            .any(|params| params["threadId"] == GREETING && params["includeTurns"] == false),
        "copy path did not read the thread without turns"
    );
    Ok(())
}
