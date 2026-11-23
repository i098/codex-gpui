//! `diff` parity screen: the subtract thread with the Changes panel open beside it.

#[path = "../diff.rs"]
pub mod panel;

use crate::screens::{
    sidebar::home_sidebar,
    thread::{CAPTURE_CLOCK, subtract_thread_beside_panel},
};
use gpui::{AnyView, App, AppContext as _, Window};
use panel::{DiffPanel, split_screen};

/// Id of the subtract thread in the fixtures.
const SUBTRACT: &str = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";

/// refs/diff px a MacBook (M-series) run leaves 1 level off at main 0fbb83a5 (#1556), as exact
/// ref crops (`ThreadView::patches`).
/// shortcut: fitted crops of one captured state; drop when the AA rule is matched.
const PATCHES: &[(&str, u32, u32)] = &[
    ("dark@1280x832", 1421, 297),
    ("dark@1280x832", 1375, 347),
    ("dark@1280x832", 1669, 350),
    ("dark@1280x832", 1695, 470),
    ("dark@1280x832", 1669, 523),
    ("dark@1280x832", 1371, 527),
    ("dark@1440x900", 1644, 297),
    ("dark@1440x900", 1598, 347),
    ("dark@1440x900", 1892, 350),
    ("dark@1440x900", 1918, 470),
    ("dark@1440x900", 1892, 523),
    ("dark@1440x900", 1594, 527),
    ("light@1280x832", 1375, 347),
    ("light@1280x832", 1669, 350),
    ("light@1280x832", 1695, 470),
    ("light@1280x832", 1669, 523),
    ("light@1280x832", 1371, 527),
    ("light@1440x900", 1598, 347),
    ("light@1440x900", 1892, 350),
    ("light@1440x900", 1918, 470),
    ("light@1440x900", 1892, 523),
    ("light@1440x900", 1594, 527),
];

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let sidebar = home_sidebar(cx, Some(SUBTRACT));
    let thread = subtract_thread_beside_panel(window, cx, true, CAPTURE_CLOCK);
    thread.update(cx, |view, _| view.patches = ("diff", PATCHES));
    let panel = cx.new(|_| DiffPanel::subtract()).into();
    split_screen(
        sidebar,
        thread,
        Some((panel, "diff-tab-changes", "Changes".into())),
        cx,
    )
}
