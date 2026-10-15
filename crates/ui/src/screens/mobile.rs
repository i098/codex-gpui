//! `mobile` parity screen: `/codex-mobile` (`CodexMobilePage`), initial setup step, from
//! `refs/mobile/*.spec.json`. Sidebar closed, no rail item current.

use crate::{EmptySidebar, Shell, icon, text};
use gpui::{
    AnyView, App, AppContext as _, Bounds, Context, Corners, Div, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement, Pixels, Render, RenderImage,
    StatefulInteractiveElement as _, Styled, Window, actions, canvas, div, point, px, rgb, size,
};
use image::{Frame, Rgba, RgbaImage};
use shapes::Corner::Squircle;
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
    let shell = shell(cx);
    shell.update(cx, |shell, _| shell.can_go_back = true);
    shell.into()
}

/// `/codex-mobile`: sidebar closed, no rail item current.
pub fn shell(cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    let main = cx.new(|_| MobileSetupPage::default()).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, main);
        shell.sidebar_open = false;
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell
    })
}

/// One `codexMobile.setupDialog.initial` feature row: 32 px icon slot, title and description.
fn feature(glyph: &'static str, title: &'static str, body: &'static str, t: &Theme) -> Div {
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
                .child(icon::icon(glyph, 20., t.text)),
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
    /// The artwork as last drawn: its box (the tile grid matters) and page color.
    artwork: ArtworkCache,
    /// Pointer over `Get started`.
    hovered: bool,
}

type ArtworkCache = Rc<RefCell<Option<(Bounds<Pixels>, Hsla, Arc<RenderImage>, bool)>>>;

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

/// The `rounded-xl` artwork box: `page-artwork-background` (cover) under `page-artwork-shared`
/// (contain). On macOS Skia rasterizes both inside the anti-aliased clip over `page` like
/// Chrome's software raster, drawn 1:1 since GPU texture filtering rounds differently;
/// elsewhere [`compose`] resamples them on the CPU and GPUI rounds the corners.
fn artwork(cache: ArtworkCache, page: Hsla) -> impl IntoElement {
    let radius = Corners::all(Theme::RADIUS_XL);
    canvas(
        move |bounds, window, _| {
            let mut cache = cache.borrow_mut();
            if let Some((b, p, image, rounded)) = &*cache
                && *b == bounds
                && *p == page
            {
                return (image.clone(), *rounded);
            }
            let scale = window.scale_factor();
            let dev = |v: Pixels| (f32::from(v) * scale).round() as u32;
            let (w, h) = (dev(bounds.size.width), dev(bounds.size.height));
            let (background, overlay) = &*ARTWORK;
            let dst = |src: &RgbaImage, cover: bool| {
                let [(x, iw), (y, ih)] = fit(w, h, src, cover);
                let at = |v: f32| px(v / scale);
                Bounds::new(bounds.origin + point(at(x), at(y)), size(at(iw), at(ih)))
            };
            let skia = shapes::raster_images(
                &[
                    (background, dst(background, true)),
                    (overlay, dst(overlay, false)),
                ],
                bounds,
                radius,
                Squircle,
                page,
                window,
            )
            .and_then(|rgba| RgbaImage::from_raw(w, h, rgba));
            let rounded = skia.is_none();
            let image = Arc::new(match skia {
                Some(mut rgba) => {
                    rgba.pixels_mut().for_each(|p| p.0.swap(0, 2));
                    RenderImage::new([Frame::new(rgba)])
                }
                None => compose(w, h),
            });
            *cache = Some((bounds, page, image.clone(), rounded));
            (image, rounded)
        },
        move |bounds, (image, rounded), window, _| {
            let radius = if rounded { radius } else { Corners::default() };
            window
                .paint_image(bounds, bounds, radius, image, 0, false)
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

/// Background (cover) with the premultiplied overlay (contain) composited src-over, as BGRA:
/// Chrome's software raster on the CPU, without its span restarts (the fallback off macOS).
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
        let button_bg = if self.hovered {
            t.button_primary_solid_hover
        } else {
            t.button_primary_solid
        };
        // `divide-y-[0.5px] divide-border`
        let divider = |row: Div| row.border_b(px(0.5)).border_color(t.border);
        div()
            .size_full()
            .relative()
            .flex()
            // The page's own `bg-token-main-surface-primary`: a second fill of the main
            // surface color, a rect under `<main>`'s 12 px rounded clip.
            .child(shapes::clip_path_fill(
                Corners::all(Theme::RADIUS_XL_BASE),
                t.surface,
            ))
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
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: Corners::all(px(20.)),
                                        corner: Squircle,
                                        border: Some((px(1.), t.border)),
                                        ..Default::default()
                                    }))
                                    .p(px(1.))
                                    .text_left()
                                    .child(divider(feature(
                                        "mobile-phone",
                                        "Pick up where you left off",
                                        "Continue any chat or project from the ChatGPT desktop app",
                                        t,
                                    )))
                                    .child(divider(feature(
                                        "mobile-chat",
                                        "Stay in the loop",
                                        "Get notified when ChatGPT finishes a task or needs your attention",
                                        t,
                                    )))
                                    .child(feature(
                                        "mobile-start",
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
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: Corners::all(px(9999.)),
                                        fill: Some(button_bg),
                                        border: Some((px(1.), t.border)),
                                        ..Default::default()
                                    }))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        this.hovered = *hovered;
                                        cx.notify();
                                    }))
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
                    .child(artwork(self.artwork.clone(), t.surface)),
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
