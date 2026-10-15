//! Chromium's own raster at runtime (macOS). Each decoration layer and icon path is drawn with
//! the Skia build chrome-headless-shell uses (skia-safe linked against the archive from
//! tools/shapes/skia.sh) through the same calls Blink makes, once per cc raster tile (512 px
//! bitmaps at a 510 px stride, tile `k` owning layer px `510k+1 ..= 510k+510`). The tiles are
//! read back as per-pixel coverage (white on black) and blitter (opaque red on white: the
//! green channel tells `blitAntiH` from `blitAntiH2`), and painted as GPUI monochrome sprites
//! whose hue carries the blend mode to the patched shader (crates/textsys/gpui_apple).

use crate::{Corner, Decoration};
use base64::Engine as _;
use gpui::{
    App, Bounds, Hsla, Pixels, Rgba, SharedString, TransformationMatrix, Window, point, px, size,
};
use image::{ExtendedColorType, ImageEncoder as _, codecs::png::PngEncoder};
use parking_lot::Mutex;
use skia_safe::{
    BlendMode, BlurStyle, Canvas, ClipOp, Color4f, ColorSpace, MaskFilter, Matrix, Paint, PaintCap,
    PaintJoin, PaintStyle, Path, PathBuilder, PathFillType, PathOp, RRect, Rect, Vector,
    color_filters, surfaces,
};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    sync::{Arc, LazyLock},
};

const TILE: i32 = 512;
const STRIDE: i32 = 510;
/// Sprites are cut into chunks of at most this many device px; equal chunks share one atlas
/// entry.
const CHUNK: i32 = 128;
/// Blend modes of the shader (`hue + 2 * mode`): legacy `blitAntiH`/`blitMask`/`blitV`,
/// legacy `blitAntiH2`/`blitAntiV2` (`SkBlendARGB32`), raster pipeline lowp srcover.
const MASK: u8 = 0;
const ARGB32: u8 = 1;
const PIPELINE: u8 = 2;
/// Blit into the transparent tile of a composited layer, then the compositor's srcover.
const LAYER: u8 = 3;

/// gfx::SizeF's clamp: dimensions at or below 8 epsilon are 0.
fn clamp(v: f32) -> f32 {
    if v > 8. * f32::EPSILON { v } else { 0. }
}

/// Blink `FloatRoundedRect` in device px: `[x, y, w, h]` and `[w, h]` radii tl, tr, br, bl.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rounded {
    r: [f32; 4],
    radii: [[f32; 2]; 4],
}

impl Rounded {
    fn is_rounded(&self) -> bool {
        self.radii.iter().any(|c| *c != [0., 0.])
    }

    fn is_empty(&self) -> bool {
        self.r[2] == 0. || self.r[3] == 0.
    }

    fn rect(&self) -> Rect {
        rect(self.r)
    }

    fn sk(&self) -> RRect {
        RRect::new_rect_radii(self.rect(), &self.radii.map(|[w, h]| Vector::new(w, h)))
    }

    /// `FloatRoundedRect::Outset`: rect and every non-zero radius dimension.
    fn outset(&mut self, o: f32) {
        self.r = outset(self.r, o);
        for d in self.radii.as_flattened_mut() {
            if *d > 0. {
                *d = clamp(*d + o);
            }
        }
    }

    /// `FloatRoundedRect::OutsetWithCornerCorrection` (css-backgrounds-3 corner shaping).
    fn outset_corrected(&mut self, o: f32) {
        if o == 0. {
            return;
        }
        let edge = [self.r[2], self.r[3]];
        self.r = outset(self.r, o);
        if edge[0] == 0. || edge[1] == 0. {
            return;
        }
        let adjust = |radius: f32, coverage: f32| -> f32 {
            if radius > o || coverage > 1. {
                return radius + o;
            }
            let ratio = radius / o;
            let (ratio, coverage) = (f64::from(ratio), f64::from(coverage));
            (f64::from(radius)
                + f64::from(o) * (1. - (1. - ratio).powf(3.) * (1. - coverage.powf(3.))))
                as f32
        };
        for c in &mut self.radii {
            if *c == [0., 0.] {
                continue;
            }
            let coverage = 2. * (c[0] / edge[0]).min(c[1] / edge[1]);
            *c = [clamp(adjust(c[0], coverage)), clamp(adjust(c[1], coverage))];
        }
    }

    /// `FloatRoundedRect::ConstrainRadii`.
    fn constrain(&mut self) {
        let [tl, tr, br, bl] = self.radii;
        let mut factor = 1f32;
        let horizontal = (tl[0] + tr[0]).max(bl[0] + br[0]);
        if horizontal > self.r[2] {
            factor = (self.r[2] / horizontal).min(factor);
        }
        let vertical = (tl[1] + bl[1]).max(tr[1] + br[1]);
        if vertical > self.r[3] {
            factor = (self.r[3] / vertical).min(factor);
        }
        if factor == 1. {
            return;
        }
        for c in &mut self.radii {
            *c = c.map(|v| clamp(v * factor));
            if c[0] == 0. || c[1] == 0. {
                *c = [0., 0.];
            }
        }
    }

    fn renderable(&self) -> bool {
        let t = 1.0001;
        let [tl, tr, br, bl] = self.radii;
        tl[0] + tr[0] <= self.r[2] * t
            && bl[0] + br[0] <= self.r[2] * t
            && tl[1] + bl[1] <= self.r[3] * t
            && tr[1] + br[1] <= self.r[3] * t
    }
}

