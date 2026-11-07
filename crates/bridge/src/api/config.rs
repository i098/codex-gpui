//! `config/batchWrite` edits.

use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeStrategy {
    Replace,
    Upsert,
}

/// One edit: set `key_path` (dotted) to `value`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEdit {
    pub key_path: String,
    pub value: Value,
    pub merge_strategy: MergeStrategy,
}

impl ConfigEdit {
    pub fn upsert(key_path: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            key_path: key_path.into(),
            value: value.into(),
            merge_strategy: MergeStrategy::Upsert,
        }
    }

    pub fn replace(key_path: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            merge_strategy: MergeStrategy::Replace,
            ..Self::upsert(key_path, value)
        }
    }
}

pub(super) fn params(edits: &[ConfigEdit]) -> Value {
    json!({
        "edits": edits,
        "filePath": null,
        "expectedVersion": null,
        "reloadUserConfig": true,
    })
}
