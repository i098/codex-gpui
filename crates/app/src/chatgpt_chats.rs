//! The ChatGPT and Work sidebars' `Chats`: the original's `chatgpt-conversations` query, the
//! first page of `GET /conversations` through the same desktop fetch as the cloud search.

use crate::host::{Identity, Request};
use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use ui::screens::sidebar::ChatGptChat;
use url::Url;

/// The original's page size for the non-task conversation history (`mFr`).
const PAGE: usize = 20;

pub(crate) fn request(identity: &Identity) -> Request {
    Request::Http(json!({
        // `gPr` with the task (`tpp`) conversations excluded; byte-for-byte the original's URL.
        "url": format!(
            "/conversations?exclude_conversation_origin=tpp&expand=false&hide_snorlax=false\
             &is_archived=false&is_starred=false&limit={PAGE}&order=updated&offset=0"
        ),
        "method": "GET",
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
struct Page {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    id: Option<String>,
    title: Option<String>,
}

/// Rows in server order; items without a string id are dropped like the original's `FO` filter.
pub(crate) fn chats(body: Value) -> Result<Vec<ChatGptChat>> {
    let page: Page = serde_json::from_value(body).context("invalid ChatGPT conversations page")?;
    let base = Url::parse("https://chatgpt.com/").expect("static ChatGPT URL");
    Ok(page
        .items
        .into_iter()
        .filter_map(|item| {
            let id = item.id?;
            let mut url = base.clone();
            url.path_segments_mut().unwrap().extend(["c", &id]);
            let title = item.title.unwrap_or_default();
            let title = title.trim();
            Some(ChatGptChat {
                id,
                title: if title.is_empty() { "New chat" } else { title }
                    .to_owned()
                    .into(),
                url: url.into(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_server_order_and_drop_items_without_ids() -> Result<()> {
        let rows = chats(json!({"items": [
            {"id": "b/1", "title": "  Second  "},
            {"title": "no id"},
            {"id": "a", "title": null},
        ], "total": 3, "limit": 20, "offset": 0}))?;
        let shown: Vec<_> = rows
            .iter()
            .map(|r| (r.title.as_ref(), r.url.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Second", "https://chatgpt.com/c/b%2F1"),
                ("New chat", "https://chatgpt.com/c/a"),
            ]
        );
        Ok(())
    }
}
