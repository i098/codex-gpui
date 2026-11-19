//! A ChatGPT-mode home send: the original's `$cn` starts a ChatGPT conversation turn and
//! streams the reply into the conversation view, not a local Codex thread. The open
//! [`ui::ThreadView`] shows the turn from notifications built here.

use crate::{Root, chatgpt_conversation};
use anyhow::Context as _;
use bridge::Event;
use bridge::types::{Thread, ThreadStatus};
use gpui::{AppContext as _, Context, Window};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;
use ui::composer::Submission;

enum Update {
    Delta(String),
    Done(anyhow::Result<chatgpt_conversation::Turn>),
}

impl Root {
    /// A ChatGPT-mode home send: opens a new conversation view and streams its first turn.
    pub(crate) fn start_chatgpt_turn(
        &mut self,
        s: Submission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.host.identity().is_none() {
            return self.restore_start_draft(&s, cx);
        }
        let id = format!("chatgpt:{}", crate::uuid_v4());
        self.shortcut_state.ephemeral.insert(id.clone());
        let work = ui::screens::sidebar::MODES[self.sidebar.read(cx).mode()] == "Work";
        self.shortcut_state.chatgpt_turns.insert(
            id.clone(),
            chatgpt_conversation::Turn {
                work,
                ..Default::default()
            },
        );
        self.show_thread(thread(&id, &s.text), window, cx);
        self.chatgpt_turn(id, s, cx);
    }

    /// One turn in the open ChatGPT conversation `id`, a follow-up to its last finished turn.
    pub(crate) fn chatgpt_turn(&mut self, id: String, s: Submission, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let Some(identity) = host.identity() else {
            return;
        };
        let previous = self
            .shortcut_state
            .chatgpt_turns
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let turn = format!("{id}:{}", crate::uuid_v4());
        let reply = format!("{turn}:reply");
        self.apply_chatgpt(
            &id,
            "turn/started",
            json!({"turn": turn_json(&turn, "inProgress", None)}),
            cx,
        );
        for (method, item) in [
            (
                "item/completed",
                json!({"type": "userMessage", "id": format!("{turn}:user"), "content": [{"type": "text", "text": s.text}]}),
            ),
            (
                "item/started",
                json!({"type": "agentMessage", "id": reply, "text": "", "phase": null}),
            ),
        ] {
            self.apply_chatgpt(&id, method, json!({"turnId": turn, "item": item}), cx);
        }
        let (send, receive) = async_channel::unbounded();
        let prompt = s.text.clone();
        cx.background_spawn(async move {
            let mut shown = 0;
            // shortcut: nothing cancels a turn yet; it ends with the stream or its 600 s budget.
            let result = chatgpt_conversation::send(
                &*host,
                &identity,
                &prompt,
                &previous,
                &AtomicBool::new(false),
                &mut |turn| {
                    // A rewritten reply sends no delta; the final `item/completed` carries it.
                    if let Some(delta) = turn.reply.get(shown..).filter(|d| !d.is_empty()) {
                        shown = turn.reply.len();
                        let _ = send.send_blocking(Update::Delta(delta.to_owned()));
                    }
                },
            )
            .context("ChatGPT conversation");
            let _ = send.send_blocking(Update::Done(result));
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(update) = receive.recv().await {
                let finished = matches!(update, Update::Done(_));
                let open = this.update(cx, |this, cx| match update {
                    Update::Delta(delta) => this.apply_chatgpt(
                        &id,
                        "item/agentMessage/delta",
                        json!({"turnId": turn, "itemId": reply, "delta": delta}),
                        cx,
                    ),
                    Update::Done(result) => {
                        let (status, error) = match result {
                            Ok(done) => {
                                let item = json!({"type": "agentMessage", "id": reply, "text": done.reply, "phase": null});
                                this.apply_chatgpt(&id, "item/completed", json!({"turnId": turn, "item": item}), cx);
                                this.shortcut_state.chatgpt_turns.insert(id.clone(), done);
                                ("completed", None)
                            }
                            Err(error) => ("failed", Some(format!("{error:#}"))),
                        };
                        this.apply_chatgpt(&id, "turn/completed", json!({"turn": turn_json(&turn, status, error)}), cx);
                    }
                });
                if open.is_err() || finished {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_chatgpt(&mut self, id: &str, method: &str, mut params: Value, cx: &mut Context<Self>) {
        params["threadId"] = id.into();
        let event = Event::Notification {
            method: method.to_owned(),
            params,
        };
        if let Some(view) = &self.thread {
            view.update(cx, |view, cx| view.apply(&event, cx));
        }
    }
}

fn turn_json(id: &str, status: &str, error: Option<String>) -> Value {
    json!({
        "id": id,
        "items": [],
        "status": status,
        "error": error.map(|message| json!({"message": message})),
    })
}

fn thread(id: &str, prompt: &str) -> Thread {
    Thread {
        id: id.to_owned(),
        preview: prompt.to_owned(),
        name: Some("New chat".to_owned()),
        model: None,
        reasoning_effort: None,
        model_provider: String::new(),
        cwd: String::new(),
        path: None,
        created_at: 0,
        updated_at: 0,
        section: None,
        status: ThreadStatus::Idle,
        git_info: None,
        daybreak_enabled: None,
        forked_from_id: None,
        turns: Vec::new(),
        older_cursor: None,
    }
}
