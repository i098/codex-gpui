#![cfg(any(target_os = "macos", target_os = "ios"))]
//! Shared Apple rendering, task dispatch, and system services for GPUI.
//!
//! This crate renders GPUI scenes directly with Metal on Apple platforms. It
//! owns GPU resources, text rendering, Keychain access, and thermal notifications
//! while leaving application lifecycle, windowing, and input to each platform backend.

mod dispatcher;
pub mod keychain;
mod metal_atlas;
pub mod metal_renderer;
pub mod thermal;


pub use dispatcher::{AppleActivity, AppleDispatcher};
/// Chromium-exact text (crates/textsys) for every GPUI app on this checkout.
#[cfg(feature = "font-kit")]
pub use textsys::SkiaMatchedTextSystem as AppleTextSystem;
