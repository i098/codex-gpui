//! `backdrop-filter: blur()` as Chromium's software compositor draws it (macOS): the backdrop
//! (the target's pixels in the box's snapped rect) blurred by Skia with sigma = blur in device
//! px and mirrored edges at that rect, masked by the box's anti-aliased contour clip
//! (`(px * (coverage + 1)) >> 8`), blended onto the target (lowp srcover), and the box's
//! layer blended over that. A nested box does the same inside its outer box's layer.
//!
//! The backdrop is rebuilt from the decorations `shapes` painted this frame (recorded by
//! [`record`], rasterized again per cc tile), so the box's final pixels are computed here and
//! painted as one image.

use crate::{
    Decoration, Fill,
    skia::{self, Contour, Layer, Op, STRIDE, TILE, color4f},
};
use gpui::{Bounds, Corners, Hsla, Pixels, RenderImage, Rgba, Window, WindowId, point, px};
use image::{Frame, RgbaImage};
use skia_safe::{Canvas, ClipOp, Color, Color4f, Paint, Rect, TileMode, gradient, image_filters};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

/// A box with a backdrop filter, device px: its contour, blur sigma and layer ops; `gradient`:
/// its fill is a one-color `linear-gradient()` image of this (unquantized) color.
#[derive(Clone, Debug)]
struct Filtered {
    contour: Contour,
    sigma: f32,
    ops: Vec<(Op, Color4f)>,
    gradient: Option<Color4f>,
}

/// A backdrop-filtered box painted at the root: its backdrop is the first `below` painted
/// ops; `nested` boxes blur its layer, in paint order.
#[derive(Clone, Debug)]
struct Surface {
    filtered: Filtered,
    below: usize,
    nested: Vec<Filtered>,
}

#[derive(Default)]
struct FrameState {
    /// Decoration ops painted at the root, device px.
    painted: Vec<(Op, Color4f)>,
    surfaces: Vec<Surface>,
}

thread_local! {
    /// What each window painted this frame; dropped after the frame.
    static FRAMES: RefCell<HashMap<WindowId, FrameState>> = Default::default();
    /// Composed images by their inputs.
    /// ponytail: cleared wholesale when full; an LRU only if animation ever thrashes it.
    static IMAGES: RefCell<HashMap<String, Arc<RenderImage>>> = Default::default();
}

fn with_frame<R>(window: &Window, f: impl FnOnce(&mut FrameState) -> R) -> R {
    let id = window.window_handle().window_id();
    FRAMES.with_borrow_mut(|frames| {
        if !frames.contains_key(&id) {
            window.on_next_frame(move |_, _| {
                FRAMES.with_borrow_mut(|frames| frames.remove(&id));
            });
        }
        f(frames.entry(id).or_default())
    })
}

/// Records root decoration ops for the backdrops painted later this frame.
/// ponytail: ops of composited layers (scroll contents) are drawn as if at the root.
pub(crate) fn record(ops: &[(Op, Hsla)], window: &Window) {
    with_frame(window, |f| {
        f.painted
            .extend(ops.iter().map(|&(op, c)| (op, color4f(c))))
    });
}

