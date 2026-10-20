#![cfg(target_os = "macos")]

use gpui::{
    AppContext, Corners, IntoElement, ParentElement, Render, Styled, Window, canvas, div,
    prelude::FluentBuilder, px, size,
};
use std::sync::Arc;

struct ImageLayers;

impl Render for ImageLayers {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let mut root = div().size_full().relative().bg(gpui::rgb(0x2b2b2b));
        for row in 0..3 {
            let mut panel = div()
                .absolute()
                .left(px(10.))
                .top(px(10. + row as f32 * 40.))
                .w(px(30.))
                .h(px(30.))
                .when(row != 0, |d| d.top_layer());
            if row == 2 {
                panel = panel.child(shapes::decoration(shapes::Decoration {
                    fill: Some(gpui::rgb(0x2b2b2b).into()),
                    ..Default::default()
                }));
            } else {
                let image = Arc::new(gpui::RenderImage::new([image::Frame::new(
                    image::RgbaImage::from_pixel(2, 2, image::Rgba([43, 43, 43, 255])),
                )]));
                panel = panel.child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _| {
                            window
                                .paint_image(bounds, bounds, Corners::default(), image, 0, false)
                                .expect("opaque image");
                        },
                    )
                    .absolute()
                    .size_full(),
                );
            }
            panel = panel.child(shapes::decoration(shapes::Decoration {
                fill: Some(gpui::Hsla {
                    a: 0.1,
                    ..gpui::rgb(0xff6764).into()
                }),
                ..Default::default()
            }));
            root = root.child(panel);
        }
        root
    }
}

#[test]
fn image_background_keeps_the_destination_for_following_masks() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(64.), px(140.)), |_, cx| cx.new(|_| ImageLayers))?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    for y in [40, 120, 200] {
        assert_eq!(image.get_pixel(40, y).0, [64, 49, 48, 255]);
    }
    Ok(())
}
