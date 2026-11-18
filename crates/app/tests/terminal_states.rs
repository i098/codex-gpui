#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bridge::Event;
use common::{Fx, SUBTRACT, exchange, script};
use gpui::Focusable as _;
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

/// Points `$SHELL` at a script that prints nothing, so the pseudo-terminal carries only what a
/// test feeds the surface: it writes its working directory to `<script>.pwd`, then exits with
/// `exit` or stays alive.
fn quiet_shell(exit: Option<i32>) -> Result<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    let path = std::env::temp_dir().join(format!("cgx-quiet-shell-{}", std::process::id()));
    let end = exit.map_or("exec sleep 3600".to_owned(), |code| format!("exit {code}"));
    std::fs::write(
        &path,
        format!("#!/bin/sh\npwd > \"$0.pwd\"\nstty -echo\n{end}\n"),
    )?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    let _ = std::fs::remove_file(format!("{}.pwd", path.display()));
    // SAFETY: the tests of this binary run on one thread (`--test-threads=1`).
    unsafe { std::env::set_var("SHELL", &path) };
    Ok(path)
}

/// Writes `bytes` to the active terminal tab as the shell's output.
fn feed(fx: &mut Fx, bytes: &[u8]) -> Result<()> {
    let surface = fx
        .read(|root, cx| {
            root.terminal_panel()
                .and_then(|p| p.read(cx).surface().cloned())
        })
        .context("terminal surface")?;
    surface.update(&mut fx.cx, |surface, cx| surface.append(bytes, cx));
    fx.cx.run_until_parked();
    Ok(())
}

