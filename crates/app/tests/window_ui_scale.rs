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
