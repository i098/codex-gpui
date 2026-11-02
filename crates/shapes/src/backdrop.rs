//! `backdrop-filter: blur()` as Chromium's software compositor draws it (macOS): the backdrop
//! (the target's pixels in the box's snapped rect) blurred by Skia with sigma = blur in device
//! px and mirrored edges at that rect, masked by the box's anti-aliased contour clip
//! (`(px * (coverage + 1)) >> 8`), blended onto the target (lowp srcover), and the box's
//! layer blended over that. Nested boxes filter parent content, not its filtered backdrop.
//!
//! The backdrop comes from the current frame's scene prefix, including text, images and
//! every primitive painted before this box. The same sprite atlas supplies the capture.
//!
//! An isolated popup keeps its translucent content separate from the filtered backdrop.
//! Its shadow bounds set the raster origin and tile size; its paths use box-local coordinates.
//! Already-composited images paint outside the content layer.

use crate::{
    Decoration, Fill,
    skia::{self, Contour, Layer, Op, color4f},
};
use gpui::{Bounds, Corners, DevicePixels, Pixels, RenderImage, Window, WindowId, point, px};
use image::{Frame, RgbaImage};
use skia_safe::{
    Canvas, ClipOp, Color, Color4f, ColorSpace, Paint, Rect, TileMode, gradient, image_filters,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

/// A box with a backdrop filter, device px: its contour, blur sigma and layer ops; `gradient`:
/// its fill is a one-color `linear-gradient()` image of this color (8-bit, as Blink passes it:
/// an alpha of 0.96 is 245, whose premultiplied white the dither leaves at 245); `gap_row`: see
/// [`crate::backdrop_blur`].
#[derive(Clone, Debug)]
struct Filtered {
    contour: Contour,
    sigma: f32,
    ops: Vec<(Op, Color4f)>,
    gradient: Option<Color4f>,
    gap_row: bool,
}

struct Surface {
    filtered: Filtered,
    backdrop: Pixmap,
    // Initialized on the first nested box; `backdrop` then holds the blurred parent backdrop.
    layer: Option<Pixmap>,
    checkpoint: (u64, usize),
    raster: Option<(u8, Bounds<Pixels>, [i32; 4])>,
}

#[derive(Default)]
struct FrameState {
    key: Option<(WindowId, u64)>,
    surfaces: Vec<Surface>,
}

thread_local! {
    static FRAME: RefCell<FrameState> = RefCell::new(FrameState::default());
    /// Composed images by their inputs.
    /// ponytail: cleared wholesale when full; an LRU only if animation ever thrashes it.
    static IMAGES: RefCell<HashMap<String, Arc<RenderImage>>> = Default::default();
    /// Blurred backdrops and base layers of boxes with nested boxes, by their inputs: the
    /// blur costs milliseconds, and the backdrop of a still box repeats every frame.
    static LAYERS: RefCell<HashMap<u64, [Pixmap; 2]>> = Default::default();
}

pub(crate) fn compositor_rect(window: &Window) -> Option<[i32; 4]> {
    let (bounds, _) = window.composited_layer()?;
    let owner = window.composited_layer_id()?;
    let key = (
        window.window_handle().window_id(),
        window.scene_checkpoint().0,
    );
    FRAME.with_borrow(|frame| {
        if frame.key != Some(key) {
            return None;
        }
        frame
            .surfaces
            .iter()
            .rev()
            .filter_map(|surface| surface.raster)
            .find_map(|(id, layer_bounds, rect)| {
                (id == owner && layer_bounds == bounds).then_some(rect)
            })
    })
}

/// `crate::backdrop_blur` on macOS.
pub(crate) fn paint(
    blur: Pixels,
    fill: Fill,
    gap_row: bool,
    d: &Decoration,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let s = window.scale_factor();
    let (contour, ops) = skia::ops(d, bounds, s);
    let gradient = d.fill.filter(|_| fill == Fill::Gradient).map(color4f);
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
        gap_row,
    };
    let size = Layer::new(window, None).window;
    let out = region(&filtered, size);
    if out[0] >= out[2] || out[1] >= out[3] {
        return;
    }
    let checkpoint = window.scene_checkpoint();
    let frame_key = (window.window_handle().window_id(), checkpoint.0);
    FRAME.with_borrow_mut(|frame| {
        if frame.key != Some(frame_key) {
            frame.key = Some(frame_key);
            frame.surfaces.clear();
        }
        let parent = frame.surfaces.iter().rposition(|surface| {
            let [x, y, w, h] = filtered.contour.rr.r;
            let [px, py, pw, ph] = surface.filtered.contour.rr.r;
            x >= px && y >= py && x + w <= px + pw && y + h <= py + ph
        });
        let capture_region = parent.map_or(out, |index| frame.surfaces[index].backdrop.r);
        let since = parent.map(|index| frame.surfaces[index].checkpoint);
        let capture_bounds = Bounds::new(
            point(
                DevicePixels(capture_region[0]),
                DevicePixels(capture_region[1]),
            ),
            gpui::size(
                DevicePixels(capture_region[2] - capture_region[0]),
                DevicePixels(capture_region[3] - capture_region[1]),
            ),
        );
        let pixels = match window.capture_scene_prefix(capture_bounds, since) {
            Ok(pixels) => pixels,
            Err(_) => {
                crate::quads(d, bounds, window);
                return;
            }
        };
        let (image, out, content) = if let Some(index) = parent {
            let surface = &mut frame.surfaces[index];
            ensure_layer(surface);
            let layer = surface.layer.as_mut().unwrap();
            for (target, source) in layer.px.iter_mut().zip(pixels) {
                *target = over(source, *target);
            }
            filter(layer, &filtered);
            let out = intersect(out, surface.backdrop.r);
            let mut hasher = DefaultHasher::new();
            surface.backdrop.px.hash(&mut hasher);
            layer.px.hash(&mut hasher);
            let key = format!("nested{out:?}{}", hasher.finish());
            let image = cached_image(key, window, || compose_layer(&surface.backdrop, layer, out));
            (image, out, None)
        } else {
            let backdrop = Pixmap { r: out, px: pixels };
            let mut hasher = DefaultHasher::new();
            backdrop.px.hash(&mut hasher);
            let composited = window.composited_layer().is_some() && !filtered.ops.is_empty();
            let surface_key = format!("surface{filtered:?}{out:?}");
            let key = format!("{composited}{surface_key}{}", hasher.finish());
            let image = cached_image(key, window, || {
                if composited {
                    compose_backdrop(&filtered, &backdrop)
                } else {
                    compose(&filtered, &backdrop)
                }
            });
            let content = composited
                .then(|| cached_image(surface_key, window, || surface_image(&filtered, out)));
            let raster = if composited {
                window
                    .composited_layer_id()
                    .zip(window.composited_layer())
                    .map(|(owner, (bounds, _))| (owner, bounds, layer_rect(&filtered)))
            } else {
                None
            };
            frame.surfaces.push(Surface {
                filtered,
                backdrop,
                layer: None,
                checkpoint,
                raster,
            });
            (image, out, content)
        };
        let at = Bounds::new(
            point(px(out[0] as f32 / s), px(out[1] as f32 / s)),
            gpui::size(
                px((out[2] - out[0]) as f32 / s),
                px((out[3] - out[1]) as f32 / s),
            ),
        );
        if let Some(content) = content {
            window
                .with_unlayered_paint(|window| {
                    window.paint_image(at, at, Corners::default(), image, 0, false)
                })
                .ok();
            window
                .paint_image(at, at, Corners::default(), content, 0, false)
                .ok();
        } else {
            window
                .with_unlayered_paint(|window| {
                    window.paint_image(at, at, Corners::default(), image, 0, false)
                })
                .ok();
        }
        let index = parent.unwrap_or(frame.surfaces.len() - 1);
        frame.surfaces[index].checkpoint = window.scene_checkpoint();
    });
}

