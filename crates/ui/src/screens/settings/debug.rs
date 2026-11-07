//! The Debug page (debug-settings chunk): the GPU Tearing Debug group, session-only switches
//! that turn one compositing effect off to isolate it as a tearing trigger. The original's
//! Chrome debug pages row is Chrome-only, and its opaque web background row has no native
//! equivalent (the app windows are already opaque), so neither is shown.

use super::{SettingsPane, settings_row};
use gpui::{App, Context, Div, Window};
use std::sync::atomic::{AtomicBool, Ordering};
use theme::Theme;

/// `(key, label, description)` in the original's order.
const ROWS: [(&str, &str, &str); 4] = [
    (
        "session.disableScrollFadeMask",
        "Disable scroll fade mask",
        "Removes scroll-edge fade masks entirely to isolate mask compositing as a tearing trigger",
    ),
    (
        "session.disableBackdropBlur",
        "Disable backdrop blur",
        "Forces backdrop filters off across the app to reduce layered blur composition",
    ),
    (
        "session.disableMotion",
        "Disable motion",
        "Turns off animations and transitions to isolate compositor animation work",
    ),
    (
        "session.disableSquircles",
        "Disable squircles",
        "Uses standard round corners instead of superellipses to isolate squircle rendering cost",
    ),
];

/// The nav row (`ga`): the settings nav asks `allowDebugMenu("prod")`, which is false, or the
/// `2423536643` Statsig gate. Here a build started with `CODEX_GPUI_DEBUG_MENU=1` also shows it.
pub fn allowed(evaluations: &serde_json::Value) -> bool {
    std::env::var_os("CODEX_GPUI_DEBUG_MENU").is_some_and(|v| v == "1")
        || evaluations["feature_gates"]["2423536643"]["value"] == true
}

/// Applies a Debug switch the page just stored; `reduce_motion` is the Appearance page's
/// own reduced-motion choice, which turning `Disable motion` off restores.
pub(super) fn apply(path: &str, on: bool, reduce_motion: bool, cx: &mut App) {
    let switch: &AtomicBool = match path {
        "session.disableScrollFadeMask" => &shapes::debug::NO_SCROLL_FADE,
        "session.disableBackdropBlur" => &shapes::debug::NO_BACKDROP_BLUR,
        "session.disableSquircles" => &shapes::debug::NO_SQUIRCLES,
        "session.disableMotion" => {
            cx.set_reduce_motion(on || reduce_motion);
            cx.refresh_windows();
            return;
        }
        _ => return,
    };
    switch.store(on, Ordering::Relaxed);
    cx.refresh_windows();
}

impl SettingsPane {
    pub(super) fn debug_page(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let rows = ROWS
            .iter()
            .map(|&(key, label, detail)| {
                settings_row(
                    label,
                    Some(detail),
                    self.toggle(key, false, t, window, cx),
                    t,
                )
            })
            .collect();
        vec![("GPU Tearing Debug", rows)]
    }
}
