#![cfg(target_os = "macos")]
use gpui::{
    AppContext, ColorSpace, IntoElement, ParentElement, Render, Styled, Window, div,
    linear_color_stop, linear_gradient, px, size,
};
use std::sync::Arc;

struct GradientScene {
    light: bool,
    oklab: bool,
    top: f32,
}
impl Render for GradientScene {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let color = if self.light {
            gpui::white()
        } else {
            gpui::rgb(0x181818).into()
        };
        let end = if self.oklab {
            gpui::transparent_black()
        } else {
            color.opacity(0.)
        };
        div().size_full().relative().bg(color).child(
            div()
                .absolute()
                .left(px(444.25))
                .top(px(self.top))
                .w(px(728.))
                .h(px(32.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(color, 0.),
                    linear_color_stop(end, 1.),
                )
                .color_space(if self.oklab {
                    ColorSpace::Oklab
                } else {
                    ColorSpace::Srgb
                })),
        )
    }
}

#[test]
fn transparent_gradient_preserves_its_background() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let mut differences = [0usize; 4];
    for (index, (light, oklab, top)) in [
        (false, false, 223.),
        (false, true, 247.16),
        (true, false, 223.),
        (true, true, 247.16),
    ]
    .into_iter()
    .enumerate()
    {
        let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
            cx.new(|_| GradientScene { light, oklab, top })
        })?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        if let Some(directory) = std::env::var_os("CGX_GRADIENT_PROOF") {
            let directory = std::path::Path::new(&directory);
            std::fs::create_dir_all(directory)?;
            image.save(directory.join(format!("light-{light}-oklab-{oklab}.png")))?;
        }
        let value = if light { 255 } else { 24 };
        let mut count = 0;
        let first_row = (top * 2.).round() as u32 + 1;
        for y in first_row..first_row + 62 {
            for x in 890..2344 {
                count += usize::from(image.get_pixel(x, y).0 != [value, value, value, 255]);
            }
        }
        differences[index] = count;
        cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    }
    println!("GRADIENT_DIFFERENCES {differences:?}");
    assert_eq!(
        differences, [0; 4],
        "a transparent fade over its own color must not darken or add noise"
    );
    Ok(())
}

struct ComposedGradient;

impl Render for ComposedGradient {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let surface = gpui::rgb(0x181818);
        div().size_full().relative().bg(surface).child(
            div()
                .absolute()
                .left(px(340.))
                .top(px(44.))
                .w(px(936.))
                .h(px(788.))
                .layer()
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .size(px(16.))
                        .bg(gpui::rgb(0x123456)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(104.25))
                        .top(px(179.))
                        .w(px(728.))
                        .h(px(32.))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(surface, 0.),
                            linear_color_stop(gpui::Hsla::from(surface).opacity(0.), 1.),
                        )),
                ),
        )
    }
}

#[test]
fn composed_gradient_matches_the_original_pixel_rounding() -> anyhow::Result<()> {
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        cx.new(|_| ComposedGradient)
    })?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    if let Some(directory) = std::env::var_os("CGX_GRADIENT_PROOF") {
        std::fs::create_dir_all(&directory)?;
        image.save(std::path::Path::new(&directory).join("composed.png"))?;
    }
    let expected = [[24, 25, 24, 25], [24; 4], [23, 24, 23, 24], [24; 4]];
    for (row, values) in expected.iter().enumerate() {
        for (column, &value) in values.iter().enumerate() {
            let (x, y) = (900 + column as u32, 450 + row as u32);
            assert_eq!(
                image.get_pixel(x, y).0,
                [value, value, value, 255],
                "original Usage gradient at {x},{y}"
            );
        }
    }
    assert_eq!(image.get_pixel(682, 90).0, [0x12, 0x34, 0x56, 255]);
    Ok(())
}

struct GradientOpacity(f32);

impl Render for GradientOpacity {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(gpui::black())
            .child(
                div()
                    .size_full()
                    .layer()
                    .child(div().size_full().opacity(self.0).bg(linear_gradient(
                        180.,
                        linear_color_stop(gpui::white(), 0.),
                        linear_color_stop(gpui::white(), 1.),
                    ))),
            )
    }
}

#[test]
fn composed_gradient_applies_element_opacity_once() -> anyhow::Result<()> {
    let mut cx = gpui::HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    for (opacity, expected) in [(0., 0), (0.5, 128), (1., 255)] {
        let window = cx.open_window(size(px(64.), px(64.)), |_, cx| {
            cx.new(|_| GradientOpacity(opacity))
        })?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        assert_eq!(
            image.get_pixel(32, 32).0,
            [expected, expected, expected, 255],
            "gradient opacity {opacity}"
        );
        cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    }
    Ok(())
}
