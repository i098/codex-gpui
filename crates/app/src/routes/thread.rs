use crate::host::run_now::heartbeat::RendererState;
use crate::*;
use anyhow::{Context as _, bail, ensure};
use bridge::types::Turn;
use gpui::Div;

pub(crate) async fn load_thread(
    bridge: &Bridge,
    this: &WeakEntity<Root>,
    id: &str,
    resume: bool,
    cx: &mut AsyncWindowContext,
) -> Result<(Thread, Option<RendererState>)> {
    let response: Value = bridge
        .request(
            "thread/read",
            json!({"threadId": id, "includeTurns": false}),
        )
        .await?;
    if response["thread"]["threadSource"] == "automation" {
        this.update(cx, |this, _| this.automation_threads.insert(id.to_owned()))?;
    }
    let mut thread = serde_json::from_value::<ThreadReadResponse>(response)?.thread;
    match turns_page(bridge, id, None).await {
        Ok((turns, cursor)) => {
            thread.turns = turns;
            thread.older_cursor = cursor;
        }
        Err(error) => {
            // A server without `thread/turns/list` still serves the whole history.
            eprintln!("codex-gpui: thread/turns/list: {error:#}; reading all turns");
            thread = bridge.thread_read(id, true).await?.thread;
        }
    }
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

/// Turns per `thread/turns/list` page; the original asks for 5, the Load earlier row asks again.
const TURNS_PER_PAGE: u32 = 10;

/// One page of turns, oldest first, and the cursor of the page before it.
pub(crate) async fn turns_page(
    bridge: &Bridge,
    id: &str,
    cursor: Option<&str>,
) -> Result<(Vec<Turn>, Option<String>)> {
    let params = json!({
        "threadId": id,
        "cursor": cursor,
        "limit": TURNS_PER_PAGE,
        "itemsView": "full",
        "sortDirection": "desc",
    });
    parse_turns_page(bridge.request("thread/turns/list", params).await?)
}

fn parse_turns_page(mut response: Value) -> Result<(Vec<Turn>, Option<String>)> {
    let mut turns: Vec<Turn> = serde_json::from_value(response["data"].take())?;
    turns.reverse();
    let cursor = response["nextCursor"].as_str().map(str::to_owned);
    Ok((turns, cursor))
}

/// The turns holding a `thread/searchOccurrences` match, each once.
pub(crate) async fn search_turns(bridge: &Bridge, id: &str, query: &str) -> Result<Vec<String>> {
    let params = json!({"threadId": id, "searchTerm": query, "limit": 250});
    let response: Value = bridge.request("thread/searchOccurrences", params).await?;
    let mut turns: Vec<String> = Vec::new();
    for turn in response["data"].as_array().into_iter().flatten() {
        if let Some(turn) = turn["turnId"].as_str()
            && !turns.iter().any(|seen| seen == turn)
        {
            turns.push(turn.to_owned());
        }
    }
    Ok(turns)
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
    let permission = if let Some(profile) = response["activePermissionProfile"]["id"]
        .as_str()
        .filter(|profile| !profile.starts_with(':'))
    {
        Permission::Named(profile.to_owned().into())
    } else if sandbox == "workspace-write" && !standard_workspace {
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

/// Runs `f` on the selected browser tab; without one the key goes on to other handlers.
fn on_active_browser(
    root: &mut Root,
    cx: &mut Context<Root>,
    f: impl FnOnce(&mut ui::browser::BrowserTab, &mut Context<ui::browser::BrowserTab>),
) {
    match root.active_browser_tab() {
        Some(tab) => tab.update(cx, f),
        None => cx.propagate(),
    }
}

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &FindInThread, window, cx| {
        find_in_workspace(this, window, cx);
    }))
    .on_action(
        cx.listener(|this, open: &ui::browser::OpenBrowserTab, window, cx| {
            let url = open.0.as_deref().and_then(|url| url::Url::parse(url).ok());
            this.open_browser_tab(url, window, cx);
        }),
    )
    .on_action(cx.listener(
        |this, a: &ui::screens::thread::view::RunEnvironmentAction, window, cx| {
            this.run_environment_action(a.0, window, cx);
        },
    ))
    .on_action(cx.listener(|this, _: &ui::browser::BrowserBack, _, cx| {
        on_active_browser(this, cx, |tab, cx| tab.go_back(cx));
    }))
    .on_action(cx.listener(|this, _: &ui::browser::BrowserForward, _, cx| {
        on_active_browser(this, cx, |tab, cx| tab.go_forward(cx));
    }))
    .on_action(
        cx.listener(|this, _: &ui::browser::ReloadBrowserPage, _, cx| {
            on_active_browser(this, cx, |tab, cx| tab.reload(false, cx));
        }),
    )
    .on_action(
        cx.listener(|this, _: &ui::browser::HardReloadBrowserPage, _, cx| {
            on_active_browser(this, cx, |tab, cx| tab.reload(true, cx));
        }),
    )
    .on_action(cx.listener(
        |this, _: &ui::browser::FocusBrowserAddressBar, window, cx| match this.active_browser_tab()
        {
            Some(tab) => tab.update(cx, |tab, cx| tab.focus_address(window, cx)),
            None => cx.propagate(),
        },
    ))
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
        // The control that opened the panel keeps its focus, as in the original; the route
        // refresh would move it to the composer.
        let focused = window.focused(cx);
        this.set_side(SidePanel::Changes, window, cx);
        restore_focus(focused, window, cx);
    }))
    .on_action(cx.listener(|this, action: &ui::ViewChanges, window, cx| {
        let focused = window.focused(cx);
        if let Some(panel) = &this.diff {
            panel.update(cx, |panel, cx| panel.view_changes(&action.0, cx));
        }
        this.set_side(SidePanel::Changes, window, cx);
        restore_focus(focused, window, cx);
    }))
    .on_action(cx.listener(Root::open_file))
    .on_action(cx.listener(
        |this, action: &ui::screens::thread::view::OpenImage, window, cx| {
            this.image = Some(cx.new(|_| ui::screens::thread::view::ImageView {
                image: action.image.clone(),
            }));
            if this.side_visible(SidePanel::Image) {
                this.sync(window, cx);
            } else {
                this.set_side(SidePanel::Image, window, cx);
            }
        },
    ))
    .on_action(cx.listener(|this, _: &OpenBackgroundTerminal, window, cx| {
        this.set_side(SidePanel::Terminal, window, cx)
    }))
    .on_action(cx.listener(|this, _: &ui::OpenSourcesPanel, window, cx| {
        this.set_side(SidePanel::Sources, window, cx)
    }))
    .on_action(cx.listener(|this, _: &ui::OpenPlanPanel, window, cx| {
        let Some(thread) = this.thread.clone() else {
            return;
        };
        let Some((key, text)) = ui::screens::plan::latest_plan(thread.read(cx).turns())
            .map(|(key, text)| (key.to_owned(), text.to_owned()))
        else {
            return;
        };
        if this
            .plan
            .as_ref()
            .is_none_or(|plan| plan.read(cx).key() != key)
        {
            let plan =
                cx.new(|cx| ui::screens::plan::PlanView::new(&thread, &key, &text, window, cx));
            this.plan = Some(plan);
        }
        this.set_side(SidePanel::Plan, window, cx)
    }))
    .on_action(cx.listener(|this, _: &ui::OpenSubagentsPanel, window, cx| {
        let Some(thread) = this.thread.clone() else {
            return;
        };
        if this.subagents.is_none() {
            this.subagents =
                Some(cx.new(|cx| ui::screens::subagents::SubagentsView::new(thread, cx)));
        }
        this.set_side(SidePanel::Subagents, window, cx)
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
    .on_action(cx.listener(
        |this, a: &ui::screens::thread::view::SubmitRating, window, cx| {
            this.submit_rating(a, window, cx)
        },
    ))
    .on_action(cx.listener(|this, edit: &EditMessage, window, cx| {
        this.edit_message(edit.clone(), window, cx)
    }))
    .on_action(cx.listener(|this, a: &RevertTurn, window, cx| this.revert(a.0.clone(), window, cx)))
    .on_action(cx.listener(
        |this, a: &ui::screens::thread::view::OpenHookStats, window, cx| {
            let dialog = cx.new(|cx| Dialog::hook_stats(a.0.clone(), cx));
            this.show_dialog(dialog, window, cx);
        },
    ))
    .on_action(cx.listener(
        |this, action: &ui::screens::thread::view::PatchTurn, _, cx| {
            this.patch_turn(action, cx);
        },
    ))
    .on_action(cx.listener(|this, a: &ThreadPressed, _, cx| {
        this.sidebar
            .update(cx, |sidebar, cx| sidebar.set_read(&a.0, cx));
    }))
    .on_action(cx.listener(|this, a: &GoToChat, window, cx| this.go_to_chat(Some(a.0), window, cx)))
}

/// Gives `focused` its focus back once the route refresh, which moves it to the composer, has run.
fn restore_focus(focused: Option<gpui::FocusHandle>, window: &mut Window, cx: &mut Context<Root>) {
    if let Some(focused) = focused {
        window.defer(cx, move |window, cx| window.focus(&focused, cx));
    }
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
    fn resumed_named_profile_preserves_its_identity() {
        let mut response = response();
        response["activePermissionProfile"] = json!({"id": "research", "extends": ":read-only"});
        response["sandbox"]["writableRoots"] = json!(["/extra"]);
        response["approvalPolicy"] = json!({"granular": {"rules": false}});
        assert_eq!(
            resumed_settings(response, "thread-a").unwrap().permission,
            Permission::Named("research".into()),
        );
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

#[cfg(test)]
mod page_tests {
    use super::*;

    #[test]
    fn turn_pages_are_reversed_to_oldest_first() {
        let turn = |id: &str| {
            json!({"id": id, "items": [], "status": "completed", "error": null,
                "startedAt": null, "completedAt": null, "durationMs": null})
        };
        let page = json!({"data": [turn("t3"), turn("t2")], "nextCursor": "c1"});
        let (turns, cursor) = parse_turns_page(page).unwrap();
        assert_eq!(
            turns.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["t2", "t3"]
        );
        assert_eq!(cursor.as_deref(), Some("c1"));
        let (_, cursor) = parse_turns_page(json!({"data": [], "nextCursor": null})).unwrap();
        assert_eq!(cursor, None);
    }
}
