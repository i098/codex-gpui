use super::{Backend, Bridge, Incoming, Sink};
use anyhow::{Context as _, Result, anyhow, bail, ensure};
use async_channel::{Receiver, Sender, TryRecvError};
use polling::{Event as Ready, Events, Poller};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::{HeaderValue, Request};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error, HandshakeError, Message, WebSocket, client_tls_with_config};

fn tls_connector() -> Result<tungstenite::Connector> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let _ = roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(tungstenite::Connector::Rustls(Arc::new(config)))
}

pub const CLOUD_ENDPOINT: &str = "wss://codex-cloud-backend.chatgpt.com/";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const PING_INTERVAL: Duration = Duration::from_secs(10);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_MESSAGE_BYTES: usize = 100 * 1024 * 1024;

type Socket = WebSocket<MaybeTlsStream<HandshakeStream>>;

impl Bridge {
    /// Connects to a trusted durable app-server endpoint using the current ChatGPT token.
    /// Call this off the UI thread, then call `initialize` before sending RPC requests.
    /// Authentication, TLS, and handshake failures return errors without exposing the token.
    /// Account changes and reconnection remain the caller's responsibility.
    /// Non-local endpoints require TLS; plain WebSocket is limited to loopback fixtures.
    ///
    /// ```no_run
    /// # fn open(token: &str, account: &str) -> anyhow::Result<()> {
    /// let (bridge, events) = bridge::Bridge::connect_cloud(bridge::CLOUD_ENDPOINT, token, Some(account))?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn connect_cloud(
        endpoint: &str,
        token: &str,
        account_id: Option<&str>,
    ) -> Result<(Self, Receiver<super::Event>)> {
        connect(endpoint, token, account_id)
    }
}

fn handshake_request(endpoint: &str, token: &str, account_id: Option<&str>) -> Result<Request<()>> {
    ensure!(
        !token.is_empty()
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)),
        "invalid cloud authentication token"
    );
    let mut request = endpoint.into_client_request()?;
    let headers = request.headers_mut();
    headers.insert("x-openai-product-sku", HeaderValue::from_static("codex"));
    headers.insert(
        "x-openai-allow-environmentless-fallback",
        HeaderValue::from_static("true"),
    );
    headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
    let os = match std::env::consts::OS {
        "macos" => "Mac OS",
        "windows" => "Windows",
        _ => "Linux",
    };
    let user_agent = format!(
        "Codex Desktop/{} ({os}; {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    );
    headers.insert("user-agent", HeaderValue::from_str(&user_agent)?);
    let mut protocols = HeaderValue::from_str(&format!(
        "codex-app-server, codex-client.desktop, openai-bearer.{token}"
    ))
    .map_err(|_| anyhow!("invalid cloud authentication token"))?;
    protocols.set_sensitive(true);
    headers.insert("sec-websocket-protocol", protocols);
    if let Some(account_id) = account_id {
        headers.insert(
            "chatgpt-account-id",
            HeaderValue::from_str(account_id)
                .map_err(|_| anyhow!("invalid cloud account identifier"))?,
        );
    }
    Ok(request)
}

fn connect(
    endpoint: &str,
    token: &str,
    account_id: Option<&str>,
) -> Result<(Bridge, Receiver<super::Event>)> {
    let request = handshake_request(endpoint, token, account_id)?;
    let uri = request.uri();
    let host = uri.host().context("cloud endpoint has no host")?;
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let secure = uri.scheme_str() == Some("wss");
    let loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    ensure!(
        secure || uri.scheme_str() == Some("ws") && loopback,
        "cloud endpoint requires TLS or loopback"
    );
    ensure!(
        uri.authority()
            .is_none_or(|authority| !authority.as_str().contains('@')),
        "cloud endpoint must not contain credentials"
    );
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") {
            443
        } else {
            80
        });
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut connected = None;
    for address in (host, port)
        .to_socket_addrs()
        .context("resolve cloud app-server")?
    {
        ensure!(
            secure || address.ip().is_loopback(),
            "plain cloud endpoint resolved outside loopback"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Ok(stream) = TcpStream::connect_timeout(&address, remaining) {
            connected = Some(stream);
            break;
        }
    }
    let stream = connected.context("connect to cloud app-server")?;
    stream.set_nodelay(true)?;
    let stream = HandshakeStream {
        stream,
        deadline: Some(deadline),
    };
    let config = tungstenite::protocol::WebSocketConfig::default()
        .write_buffer_size(0)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let connector = if secure { Some(tls_connector()?) } else { None };
    let (mut socket, response) = client_tls_with_config(request, stream, Some(config), connector)
        .map_err(|error| match error {
        HandshakeError::Failure(Error::Http(response)) => anyhow!(
            "cloud WebSocket handshake rejected (HTTP {})",
            response.status()
        ),
        _ => anyhow!("cloud WebSocket handshake failed"),
    })?;
    ensure!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .is_some_and(|value| value == "codex-app-server"),
        "cloud app-server selected an unsupported protocol"
    );
    let stream = stream_mut(&mut socket)?;
    stream.deadline = None;
    stream.stream.set_read_timeout(None)?;
    stream.stream.set_write_timeout(None)?;
    stream.stream.set_nonblocking(true)?;
    let poller = Arc::new(Poller::new()?);
    let (sender, receiver) = async_channel::unbounded();
    let (sink, events) = Sink::new();
    let worker_poller = poller.clone();
    let worker_sink = sink.clone();
    std::thread::Builder::new()
        .name("codex-cloud".into())
        .spawn(move || {
            if let Ok(stream) = tcp(&socket) {
                // SAFETY: The worker removes the socket before it closes or drops it.
                let registered = unsafe { worker_poller.add(stream, Ready::readable(0)) };
                if registered.is_ok() {
                    let _ = run(&mut socket, &receiver, &worker_poller, &worker_sink);
                    if let Ok(stream) = tcp(&socket) {
                        let _ = worker_poller.delete(stream);
                    }
                }
            }
            worker_sink.close();
        })
        .context("start cloud transport")?;
    let backend = CloudBackend { sender, poller };
    Ok((Bridge::new(Box::new(backend), sink, None, false), events))
}

