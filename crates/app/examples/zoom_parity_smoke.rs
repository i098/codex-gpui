#[cfg(target_os = "macos")]
#[path = "../tests/common/mod.rs"]
mod common;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    use anyhow::ensure;
    use std::{fs, path::PathBuf, time::Duration};
    let out = PathBuf::from(std::env::var("ZOOM_OUT")?);
    fs::create_dir_all(&out)?;
    let mut results = Vec::new();
    let mut counts = Vec::new();
    for theme in ["dark", "light"] {
        for (state, item, expected) in [
            ("zoom-in", "Zoom In", 1.1),
            ("zoom-out", "Zoom Out", 0.9),
            ("zoom-reset", "Actual Size", 1.0),
        ] {
            let mut fx = common::Fx::with(theme, (1280., 832.), None, |_| {})?;
            fx.menu("View", item)?;
            fx.hover((1109.95, 30.))?;
            fx.advance(Duration::from_millis(2100))?;
            let scale = fx.cx.update_window(fx.window, |_, window, cx| {
                ui::composer::Composer::set_caret_transparent(cx, true);
                window.refresh();
                window.ui_scale()
            })?;
            ensure!(scale == expected, "wrong zoom for {state}: {scale}");
            let image = parity::capture_frozen(&mut fx.cx, fx.window)?;
            ensure!(image.width() == 2560 && image.height() == 1664, "native window size changed");
            image.save(out.join(format!("{theme}-{state}.png")))?;
            counts.push(parity::diff_ref(&out.join("refs"), &out.join("out"), state, theme, &image)?);
            results.push(serde_json::json!({ "theme": theme, "state": state, "zoom": scale, "nativeSize": [image.width(), image.height()] }));
        }
    }
    fs::write(out.join("counts.json"), serde_json::to_vec_pretty(&counts)?)?;
    fs::write(out.join("result.json"), serde_json::to_vec_pretty(&results)?)?;
    println!("PASS: six native zoom states preserve native window dimensions");
    Ok(())
}
