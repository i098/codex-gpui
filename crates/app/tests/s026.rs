//! S026 `cmdmenu`: HC-073 (the command groups after `Quick actions` and `Settings`: `Chat`,
//! `Navigation`, `Panels`, `Project`, `Skills`, `Configure`, `App`; commands without a
//! handler in the context are hidden) and HC-083 (a query lists the matching chats in
//! `Chats` with a substring highlight, the project label, the `⌃n` kbd and the
//! `thread/search` snippet under the title), replayed from `refs/interactions/s026.json`
//! (the original on the route1 Linux stack; `thread/search` answered from
//! `fixtures/sessions/s026-thread-search.jsonl`).
#![cfg(target_os = "macos")]

use anyhow::Result;
use serde_json::Value;

const RECORDING: &str = include_str!("../../../refs/interactions/s026.json");
type MenuGroups = Vec<(String, Vec<(String, bool)>)>;

/// `heading -> rows` of a recorded `groups` entry.
fn groups(entry: &Value) -> Vec<(String, Vec<String>)> {
    entry["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["heading"].as_str().unwrap().to_owned(),
                g["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i.as_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

#[path = "common/mod.rs"]
mod common;

/// The app-server of [`common::Fx::captured`].
#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn command_menu_groups_and_chat_search_match_the_original() -> Result<()> {
    use anyhow::{Context as _, ensure};
    use app::{GoToChat, Route};
    use common::{Fx, GREETING, script};
    use std::time::Duration;
    use ui::OpenCommandMenu;

    let recording: Value = serde_json::from_str(RECORDING)?;
    let steps = recording["steps"].as_array().context("steps")?;
    let dom = recording["result"]["dom"].as_array().context("dom")?;
    let at = |step: &Value| {
        let b = &step["target"]["bbox"];
        let n = |k: &str| b[k].as_f64().unwrap() as f32;
        (n("x") + n("w") / 2., n("y") + n("h") / 2.)
    };
    let wait = |step: &Value| Duration::from_millis(step["waitMs"].as_u64().unwrap_or(0));

    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        script(Some("sessions/s026-thread-search.jsonl"), |_| {})?,
    )?;
    // The port's rows in the recorder's format, per visible group.
    let describe = |fx: &mut Fx| -> Result<MenuGroups> {
        let menu = fx
            .read(|root, _| root.command_menu().cloned())
            .context("command menu open")?;
        Ok(fx.read(|_, cx| {
            menu.read(cx)
                .describe()
                .into_iter()
                .map(|(h, rows)| (h.to_owned(), rows))
                .collect()
        }))
    };
    let rows_only = |groups: MenuGroups| -> Vec<(String, Vec<String>)> {
        groups
            .into_iter()
            .map(|(h, rows)| (h, rows.into_iter().map(|(t, _)| t).collect()))
            .collect()
    };
    let before = fx.sent()?.len();

    // HC-073: the sidebar `Search` opens the menu; every group and row as recorded.
    fx.click(at(&steps[0]))?;
    fx.advance(wait(&steps[0]))?;
    let port = rows_only(describe(&mut fx)?);
    ensure!(port == groups(&dom[0]), "empty query: {port:#?}");

    // HC-083: `greet` keeps the greeting chat with its title highlighted and the snippet.
    fx.type_text(steps[1]["text"].as_str().unwrap())?;
    fx.advance(wait(&steps[1]))?;
    fx.wait("greet content result", |fx| {
        describe(fx)
            .unwrap()
            .iter()
            .flat_map(|(_, rows)| rows)
            .any(|(text, _)| text.contains(" || "))
    })?;
    let port = rows_only(describe(&mut fx)?);
    ensure!(port == groups(&dom[2]), "query greet: {port:#?}");
    let settled_header = image::imageops::crop_imm(&fx.shot()?, 870, 432, 30, 30).to_image();
    fx.shot()?
        .save(std::env::temp_dir().join("s026-greet.png"))?;

    // A content-only match: the title stays plain, the snippet carries the highlight.
    fx.keys("escape")?;
    fx.click(at(&steps[3]))?;
    fx.advance(wait(&steps[3]))?;
    fx.type_text(steps[4]["text"].as_str().unwrap())?;
    fx.advance(wait(&steps[4]))?;
    fx.wait("world content result", |fx| {
        describe(fx)
            .unwrap()
            .iter()
            .flat_map(|(_, rows)| rows)
            .any(|(text, _)| text.contains(" || "))
    })?;
    let port = rows_only(describe(&mut fx)?);
    ensure!(port == groups(&dom[3]), "query world: {port:#?}");
    fx.shot()?
        .save(std::env::temp_dir().join("s026-world.png"))?;

    // The content searches, in order, with the original's params.
    let sent: Vec<_> = fx.sent()?[before..]
        .iter()
        .filter(|m| m["method"] == "thread/search")
        .map(|m| m["params"].clone())
        .collect();
    let want: Vec<Value> = recording["result"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str(r["params"].as_str().unwrap()).unwrap())
        .collect();
    ensure!(sent == want, "thread/search params: {sent:?}");

    // Escape closes the menu on Home.
    fx.keys("escape")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    ensure!(fx.route() == Route::Home);

    // A thread route registers more commands: the recorded thread-route groups.
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.action(Box::new(OpenCommandMenu))?;
    let port = rows_only(describe(&mut fx)?);
    ensure!(
        port == groups(&recording["result"]["threadRoute"]),
        "thread route: {port:#?}"
    );
    fx.keys("escape")?;

    // The unread dot [INFERENCE: 8 px `info-solid` dot centred in the leading slot; the
    // route1 stack has no unread thread to record]: an unread chat's row carries it.
    fx.read(|root, _| root.sidebar().clone())
        .update(&mut fx.cx, |sidebar, _| {
            sidebar.unread = vec![GREETING.into()]
        });
    fx.action(Box::new(OpenCommandMenu))?;
    let unread: Vec<_> = describe(&mut fx)?
        .into_iter()
        .flat_map(|(_, rows)| rows)
        .filter(|(_, unread)| *unread)
        .map(|(text, _)| text)
        .collect();
    ensure!(
        unread == [groups(&dom[0])[0].1[0].clone()],
        "unread rows: {unread:?}"
    );
    let image = fx.shot()?;
    image.save(std::env::temp_dir().join("s026-unread.png"))?;
    let scale = image.width() as f32 / 1280.;
    // The first chat row's leading slot: dialog x 385 + `px-2` 8, centre of 16 px.
    let pixel = image
        .get_pixel((401. * scale) as u32, (252.5 * scale) as u32)
        .0;
    ensure!(pixel[2] > pixel[0] + 60, "info-solid dot, got {pixel:?}");
    fx.keys("down")?;
    let faded = fx
        .shot()?
        .get_pixel((401. * scale) as u32, (252.5 * scale) as u32)
        .0;
    ensure!(
        faded[2] < pixel[2] && faded[2] > faded[0] + 60,
        "the unread dot follows the unselected row opacity: {faded:?}"
    );
    fx.keys("escape")?;
    fx.action(Box::new(ui::NewChat))?;
    fx.action(Box::new(OpenCommandMenu))?;
    fx.type_text("chat")?;
    let menu = fx.read(|root, _| root.command_menu().unwrap().clone());
    let hits: Vec<ui::screens::sidebar::SearchHit> = serde_json::from_str(
        include_str!("../../../fixtures/sessions/s026-thread-search.jsonl")
            .lines()
            .nth(1)
            .unwrap(),
    )
    .map(|line: Value| serde_json::from_value(line["msg"]["result"]["data"].clone()).unwrap())?;
    menu.update(&mut fx.cx, |menu, cx| {
        menu.set_search_results("chat", hits, cx)
    });
    drop(menu);
    fx.keys("enter")?;
    ensure!(
        fx.route() == Route::Home,
        "the delayed reply must keep New chat selected"
    );

    // The two successful responses are consumed. The next search receives a replay error.
    fx.action(Box::new(OpenCommandMenu))?;
    fx.type_text("greet")?;
    fx.advance(Duration::from_millis(250))?;
    fx.wait("the failed search clears its spinner", |fx| {
        let image = fx.shot().unwrap();
        image::imageops::crop_imm(&image, 870, 432, 30, 30).to_image() == settled_header
    })?;
    Ok(())
}
