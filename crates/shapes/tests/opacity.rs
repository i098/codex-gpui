#![cfg(target_os = "macos")]
use gpui::{
    AppContext, Bounds, Hsla, IntoElement, ParentElement, Render, Styled, TransformationMatrix,
    Window, div, point, px, size,
};
use std::sync::Arc;

fn mask(window: &mut Window, cx: &gpui::App, x: f32, y: f32, coverage: u8, color: Hsla) {
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><path d='M0 0H1V1H0Z' fill='white' opacity='{}'/></svg>",
        f32::from(coverage) / 255.
    );
    window
        .paint_svg(
            Bounds::new(point(px(x), px(y)), size(px(1.), px(1.))),
            format!("opacity-mask-{coverage}").into(),
            Some(svg.as_bytes()),
            TransformationMatrix::unit(),
            color,
            cx,
        )
        .expect("paint measured opacity coverage");
}
struct OpacityScene;
impl Render for OpacityScene {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .bg(gpui::rgb(0x2a2a2a))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(28.))
                    .h(px(16.))
                    .opacity(0.75)
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            |_, _, window, cx| {
                                for (x, m) in [(4., 46), (8., 150), (12., 233), (16., 14)] {
                                    mask(window, cx, x, 4., m, gpui::white());
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(16.))
                    .w(px(28.))
                    .h(px(16.))
                    .opacity(0.5)
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            |_, _, window, cx| {
                                mask(window, cx, 4., 20., 208, gpui::white());
                                mask(window, cx, 8., 20., 164, gpui::white());
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(32.))
                    .top_0()
                    .w(px(8.))
                    .h(px(16.))
                    .bg(gpui::rgb(0x2d2d2d)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(32.))
                    .top_0()
                    .w(px(8.))
                    .h(px(16.))
                    .opacity(0.4)
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            |_, _, window, cx| {
                                mask(window, cx, 36., 4., 255, gpui::white());
                                mask(window, cx, 36., 4., 255, gpui::rgb(0x2d2d2d).into());
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(44.))
                    .top_0()
                    .w(px(8.))
                    .h(px(16.))
                    .opacity(0.)
                    .child(shapes::decoration(shapes::Decoration {
                        fill: Some(gpui::rgb(0xff0000).into()),
                        ..Default::default()
                    }))
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            |_, _, window, cx| {
                                shapes::paint_icon(
                                    b"<svg viewBox='0 0 1 1'><path d='M0 0H1V1H0Z'/></svg>",
                                    "hidden-opacity-icon",
                                    Bounds::new(point(px(44.), px(4.)), size(px(1.), px(1.))),
                                    false,
                                    gpui::white(),
                                    None,
                                    window,
                                    cx,
                                );
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    |_, _, window, cx| {
                        mask(
                            window,
                            cx,
                            4.,
                            36.,
                            98,
                            Hsla {
                                a: 127. / 255.,
                                ..gpui::white()
                            },
                        );
                    },
                )
                .absolute()
                .left_0()
                .top(px(32.))
                .w(px(28.))
                .h(px(16.)),
            )
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    |_, _, window, cx| mask(window, cx, 56., 4., 255, gpui::white()),
                )
                .absolute()
                .left(px(52.))
                .top_0()
                .w(px(12.))
                .h(px(16.))
                .opacity(0.5),
            )
            .child(
                div()
                    .absolute()
                    .left(px(52.))
                    .top(px(16.))
                    .w(px(12.))
                    .h(px(16.))
                    .opacity(0.5)
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            |_, _, window, cx| mask(window, cx, 56., 20., 255, gpui::white()),
                        )
                        .size_full()
                        .top_layer(),
                    ),
            )
    }
}
#[test]
fn svg_opacity_uses_one_layer_restore() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(64.), px(48.)), |_, cx| cx.new(|_| OpacityScene))?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    if let Some(path) = std::env::var_os("CGX_OPACITY_PROOF") {
        image.save(path)?;
    }
    let points = [
        (9, 9),
        (17, 9),
        (25, 9),
        (33, 9),
        (9, 41),
        (17, 41),
        (73, 9),
        (9, 73),
        (89, 9),
        (113, 9),
        (113, 41),
    ];
    let actual: Vec<_> = points.map(|(x, y)| image.get_pixel(x, y).0).into();
    let expected: Vec<_> = [70, 136, 188, 50, 129, 111, 45, 82, 42, 149, 255]
        .map(|value| [value, value, value, 255])
        .into();
    println!("OPACITY_PIXELS {actual:?}");
    assert_eq!(
        actual, expected,
        "SVG and canvas opacity must restore each layer once; direct color alpha must keep MASK"
    );
    Ok(())
}
