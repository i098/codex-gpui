//! Icons with Chromium's exact pixels. [`Icon`] paints, in this order:
//!
//! 1. A fitted image where one exists: `tools/icons/build.py` fits one straight-alpha RGBA
//!    image per (icon, device size, color) against the refs,
//!    `assets/icons/px/<name>[-flipx]@<W>x<H>/<rrggbbaa>[@<x>_<y>].png`, painted 1:1 at the
//!    device pixel where Chromium snaps the svg.
//! 2. The svg's paths rasterized at runtime with Chromium's Skia (`shapes::paint_icon`).
//! 3. The plain svg mask (Linux, svg features the rasterizer does not parse).
//!
//! Opacity: put it in the color (`color.opacity(op)`) or on the icon itself (`.opacity(op)`),
//! not on an ancestor. Icons that Chromium paints into a composited layer (inside a scroll
//! container) round differently: give them the layer's origin ([`Icon::composited`]).

use gpui::{
    App, Bounds, Corners, Element, ElementId, GlobalElementId, Hitbox, Hsla, Image, ImageFormat,
    InspectorElementId, InteractiveElement, Interactivity, IntoElement, LayoutId, Pixels, Point,
    RenderImage, Rgba, SharedString, StyleRefinement, Styled, Transformation, Window, point, px,
    size,
};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

/// `size` x `size` CSS px icon `assets/icons/<name>.svg` in `color` (same signature as
/// `crate::icon`).
pub fn icon(name: &str, size: f32, color: Hsla) -> Icon {
    icon_sized(name, size, size, color)
}

/// An icon whose box is not square, e.g. `codex-wordmark` (53.22 x 18).
pub fn icon_sized(name: &str, width: f32, height: f32, color: Hsla) -> Icon {
    Icon {
        name: name.into(),
        width,
        height,
        flip_x: false,
        composited: None,
        parent_composited: false,
        own_layer: false,
        runtime: false,
        saved_opacity: None,
        transformation: None,
        interactivity: Interactivity::new(),
    }
    .w(px(width))
    .h(px(height))
    .flex_none()
    .text_color(color)
}

pub struct Icon {
    name: SharedString,
    /// CSS size: the svg's viewBox maps onto it unsnapped.
    width: f32,
    height: f32,
    flip_x: bool,
    composited: Option<Point<Pixels>>,
    parent_composited: bool,
    own_layer: bool,
    runtime: bool,
    saved_opacity: Option<u8>,
    transformation: Option<Transformation>,
    interactivity: Interactivity,
}

impl Icon {
    /// Mirrored horizontally, like the original's `-scale-x-100` (Forward = Back mirrored).
    pub fn flip_x(mut self) -> Self {
        self.flip_x = true;
        self
    }

    /// Chromium paints this icon into the transparent tile of the composited layer whose
    /// origin is `layer` (a scroll container's contents, window coordinates) and composites
    /// the tile, instead of blending into the page (`shapes::paint`'s `layer`).
    /// `None` uses the current compositor layer and rasterizes the SVG at its actual position.
    pub fn composited(mut self, layer: Option<Point<Pixels>>) -> Self {
        self.composited = layer;
        self.parent_composited = layer.is_none();
        self.runtime |= self.parent_composited;
        self
    }

    /// Chromium blits this icon into a transparent layer of its own, which it then composites:
    /// the overlap layer at `layer` that squashes it, or with `None` the bitmap an svg `<img>`
    /// is rasterized into (its own box). Rasterized at runtime in that layer (whose origin sets
    /// the raster's float geometry), as the fitted images hold the page blit.
    pub fn own_layer(mut self, layer: Option<Point<Pixels>>) -> Self {
        self.composited = layer;
        self.own_layer = true;
        self.top_layer()
    }

    /// Rasterized at runtime where it paints, not from a fitted image: those hold the blit at
    /// their position under the page's clips, which a layer there (the docked composer's
    /// body) does not have.
    pub fn runtime(mut self) -> Self {
        self.runtime = true;
        self
    }

    /// Applies CSS opacity to the SVG's painted bounds, preserving its fill and stroke composition.
    pub fn saved_opacity(mut self, alpha: u8) -> Self {
        self.saved_opacity = Some(alpha);
        self.runtime = true;
        self
    }
    /// Transforms the rendered icon without changing its layout or hitbox.
    pub fn with_transformation(mut self, transformation: Transformation) -> Self {
        self.transformation = Some(transformation);
        self
    }
}

