mod common;

use anyhow::{Result, ensure};
use common::{Fx, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn mcp_slash_opens_existing_settings_without_sending_the_draft() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.focus_composer()?;
    fx.type_text("keep this /mcp")?;
    fx.keys("enter")?;
    ensure!(fx.route() == app::Route::Settings);
    fx.wait("MCP server status", |fx| {
        fx.sent().is_ok_and(|sent| {
            sent.iter()
                .any(|request| request["method"] == "mcpServerStatus/list")
        })
    })?;
    ensure!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "turn/start")
    );
    Ok(())
}

#[test]
fn slash_skill_inserts_the_same_link_as_a_mention() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.focus_composer()?;
    fx.type_text("keep this /amazon-bedrock")?;
    fx.keys("tab")?;
    let text = fx.read(|root, cx| root.composer(cx).read(cx).prompt());
    ensure!(
        text.starts_with("keep this [$amazon-bedrock]("),
        "skill invocation: {text}"
    );
    ensure!(text.ends_with("/SKILL.md) "));
    ensure!(fx.composer_focused()?);
    ensure!(fx.read(|root, cx| root.composer(cx).read(cx).slash_state().is_none()));
    Ok(())
}

#[test]
fn changed_skills_refresh_open_composers_and_remove_disabled_skills() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(common::exchange("skills/list", json!({"data": [{"cwd": common::CWD, "skills": [
        {"name": "fresh-skill", "description": "Refreshed skill", "path": "/skills/fresh/SKILL.md", "enabled": true},
        {"name": "disabled-skill", "path": "/skills/disabled/SKILL.md", "enabled": false}
    ]}]})));
    lines.extend(common::exchange("skills/list", json!({"data": []})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.focus_composer()?;
    fx.type_text("/")?;
    ensure!(fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .slash_state()
            .unwrap()
            .0
            .iter()
            .all(|row| !row.starts_with("Fresh Skill\n"))
    }));
    fx.keys("down down down")?;
    let selected = fx.read(|root, cx| root.composer(cx).read(cx).slash_state().unwrap().1);
    ensure!(selected.is_some_and(|index| index > 0));
    let root = fx.root.clone();
    fx.cx.update(|cx| {
        root.update(cx, |root, cx| {
            root.on_event(
                bridge::Event::Notification {
                    method: "skills/changed".into(),
                    params: json!({}),
                },
                cx,
            )
        })
    });
    fx.wait("updated skill menu", |fx| {
        fx.read(|root, cx| {
            root.composer(cx)
                .read(cx)
                .slash_state()
                .is_some_and(|state| state.0.iter().any(|row| row.starts_with("Fresh Skill\n")))
        })
    })?;
    ensure!(fx.read(|root, cx| root.composer(cx).read(cx).slash_state().unwrap().1) == selected);
    let sent = fx.sent()?;
    ensure!(
        sent.iter().any(|request| request["method"] == "skills/list"
            && request["params"]["forceReload"] == true)
    );
    fx.keys("cmd-a backspace")?;
    fx.type_text("/fresh-skill")?;
    fx.keys("tab")?;
    ensure!(
        fx.read(|root, cx| root.composer(cx).read(cx).prompt())
            == "[$fresh-skill](/skills/fresh/SKILL.md) "
    );
    fx.keys("cmd-a backspace")?;
    fx.type_text("/disabled-skill")?;
    ensure!(fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .slash_state()
            .unwrap()
            .0
            .is_empty()
    }));
    fx.keys("escape cmd-a backspace")?;
    fx.type_text("/fresh-skill")?;
    fx.cx.update(|cx| {
        root.update(cx, |root, cx| {
            root.on_event(
                bridge::Event::Notification {
                    method: "codex/event/skills_update_available".into(),
                    params: json!({}),
                },
                cx,
            )
        })
    });
    fx.wait("removed skill", |fx| {
        fx.read(|root, cx| {
            root.composer(cx)
                .read(cx)
                .slash_state()
                .is_some_and(|state| state.0.is_empty())
        })
    })?;
    Ok(())
}