/// `crate::paint_icon_in_backdrop` on macOS: draws the icon in an `opacity` layer on the layer
/// of the innermost painted backdrop box that holds its box, then shows that box's blurred
/// backdrop and layer under the icon's box; `clip`: the box clips its content to its contour
/// (`overflow: hidden`). False when no such box was painted this frame.
pub(crate) fn paint_icon(
    src: &[u8],
    key: &str,
    bounds: Bounds<Pixels>,
    color: gpui::Hsla,
    opacity: f32,
    clip: bool,
    window: &mut Window,
) -> bool {
    let s = window.scale_factor();
    let snap = |v: Pixels| (f32::from(v) * s + 0.5).floor() as i32;
    let rect = [
        snap(bounds.left()),
        snap(bounds.top()),
        snap(bounds.right()),
        snap(bounds.bottom()),
    ];
    let frame_key = (
        window.window_handle().window_id(),
        window.scene_checkpoint().0,
    );
    let painted = FRAME.with_borrow_mut(|frame| {
        if frame.key != Some(frame_key) {
            return None;
        }
        let index = frame.surfaces.iter().rposition(|surface| {
            let r = snapped(&surface.filtered.contour);
            surface.raster.is_none()
                && r[0] <= rect[0]
                && r[1] <= rect[1]
                && rect[2] <= r[2]
                && rect[3] <= r[3]
        })?;
        let surface = &mut frame.surfaces[index];
        ensure_layer(surface);
        let (origin, contour) = (layer_rect(&surface.filtered), surface.filtered.contour);
        let layer = surface.layer.as_mut().unwrap();
        let out = intersect(rect, layer.r);
        if out[0] >= out[2] || out[1] >= out[3] {
            return None;
        }
        // Chromium rasters the icon's opacity group on the box's composited layer.
        let (before, at) = (layer.image(), [layer.r[0] as f32, layer.r[1] as f32]);
        let mut drawn = true;
        layer.blit(
            out,
            Color::TRANSPARENT,
            [origin[0], origin[1]],
            Some((origin[2] - origin[0], origin[3] - origin[1])),
            ColorSpace::new_srgb(),
            |c| {
                let mut copy = Paint::default();
                copy.set_blend_mode(skia_safe::BlendMode::Src);
                c.draw_image(&before, (at[0], at[1]), Some(&copy));
                c.save();
                if clip {
                    contour.clip(c, ClipOp::Intersect);
                }
                c.save_layer_alpha_f(None, opacity);
                drawn = skia::draw_icon(c, src, key, bounds, color, s);
                c.restore();
                c.restore();
            },
        );
        if !drawn {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        surface.backdrop.px.hash(&mut hasher);
        layer.px.hash(&mut hasher);
        let image_key = format!("icon{out:?}{}", hasher.finish());
        let backdrop = &surface.backdrop;
        let image = cached_image(image_key, window, || compose_layer(backdrop, layer, out));
        Some((image, out, index))
    });
    let Some((image, out, index)) = painted else {
        return false;
    };
    let at = Bounds::new(
        point(px(out[0] as f32 / s), px(out[1] as f32 / s)),
        gpui::size(
            px((out[2] - out[0]) as f32 / s),
            px((out[3] - out[1]) as f32 / s),
        ),
    );
    window
        .with_unlayered_paint(|window| {
            window.paint_image(at, at, Corners::default(), image, 0, false)
        })
        .ok();
    // Nested boxes capture what paints after the icon, not the icon's composed px again.
    let checkpoint = window.scene_checkpoint();
    FRAME.with_borrow_mut(|frame| frame.surfaces[index].checkpoint = checkpoint);
    true
}

/// Blurs `surface`'s backdrop and starts its layer, unless already done this frame.
fn ensure_layer(surface: &mut Surface) {
    if surface.layer.is_some() {
        return;
    }
    let mut hasher = DefaultHasher::new();
    format!("{:?}{:?}", surface.filtered, surface.backdrop.r).hash(&mut hasher);
    surface.backdrop.px.hash(&mut hasher);
    let key = hasher.finish();
    let [blurred, layer] = LAYERS.with_borrow_mut(|layers| {
        if let Some(hit) = layers.get(&key) {
            return hit.clone();
        }
        // ponytail: cleared wholesale when full, as IMAGES.
        if layers.len() >= 16 {
            layers.clear();
        }
        let mut backdrop = surface.backdrop.clone();
        blur_backdrop(&mut backdrop, &surface.filtered);
        let mut layer = Pixmap::new(backdrop.r);
        draw_layer(&mut layer, &surface.filtered);
        layers.insert(key, [backdrop.clone(), layer.clone()]);
        [backdrop, layer]
    });
    surface.backdrop = blurred;
    surface.layer = Some(layer);
}

fn cached_image(
    key: String,
    window: &mut Window,
    make: impl FnOnce() -> RenderImage,
) -> Arc<RenderImage> {
    IMAGES.with_borrow_mut(|images| {
        if let Some(image) = images.get(&key) {
            return image.clone();
        }
        if images.len() >= 64 {
            images.retain(|_, image| {
                if window.is_image_in_use(image) {
                    true
                } else {
                    window.drop_image(image.clone()).ok();
                    false
                }
            });
        }
        let image = Arc::new(make());
        images.insert(key, image.clone());
        image
    })
}

/// The rect `[x0, y0, x1, y1]` both `a` and `b` cover (empty if none).
fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

/// The box's integer rect `[x0, y0, x1, y1]` (Blink snaps it).
fn snapped(c: &Contour) -> [i32; 4] {
    let r = c.rr.r;
    [r[0], r[1], r[0] + r[2], r[1] + r[3]].map(|v| v.round() as i32)
}

/// Device px the surface can change: its rect and its layer ops, within the window.
fn region(filtered: &Filtered, window: [i32; 2]) -> [i32; 4] {
    let mut r = snapped(&filtered.contour);
    for (op, _) in &filtered.ops {
        let b = op.bbox();
        r = [
            r[0].min(b[0]),
            r[1].min(b[1]),
            r[2].max(b[2]),
            r[3].max(b[3]),
        ];
    }
    intersect(r, [0, 0, window[0], window[1]])
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

    /// Renders `draw` (window device px) on a compositor surface of `r` cleared to `clear` and
    /// reads it.
    fn draw(r: [i32; 4], clear: Color, draw: impl FnOnce(&Canvas)) -> Self {
        let mut out = Self::new(r);
        out.blit(r, clear, [r[0], r[1]], None, None, draw);
        out
    }

    /// Renders `draw` on a surface of color space `cs` at `origin` (window device px) of `size`
    /// (`None`: to `area`'s end), and copies the pixels of `area` into `self`.
    fn blit(
        &mut self,
        area: [i32; 4],
        clear: Color,
        origin: [i32; 2],
        size: Option<(i32, i32)>,
        cs: impl Into<Option<ColorSpace>>,
        draw: impl FnOnce(&Canvas),
    ) {
        let size = size.unwrap_or((area[2] - origin[0], area[3] - origin[1]));
        let (bytes, rb) = skia::surface_pixels(origin, size, clear.into(), cs, draw);
        for y in area[1]..area[3] {
            for x in area[0]..area[2] {
                let o = (y - origin[1]) as usize * rb + (x - origin[0]) as usize * 4;
                let i = self.index(x, y);
                self.px[i] = [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]];
            }
        }
    }

    /// The pixels of `r` on a transparent pixmap of `area` (which holds `r`).
    fn sub(&self, r: [i32; 4], area: [i32; 4]) -> Self {
        let mut out = Self::new(area);
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
    blur_backdrop(target, f);
    draw_layer(target, f);
}

/// The backdrop filter of `f`: `target` blurred in the box's rect (clamped to `target`'s
/// region; with `gap_row`, plus a transparent row below), masked by the contour and blended
/// back onto `target`.
fn blur_backdrop(target: &mut Pixmap, f: &Filtered) {
    let (contour, sigma) = (&f.contour, f.sigma);
    let rect = intersect(snapped(contour), target.r);
    if rect[0] < rect[2] && rect[1] < rect[3] {
        let src = [rect[0], rect[1], rect[2], rect[3] + i32::from(f.gap_row)];
        let (w, h) = (src[2] - src[0], src[3] - src[1]);
        // In the draw's local space: window device px.
        let crop = Rect::from_xywh(src[0] as f32, src[1] as f32, w as f32, h as f32);
        let blur = image_filters::blur((sigma, sigma), TileMode::Mirror, None, Some(crop.into()))
            .expect("blur filter");
        let mut paint = Paint::default();
        paint.set_image_filter(blur);
        let source = target.sub(rect, src).image();
        let blurred = Pixmap::draw(rect, Color::TRANSPARENT, |c| {
            c.draw_image(&source, (rect[0] as f32, rect[1] as f32), Some(&paint));
        });
        // Chromium's mask layer clips with the path in the box's own space: Skia's convexity
        // test on those coordinates (a squircle can come out concave) picks AAA's walker.
        let coverage = Pixmap::draw(rect, Color::BLACK, |c| {
            let [x, y, ..] = contour.rr.r;
            c.translate((x, y));
            contour.moved([-x, -y]).clip(c, ClipOp::Intersect);
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
    let layer = raster_layer(f, target.r, [0., 0.]);
    for (d, s) in target.px.iter_mut().zip(&layer.px) {
        *d = over(*s, *d);
    }
}

fn raster_layer(f: &Filtered, area: [i32; 4], origin: [f32; 2]) -> Pixmap {
    // Chromium rasters the box's composited layer once per cc tile: Skia's dither matrix and
    // the shadow's mask blur (cut at the device edge) start at each tile's origin.
    let r = layer_rect(f);
    let size = [r[2] - r[0], r[3] - r[1]];
    let local = [
        area[0] - r[0],
        area[1] - r[1],
        area[2] - r[0],
        area[3] - r[1],
    ];
    let mut layer = Pixmap::new(area);
    let draw = |c: &Canvas| {
        let [x, y] = origin;
        c.translate((x, y));
        for &(op, color) in &f.ops {
            let op = op.moved([-x, -y]);
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
    };
    for (o, ts, owned) in skia::tiles(local, size, skia::cc_tile(size)) {
        let owned = [
            owned[0] + r[0],
            owned[1] + r[1],
            owned[2] + r[0],
            owned[3] + r[1],
        ];
        let target = intersect(owned, area);
        if target[0] < target[2] && target[1] < target[3] {
            let o = [o[0] + r[0], o[1] + r[1]];
            layer.blit(
                target,
                Color::TRANSPARENT,
                o,
                Some(ts),
                ColorSpace::new_srgb(),
                draw,
            );
        }
    }
    layer
}

/// The box's composited layer, device px: its rect and its shadows' ink overflow (Blink's
/// `ShadowData::RectOutsets`: three sigmas plus the spread, moved by the offset).
fn layer_rect(f: &Filtered) -> [i32; 4] {
    let mut r = snapped(&f.contour);
    for (op, _) in &f.ops {
        if let Op::Shadow { border, s, .. } = op {
            let e = 1.5 * s[2] + s[3];
            let [x, y, w, h] = border.rr.r;
            r = [
                r[0].min((x - e + s[0]).floor() as i32),
                r[1].min((y - e + s[1]).floor() as i32),
                r[2].max((x + w + e + s[0]).ceil() as i32),
                r[3].max((y + h + e + s[1]).ceil() as i32),
            ];
        }
    }
    r
}

/// Composes the filtered box over its same-frame backdrop as a BGRA image.
fn compose(filtered: &Filtered, before: &Pixmap) -> RenderImage {
    let mut target = before.clone();
    filter(&mut target, filtered);
    changed_image(&target, before)
}

fn compose_backdrop(filtered: &Filtered, before: &Pixmap) -> RenderImage {
    let mut target = before.clone();
    blur_backdrop(&mut target, filtered);
    changed_image(&target, before)
}

fn changed_image(target: &Pixmap, before: &Pixmap) -> RenderImage {
    let [x0, y0, x1, y1] = target.r;
    let mut image = RgbaImage::new((x1 - x0) as u32, (y1 - y0) as u32);
    for (pixel, (&after, &before)) in image.pixels_mut().zip(target.px.iter().zip(&before.px)) {
        if after != before {
            *pixel = image::Rgba([after[2], after[1], after[0], 255]);
        }
    }
    RenderImage::new(vec![Frame::new(image)])
}

fn surface_image(filtered: &Filtered, area: [i32; 4]) -> RenderImage {
    let [x, y, ..] = filtered.contour.rr.r;
    let mut layer = raster_layer(filtered, area, [x, y]);
    for pixel in &mut layer.px {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
        pixel.swap(0, 2);
    }
    let image = RgbaImage::from_raw(
        (area[2] - area[0]) as u32,
        (area[3] - area[1]) as u32,
        layer.px.into_flattened(),
    )
    .expect("surface image size");
    RenderImage::new(vec![Frame::new(image)])
}

fn compose_layer(backdrop: &Pixmap, layer: &Pixmap, out: [i32; 4]) -> RenderImage {
    let mut image = RgbaImage::new((out[2] - out[0]) as u32, (out[3] - out[1]) as u32);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let index = backdrop.index(out[0] + x as i32, out[1] + y as i32);
        let value = over(layer.px[index], backdrop.px[index]);
        *pixel = image::Rgba([value[2], value[1], value[0], 255]);
    }
    RenderImage::new(vec![Frame::new(image)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, IntoElement, ParentElement, Render, Styled, div, size};

    struct BackdropScene {
        changed: bool,
        popup: bool,
        image: Arc<RenderImage>,
    }

    fn picture(revision: u8) -> Arc<RenderImage> {
        let image = RgbaImage::from_fn(12, 12, |x, y| {
            let bright = (x < 6) != (revision % 2 == 1);
            image::Rgba(if bright {
                [230 - revision, 40 + y as u8 * 10, 70, 255]
            } else {
                [30, 160, 220 - y as u8 * 10 - revision, 255]
            })
        });
        Arc::new(RenderImage::new(vec![Frame::new(image)]))
    }

    fn popup(bounds: Bounds<Pixels>, blur: f32) -> impl IntoElement {
        div()
            .absolute()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .child(crate::backdrop_blur(
                px(blur),
                Fill::Color,
                false,
                Decoration::default(),
            ))
    }

    fn boxes() -> [(Bounds<Pixels>, f32); 2] {
        [
            (
                Bounds::new(point(px(8.), px(8.)), size(px(48.), px(20.))),
                2.,
            ),
            (
                Bounds::new(point(px(8.), px(32.)), size(px(48.), px(24.))),
                1.,
            ),
        ]
    }

    impl Render for BackdropScene {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let image = self.image.clone();
            div()
                .size_full()
                .relative()
                .child(crate::decoration(Decoration {
                    fill: Some(gpui::rgb(0x242424).into()),
                    ..Default::default()
                }))
                .child(
                    div()
                        .absolute()
                        .left(px(12.))
                        .top(px(12.))
                        .font_family(".SystemUIFont")
                        .text_size(px(14.))
                        .line_height(px(18.))
                        .text_color(gpui::white())
                        .child(if self.changed { "New" } else { "Old" }),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(12.))
                        .top(px(36.))
                        .w(px(40.))
                        .h(px(16.))
                        .overflow_hidden()
                        .opacity(0.65)
                        .child(
                            gpui::canvas(
                                move |_, _, _| image,
                                |bounds, image, window, _| {
                                    window
                                        .paint_image(
                                            bounds,
                                            bounds,
                                            Corners::default(),
                                            image,
                                            0,
                                            false,
                                        )
                                        .expect("paint the source image");
                                },
                            )
                            .size_full(),
                        ),
                )
                .children(
                    self.popup
                        .then(boxes)
                        .into_iter()
                        .flatten()
                        .map(|(bounds, blur)| popup(bounds, blur)),
                )
        }
    }

    #[test]
    fn backdrop_uses_same_frame_text_and_images() -> anyhow::Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        let window = cx.open_window(size(px(64.), px(64.)), |_, cx| {
            cx.new(|_| BackdropScene {
                changed: false,
                popup: false,
                image: picture(0),
            })
        })?;
        let view = cx.read_window(&window, |view, _| view)?;
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| {
            view.changed = true;
            view.popup = true;
            view.image = picture(1);
            cx.notify();
        });
        cx.run_until_parked();
        for revision in 2..70 {
            view.update(&mut cx, |view, cx| {
                view.image = picture(revision);
                cx.notify();
            });
            cx.run_until_parked();
        }
        let actual = cx.capture_screenshot(window.into())?;
        view.update(&mut cx, |view, cx| {
            view.popup = false;
            cx.notify();
        });
        cx.run_until_parked();
        let baseline = cx.capture_screenshot(window.into())?;
        let mut expected = Pixmap {
            r: [0, 0, 128, 128],
            px: baseline.pixels().map(|pixel| pixel.0).collect(),
        };
        for (bounds, blur) in boxes() {
            let (contour, ops) = skia::ops(&Decoration::default(), bounds, 2.);
            filter(
                &mut expected,
                &Filtered {
                    contour,
                    sigma: blur * 2.,
                    ops: ops
                        .into_iter()
                        .map(|(op, color)| (op, color4f(color)))
                        .collect(),
                    gradient: None,
                    gap_row: false,
                },
            );
        }
        if let Some(out) = std::env::var_os("CGX_BACKDROP_PROOF") {
            std::fs::create_dir_all(&out)?;
            actual.save(std::path::Path::new(&out).join("actual.png"))?;
            baseline.save(std::path::Path::new(&out).join("baseline.png"))?;
            RgbaImage::from_raw(128, 128, expected.px.clone().into_flattened())
                .unwrap()
                .save(std::path::Path::new(&out).join("expected.png"))?;
        }
        assert!(
            actual.as_raw().as_slice() == expected.px.as_flattened(),
            "the backdrop must include same-frame text and clipped translucent images"
        );
        Ok(())
    }
}
