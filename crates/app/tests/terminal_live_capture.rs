#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;
use anyhow::{Context as _, Result, ensure};
use bridge::{Bridge, Event};
use common::{Fx, SUBTRACT};
use serde_json::json;
use std::io::Write as _;
use std::process::Command;

fn wait_prompt(fx: &mut Fx) -> Result<()> {
    fx.wait("real shell prompt", |fx| {
        fx.read(|root, cx| {
            root.terminal_panel()
                .and_then(|p| p.read(cx).surface())
                .is_some_and(|surface| {
                    surface
                        .read(cx)
                        .buffer
                        .rows
                        .iter()
                        .any(|row| row.text(0, row.cells.len()).contains("fixture$"))
                })
        })
    })
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
#[ignore]
fn capture_live_terminal() -> Result<()> {
    common::install_panic_hook();
    let proxy = std::env::var("CGX_TERMINAL_PROXY").context("CGX_TERMINAL_PROXY")?;
    let record = std::env::var("CGX_TERMINAL_RECORD").context("CGX_TERMINAL_RECORD")?;
    let out = std::path::PathBuf::from(
        std::env::var("CGX_TERMINAL_EVIDENCE").context("CGX_TERMINAL_EVIDENCE")?,
    );
    std::fs::create_dir_all(&out)?;
    let mut fixture = tempfile::NamedTempFile::new()?;
    for row in common::script(None, |_| {})? {
        writeln!(fixture, "{row}")?;
    }
    let log = tempfile::NamedTempFile::new()?;
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join(".profile"),
        "PS1='fixture$ '\nexport PS1\n",
    )?;
    let mut command = Command::new("python3");
    command
        .args(["-u", &proxy])
        .arg(std::env::current_exe()?)
        .arg(fixture.path())
        .arg(log.path())
        .arg(record)
        .env("CODEX_HOME", home.path())
        .env("HOME", home.path())
        .env("SHELL", "/bin/sh");
    let (bridge, events) = Bridge::spawn(command)?;
    let mut fx = Fx::boot(
        "dark",
        (1280., 832.),
        bridge,
        events,
        common::recorded_host_settings()?,
    )?;
    let source_home = tempfile::tempdir()?;
    let directories = std::env::var_os("CGX_SOURCE_DIRECTORIES").is_some();
    if directories {
        std::fs::write(
            source_home.path().join(".codex-global-state.json"),
            serde_json::to_vec(&json!({
                "local-projects": {"terminal-sources": {
                    "id":"terminal-sources", "name":"Terminal sources",
                    "createdAt":1792083600000_i64, "updatedAt":1792083600000_i64,
                    "rootPaths":["/private/tmp/cgx-mac/proj","/var/tmp"]
                }},
                "thread-project-assignments": {SUBTRACT: {"projectKind":"local","projectId":"terminal-sources"}},
                "electron-persisted-atom-state": {"sidebar-project-expanded-v1-codex:terminal-sources": false}
            }))?,
        )?;
        let host = std::sync::Arc::new(app::host::LocalHost::new(source_home.path()));
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    }
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-j")?;
    wait_prompt(&mut fx)?;
    fx.shot()?.save(out.join("opening.png"))?;
    if directories {
        fx.cx.run_until_parked();
        fx.shot()?.save(out.join("source-directories.png"))?;
        fx.keys("alt-2")?;
        fx.type_text("pwd")?;
        fx.keys("enter")?;
        fx.wait("selected working directory", |fx| {
            fx.read(|root, cx| {
                root.terminal_panel()
                    .and_then(|p| p.read(cx).surface())
                    .is_some_and(|surface| {
                        surface
                            .read(cx)
                            .buffer
                            .rows
                            .iter()
                            .any(|row| row.text(0, row.cells.len()).contains("/var/tmp"))
                    })
            })
        })?;
        fx.shot()?.save(out.join("directory-selected.png"))?;
        return Ok(());
    }
    if std::env::var_os("CGX_TERMINAL_CONTROLS").is_some() {
        fx.keys("cmd-t")?;
        wait_prompt(&mut fx)?;
        fx.shot()?.save(out.join("new-tab-menu.png"))?;
        fx.keys("cmd-t")?;
        wait_prompt(&mut fx)?;
        let panel = fx
            .read(|root, _| root.terminal_panel().cloned())
            .context("terminal panel")?;
        let id = fx
            .read(|_, cx| panel.read(cx).process_id().map(str::to_owned))
            .context("terminal process")?;
        fx.root.update(&mut fx.cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: "command/exec/outputDelta".into(),
                    params: json!({"processId":id,"deltaBase64":"not-base64!"}),
                },
                cx,
            )
        });
        fx.cx.run_until_parked();
        fx.shot()?.save(out.join("render-error.png"))?;
        fx.click((808.25, 739.75))?;
        fx.wait("terminal reload", |fx| {
            fx.read(|_, cx| panel.read(cx).error().is_none())
        })?;
        fx.shot()?.save(out.join("reloaded.png"))?;
        panel.update(&mut fx.cx, |panel, cx| {
            panel.set_workspace("/var/tmp".into(), vec!["/var/tmp".into()], cx)
        });
        fx.keys("cmd-t")?;
        wait_prompt(&mut fx)?;
        panel.update(&mut fx.cx, |panel, cx| {
            panel.set_workspace(
                "/private/tmp/cgx-mac/proj".into(),
                vec!["/private/tmp/cgx-mac/proj".into()],
                cx,
            )
        });
        fx.shot()?.save(out.join("workspace-warning.png"))?;
        return Ok(());
    }
    fx.type_text("printf 'CGX_TERMINAL_READY\\n'")?;
    fx.keys("enter")?;
    fx.wait("actual terminal command output", |fx| {
        fx.read(|root, cx| {
            root.terminal_panel()
                .and_then(|p| p.read(cx).surface())
                .is_some_and(|surface| {
                    surface
                        .read(cx)
                        .buffer
                        .rows
                        .iter()
                        .any(|row| row.text(0, row.cells.len()) == "CGX_TERMINAL_READY")
                })
        })
    })?;
    fx.shot()?.save(out.join("running.png"))?;
    let id = fx
        .read(|root, cx| {
            root.terminal_panel()
                .and_then(|p| p.read(cx).process_id())
                .map(str::to_owned)
        })
        .context("active process")?;
    fx.keys("cmd-j cmd-j")?;
    ensure!(fx.read(
        |root, cx| root.terminal_panel().unwrap().read(cx).process_id() == Some(id.as_str())
    ));
    fx.shot()?.save(out.join("reopened.png"))?;
    fx.type_text("exit 0")?;
    fx.keys("enter")?;
    fx.wait("real shell exits", |fx| {
        fx.read(|root, cx| {
            root.terminal_panel()
                .and_then(|p| p.read(cx).surface())
                .is_some_and(|s| !s.read(cx).writable)
        })
    })?;
    fx.shot()?.save(out.join("exited.png"))?;
    Ok(())
}
