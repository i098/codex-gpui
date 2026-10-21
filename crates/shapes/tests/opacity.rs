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

struct SavedDeviceScene;

impl Render for SavedDeviceScene {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::rgb(0x2d2d2d)).child(
            gpui::canvas(
                |_, _, _| (),
                |_, _, window, cx| {
                    let bounds = Bounds::new(point(px(987.), px(179.99)), size(px(16.), px(16.)));
                    let source = include_bytes!("../../../assets/icons/diff-load-full-files.svg");
                    let device = shapes::icon_paint_bounds(
                        source,
                        "saved-device-icon",
                        bounds,
                        false,
                        window,
                    )
                    .expect("painted SVG bounds");
                    shapes::with_layer(191, device, window, |window| {
                        assert!(shapes::paint_icon(
                            source,
                            "saved-device-icon",
                            bounds,
                            false,
                            gpui::white(),
                            Some(bounds.origin),
                            window,
                            cx,
                        ));
                    });
                },
            )
            .size_full(),
        )
    }
}

#[test]
fn svg_opacity_respects_saved_device_edges() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        cx.new(|_| SavedDeviceScene)
    })?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    if let Some(path) = std::env::var_os("CGX_SAVED_DEVICE_PROOF") {
        image.save(path)?;
    }
    let points = [
        (1977, 381),
        (1980, 381),
        (1984, 381),
        (1986, 381),
        (1987, 381),
        (1977, 382),
        (1980, 382),
        (1986, 382),
        (1987, 382),
    ];
    let actual = points.map(|(x, y)| image.get_pixel(x, y).0);
    let expected =
        [85, 106, 162, 183, 46, 84, 202, 184, 46].map(|value| [value, value, value, 255]);
    assert_eq!(
        actual, expected,
        "the saved device must end at its painted bounds"
    );
    Ok(())
}

struct NestedSavedDeviceScene;

impl Render for NestedSavedDeviceScene {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::black()).child(
            gpui::canvas(
                |_, _, _| (),
                |_, _, window, cx| {
                    let parent = Bounds::new(point(px(10.), px(10.)), size(px(20.), px(20.)));
                    let child = Bounds::new(point(px(15.), px(15.)), size(px(25.), px(25.)));
                    shapes::with_layer(255, parent, window, |window| {
                        shapes::with_layer(255, child, window, |window| {
                            shapes::paint(
                                &shapes::Decoration {
                                    fill: Some(gpui::white()),
                                    ..Default::default()
                                },
                                None,
                                child,
                                window,
                                cx,
                            );
                        });
                    });
                },
            )
            .size_full(),
        )
    }
}

#[test]
fn nested_saved_device_stays_inside_parent() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(50.), px(50.)), |_, cx| {
        cx.new(|_| NestedSavedDeviceScene)
    })?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    assert_eq!(image.get_pixel(45, 45).0, [255, 255, 255, 255]);
    assert_eq!(image.get_pixel(65, 45).0, [0, 0, 0, 255]);
    assert_eq!(image.get_pixel(45, 65).0, [0, 0, 0, 255]);
    Ok(())
}

struct NestedOpacityScene {
    ancestor_layers: usize,
}

impl Render for NestedOpacityScene {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let mut content = gpui::canvas(
            |_, _, _| (),
            |_, _, window, cx| {
                let bounds = Bounds::new(point(px(4.), px(4.)), size(px(1.), px(1.)));
                shapes::with_composited_opacity(128, bounds, window, cx, |window| {
                    shapes::with_layer(191, bounds, window, |window| {
                        mask(window, cx, 4., 4., 255, gpui::white());
                    });
                });
            },
        )
        .size_full()
        .into_any_element();
        for _ in 0..self.ancestor_layers {
            content = div().size_full().layer().child(content).into_any_element();
        }
        div().size_full().bg(gpui::rgb(0x2d2d2d)).child(content)
    }
}

#[test]
fn ancestor_layers_preserve_opacity_before_a_compositor_mask() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    for ancestor_layers in 0..=2 {
        let window = cx.open_window(size(px(16.), px(16.)), |_, cx| {
            cx.new(|_| NestedOpacityScene { ancestor_layers })
        })?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        assert_eq!(
            image.get_pixel(9, 9).0,
            [124, 124, 124, 255],
            "{ancestor_layers} ancestor layers"
        );
        cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    }
    Ok(())
}

struct MovedIcon {
    source_x: f32,
    translation: f32,
    clipped: bool,
}

impl Render for MovedIcon {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let (source_x, translation, clipped) = (self.source_x, self.translation, self.clipped);
        div().size_full().bg(gpui::black()).child(
            gpui::canvas(
                |_, _, _| (),
                move |_, _, window, cx| {
                    let bounds = Bounds::new(point(px(source_x), px(4.)), size(px(16.), px(16.)));
                    let matrix = gpui::Transformation::translate(point(px(translation), px(0.)))
                        .into_matrix(bounds.center(), window.scale_factor());
                    let paint = |window: &mut Window| {
                        assert!(shapes::paint_icon_transformed(
                            b"<svg viewBox='0 0 1 1'><path d='M0 0H1V1H0Z'/></svg>",
                            "moved-icon",
                            bounds,
                            false,
                            gpui::white(),
                            None,
                            matrix,
                            window,
                            cx,
                        ));
                    };
                    if clipped {
                        window.with_content_mask(
                            Some(gpui::ContentMask {
                                bounds: Bounds::new(point(px(20.), px(4.)), size(px(16.), px(16.))),
                            }),
                            paint,
                        );
                    } else {
                        paint(window);
                    }
                },
            )
            .size_full(),
        )
    }
}

#[test]
fn transformed_icons_enter_the_window_and_parent_clip() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let mut images = Vec::new();
    for (source_x, translation, clipped) in [
        (-12., 32., false),
        (-32., 52., false),
        (2., 18., true),
        (20., 0., false),
    ] {
        let window = cx.open_window(size(px(64.), px(24.)), |_, cx| {
            cx.new(|_| MovedIcon {
                source_x,
                translation,
                clipped,
            })
        })?;
        cx.run_until_parked();
        images.push(cx.capture_screenshot(window.into())?);
        cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    }
    let expected = images.last().unwrap();
    assert_eq!(expected.get_pixel(40, 8).0, [255, 255, 255, 255]);
    assert_eq!(expected.get_pixel(39, 8).0, [0, 0, 0, 255]);
    assert_eq!(
        expected
            .pixels()
            .filter(|p| p.0 == [255, 255, 255, 255])
            .count(),
        1024
    );
    for actual in &images[..3] {
        assert_eq!(actual, expected);
    }
    Ok(())
}
