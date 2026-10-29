#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use common::{Fx, GREETING, exchange, recorded_thread, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn open_chat(full_access: bool, accept: bool) -> Result<Fx> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/resume",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    if accept {
        lines.extend(exchange("thread/shellCommand", json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    if full_access {
        let composer = fx.composer();
        fx.cx.update_window(fx.window, |_, _, cx| {
            composer.update(cx, |composer, cx| composer.confirm_full_access(cx));
        })?;
    }
    Ok(fx)
}

fn shell_requests(fx: &mut Fx) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "thread/shellCommand")
        .collect())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(dir) = std::env::var_os("CGX_SHELL_SHOTS") {
        fx.shot()?
            .save(std::path::Path::new(&dir).join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn sandboxed_shell_is_disabled_and_keeps_the_draft() -> Result<()> {
    let mut fx = open_chat(false, true)?;
    fx.type_text("!printf shell-proof")?;
    shot(&mut fx, "disabled")?;
    fx.keys("enter")?;
    ensure!(shell_requests(&mut fx)?.is_empty());
    ensure!(fx.composer_text()? == "!printf shell-proof");
    ensure!(fx.sent()?.iter().all(|r| r["method"] != "turn/start"));
    Ok(())
}

#[test]
fn shell_confirmation_cancel_keeps_the_draft() -> Result<()> {
    let mut fx = open_chat(true, true)?;
    fx.type_text("!printf shell-proof")?;
    shot(&mut fx, "before")?;
    fx.keys("enter")?;
    shot(&mut fx, "confirmation")?;
    ensure!(shell_requests(&mut fx)?.is_empty());
    fx.keys("escape")?;
    ensure!(fx.composer_text()? == "!printf shell-proof");
    ensure!(fx.composer_focused()?);
    ensure!(shell_requests(&mut fx)?.is_empty());
    ensure!(fx.sent()?.iter().all(|r| r["method"] != "turn/start"));
    Ok(())
}

#[test]
fn confirmed_shell_uses_thread_shell_command_not_a_model_turn() -> Result<()> {
    let mut fx = open_chat(true, true)?;
    fx.type_text("!printf '%s\\n' shell-proof | cat")?;
    fx.keys("enter tab enter")?;
    fx.wait("shell request", |fx| {
        shell_requests(fx).is_ok_and(|r| !r.is_empty())
    })?;
    ensure!(
        shell_requests(&mut fx)?[0]["params"]
            == json!({
                "threadId": GREETING, "command": "printf '%s\\n' shell-proof | cat"
            })
    );
    ensure!(fx.composer_text()?.is_empty());
    ensure!(
        fx.sent()?
            .iter()
            .all(|r| r["method"] != "turn/start" && r["method"] != "thread/queue/add")
    );
    Ok(())
}

#[test]
fn shell_rpc_refusal_restores_the_draft_during_a_model_turn() -> Result<()> {
    let mut fx = open_chat(true, false)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "turn/started".into(),
                params: json!({
                    "threadId": GREETING,
                    "turn": {"id": "shell-busy", "items": [], "status": "inProgress"}
                }),
            },
            cx,
        );
    });
    fx.cx.run_until_parked();
    ensure!(fx.read(|root, cx| root.thread().is_some_and(|view| {
        view.read(cx)
            .turns()
            .iter()
            .any(|turn| turn.status == bridge::types::TurnStatus::InProgress)
    })));
    fx.type_text("!printf shell-proof")?;
    fx.keys("enter tab enter")?;
    fx.wait("shell refusal", |fx| {
        fx.composer_text().is_ok_and(|t| t == "!printf shell-proof")
    })?;
    ensure!(shell_requests(&mut fx)?.len() == 1);
    ensure!(fx.read(|root, cx| {
        root.toasts(cx)
            .iter()
            .any(|t| t == "Error submitting message")
    }));
    Ok(())
}