/// `gfx::RectF::Outset`.
fn outset(r: [f32; 4], o: f32) -> [f32; 4] {
    [
        r[0] - o,
        r[1] - o,
        clamp(r[2] - (-o + -o)),
        clamp(r[3] - (-o + -o)),
    ]
}

/// Blink `ContouredRect`: the rounded rect, per-corner curvature (2 = round, `2^1.5` =
/// `superellipse(1.5)`) and the origin rect the shape was derived from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Contour {
    rr: Rounded,
    k: [f32; 4],
    origin: Rounded,
}

impl Contour {
    fn is_rounded(&self) -> bool {
        self.rr.is_rounded() || self.origin.is_rounded()
    }

    fn round_curvature(&self) -> bool {
        self.k.iter().all(|&k| k == 2.) || !self.is_rounded()
    }

    /// `PathBuilder::AddContouredRect`.
    /// ponytail: when the rect differs from its origin rect (spread shadows, inner border
    /// edges) Blink clips each corner with path ops; this draws the plain contour there.
    fn path(&self) -> Path {
        if self.round_curvature() {
            return Path::rrect(self.rr.sk(), None);
        }
        let [x, y, w, h] = self.rr.r;
        let (r, b) = (x + w, y + h);
        let [tl, tr, br, bl] = self.rr.radii;
        let corners = [
            corner([r - tr[0], y, tr[0], tr[1]], 0, self.k[1]),
            corner([r - br[0], b - br[1], br[0], br[1]], 1, self.k[2]),
            corner([x, b - bl[1], bl[0], bl[1]], 2, self.k[3]),
            corner([x, y, tl[0], tl[1]], 3, self.k[0]),
        ];
        let mut path = PathBuilder::new();
        path.move_to(sk_point(corners[0].0[0]));
        for (v, k) in corners {
            curved_corner(&mut path, v, k);
        }
        path.close();
        path.detach()
    }

    fn fill(&self, c: &Canvas, paint: &Paint) {
        if !self.round_curvature() {
            c.draw_path(&self.path(), paint);
        } else if !self.rr.is_rounded() || !self.rr.renderable() {
            c.draw_rect(self.rr.rect(), paint);
        } else {
            c.draw_rrect(self.rr.sk(), paint);
        }
    }

    /// `GraphicsContext::ClipContouredRect`, anti-aliased.
    fn clip(&self, c: &Canvas, op: ClipOp) {
        if !self.is_rounded() {
            c.clip_rect(self.rr.rect(), op, true);
        } else if self.round_curvature() {
            c.clip_rrect(self.rr.sk(), op, true);
        } else {
            c.clip_path(&self.path(), op, true);
        }
    }
}

type Pt = [f32; 2];

fn sk_point([x, y]: Pt) -> skia_safe::Point {
    skia_safe::Point::new(x, y)
}

/// `ContouredRect::Corner(rect, rotation, curvature)`: start, outer, end, center.
fn corner(r: [f32; 4], rotation: usize, k: f32) -> ([Pt; 4], f32) {
    let mut v = [
        [r[0], r[1]],
        [r[0] + r[2], r[1]],
        [r[0] + r[2], r[1] + r[3]],
        [r[0], r[1] + r[3]],
    ];
    v.rotate_left(rotation);
    (v, k)
}

/// `AddCurvedCorner` for convex corners (Blink path_builder.cc).
fn curved_corner(path: &mut PathBuilder, [start, outer, end, center]: [Pt; 4], k: f32) {
    let sub = |a: Pt, b: Pt| [a[0] - b[0], a[1] - b[1]];
    let (v1, v2, v4) = (sub(outer, start), sub(end, outer), sub(start, center));
    let [start, outer, end] = [start, outer, end].map(sk_point);
    path.line_to(start);
    if k == 1000. || v1 == [0., 0.] || v2 == [0., 0.] {
        path.line_to(outer);
        path.line_to(end);
    } else if k == 1. {
        path.line_to(end);
    } else if k == 2. {
        path.conic_to(outer, end, std::f32::consts::FRAC_1_SQRT_2);
    } else {
        let map = |[x, y]: Pt| {
            let p = [center[0] + v1[0] * x, center[1] + v1[1] * x];
            sk_point([p[0] + v4[0] * y, p[1] + v4[1] * y])
        };
        let [c1, c2, c3] = superellipse_half_corner(k);
        path.cubic_to(map(c1), map(c2), map(c3));
        path.cubic_to(map([c2[1], c2[0]]), map([c1[1], c1[0]]), end);
    }
}

/// `ApproximateSuperellipseHalfCornerAsBezierCurve`, with C++'s float/double promotions.
fn superellipse_half_corner(curvature: f32) -> [Pt; 3] {
    const P: [f64; 7] = [
        1.2430920942724248,
        2.010479023614843,
        0.32922901179443753,
        0.2823023142212073,
        1.3473704261055421,
        2.9149468637949814,
        0.9106507102917086,
    ];
    let s = curvature.log2();
    let sd = f64::from(s);
    let slope = (P[0] + (P[6] - P[0]) * 0.5 * (1. + (P[5] * (sd - P[1])).tanh())) as f32;
    let base = (1. / (1. + (f64::from(-slope) * (0. - P[1])).exp())) as f32;
    let logistic = (1. / (1. + (f64::from(-slope) * (sd - P[1])).exp())) as f32;
    let a = (logistic - base) / (1. - base);
    let b = (P[2] * (-P[3] * sd.powf(P[4])).exp()) as f32;
    let half = 0.5f64.powf(f64::from(1. / curvature.clamp(1. / 1000., 1000.))) as f32;
    [[a, 1.], [half - b, half + b], [half, half]]
}