fn with_saved_opacity<R>(
    alpha: Option<u8>,
    bounds: Option<Bounds<Pixels>>,
    window: &mut Window,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    match (alpha, bounds) {
        (Some(alpha), Some(bounds)) => shapes::with_layer(alpha, bounds, window, paint),
        (Some(alpha), None) => window.with_skia_layer(alpha, paint),
        (None, _) => paint(window),
    }
}

/// Decoded images by asset path; `None` = no image for that path.
/// ponytail: misses are cached per device position too, so scrolling icons add entries; bound
/// it (or list the position variants once) if that ever shows up in memory.
static IMAGES: LazyLock<Mutex<HashMap<String, Option<Arc<RenderImage>>>>> =
    LazyLock::new(Default::default);

fn load(path: String, cx: &App) -> Option<Arc<RenderImage>> {
    let mut images = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
    images
        .entry(path)
        .or_insert_with_key(|path| {
            let bytes = cx.asset_source().load(path).ok()??;
            Image::from_bytes(ImageFormat::Png, bytes.into_owned())
                .to_image_data(cx.svg_renderer())
                .ok()
        })
        .clone()
}

/// `color` as the 8-bit `rrggbbaa` key of the fitted images.
fn color_key(color: Hsla) -> String {
    let c = Rgba::from(color);
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("{:02x}{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b), b(c.a))
}

impl Element for Icon {
    type RequestLayoutState = LayoutId;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );
        (layout_id, layout_id)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout_id: &mut LayoutId,
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // The icon's own opacity goes into the color (and the fitted images' key).
        let opacity = self.interactivity.base_style.opacity.take().unwrap_or(1.);
        let (name, flip_x, own_layer) = (self.name.clone(), self.flip_x, self.own_layer);
        let (composited, runtime) = (self.composited, self.runtime);
        let parent_composited = self.parent_composited;
        let saved_opacity = self.saved_opacity;
        let transformation = self.transformation;
        let css = size(px(self.width), px(self.height));
        let mut ink_bounds = Bounds::new(bounds.origin, css);
        if saved_opacity.is_some() {
            ink_bounds.origin.x += window.layout_origin_residual_x(*layout_id);
            ink_bounds.origin.y += window.layout_origin_residual_y(*layout_id);
        }
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, cx| {
                let Some(color) = style.text.color.map(|c| c.opacity(opacity)) else {
                    return;
                };
                // Chromium snaps the svg origin to the nearest device pixel (half up) and
                // rasterizes its box rounded up to whole device pixels.
                let scale = window.scale_factor();
                let transform = transformation
                    .map(|value| value.into_matrix(bounds.center(), scale))
                    .unwrap_or_default();
                let snap = |v: Pixels| (f32::from(v) * scale + 0.5).floor();
                let (x, y) = (snap(bounds.origin.x), snap(bounds.origin.y));
                let dev = |v: Pixels| (f32::from(v) * scale).ceil();
                let (w, h) = (dev(css.width), dev(css.height));
                let flipped = if flip_x { "-flipx" } else { "" };
                let key = color_key(color);
                let dir = format!("icons/px/{name}{flipped}@{w}x{h}/{key}");
                let fitted = || {
                    load(format!("{dir}@{x}_{y}.png"), cx)
                        .or_else(|| load(format!("{dir}.png"), cx))
                };
                if let Some(image) = (!own_layer && !runtime && transformation.is_none())
                    .then(fitted)
                    .flatten()
                {
                    let device = Bounds {
                        origin: point(px(x / scale), px(y / scale)),
                        size: size(px(w / scale), px(h / scale)),
                    };
                    // Fitted images contain the page blend, not an unblended layer source.
                    window
                        .with_unlayered_paint(|window| {
                            window.paint_image(device, device, Corners::default(), image, 0, false)
                        })
                        .ok();
                    return;
                }
                let path = format!("icons/{name}.svg");
                if let Ok(Some(src)) = cx.asset_source().load(&path) {
                    let layer = composited
                        .or(own_layer.then_some(bounds.origin))
                        .or_else(|| {
                            parent_composited
                                .then(|| window.composited_layer().map(|(bounds, _)| bounds.origin))
                                .flatten()
                        });
                    let paint = |window: &mut Window, cx: &mut App| {
                        if transformation.is_some() {
                            shapes::paint_icon_transformed(
                                &src,
                                &path,
                                Bounds::new(bounds.origin, css),
                                flip_x,
                                color,
                                layer,
                                transform,
                                window,
                                cx,
                            )
                        } else {
                            shapes::paint_icon(
                                &src,
                                &path,
                                Bounds::new(bounds.origin, css),
                                flip_x,
                                color,
                                layer,
                                window,
                                cx,
                            )
                        }
                    };
                    let ink = saved_opacity.map(|_| {
                        shapes::icon_paint_bounds(&src, &path, ink_bounds, flip_x, window)
                            .unwrap_or(bounds)
                    });
                    let painted =
                        with_saved_opacity(saved_opacity, ink, window, |window| paint(window, cx));
                    if painted {
                        return;
                    }
                }
                let mut matrix = transform;
                if flip_x {
                    let center = bounds.center();
                    matrix = matrix
                        .translate(center.scale(scale))
                        .scale(size(-1., 1.))
                        .translate(center.scale(-scale));
                }
                let paint = |window: &mut Window| {
                    window.paint_svg(bounds, path.into(), None, matrix, color, cx)
                };
                with_saved_opacity(saved_opacity, None, window, paint).ok();
            },
        )
    }
}

