//! Box decorations and icons that land on Chromium's exact pixels: rounded and
//! `corner-shape: superellipse(1.5)` backgrounds and borders, `box-shadow` layers and svg icon
//! paths, as the Codex desktop app's renderer (chrome-headless-shell, Skia CPU raster, 2x)
//! draws them.
//!
//! # API
//!
//! ```ignore
//! div().relative().child(shapes::decoration(shapes::Decoration {
//!     radius: Corners::all(px(12.5)),
//!     corner: shapes::Corner::Squircle,          // `corner-shape: superellipse(1.5)`
//!     fill: Some(bg),                             // `background-color`
//!     border: Some((px(1.), border)),             // `border: 1px solid border`
//!     shadows: vec![shapes::Shadow {              // `box-shadow`, CSS order (top layer first)
//!         color, offset: point(px(0.), px(4.)), blur: px(16.), spread: px(0.), inset: false,
//!     }],
//! }))
//! ```
//!
//! [`decoration`] is an absolutely positioned element that covers its parent (put it first so
//! the content paints on top) in place of `.rounded()`, `.bg()`, `.border()` and `.shadow()`.
//! [`paint`] does the same inside any paint callback. Radii are CSS values (`px(9999.)` for
//! pills); they are constrained to the box like Blink's `ConstrainRadii`. Paint order is
//! Blink's: outer shadows (last listed first), fill, inset shadows, border. The box snaps to
//! device pixels like Blink's `PixelSnappedIntRect`. [`paint_icon`] paints an svg icon's
//! paths the same way.
//!
//! # How
//!
//! On macOS every layer is rasterized at runtime by the Skia build Chromium uses, through the
//! calls Blink makes, once per cc raster tile of the layer's recording (`src/skia.rs`). The result is
//! read back as per-pixel coverage and blend mode and painted as GPUI monochrome sprites
//! (`Window::paint_svg` with an SVG wrapping the mask as a nearest-neighbor PNG); the vendored
//! gpui_apple shader blends each with Skia's integer formula for that mode. Elsewhere it
//! falls back to GPUI's quad and shadow primitives, which are not Skia-exact.
//!
//! Composited layers: put the content in a GPUI `.layer()` div whose origin is Chromium's
//! layer origin and paint with `layer: None`. Every op is
//! rasterized in that layer's space (Blink's float geometry depends on it) and blitted into
//! the layer, which is composited once, as cc does. Keep GPUI quads (`.bg()`) out of such a
//! layer: they bypass it. Without a GPUI layer, [`paint`]'s `layer` gives the origin and each
//! op is composited on its own.
//! Sparse viewport layers keep their window origin and record their decoration bounds with
//! [`recorded_bounds`] during prepaint and [`with_recorded_bounds`] during paint.
//!
//! Colors outside [0, 1] (out-of-gamut oklab/color-mix results) are drawn as Skia draws them,
//! with its float raster pipeline. Limits: inset shadows paint apart from Blink's clip layer.
//! Use [`with_composited_opacity`] for compositor opacity; [`with_layer`] retains Skia's save-layer rounding.

use gpui::{
    App, Bounds, Corners, Hsla, IntoElement, Pixels, Point, Styled as _, Window, canvas, point, px,
};

#[cfg(target_os = "macos")]
mod backdrop;
#[cfg(target_os = "macos")]
mod mask;
#[cfg(target_os = "macos")]
mod skia;

