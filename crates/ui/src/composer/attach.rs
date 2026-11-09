use super::*;
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
    ContextFile,
    Text(String),
    Image(Arc<gpui::Image>),
}

pub(super) struct Attachment {
    pub path: PathBuf,
    content: Content,
    /// Pasted content belongs to the chip. Connected composers save it on their app-server;
    /// offline image pastes use [`persist`]. Picked and dropped files remain user-owned.
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

    pub(super) fn is_context_file(&self) -> bool {
        matches!(self.content, Content::ContextFile)
    }

    fn image_format(path: &Path) -> Option<gpui::ImageFormat> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        gpui::ImageFormat::from_mime_type(&format!("image/{extension}"))
    }

    fn is_image_path(path: &Path) -> bool {
        Self::image_format(path).is_some()
    }

    /// A dropped or picked file: an image shows its thumbnail (its bytes stay where they
    /// are), anything else its chip.
    async fn dropped(path: PathBuf, bridge: Option<Arc<Bridge>>) -> Result<Self> {
        let Some(format) = Self::image_format(&path) else {
            return Ok(Self::file(path));
        };
        let bytes = match bridge {
            Some(bridge) => {
                let result = request(&bridge, "fs/readFile", json!({ "path": path })).await?;
                BASE64.decode(
                    result["dataBase64"]
                        .as_str()
                        .context("missing image data")?,
                )?
            }
            None => fs::read(&path)?,
        };
        Ok(Self {
            path,
            content: Content::Image(Arc::new(gpui::Image::from_bytes(format, bytes))),
            persisted: false,
        })
    }
}

/// `$CODEX_HOME` (default `~/.codex`).
fn codex_home() -> io::Result<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| io::Error::other("CODEX_HOME and HOME are unset"))
}

