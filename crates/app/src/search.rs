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
    let mut hits: Vec<CloudSearchHit> =
        Vec::with_capacity(response.items.len().min(SEARCH_LIMIT as usize));
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
        let snippet = if content_match { item.snippet } else { None };
        if let Some(hit) = hits.iter_mut().find(|hit| hit.url == url.as_str()) {
            if hit.snippet.is_none() {
                hit.snippet = snippet.map(clean_snippet);
            }
            continue;
        }
        hits.push(CloudSearchHit {
            url: url.into(),
            title: if item.title.trim().is_empty() {
                "Untitled chat".into()
            } else {
                trimmed(item.title)
            },
            snippet: snippet.map(clean_snippet),
            updated_at: item.update_time.unwrap_or_default() as i64,
            source,
        });
    }
    Ok(hits)
}

fn trimmed(mut text: String) -> String {
    text.truncate(text.trim_end().len());
    let start = text.len() - text.trim_start().len();
    text.drain(..start);
    text
}

// The original recents module removes citation tokens and unwraps sandbox links, not general Markdown.
fn clean_snippet(mut text: String) -> String {
    let mut from = 0;
    let mut clean = None;
    while let Some(offset) = text[from..].find('\u{e200}') {
        let start = from + offset;
        let Some(end) = text[start..].find('\u{e201}') else {
            break;
        };
        let out = clean.get_or_insert_with(|| String::with_capacity(text.len()));
        out.push_str(&text[from..start]);
        from = start + end + '\u{e201}'.len_utf8();
    }
    if let Some(mut out) = clean {
        out.push_str(&text[from..]);
        text = out;
    }
    if let Some(start) = text.rfind('\u{e200}')
        && (text[start..].starts_with("\u{e200}filecite\u{e202}")
            || text[start..].starts_with("\u{e200}cite\u{e202}"))
    {
        text.truncate(start);
    }

    let mut links = Vec::new();
    let mut output = None;
    let (mut at, mut from) = (0, 0);
    let has_sandbox_target = text.contains("(sandbox:") || text.contains("(<sandbox:");
    while has_sandbox_target && at < text.len() {
        let character = text[at..].chars().next().unwrap();
        let next = at + character.len_utf8();
        match character {
            '\\' => {
                at = next + text[next..].chars().next().map_or(0, char::len_utf8);
                continue;
            }
            '\n' | '\r' => links.clear(),
            '[' => links.push(at),
            ']' => {
                if let Some(start) = links.pop() {
                    let prefix = if text[next..].starts_with("(<sandbox:") {
                        Some(("(<sandbox:", true))
                    } else if text[next..].starts_with("(sandbox:") {
                        Some(("(sandbox:", false))
                    } else {
                        None
                    };
                    if let Some((prefix, angled)) = prefix {
                        let end = sandbox_link_end(&text, next + prefix.len(), angled);
                        let out = output.get_or_insert_with(|| String::with_capacity(text.len()));
                        let start_image = start > 0 && text.as_bytes()[start - 1] == b'!';
                        out.push_str(&text[from..start - usize::from(start_image)]);
                        out.push_str(&text[start + 1..at]);
                        from = end;
                        at = end;
                        links.clear();
                        continue;
                    }
                }
            }
            _ => {}
        }
        at = next;
    }
    if let Some(mut out) = output {
        out.push_str(&text[from..]);
        text = out;
    }
    text = trimmed(text);
    let space = |byte: u8| matches!(byte, b' ' | b'\t');
    if !text
        .as_bytes()
        .windows(2)
        .any(|pair| space(pair[0]) && space(pair[1]))
    {
        return text;
    }
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if matches!(character, ' ' | '\t')
            && characters
                .peek()
                .is_some_and(|next| matches!(*next, ' ' | '\t'))
        {
            output.push(' ');
            while characters
                .peek()
                .is_some_and(|next| matches!(*next, ' ' | '\t'))
            {
                characters.next();
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn sandbox_link_end(text: &str, start: usize, angled: bool) -> usize {
    if angled {
        let end = text[start..]
            .find(['>', '\n', '\r'])
            .map_or(text.len(), |offset| start + offset);
        let end = end + usize::from(text.as_bytes().get(end) == Some(&b'>'));
        return end + usize::from(text.as_bytes().get(end) == Some(&b')'));
    }
    let (mut depth, mut escaped) = (0, false);
    for (offset, character) in text[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '(' => depth += 1,
            ')' if depth == 0 => return start + offset + 1,
            ')' => depth -= 1,
            _ => {}
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_success_status_without_a_search_payload() {
        assert!(results(json!({})).is_err());
    }

    #[test]
    fn snippets_keep_labels_without_citations_or_sandbox_targets() {
        assert_eq!(
            clean_snippet(
                " See [report](sandbox:/report.csv) \u{e200}cite\u{e202}turn1\u{e201}  now ".into()
            ),
            "See report now"
        );
        assert_eq!(
            clean_snippet("[diagram](<sandbox:/图.png>)\u{e200}filecite\u{e202}partial".into()),
            "diagram"
        );
    }

    #[test]
    fn content_matches_fill_an_existing_destination_without_changing_its_target() -> Result<()> {
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
        let mut title_match = item.clone();
        title_match["snippet"] = Value::Null;
        title_match["match_kind"] = json!("title");
        let hits = results(json!({"items": [title_match, item]}))?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet.as_deref(), Some("a matching message"));
        assert_eq!(
            hits[0].url,
            "https://chatgpt.com/dots/dot%2Fid?roomId=room%26other&messageId=message%231"
        );
        Ok(())
    }
}
