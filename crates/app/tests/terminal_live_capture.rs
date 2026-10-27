#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;
use anyhow::{Context as _, Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

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
#[ignore]
fn capture_live_terminal() -> Result<()> {
    common::install_panic_hook();
    let out = std::path::PathBuf::from(
        std::env::var("CGX_TERMINAL_EVIDENCE").context("CGX_TERMINAL_EVIDENCE")?,
    );
    std::fs::create_dir_all(&out)?;
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join(".profile"),
        "PS1='fixture$ '\nexport PS1\n",
    )?;
    // SAFETY: this ignored test is the only one in its binary.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("SHELL", "/bin/sh");
    }
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    // The originals show the Explore dot; main draws it from fetched unseen destinations, which
    // this replay cannot supply.
    let shell = fx.read(|root, _| root.shell().clone());
    shell.update(&mut fx.cx, |shell, _| {
        shell.unread_destinations = Some(["unseen".into()].into());
    });
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
                        let rows: Vec<String> = (surface.read(cx).buffer.rows.iter())
                            .map(|row| row.text(0, row.cells.len()))
                            .collect();
                        // `pwd`'s output and the prompt after it: the echoed
                        // `cd -- '/var/tmp'` holds the path before `pwd` has run.
                        rows.windows(2).any(|pair| {
                            pair[0].trim_end() == "/var/tmp" && pair[1].starts_with("fixture$")
                        })
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
        panel.update(&mut fx.cx, |panel, cx| {
            panel.fail("The terminal returned invalid output".into(), cx)
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
