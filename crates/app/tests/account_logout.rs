#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::{Fx, GREETING, exchange, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

fn open_logout(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text("Log out")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_LOGOUT_SHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        fx.shot()?.save(directory.join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn logout_requires_confirmation_and_preserves_the_session_on_failure() -> Result<()> {
    let mut lines = script(None, |line| {
        if line["dir"] == "send" && line["msg"]["method"] == "thread/list" {
            line["msg"]["params"] = json!({});
        }
    })?;
    let mut failed = exchange("account/logout", json!({}));
    failed[1]["msg"].as_object_mut().unwrap().remove("result");
    failed[1]["msg"]["error"] = json!({"code": -32603, "message": "Replay logout failure"});
    lines.extend(failed);
    lines.extend(exchange("account/logout", json!({})));
    let mut new_thread = common::recorded_thread(GREETING)?;
    new_thread["turns"] = json!([]);
    let pages: Vec<_> = (0..11)
        .map(|index| {
            let mut thread = new_thread.clone();
            thread["id"] = format!("after-login-{index}").into();
            thread
        })
        .collect();
    lines.extend(exchange(
        "thread/list",
        json!({"data": &pages[..10], "nextCursor": "after-login-page"}),
    ));
    let mut next_page = exchange(
        "thread/list",
        json!({"data": &pages[10..], "nextCursor": null}),
    );
    next_page[0]["msg"]["params"] = json!({"cursor": "after-login-page", "limit": 10});
    lines.extend(next_page);
    new_thread["id"] = "after-login-new-chat".into();
    lines.extend(exchange(
        "thread/start",
        json!({
            "thread": new_thread,
            "model": "gpt-6-astra",
            "cwd": common::CWD,
            "reasoningEffort": "low",
        }),
    ));
    lines.extend(exchange("turn/start", json!({})));
    lines.extend(exchange("turn/start", json!({})));
    let theme = match std::env::var("CGX_LOGOUT_THEME").as_deref() {
        Ok("light") => "light",
        _ => "dark",
    };
    let mut fx = Fx::captured(theme, (1280., 832.), lines)?;
    let home = tempfile::tempdir()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(home.path())),
            cx,
        );
    });
    let home_composer = fx.composer();
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.type_text("Keep this draft")?;
    let requests = fx.sent()?;
    open_logout(&mut fx)?;
    shot(&mut fx, "confirmation")?;
    ensure!(
        fx.read(|root, _| root.dialog().is_some()),
        "Log out must open confirmation"
    );
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.dialog().is_none() && !root.signed_out()));
    ensure!(fx.sent()? == requests, "Cancel must send nothing");
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    ensure!(fx.composer_text()? == "Keep this draft");

    open_logout(&mut fx)?;
    fx.keys("escape")?;
    ensure!(fx.sent()? == requests, "Escape must send nothing");
    fx.click((26., 806.))?;
    fx.click((118., 672.))?;
    ensure!(fx.read(|root, _| root.dialog().is_some()));
    fx.keys("tab")?;
    fx.keys("enter")?;
    fx.wait("logout failure", |fx| {
        !fx.read(|root, cx| root.toasts(cx).is_empty())
    })?;
    ensure!(!fx.read(|root, _| root.signed_out()));
    ensure!(fx.thread_id().as_deref() == Some(GREETING));
    ensure!(fx.composer_focused()?);
    fx.type_text(" after failure")?;
    ensure!(fx.composer_text()? == "Keep this draft after failure");
    shot(&mut fx, "failure")?;
    let image = home.path().join("previous-account.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255])).save(&image)?;
    for composer in [home_composer, fx.composer()] {
        composer.update(&mut fx.cx, |composer, cx| {
            composer.drop_files(std::slice::from_ref(&image), cx);
        });
    }

    fx.menu("Codex", "Log Out")?;
    ensure!(fx.read(|root, _| root.dialog().is_some()));
    fx.keys("tab")?;
    fx.keys("enter")?;
    fx.wait("login after logout", |fx| {
        fx.read(|root, _| root.signed_out())
    })?;
    ensure!(fx.read(|root, _| root.thread().is_none() && root.recents().is_empty()));
    ensure!(fx.route() == Route::Home);
    shot(&mut fx, "success")?;
    let requests = fx.sent()?;
    let logout: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "account/logout")
        .map(|r| r["params"].clone())
        .collect();
    ensure!(logout == [json!({}), json!({})]);
    let late_thread = common::recorded_thread(GREETING)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "thread/started".into(),
                params: json!({"thread": late_thread}),
            },
            cx,
        );
    });
    ensure!(fx.read(|root, _| root.thread().is_none() && root.recents().is_empty()));
    for key in ["cmd-[", "cmd-1", "cmd-k", "cmd-n"] {
        fx.keys(key)?;
    }
    ensure!(fx.read(|root, _| root.signed_out() && root.command_menu().is_none()));
    ensure!(fx.route() == Route::Home && fx.sent()? == requests);
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "account/login/completed".into(),
                params: json!({"success": true}),
            },
            cx,
        );
    });
    fx.wait("first signed-in chat page", |fx| {
        fx.read(|root, _| root.recents().len() >= 10)
    })?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |_, cx| {
        cx.emit(ui::screens::sidebar::LoadMore);
    });
    fx.wait("remaining signed-in chats", |fx| {
        fx.read(|root, _| root.recents().contains(&"after-login-10".into()))
    })?;
    fx.keys("cmd-n")?;
    fx.type_text("New account home message")?;
    fx.keys("enter")?;
    fx.wait_thread("after-login-new-chat")?;
    fx.type_text("New account thread message")?;
    fx.keys("enter")?;
    fx.wait("new account sends", |fx| {
        fx.sent().is_ok_and(|requests| {
            requests
                .iter()
                .filter(|r| r["method"] == "turn/start")
                .count()
                == 2
        })
    })?;
    let inputs: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "turn/start")
        .map(|r| r["params"]["input"].clone())
        .collect();
    ensure!(
        inputs
            == [
                json!([{"type": "text", "text": "New account home message", "text_elements": []}]),
                json!([{"type": "text", "text": "New account thread message", "text_elements": []}]),
            ],
        "Previous account attachments reached a new submission: {inputs:?}"
    );
    Ok(())
}