/// Restore the root under page tiles that an opaque scroller removes before rounded clipping.
#[cfg(target_os = "macos")]
pub fn paint_page_occlusion(
    page: Bounds<Pixels>,
    opaque: Bounds<Pixels>,
    root: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let scale = window.scale_factor();
    let rect = |bounds: Bounds<Pixels>| {
        [
            f32::from(bounds.left()) * scale,
            f32::from(bounds.top()) * scale,
            f32::from(bounds.right()) * scale,
            f32::from(bounds.bottom()) * scale,
        ]
        .map(|v| v.round() as i32)
    };
    let page = rect(page);
    let opaque = rect(opaque);
    let fill = Decoration {
        fill: Some(root),
        ..Default::default()
    };
    for (_, _, tile) in skia::tiles(page, [page[2], page[3]], [skia::TILE; 2]) {
        if let Some(removed) = occluded_tile(tile, opaque) {
            let [left, top, right, bottom] = removed.map(|v| gpui::px(v as f32 / scale));
            paint(
                &fill,
                None,
                Bounds::from_corners(gpui::point(left, top), gpui::point(right, bottom)),
                window,
                cx,
            );
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn occluded_tile(tile: [i32; 4], opaque: [i32; 4]) -> Option<[i32; 4]> {
    let removed = [
        tile[0].max(opaque[0]),
        tile[1].max(opaque[1]),
        tile[2].min(opaque[2]),
        tile[3].min(opaque[3]),
    ];
    if removed[0] >= removed[2] || removed[1] >= removed[3] {
        return None;
    }
    let trims_width = removed[0] == tile[0]
        && removed[2] == tile[2]
        && (removed[1] == tile[1] || removed[3] == tile[3]);
    let trims_height = removed[1] == tile[1]
        && removed[3] == tile[3]
        && (removed[0] == tile[0] || removed[2] == tile[2]);
    (trims_width || trims_height).then_some(removed)
}

#[test]
fn page_occlusion_preserves_nonrectangular_tile_remainders() {
    let opaque = [1323, 186, 2552, 1656];
    assert_eq!(
        occluded_tile([2041, 1531, 2551, 1664], opaque),
        Some([2041, 1531, 2551, 1656])
    );
    assert_eq!(occluded_tile([2551, 1531, 2560, 1664], opaque), None);
    assert_eq!(
        occluded_tile([2041, 0, 2551, 511], opaque),
        Some([2041, 186, 2551, 511])
    );
    assert_eq!(occluded_tile([0, 0, 511, 511], opaque), None);
    assert_eq!(occluded_tile([0, 0, 10, 10], [2, 2, 8, 8]), None);
}

/// CSS `corner-shape`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Corner {
    #[default]
    Round,
    /// `superellipse(1.5)` (`--codex-corner-shape`, `--menu-corner-shape`, `data-squircle`).
    Squircle,
}

/// One `box-shadow` layer, CSS px.
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub color: Hsla,
    pub offset: Point<Pixels>,
    pub blur: Pixels,
    pub spread: Pixels,
    pub inset: bool,
}

/// The decorated box's paint: what `.rounded()`, `.bg()`, `.border()` and `.shadow()` set.
#[derive(Clone, Debug, Default)]
pub struct Decoration {
    pub radius: Corners<Pixels>,
    pub corner: Corner,
    pub fill: Option<Hsla>,
    /// Uniform border: width, color.
    pub border: Option<(Pixels, Hsla)>,
    /// `border-style: dashed` (round corners; Blink's curved dashed side).
    pub dashed: bool,
    /// CSS order: the first shadow is the top layer.
    pub shadows: Vec<Shadow>,
}

/// Reports a box that something other than a decoration paints (an icon's raster box, a
/// quad) into the [`recorded_bounds`] being collected, as cc's recorded bounds cover every
/// painted op. Outside a collection it does nothing.
pub fn record_painted(bounds: Bounds<Pixels>, window: &Window) {
    #[cfg(target_os = "macos")]
    skia::record_decoration(&Decoration::default(), bounds, window);
    #[cfg(not(target_os = "macos"))]
    let _ = (bounds, window);
}

/// An element covering its parent's box that paints `d`.
pub fn decoration(d: Decoration) -> impl IntoElement {
    translated_decoration(d, Point::default())
}

/// [`decoration`] moved by `offset` CSS px after the box snaps, as Blink paints a snapped box
/// under a subpixel `transform: translate(..)`.
pub fn translated_decoration(d: Decoration, offset: Point<Pixels>) -> impl IntoElement {
    cover(recorded(d), move |bounds, d, window, cx| {
        #[cfg(target_os = "macos")]
        skia::paint(&d, None, offset, bounds, window, cx);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cx;
            quads(&d, Bounds::new(bounds.origin + offset, bounds.size), window);
        }
    })
}

