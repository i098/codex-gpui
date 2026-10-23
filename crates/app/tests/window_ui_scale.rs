#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use common::Fx;

#[test]
fn window_scaling_preserves_input_geometry_and_device_pixel_metrics() -> Result<()> {
    let mut fx = Fx::new()?;
    let bounds = fx
        .cx
        .update_window(fx.window, |_, window, _| window.window_bounds())?;
    for scale in [1.1, 0.9, 1.0] {
        fx.cx.update_window(fx.window, |_, window, cx| {
            assert!(window.set_ui_scale(scale));
            assert_eq!(window.window_bounds(), bounds);
            assert_eq!(window.ui_scale(), scale);
            let line = window.text_system().shape_line(
                "Zoom".into(),
                gpui::px(28.),
                &[window.text_style().to_run(4)],
                None,
            );
            for metric in [line.ascent, line.descent] {
                let device_pixels = f32::from(metric) * window.scale_factor();
                assert!((device_pixels - device_pixels.round()).abs() < 0.0001);
            }
            window.bounds_changed(cx);
            assert_eq!(window.ui_scale(), scale);
            assert!(!window.set_ui_scale(f32::NAN));
            assert!(!window.set_ui_scale(0.));
            assert_eq!(window.ui_scale(), scale);
        })?;
    }
    fx.cx.update_window(fx.window, |_, window, _| {
        window.set_ui_scale(1.1);
    })?;
    fx.click((600., 735.))?;
    fx.type_text("Scaled input")?;
    assert_eq!(fx.composer_text()?, "Scaled input");
    Ok(())
}

#[test]
fn scroll_limits_ignore_paint_rounding_and_clamp_at_both_ends() -> Result<()> {
    use gpui::{
        Context, InteractiveElement, IntoElement, ParentElement, Render, ScrollDelta, ScrollHandle,
        StatefulInteractiveElement, Styled, TouchPhase, Window, div, point, prelude::FluentBuilder,
        px,
    };
    use std::time::Duration;

    struct ScrollBox {
        client: (f32, f32),
        content: (f32, f32),
        origin: f32,
        border: bool,
        padding: f32,
        scroll: ScrollHandle,
    }

    impl Render for ScrollBox {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .id("scroll-boundary")
                    .absolute()
                    .left(px(self.origin))
                    .top(px(self.origin))
                    .w(px(self.client.0))
                    .h(px(self.client.1))
                    .when(self.border, |d| d.border_1())
                    .p(px(self.padding))
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .w(px(self.content.0))
                            .h(px(self.content.1))
                            .flex_none(),
                    ),
            )
        }
    }

    let mut fx = Fx::new()?;
    for scale in [1., 2.] {
        for origin in [0.1, 0.3] {
            for (client, content, border, padding, expected) in [
                ((100.25, 565.203125), (125.6, 800.), false, 0., (26., 235.)),
                ((100.75, 50.75), (100.1, 50.2), false, 0., (0., 0.)),
                ((100.25, 50.49), (101.49, 50.5), false, 0., (1., 1.)),
                ((100.25, 50.5), (101.5, 51.5), false, 0., (2., 1.)),
                ((100., 50.), (100., 50.), true, 2., (6., 6.)),
            ] {
                let scroll = ScrollHandle::new();
                fx.cx.update_window(fx.window, |_, window, cx| {
                    window.set_ui_scale(scale);
                    window.replace_root(cx, |_, _| ScrollBox {
                        client,
                        content,
                        origin,
                        border,
                        padding,
                        scroll: scroll.clone(),
                    });
                })?;
                fx.advance(Duration::ZERO)?;
                let expected = point(px(expected.0), px(expected.1));
                assert_eq!(
                    scroll.max_offset(),
                    expected,
                    "scale={scale}, origin={origin}, client={client:?}, content={content:?}"
                );
                for delta in [point(px(-10000.), px(0.)), point(px(0.), px(-10000.))] {
                    fx.wheel(
                        (origin + 10., origin + 10.),
                        ScrollDelta::Pixels(delta),
                        TouchPhase::Started,
                    )?;
                    fx.advance(Duration::ZERO)?;
                }
                assert_eq!(scroll.offset(), -expected);
                for delta in [point(px(10000.), px(0.)), point(px(0.), px(10000.))] {
                    fx.wheel(
                        (origin + 10., origin + 10.),
                        ScrollDelta::Pixels(delta),
                        TouchPhase::Started,
                    )?;
                    fx.advance(Duration::ZERO)?;
                }
                assert_eq!(scroll.offset(), point(px(0.), px(0.)));
            }
        }
    }
    Ok(())
}