/// One Blink paint op of a decoration, device px in layer space.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    /// `FillContouredRect` (background color).
    Fill(Contour),
    /// Uniform solid border (`BoxBorderPainter`).
    Border { outer: Contour, inner: Contour },
    /// Outer `box-shadow` layer (`PaintNormalBoxShadow`).
    Shadow {
        border: Contour,
        opaque_bg: bool,
        has_radius: bool,
        s: [f32; 4],
    },
    /// Inset `box-shadow` layer (`PaintInsetBoxShadow`); `bounds` is the padding box.
    Inset { bounds: Contour, s: [f32; 4] },
}

/// Shadow params `[x, y, blur, spread]`: the draw looper's paint for color `col`.
fn looper(c: &Canvas, [x, y, blur, _]: [f32; 4], col: Color4f) -> Paint {
    c.translate((x, y));
    let mut paint = Paint::new(Color4f::new(0., 0., 0., 1.), None);
    paint.set_anti_alias(true);
    if blur > 0. {
        paint.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, blur * 0.5, true));
    }
    paint.set_color_filter(color_filters::blend_with_color_space(
        col,
        ColorSpace::new_srgb(),
        BlendMode::SrcIn,
    ));
    paint
}

fn fill_paint(col: Color4f) -> Paint {
    let mut paint = Paint::new(col, None);
    paint.set_anti_alias(true);
    paint
}

impl Op {
    /// The inset shadow's hole (`inner_contoured_rect`).
    fn hole(bounds: &Contour, spread: f32) -> Contour {
        let mut hole = *bounds;
        if spread != 0. {
            hole.rr.outset_corrected(-spread);
            hole.rr.constrain();
        }
        hole
    }

    /// Device px the op can touch: `[x0, y0, x1, y1]`.
    fn bbox(&self) -> [i32; 4] {
        let r = match self {
            Op::Fill(c) | Op::Border { outer: c, .. } | Op::Inset { bounds: c, .. } => c.rr.r,
            Op::Shadow { border, s, .. } => {
                let r = outset(border.rr.r, s[3].abs() + (1.5 * s[2]).ceil() + 2.);
                [r[0] + s[0], r[1] + s[1], r[2], r[3]]
            }
        };
        [
            r[0].floor() as i32 - 1,
            r[1].floor() as i32 - 1,
            (r[0] + r[2]).ceil() as i32 + 1,
            (r[1] + r[3]).ceil() as i32 + 1,
        ]
    }

    fn draw(&self, c: &Canvas, col: Color4f) {
        match *self {
            Op::Fill(contour) if contour.round_curvature() => contour.fill(c, &fill_paint(col)),
            Op::Fill(contour) => {
                // Blink's slow background path (`RoundedInnerRectClipper`): the snapped rect
                // inside an anti-aliased clip of the contour.
                // ponytail: unrenderable radii clip corner by corner in Blink; not drawn here.
                c.save();
                contour.clip(c, ClipOp::Intersect);
                c.draw_rect(contour.rr.rect(), &fill_paint(col));
                c.restore();
            }
            Op::Border { outer, inner } => {
                let mut paint = fill_paint(col);
                if inner.rr.renderable() && inner.round_curvature() {
                    let width = inner.rr.r[0] - outer.rr.r[0];
                    paint.set_style(PaintStyle::Stroke).set_stroke_width(width);
                    if !outer.rr.is_rounded() {
                        // DrawSolidBorderRect
                        c.draw_rect(
                            rect(outset(outer.rr.r, -width / 2.).map(f32::round)),
                            &paint,
                        );
                    } else if simple_drrect(&outer.rr, &inner.rr) {
                        let mut rr = outer.rr.sk();
                        rr.inset((width / 2., width / 2.));
                        c.draw_rrect(rr, &paint);
                    } else {
                        c.draw_drrect(outer.rr.sk(), inner.rr.sk(), &fill_paint(col));
                    }
                } else {
                    // ponytail: Blink's slow border path draws the four sides inside these
                    // clips (and a transparency layer for translucent colors); one fill of
                    // the clipped ring stands in for it.
                    c.save();
                    outer.clip(c, ClipOp::Intersect);
                    if inner.rr.renderable() && !inner.rr.is_empty() {
                        inner.clip(c, ClipOp::Difference);
                    }
                    c.draw_rect(outer.rr.rect(), &paint);
                    c.restore();
                }
            }
            Op::Shadow {
                border,
                opaque_bg,
                has_radius,
                s,
            } => {
                let r = outset(border.rr.r, s[3]);
                if r[2] == 0. || r[3] == 0. {
                    return;
                }
                c.save();
                let mut clip = border;
                if opaque_bg {
                    clip.rr.outset(-1.);
                }
                if !clip.rr.is_empty() {
                    if has_radius {
                        clip.clip(c, ClipOp::Difference);
                    } else {
                        c.clip_rect(clip.rr.rect(), ClipOp::Difference, false);
                    }
                }
                let paint = looper(c, s, col);
                if has_radius {
                    Self::hole(&border, -s[3]).fill(c, &paint);
                } else {
                    c.draw_rect(rect(outset(border.rr.r, s[3])), &paint);
                }
                c.restore();
            }
            Op::Inset { bounds, s } => {
                let hole = Self::hole(&bounds, s[3]);
                if hole.rr.is_empty() {
                    bounds.fill(c, &fill_paint(col));
                    return;
                }
                c.save();
                if bounds.is_rounded() {
                    bounds.clip(c, ClipOp::Intersect);
                } else {
                    c.clip_rect(bounds.rr.rect(), ClipOp::Intersect, false);
                }
                let paint = looper(c, s, col);
                // AreaCastingShadowInHole
                let mut area = outset(bounds.rr.r, s[2]);
                if s[3] < 0. {
                    area = outset(area, -s[3]);
                }
                let moved = [area[0] - s[0], area[1] - s[1], area[2], area[3]];
                let (x0, y0) = (area[0].min(moved[0]), area[1].min(moved[1]));
                let x1 = (area[0] + area[2]).max(moved[0] + moved[2]);
                let y1 = (area[1] + area[3]).max(moved[1] + moved[3]);
                let area = Rect::new(x0, y0, x1, y1);
                if hole.round_curvature() {
                    c.draw_drrect(RRect::new_rect(area), hole.rr.sk(), &paint);
                } else if let Some(p) = Path::rect(area, None).op(&hole.path(), PathOp::Difference)
                {
                    c.draw_path(&p, &paint);
                }
                c.restore();
            }
        }
    }
}

