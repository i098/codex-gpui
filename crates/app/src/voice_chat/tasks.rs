use super::Root;
use gpui::{BorrowAppContext as _, Context, EntityId, Global, SharedString};
use serde_json::Value;
use std::time::{Duration, Instant};

const LIFETIME: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Created,
    Sent,
    Running,
    Completed,
    Failed,
    Interrupted,
    Stopped,
    Missing,
    Update,
}

impl Kind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Created => "Created task",
            Self::Sent => "Message sent",
            Self::Completed => "Task returned",
            _ => "Task update",
        }
    }

    fn body(self) -> &'static str {
        match self {
            Self::Created => "Working on delegated request",
            Self::Running => "Task is still working",
            Self::Completed => "Task finished",
            Self::Failed => "Task failed",
            Self::Interrupted => "Task was interrupted",
            Self::Stopped => "Task stopped",
            Self::Missing => "Task is no longer available",
            Self::Sent | Self::Update => "Task sent an update",
        }
    }
}

struct Update<'a> {
    item: &'a str,
    target: &'a str,
    kind: Kind,
    message: Option<&'a str>,
}

fn updates<'a>(active_thread: &str, params: &'a Value, mut emit: impl FnMut(Update<'a>)) {
    let item = &params["item"];
    let Some(item_id) = item["id"].as_str() else {
        return;
    };
    let mut emit = |target, kind, message| {
        emit(Update {
            item: item_id,
            target,
            kind,
            message,
        });
    };
    if item["type"] == "subAgentActivity" && params["threadId"] == active_thread {
        let Some(target) = item["agentThreadId"].as_str() else {
            return;
        };
        let kind = match item["kind"].as_str() {
            Some("started") => Kind::Created,
            Some("interacted") => Kind::Running,
            Some("completed") => Kind::Completed,
            Some("interrupted") => Kind::Interrupted,
            _ => return,
        };
        emit(target, kind, None);
        return;
    }
    if item["type"] != "collabAgentToolCall" {
        return;
    }
    let (Some(sender), Some(receivers)) = (
        item["senderThreadId"].as_str(),
        item["receiverThreadIds"].as_array(),
    ) else {
        return;
    };
    let tool = item["tool"].as_str();
    let status = item["status"].as_str();
    let prompt = item["prompt"].as_str();
    if sender != active_thread {
        if tool == Some("sendInput")
            && status == Some("completed")
            && receivers
                .iter()
                .any(|id| id.as_str() == Some(active_thread))
        {
            emit(sender, Kind::Update, prompt);
        }
        return;
    }
    for target in receivers.iter().filter_map(Value::as_str) {
        let (kind, body) = match (tool, status) {
            (Some("spawnAgent"), Some("completed")) => (Kind::Created, prompt),
            (Some("sendInput"), Some("completed")) => (Kind::Sent, prompt),
            (Some("wait"), Some("completed" | "failed")) => {
                let state = &item["agentsStates"][target];
                let kind = match state["status"].as_str() {
                    Some("pendingInit" | "running") => Kind::Running,
                    Some("completed") => Kind::Completed,
                    Some("errored") => Kind::Failed,
                    Some("interrupted") => Kind::Interrupted,
                    Some("shutdown") => Kind::Stopped,
                    Some("notFound") => Kind::Missing,
                    _ if status == Some("failed") => Kind::Failed,
                    _ => Kind::Update,
                };
                (kind, state["message"].as_str())
            }
            _ => return,
        };
        emit(target, kind, body);
    }
}

pub(crate) struct Card {
    item: SharedString,
    pub(crate) target: SharedString,
    pub(crate) kind: Kind,
    pub(crate) body: SharedString,
    expires: Instant,
}

#[derive(Default)]
pub(crate) struct VoiceTasks {
    owner: Option<EntityId>,
    cards: Vec<Card>,
}

impl Global for VoiceTasks {}

impl VoiceTasks {
    pub(crate) fn cards(&self) -> impl DoubleEndedIterator<Item = &Card> {
        let now = Instant::now();
        self.cards.iter().filter(move |card| card.expires > now)
    }

    pub(super) fn start(cx: &mut Context<Root>) {
        let owner = cx.entity_id();
        if cx.try_global::<Self>().and_then(|state| state.owner) != Some(owner) {
            cx.set_global(Self {
                owner: Some(owner),
                cards: Vec::new(),
            });
        }
    }

    pub(super) fn stop(cx: &mut Context<Root>) {
        if cx.try_global::<Self>().and_then(|state| state.owner) == Some(cx.entity_id()) {
            cx.set_global(Self::default());
        }
    }

    fn push(&mut self, update: Update<'_>, now: Instant) -> bool {
        if self.cards.iter().any(|card| {
            card.expires > now
                && card.target.as_ref() == update.target
                && (card.item.as_ref() == update.item
                    || (card.kind == update.kind
                        && matches!(update.kind, Kind::Created | Kind::Running)))
        }) {
            return false;
        }
        self.cards.push(Card {
            item: update.item.to_owned().into(),
            target: update.target.to_owned().into(),
            kind: update.kind,
            body: update
                .message
                .map_or_else(|| update.kind.body().into(), |text| text.to_owned().into()),
            expires: now + LIFETIME,
        });
        true
    }

    pub(super) fn apply(thread: &str, params: &Value, cx: &mut Context<Root>) {
        if cx.try_global::<Self>().and_then(|state| state.owner) != Some(cx.entity_id()) {
            return;
        }
        let mut changed = false;
        let now = Instant::now();
        updates(thread, params, |update| {
            changed |= cx.update_global::<Self, _>(|state, _| state.push(update, now));
        });
        if changed {
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(LIFETIME).await;
                cx.update_global::<Self, _>(|state, _| {
                    let now = Instant::now();
                    state.cards.retain(|card| card.expires > now);
                });
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_one_live_creation_and_running_card_per_task() {
        let now = Instant::now();
        let mut state = VoiceTasks::default();
        for (item, kind, expected) in [
            ("spawn", Kind::Created, true),
            ("started", Kind::Created, false),
            ("wait-1", Kind::Running, true),
            ("wait-2", Kind::Running, false),
            ("reply", Kind::Completed, true),
            ("reply", Kind::Completed, false),
        ] {
            assert_eq!(
                state.push(
                    Update {
                        item,
                        target: "task",
                        kind,
                        message: None,
                    },
                    now,
                ),
                expected
            );
        }
    }

    #[test]
    fn maps_wait_results_per_receiver_only_for_the_voice_thread() {
        let value = json!({"threadId": "voice", "item": {
            "id": "wait", "type": "collabAgentToolCall", "tool": "wait",
            "status": "completed", "senderThreadId": "voice",
            "receiverThreadIds": ["one", "two", "three"], "agentsStates": {
                "one": {"status": "running"},
                "two": {"status": "completed", "message": "Result"},
                "three": {"status": "errored"}
            }
        }});
        let mut observed = Vec::new();
        updates("voice", &value, |u| {
            observed.push((
                u.target,
                u.kind.title(),
                u.message.unwrap_or_else(|| u.kind.body()),
            ));
        });
        updates("other", &value, |_| {
            panic!("unrelated voice thread received a card")
        });
        assert_eq!(
            observed,
            [
                ("one", "Task update", "Task is still working"),
                ("two", "Task returned", "Result"),
                ("three", "Task update", "Task failed"),
            ]
        );
    }
}
