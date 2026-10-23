mod common;

use anyhow::{Context as _, Result};
use app::host::{Host, Identity, LocalHost, Request};
use bridge::Event;
use common::Fx;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};

const DESTINATION: &str = "mcp:[\"mcp-global\",\"notes\",\"open\",\"global\"]";

struct StoredHost {
    local: LocalHost,
    identity: Mutex<Identity>,
}

impl Host for StoredHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        self.local.fetch(request)
    }

    fn identity(&self) -> Option<Identity> {
        Some(self.identity.lock().unwrap().clone())
    }
}

fn catalog(line: &mut Value) {
    if line["dir"] == "recv" && line["msg"]["result"]["data"][0]["name"] == "codex_apps" {
        line["msg"]["result"] = json!({
            "data": [{
                "name": "notes", "pluginId": null, "toolsError": null,
                "tools": {"open": {
                    "name": "open", "inputSchema": {"type": "object"},
                    "_meta": {
                        "ui": {"resourceUri": "ui://notes/app"},
                        "openai/ui": {"entrypoints": [{"type": "global"}]}
                    }
                }}
            }],
            "nextCursor": null
        });
    }
}

fn write_state(home: &Path, pinned: bool, seen: bool) -> Result<()> {
    let read: Vec<&str> = if seen { vec![DESTINATION] } else { Vec::new() };
    let state = json!({"electron-persisted-atom-state": {
        "inline-sidebar-customization-state-by-account-id-v2": {
            "version": 2, "orderByAccountId": {},
            "visibilityByAccountId": {"account-a": {(DESTINATION): pinned}}
        },
        "sidebar-more-menu-seen-mcp-destination-ids-by-account-id-v1": {
            "account-a": read
        }
    }});
    std::fs::write(
        home.join(".codex-global-state.json"),
        serde_json::to_vec(&state)?,
    )?;
    Ok(())
}

fn refresh(fx: &mut Fx, method: &str) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: method.into(),
                params: json!({}),
            },
            cx,
        );
    });
}

fn wait_for(fx: &mut Fx, expected: &[&str]) -> Result<()> {
    fx.wait("source-bound navigation state", |fx| {
        fx.read(|root, cx| {
            root.shell()
                .read(cx)
                .unread_destinations
                .as_ref()
                .is_some_and(|ids| {
                    ids.iter()
                        .map(|id| id.as_ref())
                        .eq(expected.iter().copied())
                })
        })
    })
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_DESTINATION_EVIDENCE") {
        std::fs::create_dir_all(&directory)?;
        fx.reference_shot()?
            .save(Path::new(&directory).join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn stored_pins_and_seen_state_drive_the_marker_across_accounts() -> Result<()> {
    let home = tempfile::tempdir()?;
    write_state(home.path(), false, false)?;
    let host = Arc::new(StoredHost {
        local: LocalHost::new(home.path()),
        identity: Mutex::new(Identity {
            account_id: "account-a".into(),
            user_id: "user-a".into(),
            device_id: "test".into(),
        }),
    });
    let mut script = common::script(None, catalog)?;
    let account = script
        .iter()
        .find(|line| line["msg"]["result"]["account"].is_object())
        .context("recorded account")?["msg"]["result"]
        .clone();
    let mcp = script
        .iter()
        .find(|line| line["msg"]["result"]["data"][0]["name"] == "notes")
        .context("recorded MCP catalog")?["msg"]["result"]
        .clone();
    for _ in 0..8 {
        script.extend(common::exchange("account/read", account.clone()));
        script.extend(common::exchange("mcpServerStatus/list", mcp.clone()));
    }
    let mut fx = Fx::scripted(script)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    wait_for(&mut fx, &[DESTINATION])?;
    capture(&mut fx, "unread")?;
    #[cfg(target_os = "macos")]
    {
        let pixel = fx.reference_shot()?.get_pixel(72, 472).0;
        assert!(
            pixel[2] > pixel[0],
            "An unread unpinned destination must show the blue marker"
        );
    }

    write_state(home.path(), true, false)?;
    refresh(&mut fx, "mcpServer/startupStatus/updated");
    wait_for(&mut fx, &[])?;
    capture(&mut fx, "pinned")?;
    #[cfg(target_os = "macos")]
    {
        let pixel = fx.reference_shot()?.get_pixel(72, 472).0;
        assert_eq!(
            pixel[0], pixel[2],
            "A pinned destination must not light Explore"
        );
    }

    write_state(home.path(), false, false)?;
    refresh(&mut fx, "mcpServer/startupStatus/updated");
    wait_for(&mut fx, &[DESTINATION])?;
    write_state(home.path(), false, true)?;
    refresh(&mut fx, "mcpServer/startupStatus/updated");
    wait_for(&mut fx, &[])?;
    capture(&mut fx, "read")?;

    host.identity.lock().unwrap().account_id = "account-b".into();
    refresh(&mut fx, "account/updated");
    wait_for(&mut fx, &[DESTINATION])?;
    capture(&mut fx, "account-b-unread")?;
    host.identity.lock().unwrap().account_id = "account-a".into();
    refresh(&mut fx, "account/updated");
    wait_for(&mut fx, &[])?;
    capture(&mut fx, "account-a-read")?;
    Ok(())
}