fn rect(r: [f32; 4]) -> Rect {
    Rect::from_xywh(r[0], r[1], r[2], r[3])
}

/// `IsSimpleDRRect` (graphics_context.cc): drawable as one stroked rrect.
fn simple_drrect(outer: &Rounded, inner: &Rounded) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() <= f32::EPSILON * a.abs().max(b.abs()).max(1.);
    let stroke = [inner.r[0] - outer.r[0], inner.r[1] - outer.r[1]];
    if !near(stroke[0], stroke[1])
        || !near(
            stroke[0],
            outer.r[0] + outer.r[2] - (inner.r[0] + inner.r[2]),
        )
        || !near(
            stroke[1],
            outer.r[1] + outer.r[3] - (inner.r[1] + inner.r[3]),
        )
    {
        return false;
    }
    outer.radii.iter().zip(&inner.radii).all(|(o, i)| {
        (*o == [0., 0.] && *i == [0., 0.])
            || near(o[0], o[1]) && near(i[0], i[1]) && near(o[0], i[0] + stroke[0])
    })
}

/// The patched shader's blend of one 8-bit channel: unpremultiplied color `c` with alpha `a`
/// over `d` at coverage `m` (crates/textsys/gpui_apple `monochrome_sprite_fragment`).
fn blend(mode: u8, c: u8, a: u8, d: u8, m: u8) -> u8 {
    let (c, a, d, m) = (u32::from(c), u32::from(a), u32::from(d), u32::from(m));
    let div255 = |v: u32| (v + ((v + 128) >> 8) + 128) >> 8;
    let src = div255(c * a);
    let scale = m + 1;
    (match mode {
        ARGB32 if m == 0 => d,
        ARGB32 => {
            let inv = 0xFFFF - a * scale;
            ((src * scale + d * ((inv + (inv >> 8)) >> 8)) >> 8) & 0xFF
        }
        PIPELINE => div255(src * m) + div255(d * (255 - div255(a * m))),
        _ => ((src * scale) >> 8) + ((d * (256 - ((a * scale) >> 8))) >> 8),
    }) as u8
}

/// Coverage and blend mode of up to two blits per pixel: `[m1, mode1, m2, mode2]`.
type Blits = [u8; 4];

/// A probe paint: unpremultiplied rgb and alpha over an opaque rgb background.
#[derive(Clone, Copy)]
struct Probe {
    fg: [u8; 3],
    a: u8,
    bg: [u8; 3],
}

/// The cc layer a draw lands in, device px: the window's root layer, or a composited layer
/// (a scroll container's contents) whose transparent tiles the compositor blends onto the
/// page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layer {
    /// Layer origin in the window; the tile grid starts here.
    origin: [i32; 2],
    /// Window size (bounds the root layer's tiles).
    window: [i32; 2],
    composited: bool,
}

impl Layer {
    pub(crate) fn new(window: &Window, composited: Option<gpui::Point<Pixels>>) -> Self {
        let (vp, s) = (window.viewport_size(), window.scale_factor());
        let dev = |v: Pixels| (f32::from(v) * s).round() as i32;
        let origin = composited.map_or([0, 0], |o| [dev(o.x), dev(o.y)]);
        let window = [dev(vp.width), dev(vp.height)];
        Self {
            origin,
            window,
            composited: composited.is_some(),
        }
    }
}

