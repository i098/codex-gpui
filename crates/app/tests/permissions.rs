mod common;

use anyhow::Result;
use common::Fx;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};
use ui::composer::{Approval, Permission};

#[test]
fn named_profile_selection_reaches_the_submission() -> Result<()> {
    let mut app = Fx::new()?;
    let composer = app.composer();
    let sent = Rc::new(RefCell::new(Vec::new()));
    let captured = sent.clone();
    app.cx.update_window(app.window, |_, _, cx| {
        composer.update(cx, |composer, cx| {
            composer.set_permission_profiles(
                &json!({"data": [
                    {"id": "research", "allowed": true, "description": "Read project files"},
                    {"id": "restricted", "allowed": false},
                ]}),
                cx,
            );
            composer
                .on_submit(move |submission, _, _| captured.borrow_mut().push(submission.clone()));
        });
    })?;
    if let Some(dir) = std::env::var_os("CGX_PERMISSION_SHOTS") {
        app.click(app.at("home", "Change permissions")?)?;
        app.shot()?
            .save(std::path::Path::new(&dir).join("named-permissions.png"))?;
        app.keys("escape")?;
    }
    app.click(app.at("home", "Change permissions")?)?;
    app.keys("r e s e a r c h enter")?;
    assert_eq!(
        composer.read_with(&app.cx, |composer, _| composer.permission()),
        Permission::Named("research".into())
    );
    app.focus_composer()?;
    app.type_text("Read the project")?;
    app.keys("enter")?;
    assert_eq!(
        sent.borrow()[0].permission,
        Permission::Named("research".into())
    );
    Ok(())
}

#[test]
fn human_command_preserves_request_identity_and_rejects_unoffered_acceptance() -> Result<()> {
    let mut app = Fx::new()?;
    let composer = app.composer();
    let replies = Rc::new(RefCell::new(Vec::new()));
    let captured = replies.clone();
    let request = serde_json::from_value(json!({
        "threadId": "thread-a", "turnId": "turn-b", "itemId": "command-c",
        "kind": "humanCommand", "command": "echo allowed",
        "availableDecisions": ["cancel"],
    }))?;
    app.cx.update_window(app.window, |_, _, cx| {
        composer.update(cx, |composer, cx| {
            composer.on_approval(move |id, thread, response, _, _| {
                captured.borrow_mut().push((id, thread, response));
            });
            composer.set_approval(Some(Approval::command(json!(17), &request)), cx);
        });
    })?;
    app.focus_composer()?;
    if let Some(dir) = std::env::var_os("CGX_PERMISSION_SHOTS") {
        app.shot()?
            .save(std::path::Path::new(&dir).join("human-command.png"))?;
    }
    app.keys("enter")?;
    assert!(
        replies.borrow().is_empty(),
        "Enter must not grant an unoffered decision"
    );
    assert!(composer.read_with(&app.cx, |composer, _| composer.approval().is_some()));
    // Escape must not turn a human-only checkpoint into a reusable grant.
    app.keys("escape")?;
    assert_eq!(
        *replies.borrow(),
        vec![(json!(17), "thread-a".into(), json!({"decision": "cancel"}))]
    );
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn approve_retries_the_current_threads_denial_only_once() -> Result<()> {
    use common::{GREETING, exchange, script};
    let mut lines = script(None, |_| {})?;
    let denied = json!({
        "threadId": GREETING, "turnId": "denied-turn", "reviewId": "review-1",
        "targetItemId": "command-1", "startedAtMs": 1, "completedAtMs": 2,
        "decisionSource": "agent",
        "review": {"status": "denied", "riskLevel": "low", "userAuthorization": "high", "rationale": "Test policy"},
        "action": {"type": "command", "source": "unifiedExec", "command": "echo proof", "cwd": "/tmp"},
    });
    let mut other = denied.clone();
    other["threadId"] = json!(common::SUBTRACT);
    other["reviewId"] = json!("other-review");
    lines.extend(exchange("thread/approveGuardianDeniedAction", json!({})));
    let mut app = Fx::captured("dark", (1280., 832.), lines)?;
    app.keys("cmd-1")?;
    app.wait_thread(GREETING)?;
    app.root.update(&mut app.cx, |root, cx| {
        for params in [denied, other] {
            root.on_event(
                bridge::Event::Notification {
                    method: "item/autoApprovalReview/completed".into(),
                    params,
                },
                cx,
            );
        }
    });
    app.cx.run_until_parked();
    app.focus_composer()?;
    app.type_text("/approve")?;
    app.keys("enter")?;
    app.wait("one retry approved", |app| {
        app.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|toast| toast == "One retry approved")
        })
    })?;
    app.type_text("/approve")?;
    app.keys("enter")?;
    let requests: Vec<_> = app
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "thread/approveGuardianDeniedAction")
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["params"]["threadId"], GREETING);
    assert_eq!(requests[0]["params"]["event"]["id"], "review-1");
    assert_eq!(
        requests[0]["params"]["event"]["action"]["source"],
        "unified_exec"
    );
    Ok(())
}
