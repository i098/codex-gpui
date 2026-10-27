//! GPUI text system that lands glyphs on the same pixels as the Codex desktop app's
//! Chromium renderer. The real implementation is macOS-only (`mac.rs`); other platforms
//! (CI builds) get GPUI's no-op text system. textsys cannot depend on gpui_platform: the
//! vendored gpui_apple depends on textsys.

use gpui::{FontFeatures, PlatformTextSystem};
use std::sync::Arc;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{SkiaMatchedTextSystem, show_definition};

#[cfg(target_os = "macos")]
pub fn text_system() -> Arc<dyn PlatformTextSystem> {
    Arc::new(SkiaMatchedTextSystem::default())
}

#[cfg(not(target_os = "macos"))]
pub fn text_system() -> Arc<dyn PlatformTextSystem> {
    Arc::new(gpui::NoopTextSystem::new())
}

/// Private `FontFeatures` tag carrying CSS `letter-spacing` as f32 bits; textsys
/// applies it while shaping and never passes it to HarfBuzz.
const LETTER_SPACING_TAG: &str = "ltsp";

/// CSS `letter-spacing: <px>` for a text element: `.font_features(textsys::letter_spacing(-0.45))`.
/// Added after every character like Blink (kerning kept, optional ligatures off).
pub fn letter_spacing(px: f32) -> FontFeatures {
    FontFeatures(Arc::new(vec![(LETTER_SPACING_TAG.into(), px.to_bits())]))
}

/// Private `FontFeatures` tag carrying the CSS px per layout px of a font size (f32 bits);
/// textsys reads it per face and never passes it to HarfBuzz.
const OUTSIDE_ZOOM_TAG: &str = "otzm";

/// Text outside the window zoom (the original's fixed zoom banner) at `ui_scale`: its layout
/// font size is its CSS size / `ui_scale`, and `opsz` and AAT `trak` follow the CSS size.
/// Combine it with other features as `features.extend(outside_zoom(s).0.iter().cloned())`.
pub fn outside_zoom(ui_scale: f32) -> FontFeatures {
    FontFeatures(Arc::new(vec![(
        OUTSIDE_ZOOM_TAG.into(),
        ui_scale.to_bits(),
    )]))
}
