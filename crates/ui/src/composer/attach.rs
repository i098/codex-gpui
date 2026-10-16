use super::*;
use crate::screens::settings::AppServer;
use anyhow::{Context as _, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bridge::Bridge;
use gpui::{StyledImage as _, Task};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) enum Content {
    File,
    Text(String),
    Image(Arc<gpui::Image>),
}

pub(super) struct Attachment {
    pub path: PathBuf,
    content: Content,
    /// Written under `$CODEX_HOME/attachments`: pasted text through the app-server's `fs/*`
    /// like the original ([`save_text`]), a pasted image by this process ([`persist`]). Both
    /// go with the chip. A picked or dropped file stays where the user keeps it.
    persisted: bool,
}

impl Attachment {
    pub fn file(path: PathBuf) -> Self {
        Self {
            path,
            content: Content::File,
            persisted: false,
        }
    }

    /// A dropped or picked file: an image shows its thumbnail (its bytes stay where they
    /// are), anything else its chip.
    fn dropped(path: PathBuf) -> Self {
        let format = path.extension().and_then(|e| e.to_str()).and_then(|e| {
            gpui::ImageFormat::from_mime_type(&format!("image/{}", e.to_ascii_lowercase()))
        });
        match format.and_then(|f| Some((f, fs::read(&path).ok()?))) {
            Some((format, bytes)) => Self {
                path,
                content: Content::Image(Arc::new(gpui::Image::from_bytes(format, bytes))),
                persisted: false,
            },
            None => Self::file(path),
        }
    }
}

/// `$CODEX_HOME/attachments` (default `~/.codex`), the original's attachments root: the
/// recording (`refs/interactions/s017.json`) holds the host's codex home.
fn attachments_root() -> io::Result<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .map(|root| root.join("attachments"))
        .ok_or_else(|| io::Error::other("CODEX_HOME and HOME are unset"))
}

/// The original's `pasted-text-attachments.json` under the attachments root: every pasted
/// text file, those waiting for removal and the first 2,000 UTF-16 units of each text. Read
/// once per process, written through the app-server after every change.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registry {
    attachment_paths: Vec<String>,
    pending_removal_paths: Vec<String>,
    #[serde(default)]
    text_excerpts_by_path: serde_json::Map<String, Value>,
}

const REGISTRY_FILE: &str = "pasted-text-attachments.json";
static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

async fn request(bridge: &Bridge, method: &str, params: Value) -> Result<Value> {
    bridge
        .request(method, params)
        .await
        .context(method.to_owned())
}

async fn load_registry(bridge: &Bridge, root: &Path) {
    if REGISTRY.lock().is_some() {
        return;
    }
    // ponytail: every read error counts as a missing registry; the original only ENOENT.
    let registry = request(
        bridge,
        "fs/readFile",
        json!({ "path": root.join(REGISTRY_FILE) }),
    )
    .await
    .ok()
    .and_then(|v| BASE64.decode(v["dataBase64"].as_str()?).ok())
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default();
    REGISTRY.lock().get_or_insert(registry);
}

/// Applies `edit` and writes the whole registry, like the original's `writeState`.
async fn write_registry(
    bridge: &Bridge,
    root: &Path,
    edit: impl FnOnce(&mut Registry),
) -> Result<()> {
    let json = {
        let mut guard = REGISTRY.lock();
        let registry = guard.get_or_insert_default();
        edit(registry);
        serde_json::to_string(registry)?
    };
    let params = json!({ "path": root.join(REGISTRY_FILE), "dataBase64": BASE64.encode(json) });
    request(bridge, "fs/writeFile", params).await.map(drop)
}

/// The original's `createPastedTextAttachment`: a new directory, the file, the registry.
async fn save_text(bridge: Arc<Bridge>, text: String) -> Result<Attachment> {
    let root = attachments_root()?;
    load_registry(&bridge, &root).await;
    let directory = root.join(uuid::Uuid::new_v4().to_string());
    let params = json!({ "path": directory, "recursive": true });
    request(&bridge, "fs/createDirectory", params).await?;
    let path = directory.join("Pasted text.txt");
    let params = json!({ "path": path, "dataBase64": BASE64.encode(&text) });
    request(&bridge, "fs/writeFile", params).await?;
    let units: Vec<u16> = text.trim().encode_utf16().take(2000).collect();
    let excerpt = String::from_utf16_lossy(&units);
    let key = path.to_string_lossy().into_owned();
    write_registry(&bridge, &root, |r| {
        r.attachment_paths.push(key.clone());
        r.text_excerpts_by_path.insert(key, excerpt.into());
    })
    .await?;
    Ok(Attachment {
        path,
        content: Content::Text(text),
        persisted: true,
    })
}

/// The original's `remove`: the path goes pending, the file goes, the path goes.
async fn remove_text(bridge: Arc<Bridge>, path: PathBuf) -> Result<()> {
    let root = attachments_root()?;
    let key = path.to_string_lossy().into_owned();
    write_registry(&bridge, &root, |r| {
        r.pending_removal_paths.push(key.clone())
    })
    .await?;
    request(&bridge, "fs/remove", json!({ "path": path, "force": true })).await?;
    write_registry(&bridge, &root, |r| {
        r.attachment_paths.retain(|p| *p != key);
        r.pending_removal_paths.retain(|p| *p != key);
        r.text_excerpts_by_path.remove(&key);
    })
    .await
}