impl IntoElement for Icon {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Icon {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

/// Renders every icon use in refs/ (`tools/icons/check.json`) on its background at its device
/// position with GPUI's svg mask ("before") and with [`icon`] ("after": fitted image, else
/// runtime raster), and counts the pixels that differ from the ref. Needs Metal:
/// `tools/mac.sh -l <lane> 'cargo test -p ui --lib icon -- --ignored --nocapture'`.
/// The runtime raster blends as the icon's own layers do, which this flat scene lacks: the
/// glyphs build.py leaves to it (`RUNTIME`) differ here and are checked by the parity run.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::{
        AppContext as _, HeadlessAppContext, ParentElement, Render, Transformation, div, rgb, rgba,
        svg,
    };
    use serde::Deserialize;
    use std::{collections::BTreeMap, path::PathBuf};

    #[derive(Deserialize, Clone)]
    struct Case {
        r#ref: String,
        name: Option<String>,
        flip: bool,
        w: f32,
        h: f32,
        x: u32,
        y: u32,
        /// With the ancestors' opacity folded in (the fitted images' key).
        color: String,
        /// CSS `color` and the product of the ancestors' `opacity`.
        css_color: String,
        opacity: f32,
        bg: [u8; 3],
    }

    impl Case {
        fn device(&self) -> (u32, u32) {
            ((self.w * 2.).ceil() as u32, (self.h * 2.).ceil() as u32)
        }
    }

    struct Scene {
        cases: Vec<Case>,
        svg_mask: bool,
    }