/// `$CODEX_HOME/attachments`, the original's attachments root: the recording
/// (`refs/interactions/s017.json`) holds the host's codex home.
fn attachments_root() -> io::Result<PathBuf> {
    codex_home().map(|root| root.join("attachments"))
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
static REGISTRIES: Mutex<Vec<(std::sync::Weak<Bridge>, Registry)>> = Mutex::new(Vec::new());

async fn request(bridge: &Bridge, method: &str, params: Value) -> Result<Value> {
    bridge
        .request(method, params)
        .await
        .context(method.to_owned())
}

async fn load_registry(bridge: &Arc<Bridge>, root: &Path) {
    let connection = Arc::downgrade(bridge);
    if REGISTRIES
        .lock()
        .iter()
        .any(|(key, _)| key.ptr_eq(&connection))
    {
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
    let mut registries = REGISTRIES.lock();
    registries.retain(|(key, _)| key.strong_count() != 0);
    if !registries.iter().any(|(key, _)| key.ptr_eq(&connection)) {
        registries.push((connection, registry));
    }
}

/// Applies `edit` and writes the whole registry, like the original's `writeState`.
async fn write_registry(
    bridge: &Arc<Bridge>,
    root: &Path,
    edit: impl FnOnce(&mut Registry),
) -> Result<()> {
    load_registry(bridge, root).await;
    let connection = Arc::downgrade(bridge);
    let json = {
        let mut registries = REGISTRIES.lock();
        let registry = &mut registries
            .iter_mut()
            .find(|(key, _)| key.ptr_eq(&connection))
            .context("attachment registry is unavailable")?
            .1;
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

/// The original's `createLocalFile`: a file on the app-server's host is copied by `fs/copy`
/// into a new directory under the attachments root and attached there. When the host cannot
/// see `source` but this machine can read it, the bytes go over `fs/writeFile` instead.
async fn copy_file(bridge: Arc<Bridge>, source: PathBuf) -> Result<Attachment> {
    let name = source
        .file_name()
        .context("the file has no name")?
        .to_owned();
    let directory = attachments_root()?.join(uuid::Uuid::new_v4().to_string());
    let path = directory.join(name);
    let params = json!({ "path": directory, "recursive": true });
    request(&bridge, "fs/createDirectory", params).await?;
    let params = json!({ "sourcePath": source, "destinationPath": path });
    if let Err(copy_error) = request(&bridge, "fs/copy", params).await {
        let Ok(bytes) = fs::read(&source) else {
            return Err(copy_error);
        };
        let params = json!({ "path": path, "dataBase64": BASE64.encode(bytes) });
        request(&bridge, "fs/writeFile", params).await?;
    }
    Ok(Attachment {
        path,
        content: Content::File,
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

async fn persist_on_host(bridge: Arc<Bridge>, image: Arc<gpui::Image>) -> Result<Attachment> {
    let home = bridge
        .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
        .await?
        .codex_home;
    let directory = PathBuf::from(home)
        .join("attachments")
        .join(uuid::Uuid::new_v4().to_string());
    request(
        &bridge,
        "fs/createDirectory",
        json!({ "path": directory, "recursive": true }),
    )
    .await?;
    let path = directory.join(format!("image.{}", image.format.extension()));
    request(
        &bridge,
        "fs/writeFile",
        json!({ "path": path, "dataBase64": BASE64.encode(&image.bytes) }),
    )
    .await?;
    Ok(Attachment {
        path,
        content: Content::Image(image),
        persisted: true,
    })
}

impl Composer {
    pub fn attachment_generation(&self) -> u64 {
        self.attachment_generation
    }

    pub fn attach_png(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.save_paste(
            Content::Image(Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                bytes,
            ))),
            cx,
        );
    }

    pub(super) fn save_paste(&mut self, content: Content, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let (detail, save): (_, Task<Result<Attachment>>) = match content {
            Content::Text(text) => (
                "large pasted text",
                match self.bridge.clone() {
                    Some(bridge) => cx.foreground_executor().spawn(save_text(bridge, text)),
                    None => Task::ready(Err(anyhow!("no app-server"))),
                },
            ),
            Content::Image(image) => (
                "image",
                match bridge {
                    Some(bridge) => cx
                        .foreground_executor()
                        .spawn(persist_on_host(bridge, image)),
                    None => cx
                        .background_executor()
                        .spawn(async move { persist(image).map_err(Into::into) }),
                },
            ),
            Content::File | Content::ContextFile => {
                unreachable!("only clipboard contents are persisted")
            }
        };
        self.queue_attachment(detail, save, cx);
    }

    fn queue_attachment(
        &mut self,
        detail: &'static str,
        save: Task<Result<Attachment>>,
        cx: &mut Context<Self>,
    ) {
        self.pending_attachments += 1;
        self.attachment_error = None;
        let generation = self.attachment_generation;
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.attachment_generation {
                    return;
                }
                this.pending_attachments -= 1;
                match result {
                    Ok(attachment) => this.attachments.push(attachment),
                    Err(error) => {
                        eprintln!("codex-gpui: {detail} could not be attached: {error}");
                        this.attachment_error = Some("Unable to upload file");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn add_files(&mut self, cx: &mut Context<Self>) {
        let generation = self.attachment_generation;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| {
                    if generation != this.attachment_generation {
                        return;
                    }
                    this.drop_files(&paths, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Files dropped on the composer's drop target (CP-030): each becomes an attachment at
    /// its own path.
    pub fn drop_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            let bridge = self.bridge.clone();
            let path = path.clone();
            let is_image = Attachment::is_image_path(&path);
            let save = cx.foreground_executor().spawn(async move {
                match bridge {
                    Some(bridge) if !is_image => copy_file(bridge, path).await,
                    bridge => Attachment::dropped(path, bridge).await,
                }
            });
            self.queue_attachment("image", save, cx);
        }
        self.set_drag_active(false, cx);
        cx.notify();
    }

    /// Adds a context-file card without changing the draft or its selection.
    pub fn add_context_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.attachments.iter().any(|attachment| {
            matches!(attachment.content, Content::File | Content::ContextFile)
                && attachment.path == path
        }) {
            return;
        }
        self.attachments.push(Attachment {
            path,
            content: Content::ContextFile,
            persisted: false,
        });
        cx.notify();
    }

    /// A downloaded Library file (the original's `createFileAttachment`): written to a new
    /// directory under the attachments root and attached there. Returns that path.
    pub fn attach_download(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf> {
        // The name comes from the server: only its last component is kept.
        let name = Path::new(name)
            .file_name()
            .context("the file has no name")?
            .to_owned();
        let directory = attachments_root()?.join(uuid::Uuid::new_v4().to_string());
        let path = directory.join(name);
        let bridge = self.bridge.clone();
        let target = path.clone();
        let save: Task<Result<Attachment>> = cx.foreground_executor().spawn(async move {
            match &bridge {
                Some(bridge) => {
                    let params = json!({ "path": directory, "recursive": true });
                    request(bridge, "fs/createDirectory", params).await?;
                    let params = json!({ "path": target, "dataBase64": BASE64.encode(&bytes) });
                    request(bridge, "fs/writeFile", params).await?;
                }
                None => {
                    fs::create_dir_all(&directory)?;
                    fs::write(&target, &bytes)?;
                }
            }
            let mut attachment = Attachment::dropped(target, bridge).await?;
            attachment.persisted = true;
            Ok(attachment)
        });
        self.queue_attachment("Library file", save, cx);
        Ok(path)
    }

    /// Whether an attachment sits at `path`.
    pub fn has_attachment(&self, path: &Path) -> bool {
        self.attachments.iter().any(|a| a.path == path)
    }

    /// Removes the attachment at `path`, as its chip's remove button does.
    pub fn remove_attachment_at(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(index) = self.attachments.iter().position(|a| a.path == path) {
            self.remove_attachment(index, cx);
            cx.notify();
        }
    }

    fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.remove_paste(index, cx);
        }
    }

    /// The status line above the editor and the reason a send is blocked: `Files uploading…`
    /// while a paste saves, the upload failure (`composer.submit.waitForFileUploads`,
    /// `composer.fileAttachment.uploadError`), or images for a text-only model
    /// (`composer.submit.imageInputsUnsupported`).
    pub fn upload_status(&self) -> Option<&'static str> {
        if self.pending_attachments > 0 {
            Some("Files uploading…")
        } else if self.attachment_error.is_some() {
            self.attachment_error
        } else if self.current_model().is_some_and(|model| !model.images)
            && self
                .attachments
                .iter()
                .any(|a| matches!(a.content, Content::Image(_)))
        {
            Some("Remove images or switch models to send this message")
        } else {
            None
        }
    }

    pub(super) fn attachments_row(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .pt(px(8.))
            .px(px(8.))
            .pb(px(6.))
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .when(
                !self.attachments.is_empty() || !self.diff_comments.is_empty(),
                |d| {
                    d.pt(px(12.))
                        .px(px(12.))
                        .pb(px(10.))
                        .gap(px(12.))
                        .items_end()
                },
            )
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
                            Content::ContextFile => {
                                return self.context_file_card(i, attachment, t, cx);
                            }
                            Content::Text(content) => {
                                return self.text_attachment(i, content, t, cx);
                            }
                            Content::Image(image) => {
                                let context = crate::image_context::ImageContext {
                                    image: image.clone(),
                                    path: Some(attachment.path.clone()),
                                };
                                let surface = div()
                                    .relative()
                                    .size_full()
                                    .child(shapes::decoration(deco(
                                        0.,
                                        t.surface_secondary,
                                        vec![],
                                    )))
                                    .child(image_thumbnail(image.clone(), t));
                                return self
                                    .attachment_surface(i, attachment, px(122.), surface, t, cx)
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, window, cx| {
                                                this.menu = None;
                                                this.context_spelling = None;
                                                this.context_menu =
                                                    Some(context.menu(event.position, window, cx));
                                                this.context_image = Some(context.clone());
                                                cx.stop_propagation();
                                                cx.notify();
                                            },
                                        ),
                                    )
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
                                        this.remove_attachment(i, cx)
                                    })),
                            )
                            .into_any_element()
                    }),
            )
            .when(!self.diff_comments.is_empty(), |d| {
                d.child(self.diff_comment_attachment(t, cx))
            })
    }

    fn context_file_card(
        &self,
        i: usize,
        attachment: &Attachment,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = attachment
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let code = attachment
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                [
                    "py", "rs", "js", "jsx", "ts", "tsx", "go", "c", "cpp", "h", "java", "swift",
                    "json", "toml", "yaml", "yml", "sh", "css", "html",
                ]
                .iter()
                .any(|suffix| ext.eq_ignore_ascii_case(suffix))
            });
        let surface = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(shapes::decoration(deco(0., t.surface_secondary, vec![])))
            .child(
                div()
                    .h(px(90.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("context-file-document", 24., t.text.opacity(0.65)).runtime()),
            )
            .child(
                div()
                    .relative()
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(8.))
                    .child(shapes::decoration(deco(0., t.surface, vec![])))
                    .child(
                        icon(
                            if code {
                                "context-file-code"
                            } else {
                                "context-file-document"
                            },
                            16.,
                            t.text,
                        )
                        .runtime(),
                    )
                    .child(
                        text(name, 12., 16., 430., t.text)
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis(),
                    ),
            );
        self.attachment_surface(i, attachment, px(160.), surface, t, cx)
            .into_any_element()
    }

    /// The original's `composer-attachment-surface`: `content` in a squircle clip, an inset ring
    /// that strengthens on hover, and a remove button shown only on hover or focus.
    fn attachment_surface(
        &self,
        i: usize,
        attachment: &Attachment,
        width: Pixels,
        content: Div,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let name = attachment
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let remove_label = format!("Remove {name}");
        let group: SharedString = format!("attachment-surface-{i}").into();
        let surface_decoration = |color, fill| {
            shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(10.)),
                corner: shapes::Corner::Squircle,
                fill,
                shadows: vec![shapes::Shadow {
                    color,
                    offset: point(px(0.), px(0.)),
                    blur: px(0.),
                    spread: px(1.),
                    inset: true,
                }],
                ..Default::default()
            })
        };
        div()
            .id(("attachment-surface", i))
            .group(group.clone())
            .role(gpui::Role::Group)
            .aria_label(name)
            .relative()
            .w(width)
            .h(px(122.))
            .flex_none()
            .child(crate::rounded_clip(
                px(10.),
                shapes::Corner::Squircle,
                content,
            ))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .group_hover(group.clone(), |s| s.opacity(0.))
                    .child(surface_decoration(t.border, None)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
                    .child(surface_decoration(t.border_strong, Some(t.text_alpha_5))),
            )
            .child(
                self.remove_paste_button(i, t, cx)
                    .role(gpui::Role::Button)
                    .focusable()
                    .tab_index(0)
                    .focus_visible(crate::focus::trigger_focus)
                    .aria_label(remove_label)
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.remove_paste(i, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .top(px(-2.))
                    .right(px(-2.))
                    .shadow(vec![
                        gpui::BoxShadow {
                            color: t.surface,
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(2.),
                            inset: false,
                        },
                        gpui::BoxShadow {
                            color: t.shadow_sm,
                            offset: point(px(0.), px(1.)),
                            blur_radius: px(2.),
                            spread_radius: -px(1.),
                            inset: false,
                        },
                    ])
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .focus(|s| s.opacity(1.)),
            )
    }

    fn remove_paste_button(&self, i: usize, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
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
                if let Some(bridge) = self.bridge.clone() {
                    let path = attachment.path.clone();
                    cx.foreground_executor()
                        .spawn(async move {
                            if let Err(error) = remove_text(bridge, path).await {
                                eprintln!("codex-gpui: pasted text not removed: {error:#}");
                            }
                        })
                        .detach();
                }
            }
            (Content::Image(_) | Content::File, true) => {
                let path = attachment.path.clone();
                if let Some(bridge) = self.bridge.clone() {
                    cx.foreground_executor()
                        .spawn(async move {
                            if let Err(error) = request(
                                &bridge,
                                "fs/remove",
                                json!({ "path": path, "force": true }),
                            )
                            .await
                            {
                                eprintln!("codex-gpui: pasted image not removed: {error:#}");
                            }
                        })
                        .detach();
                } else {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = fs::remove_file(&path);
                            if let Some(parent) = path.parent() {
                                let _ = fs::remove_dir(parent);
                            }
                        })
                        .detach();
                }
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

