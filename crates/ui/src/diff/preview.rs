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
            let (pixels, pages) = super::pdf::render(&bytes, request.page, request.width)?;
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
}

impl Preview {
    pub(super) fn loaded(&self) -> bool {
        self.sides.iter().all(|side| !side.loading)
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
                let columns = if preview.after_only { 1. } else { 2. };
                let desired = ((f32::from(width) / columns - 24.).max(1.) * scale).round() as u32;
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
            let preview = Preview {
                pdf: is_pdf(&file.name),
                svg: image_format(&file.name) == Some(ImageFormat::Svg),
                sides: sources.map(Side::new),
                after_only,
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
        let columns = if preview.after_only { 1. } else { 2. };
        cx.emit(DiffRequest::LoadPreview(PreviewRequest {
            index,
            side,
            generation: self.content_generation,
            source,
            page: value.page,
            width: ((f32::from(width) / columns - 24.).max(1.) * scale).round() as u32,
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

    pub(super) fn preview(&self, index: usize, width: Pixels, cx: &Context<Self>) -> Option<Div> {
        let file = self.files.get(index)?;
        if !enabled(&file.name, self.rich_preview) {
            return None;
        }
        let preview = file.preview.as_ref()?;
        let t = cx.theme();
        let columns = if preview.after_only { 1. } else { 2. };
        let available = (width / columns - px(24.)).max(px(1.));
        let render_side = |side: usize| {
            let value = &preview.sides[side];
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
            let body = if let (false, Some(content)) = (value.loading, value.content.as_ref()) {
                let original = content.pixels.dimensions();
                let image_width = if preview.pdf || preview.svg {
                    available
                } else {
                    available.min(px(original.0 as f32))
                };
                let padding = if preview.svg { 24. } else { 0. };
                let image_height = px((f32::from(image_width) - padding).max(1.)
                    * original.1 as f32
                    / original.0.max(1) as f32
                    + padding);
                let picture = if content.image.frame_count() > 1 {
                    gpui::img(content.image.clone())
                        .w(image_width)
                        .h(image_height)
                        .into_any_element()
                } else {
                    picture(content, value.raster.clone(), t.surface, preview.svg)
                        .w(image_width)
                        .h(image_height)
                        .into_any_element()
                };
                div()
                    .id(SharedString::from(format!("diff-preview-{index}-{side}")))
                    .role(gpui::Role::Image)
                    .aria_label(if preview.pdf {
                        "PDF preview"
                    } else {
                        "Image preview"
                    })
                    .relative()
                    .child(shapes::decoration(Decoration {
                        radius: Corners::all(px(7.5)),
                        shadows: vec![shapes::Shadow {
                            color: gpui::black().opacity(20. / 255.),
                            offset: point(px(0.), px(1.)),
                            blur: px(2.),
                            spread: px(-1.),
                            inset: false,
                        }],
                        ..Default::default()
                    }))
                    .child(picture)
                    .into_any_element()
            } else {
                text(
                    label,
                    if value.loading { 12. } else { 14. },
                    20.,
                    400.,
                    t.text_secondary,
                )
                .into_any_element()
            };
            let pages = value.content.as_ref().map_or(0, |content| content.pages);
            div()
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
        Some(
            div()
                .w_full()
                .flex()
                .gap(px(1.))
                .when(!preview.after_only, |d| d.child(render_side(0)))
                .child(render_side(1)),
        )
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
        let Some(preview) = self
            .files
            .get_mut(index)
            .and_then(|file| file.preview.as_mut())
        else {
            return;
        };
        let side_value = &mut preview.sides[side];
        let Some(page) = side_value.page.checked_add_signed(step) else {
            return;
        };
        if page == 0
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
}

fn picture(content: &PreviewContent, cache: Raster, page: Hsla, svg: bool) -> gpui::Canvas<()> {
    let pixels = content.pixels.clone();
    let fallback = content.image.clone();
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _cx| {
            let scale = window.scale_factor();
            let cached = cache
                .borrow()
                .as_ref()
                .filter(|(b, s, p, _)| *b == bounds && *s == scale && *p == page)
                .map(|(_, _, _, image)| image.clone());
            let image = cached.or_else(|| {
                let padding = if svg { px(12.) } else { px(0.) };
                let destination = Bounds::new(
                    bounds.origin + point(padding, padding),
                    bounds.size - size(padding * 2., padding * 2.),
                );
                let image = Arc::new(shapes::raster_images(
                    &[(&pixels, destination)],
                    bounds,
                    Corners::all(px(7.5)),
                    shapes::Corner::Round,
                    shapes::ImageComposition::Direct(if svg { gpui::white() } else { page }),
                    window,
                )?);
                *cache.borrow_mut() = Some((bounds, scale, page, image.clone()));
                Some(image)
            });
            if let Some(image) = image {
                let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
                let at = Bounds::from_corners(
                    point(snap(bounds.left()), snap(bounds.top())),
                    point(snap(bounds.right()), snap(bounds.bottom())),
                );
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

#[cfg(test)]
mod tests {
    use super::enabled;

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
}
