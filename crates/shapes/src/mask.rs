//! CSS `mask-image` on a composited layer (macOS): viz multiplies the layer's pixels by the
//! mask layer's alpha (`DstIn`) before it composites the layer, which the shader does to the
//! GPUI layer's pixel with [`DST_IN`] sprites.

use crate::skia::{Blits, DST_IN, MASK, paint_sprites, sprites};
use gpui::{App, Bounds, Hsla, Pixels, Window};

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
