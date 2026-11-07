//! `Share chat` (`/share`, the header's `Share` and the row menu's `Share`): the original's
//! `shareThreadSnapshot` / `publishSharedSnapshot`. It reads the thread, builds the version 1
//! snapshot, redacts secrets, uploads it through `/wham/shared_threads/upload_urls` and
//! `POST /wham/shared_threads`, then copies the returned `share_url`.
//! shortcut: images become the original's `codex:shared-image-unavailable` placeholder (no
//! asset upload), no omission of file changes for a snapshot over 10 MiB (it refuses), and the
//! access picker is absent (the backend's default audience applies); port them if a share
//! needs them. Live smoke pending: no ChatGPT sign-in exists on this host.

use crate::cloud::request;
use crate::*;
use gpui::ClipboardItem;
use regex::{Captures, Regex};
use std::sync::LazyLock;

const MAX_BYTES: usize = 10 * 1024 * 1024;
const UNAVAILABLE_IMAGE: &str = "codex:shared-image-unavailable";

impl Root {
    pub(crate) fn share_thread(&mut self, id: String, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        self.toast(Toast::info("Creating share link…"), cx);
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let read: Result<Value> = bridge
                .request("thread/read", json!({"threadId": id, "includeTurns": true}))
                .await;
            let url = match read {
                Ok(read) => {
                    cx.background_spawn(async move { publish(host.as_ref(), &read["thread"]) })
                        .await
                }
                Err(error) => Err(error),
            };
            this.update(cx, |this, cx| match &url {
                Ok(url) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                    this.toast(Toast::success("Link copied"), cx);
                }
                Err(error) => {
                    eprintln!("codex-gpui: cannot share thread: {error:#}");
                    this.toast(
                        Toast::danger("Couldn't share chat").description(error.to_string()),
                        cx,
                    );
                }
            })?;
            url.map(drop)
        });
        log_err(task, cx);
    }

    /// `/share` and the header's `Share` act on the open thread.
    pub(crate) fn share_open_thread(&mut self, cx: &mut Context<Self>) {
        if let Route::Thread(id) = self.state.route.clone() {
            self.share_thread(id, cx);
        }
    }
}

/// Uploads the redacted snapshot of `thread` (a `thread/read` result) and returns its URL.
fn publish(host: &dyn host::Host, thread: &Value) -> Result<String> {
    let identity = host
        .identity()
        .context("Sign in with ChatGPT to share a chat")?;
    let snapshot = redact(snapshot(thread));
    let body = serde_json::to_string(&snapshot)?;
    ensure!(
        body.len() <= MAX_BYTES,
        "This chat is too large to share (over 10 MiB)"
    );
    let targets = request(
        host,
        &identity,
        "POST",
        "/wham/shared_threads/upload_urls",
        Some(json!({"snapshot": {"size": body.len(), "version": 1}})),
    )?;
    let target = &targets["snapshot"];
    let upload_url = target["upload_url"]
        .as_str()
        .context("Missing shared thread snapshot upload target")?;
    upload(host, upload_url, &target["headers"], body)?;
    let created = request(
        host,
        &identity,
        "POST",
        "/wham/shared_threads",
        Some(json!({
            "snapshot": {"etag": target["etag"], "file_id": target["file_id"]},
            "selection": "full_thread",
        })),
    )?;
    created["share_url"]
        .as_str()
        .map(str::to_owned)
        .context("The share response has no link")
}

/// `PUT` to the signed upload URL. It carries no ChatGPT credentials, only the target's headers.
fn upload(host: &dyn host::Host, url: &str, headers: &Value, body: String) -> Result<()> {
    let mut sent: serde_json::Map<String, Value> = headers.as_object().cloned().unwrap_or_default();
    if !sent.keys().any(|k| k.eq_ignore_ascii_case("content-type")) {
        sent.insert("Content-Type".into(), "application/json".into());
    }
    let response = host.fetch(&host::Request::Http(json!({
        "url": url, "method": "PUT", "headers": sent, "body": body,
        "returnErrorResponse": true,
    })))?;
    let response = response.get("value").unwrap_or(&response);
    let response = response.pointer("/response/__response").unwrap_or(response);
    let status = response["status"]
        .as_u64()
        .context("Upload has no status")?;
    ensure!(
        (200..300).contains(&status),
        "Snapshot upload failed (HTTP {status})"
    );
    Ok(())
}