fn open(fx: &mut Fx) -> Result<(String, String, serde_json::Value)> {
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    let mut command = None;
    let at = ui::screens::terminal::FOLLOW_UP_AT;
    for event in ui::screens::terminal::running_command_replay(at) {
        if let Event::Notification { method, params } = &event {
            if method == "item/commandExecution/outputDelta" {
                continue;
            }
            if method == "item/started" && params["item"]["type"] == "commandExecution" {
                command = Some(params.clone());
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
fn interactive_output_survives_hide_refit_and_copy() -> Result<()> {
    let recording: Vec<serde_json::Value> =
        include_str!("../../../fixtures/sessions/terminal-shell.jsonl")
            .lines()
            .map(serde_json::from_str)
            .collect::<serde_json::Result<_>>()?;
    quiet_shell(None)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-j")?;
    let id = fx
        .read(|root, cx| {
            root.terminal_panel()
                .and_then(|panel| panel.read(cx).process_id())
                .map(str::to_owned)
        })
        .context("interactive terminal")?;
    for record in recording
        .iter()
        .filter(|row| row["msg"]["method"] == "command/exec/outputDelta")
    {
        let bytes = STANDARD.decode(
            record["msg"]["params"]["deltaBase64"]
                .as_str()
                .context("recorded output")?,
        )?;
        feed(&mut fx, &bytes)?;
    }
    shot(&mut fx, "interactive-output")?;
    let terminal_focused = fx.cx.update_window(fx.window, |_, window, cx| {
        fx.root
            .read(cx)
            .terminal_panel()
            .unwrap()
            .read(cx)
            .surface()
            .unwrap()
            .focus_handle(cx)
            .is_focused(window)
    })?;
    ensure!(
        terminal_focused,
        "opening the terminal did not keep keyboard focus"
    );
    fx.keys("cmd-a cmd-c")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|item| item.text())
        .context("terminal selection")?;
    ensure!(copied.contains("\nTERMINAL_SMOKE\n"), "{copied:?}");
    ensure!(
        !copied.contains('\u{1b}'),
        "copy exposed ANSI control bytes"
    );
    fx.keys("cmd-j")?;
    ensure!(fx.read(|root, cx| root.shell().read(cx).bottom.is_none()));
    fx.resize(767., 832.)?;
    fx.keys("cmd-j")?;
    ensure!(fx.read(
        |root, cx| root.terminal_panel().unwrap().read(cx).process_id() == Some(id.as_str())
    ));
    fx.keys("cmd-a cmd-c")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|item| item.text())
        .context("refitted selection")?;
    ensure!(copied.contains("TERMINAL_SMOKE"), "{copied:?}");
    shot(&mut fx, "interactive-refit")?;
    Ok(())
}

#[test]
fn workspace_warning_and_reload_keep_the_terminal_session_scoped() -> Result<()> {
    quiet_shell(Some(7))?;
    let mut fx = Fx::captured("light", (767., 832.), script(None, |_| {})?)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-j")?;
    let panel = fx
        .read(|root, _| root.terminal_panel().cloned())
        .context("terminal panel")?;
    let id = fx
        .read(|_, cx| panel.read(cx).process_id().map(str::to_owned))
        .context("process id")?;
    fx.wait("shell exit", |fx| {
        fx.read(|_, cx| {
            !panel
                .read(cx)
                .surface()
                .is_some_and(|surface| surface.read(cx).writable)
        })
    })?;
    let cwd = fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned());
    panel.update(&mut fx.cx, |panel, cx| {
        panel.set_workspace("/var/tmp".into(), vec!["/var/tmp".into()], cx)
    });
    ensure!(fx.read(|_, cx| panel.read(cx).workspace_warning()));
    fx.cx.run_until_parked();
    shot(&mut fx, "interactive-workspace-warning")?;
    panel.update(&mut fx.cx, |panel, cx| {
        panel.set_workspace(cwd.clone(), vec![cwd], cx)
    });
    ensure!(!fx.read(|_, cx| panel.read(cx).workspace_warning()));
    panel.update(&mut fx.cx, |panel, cx| {
        panel.fail("The terminal returned invalid output".into(), cx)
    });
    fx.cx.run_until_parked();
    ensure!(fx.read(|_, cx| panel.read(cx).error().is_some()));
    shot(&mut fx, "interactive-error")?;
    let reload = fx
        .cx
        .debug_bounds(fx.window, "terminal-reload")?
        .context("terminal Reload button")?
        .center();
    fx.click((reload.x.into(), reload.y.into()))?;
    fx.wait("replacement terminal after Reload", |fx| {
        fx.read(|_, cx| {
            let panel = panel.read(cx);
            panel.process_id() != Some(id.as_str()) && panel.error().is_none()
        })
    })?;
    shot(&mut fx, "interactive-reloaded")?;
    Ok(())
}

#[test]
fn background_depth_retains_selection_scrollback_and_completed_output() -> Result<()> {
    use gpui::{Modifiers, MouseMoveEvent, PlatformInput, point, px};
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
    let append = |fx: &mut Fx, part: usize| {
        emit(
            fx,
            "item/commandExecution/outputDelta",
            json!({
                "threadId":SUBTRACT, "turnId":payload["turnId"], "itemId":payload["itemId"],
                "delta":payload["parts"][part]
            }),
        );
    };
    append(&mut fx, 0);
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
    fx.click((left - 21., 22.))?;
    // The original's press at xterm x + 2, y + 80 lands on the panel resizer, not the terminal.
    fx.click((left + 15., 136.))?;
    fx.hover((768., 6.))?;
    append(&mut fx, 1);
    shot(&mut fx, "scrollback-bottom")?;
    fx.scroll((left + 112., 156.), 12000.)?;
    shot(&mut fx, "scrollback-top")?;
    let before = fx.shot()?;
    append(&mut fx, 2);
    let after = fx.shot()?;
    let x = (left * 2.).ceil() as u32;
    let width = before.width() - x - 8;
    ensure!(
        image::imageops::crop_imm(&before, x, 88, width, 1568).to_image()
            == image::imageops::crop_imm(&after, x, 88, width, 1568).to_image(),
        "new output moved the manually scrolled terminal"
    );
    shot(&mut fx, "scrollback-hold")?;
    fx.scroll((left + 112., 156.), -12000.)?;
    shot(&mut fx, "scrollback-restored")?;
    fx.input(PlatformInput::MouseMove(MouseMoveEvent {
        position: point(px(left + 27.), px(790.25)),
        pressed_button: None,
        modifiers: Modifiers {
            platform: true,
            ..Default::default()
        },
    }))?;
    fx.advance(std::time::Duration::from_millis(700))?;
    shot(&mut fx, "link-hover")?;
    fx.click((581.2, 511.1))?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Changes)));
    fx.hover((left + 120., 22.))?;
    fx.advance(std::time::Duration::from_millis(700))?;
    shot(&mut fx, "thumbnail")?;
    fx.click((left + 120., 22.))?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Terminal)));
    fx.hover((768., 6.))?;
    emit(
        &mut fx,
        "item/completed",
        payload["itemComplete"]["params"].clone(),
    );
    shot(&mut fx, "completed")?;
    for width in [767., 600.] {
        fx.resize(width, 832.)?;
        shot(&mut fx, &format!("refit-{}", width as u32))?;
    }
    for width in [900., 960., 980., 984., 985., 1000., 1024.] {
        fx.resize(width, 832.)?;
        fx.advance(std::time::Duration::from_millis(250))?;
    }
    fx.resize(767., 832.)?;
    fx.advance(std::time::Duration::from_millis(300))?;
    fx.click((745., 22.))?;
    fx.hover((460., 6.))?;
    shot(&mut fx, "compact-open")?;
    let output = fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .terminal()
            .unwrap()
            .2
            .to_owned()
    });
    fx.resize(1280., 832.)?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        fx.root.update(cx, |root, cx| {
            root.set_theme(app::ThemeSetting::Light, window, cx)
        });
    })?;
    shot(&mut fx, "theme-light")?;
    fx.cx.update(|cx| {
        cx.global_mut::<app::host_settings::Settings>().write(
            "appearanceLightChromeTheme",
            json!({"fonts":{"code":"Courier New"}}),
        )
    })?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        fx.root.update(cx, |root, cx| {
            root.set_theme(app::ThemeSetting::Light, window, cx)
        });
    })?;
    shot(&mut fx, "font-changed")?;
    ensure!(fx.read(|root, cx| root.thread().unwrap().read(cx).terminal().unwrap().2 == output));
    Ok(())
}

#[test]
fn home_terminal_shortcuts_preserve_the_shell_and_project_directory() -> Result<()> {
    let shell = quiet_shell(None)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.keys("ctrl-`")?;
    let id = fx
        .read(|root, cx| {
            root.terminal_panel()
                .and_then(|panel| panel.read(cx).process_id())
                .map(str::to_owned)
        })
        .context("Home terminal process")?;
    ensure!(fx.read(|root, cx| root.shell().read(cx).bottom.is_some()));
    fx.keys("cmd-j")?;
    ensure!(fx.read(|root, cx| root.shell().read(cx).bottom.is_none()));
    fx.keys("ctrl-`")?;
    ensure!(fx.read(
        |root, cx| root.terminal_panel().unwrap().read(cx).process_id() == Some(id.as_str())
    ));
    fx.wait("shell start", |_| {
        std::fs::read_to_string(format!("{}.pwd", shell.display()))
            .is_ok_and(|dir| dir.trim_end() == common::CWD)
    })?;
    Ok(())
}
