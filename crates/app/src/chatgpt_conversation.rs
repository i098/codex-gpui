//! A ChatGPT conversation turn as the original sends it from the ChatGPT homes (`$cn` in
//! `app-initial-6350d04672d0.js`): the sentinel chat requirements (`tln`, `x3t`, `S3t`),
//! `POST /f/conversation` with the body of `cli`/`sli`, and the `v1` delta stream it answers
//! with (`yOn`, `jOn`, `wOn` in `app-shared-8bd9d0399038.js`).

use crate::chatgpt;
use crate::host::{self, Host, Identity, Request};
use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The streamed state of one turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    pub conversation_id: Option<String>,
    /// The visible assistant message, the parent of a follow-up.
    pub message_id: Option<String>,
    /// The visible assistant reply so far.
    pub reply: String,
    /// The server sent `message_stream_complete`.
    pub complete: bool,
}

/// Sends `prompt` as a new conversation, or as a follow-up to the `previous` turn, and reports
/// every change of the turn to `on_update` until the stream ends or `cancelled` is set.
pub fn send(
    host: &dyn Host,
    identity: &Identity,
    prompt: &str,
    previous: Option<&Turn>,
    cancelled: &AtomicBool,
    on_update: &mut dyn FnMut(&Turn),
) -> Result<Turn> {
    let started = Instant::now();
    let requirements = host::json_body(&host.fetch(&prepare_request(identity, started))?)
        .context("ChatGPT chat requirements")?;
    let mut body = body(prompt, &host::device_id());
    if let Some(Turn {
        conversation_id: Some(conversation),
        message_id: Some(parent),
        ..
    }) = previous
    {
        body["conversation_id"] = conversation.as_str().into();
        body["parent_message_id"] = parent.as_str().into();
    }
    let request = conversation_request(identity, &sentinel_headers(&requirements, started)?, &body);
    let mut decoder = Decoder::default();
    let mut failure = None;
    host.stream(
        &request,
        cancelled,
        &mut |name, data| match decoder.event(name, data) {
            Ok(true) => {
                on_update(&decoder.turn);
                true
            }
            Ok(false) => true,
            Err(error) => {
                failure = Some(error);
                false
            }
        },
    )?;
    if let Some(error) = failure {
        return Err(error);
    }
    ensure!(
        decoder.turn.complete || cancelled.load(Ordering::Acquire),
        "The ChatGPT reply ended before it was complete"
    );
    Ok(decoder.turn)
}

fn prepare_request(identity: &Identity, started: Instant) -> Request {
    let mut config = config(started);
    config[3] = 1.into();
    config[9] = elapsed_ms(started).into();
    let p = format!("gAAAAAC{}", encode(&Value::Array(config)));
    chatgpt::web_request(
        identity,
        "POST",
        "/sentinel/chat-requirements/prepare",
        Some(json!({ "p": p }).to_string()),
    )
}

/// `S3t`: the chat requirements token (or its prepare token) and the proof of work.
fn sentinel_headers(requirements: &Value, started: Instant) -> Result<Vec<(&'static str, String)>> {
    ensure!(
        requirements.is_object(),
        "Chat requirements returned a non-object response."
    );
    ensure!(
        requirements["force_login"] != true,
        "ChatGPT asks to sign in again"
    );
    let mut headers = Vec::new();
    if let Some(token) = requirements["token"].as_str().filter(|t| !t.is_empty()) {
        headers.push(("OpenAI-Sentinel-Chat-Requirements-Token", token.to_owned()));
    } else if let Some(token) = requirements["prepare_token"]
        .as_str()
        .filter(|t| !t.is_empty())
    {
        headers.push((
            "OpenAI-Sentinel-Chat-Requirements-Prepare-Token",
            token.to_owned(),
        ));
    }
    let work = &requirements["proofofwork"];
    if work["required"] == true {
        let (Some(seed), Some(difficulty)) = (work["seed"].as_str(), work["difficulty"].as_str())
        else {
            bail!("Chat requirements requested proof of work without a seed and difficulty.");
        };
        headers.push((
            "OpenAI-Sentinel-Proof-Token",
            proof_token(seed, difficulty, started),
        ));
    }
    // The turnstile VM (`I4t`) probes a browser this native app does not have; the backend
    // accepts the turn without its token (checked 2026-11-19).
    Ok(headers)
}