/// [`decoration`] for a box that Chromium paints into a transparent composited layer of its
/// own (a fixed popup over composited content), which the compositor blends onto the page
/// with rounding. The layer starts up and left of every shadow px: Skia's blur reaches 3
/// sigma (blur / 2 each) past the spread box.
pub fn composited_decoration(d: Decoration) -> impl IntoElement {
    cover(recorded(d), |bounds, d, window, cx| {
        let reach = |s: &Shadow, offset: Pixels| {
            (f32::from(s.blur) * 1.5 + f32::from(s.spread) - f32::from(offset)).max(0.)
        };
        let (mut x, mut y) = (0f32, 0f32);
        for s in d.shadows.iter().filter(|s| !s.inset) {
            x = x.max(reach(s, s.offset.x));
            y = y.max(reach(s, s.offset.y));
        }
        let layer = bounds.origin - point(px(x), px(y));
        paint(&d, Some(layer), bounds, window, cx)
    })
}

/// The prepaint of a decoration element: records `d`'s paint bounds and passes it on.
fn recorded(d: Decoration) -> impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> Decoration {
    move |bounds, window, _| {
        #[cfg(target_os = "macos")]
        skia::record_decoration(&d, bounds, window);
        #[cfg(not(target_os = "macos"))]
        let _ = (bounds, window);
        d
    }
}

