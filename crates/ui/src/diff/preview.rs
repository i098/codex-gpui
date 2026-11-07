use super::*;
use gpui::{Bounds, Image, ImageFormat, RenderImage, SvgRenderer, canvas, point, size};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Debug)]
pub enum PreviewSource {
    Git {
        cwd: String,
        path: String,
        reference: String,
    },
    Worktree {
        cwd: String,
        path: String,
    },
}

#[derive(Clone)]
pub struct PreviewRequest {
    pub index: usize,
    pub side: usize,
    pub generation: u64,
    pub source: PreviewSource,
    pub page: usize,
    pub width: u32,
    /// Device px per CSS px at request time.
    pub scale: f32,
    pub pdf: bool,
    pub data: Option<Arc<Vec<u8>>>,
}

pub struct PreviewContent {
    image: Arc<RenderImage>,
    pixels: Arc<image::RgbaImage>,
    pages: usize,
    data: Option<Arc<Vec<u8>>>,
}

impl PreviewContent {
    pub fn decode(
        bytes: Arc<Vec<u8>>,
        mime: Option<&str>,
        request: &PreviewRequest,
        svg: SvgRenderer,
    ) -> anyhow::Result<Self> {
        if request.pdf {
            let (pixels, pages) =
                super::pdf::render(&bytes, request.page, request.width, request.scale)?;
            let mut bgra = pixels.clone();
            for pixel in bgra.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            return Ok(Self {
                image: Arc::new(RenderImage::new([image::Frame::new(bgra)])),
                pixels: Arc::new(pixels),
                pages,
                data: Some(bytes),
            });
        }
        let path = match &request.source {
            PreviewSource::Git { path, .. } | PreviewSource::Worktree { path, .. } => path,
        };
        let format = mime
            .and_then(ImageFormat::from_mime_type)
            .or_else(|| image_format(path))
            .ok_or_else(|| anyhow::anyhow!("Unknown image format"))?;
        let image = Image::from_bytes(format, Arc::unwrap_or_clone(bytes)).to_image_data(svg)?;
        let dimensions = image.size(0);
        let mut pixels = image::RgbaImage::from_raw(
            dimensions.width.0 as u32,
            dimensions.height.0 as u32,
            image
                .as_bytes(0)
                .ok_or_else(|| anyhow::anyhow!("Empty image"))?
                .to_vec(),
        )
        .ok_or_else(|| anyhow::anyhow!("Invalid image size"))?;
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        shapes::premultiply(&mut pixels);
        Ok(Self {
            image,
            pixels: Arc::new(pixels),
            pages: 1,
            data: None,
        })
    }
}

fn image_format(path: &str) -> Option<ImageFormat> {
    let extension = std::path::Path::new(path).extension()?.to_str()?;
    [
        ("png", ImageFormat::Png),
        ("jpg", ImageFormat::Jpeg),
        ("jpeg", ImageFormat::Jpeg),
        ("gif", ImageFormat::Gif),
        ("webp", ImageFormat::Webp),
        ("svg", ImageFormat::Svg),
        ("bmp", ImageFormat::Bmp),
        ("tif", ImageFormat::Tiff),
        ("tiff", ImageFormat::Tiff),
        ("ico", ImageFormat::Ico),
    ]
    .into_iter()
    .find_map(|(name, format)| extension.eq_ignore_ascii_case(name).then_some(format))
}