fn conversation_request(identity: &Identity, sentinel: &[(&str, String)], body: &Value) -> Request {
    let mut request =
        chatgpt::web_request(identity, "POST", "/f/conversation", Some(body.to_string()));
    if let Request::Http(http) = &mut request {
        http["chatgptSse"] = json!({});
        let headers = &mut http["headers"];
        headers["Accept"] = "text/event-stream".into();
        headers["x-oai-turn-trace-id"] = host::device_id().into();
        for (name, value) in sentinel {
            headers[*name] = value.as_str().into();
        }
    }
    request
}

/// `cli`/`sli` for a new conversation's first message.
fn body(prompt: &str, message_id: &str) -> Value {
    json!({
        "action": "next",
        "messages": [{
            "id": message_id,
            "author": {"role": "user"},
            "content": {"content_type": "text", "parts": [prompt]},
            "metadata": {},
        }],
        "parent_message_id": "client-created-root",
        "model": "auto",
        "timezone": iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()),
        "timezone_offset_min": -crate::utc_offset() / 60,
        "supported_encodings": ["v1"],
    })
}

/// `A3t`, with what a native app has: no screen, heap, scripts, or DOM.
fn config(started: Instant) -> Vec<Value> {
    let now = chrono::Local::now();
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
    let mut config = vec![
        Value::Null,
        format!("{} ({zone})", now.format("%a %b %d %Y %H:%M:%S GMT%z")).into(),
        Value::Null,
        0.into(),
        host::user_agent().into(),
        Value::Null,
        Value::Null,
        "en-US".into(),
        "en-US".into(),
        0.into(),
        Value::Null,
        Value::Null,
        Value::Null,
        elapsed_ms(started).into(),
        host::device_id().into(),
        "".into(),
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .into(),
        (epoch_ms - elapsed_ms(started) as f64).into(),
    ];
    config.resize(25, 0.into());
    config
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// `N3t`: base64 of the JSON text.
fn encode(value: &impl serde::Serialize) -> String {
    STANDARD.encode(serde_json::to_string(value).unwrap_or_default())
}

/// `O3t`: the first nonce whose hash prefix is at most `difficulty`.
fn proof_token(seed: &str, difficulty: &str, started: Instant) -> String {
    let mut config = config(started);
    for nonce in 0..500_000u32 {
        config[3] = nonce.into();
        config[9] = elapsed_ms(started).into();
        let answer = encode(&config);
        let hash = hash(&format!("{seed}{answer}"));
        if hash.get(..difficulty.len()).unwrap_or(&hash) <= difficulty {
            return format!("gAAAAAB{answer}~S");
        }
    }
    format!(
        "gAAAAABwQ8Lk5FbGpA2NcR9dShT6gYjU7VxZ4D{}",
        encode(&json!("e"))
    )
}

/// `k3t`: FNV-1a over UTF-16 code units with a murmur3 finalizer, as 8 hex digits.
fn hash(text: &str) -> String {
    let mut h: u32 = 2_166_136_261;
    for unit in text.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(16_777_619);
    }
    h ^= h >> 16;
    h = h.wrapping_mul(2_246_822_507);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_909);
    h ^= h >> 16;
    format!("{h:08x}")
}

/// The previous delta's channel, path, and op, which a delta that omits them reuses (`xOn`).
struct Previous {
    channel: usize,
    path: String,
    op: String,
}

/// `yOn`/`jOn` plus the turn state the decoded events build.
struct Decoder {
    encoding: bool,
    channels: Vec<Value>,
    previous: Previous,
    turn: Turn,
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            encoding: false,
            channels: Vec::new(),
            previous: Previous {
                channel: 0,
                path: String::new(),
                op: "add".into(),
            },
            turn: Turn::default(),
        }
    }
}

impl Decoder {
    /// Applies one server-sent event; `true` when the turn changed.
    fn event(&mut self, name: Option<&str>, data: Value) -> Result<bool> {
        match name {
            Some("delta_encoding") => {
                ensure!(data == "v1", "[delta] unknown delta encoding: {data}");
                self.encoding = true;
                Ok(false)
            }
            Some("delta") => {
                ensure!(self.encoding, "[delta] delta event before delta_encoding");
                let value = self.apply(&data)?;
                self.observe(&value)
            }
            Some("ping") => Ok(false),
            _ => self.observe(&data),
        }
    }

