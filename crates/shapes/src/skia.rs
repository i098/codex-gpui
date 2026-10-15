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
    AlphaType, BlendMode, BlurStyle, Canvas, ClipOp, Color4f, ColorSpace, ColorType,
    ContourMeasureIter, Data, FilterMode, ImageInfo, MaskFilter, Matrix, MipmapMode, Paint,
    PaintCap, PaintJoin, PaintStyle, Path, PathBuilder, PathDirection, PathEffect, PathFillType,
    PathOp, RRect, Rect, SamplingOptions, Vector, color_filters, surfaces,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    sync::{Arc, LazyLock},
};

pub(crate) const TILE: i32 = 512;
/// Sprites are cut into chunks of at most this many device px; equal chunks share one atlas
/// entry.
const CHUNK: i32 = 128;
/// Blend modes of the shader (`hue + 2 * mode`): legacy `blitMask`/`blitV`, legacy
/// `blitAntiH2`/`blitAntiV2` (`SkBlendARGB32`), raster pipeline lowp srcover.
pub(crate) const MASK: u8 = 0;
pub(crate) const ARGB32: u8 = 1;
const PIPELINE: u8 = 2;
/// Blit into the transparent tile of a composited layer, then the compositor's srcover.
const LAYER: u8 = 3;
/// A mask layer's alpha applied to the layer pixel (`DstIn` at the sprite's coverage).
pub(crate) const DST_IN: u8 = 5;
/// What is under the enclosing GPUI layer drawn onto the layer's pixel through a clip whose
/// coverage is 255 minus the sprite's (viz drawing a clipped layer's content over that layer).
pub(crate) const CLIP: u8 = 7;

/// gfx::SizeF's clamp: dimensions at or below 8 epsilon are 0.
fn clamp(v: f32) -> f32 {
    if v > 8. * f32::EPSILON { v } else { 0. }
}

/// Blink `FloatRoundedRect` in device px: `[x, y, w, h]` and `[w, h]` radii tl, tr, br, bl.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rounded {
    pub(crate) r: [f32; 4],
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
pub(crate) struct Contour {
    pub(crate) rr: Rounded,
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

    /// The contour moved by `[dx, dy]` device px.
    pub(crate) fn moved(mut self, [dx, dy]: [f32; 2]) -> Self {
        for r in [&mut self.rr.r, &mut self.origin.r] {
            r[0] += dx;
            r[1] += dy;
        }
        self
    }

    /// `PathBuilder::AddContouredRect`: when the rect differs from its origin rect (spread
    /// shadows, inner border edges) the rect minus each corner's clip-out path. Its convexity
    /// is computed up front, as Chromium's paths reach Skia with it known: AAA and the AA
    /// clip raster a path whose convexity is cached differently once a tile edge cuts it.
    fn path(&self) -> Path {
        let path = self.contour_path();
        path.is_convex();
        path
    }

