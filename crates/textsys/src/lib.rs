//! GPUI text system that lands glyphs on the same pixels as the Codex desktop app's
//! Chromium renderer. The real implementation is macOS-only (`mac.rs`); other platforms
//! (CI builds) get GPUI's no-op text system. textsys cannot depend on gpui_platform: the
//! vendored gpui_apple depends on textsys.

use gpui::{FontFeatures, PlatformTextSystem};
use std::sync::Arc;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{SkiaMatchedTextSystem, font_faces, show_definition};

#[cfg(target_os = "macos")]
pub fn text_system() -> Arc<dyn PlatformTextSystem> {
    Arc::new(SkiaMatchedTextSystem::default())
}

#[cfg(not(target_os = "macos"))]
pub fn text_system() -> Arc<dyn PlatformTextSystem> {
    Arc::new(gpui::NoopTextSystem::new())
}

#[derive(Clone, Debug)]
pub struct FontFace {
    pub family: String,
    pub style: String,
    pub full_name: String,
    pub postscript_name: String,
}

#[cfg(not(target_os = "macos"))]
pub fn font_faces(_: &str) -> Vec<FontFace> {
    Vec::new()
}

/// Private `FontFeatures` tag carrying CSS `letter-spacing` (1/100 CSS px, i32 bits); textsys
/// applies it while shaping and never passes it to HarfBuzz.
const LETTER_SPACING_TAG: &str = "ltsp";

/// CSS `letter-spacing: <px>` for a text element: `.font_features(textsys::letter_spacing(-0.45))`.
/// Added after every character like Blink (kerning kept, optional ligatures off).
pub fn letter_spacing(px: f32) -> FontFeatures {
    FontFeatures(Arc::new(vec![(
        LETTER_SPACING_TAG.into(),
        (px * 100.).round() as i32 as u32,
    )]))
}

pub const FONT_SMOOTHING_TAG: &str = "cgsm";

pub fn font_smoothing(enabled: bool) -> FontFeatures {
    if enabled {
        FontFeatures::default()
    } else {
        FontFeatures(Arc::new(vec![(FONT_SMOOTHING_TAG.into(), 1)]))
    }
}
