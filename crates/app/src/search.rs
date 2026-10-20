use crate::host::{Identity, Request};
use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use ui::screens::sidebar::{CloudSearchHit, SEARCH_LIMIT};
use url::Url;

pub(crate) fn request(term: &str, identity: &Identity, query_id: &str) -> Request {
    Request::Http(json!({
        "url": "/global/search",
        "method": "POST",
        "body": json!({
            "entrypoint": "global_search",
            "limit": SEARCH_LIMIT,
            "query": term,
            "source_requests": [{"type": "conversation"}, {"type": "dot_messages"}],
            "cursor": null,
            "query_id": query_id,
        }).to_string(),
        "headers": {
            "x-openai-web-frontend": "codex_webview",
            "x-oai-mcp-form-version": "2",
            "x-openai-codex-window-type": "electron",
            "OAI-Language": "en-US",
            "oai-did": identity.device_id,
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "returnErrorResponse": true,
    }))
}

#[derive(Deserialize)]
struct Response {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    title: String,
    snippet: Option<String>,
    match_kind: String,
    update_time: Option<f64>,
    payload: Value,
}

pub(crate) fn results(body: Value) -> Result<Vec<CloudSearchHit>> {
    let response: Response =
        serde_json::from_value(body).context("invalid ChatGPT search response")?;
    let mut hits = Vec::with_capacity(response.items.len().min(SEARCH_LIMIT as usize));
    let base = Url::parse("https://chatgpt.com/").expect("static ChatGPT URL");
    for item in response.items.into_iter().take(SEARCH_LIMIT as usize) {
        let mut url = base.clone();
        let content_match = item.match_kind == "content" || item.payload["kind"] == "dot_message";
        let source = if item.payload["kind"] == "dot_message" {
            let id = item.payload["thread_id"]
                .as_str()
                .context("dot result has no thread id")?;
            let room = item.payload["room_id"]
                .as_str()
                .context("dot result has no room id")?;
            let message = item.payload["message_id"]
                .as_str()
                .context("dot result has no message id")?;
            url.path_segments_mut().unwrap().extend(["dots", id]);
            url.query_pairs_mut()
                .append_pair("roomId", room)
                .append_pair("messageId", message);
            "Your dot"
        } else if let Some(id) = item.payload["conversation_id"]
            .as_str()
            .filter(|id| !id.is_empty())
        {
            url.path_segments_mut().unwrap().extend(["c", id]);
            "ChatGPT"
        } else {
            continue;
        };
        if hits
            .iter()
            .any(|hit: &CloudSearchHit| hit.url == url.as_str())
        {
            continue;
        }
        hits.push(CloudSearchHit {
            url: url.into(),
            title: if item.title.trim().is_empty() {
                "Untitled chat".into()
            } else {
                item.title
            },
            snippet: content_match.then_some(item.snippet).flatten(),
            updated_at: item.update_time.unwrap_or_default() as i64,
            source,
        });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_success_status_without_a_search_payload() {
        assert!(results(json!({})).is_err());
    }

    #[test]
    fn keeps_one_result_per_destination_and_encodes_dot_navigation() -> Result<()> {
        let item = json!({
            "title": "math",
            "snippet": "a matching message",
            "match_kind": "content",
            "payload": {
                "kind": "dot_message",
                "conversation_id": "must-not-replace-the-dot-route",
                "thread_id": "dot/id",
                "room_id": "room&other",
                "message_id": "message#1"
            }
        });
        let hits = results(json!({"items": [item.clone(), item]}))?;
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].url,
            "https://chatgpt.com/dots/dot%2Fid?roomId=room%26other&messageId=message%231"
        );
        Ok(())
    }
}