/// The blits of each pixel of `[x0, y0, x1, y1]` (window device px, clamped to the layer),
/// rendering `draw` once per cc tile like Chromium's raster worker. Skia picks its blitter by
/// the paint (opaque legacy, translucent, color filter), so the probes that tell the modes
/// apart draw with the real color's `alpha`. AAA blits some edge pixels twice (a partial
/// row, then the rest of it): those are solved for two blits. In a composited layer every
/// pixel is one [`LAYER`] blit of its coverage.
fn raster(
    [x0, y0, x1, y1]: [i32; 4],
    layer: Layer,
    alpha: u8,
    draw: &dyn Fn(&Canvas, Color4f),
) -> ([i32; 4], Vec<Blits>) {
    // Layer space from here on.
    let [lx, ly] = layer.origin;
    let size = [layer.window[0] - lx, layer.window[1] - ly];
    let (x0, y0) = ((x0 - lx).max(0), (y0 - ly).max(0));
    let (x1, y1) = ((x1 - lx).min(size[0]), (y1 - ly).min(size[1]));
    if x0 >= x1 || y0 >= y1 {
        return ([0; 4], Vec::new());
    }
    let alpha = (!layer.composited).then_some(alpha);
    // The first probe (opaque white on black) reads the coverage; the others, with distinct
    // channels, tell the blits apart (`blitAntiH`, `blitAntiH2`, raster pipeline).
    let mut probes = vec![Probe {
        fg: [255; 3],
        a: 255,
        bg: [0; 3],
    }];
    let pairs = [
        ([255, 0, 0], [255, 255, 255]),
        ([26, 128, 230], [252, 36, 128]),
        ([200, 60, 100], [24, 200, 246]),
    ];
    let modes: &[u8] = match alpha {
        None => &[LAYER],
        Some(a) => {
            probes.extend(pairs.map(|(fg, bg)| Probe { fg, a, bg }));
            if a == 255 {
                &[MASK, ARGB32]
            } else {
                probes.push(Probe {
                    fg: [255; 3],
                    a,
                    bg: [0; 3],
                });
                &[MASK, ARGB32, PIPELINE]
            }
        }
    };
    let (w, h) = (x1 - x0, y1 - y0);
    let mut out = vec![[0, MASK, 0, MASK]; (w * h) as usize];
    let owner = |v: i32| if v <= 0 { 0 } else { (v - 1) / STRIDE };
    let color = |[r, g, b]: [u8; 3], a: u8| {
        let f = |v: u8| f32::from(v) / 255.;
        Color4f::new(f(r), f(g), f(b), f(a))
    };
    let tile = |o: [i32; 2], size: (i32, i32), p: Probe| {
        let mut surface = surfaces::raster_n32_premul(size).expect("shapes: tile surface");
        let canvas = surface.canvas();
        canvas.clear(color(p.bg, 255));
        canvas.translate(((-o[0] - lx) as f32, (-o[1] - ly) as f32));
        draw(canvas, color(p.fg, p.a));
        let pixmap = surface.peek_pixels().expect("shapes: tile pixels");
        (
            pixmap.bytes().expect("shapes: tile bytes").to_vec(),
            pixmap.row_bytes(),
        )
    };
    for ty in owner(y0)..=owner(y1 - 1) {
        for tx in owner(x0)..=owner(x1 - 1) {
            let o = [tx * STRIDE, ty * STRIDE];
            let tile_size = ((size[0] - o[0]).min(TILE), (size[1] - o[1]).min(TILE));
            let seen: Vec<_> = probes.iter().map(|&p| tile(o, tile_size, p)).collect();
            let own = |v: i32, t: i32| if t == 0 { 0 } else { v + 1 };
            for y in y0.max(own(o[1], ty))..y1.min(o[1] + STRIDE + 1) {
                for x in x0.max(own(o[0], tx))..x1.min(o[0] + STRIDE + 1) {
                    // RGBA bytes (macOS N32).
                    let i = (y - o[1]) as usize * seen[0].1 + (x - o[0]) as usize * 4;
                    let px: Vec<[u8; 3]> = seen
                        .iter()
                        .map(|s| [s.0[i], s.0[i + 1], s.0[i + 2]])
                        .collect();
                    let m = px[0][1];
                    if m == 0 {
                        continue;
                    }
                    out[((y - y0) * w + x - x0) as usize] = match alpha {
                        None => [m, LAYER, 0, MASK],
                        Some(_) => solve(&probes, &px, modes).unwrap_or([m, MASK, 0, MASK]),
                    };
                }
            }
        }
    }
    ([x0 + lx, y0 + ly, w, h], out)
}

/// One or two blits that reproduce every probe value `px`. The coverage probe (opaque) may
/// take other legacy modes than the real paint's when that is translucent.
fn solve(probes: &[Probe], px: &[[u8; 3]], modes: &[u8]) -> Option<Blits> {
    let run = |b: Blits, k: [u8; 2], c, a, d| {
        let d = blend(k[0], c, a, d, b[0]);
        if b[2] == 0 {
            d
        } else {
            blend(k[1], c, a, d, b[2])
        }
    };
    let probe = |b: Blits, k: [u8; 2], p: &Probe, v: &[u8; 3]| {
        (0..3).all(|i| run(b, k, p.fg[i], p.a, p.bg[i]) == v[i])
    };
    let legacy = [
        [MASK, MASK],
        [MASK, ARGB32],
        [ARGB32, MASK],
        [ARGB32, ARGB32],
    ];
    let opaque = probes[1].a == 255;
    let fits = |b: Blits| {
        let k = [b[1], b[3]];
        probes
            .iter()
            .zip(px)
            .skip(1)
            .all(|(p, v)| probe(b, k, p, v))
            && if opaque {
                probe(b, k, &probes[0], &px[0])
            } else {
                legacy.iter().any(|k| probe(b, *k, &probes[0], &px[0]))
            }
    };
    let m = px[0][1];
    let single = modes
        .iter()
        .map(|&mode| [m, mode, 0, MASK])
        .find(|&b| fits(b));
    single.or_else(|| {
        (1..=255)
            .flat_map(|m1| (1..=255).map(move |m2| (m1, m2)))
            .find_map(|(m1, m2)| {
                modes
                    .iter()
                    .flat_map(|&k1| modes.iter().map(move |&k2| [m1, k1, m2, k2]))
                    .find(|&b| fits(b))
            })
    })
}

/// A mask chunk painted as one monochrome sprite, device px in layer space.
struct Sprite {
    at: [i32; 2],
    size: [i32; 2],
    mode: u8,
    path: SharedString,
    svg: Arc<str>,
}

