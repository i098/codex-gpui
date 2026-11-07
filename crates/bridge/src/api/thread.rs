//! Builders for `thread/list` and `turn/start`.

use serde_json::{Value, json};

/// Filters of `thread/list`; the defaults list live, non-archived threads by recency.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadListQuery {
    pub limit: Option<u32>,
    pub archived: bool,
    pub search_term: Option<String>,
}

impl Default for ThreadListQuery {
    fn default() -> Self {
        Self {
            limit: Some(50),
            archived: false,
            search_term: None,
        }
    }
}

impl ThreadListQuery {
    pub(super) fn params(&self) -> Value {
        let mut params = json!({
            "limit": self.limit,
            "cursor": null,
            "sortKey": "recency_at",
            "modelProviders": null,
            "archived": self.archived,
            "sourceKinds": [],
            "useStateDbOnly": true,
        });
        if let Some(term) = &self.search_term {
            params["searchTerm"] = term.as_str().into();
        }
        params
    }
}

/// `turn/start`: text input plus the optional per-turn overrides the UI sends.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnStart {
    pub thread_id: String,
    pub text: String,
    pub effort: Option<String>,
    pub service_tier: Option<String>,
    pub model: Option<String>,
}

impl TurnStart {
    pub fn new(thread_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            text: text.into(),
            effort: None,
            service_tier: None,
            model: None,
        }
    }

    pub(super) fn params(&self) -> Value {
        let mut params = json!({
            "threadId": self.thread_id,
            "input": [{ "type": "text", "text": self.text, "text_elements": [] }],
        });
        for (key, value) in [
            ("effort", &self.effort),
            ("serviceTier", &self.service_tier),
            ("model", &self.model),
        ] {
            if let Some(value) = value {
                params[key] = value.as_str().into();
            }
        }
        params
    }
}