fn is_pdf(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

pub(super) fn enabled(path: &str, rich_preview: bool) -> bool {
    is_pdf(path)
        || image_format(path).is_some_and(|format| format != ImageFormat::Svg || rich_preview)
}

type Raster = Rc<RefCell<Option<(Bounds<Pixels>, f32, Hsla, Arc<RenderImage>)>>>;

struct Side {
    source: Option<PreviewSource>,
    loading: bool,
    error: bool,
    page: usize,
    content: Option<PreviewContent>,
    raster: Raster,
}

impl Side {
    fn new(source: Option<PreviewSource>) -> Self {
        Self {
            loading: source.is_some(),
            source,
            error: false,
            page: 1,
            content: None,
            raster: Rc::new(RefCell::new(None)),
        }
    }
}

pub(super) struct Preview {
    sides: [Side; 2],
    after_only: bool,
    pdf: bool,
    svg: bool,
    pager_focus: Option<[FocusHandle; 2]>,
}

impl Preview {
    pub(super) fn loaded(&self) -> bool {
        self.sides.iter().all(|side| !side.loading)
    }

    fn content_width(&self, available: Pixels) -> Pixels {
        let columns = if self.after_only { 1. } else { 2. };
        ((available - px(columns - 1.)) / columns - px(24.)).max(px(1.))
    }

    fn raster_width(&self, available: Pixels, scale: f32) -> u32 {
        let width = self.content_width(available);
        let width = if self.pdf { width.floor() } else { width };
        (f32::from(width) * scale).round() as u32
    }
}

impl DiffPanel {
    pub fn shows_rich_preview(&self) -> bool {
        self.rich_preview
    }

    pub fn preview_page(&self, index: usize, side: usize) -> Option<(usize, usize)> {
        let value = self.files.get(index)?.preview.as_ref()?.sides.get(side)?;
        (!value.loading).then_some((value.page, value.content.as_ref()?.pages))
    }

    pub(super) fn toggle_rich_preview(&mut self, cx: &mut Context<Self>) {
        self.rich_preview = !self.rich_preview;
        cx.emit(DiffRequest::Preference(
            "host.diffRichPreview",
            self.rich_preview.into(),
        ));
        cx.notify();
    }
    fn preview_sources(&self, index: usize) -> [Option<PreviewSource>; 2] {
        let Some(path) = self.file_path(index) else {
            return [None, None];
        };
        let file = &self.files[index];
        let cwd = if self.source == DiffSource::LastTurn {
            &self.cwd
        } else {
            self.file_root()
        };
        let (before, after) = match self.source {
            DiffSource::LastTurn | DiffSource::Uncommitted => (Some("head"), None),
            DiffSource::Unstaged => (Some("index"), None),
            DiffSource::Staged => (Some("head"), Some("index")),
            DiffSource::Branch | DiffSource::Commits => self
                .repository
                .as_ref()
                .map(|r| (r.before_ref.as_deref(), r.after_ref.as_deref()))
                .unwrap_or((None, None)),
            DiffSource::Cloud => return [None, None],
        };
        let old_path = file
            .raw
            .lines()
            .find_map(|line| line.strip_prefix("rename from "))
            .and_then(super::patch::unquote)
            .map(|path| path.into_owned())
            .unwrap_or_else(|| path.clone());
        let before = if file.raw_kind == Some(Kind::Added)
            || file
                .raw
                .lines()
                .any(|line| line.starts_with("new file mode ") || line == "--- /dev/null")
        {
            None
        } else {
            before.map(|reference| PreviewSource::Git {
                cwd: cwd.to_owned(),
                path: old_path,
                reference: reference.to_owned(),
            })
        };
        let after = if file.raw_kind == Some(Kind::Removed)
            || file
                .raw
                .lines()
                .any(|line| line.starts_with("deleted file mode ") || line == "+++ /dev/null")
        {
            None
        } else {
            Some(match after {
                Some(reference) => PreviewSource::Git {
                    cwd: cwd.to_owned(),
                    path,
                    reference: reference.to_owned(),
                },
                None => PreviewSource::Worktree {
                    cwd: cwd.to_owned(),
                    path,
                },
            })
        };
        [before, after]
    }

    pub(super) fn request_visible_previews(
        &mut self,
        width: Pixels,
        scale: f32,
        cx: &mut Context<Self>,
    ) {
        let first = if self.capped() {
            self.active_file
        } else {
            self.scroll.top_item()
        };
        let last = if self.capped() {
            first
        } else {
            self.scroll.bottom_item()
        };
        for index in first..=last.min(self.files.len().saturating_sub(1)) {
            let Some(file) = self.files.get(index) else {
                continue;
            };
            if !file.open || !enabled(&file.name, self.rich_preview) {
                continue;
            }
            if file.preview.is_some() {
                let preview = self.files[index].preview.as_mut().unwrap();
                let desired = preview.raster_width(width, scale);
                let pdf = preview.pdf;
                let reload = preview.sides.each_mut().map(|side| {
                    let reload = pdf
                        && !side.loading
                        && side
                            .content
                            .as_ref()
                            .is_some_and(|content| content.pixels.width() != desired);
                    if reload {
                        side.loading = true;
                    }
                    reload
                });
                for (side, reload) in reload.into_iter().enumerate() {
                    if reload {
                        self.request_preview(index, side, width, scale, cx);
                    }
                }
                continue;
            }
            let sources = self.preview_sources(index);
            if sources.iter().all(Option::is_none) {
                continue;
            }
            let after_only = sources[0].is_none() && sources[1].is_some();
            let pdf = is_pdf(&file.name);
            let preview = Preview {
                pdf,
                svg: image_format(&file.name) == Some(ImageFormat::Svg),
                sides: sources.map(Side::new),
                after_only,
                pager_focus: pdf.then(|| [cx.focus_handle(), cx.focus_handle()]),
            };
            self.files[index].preview = Some(preview);
            for side in 0..2 {
                self.request_preview(index, side, width, scale, cx);
            }
        }
    }

    fn request_preview(
        &self,
        index: usize,
        side: usize,
        width: Pixels,
        scale: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = self.files.get(index).and_then(|f| f.preview.as_ref()) else {
            return;
        };
        let value = &preview.sides[side];
        let Some(source) = value.source.clone() else {
            return;
        };
        cx.emit(DiffRequest::LoadPreview(PreviewRequest {
            index,
            side,
            generation: self.content_generation,
            source,
            page: value.page,
            width: preview.raster_width(width, scale),
            scale,
            pdf: preview.pdf,
            data: value.content.as_ref().and_then(|value| value.data.clone()),
        }));
    }

    pub fn preview_loaded(
        &mut self,
        request: &PreviewRequest,
        result: anyhow::Result<Option<PreviewContent>>,
        cx: &mut Context<Self>,
    ) {
        if request.generation != self.content_generation {
            return;
        }
        let Some(side) = self
            .files
            .get_mut(request.index)
            .and_then(|f| f.preview.as_mut())
            .and_then(|p| p.sides.get_mut(request.side))
        else {
            return;
        };
        if side.page != request.page {
            return;
        }
        side.loading = false;
        side.error = result.is_err();
        side.content = result.ok().flatten();
        *side.raster.borrow_mut() = None;
        cx.notify();
    }

    pub(super) fn preview(
        &self,
        index: usize,
        width: Pixels,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let file = self.files.get(index)?;
        if !enabled(&file.name, self.rich_preview) {
            return None;
        }
        let preview = file.preview.as_ref()?;
        let t = cx.theme();
        let available = preview.content_width(width);
        let render_side = |side: usize| {
            let value = &preview.sides[side];
            // `artifact-annotation`: only the current (after) side takes annotations.
            let path = (side == 1).then(|| self.file_path(index)).flatten();
            let body = side_body(
                preview,
                value,
                available,
                format!("diff-preview-{index}-{side}").into(),
                t,
                |content, image_width| {
                    path.map(|path| {
                        let page = artifact_annotations::Page {
                            path,
                            title: file.name.to_string(),
                            kind: if preview.pdf { "pdf" } else { "image" },
                            number: value.page,
                            count: content.pages,
                            size: comments::PageSize {
                                width: content.pixels.dimensions().0 as f32,
                                height: content.pixels.dimensions().1 as f32,
                            },
                            bounds: Default::default(),
                        };
                        self.artifact_layer(page, image_width, cx)
                    })
                    .unwrap_or_default()
                },
            );
            let pages = value.content.as_ref().map_or(0, |content| content.pages);
            div()
                .group("diff-pdf-preview")
                .flex_1()
                .min_w_0()
                .min_h(px(160.))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .p(px(12.))
                .child(body)
                .when(preview.pdf && pages > 1, |d| {
                    let button = |step: isize, label: &'static str, name: &'static str| {
                        let enabled = if step < 0 {
                            value.page > 1
                        } else {
                            value.page < pages
                        };
                        round_icon(name, 24., t)
                            .id(SharedString::from(format!("pdf-{index}-{side}-{step}")))
                            .role(gpui::Role::Button)
                            .aria_label(label)
                            .aria_disabled(!enabled)
                            .when(enabled, |d| d.focusable().cursor_pointer())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.turn_pdf_page(index, side, step, width, window, cx);
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.turn_pdf_page(index, side, step, width, window, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                    };
                    d.child(
                        div()
                            .id(("diff-pdf-pager", index * 2 + side))
                            .tab_index(-1)
                            .opacity(0.)
                            .group_hover("diff-pdf-preview", |s| s.opacity(1.))
                            .when_some(preview.pager_focus.as_ref(), |d, focus| {
                                d.track_focus(&focus[side])
                                    .when(focus[side].contains_focused(window, cx), |d| {
                                        d.opacity(1.)
                                    })
                            })
                            .absolute()
                            .right(px(2.))
                            .top(px(2.))
                            .flex()
                            .items_center()
                            .rounded_full()
                            .bg(t.surface)
                            .p(px(2.))
                            .gap(px(2.))
                            .child(button(-1, "Previous page", "request-chevron-left"))
                            .child(text(
                                format!("{}/{pages}", value.page),
                                14.,
                                20.,
                                400.,
                                t.text_secondary,
                            ))
                            .child(button(1, "Next page", "request-chevron-right")),
                    )
                })
        };
        let review = self
            .file_path(index)
            .and_then(|path| self.artifact_review_bar(&path, index, width, cx));
        Some(
            div()
                .w_full()
                .relative()
                .flex()
                .gap(px(1.))
                .when(!preview.after_only, |d| d.child(render_side(0)))
                .child(render_side(1))
                .children(review),
        )
    }

    /// Show `page` of a PDF preview side.
    pub(super) fn show_preview_page(
        &mut self,
        index: usize,
        side: usize,
        page: usize,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = self
            .files
            .get_mut(index)
            .and_then(|file| file.preview.as_mut())
        else {
            return;
        };
        let side_value = &mut preview.sides[side];
        if page == 0
            || page == side_value.page
            || side_value
                .content
                .as_ref()
                .is_none_or(|content| page > content.pages)
        {
            return;
        }
        side_value.page = page;
        side_value.loading = true;
        self.request_preview(index, side, width, window.scale_factor(), cx);
        cx.notify();
    }

    fn turn_pdf_page(
        &mut self,
        index: usize,
        side: usize,
        step: isize,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self
            .files
            .get(index)
            .and_then(|file| file.preview.as_ref())
            .and_then(|preview| preview.sides[side].page.checked_add_signed(step))
        else {
            return;
        };
        self.show_preview_page(index, side, page, width, window, cx);
    }
}

/// One side's picture at its fitted size, else its loading or failure label.
fn side_body(
    preview: &Preview,
    value: &Side,
    available: Pixels,
    id: SharedString,
    t: &Theme,
    layer: impl FnOnce(&PreviewContent, Pixels) -> Vec<gpui::AnyElement>,
) -> gpui::AnyElement {
    let Some(content) = value.content.as_ref().filter(|_| !value.loading) else {
        let label = if value.loading {
            if value.page > 1 {
                "Loading page…"
            } else {
                "Loading preview…"
            }
        } else if preview.pdf && value.error {
            "Unable to render PDF preview"
        } else if preview.pdf {
            "No PDF preview"
        } else {
            "No image"
        };
        return text(
            label,
            if value.loading { 12. } else { 14. },
            20.,
            400.,
            t.text_secondary,
        )
        .into_any_element();
    };
    let original = content.pixels.dimensions();
    let image_width = if preview.pdf || preview.svg {
        available
    } else {
        available.min(px(original.0 as f32))
    };
    let canvas_width = if preview.pdf {
        image_width.floor()
    } else {
        image_width
    };
    let padding = if preview.svg { 24. } else { 0. };
    let image_height = px(
        (f32::from(canvas_width) - padding).max(1.) * original.1 as f32 / original.0.max(1) as f32
            + padding,
    );
    let image_height = if preview.pdf {
        image_height.floor()
    } else {
        image_height
    };
    let picture = if content.image.frame_count() > 1 {
        gpui::img(content.image.clone())
            .w(image_width)
            .h(image_height)
            .into_any_element()
    } else {
        picture(content, value.raster.clone(), t.surface, preview)
            .w(image_width)
            .h(image_height)
            .into_any_element()
    };
    div()
        .id(id)
        .role(gpui::Role::Image)
        .aria_label(if preview.pdf {
            "PDF preview"
        } else {
            "Image preview"
        })
        .relative()
        .child(shapes::decoration(frame(None)))
        .child(picture)
        .children(layer(content, image_width))
        .into_any_element()
}

/// The Files tab's image or PDF: the bytes read once, decoded per page and raster width as a
/// Changes preview's after side is.
pub(super) struct FileMedia {
    bytes: Arc<Vec<u8>>,
    mime: Option<String>,
    preview: Preview,
    /// The `(page, raster width)` decoded or decoding; a raster image ignores the width.
    requested: Option<(usize, u32)>,
}

impl FileView {
    /// Whether the Files tab shows `path` as an image or PDF preview instead of text.
    pub fn previews(path: &str) -> bool {
        enabled(path, true)
    }

    /// The bundle's `sD`: the artifact type a file link opens in an artifact tab, not a file
    /// tab.
    pub fn artifact_type(path: &str) -> Option<&'static str> {
        let extension = std::path::Path::new(path)
            .extension()?
            .to_str()?
            .to_ascii_lowercase();
        Some(match extension.as_str() {
            "csv" | "tsv" | "xlsx" => "spreadsheet",
            "docx" => "document",
            "ipynb" => "notebook",
            "pdf" | "tex" => "pdf",
            "pptx" => "slides",
            _ => return None,
        })
    }

    /// Whether an artifact tab can show `path`: a PDF's pages or a text file's lines.
    // shortcut: Office files show `This document type cannot be viewed here yet`, not the
    // bundle's document, sheet and slide renderers; add one when the app ships it.
    pub fn artifact_viewable(path: &str) -> bool {
        !std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["docx", "pptx", "xlsx"]
                    .iter()
                    .any(|office| extension.eq_ignore_ascii_case(office))
            })
    }

    /// The host's `read-file-binary` answer for a [`Self::previews`] path: `(bytes, mime)`.
    /// False when the viewer moved to another file meanwhile.
    pub fn set_file_media(
        &mut self,
        revision: u64,
        media: Result<(Arc<Vec<u8>>, Option<String>), String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.revision != revision {
            return false;
        }
        match media {
            Ok((bytes, mime)) => {
                let source = PreviewSource::Worktree {
                    cwd: self.workspace_root().to_owned(),
                    path: self.path.clone(),
                };
                self.media = Some(FileMedia {
                    bytes,
                    mime,
                    preview: Preview {
                        sides: [Side::new(None), Side::new(Some(source))],
                        after_only: true,
                        pdf: is_pdf(&self.path),
                        svg: image_format(&self.path) == Some(ImageFormat::Svg),
                        pager_focus: None,
                    },
                    requested: None,
                });
            }
            Err(error) => self.contents = Some(Err(error)),
        }
        cx.notify();
        true
    }

    /// Whether an image or PDF page is decoded and shown.
    pub fn media_loaded(&self) -> bool {
        self.media
            .as_ref()
            .is_some_and(|media| media.preview.sides[1].content.is_some())
    }

    /// The body's width: the panel less the open file tree (`w-[250px] max-w-[60%]`).
    // shortcut: the stored split width, so a PDF in full view rasterizes at the split width;
    // measure the body when that blurs a page.
    fn media_width(&self, window: &Window, cx: &App) -> Pixels {
        let panel = panel_width(window, cx);
        if self.tree.open {
            panel - (panel * 0.6).min(px(251.))
        } else {
            panel
        }
    }

    /// Decodes the shown page off the main thread when the page or a PDF's raster width
    /// changed.
    pub(super) fn request_media(&mut self, window: &Window, cx: &mut Context<Self>) {
        let width = self.media_width(window, cx);
        let scale = window.scale_factor();
        let revision = self.revision;
        let Some(media) = self.media.as_mut() else {
            return;
        };
        let pdf = media.preview.pdf;
        let raster = if pdf {
            media.preview.raster_width(width, scale)
        } else {
            0
        };
        let side = &mut media.preview.sides[1];
        let key = (side.page, raster);
        if media.requested == Some(key) {
            return;
        }
        media.requested = Some(key);
        side.loading = true;
        let Some(source) = side.source.clone() else {
            return;
        };
        let request = PreviewRequest {
            index: 0,
            side: 1,
            generation: revision,
            source,
            page: key.0,
            width: key.1,
            scale,
            pdf,
            data: None,
        };
        let (bytes, mime, svg) = (media.bytes.clone(), media.mime.clone(), cx.svg_renderer());
        let decode = cx.background_spawn(async move {
            PreviewContent::decode(bytes, mime.as_deref(), &request, svg)
        });
        cx.spawn(async move |view, cx| {
            let result = decode.await;
            if let Err(error) = &result {
                eprintln!("codex-gpui: cannot preview file: {error:#}");
            }
            view.update(cx, |view, cx| {
                let Some(media) = view
                    .media
                    .as_mut()
                    .filter(|media| view.revision == revision && media.requested == Some(key))
                else {
                    return;
                };
                let side = &mut media.preview.sides[1];
                side.loading = false;
                side.error = result.is_err();
                side.content = result.ok();
                *side.raster.borrow_mut() = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn turn_media_page(&mut self, step: isize, cx: &mut Context<Self>) {
        let page = self
            .media
            .as_ref()
            .and_then(|media| media.preview.sides[1].page.checked_add_signed(step));
        if let Some(page) = page {
            self.show_media_page(page, cx);
        }
    }

    /// Shows PDF page `page` when the document has it.
    pub(super) fn show_media_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let Some(media) = self.media.as_mut() else {
            return;
        };
        let side = &mut media.preview.sides[1];
        let pages = side.content.as_ref().map_or(0, |content| content.pages);
        if (1..=pages).contains(&page) && page != side.page {
            side.page = page;
            cx.notify();
        }
    }

    /// The image or PDF page, fitted to the body, with a PDF's page controls. An artifact
    /// PDF takes annotations and shows their review bar.
    pub(super) fn media_view(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let media = self.media.as_ref()?;
        let t = cx.theme();
        let preview = &media.preview;
        let value = &preview.sides[1];
        let available = preview.content_width(self.media_width(window, cx));
        let pages = value.content.as_ref().map_or(0, |content| content.pages);
        let button = |step: isize, label: &'static str, name: &'static str| {
            let enabled = if step < 0 {
                value.page > 1
            } else {
                value.page < pages
            };
            round_icon(name, 24., t)
                .id(SharedString::from(format!("file-pdf-page{step}")))
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(!enabled)
                .when(enabled, |d| d.cursor_pointer())
                .on_click(cx.listener(move |view, _, _, cx| view.turn_media_page(step, cx)))
        };
        let annotated = preview.pdf && Self::artifact_type(&self.path).is_some();
        let review = annotated
            .then(|| self.artifact_review_bar(&self.path, 0, available, cx))
            .flatten();
        let pages_view = div()
            .id("file-preview")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .p(px(12.))
            .child(side_body(
                preview,
                value,
                available,
                "file-preview-image".into(),
                t,
                |content, image_width| {
                    if !annotated {
                        return Vec::new();
                    }
                    let (width, height) = content.pixels.dimensions();
                    let page = artifact_annotations::Page {
                        path: self.path.clone(),
                        title: self.name().to_string(),
                        kind: "pdf",
                        number: value.page,
                        count: content.pages,
                        size: comments::PageSize {
                            width: width as f32,
                            height: height as f32,
                        },
                        bounds: Default::default(),
                    };
                    self.artifact_layer(page, image_width, cx)
                },
            ))
            .when(preview.pdf && pages > 1, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(14.))
                        .top(px(14.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .bg(t.surface)
                        .p(px(2.))
                        .gap(px(2.))
                        .child(button(-1, "Previous page", "request-chevron-left"))
                        .child(text(
                            format!("{}/{pages}", value.page),
                            14.,
                            20.,
                            400.,
                            t.text_secondary,
                        ))
                        .child(button(1, "Next page", "request-chevron-right")),
                )
            });
        // The review bar floats over the pages, not inside their scroll.
        Some(match review {
            Some(review) => div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(pages_view)
                .child(review)
                .into_any_element(),
            None => pages_view.into_any_element(),
        })
    }
}

fn picture(
    content: &PreviewContent,
    cache: Raster,
    page: Hsla,
    preview: &Preview,
) -> gpui::Canvas<()> {
    let (svg, pdf) = (preview.svg, preview.pdf);
    let pixels = content.pixels.clone();
    let fallback = content.image.clone();
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _cx| {
            let scale = window.scale_factor();
            let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
            let at = Bounds::from_corners(
                point(snap(bounds.left()), snap(bounds.top())),
                point(snap(bounds.right()), snap(bounds.bottom())),
            );
            let cached = cache
                .borrow()
                .as_ref()
                .filter(|(b, s, p, _)| *b == bounds && *s == scale && *p == page)
                .map(|(_, _, _, image)| image.clone());
            let image = cached.or_else(|| {
                let padding = if svg { px(12.) } else { px(0.) };
                let mut destination = Bounds::new(
                    bounds.origin + point(padding, padding),
                    bounds.size - size(padding * 2., padding * 2.),
                );
                if pdf {
                    // The canvas's own device px, painted 1:1 from the box's snapped origin.
                    let (w, h) = pixels.dimensions();
                    let canvas = size(px(w as f32 / scale), px(h as f32 / scale));
                    destination = Bounds::new(at.origin, canvas);
                }
                let image = Arc::new(shapes::raster_decorated_images(
                    &[(&pixels, destination)],
                    bounds,
                    &frame((svg || pdf).then(gpui::white)),
                    if svg || pdf {
                        shapes::ImageComposition::Direct(page)
                    } else {
                        shapes::ImageComposition::Group(page)
                    },
                    window,
                )?);
                *cache.borrow_mut() = Some((bounds, scale, page, image.clone()));
                Some(image)
            });
            if let Some(image) = image {
                window
                    .paint_image(at, at, Corners::default(), image, 0, false)
                    .ok();
            } else {
                window
                    .paint_image(
                        bounds,
                        bounds,
                        Corners::all(px(7.5)),
                        fallback.clone(),
                        0,
                        false,
                    )
                    .ok();
            }
        },
    )
}

/// The preview box (`overflow-hidden rounded-sm shadow-sm`; PDF and svg pages on white).
fn frame(fill: Option<Hsla>) -> Decoration {
    Decoration {
        radius: Corners::all(px(7.5)),
        fill,
        shadows: vec![shapes::Shadow {
            color: gpui::black().opacity(20. / 255.),
            offset: point(px(0.), px(1.)),
            blur: px(2.),
            spread: px(-1.),
            inset: false,
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{FileView, Preview, Side, enabled, px};

    #[test]
    fn artifact_links_follow_the_bundle_types_and_office_files_have_no_viewer() {
        for (path, kind) in [
            ("data.CSV", Some("spreadsheet")),
            ("data.tsv", Some("spreadsheet")),
            ("book.xlsx", Some("spreadsheet")),
            ("memo.docx", Some("document")),
            ("run.ipynb", Some("notebook")),
            ("paper.pdf", Some("pdf")),
            ("paper.tex", Some("pdf")),
            ("deck.pptx", Some("slides")),
            ("picture.png", None),
            ("src/main.rs", None),
            ("Makefile", None),
        ] {
            assert_eq!(FileView::artifact_type(path), kind, "{path}");
        }
        assert!(!FileView::artifact_viewable("memo.DOCX"));
        assert!(!FileView::artifact_viewable("deck.pptx"));
        assert!(!FileView::artifact_viewable("book.xlsx"));
        assert!(FileView::artifact_viewable("data.csv"));
        assert!(FileView::artifact_viewable("paper.pdf"));
        assert!(FileView::artifact_viewable("src/main.rs"));
    }

    #[test]
    fn raster_images_and_pdf_ignore_the_svg_preview_toggle() {
        for path in ["image.png", "image.JPG", "image.gif", "page.PDF"] {
            assert!(enabled(path, false));
            assert!(enabled(path, true));
        }
        assert!(!enabled("drawing.svg", false));
        assert!(enabled("drawing.SVG", true));
        assert!(!enabled("source.rs", true));
    }

    #[test]
    fn previews_exclude_the_column_gap_before_rounding_pdf_width() {
        let mut preview = Preview {
            sides: [Side::new(None), Side::new(None)],
            after_only: false,
            pdf: false,
            svg: false,
            pager_focus: None,
        };
        assert_eq!(preview.content_width(px(364.5)), px(157.75));
        preview.pdf = true;
        assert_eq!(preview.content_width(px(364.5)), px(157.75));
        assert_eq!(preview.raster_width(px(364.5), 2.), 314);
        preview.after_only = true;
        assert_eq!(preview.content_width(px(364.5)), px(340.5));
        assert_eq!(preview.raster_width(px(364.5), 2.), 680);
    }
}
