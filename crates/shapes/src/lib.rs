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
//! calls Blink makes, once per cc raster tile of the window (`src/skia.rs`). The result is
//! read back as per-pixel coverage and blend mode and painted as GPUI monochrome sprites
//! (`Window::paint_svg` with an SVG wrapping the mask as a nearest-neighbor PNG); the vendored
//! gpui_apple shader blends each with Skia's integer formula for that mode. Elsewhere it
//! falls back to GPUI's quad and shadow primitives, which are not Skia-exact.
//!
//! Composited layers: put the content in a GPUI `.layer()` div whose origin is Chromium's
//! layer origin (its ink start: shadows included) and paint with `layer: None`. Every op is
//! rasterized in that layer's space (Blink's float geometry depends on it) and blitted into
//! the layer, which is composited once, as cc does. Keep GPUI quads (`.bg()`) out of such a
//! layer: they bypass it. Without a GPUI layer, [`paint`]'s `layer` gives the origin and each
//! op is composited on its own.
//!
//! Limits: inset shadows paint apart from Blink's clip layer. A translucent `superellipse`
//! border (alone or in the clip layer) comes back from its transparency layer as
//! `SkBlendARGB32` at the clip coverage; Blink paints its sides one by one, and some
//! horizontal sides (in some cc tiles) come back 1 level brighter. A `superellipse` shadow
//! with a large blur and spread is 1 level off in its far tail. A fill under a CSS `opacity`
//! rounds 1 level darker on its anti-aliased edge.

use gpui::{App, Bounds, Corners, Hsla, IntoElement, Pixels, Point, Styled as _, Window, canvas};

#[cfg(target_os = "macos")]
mod backdrop;
#[cfg(target_os = "macos")]
mod mask;
#[cfg(target_os = "macos")]
mod skia;

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
    /// CSS order: the first shadow is the top layer.
    pub shadows: Vec<Shadow>,
}

/// An element covering its parent's box that paints `d`.
pub fn decoration(d: Decoration) -> impl IntoElement {
    cover(move |bounds, window, cx| paint(&d, None, bounds, window, cx))
}

/// An element covering its parent's box, painted by `paint`.
fn cover(paint: impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| paint(bounds, window, cx),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Paints `d` for the border box `bounds`. `layer`: for a box that Chromium paints into a
/// composited layer (a scroll container's contents), that layer's origin in window
/// coordinates: the layer's tiles start there and are transparent, and the compositor
/// blends them onto the page. `None` takes the innermost GPUI `.layer()` div being painted
/// (`Window::composited_layer_origin`), else the root layer.
pub fn paint(
    d: &Decoration,
    layer: Option<Point<Pixels>>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    #[cfg(target_os = "macos")]
    skia::paint(d, layer, bounds, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (cx, layer);
        quads(d, bounds, window);
    }
}

/// `images` (premultiplied RGBA, each drawn into its `dst` rect in window px) over `page`,
/// inside the anti-aliased clip of the border box `clip` with `radius` and `corner`: an
/// `<img>` or `background-image` in an `overflow: hidden` rounded box, rasterized by Skia
/// (bilinear, Chromium's span and tile restarts included) once per cc tile. RGBA,
/// row-major over the box's snapped device px rect; `None` off macOS.
pub fn raster_images(
    images: &[(&image::RgbaImage, Bounds<Pixels>)],
    clip: Bounds<Pixels>,
    radius: Corners<Pixels>,
    corner: Corner,
    page: Hsla,
    window: &Window,
) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    return Some(skia::raster_images(
        images, clip, radius, corner, page, window,
    ));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (images, clip, radius, corner, page, window);
        None
    }
}

/// An element covering its parent's box that paints `fill` as a square background under
/// `clip-path: inset(... round radius)` (the clip's corners on this box): Skia draws the rect
/// inside an anti-aliased clip, whose edge differs from a rounded [`decoration`] fill.
pub fn clip_path_fill(radius: Corners<Pixels>, fill: Hsla) -> impl IntoElement {
    cover(move |bounds, window, cx| {
        #[cfg(target_os = "macos")]
        skia::paint_clip_path_fill(radius, fill, bounds, window, cx);
        #[cfg(not(target_os = "macos"))]
        paint(
            &Decoration {
                radius,
                fill: Some(fill),
                ..Default::default()
            },
            None,
            bounds,
            window,
            cx,
        );
    })
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
/// blurs what was painted below the box (here: what `shapes` painted this frame), clips it
/// to the box's shape, and composites the box's own layer (`d`, its fill painted as `fill`)
/// over it. A box inside another `backdrop_blur` box (a nested `backdrop-filter`) blurs that
/// box's layer instead. Paint it where the box's decoration goes (first child); its text and
/// children paint on top as usual.
pub fn backdrop_blur(blur: Pixels, fill: Fill, d: Decoration) -> impl IntoElement {
    cover(move |bounds, window, _| {
        #[cfg(target_os = "macos")]
        backdrop::paint(blur, fill, &d, bounds, window);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (blur, fill);
            quads(&d, bounds, window);
        }
    })
}

/// CSS `mask-image: linear-gradient(to bottom, transparent, black <fade>)` over its parent,
/// which must be a `.layer()` element (Chromium masks the composited layer): put it last so it
/// multiplies everything painted into that layer by the gradient's alpha (`DstIn`) before the
/// layer is composited. The rows past `fade` are left as they are. macOS only: elsewhere GPUI
/// has no layer pixels to mask and nothing is painted.
pub fn top_fade(fade: Pixels) -> impl IntoElement {
    cover(move |bounds, window, cx| {
        #[cfg(target_os = "macos")]
        mask::paint_top_fade(fade, bounds, window, cx);
        #[cfg(not(target_os = "macos"))]
        let _ = (fade, bounds, window, cx);
    })
}

/// Paints the svg icon `src` (asset bytes; `key` names it for the cache) into `bounds`,
/// mirrored horizontally if `flip_x`, in `color`; `layer` as in [`paint`].
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
    return skia::paint_icon(src, key, bounds, flip_x, color, layer, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (src, key, bounds, flip_x, color, layer, window, cx);
        false
    }
}

/// GPUI's own primitives, with Blink's radius constraint.
#[cfg(not(target_os = "macos"))]
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
            BorderStyle::Solid,
        ));
    }
}