/// An element covering its parent's box, painted by `paint`.
fn cover<T: 'static>(
    prepaint: impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T + 'static,
    paint: impl FnOnce(Bounds<Pixels>, T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    canvas(prepaint, paint)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// Paints `d` for the border box `bounds`. `layer`: for a box that Chromium paints into a
/// composited layer (a scroll container's contents), that layer's origin in window
/// coordinates: the layer's tiles start there and are transparent, and the compositor
/// blends them onto the page. `None` takes the innermost GPUI `.layer()` div being painted
/// (`Window::composited_layer`), else the root layer.
pub fn paint(
    d: &Decoration,
    layer: Option<Point<Pixels>>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    #[cfg(target_os = "macos")]
    skia::paint(d, layer, Point::default(), bounds, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (cx, layer);
        quads(d, bounds, window);
    }
}

/// [`paint`] for a fill-only `d` under CSS `transform: scale(factor)`. Blink paints the snapped
/// box in its local space and scales it about its transform origin, 50% of the layout box
/// (`size`, the authored size: `bounds` arrive snapped) from the snapped paint offset, so the
/// drawn edges keep the scaled fraction.
pub fn paint_scaled(
    d: &Decoration,
    layer: Option<Point<Pixels>>,
    bounds: Bounds<Pixels>,
    size: gpui::Size<Pixels>,
    factor: f32,
    window: &mut Window,
    cx: &App,
) {
    #[cfg(target_os = "macos")]
    skia::paint_scaled(d, layer, bounds, size, factor, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (cx, layer, size);
        let c = bounds.center();
        let r = [
            bounds.left(),
            bounds.top(),
            bounds.size.width,
            bounds.size.height,
        ]
        .map(f32::from);
        let [l, t, w, h] = scaled_rect(r, [c.x, c.y].map(f32::from), factor).map(gpui::px);
        quads(d, Bounds::new(gpui::point(l, t), gpui::size(w, h)), window);
    }
}

/// `r` (`[left, top, width, height]`, device px) under `scale(factor)` about `origin`.
fn scaled_rect(r: [f32; 4], origin: [f32; 2], factor: f32) -> [f32; 4] {
    let [l, t, w, h] = r;
    let [x, y] = origin;
    [
        x + (l - x) * factor,
        y + (t - y) * factor,
        w * factor,
        h * factor,
    ]
}

/// Fills a canvas rounded rectangle without snapping its bounds to device pixels.
/// The radius must be finite and nonnegative; it is constrained to half the shorter side.
/// macOS uses Skia; other platforms use GPUI's fractional path renderer.
pub fn paint_canvas_round_rect(
    radius: Pixels,
    color: Hsla,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    assert!(f32::from(radius).is_finite() && radius >= gpui::px(0.));
    if bounds.is_empty() {
        return;
    }
    let radius = radius
        .min(bounds.size.width / 2.)
        .min(bounds.size.height / 2.);
    #[cfg(target_os = "macos")]
    skia::paint_canvas_round_rect(radius, color, bounds, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        window.paint_path(canvas_round_rect_path(bounds, radius), color);
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn canvas_round_rect_path(bounds: Bounds<Pixels>, radius: Pixels) -> gpui::Path<Pixels> {
    use gpui::{PathBuilder, point, px};
    let (left, top, right, bottom) = (bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
    let mut path = PathBuilder::fill();
    path.move_to(point(left + radius, top));
    for (edge, corner) in [
        (point(right - radius, top), point(right, top + radius)),
        (point(right, bottom - radius), point(right - radius, bottom)),
        (point(left + radius, bottom), point(left, bottom - radius)),
        (point(left, top + radius), point(left + radius, top)),
    ] {
        path.line_to(edge);
        path.arc_to(point(radius, radius), px(0.), false, true, corner);
    }
    path.close();
    path.build().expect("valid canvas rounded rectangle")
}

#[test]
fn canvas_path_preserves_fractional_bounds() {
    use gpui::{point, px, size};
    for radius in [0., 1.5] {
        let bounds = Bounds::new(point(px(10.25), px(20.125)), size(px(3.), px(13.375)));
        let path = canvas_round_rect_path(bounds, px(radius));
        assert_eq!(path.bounds, bounds);
    }
}

/// Paints a repeating two-color Lab gradient with a hard stop halfway through each CSS-pixel period.
/// Colors retain their unrounded sRGB channels. The period must be finite and positive.
pub fn paint_horizontal_stripes(
    period: Pixels,
    colors: [gpui::Rgba; 2],
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    assert!(f32::from(period).is_finite() && period > gpui::px(0.));
    #[cfg(target_os = "macos")]
    skia::paint_horizontal_stripes(period, colors, bounds, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        let half = f32::from(period) / 2.;
        let mut top = f32::from(bounds.top());
        let bottom = f32::from(bounds.bottom());
        let mut index = 1;
        while top < bottom {
            window.paint_quad(gpui::fill(
                Bounds::new(
                    gpui::point(bounds.left(), gpui::px(top)),
                    gpui::size(bounds.size.width, gpui::px(half.min(bottom - top))),
                ),
                colors[index],
            ));
            index ^= 1;
            top += half;
        }
    }
}

/// Draws a vertical color-to-transparent fade in its compositor layer's raster coordinates.
/// The fade must share the layer's left edge; `layer_offset_y` is its distance below the layer top.
/// For example, `vertical_fade(color, px(56.))` follows a 56 px sticky toolbar.
/// macOS preserves the original dithering; other platforms use the native gradient.
pub fn vertical_fade(color: Hsla, layer_offset_y: Pixels) -> impl IntoElement {
    VerticalFade {
        color,
        layer_offset_y,
    }
}

struct VerticalFade {
    color: Hsla,
    layer_offset_y: Pixels,
}

impl IntoElement for VerticalFade {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for VerticalFade {
    type RequestLayoutState = gpui::LayoutId;
    type PrepaintState = Bounds<Pixels>;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let (id, _) = gpui::div()
            .absolute()
            .size_full()
            .request_layout(None, None, window, cx);
        (id, id)
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        mut bounds: Bounds<Pixels>,
        id: &mut gpui::LayoutId,
        window: &mut Window,
        _: &mut App,
    ) -> Bounds<Pixels> {
        bounds.origin.x += window.layout_origin_residual_x(*id);
        bounds.origin.y += window.layout_origin_residual_y(*id);
        bounds
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut gpui::LayoutId,
        bounds: &mut Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(target_os = "macos")]
        skia::paint_vertical_fade(self.color.into(), self.layer_offset_y, *bounds, window, cx);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (cx, self.layer_offset_y);
            window.paint_quad(gpui::fill(
                *bounds,
                gpui::linear_gradient(
                    180.,
                    gpui::linear_color_stop(self.color, 0.),
                    gpui::linear_color_stop(self.color.opacity(0.), 1.),
                ),
            ));
        }
    }
}

/// How image pixels compose over their background.
#[derive(Clone, Copy, Debug)]
pub enum ImageComposition {
    /// Draw each image directly over the background.
    Direct(Hsla),
    /// Restore the images as one Skia saveLayer over the background.
    Group(Hsla),
    /// Restore a transparent compositor tile over an opaque background.
    Composited(Hsla),
}

/// `images` (premultiplied RGBA, see [`premultiply`], each drawn into its `dst` rect in
/// window px) using `composition`, inside the anti-aliased border-box clip with `radius`
/// and `corner`: an `<img>` or `background-image` in an `overflow: hidden` rounded box,
/// rasterized by Skia (bilinear, Chromium's span and tile restarts included) once per cc
/// tile. Over the box's snapped device px rect; `None` off macOS.
pub fn raster_images(
    images: &[(&image::RgbaImage, Bounds<Pixels>)],
    clip: Bounds<Pixels>,
    radius: Corners<Pixels>,
    corner: Corner,
    composition: ImageComposition,
    window: &Window,
) -> Option<gpui::RenderImage> {
    let d = Decoration {
        radius,
        corner,
        ..Default::default()
    };
    raster_decorated_images(images, clip, &d, composition, window)
}

/// [`raster_images`] for a box that paints `d` (its outer `box-shadow`s and
/// `background-color`) below its clipped content: inside the box's rect the raster holds
/// Blink's whole paint, so the corners outside the clip keep the shadow and the fill's
/// anti-aliased edge. Paint the same `d` with [`decoration`] for the shadow outside the rect.
pub fn raster_decorated_images(
    images: &[(&image::RgbaImage, Bounds<Pixels>)],
    clip: Bounds<Pixels>,
    d: &Decoration,
    composition: ImageComposition,
    window: &Window,
) -> Option<gpui::RenderImage> {
    #[cfg(target_os = "macos")]
    return Some(skia::raster_images(images, clip, d, composition, window));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (images, clip, d, composition, window);
        None
    }
}

/// One `drawImage` call on an opaque 2D canvas: the premultiplied image (see [`premultiply`])
/// drawn whole into the device px rect `[x, y, width, height]`, with bilinear filtering when
/// `imageSmoothingEnabled`, else nearest.
pub struct CanvasDraw {
    pub image: image::RgbaImage,
    pub rect: [i32; 4],
    pub smooth: bool,
}

/// A white `width`×`height` opaque canvas after `draws`, as Chromium's software 2D canvas
/// rasterizes them (Skia's legacy bilinear); the pixels are premultiplied.
#[cfg(target_os = "macos")]
pub fn canvas_images(width: u32, height: u32, draws: &[CanvasDraw]) -> image::RgbaImage {
    skia::canvas_images(width, height, draws)
}

/// Premultiplies `img` in place as Skia's N32 premul does: `round(c * a / 255)`.
pub fn premultiply(img: &mut image::RgbaImage) {
    for p in img.pixels_mut() {
        let a = u32::from(p[3]);
        for c in &mut p.0[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// How a [`backdrop_blur`] box paints `Decoration::fill`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Fill {
    /// `background-color`.
    #[default]
    Color,
    /// A one-color `background-image: linear-gradient(c, c)`: Blink clips the image to the
    /// shape and Skia dithers it, so its edges and translucent levels differ from a color.
    Gradient,
}

/// [`decoration`] for a box with CSS `backdrop-filter: blur(blur)`: Chromium's compositor
/// blurs the same-frame scene painted below the box, including text and images, clips it
/// to the box's shape, and composites the box's own layer (`d`, its fill painted as `fill`)
/// over it. A nested box captures the already-painted parent and its content.
/// Paint it where the box's decoration goes (first child); its text and children paint on top.
/// If capture fails, the decoration remains visible without blur; the window logs the first failure.
/// `gap_row`: Chromium's blur input has one transparent row
/// below the box (measured on the diff capsules where the main surface's rounded clip owns a
/// render surface, `screens::diff::panel::panel_clamped`; its source is not found).
pub fn backdrop_blur(blur: Pixels, fill: Fill, gap_row: bool, d: Decoration) -> impl IntoElement {
    cover(
        |_, _, _| {},
        move |bounds, (), window, _| {
            #[cfg(target_os = "macos")]
            backdrop::paint(blur, fill, gap_row, &d, bounds, window);
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (blur, fill, gap_row);
                quads(&d, bounds, window);
            }
        },
    )
}

/// CSS `mask-image: linear-gradient(to bottom, transparent, black <fade>)` over its parent,
/// which must be a `.layer()` element (Chromium masks the composited layer): put it last so it
/// multiplies everything painted into that layer by the gradient's alpha (`DstIn`) before the
/// layer is composited. The rows past `fade` are left as they are. macOS only: elsewhere GPUI
/// has no layer pixels to mask and nothing is painted.
pub fn top_fade(fade: Pixels) -> impl IntoElement {
    cover(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            #[cfg(target_os = "macos")]
            mask::paint_top_fade(fade, bounds, window, cx);
            #[cfg(not(target_os = "macos"))]
            let _ = (fade, bounds, window, cx);
        },
    )
}

/// CSS `mask-image: linear-gradient(to right, transparent, black <left>, black calc(100% -
/// <right>), transparent)` over its parent (`.text-fade-truncate` has `left` 0), which must be
/// a `.layer()` element, like [`top_fade`]: put it last. macOS only, like [`top_fade`].
pub fn x_fade(left: Pixels, right: Pixels) -> impl IntoElement {
    x_fade_with(move || (left, right))
}

/// [`x_fade`] whose `(left, right)` distances come from `edges`, called once while the element
/// paints, after prepaint and layout of the whole frame. State that prepaint updates (a
/// `ScrollHandle`'s viewport size and maximum offset) is current there, but not when the
/// element is built: read it inside `edges`, never before. `edges` is not called where nothing
/// is painted (not macOS).
pub fn x_fade_with(edges: impl FnOnce() -> (Pixels, Pixels) + 'static) -> impl IntoElement {
    cover(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            #[cfg(target_os = "macos")]
            {
                let (left, right) = edges();
                mask::paint_x_fade(left, right, bounds, window, cx);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (edges, bounds, window, cx);
        },
    )
}

/// A rounded `clip-path` (or `border-radius` with `overflow: clip`) on a composited element:
/// viz draws the element's px through the clip's coverage `m` (the anti-aliased fill of the
/// rounded rect `bounds` with `radius`) over what lies below, partial coverage with Skia's
/// `SkBlendARGB32`. Paint what lies below in the enclosing GPUI layer
/// (`Window::with_skia_layer`), over the element: in the rounded corners the element (what is
/// under the layer) is drawn onto the layer that way, except at the `own` corners, whose
/// element draws its px there itself ([`paint_clipped_tile`]): there the layer's pixel stays
/// wherever the clip does not cover the px in full. Other px are left as they are. macOS only,
/// like [`top_fade`].
pub fn paint_rounded_clip(
    radius: Corners<Pixels>,
    own: Corners<bool>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    #[cfg(target_os = "macos")]
    mask::paint_rounded_clip(radius, own, bounds, window, cx);
    #[cfg(not(target_os = "macos"))]
    let _ = (radius, own, bounds, window, cx);
}

/// A composited raster tile's premultiplied device px `px(x, y, m)` (`m`: the clip's coverage
/// there) as viz draws them through the rounded clip of [`paint_rounded_clip`]: only the px
/// the clip covers in part at the corners rounded in `radius`, each blitted at that coverage
/// (`SkBlendARGB32`; a `solid` tile, which cc draws as a color quad, with the color blitter's
/// mask blit). Paint it over what lies below the element there (an `own` corner of
/// [`paint_rounded_clip`]). macOS only.
pub fn paint_clipped_tile(
    px: &dyn Fn(i32, i32, u8) -> [u8; 4],
    solid: bool,
    radius: Corners<Pixels>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    #[cfg(target_os = "macos")]
    mask::paint_clipped_tile(px, solid, radius, bounds, window, cx);
    #[cfg(not(target_os = "macos"))]
    let _ = (px, solid, radius, bounds, window, cx);
}

/// Runs `paint` with an ancestor's anti-aliased rounded clip active for the shapes and icons
/// it paints: `bounds`, `radius` and `corner` are the clipping box's padding box (`overflow:
/// hidden` with `border-radius`). Skia draws them inside that clip, and in every raster tile
/// its curves reach the AA clip makes AAA blit edge px as runs (other values than without a
/// clip). `layers: false`: the clip skips what `paint` paints in nested layers (`.layer()`,
/// own-layer icons), which Chromium rasters without it. macOS only; elsewhere `paint` just
/// runs.
pub fn with_rounded_clip<R>(
    radius: Corners<Pixels>,
    corner: Corner,
    bounds: Bounds<Pixels>,
    layers: bool,
    window: &mut Window,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    #[cfg(target_os = "macos")]
    return skia::with_rounded_clip(radius, corner, bounds, layers, window, paint);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (radius, corner, bounds, layers);
        paint(window)
    }
}

