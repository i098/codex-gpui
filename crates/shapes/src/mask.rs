//! CSS `mask-image` on a composited layer (macOS): viz multiplies the layer's pixels by the
//! mask layer's alpha (`DstIn`) before it composites the layer, which the shader does to the
//! GPUI layer's pixel with [`DST_IN`] sprites.

use crate::skia::{ARGB32, Blits, CLIP, DST_IN, MASK, paint_sprites, sprites, tile_pixels};
use gpui::{App, Bounds, Corners, Hsla, Pixels, Rgba, Window};
use skia_safe::{Color4f, Paint, RRect, Rect, Vector};
use std::collections::BTreeMap;

/// `crate::top_fade` on macOS.
pub(crate) fn paint_top_fade(fade: Pixels, bounds: Bounds<Pixels>, window: &mut Window, cx: &App) {
    let s = window.scale_factor();
    let dev = |v: Pixels| (f32::from(v) * s).round() as i32;
    let (x0, y0, fade) = (dev(bounds.left()), dev(bounds.top()), dev(fade));
    let (w, h) = (
        dev(bounds.right()) - x0,
        fade.min(dev(bounds.bottom()) - y0),
    );
    if w <= 0 || h <= 0 {
        return;
    }
    let sprites = sprites(format!("top_fade {x0} {y0} {w} {h} {fade}"), || {
        // Skia's gradient alpha at each row's center, rounded to a byte; the rows past the
        // fade are opaque and need no sprite.
        let blits = (0..h)
            .flat_map(|y| {
                let alpha = ((y as f32 + 0.5) / fade as f32 * 255.).round() as u8;
                std::iter::repeat_n([alpha, DST_IN, 0, MASK] as Blits, w as usize)
            })
            .collect();
        ([x0, y0, w, h], blits)
    });
    paint_sprites(&sprites, Hsla::default(), [0, 0], window, cx);
}

/// `crate::x_fade` on macOS: the gradient's alpha at each column's center, rounded to a byte,
/// over all of the box's rows.
pub(crate) fn paint_x_fade(
    left: Pixels,
    right: Pixels,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    let s = window.scale_factor();
    let dev = |v: Pixels| (f32::from(v) * s).round() as i32;
    let (x0, y0) = (dev(bounds.left()), dev(bounds.top()));
    let (w, h) = (dev(bounds.right()) - x0, dev(bounds.bottom()) - y0);
    let (l, r) = (f32::from(left) * s, f32::from(right) * s);
    if w <= 0 || h <= 0 {
        return;
    }
    let sprites = sprites(format!("x_fade {x0} {y0} {w} {h} {l} {r}"), || {
        let alpha = |x: i32| {
            let c = x as f32 + 0.5;
            ((c / l).min((w as f32 - c) / r).min(1.) * 255.).round() as u8
        };
        let row: Vec<Blits> = (0..w).map(|x| [alpha(x), DST_IN, 0, MASK]).collect();
        let blits = row.repeat(h as usize);
        ([x0, y0, w, h], blits)
    });
    paint_sprites(&sprites, Hsla::default(), [0, 0], window, cx);
}

/// Skia's anti-aliased coverage of the rounded rect `bounds` with `radius` in each rounded
/// corner's square of device px: (corner index top-left, top-right, bottom-right, bottom-left,
/// the square `[x0, y0, n, n]`, row-major coverage).
fn corner_coverage(
    radius: Corners<Pixels>,
    bounds: Bounds<Pixels>,
    s: f32,
) -> Vec<(usize, [i32; 4], Vec<u8>)> {
    let dev = |v: Pixels| (f32::from(v) * s).round();
    let (l, t, r, b) = (
        dev(bounds.left()),
        dev(bounds.top()),
        dev(bounds.right()),
        dev(bounds.bottom()),
    );
    let radii = [
        radius.top_left,
        radius.top_right,
        radius.bottom_right,
        radius.bottom_left,
    ]
    .map(|v| f32::from(v) * s);
    let rrect = RRect::new_rect_radii(Rect::new(l, t, r, b), &radii.map(|v| Vector::new(v, v)));
    let mut paint = Paint::new(Color4f::new(1., 1., 1., 1.), None);
    paint.set_anti_alias(true);
    // Each corner's square: its top-left device px.
    [(l, t), (r, t), (r, b), (l, b)]
        .into_iter()
        .zip(radii)
        .zip([(0., 0.), (-1., 0.), (-1., -1.), (0., -1.)])
        .enumerate()
        .filter_map(|(i, (((x, y), rd), (dx, dy)))| {
            let n = rd.ceil() as i32;
            if n == 0 {
                return None;
            }
            let (x0, y0) = ((x + dx * n as f32) as i32, (y + dy * n as f32) as i32);
            let (px, row) = tile_pixels([x0, y0], (n, n), Color4f::new(0., 0., 0., 1.), |c| {
                c.draw_rrect(rrect, &paint);
            });
            let m = px
                .chunks(row)
                .flat_map(|line| line.iter().skip(1).step_by(4).take(n as usize).copied())
                .collect();
            Some((i, [x0, y0, n, n], m))
        })
        .collect()
}

