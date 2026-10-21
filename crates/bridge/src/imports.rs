use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SYNC_ENABLED: &str = "external-agent-import-sync-enabled";
pub const SYNC_ITEM_TYPES: &str = "external-agent-import-sync-item-types";
pub const SYNC_STATE: &str = "external-agent-import-sync-state";
pub const SYNC_INTERVAL_MS: i64 = 43_200_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub enabled: bool,
    pub sync: bool,
    pub cursor: bool,
}

impl Policy {
    pub fn from_evaluations(evaluations: &Value) -> Self {
        let enabled = |key: &str| evaluations["feature_gates"][key]["value"] == true;
        Self {
            enabled: enabled("3278809559") && !enabled("2373428540"),
            sync: enabled("2108304519"),
            cursor: enabled("3203613120"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Provider {
    #[serde(rename = "claude-code")]
    ClaudeCode,
    #[serde(rename = "claude-cowork")]
    ClaudeCowork,
    #[serde(rename = "cursor")]
    Cursor,
}

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::ClaudeCowork => "claude-cowork",
            Self::Cursor => "cursor",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeCowork => "Claude Cowork",
            Self::Cursor => "Cursor",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ItemType {
    Config,
    AgentsMd,
    Skills,
    Commands,
    Subagents,
    Hooks,
    Plugins,
    McpServerConfig,
    Memory,
    Sessions,
}

impl ItemType {
    pub const ALL: [Self; 10] = [
        Self::Sessions,
        Self::AgentsMd,
        Self::Config,
        Self::Skills,
        Self::Plugins,
        Self::McpServerConfig,
        Self::Subagents,
        Self::Hooks,
        Self::Commands,
        Self::Memory,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS_MD",
            Self::Config => "CONFIG",
            Self::Skills => "SKILLS",
            Self::Commands => "COMMANDS",
            Self::Subagents => "SUBAGENTS",
            Self::Hooks => "HOOKS",
            Self::Plugins => "PLUGINS",
            Self::McpServerConfig => "MCP_SERVER_CONFIG",
            Self::Memory => "MEMORY",
            Self::Sessions => "SESSIONS",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AgentsMd => "Instructions",
            Self::Config => "Settings",
            Self::Skills => "Skills",
            Self::Commands => "Commands",
            Self::Subagents => "Agents",
            Self::Hooks => "Hooks",
            Self::Plugins => "Plugins",
            Self::McpServerConfig => "MCP servers",
            Self::Memory => "Memory",
            Self::Sessions => "Chats",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Sessions => "new-chat",
            Self::AgentsMd | Self::Memory => "settings-import",
            Self::Config | Self::McpServerConfig => "settings-configuration",
            Self::Hooks => "settings-hooks",
            Self::Skills | Self::Commands => "settings-code-review",
            Self::Subagents => "settings-profile",
            Self::Plugins => "settings-plugins",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationItem {
    pub item_type: ItemType,
    pub description: String,
    pub cwd: Option<String>,
    pub details: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedItem {
    #[serde(flatten)]
    pub item: MigrationItem,
    pub provider_id: Provider,
}

impl DetectedItem {
    pub fn project(&self) -> Option<&str> {
        self.item.cwd.as_deref().filter(|cwd| !cwd.is_empty())
    }

    pub fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.item.item_type.id(),
            self.provider_id.id(),
            self.item.cwd.as_deref().unwrap_or("home"),
            self.item.description
        )
    }

    pub fn selected(&self, selection: &BTreeMap<String, bool>) -> bool {
        if self.item.item_type == ItemType::Sessions {
            return selection.get("chats").copied().unwrap_or(false);
        }
        if self.project().is_some() {
            return selection.get("projects").copied().unwrap_or(false)
                && selection
                    .get(&format!("projects:{}", self.key()))
                    .copied()
                    .unwrap_or(false);
        }
        selection.get(&self.key()).copied().unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Detection {
    pub items: Vec<MigrationItem>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub version: u8,
    pub provider_ids: Vec<Provider>,
    pub selection: BTreeMap<String, bool>,
    pub last_sync_started_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryPreference {
    Original,
    #[serde(untagged)]
    Enabled(bool),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "Value", into = "Value")]
pub enum Preferences {
    All,
    Selected(BTreeMap<ItemType, CategoryPreference>),
    #[default]
    Original,
}
impl TryFrom<Value> for Preferences {
    type Error = serde_json::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if value.is_null() {
            Ok(Self::Original)
        } else if value == "all" {
            Ok(Self::All)
        } else {
            serde_json::from_value(value).map(Self::Selected)
        }
    }
}

impl From<Preferences> for Value {
    fn from(value: Preferences) -> Self {
        match value {
            Preferences::All => Value::String("all".into()),
            Preferences::Selected(selected) => {
                serde_json::to_value(selected).expect("preference keys are strings")
            }
            Preferences::Original => Value::Null,
        }
    }
}
impl SyncState {
    pub fn connected(&self) -> bool {
        self.version == 1 && !self.provider_ids.is_empty()
    }

    pub fn due(&self, now_ms: i64) -> bool {
        self.connected()
            && self
                .last_sync_started_at_ms
                .is_some_and(|last| now_ms.saturating_sub(last) >= SYNC_INTERVAL_MS)
    }

    pub fn category_selected(&self, category: ItemType) -> bool {
        self.category_checked(category).unwrap_or(true)
    }

    pub fn category_checked(&self, category: ItemType) -> Option<bool> {
        if category == ItemType::Sessions {
            return Some(self.selection.get("chats").copied().unwrap_or(false));
        }
        let projects = self.selection.get("projects") == Some(&true);
        let mut on = projects
            && !self
                .selection
                .keys()
                .any(|key| key.starts_with("projects:"));
        let mut off = false;
        for (key, selected) in &self.selection {
            let relevant = self.provider_ids.iter().any(|provider| {
                key.starts_with(&format!("{}:{}:", category.id(), provider.id()))
                    || (projects
                        && key.starts_with(&format!(
                            "projects:{}:{}:",
                            category.id(),
                            provider.id()
                        )))
            });
            if relevant {
                on |= selected;
                off |= !selected;
            }
        }
        if on && off { None } else { Some(on) }
    }

    pub fn merge_import(&mut self, items: &[DetectedItem], now_ms: i64) {
        self.version = 1;
        self.last_sync_started_at_ms = Some(now_ms);
        for item in items {
            if !self.provider_ids.contains(&item.provider_id) {
                self.provider_ids.push(item.provider_id);
            }
            if item.item.item_type == ItemType::Sessions {
                self.selection.insert("chats".into(), true);
            } else if item.project().is_some() {
                self.selection.insert("projects".into(), true);
                self.selection
                    .insert(format!("projects:{}", item.key()), true);
            } else {
                self.selection.insert(item.key(), true);
            }
        }
    }
}

impl Preferences {
    pub fn category_selected(&self, category: ItemType, state: &SyncState) -> bool {
        self.category_checked(category, state).unwrap_or(true)
    }

    pub fn category_checked(&self, category: ItemType, state: &SyncState) -> Option<bool> {
        match self {
            Self::All => Some(true),
            Self::Original => state.category_checked(category),
            Self::Selected(selected) => match selected.get(&category) {
                Some(CategoryPreference::Enabled(on)) => Some(*on),
                Some(CategoryPreference::Original) => state.category_checked(category),
                None => Some(false),
            },
        }
    }

    pub fn allows(&self, item: &DetectedItem, state: &SyncState) -> bool {
        state.provider_ids.contains(&item.provider_id)
            && match self {
                Self::All => true,
                Self::Original => item.selected(&state.selection),
                Self::Selected(selected) => match selected.get(&item.item.item_type) {
                    Some(CategoryPreference::Enabled(on)) => *on,
                    Some(CategoryPreference::Original) => item.selected(&state.selection),
                    None => false,
                },
            }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub item_type: ItemType,
    pub source: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub import_id: String,
    pub provider_id: Option<Provider>,
    pub completed_at_ms: i64,
    pub successes: Vec<HistoryItem>,
    pub failures: Vec<HistoryItem>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Histories {
    pub data: Vec<History>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(provider: Provider, category: ItemType, cwd: Option<&str>, name: &str) -> DetectedItem {
        DetectedItem {
            provider_id: provider,
            item: MigrationItem {
                item_type: category,
                description: name.into(),
                cwd: cwd.map(str::to_owned),
                details: None,
            },
        }
    }

    #[test]
    fn sync_preserves_original_selection_and_limits_future_categories() {
        let original = item(Provider::ClaudeCode, ItemType::Skills, None, "first");
        let project = item(
            Provider::ClaudeCode,
            ItemType::AgentsMd,
            Some("/project"),
            "instructions",
        );
        let future = item(Provider::ClaudeCode, ItemType::Skills, None, "new");
        let foreign = item(Provider::Cursor, ItemType::Skills, None, "new");
        let mut state = SyncState::default();
        state.merge_import(&[original.clone(), project.clone()], 1_000);
        assert!(Preferences::Original.allows(&original, &state));
        assert!(Preferences::Original.allows(&project, &state));
        assert!(!Preferences::Original.allows(&future, &state));
        let selected = Preferences::Selected(BTreeMap::from([(
            ItemType::Skills,
            CategoryPreference::Enabled(true),
        )]));
        assert!(selected.allows(&future, &state));
        assert!(!selected.allows(&foreign, &state));
        assert!(!state.due(1_000 + SYNC_INTERVAL_MS - 1));
        assert!(state.due(1_000 + SYNC_INTERVAL_MS));
        assert!(!state.due(999));
    }

    #[test]
    fn policy_defaults_closed_and_requires_the_original_gate_values() {
        assert!(!Policy::from_evaluations(&Value::Null).enabled);
        let policy = Policy::from_evaluations(&json!({"feature_gates":{
            "3278809559":{"value":true}, "2108304519":{"value":false}, "3203613120":{"value":true}
        }}));
        assert!(policy.enabled && policy.cursor && !policy.sync);
    }

    #[test]
    fn original_preferences_round_trip_without_enabling_absent_categories() {
        for value in [
            json!(null),
            json!("all"),
            json!({"SESSIONS":true,"HOOKS":false,"SKILLS":"original"}),
        ] {
            let preferences: Preferences = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(preferences).unwrap(), value);
        }
    }
}