/// Rasterized draws by their exact parameters (absolute device px: congruent positions are
/// not shared, the float geometry may differ).
/// ponytail: cleared wholesale when full; an LRU only if scrolling ever thrashes it.
static CACHE: LazyLock<Mutex<HashMap<String, Arc<Vec<Sprite>>>>> = LazyLock::new(Default::default);
const CACHE_MAX: usize = 4096;

fn sprites(key: String, make: impl FnOnce() -> ([i32; 4], Vec<Blits>)) -> Arc<Vec<Sprite>> {
    if let Some(s) = CACHE.lock().get(&key) {
        return s.clone();
    }
    let ([x0, y0, w, h], blits) = make();
    let mut out = Vec::new();
    for (blit, mode) in [0, 2]
        .into_iter()
        .flat_map(|b| [MASK, ARGB32, PIPELINE, LAYER].map(|m| (b, m)))
    {
        for cy in (0..h).step_by(CHUNK as usize) {
            for cx in (0..w).step_by(CHUNK as usize) {
                let (cw, ch) = (CHUNK.min(w - cx), CHUNK.min(h - cy));
                let mut plane = Vec::with_capacity((cw * ch) as usize);
                for y in cy..cy + ch {
                    let row = (y * w) as usize;
                    plane.extend((cx..cx + cw).map(|x| {
                        let b = blits[row + x as usize];
                        if b[blit + 1] == mode { b[blit] } else { 0 }
                    }));
                }
                if plane.iter().all(|&v| v == 0) {
                    continue;
                }
                let mut hasher = DefaultHasher::new();
                plane.hash(&mut hasher);
                out.push(Sprite {
                    at: [x0 + cx, y0 + cy],
                    size: [cw, ch],
                    mode,
                    path: format!("shapes/{cw}x{ch}/{:016x}", hasher.finish()).into(),
                    svg: svg(&plane, cw, ch),
                });
            }
        }
    }
    let out = Arc::new(out);
    let mut cache = CACHE.lock();
    if cache.len() >= CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, out.clone());
    out
}

/// The coverage plane as an SVG showing it nearest-neighbor: GPUI rasterizes SVGs at twice
/// the sprite size, so every texel pair the shader samples holds the same coverage.
fn svg(plane: &[u8], w: i32, h: i32) -> Arc<str> {
    let la: Vec<u8> = plane.iter().flat_map(|&m| [0, m]).collect();
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&la, w as u32, h as u32, ExtendedColorType::La8)
        .expect("shapes: encode mask png");
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}"><image width="{w}" height="{h}" preserveAspectRatio="none" image-rendering="optimizeSpeed" xlink:href="data:image/png;base64,{}"/></svg>"#,
        base64::engine::general_purpose::STANDARD.encode(&png)
    )
    .into()
}

fn paint_sprites(sprites: &[Sprite], color: Hsla, window: &mut Window, cx: &App) {
    // Opaque black takes SkARGB32_Black_Blitter, whose blitAntiH2 is the mask formula.
    let c = Rgba::from(color);
    let black = [c.r, c.g, c.b].iter().all(|&v| v * 255. < 0.5) && c.a * 255. > 254.5;
    let s = window.scale_factor();
    for sp in sprites {
        let mode = if black && sp.mode == ARGB32 {
            MASK
        } else {
            sp.mode
        };
        let bounds = Bounds::new(
            point(px(sp.at[0] as f32 / s), px(sp.at[1] as f32 / s)),
            size(px(sp.size[0] as f32 / s), px(sp.size[1] as f32 / s)),
        );
        let color = Hsla {
            h: color.h + 2. * f32::from(mode),
            ..color
        };
        window
            .paint_svg(
                bounds,
                sp.path.clone(),
                Some(sp.svg.as_bytes()),
                TransformationMatrix::unit(),
                color,
                cx,
            )
            .expect("shapes: paint mask sprite");
    }
}

fn alpha_byte(a: f32) -> u8 {
    (a.clamp(0., 1.) * 255.).round() as u8
}

fn paint_op(op: Op, color: Hsla, layer: Layer, window: &mut Window, cx: &App) {
    let a = alpha_byte(color.a);
    let sprites = sprites(format!("{op:?}{layer:?}{a}"), || {
        raster(op.bbox(), layer, a, &|c, col| op.draw(c, col))
    });
    paint_sprites(&sprites, color, window, cx);
}