    fn contour_path(&self) -> Path {
        if self.round_curvature() {
            return Path::rrect(self.rr.sk(), None);
        }
        let corners = |rr: &Rounded| {
            let [x, y, w, h] = rr.r;
            let (r, b) = (x + w, y + h);
            let [tl, tr, br, bl] = rr.radii;
            [
                corner([r - tr[0], y, tr[0], tr[1]], 0, self.k[1]),
                corner([r - br[0], b - br[1], br[0], br[1]], 1, self.k[2]),
                corner([x, b - bl[1], bl[0], bl[1]], 2, self.k[3]),
                corner([x, y, tl[0], tl[1]], 3, self.k[0]),
            ]
        };
        let target = corners(&self.rr);
        let single = || {
            let mut path = PathBuilder::new();
            path.move_to(sk_point(target[0].0[0]));
            for (v, k) in target {
                curved_corner(&mut path, v, k);
            }
            path.close();
            path.detach()
        };
        if self.origin == self.rr {
            return single();
        }
        let origin = corners(&self.origin);
        let ([x, y, w, h], [ox, oy, ow, oh]) = (self.rr.r, self.origin.r);
        let (r, b, or, ob) = (x + w, y + h, ox + ow, oy + oh);
        let insets = [
            (y - oy, or - r),
            (or - r, ob - b),
            (ob - b, x - ox),
            (x - ox, y - oy),
        ];
        let mut result = Path::rect(self.rr.rect(), None);
        for ((o, t), (start, end)) in origin.into_iter().zip(target).zip(insets) {
            if let Some(clip) = clip_out(o, t, start, end) {
                match result.op(&clip, PathOp::Difference) {
                    Some(p) => result = p,
                    None => return single(),
                }
            }
        }
        let mut path = PathBuilder::new();
        path.add_path(&result, None);
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
    pub(crate) fn clip(&self, c: &Canvas, op: ClipOp) {
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

/// `ContouredRect::Corner`: start, outer, end, center, and the curvature.
type Quad = ([Pt; 4], f32);

/// `ContouredRect::Corner(rect, rotation, curvature)`.
fn corner(r: [f32; 4], rotation: usize, k: f32) -> Quad {
    let mut v = [
        [r[0], r[1]],
        [r[0] + r[2], r[1]],
        [r[0] + r[2], r[1] + r[3]],
        [r[0], r[1] + r[3]],
    ];
    v.rotate_left(rotation);
    (v, k)
}

// gfx::Vector2dF / PointF arithmetic (float, with double cross and dot products).
fn add(a: Pt, b: Pt) -> Pt {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Pt, b: Pt) -> Pt {
    [a[0] - b[0], a[1] - b[1]]
}
fn scale(v: Pt, s: f32) -> Pt {
    [v[0] * s, v[1] * s]
}
fn len(v: Pt) -> f32 {
    v[0].hypot(v[1])
}
fn normalize(v: Pt) -> Pt {
    let l = len(v);
    [v[0] / l, v[1] / l]
}
fn cross(a: Pt, b: Pt) -> f64 {
    f64::from(a[0]) * f64::from(b[1]) - f64::from(a[1]) * f64::from(b[0])
}
fn dot(a: Pt, b: Pt) -> f64 {
    f64::from(a[0]) * f64::from(b[0]) + f64::from(a[1]) * f64::from(b[1])
}

/// `gfx::LineF(p1, p2).IntersectionWith(gfx::LineF(q1, q2))`.
fn intersect(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> Option<Pt> {
    let (a, b) = (sub(p2, p1), sub(q2, q1));
    let denom = cross(a, b) as f32;
    if denom == 0. {
        return None;
    }
    let param = (cross(sub(q1, p1), b) / f64::from(denom)) as f32;
    Some(add(p1, scale(a, param)))
}

/// `Corner::AlignedToOrigin` of `target` for convex curvatures (all this crate draws).
fn aligned(target: Quad, origin: Quad, edge_start: f32, edge_end: f32) -> Quad {
    let (t, o) = (target.0, origin.0);
    let (v2, v3) = (|c: [Pt; 4]| sub(c[2], c[1]), |c: [Pt; 4]| sub(c[3], c[2]));
    if len(sub(o[1], o[0])) == 0. || len(v2(o)) == 0. || target == origin {
        return target;
    }
    let (sr, er) = (len(v2(o)), len(v3(o)));
    let start_inset = if edge_start >= 0. && len(v2(t)) == 0. {
        edge_start
    } else {
        sr - len(v2(t))
    };
    let end_inset = if edge_end >= 0. && len(v3(t)) == 0. {
        edge_end
    } else {
        er - len(v3(t))
    };
    let sp = origin.1.log2();
    let half = 0.5f32.powf(1. / sp.abs().exp2());
    let sqrt2 = std::f32::consts::SQRT_2;
    let cpx = (half / (sqrt2 - 1.) - 1. / sqrt2).clamp(0., 1.);
    let diff = (end_inset - start_inset).clamp(-sr, er);
    let (mut scx, mut ecx) = (cpx, cpx);
    if diff != 0. {
        let delta = (sr * sr + er * er - diff * diff).sqrt();
        let nx = er * diff + sr * delta;
        let ny = -sr * diff + er * delta;
        let bevel = sr * ny / (sr * ny + er * nx);
        scx = 1. - (1. - bevel) * (2. * (1. - cpx));
        ecx = 2. * cpx - scx;
    }
    let us = normalize([(1. - scx) * sr, scx * er]);
    let ue = normalize([ecx * sr, (1. - ecx) * er]);
    let (nv3, nv2) = (normalize(v3(o)), normalize(v2(o)));
    let sn = add(scale(nv3, us[0]), scale(nv2, us[1]));
    let en = add(scale(nv3, ue[0]), scale(nv2, ue[1]));
    let outer = t[1];
    let original = sub(sub(outer, scale(nv3, end_inset)), scale(nv2, start_inset));
    let mut start = add(add(original, scale(nv3, er)), scale(sn, start_inset));
    let mut end = add(add(original, scale(nv2, sr)), scale(en, end_inset));
    let (st, et) = ([-sn[1], sn[0]], [-en[1], en[0]]);
    if start_inset < 0. {
        start = intersect(start, sub(start, st), add(outer, nv3), outer).unwrap_or(start);
    }
    if end_inset < 0. {
        end = intersect(end, add(end, et), add(outer, nv2), outer).unwrap_or(end);
    }
    let height = dot(sub(end, start), nv2) as f32;
    (
        [
            start,
            sub(end, scale(nv2, height)),
            end,
            add(start, scale(nv2, height)),
        ],
        origin.1,
    )
}

/// `CornerClipOutPath` (path_builder.cc) for convex curvatures: the part of `target`'s corner
/// outside the curve aligned to `origin`.
fn clip_out(origin: Quad, target: Quad, edge_start: f32, edge_end: f32) -> Option<Path> {
    let o = origin.0;
    if len(sub(o[2], o[1])) == 0. || len(sub(o[3], o[2])) == 0. || origin.1 == 1000. {
        return None;
    }
    let (a, k) = aligned(target, origin, edge_start, edge_end);
    let mut path = PathBuilder::new();
    path.move_to(sk_point(a[0]));
    curved_corner(&mut path, a, k);
    path.line_to(sk_point(a[2]));
    path.line_to(sk_point(target.0[1]));
    path.close();
    Some(path.detach())
}

/// `AddCurvedCorner` for convex corners (Blink path_builder.cc).
fn curved_corner(path: &mut PathBuilder, [start, outer, end, center]: [Pt; 4], k: f32) {
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
pub(crate) enum Op {
    /// `FillContouredRect` (background color).
    Fill(Contour),
    /// The fill rect inside an AA clip of the contour: an `Op::ClipLayer` whose border is
    /// transparent holds the fill alone, and cc folds a layer of one draw into that draw.
    ClipFill(Contour),
    /// Uniform solid or dashed border (`BoxBorderPainter`).
    Border {
        outer: Contour,
        inner: Contour,
        dashed: bool,
    },
    /// Outer `box-shadow` layer (`PaintNormalBoxShadow`).
    Shadow {
        border: Contour,
        opaque_bg: bool,
        has_radius: bool,
        s: [f32; 4],
    },
    /// Inset `box-shadow` layer (`PaintInsetBoxShadow`); `bounds` is the padding box.
    Inset { bounds: Contour, s: [f32; 4] },
    /// A fill (`fill`) under a translucent border (the op's color) in Blink's
    /// `kBackgroundBleedClipLayer`: inside an AA clip of `outer`, a layer holding the fill
    /// rect and the border.
    ClipLayer {
        outer: Contour,
        inner: Contour,
        fill: Color4f,
    },
}

/// A color as Skia's legacy blitters get it: the 8-bit CSS color.
pub(crate) fn color4f(c: Hsla) -> Color4f {
    let c = Rgba::from(c);
    let q = |v: f32| f32::from(alpha_byte(v)) / 255.;
    Color4f::new(q(c.r), q(c.g), q(c.b), q(c.a))
}

/// A color outside [0, 1] (an out-of-gamut CSS oklab/color-mix result, whose saturation or
/// lightness leaves [0, 1]; `Rgba::from` clamps it): Skia draws it with the float raster
/// pipeline (`SkBlitter::UseLegacyBlitter`); the shader does the same for its sprites.
pub(crate) fn wide(c: Hsla) -> bool {
    !(0. ..=1.).contains(&c.s) || !(0. ..=1.).contains(&c.l)
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
    /// Whether Chromium's raster does not blit the draw itself but restores a layer: Blink's
    /// slow (superellipse) or dashed border path paints a translucent border opaque inside a
    /// `saveLayerAlphaf` (whose draws the earlier AA clips do not cut), restored through those
    /// clips ([`restore_sprites`]).
    fn restored(&self, alpha: u8) -> bool {
        matches!(self, Op::Border { inner, dashed, .. }
            if alpha < 255 && (*dashed || !(inner.rr.renderable() && inner.round_curvature())))
    }

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
    pub(crate) fn bbox(&self) -> [i32; 4] {
        let r = match self {
            Op::Fill(c)
            | Op::ClipFill(c)
            | Op::Border { outer: c, .. }
            | Op::Inset { bounds: c, .. }
            | Op::ClipLayer { outer: c, .. } => c.rr.r,
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

    pub(crate) fn draw(&self, c: &Canvas, col: Color4f) {
        match *self {
            Op::Fill(contour) if contour.round_curvature() => contour.fill(c, &fill_paint(col)),
            Op::Fill(contour) | Op::ClipFill(contour) => {
                // Blink's slow background path (`RoundedInnerRectClipper`): the snapped rect
                // inside an anti-aliased clip of the contour.
                // ponytail: unrenderable radii clip corner by corner in Blink; not drawn here.
                c.save();
                contour.clip(c, ClipOp::Intersect);
                c.draw_rect(contour.rr.rect(), &fill_paint(col));
                c.restore();
            }
            Op::Border {
                outer,
                inner,
                dashed,
            } if dashed || !(inner.rr.renderable() && inner.round_curvature()) => {
                c.save();
                ring_clip(c, &outer, &inner, dashed);
                ring_content(c, &outer, &inner, dashed, col);
                c.restore();
            }
            Op::Border { outer, inner, .. } => {
                let mut paint = fill_paint(col);
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
            Op::ClipLayer { outer, inner, fill } => {
                c.save();
                outer.clip(c, ClipOp::Intersect);
                c.save_layer(&skia_safe::canvas::SaveLayerRec::default());
                Self::layer_content(c, &outer, &inner, fill, col);
                c.restore();
                c.restore();
            }
        }
    }

    /// `Op::ClipLayer`'s layer: the fill rect, then the border without the outer clip (the
    /// layer has it). A translucent superellipse border paints opaque in its own
    /// `saveLayerAlphaf` inside the inner clip.
    fn layer_content(c: &Canvas, outer: &Contour, inner: &Contour, fill: Color4f, col: Color4f) {
        c.draw_rect(outer.rr.rect(), &fill_paint(fill));
        if inner.rr.renderable() && inner.round_curvature() {
            // DrawBleedAdjustedDRRect: the outside of the inner rrect, to the layer bounds.
            let mut ring = Path::rrect(inner.rr.sk(), None);
            ring.set_fill_type(PathFillType::InverseWinding);
            c.draw_path(&ring, &fill_paint(col));
        } else {
            c.save();
            if !inner.rr.is_empty() {
                inner.clip(c, ClipOp::Difference);
            }
            c.save_layer_alpha_f(None, col.a);
            c.draw_rect(outer.rr.rect(), &fill_paint(Color4f { a: 1., ..col }));
            c.restore();
            c.restore();
        }
    }
}

/// The clips Blink paints a slow-path (superellipse) or dashed border in: inside the outer
/// contour, outside the inner one.
fn ring_clip(c: &Canvas, outer: &Contour, inner: &Contour, dashed: bool) {
    outer.clip(c, ClipOp::Intersect);
    if dashed || (inner.rr.renderable() && !inner.rr.is_empty()) {
        inner.clip(c, ClipOp::Difference);
    }
}

/// What Blink draws of such a border inside [`ring_clip`] (inside its layer when translucent,
/// see `Op::restored`): the sides' fills, whose non-AA miter polygons partition the outer rect
/// (one fill of it stands in), or the dashed centerline.
fn ring_content(c: &Canvas, outer: &Contour, inner: &Contour, dashed: bool, col: Color4f) {
    if !dashed {
        c.draw_rect(outer.rr.rect(), &fill_paint(col));
        return;
    }
    // DrawCurvedDashedDottedBoxSide: the centerline rrect (from its top-left end) stroked 2.2x
    // the width with Blink's dash, once per side inside that side's non-AA quad
    // (`ClipBorderSidePolygon`: outer corner to the inner corner's diagonal). Where the quads
    // share a diagonal, a px is stroked twice or not at all.
    // ponytail: paths too short for two dashes, and pentagon quads, are not special-cased.
    let width = inner.rr.r[0] - outer.rr.r[0];
    let mut center = outer.rr;
    center.outset(-width / 2.);
    let path = Path::rrect_with_start_index(center.sk(), PathDirection::CW, 0);
    let len = ContourMeasureIter::new(&path, false, None)
        .next()
        .map_or(0., |m| m.length().trunc());
    // StyledStrokeData: dash and gap per width, the gap fitted to whole periods.
    let (dash, gap) = if width >= 3. {
        (2. * width, width)
    } else {
        (3. * width, 2. * width)
    };
    let n = (len / (dash + gap)).floor();
    let (lo, hi) = ((len - n * dash) / n, (len - (n + 1.) * dash) / (n + 1.));
    let gap = if hi <= 0. || (lo - gap).abs() < (hi - gap).abs() {
        lo
    } else {
        hi
    };
    let mut paint = fill_paint(col);
    paint
        .set_style(PaintStyle::Stroke)
        .set_stroke_width(2.2 * width)
        .set_path_effect(PathEffect::dash(&[dash, gap], 0.));
    let ([x0, y0, w, h], [ix0, iy0, iw, ih]) = (outer.rr.r, inner.rr.r);
    let (x1, y1, ix1, iy1) = (x0 + w, y0 + h, ix0 + iw, iy0 + ih);
    // FindIntersection of the outer-to-inner corner line with the inner corner's diagonal.
    let quad = |o: Pt, i: Pt, [rw, rh]: [f32; 2], [sx, sy]: [f32; 2]| -> Pt {
        intersect(o, i, [i[0] + sx * rw, i[1]], [i[0], i[1] + sy * rh]).unwrap_or(i)
    };
    let [tl, tr, br, bl] = inner.rr.radii;
    let (otl, otr, obr, obl) = ([x0, y0], [x1, y0], [x1, y1], [x0, y1]);
    let itl = quad(otl, [ix0, iy0], tl, [1., 1.]);
    let itr = quad(otr, [ix1, iy0], tr, [-1., 1.]);
    let ibr = quad(obr, [ix1, iy1], br, [-1., -1.]);
    let ibl = quad(obl, [ix0, iy1], bl, [1., -1.]);
    // Blink's side order: top, bottom, right, left.
    for side in [
        [otl, itl, itr, otr],
        [obr, ibr, ibl, obl],
        [otr, itr, ibr, obr],
        [obl, ibl, itl, otl],
    ] {
        let clip = Path::polygon(&side.map(sk_point), false, None, None);
        c.save();
        c.clip_path(&clip, ClipOp::Intersect, false);
        c.draw_path(&path, &paint);
        c.restore();
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

/// Skia's rounding `div255`.
pub(crate) fn div255(v: u32) -> u32 {
    (v + ((v + 128) >> 8) + 128) >> 8
}

/// The patched shader's blend of one 8-bit channel: unpremultiplied color `c` with alpha `a`
/// over `d` at coverage `m` (crates/textsys/gpui_apple `monochrome_sprite_fragment`).
fn blend(mode: u8, c: u8, a: u8, d: u8, m: u8) -> u8 {
    let (c, a, d, m) = (u32::from(c), u32::from(a), u32::from(d), u32::from(m));
    let src = div255(c * a);
    let scale = m + 1;
    (match mode {
        ARGB32 => {
            let inv = 0xFFFF - a * scale;
            ((src * scale + d * ((inv + (inv >> 8)) >> 8)) >> 8) & 0xFF
        }
        PIPELINE => div255(src * m) + div255(d * (255 - div255(a * m))),
        _ => ((src * scale) >> 8) + ((d * (256 - ((a * scale) >> 8))) >> 8),
    }) as u8
}

/// Coverage and blend mode of up to two blits per pixel: `[m1, mode1, m2, mode2]`.
pub(crate) type Blits = [u8; 4];

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
    pub(crate) origin: [i32; 2],
    /// Window size (bounds the root layer's tiles).
    pub(crate) window: [i32; 2],
    /// cc tile size of the layer ([`cc_tile`]).
    pub(crate) tile: [i32; 2],
    pub(crate) composited: bool,
    /// The device origin (layer px) of the enclosing [`with_layer`].
    pub(crate) device: Option<[i32; 2]>,
}

impl Layer {
    /// `composited`: the origin of a composited layer whose transparent tiles the compositor
    /// blends onto the page; `None` takes the innermost `.layer()` div being painted, whose
    /// tiles hold its own background, so draws blit as on the page.
    pub(crate) fn new(window: &Window, composited: Option<gpui::Point<Pixels>>) -> Self {
        let (vp, s) = (window.viewport_size(), window.scale_factor());
        let dev = |v: Pixels| (f32::from(v) * s).round() as i32;
        let (origin, size) = match composited {
            Some(o) => ([dev(o.x), dev(o.y)], None),
            None => window.composited_layer().map_or(([0, 0], None), |(b, cc)| {
                (
                    [dev(b.origin.x), dev(b.origin.y)],
                    cc.then(|| [dev(b.size.width), dev(b.size.height)]),
                )
            }),
        };
        // The innermost `with_layer`, else a `saved_layer` div: Skia's layer device.
        let floor = |v: Pixels| (f32::from(v) * s).floor() as i32;
        let device = DEVICE
            .get()
            .or_else(|| {
                window
                    .saved_layer()
                    .map(|b| [floor(b.origin.x), floor(b.origin.y)])
            })
            .map(|[x, y]| [x - origin[0], y - origin[1]]);
        let window = [dev(vp.width), dev(vp.height)];
        Self {
            origin,
            window,
            // ponytail: a layer of unknown rect (not `cc_tiles`) keeps the default tiles.
            tile: size.map_or([TILE; 2], cc_tile),
            composited: composited.is_some(),
            device,
        }
    }

    /// `b` (window px) in layer space: Blink paints a layer's content in that space, and the
    /// float geometry of curves and anti-aliasing depends on where the shape lies in it.
    pub(crate) fn local(&self, b: Bounds<Pixels>, s: f32) -> Bounds<Pixels> {
        let o = point(px(self.origin[0] as f32 / s), px(self.origin[1] as f32 / s));
        Bounds::new(b.origin - o, b.size)
    }

    /// The layer's size: the window past its origin.
    pub(crate) fn size(&self) -> [i32; 2] {
        [
            self.window[0] - self.origin[0],
            self.window[1] - self.origin[1],
        ]
    }
}

/// cc `TileSizeCalculator` for CPU raster at device scale 2 (512 px default tiles, 1024 px
/// max untiled layer): a layer thin in one axis gets long tiles along the other, a small
/// layer one tile, each clamped to the layer rounded up to 64 px.
fn cc_tile([w, h]: [i32; 2]) -> [i32; 2] {
    const UNTILED: i32 = 2 * TILE;
    let mut d = [TILE; 2];
    if w < d[0] {
        d[1] = UNTILED;
    }
    if h < d[1] {
        d[0] = UNTILED;
    }
    if w < UNTILED && h < UNTILED {
        d = [UNTILED; 2];
    }
    let fit = |v: i32, d: i32| ((v.max(1) + 63) / 64 * 64).min(d);
    [fit(w, d[0]), fit(h, d[1])]
}

/// Chromium's cc tiles of size `tile` over `[x0, y0, x1, y1]` (layer px) of a layer of `size`:
/// each tile's origin, size and the rect `[x0, y0, x1, y1]` of the px it owns (the tiles
/// overlap by one).
pub(crate) fn tiles(
    [x0, y0, x1, y1]: [i32; 4],
    size: [i32; 2],
    tile: [i32; 2],
) -> impl Iterator<Item = ([i32; 2], (i32, i32), [i32; 4])> {
    let stride = tile.map(|t| t - 2);
    let owner = move |v: i32, k: usize| if v <= 0 { 0 } else { (v - 1) / stride[k] };
    let own = |o: i32| if o == 0 { 0 } else { o + 1 };
    (owner(y0, 1)..=owner(y1 - 1, 1)).flat_map(move |ty| {
        (owner(x0, 0)..=owner(x1 - 1, 0)).map(move |tx| {
            let o = [tx * stride[0], ty * stride[1]];
            let ts = ((size[0] - o[0]).min(tile[0]), (size[1] - o[1]).min(tile[1]));
            let owned = [
                x0.max(own(o[0])),
                y0.max(own(o[1])),
                x1.min(o[0] + stride[0] + 1).min(o[0] + ts.0),
                y1.min(o[1] + stride[1] + 1).min(o[1] + ts.1),
            ];
            (o, ts, owned)
        })
    })
}

/// The blits of each pixel of `[x0, y0, x1, y1]` (layer device px, clamped to the layer),
/// rendering `draw` (in layer space) once per cc tile like Chromium's raster worker; the
/// result is in window device px. Skia picks its blitter by
/// the paint (opaque legacy, translucent, color filter), so the probes that tell the modes
/// apart draw with the real color's `alpha`; an opaque black paint (`black`) takes Skia's own
/// `SkARGB32_Black_Blitter`, so its probes are black. AAA blits some edge pixels twice (a
/// partial row, then the rest of it): those are solved for two blits. In a composited layer
/// every pixel is one [`LAYER`] blit of its coverage. `clips` (layer space) are the
/// ancestors' anti-aliased clips ([`with_rounded_clip`]): Skia draws inside them, and in a
/// tile their curves reach the AA clip makes AAA blit runs only. A [`wide`] color takes
/// Skia's float pipeline (`shaders.metal` mode 4, or 6 into a composited layer), whose AAA
/// coverage on some edge px is one more than the legacy blitter's: an opaque wide probe reads
/// it, one blit per px.
fn raster(
    [x0, y0, x1, y1]: [i32; 4],
    layer: Layer,
    alpha: u8,
    black: bool,
    wide: bool,
    clips: &[Contour],
    draw: &dyn Fn(&Canvas, Color4f),
) -> ([i32; 4], Vec<Blits>) {
    let [lx, ly] = layer.origin;
    let size = layer.size();
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(size[0]), y1.min(size[1]));
    if x0 >= x1 || y0 >= y1 {
        return ([0; 4], Vec::new());
    }
    let fixed = layer
        .composited
        .then_some(LAYER)
        .or(wide.then_some(PIPELINE));
    // The first probe (opaque white on black, or black on white) reads the coverage; the
    // others, with distinct channels, tell the blits apart (`blitAntiH`, `blitAntiH2`, raster
    // pipeline).
    let fg = |fg: [u8; 3]| if black { [0; 3] } else { fg };
    let mut probes = vec![Probe {
        fg: fg([255; 3]),
        a: 255,
        bg: [if black { 255 } else { 0 }; 3],
    }];
    let pairs = [
        ([255, 0, 0], [255, 255, 255]),
        ([26, 128, 230], [252, 36, 128]),
        ([200, 60, 100], [24, 200, 246]),
    ];
    let modes: &[u8] = match fixed {
        Some(_) => &[],
        None => {
            probes.extend(pairs.map(|(f, bg)| Probe {
                fg: fg(f),
                a: alpha,
                bg,
            }));
            if alpha == 255 {
                &[MASK, ARGB32]
            } else {
                probes.push(Probe {
                    fg: [255; 3],
                    a: alpha,
                    bg: [0; 3],
                });
                &[MASK, ARGB32, PIPELINE]
            }
        }
    };
    let (w, h) = (x1 - x0, y1 - y0);
    let mut out = vec![[0, MASK, 0, MASK]; (w * h) as usize];
    let color = |[r, g, b]: [u8; 3], a: u8| {
        let f = |v: u8| f32::from(v) / 255.;
        Color4f::new(f(r), f(g), f(b), f(a))
    };
    let tile = |o: [i32; 2], size: (i32, i32), p: Probe| {
        tile_pixels(o, size, color(p.bg, 255), |c| {
            for clip in clips {
                clip.clip(c, ClipOp::Intersect);
            }
            draw(
                c,
                if wide {
                    Color4f::new(1.00003, 1., 1., 1.)
                } else {
                    color(p.fg, p.a)
                },
            );
        })
    };
    // A composited layer (a scroller's contents) runs on past the window: its tiles there
    // keep their full height.
    let grid = if layer.origin == [0, 0] {
        size
    } else {
        [size[0], i32::MAX / 2]
    };
    for (o, ts, [ox0, oy0, ox1, oy1]) in tiles([x0, y0, x1, y1], grid, layer.tile) {
        // A `with_layer` draws on a device of the layer's bounds within the tile, whose origin
        // moves the float geometry of curves.
        let lo = layer.device.map_or(o, |d| [o[0].max(d[0]), o[1].max(d[1])]);
        let ls = (o[0] + ts.0 - lo[0], o[1] + ts.1 - lo[1]);
        if ls.0 <= 0 || ls.1 <= 0 {
            continue;
        }
        let seen: Vec<_> = probes.iter().map(|&p| tile(lo, ls, p)).collect();
        for y in oy0.max(lo[1])..oy1 {
            for x in ox0.max(lo[0])..ox1 {
                // RGBA bytes (macOS N32).
                let i = (y - lo[1]) as usize * seen[0].1 + (x - lo[0]) as usize * 4;
                let px: Vec<[u8; 3]> = seen
                    .iter()
                    .map(|s| [s.0[i], s.0[i + 1], s.0[i + 2]])
                    .collect();
                // The coverage the first probe's blit shows: its value white on black, its
                // inverse black on white.
                let m = if black { 255 - px[0][1] } else { px[0][1] };
                if m == 0 {
                    continue;
                }
                out[((y - y0) * w + x - x0) as usize] = match fixed {
                    Some(mode) => [m, mode, 0, MASK],
                    None => solve(&probes, &px, m, modes).unwrap_or_else(|| {
                        // AAA may split a translucent paint's edge px into other blits than the
                        // opaque probe's: one `blitAntiH` (as under an AA clip, which forces
                        // AAA's run blitter) of the coverage the translucent white probe shows,
                        // the nearest to the opaque one.
                        let v = px[px.len() - 1][0];
                        let fit = |c: &u8| blend(MASK, 255, alpha, 0, *c) == v;
                        let rle = (0..=255).filter(fit).min_by_key(|c| c.abs_diff(m));
                        [
                            if alpha == 255 { m } else { rle.unwrap_or(m) },
                            MASK,
                            0,
                            MASK,
                        ]
                    }),
                };
            }
        }
    }
    ([x0 + lx, y0 + ly, w, h], out)
}

/// The RGBA bytes (macOS N32, premultiplied) and row bytes of the cc tile of `size` at layer
/// px `o` (window px), cleared to `clear`, after `draw`.
pub(crate) fn tile_pixels(
    o: [i32; 2],
    size: (i32, i32),
    clear: Color4f,
    draw: impl FnOnce(&Canvas),
) -> (Vec<u8>, usize) {
    // Chromium's raster canvases are sRGB: a [`wide`] paint color takes the float pipeline.
    surface_pixels(o, size, clear, ColorSpace::new_srgb(), draw)
}

/// [`tile_pixels`] on a surface of color space `cs` (`None`: the compositor's, whose filters
/// take Skia's legacy 8-bit paths).
pub(crate) fn surface_pixels(
    o: [i32; 2],
    size: (i32, i32),
    clear: Color4f,
    cs: impl Into<Option<ColorSpace>>,
    draw: impl FnOnce(&Canvas),
) -> (Vec<u8>, usize) {
    let info = ImageInfo::new_n32_premul(size, cs);
    let mut surface = surfaces::raster(&info, None, None).expect("shapes: tile surface");
    let canvas = surface.canvas();
    canvas.clear(clear);
    canvas.translate((-o[0] as f32, -o[1] as f32));
    draw(canvas);
    let pixmap = surface.peek_pixels().expect("shapes: tile pixels");
    (
        pixmap.bytes().expect("shapes: tile bytes").to_vec(),
        pixmap.row_bytes(),
    )
}

/// An op Chromium paints into a layer it then restores (`Op::ClipLayer`, a `restored` border)
/// as Skia rasterizes it, once per cc tile, painted as one sprite group per distinct restored
/// px. The layer px `l` are Skia's own (a clip layer's content drawn on a transparent device
/// of its bounds, with its own origin; a border layer's opaque sides, which no clip cuts). The
/// restore (paint alpha `a`) takes the sprite blitter when the clip is a plain rect
/// (`blit_row_s32a_blend` of `l` at `a`: [`ARGB32`] at coverage `a`), else the legacy shader
/// blitter of `l` scaled by `a` (`SkAlphaMulQ`): `blit_row_s32a_opaque` where the clip covers
/// a px (the px blitted into a GPUI layer, which restores it the same way), `SkBlendARGB32` at
/// the clip coverage on its AA edge. Skia's AA clip only turns into a plain rect when it
/// starts at its device's top row (`SkAAClip::isRect` compares a row offset with the bounds).
fn restore_sprites(op: &Op, col: Color4f, layer: Layer) -> Vec<Sprite> {
    let [lx, ly] = layer.origin;
    let size = layer.size();
    // `op` is in layer space.
    let [x0, y0, x1, y1] = op.bbox();
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(size[0]), y1.min(size[1]));
    if x0 >= x1 || y0 >= y1 {
        return Vec::new();
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let white = Color4f::new(1., 1., 1., 1.);
    let (clear, black) = (Color4f::new(0., 0., 0., 0.), Color4f::new(0., 0., 0., 1.));
    let opaque = Color4f { a: 1., ..col };
    let alpha = match op {
        Op::ClipLayer { .. } => 255,
        _ => alpha_byte(col.a),
    };
    // Distinct (premultiplied restored px, restored through the clip's AA edge or by the
    // sprite blitter), and each px's (group, coverage).
    let mut groups: Vec<([u8; 4], bool)> = Vec::new();
    let mut index = HashMap::new();
    let mut px = vec![(usize::MAX, 0u8); (w * h) as usize];
    for (o, ts, [ox0, oy0, ox1, oy1]) in tiles([x0, y0, x1, y1], size, layer.tile) {
        // The restore's clip coverage, and whether Skia made it a plain rect.
        let mut rect = false;
        let clip = tile_pixels(o, ts, black, |c| {
            match op {
                Op::ClipLayer { outer, .. } => outer.clip(c, ClipOp::Intersect),
                Op::Border {
                    outer,
                    inner,
                    dashed,
                } => ring_clip(c, outer, inner, *dashed),
                _ => {}
            }
            rect = c.is_clip_rect();
            c.draw_paint(&fill_paint(white));
        });
        let (lo, ls) = match op {
            // The clip layer's device: the outer clip's bounds within the tile.
            Op::ClipLayer { outer, .. } => {
                let r = outer.rr.r;
                let lo = [o[0].max(r[0].floor() as i32), o[1].max(r[1].floor() as i32)];
                let x1 = (o[0] + ts.0).min((r[0] + r[2]).ceil() as i32);
                let y1 = (o[1] + ts.1).min((r[1] + r[3]).ceil() as i32);
                (lo, (x1 - lo[0], y1 - lo[1]))
            }
            _ => (o, ts),
        };
        if ls.0 <= 0 || ls.1 <= 0 {
            continue;
        }
        let content = tile_pixels(lo, ls, clear, |c| match op {
            Op::ClipLayer { outer, inner, fill } => Op::layer_content(c, outer, inner, *fill, col),
            Op::Border {
                outer,
                inner,
                dashed,
            } => ring_content(c, outer, inner, *dashed, opaque),
            _ => {}
        });
        let sprite = rect && alpha < 255;
        for y in oy0.max(lo[1])..oy1.min(lo[1] + ls.1) {
            for x in ox0.max(lo[0])..ox1.min(lo[0] + ls.0) {
                let i = (y - o[1]) as usize * clip.1 + (x - o[0]) as usize * 4;
                let m = clip.0[i + 1];
                let j = (y - lo[1]) as usize * content.1 + (x - lo[0]) as usize * 4;
                let mut l: [u8; 4] = content.0[j..j + 4].try_into().expect("rgba");
                if !sprite {
                    l = l.map(|v| ((u32::from(v) * (u32::from(alpha) + 1)) >> 8) as u8);
                }
                if m == 0 || l[3] == 0 {
                    continue;
                }
                let key = (l, sprite || m < 255);
                let g = *index.entry(key).or_insert_with(|| {
                    groups.push(key);
                    groups.len() - 1
                });
                px[((y - y0) * w + x - x0) as usize] = (g, if sprite { alpha } else { m });
            }
        }
    }
    let mut out = Vec::new();
    for (g, &(l, edge)) in groups.iter().enumerate() {
        // The 8-bit color whose premultiplied value (as the shader computes it) is `l`.
        let a = u32::from(l[3]);
        let c = [0, 1, 2].map(|k| {
            (0..=255u32)
                .find(|&c| div255(c * a) == u32::from(l[k]))
                .unwrap_or(255) as u8
        });
        let color = [c[0], c[1], c[2], l[3]];
        let value = |i: usize| if px[i].0 == g { px[i].1 } else { 0 };
        // A composited layer's tile is transparent: the restore leaves the layer px scaled by
        // the clip coverage (both blitters), which the compositor blends onto the page.
        let (mode, opaque_layer) = match (layer.composited, edge) {
            (true, _) => (LAYER, false),
            (false, true) => (ARGB32, false),
            (false, false) => (MASK, true),
        };
        chunks(
            &mut out,
            [x0 + lx, y0 + ly, w, h],
            &value,
            mode,
            Some(color),
            opaque_layer,
        );
    }
    out
}

/// One or two blits that reproduce every probe value `px`, trying one blit of the coverage
/// `m` first. The coverage probe (opaque) may take other legacy modes than the real paint's
/// when that is translucent.
fn solve(probes: &[Probe], px: &[[u8; 3]], m: u8, modes: &[u8]) -> Option<Blits> {
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
    let rest = |b: Blits| {
        let k = [b[1], b[3]];
        probes
            .iter()
            .zip(px)
            .skip(1)
            .all(|(p, v)| probe(b, k, p, v))
    };
    let fits = |b: Blits| {
        rest(b)
            && if opaque {
                probe(b, [b[1], b[3]], &probes[0], &px[0])
            } else {
                legacy.iter().any(|k| probe(b, *k, &probes[0], &px[0]))
            }
    };
    let single = |c: u8| modes.iter().map(move |&mode| [c, mode, 0, MASK]);
    single(m)
        .find(|&b| fits(b))
        // A translucent paint's AAA may blit an edge px once at another coverage than the
        // opaque probe shows (nearest first): one blit is tried before two.
        .or_else(|| {
            (1..=255)
                .filter(|_| !opaque)
                .flat_map(|d| [m.checked_sub(d), m.checked_add(d)])
                .flatten()
                .filter(|&c| c > 0)
                .flat_map(single)
                .find(|&b| rest(b))
        })
        .or_else(|| {
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

/// A mask chunk painted as one monochrome sprite, device px in layer space, in the op's color
/// or its own 8-bit `color`; `opaque_layer`: blitted into a GPUI alpha-255 layer
/// (`Window::with_skia_layer`) instead of directly.
pub(crate) struct Sprite {
    at: [i32; 2],
    size: [i32; 2],
    mode: u8,
    color: Option<[u8; 4]>,
    opaque_layer: bool,
    path: SharedString,
    svg: Arc<str>,
}

/// Rasterized draws by their exact parameters (absolute device px: congruent positions are
/// not shared, the float geometry may differ).
/// ponytail: cleared wholesale when full; an LRU only if scrolling ever thrashes it.
static CACHE: LazyLock<Mutex<HashMap<String, Arc<Vec<Sprite>>>>> = LazyLock::new(Default::default);
const CACHE_MAX: usize = 4096;

pub(crate) fn cached(key: String, make: impl FnOnce() -> Vec<Sprite>) -> Arc<Vec<Sprite>> {
    if let Some(s) = CACHE.lock().get(&key) {
        return s.clone();
    }
    let out = Arc::new(make());
    let mut cache = CACHE.lock();
    if cache.len() >= CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, out.clone());
    out
}

pub(crate) fn sprites(
    key: String,
    make: impl FnOnce() -> ([i32; 4], Vec<Blits>),
) -> Arc<Vec<Sprite>> {
    cached(key, || {
        let (rect, blits) = make();
        let mut out = Vec::new();
        for (blit, mode) in [0, 2]
            .into_iter()
            .flat_map(|b| [MASK, ARGB32, PIPELINE, LAYER, DST_IN].map(|m| (b, m)))
        {
            let value = |i: usize| {
                let b = blits[i];
                if b[blit + 1] == mode { b[blit] } else { 0 }
            };
            chunks(&mut out, rect, &value, mode, None, false);
        }
        out
    })
}

/// Cuts the coverage `value` of each px of `[x0, y0, w, h]` (row-major) into sprites.
pub(crate) fn chunks(
    out: &mut Vec<Sprite>,
    [x0, y0, w, h]: [i32; 4],
    value: &dyn Fn(usize) -> u8,
    mode: u8,
    color: Option<[u8; 4]>,
    opaque_layer: bool,
) {
    for cy in (0..h).step_by(CHUNK as usize) {
        for cx in (0..w).step_by(CHUNK as usize) {
            let (cw, ch) = (CHUNK.min(w - cx), CHUNK.min(h - cy));
            let mut plane = Vec::with_capacity((cw * ch) as usize);
            for y in cy..cy + ch {
                let row = (y * w) as usize;
                plane.extend((cx..cx + cw).map(|x| value(row + x as usize)));
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
                color,
                opaque_layer,
                path: format!("shapes/{cw}x{ch}/{:016x}", hasher.finish()).into(),
                svg: svg(&plane, cw, ch),
            });
        }
    }
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

pub(crate) fn paint_sprites(sprites: &[Sprite], color: Hsla, window: &mut Window, cx: &App) {
    let s = window.scale_factor();
    for sp in sprites {
        let color: Hsla = sp.color.map_or(color, |[r, g, b, a]| {
            let f = |v: u8| f32::from(v) / 255.;
            Rgba {
                r: f(r),
                g: f(g),
                b: f(b),
                a: f(a),
            }
            .into()
        });
        let bounds = Bounds::new(
            point(px(sp.at[0] as f32 / s), px(sp.at[1] as f32 / s)),
            size(px(sp.size[0] as f32 / s), px(sp.size[1] as f32 / s)),
        );
        let color = Hsla {
            h: color.h + 2. * f32::from(sp.mode),
            ..color
        };
        let paint = |window: &mut Window| {
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
        };
        if sp.opaque_layer {
            window.with_skia_layer(255, paint)
        } else {
            paint(window)
        }
    }
}

pub(crate) fn alpha_byte(a: f32) -> u8 {
    (a.clamp(0., 1.) * 255.).round() as u8
}

thread_local! {
    /// The anti-aliased clips of [`with_rounded_clip`], window device px, outermost first;
    /// with `layers: false`, the origin of the layer they apply in.
    static CLIPS: RefCell<Vec<(Contour, Option<[i32; 2]>)>> = const { RefCell::new(Vec::new()) };
    /// The device origin of the innermost [`with_layer`], window device px.
    static DEVICE: Cell<Option<[i32; 2]>> = const { Cell::new(None) };
}

/// `crate::with_rounded_clip` on macOS.
pub(crate) fn with_rounded_clip<R>(
    radius: gpui::Corners<Pixels>,
    corner: Corner,
    bounds: Bounds<Pixels>,
    layers: bool,
    window: &mut Window,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    let d = Decoration {
        radius,
        corner,
        ..Default::default()
    };
    let (clip, _) = ops(&d, bounds, window.scale_factor());
    let only = (!layers).then(|| Layer::new(window, None).origin);
    CLIPS.with_borrow_mut(|c| c.push((clip, only)));
    let out = paint(window);
    CLIPS.with_borrow_mut(|c| c.pop());
    out
}

/// `crate::with_layer` on macOS: the [`CLIPS`] that cover the layer's device px leave it, and
/// Skia draws the content on a device that starts at the layer's top-left px.
pub(crate) fn with_layer<R>(
    alpha: u8,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    paint: impl FnOnce(&mut Window) -> R,
) -> R {
    let s = window.scale_factor();
    let [l, t, r, b] =
        [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()].map(|v| f32::from(v) * s);
    // The device px around the layer, so no anti-aliased clip edge reaches them.
    let corners = [
        (l - 1., t - 1.),
        (r + 1., t - 1.),
        (r + 1., b + 1.),
        (l - 1., b + 1.),
    ];
    let outer = CLIPS.with_borrow_mut(|c| {
        let outer = c.clone();
        c.retain(|(clip, _)| !corners.iter().all(|&p| clip.path().contains(p)));
        outer
    });
    let device = DEVICE.replace(Some([l.floor() as i32, t.floor() as i32]));
    let out = window.with_skia_layer(alpha, paint);
    DEVICE.set(device);
    CLIPS.with_borrow_mut(|c| *c = outer);
    out
}

/// The active [`CLIPS`] in `layer`'s space.
fn clips(window: &Window, layer: Layer) -> Vec<Contour> {
    let [x, y] = layer.origin.map(|v| -v as f32);
    let inner = Layer::new(window, None).origin;
    CLIPS.with_borrow(|c| {
        c.iter()
            .filter(|(_, only)| only.is_none_or(|o| o == inner))
            .map(|(c, _)| c.moved([x, y]))
            .collect()
    })
}

/// Skia's `SK_ColorBLACK` paint (8-bit color at the folded alpha `a`): it takes
/// `SkARGB32_Black_Blitter`.
fn is_black(color: Hsla, a: u8) -> bool {
    let c = color4f(color);
    a == 255 && [c.r, c.g, c.b] == [0.; 3]
}

fn paint_op(op: Op, color: Hsla, layer: Layer, window: &mut Window, cx: &App) {
    // GPUI folds an `opacity` into a lone sprite's color, as cc folds it into the paint: Skia
    // picks its blitter by that alpha.
    let a = alpha_byte(color.a * window.element_opacity());
    if matches!(op, Op::ClipLayer { .. }) || op.restored(a) {
        // ponytail: restored layers ignore the ancestors' AA clips.
        let mut col = color4f(color);
        col.a *= window.element_opacity();
        let sprites = cached(format!("{op:?}{layer:?}{col:?}"), || {
            restore_sprites(&op, col, layer)
        });
        return paint_sprites(&sprites, color, window, cx);
    }
    let clips = clips(window, layer);
    let black = is_black(color, a);
    let wide = wide(color);
    let sprites = sprites(format!("{op:?}{layer:?}{a}{black}{wide}{clips:?}"), || {
        raster(op.bbox(), layer, a, black, wide, &clips, &|c, col| {
            op.draw(c, col)
        })
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
    let (layer, s) = (Layer::new(window, composited), window.scale_factor());
    // Backdrop blurs read the window-space ops; the raster is in layer space.
    crate::backdrop::record(&ops(d, bounds, s).1, window);
    for (op, color) in ops(d, layer.local(bounds, s), s).1 {
        paint_op(op, color, layer, window, cx);
    }
}

/// `crate::raster_images` on macOS.
pub(crate) fn raster_images(
    images: &[(&image::RgbaImage, Bounds<Pixels>)],
    clip: Bounds<Pixels>,
    radius: gpui::Corners<Pixels>,
    corner: Corner,
    page: Hsla,
    window: &Window,
) -> gpui::RenderImage {
    let (layer, s) = (Layer::new(window, None), window.scale_factor());
    let d = Decoration {
        radius,
        corner,
        ..Default::default()
    };
    let (contour, _) = ops(&d, layer.local(clip, s), s);
    let [x0, y0, w, h] = contour.rr.r.map(|v| v as i32);
    let dev = |v: Pixels| f32::from(v) * s;
    let draws: Vec<_> = images
        .iter()
        .map(|(img, dst)| {
            let (iw, ih) = (img.width() as i32, img.height() as i32);
            let info = ImageInfo::new((iw, ih), ColorType::N32, AlphaType::Premul, None);
            let data = Data::new_copy(img.as_raw());
            let image = skia_safe::images::raster_from_data(&info, data, iw as usize * 4)
                .expect("shapes: image");
            let dst = layer.local(*dst, s);
            let rect = Rect::from_ltrb(
                dev(dst.left()),
                dev(dst.top()),
                dev(dst.right()),
                dev(dst.bottom()),
            );
            (image, rect)
        })
        .collect();
    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    let mut out = vec![0; (w * h * 4) as usize];
    for (o, ts, [ox0, oy0, ox1, oy1]) in tiles([x0, y0, x0 + w, y0 + h], layer.size(), layer.tile) {
        let (px, row) = tile_pixels(o, ts, color4f(page), |c| {
            contour.clip(c, ClipOp::Intersect);
            for (image, dst) in &draws {
                c.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    dst,
                    sampling,
                    &Paint::default(),
                );
            }
        });
        for y in oy0..oy1 {
            for x in ox0..ox1 {
                let i = (y - o[1]) as usize * row + (x - o[0]) as usize * 4;
                let j = (((y - y0) * w + x - x0) * 4) as usize;
                // BGRA, as GPUI's images hold it.
                out[j..j + 4].copy_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
            }
        }
    }
    let out = image::RgbaImage::from_raw(w as u32, h as u32, out).expect("shapes: raster");
    gpui::RenderImage::new([image::Frame::new(out)])
}

/// Blink's paint ops for the decoration `d` of the border box `bounds` (window px at scale
/// `s`), in paint order, and the box's snapped border contour.
pub(crate) fn ops(d: &Decoration, bounds: Bounds<Pixels>, s: f32) -> (Contour, Vec<(Op, Hsla)>) {
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
    let mut ops = Vec::new();
    let visible = |(_, color): &(_, Hsla)| alpha_byte(color.a) > 0;
    ops.extend(
        d.shadows
            .iter()
            .rev()
            .filter(|s| !s.inset)
            .map(shadow)
            .filter(visible)
            .map(|(v, color)| {
                let op = Op::Shadow {
                    border,
                    opaque_bg,
                    has_radius,
                    s: v,
                };
                (op, color)
            }),
    );
    // Blink's background bleed avoidance for a fill under a rounded border: an opaque border
    // shrinks the background into it, a translucent one paints both in a clip layer. Blink
    // decides by the border's width, so a transparent border still gets the clip layer.
    // ponytail: with inset shadows (they paint inside the clip layer too) they stay apart.
    let bleed = match (d.fill, d.border) {
        (Some(f), Some((w, b)))
            if has_radius
                && f32::from(w) > 0.
                && alpha_byte(f.a) > 0
                && !d.shadows.iter().any(|s| s.inset) =>
        {
            Some((f, b))
        }
        _ => None,
    };
    if let Some((fill, color)) = bleed.filter(|(_, b)| alpha_byte(b.a) < 255) {
        let op = if alpha_byte(color.a) == 0 {
            (Op::ClipFill(border), fill)
        } else {
            let fill = color4f(fill);
            let op = Op::ClipLayer {
                outer: border,
                inner,
                fill,
            };
            (op, color)
        };
        ops.push(op);
        return (border, ops);
    }
    let mut background = border;
    if bleed.is_some() {
        // BackgroundContouredRectAdjustedForBleedAvoidance: half the border width inside.
        background.rr.outset(-width / 2.);
    }
    ops.extend(d.fill.map(|color| (Op::Fill(background), color)));
    ops.extend(
        d.shadows
            .iter()
            .rev()
            .filter(|s| s.inset)
            .map(shadow)
            .filter(visible)
            .map(|(v, color)| {
                let op = Op::Inset {
                    bounds: inner,
                    s: v,
                };
                (op, color)
            }),
    );
    ops.extend(d.border.map(|(_, color)| {
        let op = Op::Border {
            outer: border,
            inner,
            dashed: d.dashed,
        };
        (op, color)
    }));
    (border, ops)
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
    ['"', '\''].into_iter().find_map(|q| {
        let at = tag.find(&format!(" {name}={q}"))? + name.len() + 3;
        Some(&tag[at..at + tag[at..].find(q)?])
    })
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
    partial: bool,
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
    // In layer space, like the decorations.
    let b = layer.local(bounds, s);
    let origin = [snap(b.origin.x), snap(b.origin.y)];
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
    // Blink clips the svg (or `<img>`) to its pixel-snapped box; at a path extreme on the
    // box edge that clip changes AAA's edge px.
    let clip = Rect::new(
        origin[0],
        origin[1],
        snap(b.origin.x + bounds.size.width),
        snap(b.origin.y + bounds.size.height),
    );
    let rgba = Rgba::from(color);
    let mut clips = clips(window, layer);
    if partial {
        // cc's playback rect: a plain rect clip the AA clips then start on.
        let rr = Rounded {
            r: [clip.left, clip.top, clip.width(), clip.height()],
            radii: [[0.; 2]; 4],
        };
        clips.insert(
            0,
            Contour {
                rr,
                k: [2.; 4],
                origin: rr,
            },
        );
    }
    for (n, p) in icon.paths.iter().enumerate() {
        let draw = |c: &Canvas, col: Color4f| {
            c.clip_rect(clip, ClipOp::Intersect, true);
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
        let a = alpha_byte(a * window.element_opacity());
        let black = is_black(color.into(), a);
        let sprites = sprites(
            format!("{key}#{n}{flip_x}{clip:?}{w}x{h}{layer:?}{a}{black}{clips:?}"),
            || raster(bbox, layer, a, black, false, &clips, &draw),
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

    #[test]
    fn outset_squircle_is_corner_clipped() {
        let rr = Rounded {
            r: [0., 0., 200., 100.],
            radii: [[44.; 2]; 4],
        };
        let k = [2f32.powf(1.5); 4];
        let mut c = Contour { rr, k, origin: rr };
        c.rr.outset_corrected(16.);
        let clipped = c.path();
        c.origin = c.rr;
        let single = c.path();
        assert_eq!(clipped.bounds(), single.bounds());
        assert_ne!(clipped, single);
    }

    /// Chromium's px of a `rgba(255,255,255,.082)` superellipse border on #181818 at the clip
    /// coverage `m` (all 278 corner px of a lab cell fit `SkBlendARGB32`, the mask blit not).
    #[test]
    fn translucent_squircle_border_blend() {
        let px = [
            (55, 28),
            (150, 35),
            (165, 36),
            (180, 37),
            (193, 38),
            (69, 29),
        ];
        for (m, chromium) in px {
            assert_eq!(blend(ARGB32, 255, 21, 24, m), chromium);
        }
        assert_ne!(blend(MASK, 255, 21, 24, 55), 28);
    }

    /// A [`wide`] color's squircle fill over #242424, as Skia's float pipeline draws it,
    /// against `shaders.metal` mode 4 (highp srcover at the coverage).
    #[test]
    fn wide_fill_blend() {
        let rr = Rounded {
            r: [3., 3., 56., 56.],
            radii: [[25.; 2]; 4],
        };
        let fill = Op::Fill(Contour {
            rr,
            k: [2f32.powf(1.5); 4],
            origin: rr,
        });
        let col = Color4f::new(1.00003, 0.99999, 1., 0.05);
        let bg = 36. / 255.;
        let real = tile_pixels([0, 0], (64, 64), Color4f::new(bg, bg, bg, 1.), |c| {
            fill.draw(c, col)
        });
        let cov = tile_pixels([0, 0], (64, 64), Color4f::new(0., 0., 0., 1.), |c| {
            fill.draw(c, Color4f::new(1., 1., 1., 1.))
        });
        let mut bad = Vec::new();
        for i in (0..cov.0.len()).step_by(4) {
            let m = f32::from(cov.0[i + 1]) / 255.;
            let a = col.a * m;
            let o =
                |c: f32| ((bg * (1. - a) + c * col.a * m).clamp(0., 1.) * 255.).round_ties_even();
            let want = [o(col.r), o(col.g), o(col.b)].map(|v| v as u8);
            if real.0[i..i + 3] != want {
                bad.push((cov.0[i + 1], real.0[i..i + 3].to_vec(), want));
            }
        }
        assert!(bad.is_empty(), "{} px: {bad:?}", bad.len());
    }
}