/// A pasted image, written by this process like the original's `persistImageFileToTemp`.
fn persist(image: Arc<gpui::Image>) -> io::Result<Attachment> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let unique = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let directory = attachments_root()?.join(unique);
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let path = directory.join(format!("image.{}", image.format.extension()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(&image.bytes) {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    Ok(Attachment {
        path,
        content: Content::Image(image),
        persisted: true,
    })
}

impl Composer {
    pub(super) fn save_paste(&mut self, content: Content, cx: &mut Context<Self>) {
        self.pending_attachments += 1;
        self.attachment_error = None;
        let (error_message, save): (_, Task<Result<Attachment>>) = match content {
            Content::Text(text) => (
                "Large pasted text could not be attached",
                match cx.try_global::<AppServer>() {
                    Some(AppServer(bridge)) => {
                        let bridge = bridge.clone();
                        cx.foreground_executor().spawn(save_text(bridge, text))
                    }
                    None => Task::ready(Err(anyhow!("no app-server"))),
                },
            ),
            Content::Image(image) => (
                "Image could not be attached",
                cx.background_executor()
                    .spawn(async move { persist(image).map_err(Into::into) }),
            ),
            Content::File => unreachable!("only clipboard contents are persisted"),
        };
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                this.pending_attachments -= 1;
                match result {
                    Ok(attachment) => this.attachments.push(attachment),
                    Err(error) => {
                        eprintln!("codex-gpui: {error_message}: {error}");
                        this.attachment_error = Some(error_message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn add_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| {
                    this.attachments
                        .extend(paths.into_iter().map(Attachment::file));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Files dropped on the composer's drop target (CP-030): each becomes an attachment at
    /// its own path.
    pub fn drop_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.attachments
            .extend(paths.iter().cloned().map(Attachment::dropped));
        self.set_drag_active(false, cx);
        cx.notify();
    }

    pub(super) fn attachments_row(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .pt(px(8.))
            .px(px(8.))
            .pb(px(6.))
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .when(!self.attachments.is_empty(), |d| {
                d.pt(px(12.)).px(px(12.)).gap(px(12.)).items_end()
            })
            .children(
                self.attachments
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| matches!(a.content, Content::Image(_)))
                    .chain(
                        self.attachments
                            .iter()
                            .enumerate()
                            .filter(|(_, a)| !matches!(a.content, Content::Image(_))),
                    )
                    .map(|(i, attachment)| {
                        match &attachment.content {
                            Content::Text(content) => {
                                return self.text_attachment(i, content, t, cx);
                            }
                            Content::Image(image) => {
                                return div()
                                    .relative()
                                    .size(px(122.))
                                    .rounded(px(8.))
                                    .overflow_hidden()
                                    .child(
                                        gpui::img(image.clone())
                                            .size_full()
                                            .object_fit(gpui::ObjectFit::Cover),
                                    )
                                    .child(self.remove_paste_button(i, t, cx))
                                    .into_any_element();
                            }
                            Content::File => {}
                        }
                        let name = attachment
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        div()
                            .h(px(28.))
                            .pl(px(8.))
                            .pr(px(2.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .relative()
                            .child(shapes::decoration(deco(10., t.text_alpha_5, vec![])))
                            .child(label(name, t.text))
                            .child(
                                pill(("remove-attachment", i), t)
                                    .size(px(22.))
                                    .justify_center()
                                    .child(icon("close", 12., t.text_tertiary))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.attachments.remove(i);
                                        cx.notify();
                                    })),
                            )
                            .into_any_element()
                    }),
            )
    }

    fn remove_paste_button(&self, i: usize, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(("remove-attachment", i))
            .absolute()
            .top(px(4.))
            .right(px(4.))
            .size(px(16.))
            .rounded_full()
            .bg(t.text)
            .flex()
            .items_center()
            .justify_center()
            .child(icon("close", 10., t.surface))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.remove_paste(i, cx);
            }))
    }

    fn remove_paste(&mut self, i: usize, cx: &mut Context<Self>) -> Content {
        let attachment = self.attachments.remove(i);
        match (&attachment.content, attachment.persisted) {
            (Content::Text(_), true) => {
                if let Some(AppServer(bridge)) = cx.try_global::<AppServer>() {
                    let (bridge, path) = (bridge.clone(), attachment.path.clone());
                    cx.foreground_executor()
                        .spawn(async move {
                            if let Err(error) = remove_text(bridge, path).await {
                                eprintln!("codex-gpui: pasted text not removed: {error:#}");
                            }
                        })
                        .detach();
                }
            }
            (Content::Image(_), true) => {
                let path = attachment.path.clone();
                cx.background_executor()
                    .spawn(async move {
                        let _ = fs::remove_file(&path);
                        if let Some(parent) = path.parent() {
                            let _ = fs::remove_dir(parent);
                        }
                    })
                    .detach();
            }
            _ => {}
        }
        cx.notify();
        attachment.content
    }

    fn text_attachment(
        &self,
        i: usize,
        content: &str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preview: String = content.trim().chars().take(79).collect();
        div()
            .w(px(224.))
            .h(px(53.))
            .relative()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(6.))
            .pr(px(24.))
            .rounded(px(8.))
            .bg(if t.appearance == Appearance::Dark {
                gpui::rgba(0x2d2d2df5).into()
            } else {
                t.surface
            })
            .border(px(0.5))
            .border_color(t.text_alpha_5)
            .child(
                div()
                    .size(px(40.))
                    .flex_none()
                    .rounded(px(8.))
                    .bg(t.surface)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(label("≡", t.text_secondary)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        label(format!("{preview}…"), t.text)
                            .overflow_hidden()
                            .text_ellipsis(),
                    )
                    .child(
                        div()
                            .id(("restore-paste", i))
                            .child(label("Show in text field ›", t.text_secondary))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Content::Text(text) = this.remove_paste(i, cx) {
                                    this.insert(&text, window, cx);
                                    window.focus(&this.focus_handle, cx);
                                }
                            })),
                    ),
            )
            .child(self.remove_paste_button(i, t, cx))
            .into_any_element()
    }
}