/// `crate::paint` on macOS: Blink's box decoration painting, op by op.
pub(crate) fn paint(
    d: &Decoration,
    composited: Option<gpui::Point<Pixels>>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    let s = window.scale_factor();
    let layer = Layer::new(window, composited);
    let dev = |v: Pixels| (f32::from(v) * s).round();
    let (l, t) = (dev(bounds.left()), dev(bounds.top()));
    let radius = [
        d.radius.top_left,
        d.radius.top_right,
        d.radius.bottom_right,
        d.radius.bottom_left,
    ];
    let mut rr = Rounded {
        r: [l, t, dev(bounds.right()) - l, dev(bounds.bottom()) - t],
        radii: radius.map(|v| [f32::from(v) * s; 2]),
    };
    rr.constrain();
    let squircle = 2f64.powf(1.5) as f32;
    let k = rr.radii.map(|c| match d.corner {
        Corner::Squircle if c[0] > 0. && c[1] > 0. => squircle,
        _ => 2.,
    });
    let border = Contour { rr, k, origin: rr };
    let has_radius = radius.iter().any(|r| *r > px(0.));
    let opaque_bg = d.fill.is_some_and(|c| alpha_byte(c.a) == 255);
    let width = d
        .border
        .map_or(0., |(w, _)| (f32::from(w) * s).floor().max(1.));
    let mut inner = border;
    inner.rr.r = [
        l + width,
        t + width,
        clamp(rr.r[2] - 2. * width),
        clamp(rr.r[3] - 2. * width),
    ];
    for d in inner.rr.radii.as_flattened_mut() {
        if *d > 0. {
            *d = clamp(*d - width);
        }
    }
    let shadow = |sh: &crate::Shadow| {
        let v = [sh.offset.x, sh.offset.y, sh.blur, sh.spread].map(|v| f32::from(v) * s);
        (v, sh.color)
    };
    for (v, color) in d.shadows.iter().rev().filter(|s| !s.inset).map(shadow) {
        if alpha_byte(color.a) > 0 {
            paint_op(
                Op::Shadow {
                    border,
                    opaque_bg,
                    has_radius,
                    s: v,
                },
                color,
                layer,
                window,
                cx,
            );
        }
    }
    if let Some(color) = d.fill {
        paint_op(Op::Fill(border), color, layer, window, cx);
    }
    for (v, color) in d.shadows.iter().rev().filter(|s| s.inset).map(shadow) {
        if alpha_byte(color.a) > 0 {
            paint_op(
                Op::Inset {
                    bounds: inner,
                    s: v,
                },
                color,
                layer,
                window,
                cx,
            );
        }
    }
    if let Some((_, color)) = d.border {
        paint_op(
            Op::Border {
                outer: border,
                inner,
            },
            color,
            layer,
            window,
            cx,
        );
    }
}

/// One drawable of an icon svg: the path in svg user units, its transform to the viewBox, and
/// how Blink paints it.
#[derive(Debug)]
struct IconPath {
    path: Path,
    transform: Matrix,
    stroke: Option<(f32, PaintCap, PaintJoin)>,
    /// Fixed color (`None` = the icon color), paint opacity (`fill-opacity`) and the
    /// element's `opacity` (a layer).
    color: Option<Rgba>,
    opacity: f32,
    layer: f32,
}

/// A parsed icon: viewBox and drawables, in document order.
struct IconSvg {
    view_box: [f32; 4],
    paths: Vec<IconPath>,
}

thread_local! {
    /// Parsed icons by asset key (skia paths are not `Sync`; GPUI paints on one thread).
    static ICONS: std::cell::RefCell<HashMap<String, Option<std::rc::Rc<IconSvg>>>> =
        Default::default();
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(&tag[at..at + tag[at..].find('"')?])
}

fn numbers(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c.is_whitespace() || c == '(' || c == ')')
        .filter_map(|v| v.parse().ok())
        .collect()
}

/// `translate(..)` / `scale(..)` / `matrix(..)`.
fn transform(s: &str) -> Option<Matrix> {
    let n = numbers(s);
    let t = s.trim_start();
    Some(if t.starts_with("translate") {
        Matrix::translate((n[0], n.get(1).copied().unwrap_or(0.)))
    } else if t.starts_with("scale") {
        Matrix::scale((n[0], n.get(1).copied().unwrap_or(n[0])))
    } else if t.starts_with("matrix") && n.len() == 6 {
        Matrix::new_all(n[0], n[2], n[4], n[1], n[3], n[5], 0., 0., 1.)
    } else {
        return None;
    })
}

fn paint_color(v: &str) -> Option<Option<Rgba>> {
    match v {
        "none" => None,
        "black" | "currentColor" => Some(None),
        "white" => Some(Some(Rgba {
            r: 1.,
            g: 1.,
            b: 1.,
            a: 1.,
        })),
        hex => {
            let n = u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()?;
            Some(Some(gpui::rgb(n)))
        }
    }
}

/// The subset of SVG the icon assets use: nested `<g transform>`, `<path>` with fill/stroke
/// attributes inherited from `<svg>`/`<g>`. `None` for anything else (masks).
fn parse_icon(src: &str) -> Option<IconSvg> {
    let mut view_box = None;
    let mut paths = Vec::new();
    // Inherited (transform, fill, stroke, stroke-width, linecap, linejoin) per open element.
    type Style<'a> = (
        Matrix,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
    );
    let mut stack: Vec<Style> = vec![(
        Matrix::default(),
        Some("black"),
        None,
        Some("1"),
        None,
        None,
    )];
    for tag in src.split('<').skip(1) {
        let tag = &tag[..tag.find('>')?];
        if let Some(end) = tag.strip_prefix('/') {
            if end.starts_with('g') || end.starts_with("svg") {
                stack.pop();
            }
            continue;
        }
        let name = tag.split_whitespace().next()?;
        let top = *stack.last()?;
        let style = (
            attr(tag, "transform").map_or(Some(top.0), |t| {
                transform(t).map(|m| Matrix::concat(&top.0, &m))
            })?,
            attr(tag, "fill").or(top.1),
            attr(tag, "stroke").or(top.2),
            attr(tag, "stroke-width").or(top.3),
            attr(tag, "stroke-linecap").or(top.4),
            attr(tag, "stroke-linejoin").or(top.5),
        );
        match name {
            "svg" => view_box = Some(numbers(attr(tag, "viewBox")?)),
            "g" => {}
            "path" => {
                if attr(tag, "mask").is_some() || attr(tag, "clip-path").is_some() {
                    return None;
                }
                let mut path = Path::from_svg(attr(tag, "d")?)?;
                if attr(tag, "fill-rule") == Some("evenodd") {
                    path.set_fill_type(PathFillType::EvenOdd);
                }
                let opacity = |a: &str| attr(tag, a).map_or(Ok(1.), str::parse::<f32>).ok();
                let element = opacity("opacity")?;
                if let Some(color) = style.1.and_then(paint_color) {
                    let fill = opacity("fill-opacity")?;
                    paths.push(IconPath {
                        path: path.clone(),
                        transform: style.0,
                        stroke: None,
                        color,
                        opacity: fill,
                        layer: element,
                    });
                }
                if let Some(color) = style.2.and_then(paint_color) {
                    let width = style.3?.parse().ok()?;
                    let cap = match style.4 {
                        Some("round") => PaintCap::Round,
                        Some("square") => PaintCap::Square,
                        _ => PaintCap::Butt,
                    };
                    let join = match style.5 {
                        Some("round") => PaintJoin::Round,
                        Some("bevel") => PaintJoin::Bevel,
                        _ => PaintJoin::Miter,
                    };
                    let stroke_opacity = opacity("stroke-opacity")?;
                    paths.push(IconPath {
                        path,
                        transform: style.0,
                        stroke: Some((width, cap, join)),
                        color,
                        opacity: stroke_opacity,
                        layer: element,
                    });
                }
                continue;
            }
            "mask" | "clipPath" | "circle" | "rect" | "ellipse" | "line" | "polyline"
            | "polygon" | "use" => return None,
            _ => continue,
        }
        if !tag.ends_with('/') {
            stack.push(style);
        }
    }
    let v = view_box?;
    Some(IconSvg {
        view_box: [v[0], v[1], *v.get(2)?, *v.get(3)?],
        paths,
    })
}

