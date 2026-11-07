//! Server notifications, decoded once.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::{Event, types::*};

/// `thread/status/changed`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStatusChanged {
    pub thread_id: String,
    pub status: Value,
}

/// A decoded server notification. Methods without a typed variant stay in [`Other`].
///
/// [`Other`]: ServerNotification::Other
#[derive(Debug, Clone)]
pub enum ServerNotification {
    ThreadStarted(ThreadStartedNotification),
    ThreadStatusChanged(ThreadStatusChanged),
    TurnStarted(TurnNotification),
    TurnCompleted(TurnNotification),
    TurnDiffUpdated(TurnDiffUpdatedNotification),
    ItemStarted(ItemNotification),
    ItemCompleted(ItemNotification),
    AgentMessageDelta(AgentMessageDeltaNotification),
    Error(ErrorNotification),
    Other { method: String, params: Value },
}

impl ServerNotification {
    /// Decodes `method`/`params`; a known method with a malformed payload is an error.
    pub fn decode(method: &str, params: &Value) -> Result<Self> {
        fn parse<T: serde::de::DeserializeOwned>(method: &str, params: &Value) -> Result<T> {
            T::deserialize(params).with_context(|| format!("decoding {method}"))
        }
        Ok(match method {
            "thread/started" => Self::ThreadStarted(parse(method, params)?),
            "thread/status/changed" => Self::ThreadStatusChanged(parse(method, params)?),
            "turn/started" => Self::TurnStarted(parse(method, params)?),
            "turn/completed" => Self::TurnCompleted(parse(method, params)?),
            "turn/diff/updated" => Self::TurnDiffUpdated(parse(method, params)?),
            "item/started" => Self::ItemStarted(parse(method, params)?),
            "item/completed" => Self::ItemCompleted(parse(method, params)?),
            "item/agentMessage/delta" => Self::AgentMessageDelta(parse(method, params)?),
            "error" => Self::Error(parse(method, params)?),
            _ => Self::Other {
                method: method.to_owned(),
                params: params.clone(),
            },
        })
    }

    /// `None` for requests and for [`Event::Closed`].
    pub fn from_event(event: &Event) -> Option<Result<Self>> {
        match event {
            Event::Notification { method, params } => Some(Self::decode(method, params)),
            _ => None,
        }
    }
}
