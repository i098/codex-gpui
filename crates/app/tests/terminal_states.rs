#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use bridge::Event;
use common::{Fx, SUBTRACT, exchange, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

fn emit(fx: &mut Fx, method: &str, params: serde_json::Value) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: method.into(),
                params,
            },
            cx,
        );
    });
    fx.cx.run_until_parked();
}

fn open(fx: &mut Fx) -> Result<(String, String, serde_json::Value)> {
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    let mut command = None;
    for mut event in ui::screens::terminal::running_command_replay() {
        if let Event::Notification { method, params } = &mut event {
            if method == "item/commandExecution/outputDelta" {
                continue;
            }
            if method == "item/started" {
                command = Some(params.clone());
            }
            if params["turn"].is_object() {
                params["turn"]["rootTurnId"] = params["turn"]["id"].clone();
            }
        }
        fx.root
            .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
        fx.cx.run_until_parked();
    }
    fx.click((fx.size.0 - 63., 22.))?;
    fx.click((fx.size.0 - 202., 179.7))?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Terminal)));
    let command = command.context("running command")?;
    Ok((
        command["turnId"].as_str().unwrap().into(),
        command["item"]["id"].as_str().unwrap().into(),
        command,
    ))
}

fn save_shot(image: image::RgbaImage, name: &str) -> Result<()> {
    let dir = std::env::var_os("CGX_TERMINAL_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-terminal-release"));
    std::fs::create_dir_all(&dir)?;
    image.save(dir.join(format!("{name}.png")))?;
    Ok(())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    save_shot(fx.shot()?, name)
}

#[test]
fn completion_keeps_the_selected_buffer_when_other_commands_run() -> Result<()> {
    let mut fx = Fx::new()?;
    let (turn, item, mut completed) = open(&mut fx)?;
    shot(&mut fx, "no-output")?;
    emit(
        &mut fx,
        "item/commandExecution/outputDelta",
        json!({
            "threadId": SUBTRACT, "turnId": turn, "itemId": item, "delta": "2\n",
        }),
    );
    fx.hover(((fx.size.0 * 0.6).round(), 0.))?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        ui::composer::Composer::set_caret_transparent(cx, true);
        window.refresh();
    })?;
    let streaming = fx.shot();
    fx.cx.update_window(fx.window, |_, window, cx| {
        ui::composer::Composer::set_caret_transparent(cx, false);
        window.refresh();
    })?;
    save_shot(streaming?, "streaming")?;
    let delta = format!("first command\n{}\n\nlast line\n", "0123456789".repeat(20));
    let output = format!("2\n{delta}");
    emit(
        &mut fx,
        "item/commandExecution/outputDelta",
        json!({
            "threadId": SUBTRACT, "turnId": turn, "itemId": item, "delta": delta,
        }),
    );
    let mut other = completed.clone();
    other["item"]["id"] = "other-running-command".into();
    other["item"]["command"] = "other command".into();
    other["item"]["commandActions"] = json!([]);
    other["item"]["aggregatedOutput"] = "other output".into();
    emit(&mut fx, "item/started", other);
    completed["item"]["status"] = "failed".into();
    completed["item"]["exitCode"] = 17.into();
    completed["item"]
        .as_object_mut()
        .unwrap()
        .remove("aggregatedOutput");
    emit(&mut fx, "item/completed", completed);
    ensure!(
        fx.read(|root, cx| {
            let thread = root.thread().unwrap().read(cx);
            thread
                .terminal()
                .is_some_and(|(id, _, buffer)| id == item && buffer == output)
                && thread.background_terminals() == [("other command".into(), "other output")]
                && thread
                    .turns()
                    .iter()
                    .flat_map(|turn| &turn.items)
                    .any(|entry| {
                        matches!(entry,
                bridge::types::ThreadItem::CommandExecution { id, status, exit_code: Some(17), .. }
                    if id == &item && status == "failed")
                    })
        }),
        "completion lost output, identity, or failure state"
    );
    fx.keys("alt-cmd-b")?;
    fx.keys("alt-cmd-b")?;
    ensure!(fx.read(|root, cx| root.thread().unwrap().read(cx).terminal().unwrap().2 == output));
    for width in [1280., 767., 600.] {
        fx.resize(width, 832.)?;
        shot(&mut fx, &format!("retained-{}", width as u32))?;
    }
    fx.resize(1280., 832.)?;
    fx.hover((750., 22.))?;
    shot(&mut fx, "thumbnail")?;
    fx.hover((1000., 400.))?;
    let mut focused = false;
    for _ in 0..60 {
        fx.keys("tab")?;
        let image = fx.shot()?;
        let scale = image.width() / 1280;
        let blue = (5 * scale..39 * scale)
            .flat_map(|y| (670 * scale..800 * scale).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = image.get_pixel(x, y).0;
                p[0] < 100 && p[1] > 80 && p[1] < 180 && p[2] > 200
            })
            .count();
        if blue > 20 {
            focused = true;
            break;
        }
    }
    ensure!(
        focused,
        "the terminal tab has no visible keyboard focus ring"
    );
    shot(&mut fx, "keyboard-focus")?;
    fx.keys("space")?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Terminal)));
    fx.keys("tab enter")?;
    ensure!(
        fx.read(|root, _| root.side_panel().is_none()),
        "keyboard close did not close the terminal"
    );
    Ok(())
}