/// Runs `paint` inside an anti-aliased rounded rectangle at `bounds`' unsnapped device
/// geometry, as Skia clips by a `clip-path: inset(.. round radius)` (Blink does not snap it)
/// the boxes that do snap to whole px: at a fractional edge the box's last px row blends at the
/// clip's coverage, and the corners follow the clip's arcs. As `with_rounded_clip` with
/// `layers: false`, what `paint` paints in nested layers rasters without it. macOS only;
/// elsewhere `paint` just runs.
pub fn with_unsnapped_clip<R>(
    bounds: Bounds<Pixels>,
    radius: Corners<Pixels>,
    window: &mut Window,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    #[cfg(target_os = "macos")]
    return skia::with_unsnapped_clip(bounds, radius, window, paint);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (bounds, radius);
        paint(window)
    }
}

/// `Window::with_skia_layer(alpha, paint)` for a Blink `saveLayerAlpha` with `bounds` (window
/// px; a CSS `opacity` on a box): Skia draws into the layer's own device, where an ancestor's
/// anti-aliased clip ([`with_rounded_clip`]) that covers `bounds` is a plain rect clip, so the
/// shapes painted in it raster without that clip.
/// The bounds round outward to the full device rectangle, including its right and bottom edges.
pub fn with_layer<R>(
    alpha: u8,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    #[cfg(target_os = "macos")]
    return skia::with_layer(alpha, bounds, window, paint);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bounds;
        window.with_skia_layer(alpha, paint)
    }
}

