//! Context status (`thread/tokenUsage/updated`), the compaction lifecycle (`contextCompaction`
//! items) and streamed reasoning (`item/reasoning/*` deltas) of an open chat.

use super::*;
use crate::composer::ContextUsage;

/// The local thinking display, from `config.toml` `hide_agent_reasoning` and
/// `show_raw_agent_reasoning` (the Codex CLI's keys). It changes what this app draws, not what
/// the model does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReasoningVisibility {
    /// No reasoning rows at all.
    pub hide: bool,
    /// Raw reasoning content in place of the summary, where the model sends it.
    pub raw: bool,
}

impl gpui::Global for ReasoningVisibility {}

impl ReasoningVisibility {
    pub fn from_config(config: &Value) -> Self {
        Self {
            hide: config["hide_agent_reasoning"] == true,
            raw: config["show_raw_agent_reasoning"] == true,
        }
    }

    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    /// The text a reasoning item shows: its summary, else its raw content; with `raw`, the raw
    /// content first.
    pub(super) fn text(self, summary: &[String], content: &[String]) -> String {
        let (first, second) = if self.raw {
            (content, summary)
        } else {
            (summary, content)
        };
        if first.is_empty() { second } else { first }.join("\n\n")
    }
}

/// The original's `ZT`: a streaming summary without its leading `**Heading**`.
pub(super) fn without_heading(text: &str) -> &str {
    let text = text.trim_start();
    match text.strip_prefix("**") {
        Some(rest) => match rest.find("**") {
            Some(end) if !rest[..end].contains('\n') => rest[end + 2..].trim_start(),
            _ => "",
        },
        None => text,
    }
}

/// How long `Compacting context` runs before it adds `This can take a few minutes` (`ES`).
const COMPACTION_NOTICE: Duration = Duration::from_secs(10);

impl ThreadView {
    /// The latest `thread/tokenUsage/updated` of this chat.
    pub fn context_usage(&self) -> Option<ContextUsage> {
        self.context_usage
    }

    /// Marks the next compaction of this chat as the user's (`/compact`), so it completes as
    /// `Context compacted` rather than `Context automatically compacted`.
    pub fn expect_manual_compaction(&mut self) {
        self.manual_compactions += 1;
    }

    /// Undoes [`Self::expect_manual_compaction`] after `thread/compact/start` failed.
    pub fn forget_manual_compaction(&mut self) {
        self.manual_compactions = self.manual_compactions.saturating_sub(1);
    }

    /// Folds the notifications this module owns; `false` for any other method.
    pub(super) fn apply_context(&mut self, method: &str, params: &Value) -> bool {
        match method {
            "model/rerouted" => {
                let (Some(turn), Some(from), Some(to)) = (
                    params["turnId"].as_str(),
                    params["fromModel"].as_str(),
                    params["toModel"].as_str(),
                ) else {
                    return false;
                };
                self.notices
                    .entry(turn.to_owned())
                    .or_default()
                    .push(format!("Model changed from {from} to {to}."));
            }
            "model/safetyBuffering/updated" => {
                let Some(turn) = params["turnId"].as_str() else {
                    return false;
                };
                if params["showBufferingUi"] == true {
                    self.safety_buffering.insert(turn.to_owned());
                } else {
                    self.safety_buffering.remove(turn);
                }
            }
            "model/verification" => {
                let Some(turn) = params["turnId"].as_str() else {
                    return false;
                };
                let cyber = params["verifications"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|v| v == "trustedAccessForCyber"));
                let notice = "Some cybersecurity requests may still be limited.";
                let notices = self.notices.entry(turn.to_owned()).or_default();
                if cyber && !notices.iter().any(|n| n == notice) {
                    notices.push(notice.to_owned());
                }
            }
            "thread/tokenUsage/updated" => {
                self.context_usage = ContextUsage::from_token_usage(&params["tokenUsage"]);
            }
            "item/reasoning/summaryPartAdded"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/textDelta" => {
                let Some(id) = params["itemId"].as_str() else {
                    return false;
                };
                let Some(ThreadItem::Reasoning {
                    summary, content, ..
                }) = self.item_mut(id)
                else {
                    return false;
                };
                let (parts, index) = if method == "item/reasoning/textDelta" {
                    (content, &params["contentIndex"])
                } else {
                    (summary, &params["summaryIndex"])
                };
                let Some(index) = index.as_u64().map(|i| i as usize) else {
                    return false;
                };
                if parts.len() <= index {
                    parts.resize(index + 1, String::new());
                }
                parts[index].push_str(params["delta"].as_str().unwrap_or_default());
            }
            _ => return false,
        }
        true
    }

    /// Tracks a `contextCompaction` item between `item/started` and `item/completed`.
    pub(super) fn track_compaction(&mut self, method: &str, item: &ThreadItem, now: Instant) {
        let ThreadItem::ContextCompaction { id } = item else {
            return;
        };
        if method == "item/started" {
            self.compacting.insert(id.clone(), now);
            if self.manual_compactions > 0 {
                self.manual_compactions -= 1;
                self.manual_compacted.insert(id.clone());
            }
        } else {
            self.compacting.remove(id);
        }
    }

    /// The row of compaction item `id` in a turn that is `running` or not, as the original's
    /// `xS`: in progress, interrupted (its turn ended first), or compacted.
    pub(super) fn compaction_label(&self, id: &str, running: bool, now: Instant) -> &'static str {
        match self.compacting.get(id) {
            Some(start) if running => {
                if now.saturating_duration_since(*start) >= COMPACTION_NOTICE {
                    "Compacting context · This can take a few minutes"
                } else {
                    "Compacting context"
                }
            }
            Some(_) => "Compaction interrupted",
            None if self.manual_compacted.contains(id) => "Context compacted",
            None => "Context automatically compacted",
        }
    }

    /// Whether reasoning item `id` is streaming now.
    pub(super) fn thinking(&self, id: &str) -> bool {
        self.reasoning_since.contains_key(id)
    }

    /// The inline notices under turn `turn`: reroutes, verification, and while the turn runs
    /// the safety buffering line.
    pub(super) fn turn_notices(&self, turn: &bridge::types::Turn, cx: &Context<Self>) -> Vec<Div> {
        let buffering = (self.safety_buffering.contains(&turn.id)
            && turn.status == TurnStatus::InProgress)
            .then_some("Giving this request a little extra thought");
        let color = cx.theme().text_alpha_60;
        self.notices
            .get(&turn.id)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .chain(buffering)
            .map(|line| {
                div()
                    .mt(px(16.))
                    .child(text(line.to_owned(), 14., 21., 430., color))
            })
            .collect()
    }
}