#[test]
fn stop_failure_keeps_commands_and_allows_a_retry() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let mut refused = exchange("thread/backgroundTerminals/clean", json!({}));
    refused[1]["msg"].as_object_mut().unwrap().remove("result");
    refused[1]["msg"]["error"] = json!({"code": -32000, "message": "isolated stop refusal"});
    lines.extend(refused);
    lines.extend(exchange("thread/backgroundTerminals/clean", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let (_, _, mut completed) = open(&mut fx)?;
    let mut other = completed.clone();
    other["item"]["id"] = "second-background-command".into();
    other["item"]["command"] = "second command".into();
    other["item"]["commandActions"] = json!([]);
    emit(&mut fx, "item/started", other.clone());
    let panel_width = fx.cx.update_window(fx.window, |_, window, cx| {
        f32::from(ui::screens::diff::panel::panel_width(window, cx))
    })?;
    let stop = (fx.size.0 - 4. - panel_width - 36., 179.7);
    fx.click(stop)?;
    fx.wait("stop failure toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title == "Unable to stop background terminals")
        })
    })?;
    ensure!(fx.read(|root, cx| {
        let thread = root.thread().unwrap().read(cx);
        !thread.terminals_stopping && thread.background_terminals().len() == 2
    }));
    shot(&mut fx, "stop-failure")?;
    let image = fx.shot()?;
    let scale = image.width() / 1280;
    let visible_error = (50 * scale..85 * scale)
        .flat_map(|y| (500 * scale..620 * scale).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = image.get_pixel(x, y).0;
            p[0] > 100 && p[0] / 2 > p[1] && p[0] / 2 > p[2]
        })
        .count();
    ensure!(
        visible_error > 40,
        "the summary covers the stop failure toast"
    );
    fx.click(stop)?;
    fx.wait("stop retry completes", |fx| {
        fx.read(|root, cx| !root.thread().unwrap().read(cx).terminals_stopping)
    })?;
    let requests: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "thread/backgroundTerminals/clean")
        .map(|request| request["params"].clone())
        .collect();
    ensure!(
        requests == [json!({"threadId": SUBTRACT}), json!({"threadId": SUBTRACT})],
        "{requests:?}"
    );
    for command in [&mut completed, &mut other] {
        command["item"]["status"] = "completed".into();
        command["item"]["exitCode"] = 0.into();
        command["item"]["aggregatedOutput"] = "done\n".into();
        emit(&mut fx, "item/completed", command.clone());
    }
    ensure!(
        fx.read(|root, cx| {
            let thread = root.thread().unwrap().read(cx);
            thread.background_terminals().is_empty() && thread.terminal().unwrap().2 == "done\n"
        }),
        "successful clean lost its retained output or kept a completed command running"
    );
    shot(&mut fx, "stopped")?;
    Ok(())
}

#[test]
fn background_ansi_output_supports_selection_and_copy() -> Result<()> {
    let payload: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/terminal-depth.json"))?;
    let mut replay = script(None, |_| {})?;
    let thread_list: serde_json::Value = ui::fixture::response("readback.jsonl", "thread/list", 0);
    replay.extend(exchange("thread/list", thread_list));
    let mut fx = Fx::captured("dark", (1280., 832.), replay)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    for event in payload["start"]
        .as_array()
        .context("recorded start events")?
    {
        emit(
            &mut fx,
            event["method"].as_str().context("method")?,
            event["params"].clone(),
        );
    }
    emit(
        &mut fx,
        "turn/completed",
        payload["turnComplete"]["params"].clone(),
    );
    emit(
        &mut fx,
        "thread/status/changed",
        payload["threadIdle"]["params"].clone(),
    );
    fx.click((1217., 22.))?;
    fx.click((1078., 179.7))?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Terminal)));
    emit(
        &mut fx,
        "item/commandExecution/outputDelta",
        json!({
            "threadId":SUBTRACT, "turnId":payload["turnId"], "itemId":payload["itemId"],
            "delta":payload["parts"][0]
        }),
    );
    let left = fx.cx.update_window(fx.window, |_, window, cx| {
        1280. - 4. - f32::from(ui::screens::diff::panel::panel_width(window, cx))
    })?;
    fx.hover((768., 6.))?;
    // The originals show the Explore dot; main draws it from fetched unseen destinations, which
    // this replay cannot supply.
    let shell = fx.read(|root, _| root.shell().clone());
    shell.update(&mut fx.cx, |shell, _| {
        shell.unread_destinations = Some(["unseen".into()].into());
    });
    shot(&mut fx, "ansi")?;
    fx.drag((left + 14., 80.), (left + 114., 112.), 4)?;
    shot(&mut fx, "selection")?;
    let terminal_focused = fx.cx.update_window(fx.window, |_, window, _| {
        window
            .context_stack()
            .iter()
            .any(|context| context.contains("Terminal"))
    })?;
    ensure!(terminal_focused, "drag did not focus the terminal");
    fx.keys("cmd-c")?;
    let selected = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .context("selected terminal text")?;
    ensure!(selected == "ANSI indexed\nANSI RGB\n", "{selected:?}");
    Ok(())
}