/// Runs a subtree's prepaint and returns its decoration bounds in device pixels.
pub fn recorded_bounds(prepaint: impl FnOnce()) -> Option<[i32; 4]> {
    #[cfg(target_os = "macos")]
    return skia::recorded_bounds(prepaint);
    #[cfg(not(target_os = "macos"))]
    {
        prepaint();
        None
    }
}

/// Paints a subtree on its recorded compositor tile grid without adding a layer.
pub fn with_recorded_bounds<R>(bounds: Option<[i32; 4]>, paint: impl FnOnce() -> R) -> R {
    #[cfg(target_os = "macos")]
    return skia::with_recorded_bounds(bounds, paint);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bounds;
        paint()
    }
}

/// Paints a group and applies byte opacity with the compositor's integer alpha scaling.
/// Returns the closure's result without changing it.
/// The bounds must include all of the group's drawn pixels.
pub fn with_composited_opacity<R>(
    alpha: u8,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    #[cfg(target_os = "macos")]
    {
        with_layer(255, bounds, window, |window| {
            let result = paint(window);
            mask::paint_opacity(alpha, bounds, window, cx);
            result
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        with_layer(alpha, bounds, window, paint)
    }
}

/// Paints a tightly bounded compositor recording, including its shadow overflow.
/// Take `bounds` from [`recorded_bounds`], which clips them to the current content mask.
pub fn with_composited_bounds<R>(bounds: Option<[i32; 4]>, paint: impl FnOnce() -> R) -> R {
    #[cfg(target_os = "macos")]
    return skia::with_composited_bounds(bounds, paint);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bounds;
        paint()
    }
}

/// Returns the icon's painted bounds in window coordinates, including its strokes.
/// Bounds round outward to device pixels and remain inside the SVG viewport.
/// Returns `None` for an unsupported or empty SVG, or on other platforms.
pub fn icon_paint_bounds(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    flip_x: bool,
    window: &Window,
) -> Option<Bounds<Pixels>> {
    #[cfg(target_os = "macos")]
    return skia::icon_paint_bounds(src, key, bounds, flip_x, window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (src, key, bounds, flip_x, window);
        None
    }
}

/// Paints the svg icon `src` (asset bytes; `key` names it for the cache) into `bounds`,
/// mirrored horizontally if `flip_x`, in `color`; `layer` as in [`paint`].
/// An explicit layer origin composites the complete SVG, including overlapping paths.
/// False if this platform or svg is not supported: paint it another way then.
#[allow(clippy::too_many_arguments)]
pub fn paint_icon(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    flip_x: bool,
    color: Hsla,
    layer: Option<Point<Pixels>>,
    window: &mut Window,
    cx: &App,
) -> bool {
    #[cfg(target_os = "macos")]
    return skia::paint_icon(src, key, bounds, flip_x, color, layer, None, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (src, key, bounds, flip_x, color, layer, window, cx);
        false
    }
}

/// Paints the svg icon `src` with CSS `opacity` inside the innermost [`backdrop_blur`] box
/// painted around `bounds`, as Chromium rasters a non-composited opacity group on that box's
/// composited layer and composites the layer over the blurred backdrop; `clip`: the box clips
/// its content to its contour (`overflow: hidden`). Draw nothing else in the icon's box after
/// the backdrop box. False when no such box holds `bounds`, or off macOS: paint it another way
/// then.
pub fn paint_icon_in_backdrop(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    color: Hsla,
    opacity: f32,
    clip: bool,
    window: &mut Window,
) -> bool {
    #[cfg(target_os = "macos")]
    return backdrop::paint_icon(src, key, bounds, color, opacity, clip, window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (src, key, bounds, color, opacity, clip, window);
        false
    }
}

/// Paints an icon as [`paint_icon`] does, then transforms its cached raster in device pixels.
/// The transform does not change layout, hitboxes, or the raster cache key.
#[allow(clippy::too_many_arguments)]
pub fn paint_icon_transformed(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    flip_x: bool,
    color: Hsla,
    layer: Option<Point<Pixels>>,
    transformation: gpui::TransformationMatrix,
    window: &mut Window,
    cx: &App,
) -> bool {
    #[cfg(target_os = "macos")]
    return skia::paint_icon(
        src,
        key,
        bounds,
        flip_x,
        color,
        layer,
        Some(transformation),
        window,
        cx,
    );
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            src,
            key,
            bounds,
            flip_x,
            color,
            layer,
            transformation,
            window,
            cx,
        );
        false
    }
}

