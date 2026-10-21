mod common;

use anyhow::{Result, ensure};
use common::{Fx, GREETING, exchange, script};
use serde_json::json;
use ui::dialog::Kind;

#[test]
fn pin_rename_and_find_shortcuts_act_on_the_open_chat() -> Result<()> {
    let mut recording = script(None, |_| {})?;
    recording.extend(exchange("thread/section/move", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), recording)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;

    fx.keys("cmd-f")?;
    ensure!(fx.read(|root, cx| root.thread().unwrap().read(cx).find_state().is_some()));
    fx.keys("escape")?;

    fx.keys("alt-cmd-p")?;
    fx.wait("pin request", |fx| {
        fx.sent()
            .map(|requests| {
                requests.iter().any(|r| {
                    r["method"] == "thread/section/move" && r["params"]["threadId"] == GREETING
                })
            })
            .unwrap_or(false)
    })?;

    fx.keys("alt-cmd-r")?;
    ensure!(fx.read(|root, cx| {
        root.dialog()
            .is_some_and(|d| d.read(cx).kind() == Kind::Rename)
    }));
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}
