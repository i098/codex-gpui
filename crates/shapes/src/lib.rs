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
//! Colors outside [0, 1] (out-of-gamut oklab/color-mix results) are drawn as Skia draws them,
//! with its float raster pipeline. Limits: inset shadows paint apart from Blink's clip layer.
//! A fill under a CSS `opacity` rounds 1 level darker on its anti-aliased edge.

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
    /// `border-style: dashed` (round corners; Blink's curved dashed side).
    pub dashed: bool,
    /// CSS order: the first shadow is the top layer.
    pub shadows: Vec<Shadow>,
    /// cc partial raster, fitted to a capture: per root-layer tile origin (device px), the
    /// playback rect (device px) Chromium last rastered that tile through. Skia then starts
    /// the AA clips and saveLayer devices on that rect, which moves edge px by one or two.
    /// ponytail: the rects are capture timing (svg reloads invalidate tiles); drop them with a
    /// deterministic capture.
    pub playback: &'static [([i32; 2], [i32; 4])],
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
/// (`Window::composited_layer`), else the root layer.
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

/// `images` (premultiplied RGBA, see [`premultiply`], each drawn into its `dst` rect in
/// window px) over `page`, inside the anti-aliased clip of the border box `clip` with `radius`
/// and `corner`: an `<img>` or `background-image` in an `overflow: hidden` rounded box,
/// rasterized by Skia (bilinear, Chromium's span and tile restarts included) once per cc
/// tile. Over the box's snapped device px rect; `None` off macOS.
pub fn raster_images(
    images: &[(&image::RgbaImage, Bounds<Pixels>)],
    clip: Bounds<Pixels>,
    radius: Corners<Pixels>,
    corner: Corner,
    page: Hsla,
    window: &Window,
) -> Option<gpui::RenderImage> {
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

/// CSS `mask-image: linear-gradient(to right, transparent, black <left>, black calc(100% -
/// <right>), transparent)` over its parent (`.text-fade-truncate` has `left` 0), which must be
/// a `.layer()` element, like [`top_fade`]: put it last. macOS only, like [`top_fade`].
pub fn x_fade(left: Pixels, right: Pixels) -> impl IntoElement {
    cover(move |bounds, window, cx| {
        #[cfg(target_os = "macos")]
        mask::paint_x_fade(left, right, bounds, window, cx);
        #[cfg(not(target_os = "macos"))]
        let _ = (left, right, bounds, window, cx);
    })
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

/// `Window::with_skia_layer(alpha, paint)` for a Blink `saveLayerAlpha` with `bounds` (window
/// px; a CSS `opacity` on a box): Skia draws into the layer's own device, where an ancestor's
/// anti-aliased clip ([`with_rounded_clip`]) that covers `bounds` is a plain rect clip, so the
/// shapes painted in it raster without that clip.
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

/// Paints the svg icon `src` (asset bytes; `key` names it for the cache) into `bounds`,
/// mirrored horizontally if `flip_x`, in `color`; `layer` as in [`paint`]. `partial`: cc
/// re-rasters only the icon's box (partial raster of an invalidated tile), so the
/// [`with_rounded_clip`] clips start on that box and stay anti-aliased in every tile.
/// False if this platform or svg is not supported: paint it another way then.
#[allow(clippy::too_many_arguments)]
pub fn paint_icon(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    flip_x: bool,
    color: Hsla,
    layer: Option<Point<Pixels>>,
    partial: bool,
    window: &mut Window,
    cx: &App,
) -> bool {
    #[cfg(target_os = "macos")]
    return skia::paint_icon(src, key, bounds, flip_x, color, layer, partial, window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (src, key, bounds, flip_x, color, layer, partial, window, cx);
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
            if d.dashed {
                BorderStyle::Dashed
            } else {
                BorderStyle::Solid
            },
        ));
    }
}