/// The original's `A`: user text and images, final answers and commentary, reasoning
/// summaries and completed file changes; every other item is left out.
fn snapshot(thread: &Value) -> Value {
    let cwd = thread["cwd"].as_str();
    let turns: Vec<Value> = thread["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|turn| {
            let items: Vec<Value> = turn["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| snapshot_item(item, cwd))
                .collect();
            if items.is_empty() {
                return None;
            }
            let work = items.iter().any(|item| {
                item["type"] == "reasoning"
                    || (item["type"] == "agentMessage" && item["phase"] == "commentary")
            });
            let mut out = json!({"items": items});
            if let Some(ms) = turn["durationMs"].as_f64().filter(|ms| work && *ms >= 0.0) {
                out["durationMs"] = ms.into();
            }
            Some(out)
        })
        .collect();
    let mut out = json!({"version": 1, "turns": turns});
    if let Some(title) = thread["name"].as_str() {
        out["title"] = title.into();
    }
    out
}

fn snapshot_item(item: &Value, cwd: Option<&str>) -> Option<Value> {
    match item["type"].as_str()? {
        "userMessage" => {
            let content: Vec<Value> = item["content"]
                .as_array()?
                .iter()
                .filter_map(|input| match input["type"].as_str()? {
                    "text" => {
                        let text = input["text"].as_str()?.trim();
                        (!text.is_empty()).then(|| json!({"type": "text", "text": text}))
                    }
                    "image" | "localImage" => {
                        Some(json!({"type": "image", "url": UNAVAILABLE_IMAGE}))
                    }
                    _ => None,
                })
                .collect();
            (!content.is_empty()).then(|| json!({"type": "userMessage", "content": content}))
        }
        "agentMessage" => {
            let phase = item["phase"].as_str()?;
            if phase != "final_answer" && phase != "commentary" {
                return None;
            }
            let text = item["text"].as_str()?;
            // A leading omission marker would read as the generator's own notice.
            let text = if text.starts_with("[codex-shared-snapshot-omissions-v1]: #") {
                format!("\\{text}")
            } else {
                text.to_owned()
            };
            let mut out = json!({"type": "agentMessage", "text": text.trim()});
            if phase == "commentary" {
                out["phase"] = "commentary".into();
            }
            (!text.trim().is_empty()).then_some(out)
        }
        "reasoning" => {
            let summary = item["summary"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n\n");
            let summary = summary.trim();
            (!summary.is_empty()).then(|| json!({"type": "reasoning", "summary": summary}))
        }
        "fileChange" => {
            if matches!(item["status"].as_str()?, "failed" | "declined") {
                return None;
            }
            let changes: Vec<Value> = item["changes"]
                .as_array()?
                .iter()
                .filter_map(|change| {
                    let diff = change_diff(change)?;
                    let path = change["path"].as_str().map(|path| shared_path(path, cwd));
                    Some(match path {
                        Some(path) => json!({"path": path, "diff": diff}),
                        None => json!({"diff": diff}),
                    })
                })
                .collect();
            (!changes.is_empty())
                .then(|| json!({"type": "fileChange", "status": "completed", "changes": changes}))
        }
        _ => None,
    }
}

/// The original's `be`: a diff without its git headers; added and deleted files get `+`/`-`
/// on every line.
fn change_diff(change: &Value) -> Option<String> {
    let diff = change["diff"]
        .as_str()?
        .replace("\r\n", "\n")
        .lines()
        .filter(|line| {
            const HEADERS: [&str; 10] = [
                "diff --git ",
                "index ",
                "--- ",
                "+++ ",
                "old ",
                "new ",
                "deleted file",
                "similarity index",
                "rename from",
                "rename to",
            ];
            !HEADERS.iter().any(|header| line.starts_with(header))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let diff = diff.trim();
    if diff.is_empty() {
        return None;
    }
    let prefix = match change["kind"]["type"].as_str() {
        Some("add") => "+",
        Some("delete") => "-",
        _ => return Some(diff.to_owned()),
    };
    Some(
        diff.lines()
            .map(|line| format!("{prefix}{line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The original's `xe`: a path inside the thread's `cwd` is relative to it; any other absolute
/// path keeps only its file name so no home directory leaves the machine.
fn shared_path(path: &str, cwd: Option<&str>) -> String {
    if !path.starts_with('/') {
        return path.to_owned();
    }
    if let Some(rest) = cwd
        .map(|cwd| cwd.trim_end_matches('/'))
        .and_then(|cwd| path.strip_prefix(cwd))
        .and_then(|rest| rest.strip_prefix('/'))
    {
        return rest.to_owned();
    }
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

/// The original's `NCr`: every string in the snapshot with its secrets replaced by `<SECRET>`.
fn redact(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, redact(v))).collect()),
        other => other,
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("secret pattern")
}

static KEYS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----[\s\S]{20,}?-----END (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----",
        r"\b(?:A3T[A-Z0-9]|AKIA|ASIA|ABIA|ACCA)[A-Z2-7]{16}\b",
        r"\bAIza[A-Za-z0-9_-]{30,}\b",
        r"\b(?:github_pat_[A-Za-z0-9_]{30,}|gh[pousr]_[A-Za-z0-9]{20,})\b",
        r"(?i)\b(?:glpat-[A-Za-z0-9_-]{20,}|gldt-[A-Za-z0-9_-]{20}|glptt-[a-f0-9]{40}|glrt-[A-Za-z0-9_-]{20,})\b",
        r"(?i)\bxox[baprs]-[A-Za-z0-9-]{10,}\b|\bxapp-\d-[A-Z0-9]+-\d+-[A-Za-z0-9]+\b",
        r"\b(?:sk|rk)_(?:test|live|prod)_[A-Za-z0-9]{10,99}\b",
        r"\bSG\.[A-Za-z0-9_.=-]{66,}",
        r"\b(?:hf_[A-Za-z0-9]{20,}|api_org_[A-Za-z]{34})\b",
        r"(?i)\bpypi-[A-Za-z0-9_-]{20,1000}\b",
        r"(?i)\bnpm_[A-Za-z0-9]{20,}\b",
        r"\b(?:sk-(?:proj|svcacct|admin)-[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20})\b",
        r"\bsk-ant-(?:admin01|api03)-[A-Za-z0-9_-]{93}AA\b",
        r"\bAGE-SECRET-KEY-1[QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L]{58}\b",
        r"(?i)\blin_api_[a-z0-9]{40}\b",
        r"\bpplx-[A-Za-z0-9]{48}\b",
        r"(?i)\bshp(?:at|ca|pa|ss)_[a-f0-9]{32}\b",
        r"(?i)\bdapi[a-f0-9]{32}(?:-\d)?\b",
        r"(?i)https://hooks\.slack\.com/services/[A-Za-z0-9]{8,128}/[A-Za-z0-9]{8,128}/[A-Za-z0-9]{16,512}",
    ]
    .map(re)
    .into()
});

static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)\b(?P<name>TOKEN|SECRET|PASSWORD|PASSWD|API_KEY|ACCESS_KEY|PRIVATE_KEY|CREDENTIAL|COOKIE|[A-Z][A-Z0-9_]{1,80}(?:TOKEN|SECRET|PASSWORD|PASSWD|_PWD|API_KEY|ACCESS_KEY|PRIVATE_KEY|CREDENTIAL|COOKIE|_PAT))\b[^\n\r]{0,30}?[:=]\s*["']?(?P<secret>[A-Za-z0-9._~+/=%:@!#$^&*?-]{8,})"#,
    )
});
static AUTHORIZATION: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\bAuthorization\s*:\s*(?:Bearer|Basic)\s+(?P<secret>[A-Za-z0-9._~+/=-]{16,})")
});
static URL_CREDENTIALS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)\b(?:https?|mongodb|mysql|postgres(?:ql)?|redis)://(?P<user>[^\s:/@"'<>]{1,80}):(?P<password>[^\s@"'<>]{3,160})@[^\s"'<>]+"#,
    )
});
static SIGNED_URL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)https?://[^\s"'<>]+"#));
static SIGNATURE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)[?&](?:token|sig|X-Amz-Signature|X-Amz-Security-Token)="));

