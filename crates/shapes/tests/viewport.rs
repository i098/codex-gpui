#![cfg(target_os = "macos")]

use gpui::{
    AppContext, Bounds, Corners, IntoElement, ParentElement, Render, Styled, Window, canvas, div,
    point, px, size,
};
use std::sync::Arc;

struct LongCard {
    height: f32,
    top: f32,
}

impl Render for LongCard {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let top = self.top;
        div().size_full().bg(gpui::rgb(0x181818)).child(
            div()
                .absolute()
                .left(px(340.5))
                .top(px(top))
                .w(px(935.5))
                .h(px(self.height - top - 4.))
                .layer()
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, (), window, cx| {
                            shapes::paint(
                                &shapes::Decoration {
                                    radius: Corners::all(px(20.)),
                                    corner: shapes::Corner::Squircle,
                                    fill: Some(gpui::rgb(0x232323).into()),
                                    border: Some((px(1.), gpui::white().opacity(21. / 255.))),
                                    ..Default::default()
                                },
                                None,
                                Bounds::new(
                                    point(px(444.25), px(122.59 + top)),
                                    size(px(728.), px(1_479_042.5)),
                                ),
                                window,
                                cx,
                            );
                        },
                    )
                    .absolute()
                    .size_full(),
                ),
        )
    }
}

#[test]
fn large_composited_card_keeps_visible_pixels() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    for (height, top) in [(832., 90.), (896., 90.), (832., -240.)] {
        let window = cx.open_window(size(px(1280.), px(height)), |_, cx| {
            cx.new(|_| LongCard { height, top })
        })?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        if let Some(path) = std::env::var_os("CGX_VIEWPORT_PROOF") {
            image.save(
                std::path::Path::new(&path).with_file_name(format!("viewport-{height}-{top}.png")),
            )?;
        }
        if top > 0. {
            assert_eq!(image.get_pixel(911, 426).0, [42, 42, 42, 255]);
            assert_eq!(image.get_pixel(904, 429).0, [53, 53, 53, 255]);
        }
        assert_eq!(image.get_pixel(1000, 500).0, [35, 35, 35, 255]);
        assert_eq!(
            image.get_pixel(1000, (height * 2. - 10.) as u32).0,
            [35, 35, 35, 255]
        );
        cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    }
    Ok(())
}

struct DeferredCard {
    deferred: bool,
}

impl Render for DeferredCard {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let shadow = |color, y, blur, spread| shapes::Shadow {
            color,
            offset: point(px(0.), px(y)),
            blur: px(blur),
            spread: px(spread),
            inset: false,
        };
        let card = div()
            .absolute()
            .left(px(59.))
            .top(px(124.414_06))
            .w(px(549.))
            .h(px(120.75))
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(12.5)),
                corner: shapes::Corner::Squircle,
                fill: Some(gpui::white().opacity(0.03)),
                shadows: vec![
                    shadow(gpui::white().opacity(0.157), 0., 0., 0.5),
                    shadow(gpui::black().opacity(0.04), 3., 7.5, 0.),
                    shadow(gpui::black().opacity(0.05), 0., 20., 0.),
                ],
                ..Default::default()
            }));
        let card = if self.deferred {
            gpui::deferred(card).clipped().into_any_element()
        } else {
            card.into_any_element()
        };
        div().size_full().bg(gpui::rgb(0x202020)).child(
            div()
                .absolute()
                .left(px(661.5))
                .top(px(132.))
                .w(px(700.))
                .h(px(340.))
                .layer()
                .child(shapes::decoration(shapes::Decoration {
                    fill: Some(gpui::rgb(0x202020).into()),
                    ..Default::default()
                }))
                .child(card),
        )
    }
}

#[test]
fn clipped_deferral_preserves_layer_raster_pixels() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let mut capture = |deferred| -> anyhow::Result<image::RgbaImage> {
        let window = cx.open_window(size(px(1440.), px(832.)), |_, cx| {
            cx.new(|_| DeferredCard { deferred })
        })?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        cx.update_window(window.into(), |_, window, _| window.remove_window())?;
        Ok(image)
    };
    let direct = capture(false)?;
    let deferred = capture(true)?;
    let different = direct
        .pixels()
        .zip(deferred.pixels())
        .filter(|(left, right)| left != right)
        .count();
    assert_eq!(different, 0, "deferral changed the layer's raster pixels");
    Ok(())
}
