//! Requests the server sends to the client, decoded once. Answer with [`Bridge::respond`].
//!
//! [`Bridge::respond`]: crate::Bridge::respond

use anyhow::{Context, Result};
use serde_json::Value;

use crate::{Event, types::*};

#[derive(Debug, Clone)]
pub enum ServerRequest {
    CommandApproval {
        id: Value,
        request: CommandApprovalRequest,
    },
    FileChangeApproval {
        id: Value,
        request: FileChangeApprovalRequest,
    },
    PermissionsApproval {
        id: Value,
        params: Value,
    },
    UserInput {
        id: Value,
        params: Value,
    },
    McpElicitation {
        id: Value,
        params: Value,
    },
    Other {
        id: Value,
        method: String,
        params: Value,
    },
}

impl ServerRequest {
    pub fn decode(id: &Value, method: &str, params: &Value) -> Result<Self> {
        let id = id.clone();
        let parse_err = || format!("decoding {method}");
        Ok(match method {
            "item/commandExecution/requestApproval" => Self::CommandApproval {
                id,
                request: serde_json::from_value(params.clone()).with_context(parse_err)?,
            },
            "item/fileChange/requestApproval" => Self::FileChangeApproval {
                id,
                request: serde_json::from_value(params.clone()).with_context(parse_err)?,
            },
            "item/permissions/requestApproval" => Self::PermissionsApproval {
                id,
                params: params.clone(),
            },
            "item/tool/requestUserInput" => Self::UserInput {
                id,
                params: params.clone(),
            },
            "mcpServer/elicitation/request" => Self::McpElicitation {
                id,
                params: params.clone(),
            },
            _ => Self::Other {
                id,
                method: method.to_owned(),
                params: params.clone(),
            },
        })
    }

    /// `None` for notifications and for [`Event::Closed`].
    pub fn from_event(event: &Event) -> Option<Result<Self>> {
        match event {
            Event::Request { id, method, params } => Some(Self::decode(id, method, params)),
            _ => None,
        }
    }
}