struct CloudBackend {
    sender: Sender<String>,
    poller: Arc<Poller>,
}

impl Backend for CloudBackend {
    fn send(&mut self, message: &serde_json::Value) -> Result<()> {
        let message = serde_json::to_string(message)?;
        self.sender
            .try_send(message)
            .map_err(|_| anyhow!("cloud app-server closed"))?;
        if self.poller.notify().is_err() {
            self.sender.close();
            bail!("cloud transport notification failed");
        }
        Ok(())
    }
}

impl Drop for CloudBackend {
    fn drop(&mut self) {
        self.sender.close();
        let _ = self.poller.notify();
    }
}

fn run(
    socket: &mut Socket,
    outgoing: &Receiver<String>,
    poller: &Poller,
    sink: &Sink,
) -> Result<()> {
    let mut events = Events::new();
    let mut last_ping = Instant::now();
    let mut last_pong = last_ping;
    loop {
        if outgoing.is_closed() {
            let _ = socket.close(None);
            return Ok(());
        }
        let mut write_blocked = !flush(socket)?;
        if !write_blocked {
            match outgoing.try_recv() {
                Ok(message) => match socket.write(Message::Text(message.into())) {
                    Ok(()) => {}
                    Err(error) if would_block(&error) => write_blocked = true,
                    Err(error) => return Err(error.into()),
                },
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Closed) => {
                    let _ = socket.close(None);
                    return Ok(());
                }
            }
        }
        loop {
            match socket.read() {
                Ok(Message::Text(text)) => incoming(&text, sink)?,
                Ok(Message::Binary(bytes)) => incoming(std::str::from_utf8(&bytes)?, sink)?,
                Ok(Message::Pong(_)) => last_pong = Instant::now(),
                Ok(Message::Close(_)) => {
                    let _ = socket.flush();
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) if would_block(&error) => break,
                Err(error) => return Err(error.into()),
            }
        }
        let now = Instant::now();
        ensure!(
            now.duration_since(last_pong) < PONG_TIMEOUT,
            "cloud app-server pong timeout"
        );
        if !write_blocked && now.duration_since(last_ping) >= PING_INTERVAL {
            last_ping = now;
            match socket.write(Message::Ping(Vec::new().into())) {
                Ok(()) => {}
                Err(error) if would_block(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        write_blocked = !flush(socket)?;
        if !write_blocked && !outgoing.is_empty() {
            continue;
        }
        let interest = if write_blocked {
            Ready::all(0)
        } else {
            Ready::readable(0)
        };
        poller.modify(tcp(socket)?, interest)?;
        events.clear();
        let wake = if write_blocked {
            now + PING_INTERVAL
        } else {
            last_ping + PING_INTERVAL
        };
        poller.wait(
            &mut events,
            Some(wake.saturating_duration_since(Instant::now())),
        )?;
    }
}

fn flush(socket: &mut Socket) -> Result<bool> {
    match socket.flush() {
        Ok(()) => Ok(true),
        Err(error) if would_block(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn incoming(text: &str, sink: &Sink) -> Result<()> {
    for message in serde_json::Deserializer::from_str(text).into_iter::<Incoming>() {
        sink.incoming(message.context("invalid cloud JSON-RPC message")?);
    }
    Ok(())
}

fn would_block(error: &Error) -> bool {
    matches!(error, Error::Io(error) if error.kind() == io::ErrorKind::WouldBlock)
}

fn stream_mut(socket: &mut Socket) -> Result<&mut HandshakeStream> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => Ok(stream),
        MaybeTlsStream::Rustls(stream) => Ok(&mut stream.sock),
        _ => bail!("unsupported cloud TLS stream"),
    }
}

fn tcp(socket: &Socket) -> Result<&TcpStream> {
    match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => Ok(&stream.stream),
        MaybeTlsStream::Rustls(stream) => Ok(&stream.sock.stream),
        _ => bail!("unsupported cloud TLS stream"),
    }
}

struct HandshakeStream {
    stream: TcpStream,
    deadline: Option<Instant>,
}

impl HandshakeStream {
    fn remaining(&self) -> io::Result<Option<Duration>> {
        self.deadline
            .map(|deadline| {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    Err(io::ErrorKind::TimedOut.into())
                } else {
                    Ok(remaining)
                }
            })
            .transpose()
    }
}

impl Read for HandshakeStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(remaining) = self.remaining()? {
            self.stream.set_read_timeout(Some(remaining))?;
        }
        self.stream.read(buffer)
    }
}

impl Write for HandshakeStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if let Some(remaining) = self.remaining()? {
            self.stream.set_write_timeout(Some(remaining))?;
        }
        self.stream.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests;
