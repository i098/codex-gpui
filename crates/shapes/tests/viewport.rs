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
