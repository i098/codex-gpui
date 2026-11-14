//! One typed home for the app-server protocol: a function per request, one decoded enum per
//! server message, and the shared pagination loop. Method strings live only in this module.

mod account;
mod config;
mod fs;
mod notification;
mod request;
mod thread;

pub use config::{ConfigEdit, MergeStrategy};
pub use notification::{ServerNotification, ThreadStatusChanged};
pub use request::ServerRequest;
pub use thread::{ThreadListQuery, TurnStart};

use std::collections::HashSet;

use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{Bridge, types::*};

impl Bridge {
    /// Calls a list method until `nextCursor` is null and returns every item of `data`.
    /// `params` is an object; its `cursor` is replaced on each page.
    pub async fn paged<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<Vec<T>> {
        let mut params = params;
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        params["cursor"] = Value::Null;
        loop {
            let mut page: Value = self.request(method, &params).await?;
            let data = page["data"].take();
            let Value::Array(data) = data else {
                anyhow::bail!("{method}: paginated response has no `data` array");
            };
            for item in data {
                items.push(
                    serde_json::from_value(item)
                        .with_context(|| format!("decoding {method} item"))?,
                );
            }
            let Some(next) = page["nextCursor"].as_str() else {
                return Ok(items);
            };
            ensure!(
                seen.insert(next.to_owned()),
                "{method}: pagination did not advance"
            );
            params["cursor"] = next.into();
        }
    }

    /// `config/batchWrite` to the user config file, reloading it on success.
    pub async fn config_write(&self, edits: &[ConfigEdit]) -> Result<Value> {
        self.request("config/batchWrite", config::params(edits))
            .await
    }

    /// One page of `thread/list` for `query`.
    pub async fn thread_list_page(&self, query: &ThreadListQuery) -> Result<ThreadListResponse> {
        self.request("thread/list", query.params()).await
    }

    /// Every thread matching `query`, following `nextCursor`.
    pub async fn thread_list_all(&self, query: &ThreadListQuery) -> Result<Vec<Thread>> {
        self.paged("thread/list", query.params()).await
    }

    /// `turn/start` with the fields `turn` sets.
    pub async fn turn_start_with(&self, turn: &TurnStart) -> Result<TurnStartResponse> {
        self.request("turn/start", turn.params()).await
    }

    pub async fn thread_archive(&self, thread_id: &str) -> Result<()> {
        self.thread_call("thread/archive", thread_id).await
    }

    pub async fn thread_unarchive(&self, thread_id: &str) -> Result<()> {
        self.thread_call("thread/unarchive", thread_id).await
    }

    pub async fn thread_unsubscribe(&self, thread_id: &str) -> Result<()> {
        self.thread_call("thread/unsubscribe", thread_id).await
    }

    async fn thread_call(&self, method: &str, thread_id: &str) -> Result<()> {
        self.request::<_, Value>(method, serde_json::json!({ "threadId": thread_id }))
            .await?;
        Ok(())
    }

    /// Pages of `experimentalFeature/list`.
    pub async fn experimental_features(&self) -> Result<Vec<Value>> {
        self.paged("experimentalFeature/list", serde_json::json!({}))
            .await
    }

    /// `skills/list` for `cwds`; `force_reload` bypasses the server's cache.
    pub async fn skills_list(&self, cwds: &[&str], force_reload: bool) -> Result<Value> {
        let params = serde_json::json!({ "cwds": cwds, "forceReload": force_reload });
        self.request("skills/list", params).await
    }
}

#[cfg(test)]
mod tests;
