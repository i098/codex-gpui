//! v2 app-server types at `rust-v0.162.0-alpha.17.2`, only the fields the UI reads.
//! Unknown fields are ignored; unknown enum tags land in `Other`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub user_agent: String,
    pub codex_home: String,
    pub platform_os: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub preview: String,
    pub name: Option<String>,
    pub model: Option<String>,
    pub model_provider: String,
    pub cwd: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: ThreadStatus,
    pub git_info: Option<Value>,
    /// Filled only by `thread/read` with `includeTurns`, `thread/resume` and `thread/fork`.
    #[serde(default)]
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadStatus {
    NotLoaded,
    Idle,
    SystemError,
    Active,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: String,
    pub items: Vec<ThreadItem>,
    pub status: TurnStatus,
    pub error: Option<TurnError>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnError {
    pub message: String,
    pub codex_error_info: Option<Value>,
    pub additional_details: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ThreadItem {
    UserMessage {
        id: String,
        content: Vec<UserInput>,
    },
    AgentMessage {
        id: String,
        text: String,
    },
    Plan {
        id: String,
        text: String,
    },
    Reasoning {
        id: String,
        summary: Vec<String>,
        content: Vec<String>,
    },
    CommandExecution {
        id: String,
        command: String,
        cwd: String,
        /// `inProgress` | `completed` | `failed` | `declined`
        status: String,
        aggregated_output: Option<String>,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
        /// The server's parse of `command`: `read` / `search` / `list_files` actions name the
        /// file (`Read math.py`); anything else is `unknown`.
        #[serde(default)]
        command_actions: Vec<CommandAction>,
    },
    FileChange {
        id: String,
        changes: Vec<FileUpdateChange>,
        /// `inProgress` | `completed` | `failed` | `declined`
        status: String,
    },
    McpToolCall {
        id: String,
        server: String,
        tool: String,
        status: String,
    },
    ContextCompaction {
        id: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserInput {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileUpdateChange {
    pub path: String,
    /// `{"type": "add" | "delete" | "update", "move_path"?}`
    pub kind: Value,
    pub diff: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandAction {
    /// `read` | `search` | `list_files` | `format` | `lint` | `noop` | `test` | `unknown`
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    /// File name of a `read`.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartResponse {
    pub thread: Thread,
    pub model: String,
    pub cwd: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TurnStartResponse {
    pub turn: Turn,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListResponse {
    pub data: Vec<Thread>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadReadResponse {
    pub thread: Thread,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListResponse {
    pub data: Vec<Model>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub default_reasoning_effort: String,
    pub supported_reasoning_efforts: Vec<Value>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigReadResponse {
    /// Effective `config.toml` as JSON (`model`, `model_reasoning_effort`, ...).
    pub config: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountReadResponse {
    pub account: Option<Account>,
    pub requires_openai_auth: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Account {
    ApiKey,
    Chatgpt {
        email: Option<String>,
        plan_type: String,
    },
    #[serde(other)]
    Other,
}

// Notifications, by method.

/// `thread/started`
#[derive(Debug, Clone, Deserialize)]
pub struct ThreadStartedNotification {
    pub thread: Thread,
}

/// `turn/started`, `turn/completed`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnNotification {
    pub thread_id: String,
    pub turn: Turn,
}

/// `item/started`, `item/completed`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item: ThreadItem,
}

/// `item/agentMessage/delta`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

/// `turn/diff/updated`: the turn's aggregated unified diff.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnDiffUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub diff: String,
}

/// `error`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub error: TurnError,
    pub will_retry: bool,
}

// Server requests. Answer with `bridge.respond(id, json!({ "decision": ApprovalDecision::Accept }))`.

/// `item/commandExecution/requestApproval`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandApprovalRequest {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub reason: Option<String>,
}

/// `item/fileChange/requestApproval`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeApprovalRequest {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub reason: Option<String>,
    pub grant_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}
