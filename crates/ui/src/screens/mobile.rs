//! `mobile` parity screen: `/codex-mobile` (`CodexMobilePage`), initial setup step, from
//! `refs/mobile/*.spec.json`. Sidebar closed, no rail item current.

use crate::{EmptySidebar, Shell, text};
use gpui::{
    AnyView, App, AppContext as _, Context, DevicePixels, Div, InteractiveElement as _,
    IntoElement, ParentElement, Render, RenderImage, SharedString, Size,
    StatefulInteractiveElement as _, Styled, Window, actions, canvas, div, px, rgb, size, svg,
};
use image::{Frame, Rgba, RgbaImage};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, LazyLock},
};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(
    codex_mobile,
    [
        /// `Get started`: moves to the allow-host step.
        GetStarted,
    ]
);

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let sidebar = cx.new(|_| EmptySidebar).into();
    let main = cx.new(|_| MobileSetupPage::default()).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, main);
        shell.sidebar_open = false;
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.can_go_back = true;
        shell
    })
    .into()
}

/// One `codexMobile.setupDialog.initial` feature row: 32 px icon slot, title and description.
/// `(w, h)` is the glyph's box: the SVG viewBox fitted into 20 px.
fn feature(
    glyph: &'static str,
    (w, h): (f32, f32),
    title: &'static str,
    body: &'static str,
    t: &Theme,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .px(px(14.))
        .py(px(8.))
        .child(
            div()
                .size(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(SharedString::from(format!("icons/{glyph}.svg")))
                        .w(px(w))
                        .h(px(h))
                        .text_color(t.text),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(text(title, 14., 21., 430., t.text))
                .child(text(body, 13., 19.5, 430., t.text_tertiary).whitespace_normal()),
        )
}

#[derive(Default)]
pub struct MobileSetupPage {
    /// The composed artwork for the last device size it was drawn at.
    artwork: ArtworkCache,
}

type ArtworkCache = Rc<RefCell<Option<(Size<DevicePixels>, Arc<RenderImage>)>>>;

/// The two artwork PNGs as RGBA; the overlay premultiplied (Skia's N32 premul,
/// `round(c * a / 255)`).
static ARTWORK: LazyLock<(RgbaImage, RgbaImage)> = LazyLock::new(|| {
    let decode = |bytes: &[u8]| {
        image::load_from_memory(bytes)
            .expect("artwork PNG")
            .to_rgba8()
    };
    let background = decode(include_bytes!(
        "../../../../assets/ui/mobile/page-artwork-background.png"
    ));
    let mut overlay = decode(include_bytes!(
        "../../../../assets/ui/mobile/page-artwork-shared.png"
    ));
    for p in overlay.pixels_mut() {
        let a = u32::from(p[3]);
        for c in &mut p.0[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    (background, overlay)
});

/// Fills its box with the artwork exactly as Chrome's software raster paints the two layers:
/// both are resampled on the CPU at device resolution (see [`skia_axis`]) and drawn 1:1, since
/// GPU texture filtering rounds differently.
fn artwork(cache: ArtworkCache) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let scale = window.scale_factor();
            let size = size(
                DevicePixels((f32::from(bounds.size.width) * scale).round() as i32),
                DevicePixels((f32::from(bounds.size.height) * scale).round() as i32),
            );
            let mut cache = cache.borrow_mut();
            match &*cache {
                Some((s, image)) if *s == size => image.clone(),
                _ => {
                    let image = Arc::new(compose(size.width.0 as u32, size.height.0 as u32));
                    cache.insert((size, image)).1.clone()
                }
            }
        },
        |bounds, image, window, _| {
            window
                .paint_image(bounds, bounds, Theme::RADIUS_XL.into(), image, 0, false)
                .ok();
        },
    )
    .size_full()
}

/// `object-fit` / `background-size` rect of a `src` image in a `w` x `h` box, edges snapped
/// half up to device px like Blink: `(x, len)` per axis.
fn fit(w: u32, h: u32, src: &RgbaImage, cover: bool) -> [(f32, f32); 2] {
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let (sx, sy) = (w as f32 / sw, h as f32 / sh);
    let s = if cover { sx.max(sy) } else { sx.min(sy) };
    let axis = |boxed: f32, len: f32| {
        let start = ((boxed - len) / 2. + 0.5).floor();
        (start, ((boxed + len) / 2. + 0.5).floor() - start)
    };
    [axis(w as f32, sw * s), axis(h as f32, sh * s)]
}

/// Skia's legacy bilinear sampling (`SkBitmapProcState`) along one axis for device pixels
/// `from..from + n` (relative to the box): the pixel center goes through the float inverse
/// of the dst-rect mapping, then steps in 32.32 fixed point; each result is the clamped
/// texel pair and a 4-bit weight.
fn skia_axis(
    from: i32,
    n: i32,
    (start, len): (f32, f32),
    src_len: u32,
    step: bool,
) -> Vec<(u32, u32, u32)> {
    let inv = 1. / (len / src_len as f32);
    let offset = -start * inv;
    let fixed =
        |x: i32| ((f64::from((x as f32 + 0.5) * inv + offset) * 4_294_967_296.) as i64) - (1 << 31);
    let (first, dx) = (fixed(from), (f64::from(inv) * 4_294_967_296.) as i64);
    let max = i64::from(src_len) - 1;
    (0..n)
        .map(|i| {
            // Columns step from the span start; rows map each center anew.
            let f = if step {
                first + i64::from(i) * dx
            } else {
                fixed(from + i)
            } >> 16;
            let i0 = f >> 16;
            (
                i0.clamp(0, max) as u32,
                (i0 + 1).clamp(0, max) as u32,
                ((f >> 12) & 0xF) as u32,
            )
        })
        .collect()
}

