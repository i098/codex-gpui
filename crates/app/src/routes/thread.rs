use crate::host::run_now::heartbeat::RendererState;
use crate::*;
use anyhow::{Context as _, bail, ensure};
use gpui::Div;

pub(crate) async fn load_thread(
    bridge: &Bridge,
    this: &WeakEntity<Root>,
    id: &str,
    resume: bool,
    cx: &mut AsyncWindowContext,
) -> Result<(Thread, Option<RendererState>)> {
    let response: Value = bridge
        .request("thread/read", json!({"threadId": id, "includeTurns": true}))
        .await?;
    if response["thread"]["threadSource"] == "automation" {
        this.update(cx, |this, _| this.automation_threads.insert(id.to_owned()))?;
    }
    let thread = serde_json::from_value::<ThreadReadResponse>(response)?.thread;
    let settings = if resume {
        let mut params = resume_params(this, id, cx).await?;
        params["excludeTurns"] = json!(true);
        let response = bridge.request::<_, Value>("thread/resume", params).await?;
        Some(resumed_settings(response, id)?)
    } else {
        None
    };
    Ok((thread, settings))
}

pub(crate) fn forked_thread(mut response: Value) -> Result<(Thread, RendererState)> {
    let thread: Thread = serde_json::from_value(
        response
            .get_mut("thread")
            .context("missing forked thread")?
            .take(),
    )?;
    let collaboration_mode = json!({
        "mode": "default",
        "settings": {
            "model": response["model"],
            "reasoning_effort": response["reasoningEffort"],
        },
    });
    let settings = run_settings(&response, collaboration_mode)?;
    Ok((thread, settings))
}

fn resumed_settings(mut response: Value, id: &str) -> Result<RendererState> {
    ensure!(response["thread"]["id"] == id, "resumed a different thread");
    let mode = response
        .get_mut("collaborationMode")
        .context("missing resumed model settings")?
        .take();
    run_settings(&response, mode)
}

fn run_settings(response: &Value, collaboration_mode: Value) -> Result<RendererState> {
    let sandbox = match response["sandbox"]["type"].as_str() {
        Some("readOnly") => "read-only",
        Some("workspaceWrite") => "workspace-write",
        Some("dangerFullAccess") => "danger-full-access",
        _ => bail!("missing or unsupported resumed sandbox"),
    };
    let approval = response
        .get("approvalPolicy")
        .context("missing resumed approval policy")?;
    let policy = &response["sandbox"];
    let standard_workspace = policy.as_object().is_some_and(|policy| policy.len() == 5)
        && policy["networkAccess"] == false
        && policy["excludeTmpdirEnvVar"] == false
        && policy["excludeSlashTmp"] == false
        && policy["writableRoots"]
            .as_array()
            .is_some_and(Vec::is_empty);
    let permission = if sandbox == "workspace-write" && !standard_workspace {
        Permission::Custom
    } else {
        Permission::from_config(&json!({
            "approval_policy": approval,
            "approvals_reviewer": response["approvalsReviewer"],
            "sandbox_mode": sandbox,
        }))
    };
    ensure!(
        collaboration_mode["mode"].is_string()
            && collaboration_mode["settings"]["model"]
                .as_str()
                .is_some_and(|model| !model.is_empty()),
        "missing resumed model settings"
    );
    Ok(RendererState {
        collaboration_mode,
        permission,
    })
}

