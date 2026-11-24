use std::fs::{self, File};
use std::io::{self, Read as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use async_io::{Async, Timer};
use bridge::types::ThreadItem;
use futures_lite::{AsyncReadExt as _, AsyncWriteExt as _, future};
use gpui::{
    AppContext as _, Context, Global, Image, ImageFormat, IntoElement, ObjectFit,
    ParentElement as _, Render, SharedString, Styled as _, StyledImage as _, Task, Window, div,
    img, px,
};
use serde::Deserialize;
use serde_json::{Value, json};

const REQUEST: &str = "openai.artifact-session.host-request/v1";
const RESPONSE: &str = "openai.artifact-session.host-response/v1";
const EVENT: &str = "openai.artifact-session.host-event/v1";
const HOST_CALL: &str = "openai.artifact-session.host-call/v1";
const HOST_REPLY: &str = "openai.artifact-session.host-call-response/v1";
const MAX_FRAME: usize = 256 * 1024 * 1024;
const MAX_SNAPSHOT: u64 = 32 * 1024 * 1024;

/// Connection to a local Artifact Session host. This token must have the viewer role.
#[derive(Clone)]
pub struct ArtifactSessionHost {
    socket: PathBuf,
    viewer_token: String,
    export_root: PathBuf,
}

impl Global for ArtifactSessionHost {}

impl ArtifactSessionHost {
    pub fn new(socket: PathBuf, viewer_token: String, export_root: PathBuf) -> Self {
        Self {
            socket,
            viewer_token,
            export_root,
        }
    }

    pub(crate) fn from_env() -> Option<Self> {
        let token_path = std::env::var_os("ARTIFACT_SESSION_HOST_VIEWER_TOKEN_FILE")?;
        let token = fs::read_to_string(token_path).ok()?;
        Some(Self::new(
            std::env::var_os("ARTIFACT_SESSION_HOST_SOCKET")?.into(),
            token.trim_end().to_owned(),
            std::env::var_os("ARTIFACT_SESSION_HOST_EXPORT_ROOT")?.into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Spreadsheet,
    Presentation,
}

impl Kind {
    fn format(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Spreadsheet => (
                "xlsx",
                "workbook.xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            Self::Presentation => (
                "pptx",
                "presentation.pptx",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            ),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Artifact {
    #[serde(rename = "ref")]
    reference: String,
    kind: Kind,
    title: Option<SharedString>,
    external_target: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactResult {
    schema: String,
    ok: bool,
    artifact: Artifact,
}

impl Artifact {
    pub(crate) fn from_item(item: &ThreadItem) -> Option<Self> {
        let ThreadItem::McpToolCall {
            server,
            tool,
            status,
            result: Some(result),
            error: None,
            ..
        } = item
        else {
            return None;
        };
        Self::from_result(server, tool, status, result)
    }

    fn from_result(server: &str, tool: &str, status: &str, result: &Value) -> Option<Self> {
        if (server, tool, status) != ("artifact_session", "js", "completed") {
            return None;
        }
        let result =
            ArtifactResult::deserialize(result.pointer("/_meta/codex~1artifactSession")?).ok()?;
        (result.ok
            && result.schema == "openai.artifact-session.result/v1"
            && !result.artifact.reference.is_empty()
            && result.artifact.external_target.is_none())
        .then_some(result.artifact)
    }

    pub(crate) fn from_notification(params: &Value) -> Option<(&str, Self)> {
        let item = &params["item"];
        if item["type"] != "mcpToolCall" || !item["error"].is_null() {
            return None;
        }
        Some((
            params["threadId"].as_str()?,
            Self::from_result(
                item["server"].as_str()?,
                item["tool"].as_str()?,
                item["status"].as_str()?,
                &item["result"],
            )?,
        ))
    }
}

pub(crate) struct View {
    pub(crate) artifact: Artifact,
    preview: Option<Arc<Image>>,
    disconnected: bool,
    task: Option<Task<()>>,
}

impl View {
    pub(crate) fn new(artifact: Artifact) -> Self {
        Self {
            artifact,
            preview: None,
            disconnected: false,
            task: None,
        }
    }

    pub(crate) fn reference(&self) -> &str {
        &self.artifact.reference
    }

    pub(crate) fn icon(&self) -> &'static str {
        match self.artifact.kind {
            Kind::Spreadsheet => "diff-file-spreadsheet",
            Kind::Presentation => "diff-file-slides",
        }
    }

    pub(crate) fn title(&self) -> SharedString {
        self.artifact
            .title
            .clone()
            .unwrap_or_else(|| match self.artifact.kind {
                Kind::Spreadsheet => "Sheet".into(),
                Kind::Presentation => "Slide".into(),
            })
    }

    pub(crate) fn start(
        &mut self,
        host: ArtifactSessionHost,
        thread: String,
        cx: &mut Context<Self>,
    ) {
        if self.task.is_some() {
            return;
        }
        let reference = self.artifact.reference.clone();
        let kind = self.artifact.kind;
        self.disconnected = false;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: Result<()> = async {
                let mut viewer = Viewer::open(&host, &thread, &reference, kind).await?;
                loop {
                    let output = viewer
                        .call(
                            "outputs.publishNativeSnapshot",
                            json!({"threadId":thread,"artifactRef":reference}),
                        )
                        .await?;
                    let snapshot = Snapshot::deserialize(output)?;
                    ensure!(
                        snapshot.artifact_ref == reference && snapshot.artifact_type == kind,
                        "wrong artifact snapshot"
                    );
                    let version = snapshot.state_version;
                    let export_root = host.export_root.clone();
                    let image = cx
                        .background_spawn(async move { snapshot.preview(&export_root) })
                        .await?;
                    this.update(cx, |view, cx| {
                        view.preview = Some(Arc::new(Image::from_bytes(ImageFormat::Png, image)));
                        cx.notify();
                    })?;
                    while viewer.version <= version {
                        let frame = viewer.read().await?;
                        viewer.dispatch(frame).await?;
                    }
                }
            }
            .await;
            if let Err(error) = result {
                crate::logging::write(&format!("Artifact Session viewer: {error:#}"));
                let _ = this.update(cx, |view, cx| {
                    view.disconnected = true;
                    view.task = None; // let start() reconnect
                    cx.notify();
                });
            }
        }));
    }

    pub(crate) fn stop(&mut self) {
        // The host removes every subscription owned by a closed socket.
        self.task = None;
    }
}

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p(px(12.))
            .gap(px(8.))
            .children(self.preview.as_ref().map(|image| {
                img(image.clone())
                    .w_full()
                    .min_h_0()
                    .flex_1()
                    .object_fit(ObjectFit::Contain)
            }))
            .children(if self.disconnected {
                Some("Live artifact connection closed")
            } else if self.preview.is_none() {
                Some("Loading live artifact…")
            } else {
                None
            })
    }
}

struct Viewer {
    stream: Async<UnixStream>,
    token: String,
    subscription: String,
    request_id: u64,
    version: u64,
    frame: Vec<u8>,
}

impl Viewer {
    async fn open(
        host: &ArtifactSessionHost,
        thread: &str,
        reference: &str,
        kind: Kind,
    ) -> Result<Self> {
        static SUBSCRIPTIONS: AtomicU64 = AtomicU64::new(0);
        ensure!(
            host.socket.is_absolute()
                && host.export_root.is_absolute()
                && (32..=512).contains(&host.viewer_token.len()),
            "invalid local Artifact Session host configuration"
        );
        let stream = future::race(Async::<UnixStream>::connect(&host.socket), async {
            Timer::after(Duration::from_secs(5)).await;
            Err(io::ErrorKind::TimedOut.into())
        })
        .await
        .context("connect to the local Artifact Session host")?;
        let mut viewer = Self {
            stream,
            token: host.viewer_token.clone(),
            subscription: format!(
                "gpui-{}-{}",
                std::process::id(),
                SUBSCRIPTIONS.fetch_add(1, Ordering::Relaxed)
            ),
            request_id: 0,
            version: 0,
            frame: Vec::new(),
        };
        let hello = viewer.call("host.ping", json!({})).await?;
        ensure!(
            hello["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|c| c == "native-snapshot-v1")),
            "host does not support native snapshots"
        );
        let open = viewer.call("subscriptions.open", json!({"threadId":thread,"artifactRef":reference,"subscriptionId":viewer.subscription})).await?;
        ensure!(
            open["subscriptionId"] == viewer.subscription,
            "wrong Artifact Session subscription"
        );
        let document = &open["document"];
        ensure!(
            Kind::deserialize(&document["kind"])? == kind,
            "wrong Artifact Session kind"
        );
        viewer.version = viewer.version.max(
            document["stateVersion"]
                .as_u64()
                .context("missing artifact version")?,
        );
        Ok(viewer)
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_id;
        self.request_id += 1;
        future::race(async {
            self.write(json!({"schema":REQUEST,"id":id,"token":self.token,"method":method,"params":params})).await?;
            loop {
                let mut frame = self.read().await?;
                if frame["schema"] == RESPONSE {
                    ensure!(frame["id"] == id, "wrong Artifact Session response id");
                    ensure!(frame["ok"] == true, "Artifact Session host rejected {method}");
                    return Ok(frame["result"].take());
                }
                self.dispatch(frame).await?;
            }
        }, async {
            Timer::after(Duration::from_secs(120)).await;
            bail!("Artifact Session host timed out on {method}")
        }).await
    }

    async fn dispatch(&mut self, frame: Value) -> Result<()> {
        match frame["schema"].as_str() {
            Some(EVENT) => {
                let event = &frame["event"];
                if frame["subscriptionId"] == self.subscription
                    && event["type"] == "artifact-session-update-committed"
                {
                    let version = event["stateVersion"]
                        .as_u64()
                        .context("missing committed artifact version")?;
                    ensure!(
                        event["update"]["stateVersion"] == version,
                        "inconsistent artifact versions"
                    );
                    self.version = self.version.max(version);
                }
            }
            Some(HOST_CALL) => {
                let id = &frame["id"];
                ensure!(
                    id.is_string() || id.is_u64(),
                    "invalid Artifact Session host call id"
                );
                let reply = if frame["method"] == "viewer.flush"
                    && frame["params"]["subscriptionId"] == self.subscription
                {
                    json!({"schema":HOST_REPLY,"id":id,"ok":true,"result":{"flushed":true}})
                } else {
                    json!({"schema":HOST_REPLY,"id":id,"ok":false,"error":{"code":"METHOD_NOT_ALLOWED","message":"Read-only artifact viewer"}})
                };
                self.write(reply).await?;
            }
            _ => bail!("unexpected Artifact Session frame"),
        }
        Ok(())
    }

    async fn read(&mut self) -> Result<Value> {
        let mut size = [0; 4];
        self.stream.read_exact(&mut size).await?;
        let len = u32::from_be_bytes(size) as usize;
        ensure!(
            (1..=MAX_FRAME).contains(&len),
            "invalid Artifact Session frame size"
        );
        self.frame.resize(len, 0);
        self.stream.read_exact(&mut self.frame).await?;
        Ok(serde_json::from_slice(&self.frame)?)
    }

    async fn write(&mut self, message: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&message)?;
        self.stream
            .write_all(&u32::try_from(bytes.len())?.to_be_bytes())
            .await?;
        self.stream.write_all(&bytes).await?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    artifact_ref: String,
    artifact_type: Kind,
    state_version: u64,
    format: String,
    content_type: String,
    byte_length: u64,
    binding: Binding,
    value: SnapshotPath,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    file_name: String,
}

#[derive(Deserialize)]
struct SnapshotPath {
    path: PathBuf,
}

impl Snapshot {
    fn preview(self, export_root: &Path) -> Result<Vec<u8>> {
        let (format, name, mime) = self.artifact_type.format();
        ensure!(
            self.format == format
                && self.binding.file_name == name
                && self.content_type == mime
                && (1..=MAX_SNAPSHOT).contains(&self.byte_length)
                && self.value.path.is_absolute(),
            "invalid native artifact snapshot"
        );
        let root = export_root.canonicalize()?;
        let path = self.value.path.canonicalize()?;
        ensure!(
            path.starts_with(root) && path.file_name().is_some_and(|file| file == name),
            "artifact snapshot is outside its export root"
        );
        ensure!(
            path.metadata()?.is_file(),
            "artifact snapshot is not a file"
        );
        let dir = tempfile::tempdir()?;
        let input = dir.path().join(name);
        let file = File::open(&path)?;
        let copied = io::copy(
            &mut file.take(self.byte_length + 1),
            &mut File::create(&input)?,
        )?;
        ensure!(
            copied == self.byte_length,
            "artifact snapshot length changed"
        );
        let command = Command::new("/usr/bin/qlmanage")
            .args(["-t", "-s", "1600", "-o"])
            .arg(dir.path())
            .arg(&input)
            .output()?;
        ensure!(
            command.status.success(),
            "Quick Look could not render the artifact snapshot"
        );
        Ok(fs::read(dir.path().join(format!("{name}.png")))?)
    }
}
