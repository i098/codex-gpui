//! Host side of the MCP App widget protocol: a widget's `tools/call` becomes
//! `mcpServer/tool/call`, and its event subscriptions become `mcpServer/event/stream/start` and
//! `stop`, with `mcpServer/event/stream/notification` routed back under the widget's own
//! subscription id (widget-e1674d393867.js, inventory 10.4).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::{Bridge, Event};

const SUBSCRIPTION_META: &str = "io.modelcontextprotocol/subscriptionId";
const TERMINATED: &str = "notifications/events/terminated";

pub struct WidgetHost {
    bridge: Arc<Bridge>,
    server: String,
    thread_id: String,
    /// Widget subscription id -> the id the server knows the stream by.
    streams: Mutex<HashMap<String, String>>,
}

impl WidgetHost {
    pub fn new(bridge: Arc<Bridge>, server: &str, thread_id: &str) -> Self {
        Self {
            bridge,
            server: server.to_owned(),
            thread_id: thread_id.to_owned(),
            streams: Mutex::default(),
        }
    }

    /// Answers one widget request (`tools/call`, or an event subscribe/unsubscribe) with its
    /// JSON-RPC `result`.
    pub async fn handle(&self, method: &str, params: &Value) -> Result<Value> {
        match method {
            "tools/call" => self.call_tool(params).await,
            "events/subscribe" => self.subscribe(params).await.map(|()| json!({})),
            "events/unsubscribe" => {
                self.unsubscribe(params["subscriptionId"].as_str().unwrap_or_default())
                    .await
            }
            other => bail!("unsupported widget method {other}"),
        }
    }

    async fn call_tool(&self, params: &Value) -> Result<Value> {
        let request = tool_call_params(&self.server, &self.thread_id, params);
        self.bridge.request("mcpServer/tool/call", request).await
    }

    async fn subscribe(&self, params: &Value) -> Result<()> {
        let widget_id = params["subscriptionId"].as_str().unwrap_or_default();
        if widget_id.is_empty() {
            bail!("missing subscription id");
        }
        let host_id = uuid::Uuid::new_v4().to_string();
        {
            let mut streams = self.streams.lock().unwrap();
            if streams.contains_key(widget_id) {
                bail!("MCP event subscription already exists");
            }
            streams.insert(widget_id.to_owned(), host_id.clone());
        }
        let request = stream_start_params(&self.server, &self.thread_id, &host_id, params);
        let started = self
            .bridge
            .request::<_, Value>("mcpServer/event/stream/start", request)
            .await;
        if let Err(error) = started {
            self.streams.lock().unwrap().remove(widget_id);
            return Err(error);
        }
        Ok(())
    }

    pub async fn unsubscribe(&self, widget_id: &str) -> Result<Value> {
        let host_id = self.streams.lock().unwrap().remove(widget_id);
        if let Some(host_id) = host_id {
            self.bridge
                .request::<_, Value>(
                    "mcpServer/event/stream/stop",
                    json!({ "subscriptionId": host_id }),
                )
                .await?;
        }
        Ok(json!({}))
    }

    /// The widget notification for a `mcpServer/event/stream/notification` of one of this
    /// host's streams; a terminated stream is forgotten.
    pub fn route(&self, event: &Event) -> Option<Value> {
        let Event::Notification { method, params } = event else {
            return None;
        };
        if method != "mcpServer/event/stream/notification" {
            return None;
        }
        let mut streams = self.streams.lock().unwrap();
        let (widget_id, notification) = route_notification(&streams, params)?;
        if notification["method"] == TERMINATED {
            streams.remove(&widget_id);
        }
        Some(notification)
    }

    /// Stops every open stream (the widget went away).
    pub async fn close(&self) {
        let ids: Vec<String> = self.streams.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let _ = self.unsubscribe(&id).await;
        }
    }
}

fn tool_call_params(server: &str, thread_id: &str, params: &Value) -> Value {
    let mut meta = params["_meta"].as_object().cloned().unwrap_or_default();
    meta.insert("thread_id".into(), thread_id.into());
    meta.insert("threadId".into(), thread_id.into());
    json!({
        "server": server,
        "tool": params["name"],
        "arguments": params.get("arguments").cloned().unwrap_or_else(|| json!({})),
        "threadId": thread_id,
        "_meta": meta,
    })
}

fn stream_start_params(server: &str, thread_id: &str, host_id: &str, params: &Value) -> Value {
    json!({
        "_meta": {"x-codex-turn-metadata": {"mcp_request_meta": {
            "link_id": params["linkId"],
            "selected_connector_ids": [params["connectorId"]],
        }}},
        "arguments": params.get("arguments").cloned().unwrap_or_else(|| json!({})),
        "name": params["name"],
        "server": server,
        "subscriptionId": host_id,
        "threadId": thread_id,
    })
}

fn route_notification(
    streams: &HashMap<String, String>,
    params: &Value,
) -> Option<(String, Value)> {
    let host_id = params["subscriptionId"].as_str()?;
    let (widget_id, _) = streams.iter().find(|(_, host)| host.as_str() == host_id)?;
    let mut notification = params["notification"].clone();
    if !notification.is_object() {
        return None;
    }
    let meta = &mut notification["params"]["_meta"];
    if !meta.is_object() {
        *meta = json!({});
    }
    meta[SUBSCRIPTION_META] = widget_id.as_str().into();
    Some((widget_id.clone(), notification))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_calls_carry_the_thread_in_both_meta_spellings() {
        let params = json!({"name": "search", "arguments": {"q": 1}, "_meta": {"k": "v"}});
        let call = tool_call_params("srv", "t1", &params);
        assert_eq!(call["tool"], "search");
        assert_eq!(call["threadId"], "t1");
        assert_eq!(
            call["_meta"],
            json!({"k": "v", "thread_id": "t1", "threadId": "t1"})
        );
    }

    #[test]
    fn stream_start_selects_the_widget_connector() {
        let params =
            json!({"name": "events", "linkId": "l", "connectorId": "c", "subscriptionId": "w"});
        let start = stream_start_params("srv", "t1", "h1", &params);
        assert_eq!(start["subscriptionId"], "h1");
        assert_eq!(
            start["_meta"]["x-codex-turn-metadata"]["mcp_request_meta"]["selected_connector_ids"],
            json!(["c"])
        );
        assert_eq!(start["arguments"], json!({}));
    }

    #[test]
    fn notifications_return_under_the_widget_subscription_id() {
        let streams = HashMap::from([("w1".to_owned(), "h1".to_owned())]);
        let params = json!({"subscriptionId": "h1", "notification": {"method": "notifications/x", "params": {"a": 1}}});
        let (id, note) = route_notification(&streams, &params).unwrap();
        assert_eq!(id, "w1");
        assert_eq!(note["params"]["_meta"][SUBSCRIPTION_META], "w1");
        assert_eq!(note["params"]["a"], 1);
        let other = json!({"subscriptionId": "h2", "notification": {"method": "m"}});
        assert!(route_notification(&streams, &other).is_none());
    }
}