fn find_in_workspace(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) {
    if !matches!(root.state.route, Route::Thread(_)) {
        return;
    }
    let full = root.workspace_layout() == WorkspaceLayout::Full;
    if root.side_visible(SidePanel::Changes)
        && let Some(panel) = &root.diff
        && panel.update(cx, |panel, cx| {
            if full {
                panel.focus(window, cx);
            }
            panel.open_find(window, cx)
        })
    {
        return;
    }
    if !full && let Some(thread) = &root.thread {
        thread.update(cx, |thread, cx| thread.open_find(window, cx));
    }
}

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &FindInThread, window, cx| {
        find_in_workspace(this, window, cx);
    }))
    .on_action(cx.listener(Root::find_in_chat))
    .on_action(cx.listener(Root::find_in_diff))
    .on_action(cx.listener(|this, _: &FindNext, _, cx| this.next_search_result(false, cx)))
    .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.next_search_result(true, cx)))
    .on_action(cx.listener(|this, _: &ui::composer::Stop, _, cx| this.stop(None, cx)))
    .on_action(
        cx.listener(|this, _: &ui::composer::RequestFullAccess, window, cx| {
            this.open_dialog(DialogKind::FullAccess, window, cx);
        }),
    )
    .on_action(cx.listener(|this, _: &ToggleChangesPanel, window, cx| {
        if this.side_visible(SidePanel::Changes) {
            this.close_side(SidePanel::Changes, window, cx)
        } else {
            this.set_side(SidePanel::Changes, window, cx)
        }
    }))
    .on_action(cx.listener(|this, _: &OpenChangesPanel, window, cx| {
        this.set_side(SidePanel::Changes, window, cx)
    }))
    .on_action(cx.listener(|this, action: &ui::ViewChanges, window, cx| {
        let focused = window.focused(cx);
        if let Some(panel) = &this.diff {
            panel.update(cx, |panel, cx| panel.view_changes(&action.0, cx));
        }
        this.set_side(SidePanel::Changes, window, cx);
        if let Some(focused) = focused {
            window.defer(cx, move |window, cx| window.focus(&focused, cx));
        }
    }))
    .on_action(cx.listener(Root::open_file))
    .on_action(cx.listener(|this, _: &OpenBackgroundTerminal, window, cx| {
        this.set_side(SidePanel::Terminal, window, cx)
    }))
    .on_action(cx.listener(|this, action: &OpenTerminalItem, window, cx| {
        if let Some(thread) = &this.thread {
            thread.update(cx, |thread, cx| thread.select_terminal(Some(&action.0), cx));
        }
        if this.side_visible(SidePanel::Terminal) {
            this.sync(window, cx);
        } else {
            this.set_side(SidePanel::Terminal, window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &StopBackgroundTerminals, _, cx| {
        this.stop_background_terminals(cx);
    }))
    .on_action(cx.listener(|this, a: &ActivateSideTab, window, cx| {
        if let Some(side) = this.tabs.get(a.0).copied() {
            this.set_side(side, window, cx)
        }
    }))
    .on_action(cx.listener(|this, a: &CloseSideTab, window, cx| {
        if let Some(side) = this.tabs.get(a.0).copied() {
            this.close_side(side, window, cx)
        }
    }))
    .on_action(
        cx.listener(|this, _: &OpenCommandMenu, window, cx| this.open_command_menu(window, cx)),
    )
    .on_action(cx.listener(|this, _: &PreviousChat, window, cx| this.step_chat(-1, window, cx)))
    .on_action(cx.listener(|this, _: &NextChat, window, cx| this.step_chat(1, window, cx)))
    .on_action(
        cx.listener(|this, a: &GoToRecentChat, window, cx| this.go_to_recent_chat(a.0, window, cx)),
    )
    .on_action(
        cx.listener(|this, a: &PinThread, window, cx| {
            this.set_pinned(a.0.clone(), a.1, window, cx)
        }),
    )
    .on_action(cx.listener(|this, a: &RenameThread, window, cx| {
        this.open_rename(a.0.clone(), false, window, cx)
    }))
    .on_action(cx.listener(|this, a: &ForkThread, window, cx| this.fork(&a.0, window, cx)))
    .on_action(cx.listener(|this, edit: &EditMessage, window, cx| {
        this.edit_message(edit.clone(), window, cx)
    }))
    .on_action(cx.listener(|this, a: &RevertTurn, window, cx| this.revert(a.0.clone(), window, cx)))
    .on_action(cx.listener(|this, a: &GoToChat, window, cx| this.go_to_chat(Some(a.0), window, cx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Value {
        json!({
            "thread": {"id": "thread-a"},
            "approvalPolicy": "on-request",
            "approvalsReviewer": "user",
            "sandbox": {
                "type": "workspaceWrite",
                "writableRoots": [],
                "networkAccess": false,
                "excludeTmpdirEnvVar": false,
                "excludeSlashTmp": false
            },
            "collaborationMode": {
                "mode": "default",
                "settings": {"model": "gpt-6.1-sol", "reasoning_effort": null}
            }
        })
    }

    #[test]
    fn resumed_state_preserves_the_server_model_and_permission() {
        let state = resumed_settings(response(), "thread-a").unwrap();
        assert_eq!(state.permission, Permission::Ask);
        assert_eq!(state.collaboration_mode["settings"]["model"], "gpt-6.1-sol");
        assert!(state.collaboration_mode["settings"]["reasoning_effort"].is_null());
    }

    #[test]
    fn resumed_custom_workspace_policy_keeps_the_custom_label() {
        for (field, value) in [
            ("networkAccess", json!(true)),
            ("writableRoots", json!(["/extra"])),
            ("excludeTmpdirEnvVar", json!(true)),
            ("excludeSlashTmp", json!(true)),
            ("unknownPolicyField", json!(true)),
        ] {
            let mut response = response();
            response["sandbox"][field] = value;
            assert_eq!(
                resumed_settings(response, "thread-a").unwrap().permission,
                Permission::Custom,
                "{field}"
            );
        }
    }

    #[test]
    fn malformed_fork_reply_returns_an_error() {
        for response in [Value::Null, json!("invalid"), json!([]), json!({})] {
            assert!(forked_thread(response).is_err());
        }
    }

    #[test]
    fn resumed_state_rejects_a_different_thread() {
        assert!(resumed_settings(response(), "thread-b").is_err());
    }
}