/// `crate::backdrop_blur` on macOS.
pub(crate) fn paint(
    blur: Pixels,
    fill: Fill,
    d: &Decoration,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let s = window.scale_factor();
    let (contour, ops) = skia::ops(d, bounds, s);
    let gradient = d.fill.filter(|_| fill == Fill::Gradient).map(|c| {
        let c = Rgba::from(c);
        Color4f::new(c.r, c.g, c.b, c.a)
    });
    let filtered = Filtered {
        contour,
        sigma: f32::from(blur) * s,
        ops: ops
            .into_iter()
            .map(|(mut op, c)| {
                // An image background does not count as opaque for the shadow clip.
                if let Op::Shadow { opaque_bg, .. } = &mut op {
                    *opaque_bg &= gradient.is_none();
                }
                (op, color4f(c))
            })
            .collect(),
        gradient,
    };
    let size = Layer::new(window, None).window;
    let (surface, painted) = with_frame(window, |f| {
        let r = contour.rr.r;
        let inside = |o: &Surface| {
            let q = o.filtered.contour.rr.r;
            r[0] >= q[0] && r[1] >= q[1] && r[0] + r[2] <= q[0] + q[2] && r[1] + r[3] <= q[1] + q[3]
        };
        let i = match f.surfaces.iter().rposition(inside) {
            Some(i) => {
                f.surfaces[i].nested.push(filtered);
                i
            }
            None => {
                f.surfaces.push(Surface {
                    filtered,
                    below: f.painted.len(),
                    nested: Vec::new(),
                });
                f.surfaces.len() - 1
            }
        };
        let s = &f.surfaces[i];
        (s.clone(), f.painted[..s.below].to_vec())
    });
    let region = region(&surface, size);
    if region[0] >= region[2] || region[1] >= region[3] {
        return;
    }
    let painted: Vec<(Op, Color4f)> = painted
        .into_iter()
        .filter(|(op, _)| overlaps(op.bbox(), region))
        .collect();
    let key = format!("{surface:?}{painted:?}{region:?}");
    let image = IMAGES.with_borrow_mut(|images| {
        if let Some(image) = images.get(&key) {
            return image.clone();
        }
        if images.len() >= 64 {
            for (_, image) in images.drain() {
                window.drop_image(image).ok();
            }
        }
        let image = Arc::new(compose(&surface, &painted, region, size));
        images.insert(key, image.clone());
        image
    });
    let at = Bounds::new(
        point(px(region[0] as f32 / s), px(region[1] as f32 / s)),
        gpui::size(
            px((region[2] - region[0]) as f32 / s),
            px((region[3] - region[1]) as f32 / s),
        ),
    );
    window
        .paint_image(at, at, Corners::default(), image, 0, false)
        .ok();
}

