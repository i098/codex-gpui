//! `Subagents` side panel tab (`local-conversation-subagents-panel-tab`): the thread's
//! subagents from its `collabAgentToolCall` items, `Active` and `Done`, with a detail view.

use bridge::types::{ThreadItem, Turn};
use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme as _, Theme};

use crate::icon::icon;
use crate::screens::thread::view::ThreadView;
use crate::text;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Waiting,
    Working,
    Done,
    Failed,
}

impl Status {
    /// The original's `agentsStates` status mapping.
    fn from_state(state: &str) -> Option<Self> {
        Some(match state {
            "pendingInit" => Self::Waiting,
            "running" => Self::Working,
            "completed" | "interrupted" | "shutdown" => Self::Done,
            "errored" | "notFound" => Self::Failed,
            _ => return None,
        })
    }

    fn active(self) -> bool {
        matches!(self, Self::Waiting | Self::Working)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::Working => "Working",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subagent {
    pub thread_id: String,
    /// `Subagent N`, in spawn order (the original's `nameFallback`).
    pub name: String,
    pub status: Status,
    /// The `spawnAgent` prompt: `Delegated task`.
    pub prompt: Option<String>,
    /// The last state message of a finished subagent: `Final response`.
    pub message: Option<String>,
    pub model: Option<String>,
}

/// The thread's subagents, folded from its `collabAgentToolCall` items as the original does:
/// a spawn, send or resume marks the receivers working; `agentsStates` then sets each status,
/// and a finished subagent keeps its non-empty message.
pub fn subagents(turns: &[Turn]) -> Vec<Subagent> {
    let mut agents: Vec<Subagent> = Vec::new();
    for item in turns.iter().flat_map(|turn| &turn.items) {
        let ThreadItem::CollabAgentToolCall {
            tool,
            status,
            receiver_thread_ids,
            agents_states,
            prompt,
            model,
            ..
        } = item
        else {
            continue;
        };
        let starts = matches!(tool.as_str(), "spawnAgent" | "sendInput" | "resumeAgent");
        for id in receiver_thread_ids {
            let agent = entry(&mut agents, id);
            if tool == "spawnAgent" {
                agent.prompt = prompt.clone().filter(|p| !p.trim().is_empty());
                agent.model = model.clone();
            }
            if status != "failed" && starts {
                agent.status = Status::Working;
                agent.message = None;
            }
        }
        for (id, state) in agents_states {
            let Some(status) = Status::from_state(&state.status) else {
                continue;
            };
            let agent = entry(&mut agents, id);
            agent.status = status;
            if status.active() {
                agent.message = None;
            } else if let Some(message) = state.message.as_deref().filter(|m| !m.trim().is_empty())
            {
                agent.message = Some(message.to_owned());
            }
        }
    }
    agents
}

/// The subagent of thread `id`, added as `Subagent N` on first sight.
fn entry<'a>(agents: &'a mut Vec<Subagent>, id: &str) -> &'a mut Subagent {
    let ix = match agents.iter().position(|a| a.thread_id == id) {
        Some(ix) => ix,
        None => {
            agents.push(Subagent {
                thread_id: id.to_owned(),
                name: format!("Subagent {}", agents.len() + 1),
                status: Status::Waiting,
                prompt: None,
                message: None,
                model: None,
            });
            agents.len() - 1
        }
    };
    &mut agents[ix]
}

pub struct SubagentsView {
    thread: Entity<ThreadView>,
    /// The subagent whose detail shows, by thread id.
    selected: Option<String>,
}

impl SubagentsView {
    pub fn new(thread: Entity<ThreadView>, cx: &mut Context<Self>) -> Self {
        cx.observe(&thread, |_, _, cx| cx.notify()).detach();
        Self {
            thread,
            selected: None,
        }
    }

    fn section(title: String, t: &Theme) -> gpui::Div {
        div().px(px(10.)).pt(px(12.)).pb(px(4.)).child(text(
            title,
            13.,
            18.,
            500.,
            t.text_secondary,
        ))
    }

