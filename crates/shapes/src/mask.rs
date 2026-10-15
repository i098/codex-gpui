//! CSS `mask-image` on a composited layer (macOS): viz multiplies the layer's pixels by the
//! mask layer's alpha (`DstIn`) before it composites the layer, which the shader does to the
//! GPUI layer's pixel with [`DST_IN`] sprites.

use crate::skia::{Blits, DST_IN, MASK, paint_sprites, sprites, tile_pixels};
use gpui::{App, Bounds, Corners, Hsla, Pixels, Window};
use skia_safe::{Color4f, Paint, RRect, Rect, Vector};

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
    paint_sprites(&sprites, Hsla::default(), window, cx);
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
    paint_sprites(&sprites, Hsla::default(), window, cx);
}

/// `crate::paint_rounded_clip` on macOS: each rounded corner's square of device px gets the
/// layer multiplied by Skia's anti-aliased coverage of the outside of the rounded rect.
pub(crate) fn paint_rounded_clip(
    radius: Corners<Pixels>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    let s = window.scale_factor();
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
    let squares = [(l, t), (r, t), (r, b), (l, b)]
        .into_iter()
        .zip(radii)
        .zip([(0., 0.), (-1., 0.), (-1., -1.), (0., -1.)]);
    for (((x, y), rd), (dx, dy)) in squares {
        let n = rd.ceil() as i32;
        if n == 0 {
            continue;
        }
        let (x0, y0) = ((x + dx * n as f32) as i32, (y + dy * n as f32) as i32);
        let key = format!("rounded_clip {x0} {y0} {n} {l} {t} {r} {b} {rd}");
        let sprites = sprites(key, || {
            let (px, row) = tile_pixels([x0, y0], (n, n), Color4f::new(0., 0., 0., 1.), |c| {
                c.draw_rrect(rrect, &paint);
            });
            let blits = px
                .chunks(row)
                .flat_map(|line| line.iter().skip(1).step_by(4).take(n as usize))
                .map(|&m| [255 - m, DST_IN, 0, MASK] as Blits)
                .collect();
            ([x0, y0, n, n], blits)
        });
        paint_sprites(&sprites, Hsla::default(), window, cx);
    }
}