fn overlaps(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// The box's integer rect `[x0, y0, x1, y1]` (Blink snaps it).
fn snapped(c: &Contour) -> [i32; 4] {
    let r = c.rr.r;
    [r[0], r[1], r[0] + r[2], r[1] + r[3]].map(|v| v.round() as i32)
}

/// Device px the surface can change: its rect and its layer ops, within the window.
fn region(s: &Surface, window: [i32; 2]) -> [i32; 4] {
    let mut r = snapped(&s.filtered.contour);
    for (op, _) in s
        .filtered
        .ops
        .iter()
        .chain(s.nested.iter().flat_map(|n| &n.ops))
    {
        let b = op.bbox();
        r = [
            r[0].min(b[0]),
            r[1].min(b[1]),
            r[2].max(b[2]),
            r[3].max(b[3]),
        ];
    }
    [
        r[0].max(0),
        r[1].max(0),
        r[2].min(window[0]),
        r[3].min(window[1]),
    ]
}

/// Premultiplied RGBA pixels of a `[x0, y0, x1, y1]` region.
#[derive(Clone)]
struct Pixmap {
    r: [i32; 4],
    px: Vec<[u8; 4]>,
}

impl Pixmap {
    fn new(r: [i32; 4]) -> Self {
        Self {
            r,
            px: vec![[0; 4]; ((r[2] - r[0]) * (r[3] - r[1])) as usize],
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        ((y - self.r[1]) * (self.r[2] - self.r[0]) + x - self.r[0]) as usize
    }

    /// Renders `draw` (window device px) on a surface of `r` cleared to `clear` and reads it.
    fn draw(r: [i32; 4], clear: Color, draw: impl FnOnce(&Canvas)) -> Self {
        let mut out = Self::new(r);
        out.blit(r, clear, [r[0], r[1]], draw);
        out
    }

    /// Renders `draw` on a surface at `origin` (window device px) covering `area`, and copies
    /// the pixels of `area` into `self`.
    fn blit(&mut self, area: [i32; 4], clear: Color, origin: [i32; 2], draw: impl FnOnce(&Canvas)) {
        let size = (area[2] - origin[0], area[3] - origin[1]);
        let (bytes, rb) = skia::tile_pixels(origin, size, clear.into(), draw);
        for y in area[1]..area[3] {
            for x in area[0]..area[2] {
                let o = (y - origin[1]) as usize * rb + (x - origin[0]) as usize * 4;
                let i = self.index(x, y);
                self.px[i] = [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]];
            }
        }
    }

    fn sub(&self, r: [i32; 4]) -> Self {
        let mut out = Self::new(r);
        for y in r[1]..r[3] {
            for x in r[0]..r[2] {
                let i = out.index(x, y);
                out.px[i] = self.px[self.index(x, y)];
            }
        }
        out
    }

    fn image(&self) -> skia_safe::Image {
        let (w, h) = (self.r[2] - self.r[0], self.r[3] - self.r[1]);
        let info = skia_safe::ImageInfo::new(
            (w, h),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let data = skia_safe::Data::new_copy(self.px.as_flattened());
        skia_safe::images::raster_from_data(&info, data, w as usize * 4).expect("backdrop image")
    }
}

/// Lowp srcover with Skia's rounding `div255`.
fn over(s: [u8; 4], d: [u8; 4]) -> [u8; 4] {
    let inv = 255 - u32::from(s[3]);
    std::array::from_fn(|i| (u32::from(s[i]) + skia::div255(u32::from(d[i]) * inv)).min(255) as u8)
}

/// Applies `f`'s backdrop filter to `target`, then blends `f`'s own layer over it.
fn filter(target: &mut Pixmap, f: &Filtered) {
    blur_backdrop(target, &f.contour, f.sigma);
    draw_layer(target, f);
}

/// The backdrop filter of a box with contour `contour`: `target` blurred in the box's rect
/// (clamped to `target`'s region), masked by the contour and blended back onto `target`.
fn blur_backdrop(target: &mut Pixmap, contour: &Contour, sigma: f32) {
    let rect = snapped(contour);
    let rect = [
        rect[0].max(target.r[0]),
        rect[1].max(target.r[1]),
        rect[2].min(target.r[2]),
        rect[3].min(target.r[3]),
    ];
    if rect[0] < rect[2] && rect[1] < rect[3] {
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        // In the draw's local space: window device px.
        let crop = Rect::from_xywh(rect[0] as f32, rect[1] as f32, w as f32, h as f32);
        let blur = image_filters::blur((sigma, sigma), TileMode::Mirror, None, Some(crop.into()))
            .expect("blur filter");
        let mut paint = Paint::default();
        paint.set_image_filter(blur);
        let source = target.sub(rect).image();
        let blurred = Pixmap::draw(rect, Color::TRANSPARENT, |c| {
            c.draw_image(&source, (rect[0] as f32, rect[1] as f32), Some(&paint));
        });
        let coverage = Pixmap::draw(rect, Color::BLACK, |c| {
            contour.clip(c, ClipOp::Intersect);
            c.draw_paint(&Paint::new(Color4f::new(1., 1., 1., 1.), None));
        });
        for y in rect[1]..rect[3] {
            for x in rect[0]..rect[2] {
                let i = blurred.index(x, y);
                let scale = u32::from(coverage.px[i][1]) + 1;
                let m = blurred.px[i].map(|v| ((u32::from(v) * scale) >> 8) as u8);
                let t = target.index(x, y);
                target.px[t] = over(m, target.px[t]);
            }
        }
    }
}

/// Rasterizes `f`'s layer ops into a transparent layer and blends it onto `target`.
fn draw_layer(target: &mut Pixmap, f: &Filtered) {
    if f.ops.is_empty() {
        return;
    }
    // The layer rasterizes from the box's origin (Skia's dither matrix is anchored there): a
    // surface origin congruent to it modulo the 8 px matrix, at or before the region.
    let b = snapped(&f.contour);
    let align = |v: i32, min: i32| v - ((v - min).max(0) + 7) / 8 * 8;
    let o = [align(b[0], target.r[0]), align(b[1], target.r[1])];
    let mut layer = Pixmap::new(target.r);
    layer.blit(target.r, Color::TRANSPARENT, o, |c| {
        for &(op, color) in &f.ops {
            c.save();
            match (op, f.gradient) {
                // A `background-image`: Blink clips to the shape and draws the image rect;
                // Skia dithers gradients.
                (Op::Fill(contour), Some(color)) => {
                    let [x, y, w, h] = contour.rr.r;
                    let rect = Rect::from_xywh(x, y, w, h);
                    let colors = [color, color];
                    let gradient = gradient::Gradient::new(
                        gradient::Colors::new_evenly_spaced(&colors, TileMode::Clamp, None),
                        gradient::Interpolation::default(),
                    );
                    let shader = gradient::shaders::linear_gradient(
                        (rect.tl(), skia_safe::Point::new(x, y + h)),
                        &gradient,
                        None,
                    )
                    .expect("gradient shader");
                    let mut paint = Paint::default();
                    paint.set_shader(shader).set_dither(true);
                    contour.clip(c, ClipOp::Intersect);
                    c.draw_rect(rect, &paint);
                }
                _ => op.draw(c, color),
            }
            c.restore();
        }
    });
    for (d, s) in target.px.iter_mut().zip(&layer.px) {
        *d = over(*s, *d);
    }
}

/// The surface's final pixels over `region` as a BGRA image: transparent where the target is
/// unchanged.
fn compose(
    s: &Surface,
    painted: &[(Op, Color4f)],
    region: [i32; 4],
    window: [i32; 2],
) -> RenderImage {
    // The backdrop: the painted ops, rasterized per root cc tile like Chromium.
    let mut root = Pixmap::new(region);
    let owner = |v: i32| if v <= 0 { 0 } else { (v - 1) / STRIDE };
    let own = |v: i32, t: i32| if t == 0 { 0 } else { v + 1 };
    for ty in owner(region[1])..=owner(region[3] - 1) {
        for tx in owner(region[0])..=owner(region[2] - 1) {
            let o = [tx * STRIDE, ty * STRIDE];
            let area = [
                region[0].max(own(o[0], tx)),
                region[1].max(own(o[1], ty)),
                region[2]
                    .min(o[0] + STRIDE + 1)
                    .min(o[0] + TILE)
                    .min(window[0]),
                region[3]
                    .min(o[1] + STRIDE + 1)
                    .min(o[1] + TILE)
                    .min(window[1]),
            ];
            if area[0] >= area[2] || area[1] >= area[3] {
                continue;
            }
            root.blit(area, Color::BLACK, o, |c| {
                for &(op, color) in painted {
                    c.save();
                    op.draw(c, color);
                    c.restore();
                }
            });
        }
    }
    let before = root.clone();
    // The surface's layer, with its nested backdrop-filtered boxes.
    let mut layer = Pixmap::new(region);
    draw_layer(&mut layer, &s.filtered);
    for n in &s.nested {
        filter(&mut layer, n);
    }
    blur_backdrop(&mut root, &s.filtered.contour, s.filtered.sigma);
    let mut out = RgbaImage::new(
        (region[2] - region[0]) as u32,
        (region[3] - region[1]) as u32,
    );
    for (i, (p, (r, b))) in out
        .pixels_mut()
        .zip(root.px.iter().zip(&before.px))
        .enumerate()
    {
        let l = layer.px[i];
        let f = over(l, *r);
        if l[3] > 0 || r != b {
            // BGRA for GPUI; opaque.
            *p = image::Rgba([f[2], f[1], f[0], 255]);
        }
    }
    RenderImage::new(vec![Frame::new(out)])
}