    /// `jOn.applyDelta`: the channel's value after the delta.
    fn apply(&mut self, delta: &Value) -> Result<Value> {
        let delta = delta.as_object().context("Unexpected delta non-object")?;
        if let Some(channel) = delta.get("c") {
            self.previous.channel = channel.as_u64().context("invalid delta channel")? as usize;
        }
        if let Some(path) = delta.get("p") {
            self.previous.path = path.as_str().context("invalid delta path")?.to_owned();
        }
        if let Some(op) = delta.get("o") {
            self.previous.op = op.as_str().context("invalid delta op")?.to_owned();
        }
        let channel = self.previous.channel;
        if self.channels.len() <= channel {
            self.channels.resize(channel + 1, Value::Null);
        }
        let mut root = json!({ "__root": self.channels[channel].take() });
        let value = delta.get("v").cloned().unwrap_or(Value::Null);
        apply_op(&mut root, &self.previous.path, &self.previous.op, value)?;
        self.channels[channel] = root["__root"].take();
        Ok(self.channels[channel].clone())
    }

    fn observe(&mut self, data: &Value) -> Result<bool> {
        if let Some(error) = data.get("error").filter(|e| !e.is_null()) {
            bail!(
                "ChatGPT: {}",
                error.as_str().map_or(error.to_string(), str::to_owned)
            );
        }
        let before = self.turn.clone();
        if let Some(id) = data["conversation_id"].as_str() {
            self.turn.conversation_id = Some(id.to_owned());
        }
        if data["type"] == "message_stream_complete" {
            self.turn.complete = true;
        }
        let message = &data["message"];
        if message["author"]["role"] == "assistant"
            && message["recipient"] == "all"
            && message["content"]["content_type"] == "text"
            && message["metadata"]["is_visually_hidden_from_conversation"] != true
            && let Some(parts) = message["content"]["parts"].as_array()
        {
            self.turn.reply = parts.iter().filter_map(Value::as_str).collect();
            self.turn.message_id = message["id"].as_str().map(str::to_owned);
        }
        Ok(self.turn != before)
    }
}

/// `wOn`: one operation at a JSON-pointer `path` under `root`'s `__root`.
fn apply_op(root: &mut Value, path: &str, op: &str, value: Value) -> Result<()> {
    let mut keys = vec!["__root".to_owned()];
    if !path.is_empty() {
        let path = path.strip_prefix('/').unwrap_or(path);
        keys.extend(
            path.split('/')
                .map(|k| k.replace("~1", "/").replace("~0", "~")),
        );
    }
    let last = keys.pop().context("Unexpected empty delta path")?;
    let mut parent = root;
    for (i, key) in keys.iter().enumerate() {
        let next_is_index = index(keys.get(i + 1).unwrap_or(&last)).is_some();
        let child = slot(parent, key)?;
        if child.is_null() {
            *child = if next_is_index { json!([]) } else { json!({}) };
        }
        parent = child;
    }
    match op {
        "patch" => {
            let ops = value.as_array().context("delta patch without operations")?;
            let target = slot(parent, &last)?;
            let mut wrapped = json!({ "__root": target.take() });
            for sub in ops {
                apply_op(
                    &mut wrapped,
                    sub["p"].as_str().unwrap_or(""),
                    sub["o"]
                        .as_str()
                        .context("delta patch operation without op")?,
                    sub.get("v").cloned().unwrap_or(Value::Null),
                )?;
            }
            *target = wrapped["__root"].take();
        }
        "add" => match (parent, index(&last)) {
            (Value::Array(items), Some(at)) => items.insert(at.min(items.len()), value),
            (parent, _) => *slot(parent, &last)? = value,
        },
        "remove" => match (parent, index(&last)) {
            (Value::Array(items), Some(at)) if at < items.len() => {
                items.remove(at);
            }
            (Value::Object(map), _) => {
                map.remove(&last);
            }
            _ => {}
        },
        "replace" => *slot(parent, &last)? = value,
        "append" => {
            let target = slot(parent, &last)?;
            match (target, value) {
                (Value::String(text), Value::String(more)) => text.push_str(&more),
                (Value::Array(items), Value::Array(more)) => items.extend(more),
                (Value::Array(items), more) => items.push(more),
                (Value::Object(map), Value::Object(more)) => map.extend(more),
                (target, value) => *target = value,
            }
        }
        "truncate" => {
            let length = value.as_u64().context("delta truncate without a length")? as usize;
            match slot(parent, &last)? {
                // JavaScript `substring` counts UTF-16 code units.
                Value::String(text) => {
                    *text = String::from_utf16_lossy(
                        &text.encode_utf16().take(length).collect::<Vec<_>>(),
                    );
                }
                Value::Array(items) => items.truncate(length),
                _ => {}
            }
        }
        _ => bail!("Unknown json delta operation"),
    }
    Ok(())
}