fn bilerp(
    img: &RgbaImage,
    (x0, x1, qx): (u32, u32, u32),
    (y0, y1, qy): (u32, u32, u32),
) -> [u32; 4] {
    let (a, b, c, d) = (
        img.get_pixel(x0, y0),
        img.get_pixel(x1, y0),
        img.get_pixel(x0, y1),
        img.get_pixel(x1, y1),
    );
    std::array::from_fn(|k| {
        let top = u32::from(a[k]) * (16 - qx) + u32::from(b[k]) * qx;
        let bottom = u32::from(c[k]) * (16 - qx) + u32::from(d[k]) * qx;
        (top * (16 - qy) + bottom * qy) >> 8
    })
}

/// Background (cover) with the premultiplied overlay (contain) composited src-over, as BGRA.
fn compose(w: u32, h: u32) -> RenderImage {
    let (background, overlay) = &*ARTWORK;
    let span = |(start, len): (f32, f32)| (start as i32).max(0)..((start + len) as i32);
    let [bx, by] = fit(w, h, background, true);
    let (bcols, brows) = (
        skia_axis(0, w as i32, bx, background.width(), true),
        skia_axis(0, h as i32, by, background.height(), false),
    );
    let [ox, oy] = fit(w, h, overlay, false);
    let (oxs, oys) = (span(ox), span(oy));
    let (ocols, orows) = (
        skia_axis(oxs.start, oxs.len() as i32, ox, overlay.width(), true),
        skia_axis(oys.start, oys.len() as i32, oy, overlay.height(), false),
    );
    let mut out = RgbaImage::new(w, h);
    for (y, row) in out.rows_mut().enumerate() {
        for (x, px) in row.enumerate() {
            let mut c = bilerp(background, bcols[x], brows[y]);
            let (xi, yi) = (x as i32, y as i32);
            if oxs.contains(&xi) && oys.contains(&yi) {
                let s = bilerp(
                    overlay,
                    ocols[(xi - oxs.start) as usize],
                    orows[(yi - oys.start) as usize],
                );
                for k in 0..3 {
                    c[k] = s[k] + (c[k] * (255 - s[3]) + 127) / 255;
                }
            }
            *px = Rgba([c[2] as u8, c[1] as u8, c[0] as u8, 255]);
        }
    }
    RenderImage::new([Frame::new(out)])
}

impl Render for MobileSetupPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        // Primary solid button; dark mode text is `#2d2d2d`, not `--color-text-inverse`.
        let button_fg = if t.appearance == Appearance::Dark {
            rgb(0x2d2d2d).into()
        } else {
            t.text_inverse
        };
        let hover_bg = t.button_primary_solid_hover;
        // `divide-y-[0.5px] divide-border`
        let divider = |row: Div| row.border_b(px(0.5)).border_color(t.border);
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px(px(24.))
                            .py(px(48.))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(640.))
                            .px(px(24.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(24.))
                            .text_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(400.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.))
                                    .child(
                                        text("Connect your phone to this Mac", 28., 35., 400., t.text)
                                            .whitespace_normal(),
                                    )
                                    .child(
                                        text(
                                            "Continue chats from the ChatGPT desktop app on your phone or another device",
                                            16.,
                                            24.,
                                            430.,
                                            t.text_tertiary,
                                        )
                                        .whitespace_normal(),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(400.))
                                    .flex()
                                    .flex_col()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(t.border)
                                    .rounded(px(20.))
                                    .text_left()
                                    .child(divider(feature(
                                        "mobile-phone",
                                        (20., 20.),
                                        "Pick up where you left off",
                                        "Continue any chat or project from the ChatGPT desktop app",
                                        t,
                                    )))
                                    .child(divider(feature(
                                        "mobile-chat",
                                        (20., 20. * 12. / 14.),
                                        "Stay in the loop",
                                        "Get notified when ChatGPT finishes a task or needs your attention",
                                        t,
                                    )))
                                    .child(feature(
                                        "mobile-start",
                                        (20., 20. * 20. / 21.),
                                        "Start something new",
                                        "Just send a message to start a task on your desktop",
                                        t,
                                    )),
                            )
                            .child(
                                div()
                                    .id("get-started")
                                    .w_full()
                                    .max_w(px(400.))
                                    .h(px(36.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .border_1()
                                    .border_color(t.border)
                                    .rounded_full()
                                    .bg(t.button_primary_solid)
                                    .hover(move |s| s.bg(hover_bg))
                                    .on_click(crate::dispatch::<GetStarted>)
                                    .child(text("Get started", 14., 18., 430., button_fg)),
                            )
                            .child(
                                div().w_full().max_w(px(400.)).child(
                                    text(
                                        "ChatGPT can access your desktop—including files, apps, and your browser—to complete tasks you send from your phone. This can introduce security risks. Only connect devices that you own and trust.",
                                        12.,
                                        18.,
                                        430.,
                                        t.text_tertiary,
                                    )
                                    .whitespace_normal(),
                                ),
                            ),
                    )),
            )
            // Artwork panel: `page-artwork-background` (cover) under `page-artwork-shared`
            // (contain), in a 15 px rounded box.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .p(px(12.))
                    .child(artwork(self.artwork.clone())),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn contain_rect_snaps_half_up() {
        let overlay = &super::ARTWORK.1;
        // refs/mobile/*@1440x900: 680x828 CSS box at 2x; Chrome draws the overlay at 163.
        assert_eq!(
            super::fit(1360, 1656, overlay, false),
            [(163., 1035.), (0., 1656.)]
        );
    }
}