/// An image's decoded premultiplied pixels and its last raster: box, scale, card color.
type Thumbnail = (
    image::RgbaImage,
    Option<(gpui::Bounds<Pixels>, f32, Hsla, Arc<gpui::RenderImage>)>,
);

/// The image card's `<img class="size-full object-cover">` in its `overflow-hidden` squircle
/// `bg-surface-secondary` span, rasterized as Chromium does (Skia bilinear inside the
/// anti-aliased clip, over the composer card); a GPU-scaled sprite blends with other weights.
fn image_thumbnail(image: Arc<gpui::Image>, t: &Theme) -> AnyElement {
    if !cfg!(target_os = "macos") {
        return gpui::img(image)
            .size_full()
            .rounded(px(10.))
            .object_fit(gpui::ObjectFit::Cover)
            .into_any_element();
    }
    thread_local! {
        // shortcut: entries outlive removed attachments (one decode and raster each); evict
        // by the live attachments if long sessions paste many images.
        static CACHE: std::cell::RefCell<std::collections::HashMap<u64, Thumbnail>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    let (fill, under) = (t.surface_secondary, t.composer_surface);
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let scale = window.scale_factor();
            let snap = |v: Pixels| px((f32::from(v) * scale).round() / scale);
            let at = gpui::Bounds::from_corners(
                point(snap(bounds.left()), snap(bounds.top())),
                point(snap(bounds.right()), snap(bounds.bottom())),
            );
            let raster = CACHE.with_borrow_mut(|cache| {
                let entry = match cache.entry(image.id) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let mut pixels = image::load_from_memory(&image.bytes).ok()?.to_rgba8();
                        shapes::premultiply(&mut pixels);
                        entry.insert((pixels, None))
                    }
                };
                if let Some((b, s, u, raster)) = &entry.1
                    && (*b, *s, *u) == (bounds, scale, under)
                {
                    return Some(raster.clone());
                }
                let pixels = &entry.0;
                let (w, h) = (pixels.width() as f32, pixels.height() as f32);
                let cover =
                    (f32::from(bounds.size.width) / w).max(f32::from(bounds.size.height) / h);
                let drawn = gpui::size(px(w * cover), px(h * cover));
                let destination = gpui::Bounds::new(
                    bounds.origin
                        + point(
                            (bounds.size.width - drawn.width) / 2.,
                            (bounds.size.height - drawn.height) / 2.,
                        ),
                    drawn,
                );
                let raster = Arc::new(shapes::raster_decorated_images(
                    &[(pixels, destination)],
                    bounds,
                    &shapes::Decoration {
                        radius: Corners::all(px(10.)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(fill),
                        ..Default::default()
                    },
                    shapes::ImageComposition::Direct(under),
                    window,
                )?);
                if let Some((.., old)) = entry.1.replace((bounds, scale, under, raster.clone())) {
                    window.drop_image(old).ok();
                }
                Some(raster)
            });
            if let Some(raster) = raster {
                window
                    .paint_image(at, at, Corners::default(), raster, 0, false)
                    .ok();
            }
        },
    )
    .size_full()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    #[test]
    fn context_cards_preserve_the_draft_and_deduplicate_files() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = gpui::HeadlessAppContext::new(textsys::text_system());
        let composer = cx.new(|cx| Composer::new("Work with Codex", true, cx));
        composer.update(&mut cx, |composer, cx| {
            composer.set_text("before after", cx);
            composer.selected_range = 7..7;
            composer.add_context_file("/work/main.rs".into(), cx);
            composer.add_context_file("/work/main.rs".into(), cx);
            composer
                .attachments
                .push(Attachment::file("/work/attached.rs".into()));
            composer.add_context_file("/work/attached.rs".into(), cx);
            assert_eq!(composer.prompt(), "before after");
            assert_eq!(composer.selected_range, 7..7);
            assert_eq!(
                composer
                    .attachments
                    .iter()
                    .map(|a| a.path.as_path())
                    .collect::<Vec<_>>(),
                [Path::new("/work/main.rs"), Path::new("/work/attached.rs")]
            );
        });
    }

    #[cfg(unix)] // POSIX path fixtures
    #[test]
    fn pasted_text_keeps_its_host_after_another_window_becomes_active() {
        let recording = tempfile::NamedTempFile::new().unwrap();
        let rows = [
            (
                "fs/readFile",
                json!({"dataBase64": BASE64.encode(r#"{"attachmentPaths":["remote-only"],"pendingRemovalPaths":[]}"#)}),
            ),
            ("fs/createDirectory", json!({})),
            ("fs/writeFile", json!({})),
            ("fs/writeFile", json!({})),
        ];
        let mut lines = String::new();
        for (index, (method, result)) in rows.into_iter().enumerate() {
            lines.push_str(&format!(
                "{}\n",
                json!({"dir":"send","msg":{"id":index,"method":method,"params":{}}})
            ));
            lines.push_str(&format!(
                "{}\n",
                json!({"dir":"recv","msg":{"id":index,"result":result}})
            ));
        }
        std::fs::write(recording.path(), lines).unwrap();
        let (bridge, _) = Bridge::replay(recording.path()).unwrap();
        let bridge = Arc::new(bridge);
        let unrelated = tempfile::NamedTempFile::new().unwrap();
        let (other, _) = Bridge::replay(unrelated.path()).unwrap();
        let mut cx = gpui::HeadlessAppContext::new(textsys::text_system());
        cx.update(|cx| cx.set_global(crate::screens::settings::AppServer(bridge.clone())));
        let composer = cx.new(|cx| Composer::new("Work with Codex", true, cx));
        cx.update(|cx| cx.set_global(crate::screens::settings::AppServer(Arc::new(other))));
        composer.update(&mut cx, |composer, cx| {
            composer.save_paste(Content::Text("remote paste".into()), cx);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if composer.read_with(&cx, |composer, _| composer.pending_attachments == 0) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "paste did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        composer.read_with(&cx, |composer, _| {
            assert!(composer.attachment_error.is_none(), "{:?}", composer.attachment_error);
            assert!(matches!(&composer.attachments[0].content, Content::Text(text) if text == "remote paste"));
        });
        let registries = REGISTRIES.lock();
        let registry = &registries
            .iter()
            .find(|(key, _)| key.ptr_eq(&Arc::downgrade(&bridge)))
            .unwrap()
            .1;
        assert!(
            registry
                .attachment_paths
                .iter()
                .any(|path| path == "remote-only")
        );
        assert!(
            registry
                .attachment_paths
                .iter()
                .any(|path| path.ends_with("Pasted text.txt"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_dropped_file_is_copied_on_the_host_into_the_attachments_root() {
        let recording = tempfile::NamedTempFile::new().unwrap();
        let rows = [
            (
                "fs/createDirectory",
                json!({"path": "d", "recursive": true}),
                json!({}),
            ),
            (
                "fs/copy",
                json!({"sourcePath": "/work/notes.txt", "destinationPath": "x"}),
                json!({}),
            ),
        ];
        let mut lines = String::new();
        for (index, (method, params, result)) in rows.into_iter().enumerate() {
            lines.push_str(&format!(
                "{}\n{}\n",
                json!({"dir":"send","msg":{"id":index,"method":method,"params":params}}),
                json!({"dir":"recv","msg":{"id":index,"result":result}})
            ));
        }
        std::fs::write(recording.path(), lines).unwrap();
        let (bridge, _) = Bridge::replay(recording.path()).unwrap();
        let mut cx = gpui::HeadlessAppContext::new(textsys::text_system());
        let done = Arc::new(Mutex::new(None));
        let slot = done.clone();
        let task = cx.foreground_executor().spawn(async move {
            *slot.lock() = Some(copy_file(Arc::new(bridge), "/work/notes.txt".into()).await);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while done.lock().is_none() {
            cx.run_until_parked();
            assert!(std::time::Instant::now() < deadline, "copy did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drop(task);
        let attachment = done.lock().take().unwrap().unwrap();
        assert!(attachment.persisted);
        assert!(matches!(attachment.content, Content::File));
        assert_eq!(attachment.path.file_name().unwrap(), "notes.txt");
        assert!(attachment.path.starts_with(attachments_root().unwrap()));
    }

    #[test]
    fn upload_states_and_chip_remove() {
        let mut cx = gpui::HeadlessAppContext::new(textsys::text_system());
        let composer = cx.new(|cx| Composer::new("Work with Codex", true, cx));
        composer.update(&mut cx, |composer, cx| {
            composer.save_paste(Content::Text("x".repeat(6000)), cx);
            assert_eq!(composer.upload_status(), Some("Files uploading…"));
        });
        cx.run_until_parked();
        composer.update(&mut cx, |composer, cx| {
            // No app-server: the save fails.
            assert_eq!(composer.upload_status(), Some("Unable to upload file"));
            composer
                .attachments
                .push(Attachment::file("/work/a.rs".into()));
            composer
                .attachments
                .push(Attachment::file("/work/b.rs".into()));
            composer.remove_attachment(0, cx);
            composer.remove_attachment(5, cx);
            assert_eq!(composer.attachments.len(), 1);
            assert_eq!(composer.attachments[0].path, Path::new("/work/b.rs"));
        });
    }

    #[test]
    fn text_only_model_blocks_images() {
        let mut cx = gpui::HeadlessAppContext::new(textsys::text_system());
        let composer = cx.new(|cx| Composer::new("Work with Codex", true, cx));
        composer.update(&mut cx, |composer, _| {
            let model = |id: &str, images| super::super::ModelOption {
                id: id.into(),
                label: id.into(),
                efforts: Vec::new(),
                images,
            };
            composer.models = vec![model("text", false), model("vision", true)];
            composer
                .attachments
                .push(Attachment::file("/work/notes.md".into()));
            assert_eq!(composer.upload_status(), None, "files do not need images");
            composer.attachments.push(Attachment {
                path: "/work/shot.png".into(),
                content: Content::Image(Arc::new(gpui::Image::empty())),
                persisted: false,
            });
            assert_eq!(
                composer.upload_status(),
                Some("Remove images or switch models to send this message")
            );
            composer.model = 1;
            assert_eq!(composer.upload_status(), None);
        });
    }
}