    fn row(
        &self,
        agent: &Subagent,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let id = agent.thread_id.clone();
        div()
            .id(SharedString::from(format!("subagent-{id}")))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(agent.name.clone()))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(t.list_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id.clone());
                cx.notify();
            }))
            .child(
                text(agent.name.clone(), 14., 20., 430., t.text)
                    .flex_1()
                    .truncate(),
            )
            .child(text(agent.status.label(), 12., 16., 430., t.text_tertiary))
    }

    fn detail(&self, agent: &Subagent, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let field = |label: &'static str, value: String, color| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .pt(px(12.))
                .child(text(label, 12., 16., 500., t.text_tertiary))
                .child(text(value, 14., 20., 430., color))
        };
        div()
            .flex()
            .flex_col()
            .px(px(10.))
            .child(
                div()
                    .id("subagents-back")
                    .role(gpui::Role::Button)
                    .aria_label("Back to subagents")
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected = None;
                        cx.notify();
                    }))
                    .child(icon("chevron-left", 14., t.text_secondary))
                    .child(text("Back to subagents", 13., 18., 430., t.text_secondary)),
            )
            .child(
                div()
                    .pt(px(8.))
                    .child(text(agent.name.clone(), 15., 20., 500., t.text)),
            )
            .when_some(agent.model.clone(), |d, model| {
                d.child(text(model, 12., 16., 430., t.text_tertiary))
            })
            .when_some(agent.prompt.clone(), |d, prompt| {
                d.child(field("Delegated task", prompt, t.text))
            })
            .child(field("Status", agent.status.label().into(), t.text))
            .when(!agent.status.active(), |d| {
                let (response, color) = match &agent.message {
                    Some(message) => (message.clone(), t.text),
                    None => ("No final response is available.".into(), t.text_tertiary),
                };
                d.child(field("Final response", response, color))
            })
    }
}

impl Render for SubagentsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let agents = subagents(self.thread.read(cx).turns());
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| agents.iter().find(|a| &a.thread_id == id));
        let body = div().flex().flex_col();
        let body = if let Some(agent) = selected {
            body.child(self.detail(agent, &t, cx))
        } else {
            let (active, done): (Vec<_>, Vec<_>) =
                agents.iter().partition(|agent| agent.status.active());
            let active_rows: Vec<_> = active.iter().map(|a| self.row(a, &t, cx)).collect();
            let done_rows: Vec<_> = done.iter().map(|a| self.row(a, &t, cx)).collect();
            body.child(Self::section(format!("Active · {}", active.len()), &t))
                .when(active.is_empty(), |d| {
                    d.child(div().px(px(10.)).child(text(
                        "No active subagents",
                        13.,
                        18.,
                        430.,
                        t.text_tertiary,
                    )))
                })
                .children(active_rows)
                .when(!done.is_empty(), |d| {
                    d.child(Self::section(format!("Done · {}", done.len()), &t))
                        .children(done_rows)
                })
        };
        div()
            .id("subagents-panel")
            .size_full()
            .overflow_y_scroll()
            .bg(t.surface)
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(576.))
                    .p(px(16.))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn(items: serde_json::Value) -> Turn {
        serde_json::from_value(json!({
            "id": "t", "items": items, "status": "completed", "error": null,
            "startedAt": null, "completedAt": null, "durationMs": null
        }))
        .unwrap()
    }

    #[test]
    fn spawn_then_states_fold_into_active_and_done() {
        let turns = [turn(json!([
            {"type": "collabAgentToolCall", "id": "c1", "tool": "spawnAgent",
             "status": "completed", "senderThreadId": "p", "receiverThreadIds": ["a", "b"],
             "prompt": "Write tests", "model": "gpt-5", "agentsStates": {}},
            {"type": "collabAgentToolCall", "id": "c2", "tool": "wait",
             "status": "completed", "senderThreadId": "p", "receiverThreadIds": ["a", "b"],
             "agentsStates": {
                "a": {"status": "completed", "message": "All green"},
                "b": {"status": "pendingInit", "message": "ignored"}
             }},
            {"type": "collabAgentToolCall", "id": "c3", "tool": "wait",
             "status": "completed", "senderThreadId": "p", "receiverThreadIds": [],
             "agentsStates": {"c": {"status": "notFound", "message": " "}}}
        ]))];
        let agents = subagents(&turns);
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].name, "Subagent 1");
        assert_eq!(agents[0].status, Status::Done);
        assert_eq!(agents[0].message.as_deref(), Some("All green"));
        assert_eq!(agents[0].prompt.as_deref(), Some("Write tests"));
        assert_eq!(agents[1].status, Status::Waiting);
        assert_eq!(agents[1].message, None);
        assert_eq!(agents[2].status, Status::Failed);
        assert_eq!(
            agents[2].message, None,
            "a blank message is no final response"
        );
    }

    #[test]
    fn a_failed_send_keeps_the_previous_status() {
        let turns = [turn(json!([
            {"type": "collabAgentToolCall", "id": "c1", "tool": "spawnAgent",
             "status": "completed", "senderThreadId": "p", "receiverThreadIds": ["a"],
             "agentsStates": {"a": {"status": "completed", "message": "Done it"}}},
            {"type": "collabAgentToolCall", "id": "c2", "tool": "sendInput",
             "status": "failed", "senderThreadId": "p", "receiverThreadIds": ["a"],
             "agentsStates": {}}
        ]))];
        let agents = subagents(&turns);
        assert_eq!(agents[0].status, Status::Done);
        assert_eq!(agents[0].message.as_deref(), Some("Done it"));
    }
}