/// The original's `ICr`: the byte ranges of every secret, merged, then replaced.
fn redact_text(text: &str) -> String {
    if text.starts_with("data:image/") {
        return text.to_owned();
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for pattern in KEYS.iter() {
        spans.extend(pattern.find_iter(text).map(|m| (m.start(), m.end())));
    }
    let named =
        |regex: &Regex, spans: &mut Vec<(usize, usize)>, valid: &dyn Fn(&Captures) -> bool| {
            for caps in regex.captures_iter(text) {
                if let Some(secret) = caps.name("secret").filter(|_| valid(&caps)) {
                    spans.push((secret.start(), secret.end()));
                }
            }
        };
    named(&NAMED, &mut spans, &|caps| {
        let secret = &caps["secret"];
        let minimum = if ["PASSWORD", "PASSWD", "_PWD"]
            .iter()
            .any(|word| caps["name"].to_uppercase().contains(word))
        {
            8
        } else {
            16
        };
        !secret.contains("://") && strong(secret, minimum)
    });
    named(&AUTHORIZATION, &mut spans, &|caps| {
        strong(&caps["secret"], 16)
    });
    for caps in URL_CREDENTIALS.captures_iter(text) {
        if strong(&caps["password"], 8) {
            let all = caps.get(0).expect("whole match");
            spans.push((all.start(), all.end()));
        }
    }
    spans.extend(
        SIGNED_URL
            .find_iter(text)
            .filter(|m| SIGNATURE.is_match(m.as_str()))
            .map(|m| (m.start(), m.end())),
    );
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (start, end) in merged {
        out.push_str(&text[at..start]);
        out.push_str("<SECRET>");
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// The original's `WCr`: not a placeholder, long enough, not all digits, 4+ distinct bytes.
fn strong(secret: &str, minimum: usize) -> bool {
    let secret = secret.trim().trim_matches(|c| c == '"' || c == '\'');
    let lower = secret.to_lowercase();
    let placeholder = secret.is_empty()
        || ["${", "$(", "{{", "<", "[", "process.env", "os.environ"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
        || (secret.starts_with('$')
            && secret.len() > 1
            && secret[1..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'))
        || [
            "example",
            "dummy",
            "placeholder",
            "changeme",
            "replace_me",
            "redacted",
            "masked",
        ]
        .iter()
        .any(|word| lower.contains(word))
        || [
            "password",
            "passwd",
            "secret",
            "token",
            "apikey",
            "api_key",
            "none",
            "null",
            "true",
            "false",
            "undefined",
        ]
        .contains(&lower.as_str())
        || secret
            .chars()
            .collect::<std::collections::HashSet<_>>()
            .len()
            <= 3;
    !placeholder
        && secret.len() >= minimum
        && secret.len() <= 4096
        && !secret.bytes().all(|b| b.is_ascii_digit())
        && secret
            .chars()
            .collect::<std::collections::HashSet<_>>()
            .len()
            >= 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_keeps_the_shared_item_kinds() {
        let thread = json!({
            "cwd": "/w", "name": "Fix it",
            "turns": [{
                "durationMs": 1500.0,
                "items": [
                    {"type": "userMessage", "id": "1", "content": [
                        {"type": "text", "text": " Hi "},
                        {"type": "localImage", "path": "/w/a.png"}]},
                    {"type": "commandExecution", "id": "2", "command": "ls"},
                    {"type": "agentMessage", "id": "3", "text": "Looking", "phase": "commentary"},
                    {"type": "fileChange", "id": "4", "status": "completed", "changes": [
                        {"path": "/w/src/a.rs", "kind": {"type": "add"},
                         "diff": "diff --git a/x b/x\n--- a\n+++ b\nfn a() {}"},
                        {"path": "/home/me/b.rs", "kind": {"type": "update"}, "diff": "@@\n-x\n+y"}]},
                    {"type": "fileChange", "id": "5", "status": "failed", "changes": []},
                    {"type": "agentMessage", "id": "6", "text": "Done", "phase": "final_answer"}
                ]
            }, {"items": [{"type": "commandExecution", "id": "7"}]}]
        });
        assert_eq!(
            snapshot(&thread),
            json!({"version": 1, "title": "Fix it", "turns": [{"durationMs": 1500.0, "items": [
                {"type": "userMessage", "content": [
                    {"type": "text", "text": "Hi"},
                    {"type": "image", "url": "codex:shared-image-unavailable"}]},
                {"type": "agentMessage", "text": "Looking", "phase": "commentary"},
                {"type": "fileChange", "status": "completed", "changes": [
                    {"path": "src/a.rs", "diff": "+fn a() {}"},
                    {"path": "b.rs", "diff": "@@\n-x\n+y"}]},
                {"type": "agentMessage", "text": "Done"}
            ]}]})
        );
    }

    #[test]
    fn redaction_replaces_secrets_and_keeps_placeholders() {
        let text = "key AKIAABCDEFGHIJKLMNOP and ghp_abcdefghijklmnopqrstuvwx\n\
            API_KEY=\"abcd1234efgh5678ijkl\" PASSWORD=hunter2hunter2\n\
            export TOKEN=$TOKEN API_KEY=your-example-key-here\n\
            curl -H 'Authorization: Bearer abcdefghijklmnop1234' https://u:s3cretpass@host/x\n\
            https://bucket.s3.amazonaws.com/f?X-Amz-Signature=abcdef plain https://example.com/a";
        assert_eq!(
            redact_text(text),
            "key <SECRET> and <SECRET>\n\
            API_KEY=\"<SECRET>\" PASSWORD=<SECRET>\n\
            export TOKEN=$TOKEN API_KEY=your-example-key-here\n\
            curl -H 'Authorization: Bearer <SECRET>' <SECRET>\n\
            <SECRET> plain https://example.com/a"
        );
        let pem =
            "-----BEGIN PRIVATE KEY-----\nAAAAAAAAAAAAAAAAAAAAAAAA\n-----END PRIVATE KEY-----";
        assert_eq!(redact_text(&format!("a {pem} b")), "a <SECRET> b");
        assert_eq!(
            redact(json!({"turns": [{"text": "ghp_abcdefghijklmnopqrstuvwx"}]})),
            json!({"turns": [{"text": "<SECRET>"}]})
        );
    }

    #[test]
    fn paths_outside_the_cwd_lose_their_directories() {
        assert_eq!(shared_path("/w/a/b.rs", Some("/w")), "a/b.rs");
        assert_eq!(shared_path("/wx/b.rs", Some("/w")), "b.rs");
        assert_eq!(shared_path("rel/b.rs", Some("/w")), "rel/b.rs");
    }

    struct Backend(std::sync::Mutex<Vec<Value>>);

    impl host::Host for Backend {
        fn fetch(&self, request: &host::Request) -> Result<Value> {
            let host::Request::Http(http) = request else {
                anyhow::bail!("not http")
            };
            self.0.lock().unwrap().push(http.clone());
            let body = match http["url"].as_str().unwrap() {
                "/wham/shared_threads/upload_urls" => {
                    json!({"snapshot": {"upload_url": "https://s3/up", "etag": "e1",
                        "file_id": "f1", "headers": {"x-amz-meta": "1"}}})
                }
                "/wham/shared_threads" => {
                    json!({"share_id": "s", "share_url": "https://chatgpt.com/s/s"})
                }
                _ => json!({}),
            };
            Ok(json!({"status": 200, "body": body.to_string()}))
        }

        fn identity(&self) -> Option<host::Identity> {
            Some(host::Identity {
                account_id: "a".into(),
                user_id: "u".into(),
                device_id: "d".into(),
            })
        }
    }

    #[test]
    fn publish_uploads_the_redacted_snapshot_then_creates_the_share() {
        let backend = Backend(Default::default());
        let thread = json!({"cwd": "/w", "turns": [{"items": [
            {"type": "agentMessage", "text": "ghp_abcdefghijklmnopqrstuvwx", "phase": "final_answer"}]}]});
        let url = publish(&backend, &thread).expect("share");
        assert_eq!(url, "https://chatgpt.com/s/s");
        let calls = backend.0.lock().unwrap();
        let urls: Vec<_> = calls.iter().map(|c| c["url"].as_str().unwrap()).collect();
        assert_eq!(
            urls,
            [
                "/wham/shared_threads/upload_urls",
                "https://s3/up",
                "/wham/shared_threads"
            ]
        );
        let upload = &calls[1];
        assert_eq!(upload["method"], "PUT");
        assert_eq!(upload["headers"]["x-amz-meta"], "1");
        assert!(upload["headers"].get("X-OpenAI-Attach-Auth").is_none());
        let sent: Value = serde_json::from_str(upload["body"].as_str().unwrap()).unwrap();
        assert_eq!(sent["turns"][0]["items"][0]["text"], "<SECRET>");
        let size = calls[0]["body"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(size).unwrap()["snapshot"]["size"],
            upload["body"].as_str().unwrap().len()
        );
        let created: Value = serde_json::from_str(calls[2]["body"].as_str().unwrap()).unwrap();
        assert_eq!(created["snapshot"], json!({"etag": "e1", "file_id": "f1"}));
        assert_eq!(created["selection"], "full_thread");
    }
}