/// GPUI's own primitives, with Blink's radius constraint.
fn quads(d: &Decoration, bounds: Bounds<Pixels>, window: &mut Window) {
    use gpui::{BorderStyle, BoxShadow, Edges, quad, transparent_black};
    let r = &d.radius;
    let sum = |a: Pixels, b: Pixels| f32::from(a + b);
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let f = [
        (w, sum(r.top_left, r.top_right)),
        (w, sum(r.bottom_left, r.bottom_right)),
        (h, sum(r.top_left, r.bottom_left)),
        (h, sum(r.top_right, r.bottom_right)),
    ]
    .into_iter()
    .filter(|&(_, s)| s > 0.)
    .fold(1f32, |f, (side, s)| f.min(side / s));
    let radius = Corners {
        top_left: r.top_left * f,
        top_right: r.top_right * f,
        bottom_right: r.bottom_right * f,
        bottom_left: r.bottom_left * f,
    };
    let gpui = |s: &Shadow| BoxShadow {
        color: s.color,
        offset: s.offset,
        // GPUI's blur radius is the Gaussian sigma: CSS blur / 2.
        blur_radius: s.blur / 2.,
        spread_radius: s.spread,
        inset: s.inset,
    };
    let outer: Vec<_> = d
        .shadows
        .iter()
        .rev()
        .filter(|s| !s.inset)
        .map(gpui)
        .collect();
    window.paint_drop_shadows(bounds, radius, &outer);
    if let Some(color) = d.fill {
        let none = transparent_black();
        window.paint_quad(quad(
            bounds,
            radius,
            color,
            Edges::default(),
            none,
            BorderStyle::Solid,
        ));
    }
    let inset: Vec<_> = d
        .shadows
        .iter()
        .rev()
        .filter(|s| s.inset)
        .map(gpui)
        .collect();
    window.paint_inset_shadows(bounds, radius, &inset);
    if let Some((width, color)) = d.border {
        let none = transparent_black();
        window.paint_quad(quad(
            bounds,
            radius,
            none,
            Edges::all(width),
            color,
            if d.dashed {
                BorderStyle::Dashed
            } else {
                BorderStyle::Solid
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::scaled_rect;

    #[test]
    fn scaled_rect_keeps_blink_transform_origin_fraction() {
        // The Explore dot at 110% on a 2x display: the 16 px box snaps to [62, 97] device px,
        // and `scale(0.5)` is about 50% of its 35.1875 px layout box from the snapped offset.
        let origin = [62. + 35.1875 / 2., 497. + 35.1875 / 2.];
        assert_eq!(
            scaled_rect([62., 497., 35., 35.], origin, 0.5),
            [70.796875, 497. + 8.796875, 17.5, 17.5]
        );
        // At 2x every edge is whole: the 32 px box scales to 16 px about its center.
        assert_eq!(
            scaled_rect([64., 464., 32., 32.], [80., 480.], 0.5),
            [72., 472., 16., 16.]
        );
    }
}
