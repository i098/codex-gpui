use super::*;
use gpui::StyledImage as _;
use std::{
    fs,
    io::{self, Write},
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
}

impl Attachment {
    pub fn file(path: PathBuf) -> Self {
        Self {
            path,
            content: Content::File,
        }
    }
}

fn persist(content: Content) -> io::Result<Attachment> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| io::Error::other("CODEX_HOME and HOME are unset"))?;
    let unique = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let directory = root.join("attachments").join(unique);
    let (name, bytes) = match &content {
        Content::Text(text) => ("Pasted text.txt".to_owned(), text.as_bytes()),
        Content::Image(image) => (
            format!("image.{}", image.format.extension()),
            image.bytes.as_slice(),
        ),
        Content::File => unreachable!("only clipboard contents are persisted"),
    };
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let path = directory.join(name);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(bytes) {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    Ok(Attachment { path, content })
}

impl Composer {
    pub(super) fn save_paste(&mut self, content: Content, cx: &mut Context<Self>) {
        self.pending_attachments += 1;
        self.attachment_error = None;
        let error_message = match content {
            Content::Text(_) => "Large pasted text could not be attached",
            _ => "Image could not be attached",
        };
        let save = cx
            .background_executor()
            .spawn(async move { persist(content) });
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
        cx.background_executor()
            .spawn(async move {
                let _ = fs::remove_file(&attachment.path);
                if let Some(parent) = attachment.path.parent() {
                    let _ = fs::remove_dir(parent);
                }
            })
            .detach();
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