/// A canonical array index (`kOn`).
fn index(key: &str) -> Option<usize> {
    (key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit())))
        .then(|| key.parse().ok())
        .flatten()
}

/// `parent[key]`, created as `null` when absent.
fn slot<'a>(parent: &'a mut Value, key: &str) -> Result<&'a mut Value> {
    match parent {
        Value::Object(map) => Ok(map.entry(key.to_owned()).or_insert(Value::Null)),
        Value::Array(items) => {
            let at = index(key).context("delta path key on an array")?;
            if items.len() <= at {
                items.resize(at + 1, Value::Null);
            }
            Ok(&mut items[at])
        }
        _ => bail!("delta path through a non-container"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_the_original() {
        // `k3t("0.5eyJhIjoxfQ==")` in the original bundle, run under Bun.
        assert_eq!(hash("0.5eyJhIjoxfQ=="), "5cfbb543");
    }

    #[test]
    fn proof_token_meets_the_difficulty() {
        let token = proof_token("0.25", "0fffff", Instant::now());
        let answer = token
            .strip_prefix("gAAAAAB")
            .and_then(|t| t.strip_suffix("~S"))
            .expect("solved token");
        assert!(hash(&format!("0.25{answer}"))[..6] <= *"0fffff");
    }

    #[test]
    fn deltas_reuse_the_previous_path_and_apply_every_op() -> Result<()> {
        let mut decoder = Decoder::default();
        decoder.event(Some("delta_encoding"), json!("v1"))?;
        decoder.event(
            Some("delta"),
            json!({"p": "", "o": "add", "c": 0, "v": {"message": {
                "id": "m1", "author": {"role": "assistant"}, "recipient": "all",
                "content": {"content_type": "text", "parts": [""]}, "metadata": {"a": 1}
            }, "conversation_id": "c1"}}),
        )?;
        decoder.event(
            Some("delta"),
            json!({"p": "/message/content/parts/0", "o": "append", "v": "Hel"}),
        )?;
        decoder.event(Some("delta"), json!({"v": "lo!"}))?;
        decoder.event(
            Some("delta"),
            json!({"o": "patch", "p": "", "v": [
                {"p": "/message/content/parts/0", "o": "truncate", "v": 4},
                {"p": "/message/metadata", "o": "append", "v": {"b": [2]}},
                {"p": "/message/metadata/b", "o": "append", "v": 3},
                {"p": "/message/metadata/a", "o": "remove"},
                {"p": "/message/metadata/b/0", "o": "add", "v": 1},
            ]}),
        )?;
        let channel = &decoder.channels[0];
        assert_eq!(channel["message"]["metadata"], json!({"b": [1, 2, 3]}));
        assert_eq!(
            decoder.turn,
            Turn {
                conversation_id: Some("c1".into()),
                message_id: Some("m1".into()),
                reply: "Hell".into(),
                complete: false
            }
        );
        Ok(())
    }

    #[test]
    fn stream_errors_fail_the_turn() {
        let mut decoder = Decoder::default();
        assert!(
            decoder
                .event(
                    None,
                    json!({"conversation_id": "c", "error": "Too many requests"})
                )
                .is_err()
        );
        assert!(decoder.event(Some("delta"), json!({"v": "x"})).is_err());
    }
}