    impl Render for Scene {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let hsla = |c: &str| -> Hsla { rgba(u32::from_str_radix(c, 16).unwrap()).into() };
            div()
                .size_full()
                .relative()
                .children(self.cases.iter().map(|c| {
                    let (w, h) = c.device();
                    let name = c.name.clone().unwrap();
                    let glyph = if self.svg_mask {
                        let mirror = Transformation::scale(size(if c.flip { -1. } else { 1. }, 1.));
                        svg()
                            .path(format!("icons/{name}.svg"))
                            .w(px(c.w))
                            .h(px(c.h))
                            .text_color(hsla(&c.color))
                            .with_transformation(mirror)
                            .into_any_element()
                    } else {
                        let icon = icon_sized(&name, c.w, c.h, hsla(&c.css_color));
                        let icon = if c.flip { icon.flip_x() } else { icon };
                        icon.opacity(c.opacity).into_any_element()
                    };
                    let [r, g, b] = c.bg;
                    let at = |v: u32| px(v as f32 / 2.);
                    div()
                        .absolute()
                        .left(at(c.x - 1))
                        .top(at(c.y - 1))
                        .w(at(w + 2))
                        .h(at(h + 2))
                        .bg(rgb(u32::from_be_bytes([0, r, g, b])))
                        .child(div().absolute().left(at(1)).top(at(1)).child(glyph))
                }))
        }
    }

    fn render(
        cx: &mut HeadlessAppContext,
        cases: &[Case],
        svg_mask: bool,
        (w, h): (u32, u32),
    ) -> image::RgbaImage {
        let cases = cases.to_vec();
        let window = cx
            .open_window(size(px(w as f32), px(h as f32)), |_, cx| {
                cx.new(|_| Scene { cases, svg_mask })
            })
            .unwrap();
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into()).unwrap();
        cx.update_window(window.into(), |_, window, _| window.remove_window())
            .unwrap();
        image
    }

    fn diff(image: &image::RgbaImage, reference: &image::RgbaImage, c: &Case) -> u32 {
        let (w, h) = c.device();
        let mut n = 0;
        for y in c.y - 1..c.y + h + 1 {
            for x in c.x - 1..c.x + w + 1 {
                let (a, b) = (image.get_pixel(x, y), reference.get_pixel(x, y));
                n += u32::from(a.0[..3] != b.0[..3]);
            }
        }
        n
    }

    #[test]
    #[ignore = "needs Metal and refs/; see the module doc"]
    fn icons_match_chromium() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cases: Vec<Case> =
            serde_json::from_slice(&std::fs::read(root.join("tools/icons/check.json")).unwrap())
                .unwrap();
        let mut by_ref: BTreeMap<String, Vec<Case>> = BTreeMap::new();
        let skipped = cases.iter().filter(|c| c.name.is_none()).count();
        for c in cases.into_iter().filter(|c| c.name.is_some()) {
            by_ref.entry(c.r#ref.clone()).or_default().push(c);
        }
        // No text here, and the macOS text system wants the main thread (tests run off it).
        let text_system = Arc::new(gpui::NoopTextSystem::new());
        let mut cx =
            HeadlessAppContext::with_platform(text_system, Arc::new(theme::Assets), || {
                gpui_platform::current_headless_renderer()
            });
        // name -> [cases, px before, after, exact before, after]
        let mut table: BTreeMap<String, [u32; 5]> = BTreeMap::new();
        for (path, cases) in &by_ref {
            let reference = image::open(root.join(path)).unwrap().to_rgba8();
            let (w, h) = reference.dimensions();
            let size = (w / 2, h / 2);
            let before = render(&mut cx, cases, true, size);
            let after = render(&mut cx, cases, false, size);
            // Renders for inspection: target/icon-check/<screen>-<theme@size>.<pass>.png
            let out = root
                .join("target/icon-check")
                .join(path.replace("refs/", "").replace('/', "-"));
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            let passes = [("before", &before), ("after", &after)];
            for (pass, img) in passes {
                img.save(out.with_extension(format!("{pass}.png"))).unwrap();
            }
            for c in cases {
                let label = format!(
                    "{}{}",
                    c.name.as_ref().unwrap(),
                    if c.flip { "-flipx" } else { "" }
                );
                let d = passes.map(|(_, img)| diff(img, &reference, c));
                let row = table.entry(label).or_default();
                row[0] += 1;
                for i in 0..2 {
                    row[1 + i] += d[i];
                    row[3 + i] += u32::from(d[i] == 0);
                }
            }
        }
        println!(
            "{:<36} {:>5} {:>8} {:>8} {:>6} {:>6}",
            "icon", "cases", "before", "after", "=0 b", "=0 a"
        );
        let mut total = [0; 5];
        for (name, row) in &table {
            let [n, b, a, eb, ea] = *row;
            println!("{name:<36} {n:>5} {b:>8} {a:>8} {eb:>6} {ea:>6}");
            for i in 0..5 {
                total[i] += row[i];
            }
        }
        let [n, b, a, eb, ea] = total;
        println!(
            "{:<36} {n:>5} {b:>8} {a:>8} {eb:>6} {ea:>6}  ({skipped} cases without an assets/icons svg)",
            "total"
        );
        assert_eq!(a, 0, "icons differ from Chromium");
    }

    struct RotatingIcon(f32);

    impl Render for RotatingIcon {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().size_full().bg(rgb(0x181818)).child(
                icon("sidebar-cmd-spinner", 28., gpui::white().opacity(0.65))
                    .own_layer(None)
                    .with_transformation(Transformation::rotate(gpui::radians(self.0)))
                    .absolute()
                    .left(px(18.))
                    .top(px(18.)),
            )
        }
    }

    #[test]
    fn native_icon_rotation_preserves_its_raster() -> anyhow::Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(gpui::NoopTextSystem::new()),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        let mut images = Vec::new();
        for angle in [0., std::f32::consts::FRAC_PI_2] {
            let window = cx.open_window(size(px(64.), px(64.)), |_, cx| {
                cx.new(|_| RotatingIcon(angle))
            })?;
            cx.run_until_parked();
            images.push(cx.capture_screenshot(window.into())?);
            cx.update_window(window.into(), |_, window, _| window.remove_window())?;
        }
        assert_eq!(images[0].get_pixel(64, 47).0, [190, 190, 190, 255]);
        for y in 0..128 {
            for x in 0..128 {
                assert_eq!(
                    images[1].get_pixel(x, y),
                    images[0].get_pixel(y, 127 - x),
                    "clockwise quarter turn at {x},{y}",
                );
            }
        }
        Ok(())
    }
}
