use super::*;
use crate::{Event, RpcError};
use serde_json::{Value, json};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use tungstenite::handshake::server::{Callback, ErrorResponse, Request as ServerRequest, Response};

const TOKEN: &str = "fixture.access.token";

enum Handshake {
    Accept,
    Reject,
}

impl Callback for Handshake {
    fn on_request(
        self,
        request: &ServerRequest,
        mut response: Response,
    ) -> std::result::Result<Response, ErrorResponse> {
        if matches!(self, Self::Reject) {
            return Err(tungstenite::http::Response::builder()
                .status(403)
                .body(Some(TOKEN.into()))
                .unwrap());
        }
        assert_eq!(request.headers()["x-openai-product-sku"], "codex");
        assert_eq!(
            request.headers()["x-openai-allow-environmentless-fallback"],
            "true"
        );
        assert_eq!(request.headers()["chatgpt-account-id"], "fixture-account");
        assert_eq!(
            request.headers()["sec-websocket-protocol"],
            format!("codex-app-server, codex-client.desktop, openai-bearer.{TOKEN}")
        );
        assert!(!request.headers().contains_key("authorization"));
        response.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("codex-app-server"),
        );
        Ok(response)
    }
}

struct HandshakeLogs(std::sync::Mutex<Vec<String>>);
static HANDSHAKE_LOGS: HandshakeLogs = HandshakeLogs(std::sync::Mutex::new(Vec::new()));

impl log::Log for HandshakeLogs {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.target().starts_with("tungstenite::handshake") {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}

fn capture_handshake_logs() {
    static SETUP: std::sync::Once = std::sync::Once::new();
    SETUP.call_once(|| {
        log::set_logger(&HANDSHAKE_LOGS).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

fn install_panic_hook() {
    #[cfg(target_os = "macos")]
    {
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                eprintln!("{info}");
                std::process::exit(101);
            }))
        });
    }
}

fn fixture(
    handler: impl FnOnce(WebSocket<TcpStream>) + Send + 'static,
) -> (String, JoinHandle<()>) {
    install_panic_hook();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let thread = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let socket = tungstenite::accept_hdr(stream, Handshake::Accept).unwrap();
        handler(socket);
    });
    (url, thread)
}

fn receive(socket: &mut WebSocket<TcpStream>) -> Value {
    loop {
        match socket.read().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) => socket.flush().unwrap(),
            message => panic!("unexpected fixture message: {message:?}"),
        }
    }
}

fn send(socket: &mut WebSocket<TcpStream>, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .unwrap();
}

#[test]
fn cloud_socket_routes_replies_and_bidirectional_requests() {
    let (url, server) = fixture(|mut socket| {
        let initialize = receive(&mut socket);
        assert_eq!(initialize["method"], "initialize");
        send(
            &mut socket,
            json!({"id": initialize["id"], "result": {
                "userAgent":"fixture","codexHome":"/workspace","platformOs":"linux"
            }}),
        );
        assert_eq!(receive(&mut socket)["method"], "initialized");
        socket.send(Message::Text(concat!(
            "{\"method\":\"thread/started\",\"params\":{\"threadId\":\"durable-one\"}}\n",
            "{\"id\":\"approval-id\",\"method\":\"item/tool/requestUserInput\",\"params\":{}}"
        ).into())).unwrap();
        assert_eq!(
            receive(&mut socket),
            json!({"id":"approval-id","result":{"answer":"yes"}})
        );
        let request = receive(&mut socket);
        assert_eq!(request["method"], "thread/read");
        assert_eq!(request["params"]["threadId"], "durable-one");
        send(
            &mut socket,
            json!({"id":request["id"],"error":{"code":-32004,"message":"not found"}}),
        );
        let _ = socket.close(None);
    });
    let (bridge, events) = Bridge::connect_cloud(&url, TOKEN, Some("fixture-account")).unwrap();
    futures_lite::future::block_on(async {
        bridge.initialize("fixture", "1").await.unwrap();
        assert_eq!(
            events.recv().await.unwrap(),
            Event::Notification {
                method: "thread/started".into(),
                params: json!({"threadId":"durable-one"})
            }
        );
        assert_eq!(
            events.recv().await.unwrap(),
            Event::Request {
                id: json!("approval-id"),
                method: "item/tool/requestUserInput".into(),
                params: json!({})
            }
        );
        bridge
            .respond(json!("approval-id"), json!({"answer":"yes"}))
            .unwrap();
        let error = bridge
            .request::<_, Value>("thread/read", json!({"threadId":"durable-one"}))
            .await
            .unwrap_err();
        assert_eq!(error.downcast_ref::<RpcError>().unwrap().code, -32004);
    });
    server.join().unwrap();
}

#[test]
fn disconnect_fails_pending_requests_and_closes_subscribers() {
    let (url, server) = fixture(|mut socket| {
        assert_eq!(receive(&mut socket)["method"], "thread/read");
        socket.close(None).unwrap();
    });
    let (bridge, events) = Bridge::connect_cloud(&url, TOKEN, Some("fixture-account")).unwrap();
    futures_lite::future::block_on(async {
        assert!(
            bridge
                .request::<_, Value>("thread/read", json!({"threadId":"pending"}))
                .await
                .is_err()
        );
        assert!(events.recv().await.is_err());
        assert!(
            bridge
                .request::<_, Value>("thread/list", json!({}))
                .await
                .is_err()
        );
    });
    server.join().unwrap();
}

#[test]
fn dropping_idle_bridge_wakes_and_closes_the_socket() {
    let (url, server) = fixture(|mut socket| {
        assert!(matches!(socket.read().unwrap(), Message::Close(_)));
    });
    let (bridge, events) = Bridge::connect_cloud(&url, TOKEN, Some("fixture-account")).unwrap();
    drop(bridge);
    assert!(events.recv_blocking().is_err());
    server.join().unwrap();
}

#[test]
fn authentication_cannot_inject_an_extra_subprotocol_or_header() {
    install_panic_hook();
    for token in ["", "a,b", "a\r\nb", "a b"] {
        assert!(handshake_request(CLOUD_ENDPOINT, token, None).is_err());
    }
    assert!(handshake_request(CLOUD_ENDPOINT, TOKEN, Some("a\r\nb")).is_err());
    assert!(Bridge::connect_cloud("ws://example.com/", TOKEN, None).is_err());
}

#[test]
fn refused_handshake_does_not_expose_credentials() {
    install_panic_hook();
    capture_handshake_logs();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = tungstenite::accept_hdr(stream, Handshake::Reject);
    });
    let error = Bridge::connect_cloud(&url, TOKEN, None).err().unwrap();
    assert!(error.to_string().contains("403"));
    assert!(!format!("{error:?}").contains(TOKEN));
    server.join().unwrap();
    let logs = HANDSHAKE_LOGS.0.lock().unwrap();
    assert!(
        logs.iter()
            .any(|message| message.contains("Client handshake initiated"))
    );
    assert!(logs.iter().all(|message| !message.contains(TOKEN)));
}

#[test]
fn secure_handshake_starts_tls_and_returns_an_error_without_panicking() {
    install_panic_hook();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("wss://{}/", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut header = [0; 5];
        socket.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 22);
        assert!(u16::from_be_bytes([header[3], header[4]]) > 0);
        socket.shutdown(std::net::Shutdown::Both).unwrap();
    });
    let result = Bridge::connect_cloud(&endpoint, TOKEN, None);
    assert!(result.is_err());
    server.join().unwrap();
}