/// `crate::paint_icon` on macOS.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_icon(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    flip_x: bool,
    color: Hsla,
    composited: Option<gpui::Point<Pixels>>,
    window: &mut Window,
    cx: &App,
) -> bool {
    let icon = ICONS.with_borrow_mut(|icons| {
        icons
            .entry(key.to_string())
            .or_insert_with(|| parse_icon(std::str::from_utf8(src).ok()?).map(std::rc::Rc::new))
            .clone()
    });
    let Some(icon) = icon else {
        return false;
    };
    let s = window.scale_factor();
    let layer = Layer::new(window, composited);
    // The svg root snaps to the nearest device px (half up); the viewBox maps onto its
    // unsnapped CSS size (`xMidYMid meet`), in doubles like Blink's AffineTransform.
    let snap = |v: Pixels| (f32::from(v) * s + 0.5).floor();
    let origin = [snap(bounds.origin.x), snap(bounds.origin.y)];
    let (w, h) = (
        f64::from(f32::from(bounds.size.width) * s),
        f64::from(f32::from(bounds.size.height) * s),
    );
    let [vx, vy, vw, vh] = icon.view_box.map(f64::from);
    let scale = (w / vw).min(h / vh);
    let (tx, ty) = (
        (w - vw * scale) / 2. - vx * scale,
        (h - vh * scale) / 2. - vy * scale,
    );
    let view = Matrix::new_all(
        scale as f32,
        0.,
        tx as f32,
        0.,
        scale as f32,
        ty as f32,
        0.,
        0.,
        1.,
    );
    let rgba = Rgba::from(color);
    for (n, p) in icon.paths.iter().enumerate() {
        let draw = |c: &Canvas, col: Color4f| {
            c.translate((origin[0], origin[1]));
            if flip_x {
                c.translate(((w / 2.) as f32, 0.));
                c.scale((-1., 1.));
                c.translate((-(w / 2.) as f32, 0.));
            }
            c.concat(&view);
            c.concat(&p.transform);
            let mut paint = fill_paint(col);
            if let Some((width, cap, join)) = p.stroke {
                paint
                    .set_style(PaintStyle::Stroke)
                    .set_stroke_width(width)
                    .set_stroke_cap(cap)
                    .set_stroke_join(join);
            }
            c.draw_path(&p.path, &paint);
        };
        let bbox = [
            origin[0] as i32 - 2,
            origin[1] as i32 - 2,
            (origin[0] as f64 + w).ceil() as i32 + 2,
            (origin[1] as f64 + h).ceil() as i32 + 2,
        ];
        let c = p.color.unwrap_or(rgba);
        // ponytail: the svg's `opacity` (a layer in Chromium) is folded into the paint alpha.
        let a = if p.color.is_some() { 1. } else { rgba.a } * p.opacity * p.layer;
        let color = Rgba { a, ..c };
        let a = alpha_byte(a);
        let sprites = sprites(
            format!("{key}#{n}{flip_x}{origin:?}{w}x{h}{layer:?}{a}"),
            || raster(bbox, layer, a, &draw),
        );
        paint_sprites(&sprites, color.into(), window, cx);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        let mut rr = Rounded {
            r: [0., 0., 161., 56.],
            radii: [[19998.; 2]; 4],
        };
        rr.constrain();
        assert_eq!(rr.radii, [[28.; 2]; 4]);
        let icon = parse_icon(r#"<svg viewBox="0 0 20 20" fill="black"><g transform="scale(0.5)"><path d="M0 0L1 1Z"/></g><path d="M0 0L2 2" fill="none" stroke="black" stroke-width="2"></path></svg>"#).unwrap();
        assert_eq!(icon.paths.len(), 2);
        assert_eq!(icon.paths[0].transform, Matrix::scale((0.5, 0.5)));
        assert!(icon.paths[1].stroke.is_some());
    }
}
