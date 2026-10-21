#![cfg(target_os = "macos")]

use gpui::px;

#[cfg(all(test, target_os = "macos"))]
#[test]
fn preview_mask_keeps_the_bottom_edge_transparent_on_short_content() -> anyhow::Result<()> {
    use gpui::{
        AppContext as _, Context, HeadlessAppContext, IntoElement, ParentElement, Render, Styled,
        Window, div, rgb, size,
    };
    use std::sync::Arc;
    struct Mask;
    impl Render for Mask {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().bg(rgb(0)).pt(px(16.)).child(
                div()
                    .relative()
                    .w(px(16.))
                    .h(px(32.))
                    .layer()
                    .child(shapes::decoration(shapes::Decoration {
                        fill: Some(rgb(0xffffff).into()),
                        ..Default::default()
                    }))
                    .child(shapes::bottom_fade(px(64.))),
            )
        }
    }
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(64.), px(64.)), |_, cx| cx.new(|_| Mask))?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    let scale = image.width() / 64;
    for (y, expected) in [(16 * scale, 127i16), (32 * scale, 63), (48 * scale - 1, 1)] {
        let pixel = image[(8 * scale, y)];
        for channel in &pixel.0[..3] {
            assert!(
                (i16::from(*channel) - expected).abs() <= 2,
                "row {y}: {pixel:?}"
            );
        }
    }
    assert_eq!(image[(8 * scale, 49 * scale)].0, [0, 0, 0, 255]);
    Ok(())
}