/// `crate::paint_rounded_clip` on macOS: in each rounded corner's square of device px, what is
/// under the layer is drawn onto the layer through Skia's anti-aliased coverage of the rounded
/// rect (the sprites carry 255 minus it); at the `own` corners the layer's pixel stays wherever
/// the clip does not cover the px in full.
pub(crate) fn paint_rounded_clip(
    radius: Corners<Pixels>,
    own: Corners<bool>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    let s = window.scale_factor();
    let key = format!("rounded_clip {radius:?} {own:?} {bounds:?} {s}");
    let sprites = crate::skia::cached(key, || {
        let mut out = Vec::new();
        for (i, rect, m) in corner_coverage(radius, bounds, s) {
            let own = [
                own.top_left,
                own.top_right,
                own.bottom_right,
                own.bottom_left,
            ][i];
            let value = |i: usize| match (own, m[i]) {
                (true, 255) => 0,
                (true, _) => 255,
                (false, m) => 255 - m,
            };
            crate::skia::chunks(&mut out, rect, &value, CLIP, None, false);
        }
        out
    });
    paint_sprites(&sprites, Hsla::default(), [0, 0], window, cx);
}

/// `crate::paint_clipped_tile` on macOS.
pub(crate) fn paint_clipped_tile(
    px: &dyn Fn(i32, i32, u8) -> [u8; 4],
    solid: bool,
    radius: Corners<Pixels>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    let s = window.scale_factor();
    for (_, [x0, y0, w, h], m) in corner_coverage(radius, bounds, s) {
        // The px of partial coverage by their straight 8-bit color: the shader premultiplies
        // it back (`div255(c * a)`) to the tile's px.
        let mut by_color: BTreeMap<[u8; 4], Vec<u8>> = BTreeMap::new();
        for (k, &c) in m.iter().enumerate() {
            let (x, y) = (x0 + k as i32 % w, y0 + k as i32 / w);
            let p = px(x, y, c);
            if c == 0 || c == 255 || p[3] == 0 {
                continue;
            }
            let a = u32::from(p[3]);
            let straight = p.map(|v| {
                (0..=255u32)
                    .find(|c| crate::skia::div255(c * a) == u32::from(v))
                    .unwrap_or(255) as u8
            });
            let plane = by_color
                .entry([straight[0], straight[1], straight[2], p[3]])
                .or_insert_with(|| vec![0; m.len()]);
            plane[k] = c;
        }
        for (color, plane) in by_color {
            let mut hash = std::hash::DefaultHasher::new();
            std::hash::Hash::hash(&plane, &mut hash);
            let key = format!(
                "clipped_tile {x0} {y0} {w} {solid} {color:?} {:016x}",
                std::hash::Hasher::finish(&hash)
            );
            let mode = if solid { MASK } else { ARGB32 };
            let sprites = sprites(key, || {
                let blits = plane.iter().map(|&m| [m, mode, 0, MASK] as Blits);
                ([x0, y0, w, h], blits.collect())
            });
            let [r, g, b, a] = color.map(|v| f32::from(v) / 255.);
            paint_sprites(&sprites, Rgba { r, g, b, a }.into(), [0, 0], window, cx);
        }
    }
}
