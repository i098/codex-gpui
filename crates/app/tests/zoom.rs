#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use common::Fx;

#[test]
fn zoom_actions_preserve_window_geometry_and_editable_content() -> Result<()> {
    let mut fx = Fx::new()?;
    let bounds = fx
        .cx
        .update_window(fx.window, |_, window, _| window.window_bounds())?;
    for (item, expected) in [
        ("Zoom In", 1.1),
        ("Zoom Out", 1.0),
        ("Zoom Out", 0.9),
        ("Actual Size", 1.0),
    ] {
        fx.menu("View", item)?;
        fx.cx.update_window(fx.window, |_, window, cx| {
            assert_eq!(window.ui_scale(), expected);
            assert_eq!(window.window_bounds(), bounds);
            window.bounds_changed(cx);
            assert_eq!(window.ui_scale(), expected);
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
        })?;
    }
    fx.keys("cmd-=")?;
    fx.cx
        .update_window(fx.window, |_, window, _| assert_eq!(window.ui_scale(), 1.1))?;
    fx.click((600., 735.))?;
    fx.type_text("Zoom keeps input usable")?;
    assert_eq!(fx.composer_text()?, "Zoom keeps input usable");
    fx.keys("cmd-0")?;
    fx.cx
        .update_window(fx.window, |_, window, _| assert_eq!(window.ui_scale(), 1.0))?;
    Ok(())
}
