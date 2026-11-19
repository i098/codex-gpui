//! A ChatGPT turn through `CloudHost` against a local backend that answers with the
//! recorded (scrubbed) `/f/conversation` stream.

use anyhow::{Context as _, Result};
use app::chatgpt_conversation::{Turn, send};
use app::host::{CloudHost, Credentials, Host, LocalHost};
use base64::Engine as _;
use serde_json::{Value, json};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

const STREAM: &str = include_str!("fixtures/chatgpt-conversation.sse");

struct Received {
    line: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Serves `responses` in order, one per connection, and returns what each request sent.
fn backend(
    responses: Vec<(&'static str, String)>,
) -> Result<(String, std::thread::JoinHandle<Vec<Received>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}/backend-api", listener.local_addr()?);
    let server = std::thread::spawn(move || {
        let mut received = Vec::new();
        for (content_type, body) in responses {
            let (stream, _) = listener.accept().expect("connection");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            let mut headers = Vec::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header");
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (name, value) = header.split_once(':').expect("header pair");
                headers.push((name.to_owned(), value.trim().to_owned()));
            }
            let length = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, v)| v.parse().expect("length"));
            let mut sent = vec![0; length];
            reader.read_exact(&mut sent).expect("body");
            received.push(Received {
                line: line.trim_end().to_owned(),
                headers,
                body: serde_json::from_slice(&sent).unwrap_or(Value::Null),
            });
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("response");
        }
        received
    });
    Ok((base, server))
}

#[test]
fn chat_turn_sends_the_sentinel_headers_and_decodes_the_reply() -> Result<()> {
    let requirements = json!({
        "persona": "chatgpt-freeaccount",
        "prepare_token": "prepare-token",
        "proofofwork": {"required": true, "seed": "0.25", "difficulty": "0fffff"},
        "turnstile": {"required": true, "dx": "vm"},
    });
    let (base, server) = backend(vec![
        ("application/json", requirements.to_string()),
        ("text/event-stream", STREAM.to_owned()),
    ])?;
    let claims = json!({"https://api.openai.com/auth": {
        "chatgpt_account_id": "acct", "chatgpt_user_id": "user"
    }});
    let token = format!(
        "eyJhbGciOiJSUzI1NiJ9.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let host = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(move |_| {
            Ok(Some(Credentials {
                token: token.clone(),
                backend_origin: None,
                routing_override: None,
            }))
        }),
        base,
    );
    let identity = host.identity().context("identity")?;
    let mut updates = Vec::new();
    let previous = Turn {
        conversation_id: Some("conversation".into()),
        message_id: Some("parent".into()),
        work: true,
        ..Turn::default()
    };
    let turn = send(
        &host,
        &identity,
        "Reply with the single word: pong",
        &previous,
        &AtomicBool::new(false),
        &mut |turn| updates.push(turn.reply.clone()),
    )?;
    assert_eq!(
        turn,
        Turn {
            conversation_id: Some("00000000-0000-4000-8000-000000000001".into()),
            message_id: Some("00000000-0000-4000-8000-000000000012".into()),
            reply: "pong".into(),
            complete: true,
            work: true,
            model_limits: Vec::new(),
        }
    );
    assert!(updates.contains(&"pong".to_owned()), "{updates:?}");

    let [prepare, conversation] = <[Received; 2]>::try_from(server.join().expect("server"))
        .map_err(|r| anyhow::anyhow!("{} requests", r.len()))?;
    assert_eq!(
        prepare.line,
        "POST /backend-api/sentinel/chat-requirements/prepare HTTP/1.1"
    );
    assert!(
        prepare.body["p"]
            .as_str()
            .is_some_and(|p| p.starts_with("gAAAAAC"))
    );
    assert_eq!(
        conversation.line,
        "POST /backend-api/f/conversation HTTP/1.1"
    );
    assert!(
        conversation
            .header("Authorization")
            .is_some_and(|a| a.starts_with("Bearer "))
    );
    assert_eq!(
        conversation.header("OpenAI-Sentinel-Chat-Requirements-Prepare-Token"),
        Some("prepare-token")
    );
    assert!(
        conversation
            .header("OpenAI-Sentinel-Proof-Token")
            .is_some_and(|t| t.starts_with("gAAAAAB") && t.ends_with("~S"))
    );
    let body = &conversation.body;
    assert_eq!(body["action"], "next");
    assert_eq!(body["parent_message_id"], "parent");
    assert_eq!(body["conversation_id"], "conversation");
    assert_eq!(body["supported_encodings"], json!(["v1"]));
    assert_eq!(
        body["messages"][0]["content"],
        json!({"content_type": "text", "parts": ["Reply with the single word: pong"]})
    );
    Ok(())
}
