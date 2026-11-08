#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::Event;
use common::{Fx, GREETING, exchange, refusal, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn open(lines: Vec<Value>) -> Result<Fx> {
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    Ok(fx)
}

fn notify(fx: &mut Fx, method: &str, params: Value) {
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

#[test]
fn permission_changes_update_the_live_turn_and_server_changes_restore_the_composer() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/settings/update", json!({})));
    lines.extend(exchange("turn/settings/update", json!({})));
    let mut fx = open(lines)?;
    notify(
        &mut fx,
        "turn/started",
        json!({
            "threadId": GREETING, "turn": {"id": "live", "status": "inProgress", "items": []},
        }),
    );
    fx.root.update(&mut fx.cx, |root, cx| {
        root.composer(cx)
            .update(cx, |composer, cx| composer.confirm_full_access(cx));
    });
    fx.wait("live reviewer request", |fx| {
        fx.sent().is_ok_and(|sent| {
            sent.iter()
                .any(|request| request["method"] == "turn/settings/update")
        })
    })?;
    let sent = fx.sent()?;
    let thread = sent
        .iter()
        .find(|request| request["method"] == "thread/settings/update")
        .context("thread settings request")?;
    ensure!(thread["params"]["threadId"] == GREETING);
    ensure!(thread["params"]["permissions"] == ":danger-full-access");
    ensure!(thread["params"]["approvalPolicy"] == "never");
    let turn = sent
        .iter()
        .find(|request| request["method"] == "turn/settings/update")
        .context("turn settings request")?;
    ensure!(turn["params"]["turnId"] == "live" && turn["params"]["approvalsReviewer"] == "user");
    notify(
        &mut fx,
        "thread/settings/updated",
        json!({
            "threadId": GREETING, "threadSettings": {
                "effort": "low", "permissions": ":workspace", "approvalsReviewer": "guardian_subagent",
            },
        }),
    );
    ensure!(fx.read(|root, cx| {
        let entity = root.composer(cx);
        let composer = entity.read(cx);
        composer.permission() == ui::composer::Permission::ApproveForMe
            && composer.run_settings().0["settings"]["reasoning_effort"] == "low"
    }));
    Ok(())
}

#[test]
fn old_servers_keep_next_turn_settings_but_real_errors_are_visible() -> Result<()> {
    for (message, expected_error) in [
        ("unknown variant `thread/settings/update`", false),
        ("permission denied", true),
    ] {
        let mut lines = script(None, |_| {})?;
        lines.extend(refusal("thread/settings/update", message));
        let mut fx = open(lines)?;
        fx.root.update(&mut fx.cx, |root, cx| {
            root.composer(cx)
                .update(cx, |composer, cx| composer.confirm_full_access(cx));
        });
        fx.wait("settings request", |fx| {
            fx.sent().is_ok_and(|sent| {
                sent.iter()
                    .any(|request| request["method"] == "thread/settings/update")
            })
        })?;
        if expected_error {
            // The refusal arrives over real I/O, so a fixed virtual-clock advance races it.
            fx.wait("settings error toast", |fx| {
                fx.read(|root, cx| root.toasts(cx))
                    .iter()
                    .any(|text| text.contains("Could not update thread settings"))
            })?;
        }
        fx.advance(std::time::Duration::from_millis(200))?;
        ensure!(
            fx.read(|root, cx| root.composer(cx).read(cx).permission())
                == ui::composer::Permission::FullAccess
        );
        let errors = fx.read(|root, cx| root.toasts(cx));
        ensure!(
            errors
                .iter()
                .any(|text| text.contains("Could not update thread settings"))
                == expected_error,
            "{errors:?}"
        );
    }
    Ok(())
}
