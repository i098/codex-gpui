//! Replays the original thread actions from refs/interactions/s006.json.
mod common;

use anyhow::{Result, ensure};
use common::*;
use serde_json::{Value, json};

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn message_actions_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s006.json"))?;
    let steps = rec["steps"].as_array().unwrap();
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(target(&steps[0]))?;
    fx.wait_thread(SUBTRACT)?;
    fx.hover(target(&steps[1]))?;
    let before = fx.shot()?;
    fx.click(edit_target(&steps[2])?)?;
    let editing = fx.shot()?;
    let expected: [u8; 4] = serde_json::from_value(rec["result"]["editorSurface"]["rgba"].clone())?;
    ensure!(
        editing.get_pixel(1000, 380).0 == expected,
        "inline editor fill differs"
    );
    if let Some(path) = std::env::var_os("S006_PROOF") {
        editing.save(std::path::Path::new(&path).with_extension("editing.png"))?;
    }
    ensure!(
        changed(&before, &editing, [441., 129., 736., 100.]) > 1000,
        "Edit message did not open an inline editor"
    );
    ensure!(
        !fx.composer_focused()?,
        "Edit message kept focus in the docked composer"
    );
    fx.click(target(&steps[3]))?;
    for (step, expected) in steps[4..7]
        .iter()
        .zip(rec["result"]["tooltips"].as_array().unwrap())
    {
        fx.hover(fx.at("thread", expected["aria"].as_str().unwrap())?)?;
        fx.cx.advance_clock(std::time::Duration::from_millis(
            step["waitMs"].as_u64().unwrap(),
        ));
        fx.cx.run_until_parked();
        let shot = fx.shot()?;
        let tip = fx
            .cx
            .update(|cx| ui::tooltip::shown(cx).cloned())
            .expect("action tooltip");
        ensure!(
            Some(tip.label.as_ref()) == expected["tooltip"][0]["text"].as_str(),
            "action tooltip differs"
        );
        let x = (f32::from(tip.bounds.origin.x) + 3.) as u32 * 2;
        let y = (f32::from(tip.bounds.origin.y + tip.bounds.size.height / 2.)) as u32 * 2;
        ensure!(
            shot.get_pixel(x, y).0 == [45, 45, 45, 255],
            "tooltip fill differs from the original"
        );
    }
    fx.click(fx.at("thread", "Copy")?)?;
    let plain = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|c| c.text());
    ensure!(
        plain.as_deref() == rec["result"]["clipboard"][0]["text/plain"].as_str(),
        "response copy differs"
    );
    fx.hover(target(&steps[8]))?;
    fx.click(edit_target(&steps[9])?)?;
    fx.type_text(steps[10]["text"].as_str().unwrap())?;
    fx.click(target(&steps[11]))?;
    fx.wait("edit failure toast", |fx| {
        fx.read(|r, cx| r.toasts(cx).iter().any(|t| t == "Failed to edit message"))
    })?;
    ensure!(
        fx.composer_text()?.is_empty(),
        "inline typing changed the docked draft"
    );
    if let Some(path) = std::env::var_os("S006_PROOF") {
        fx.shot()?.save(path)?;
    }
    fx.click((500., 155.))?;
    fx.keys("cmd-a cmd-c")?;
    let draft = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|c| c.text());
    ensure!(
        draft.as_deref() == rec["result"]["failure"]["draft"].as_str(),
        "failed edit lost the draft"
    );
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| {
            matches!(
                r["method"].as_str(),
                Some("thread/revert" | "thread/rollback" | "turn/start")
            )
        })
        .collect();
    ensure!(
        &writes == rec["result"]["requests"].as_array().unwrap(),
        "a failed resume sent a write"
    );
    let pasted = "\u{10400}".repeat(2501);
    fx.cx
        .update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(pasted.clone())));
    fx.keys("cmd-v cmd-a cmd-c")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|c| c.text());
    ensure!(
        copied.as_deref() == Some(pasted.as_str()),
        "inline paste became an attachment"
    );
    Ok(())
}

#[test]
fn edit_reverts_before_resending() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s006.json"))?;
    let steps = rec["steps"].as_array().unwrap();
    let mut thread = recorded_thread(SUBTRACT)?;
    let turn_id = thread["turns"][0]["id"].clone();
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/resume", json!({"thread": thread})));
    thread["turns"] = json!([]);
    lines.extend(exchange("thread/revert", json!({"thread": thread})));
    let turn = json!({
        "id": "edited-turn", "status": "completed",
        "items": [{"type": "userMessage", "id": "edited-message",
            "content": [{"type": "text", "text": rec["result"]["edited"]}]}]
    });
    lines.extend(exchange("turn/start", json!({"turn": turn})));
    lines.push(json!({"dir": "recv", "msg": {"method": "turn/completed",
        "params": {"threadId": SUBTRACT, "turn": turn}}}));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(target(&steps[0]))?;
    fx.wait_thread(SUBTRACT)?;
    fx.hover(target(&steps[1]))?;
    fx.click(edit_target(&steps[2])?)?;
    fx.type_text(steps[10]["text"].as_str().unwrap())?;
    fx.click(target(&steps[11]))?;
    fx.wait("edited turn", |fx| {
        fx.read(|r, cx| {
            r.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .is_some_and(|t| t.id == "edited-turn")
        })
    })?;
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| matches!(r["method"].as_str(), Some("thread/revert" | "turn/start")))
        .collect();
    ensure!(
        writes
            == vec![
                json!({"method": "thread/revert", "params": {"threadId": SUBTRACT, "beforeTurnId": turn_id}}),
                json!({"method": "turn/start", "params": {
                    "threadId": SUBTRACT, "turnTrigger": "edit_user_message", "cwd": thread["cwd"],
                    "input": [{"type": "text", "text": rec["result"]["edited"], "text_elements": []}]
                }})
            ],
        "edit requests differ: {writes:?}"
    );
    ensure!(
        fx.read(|r, cx| r.thread().unwrap().read(cx).turns().len()) == 1,
        "the old turn remained"
    );
    Ok(())
}

fn edit_target(step: &Value) -> Result<(f32, f32)> {
    let spec: Value = serde_json::from_slice(&std::fs::read(format!(
        "{ROOT}/refs/thread/dark@1280x832.spec.json"
    ))?)?;
    let rect = &spec["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| {
            n["cls"]
                .as_str()
                .is_some_and(|s| s.contains("group/user-message"))
        })
        .unwrap()["rect"];
    Ok((
        target(step).0,
        (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap() - 13.) as f32,
    ))
}

fn target(step: &Value) -> (f32, f32) {
    let b = &step["target"]["bbox"];
    let n = |k: &str| b[k].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}
