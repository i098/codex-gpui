//! `diff` parity screen: the subtract thread with the Changes panel open beside it.

#[path = "../diff.rs"]
pub mod panel;

use crate::screens::{sidebar::home_sidebar, thread::subtract_thread_beside_panel};
use gpui::{AnyView, App, AppContext as _, Window};
use panel::{DiffPanel, split_screen};

/// Id of the subtract thread in the fixtures.
const SUBTRACT: &str = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let sidebar = home_sidebar(cx, Some(SUBTRACT));
    let thread = subtract_thread_beside_panel(window, cx, true);
    let panel = cx.new(|_| DiffPanel::subtract()).into();
    split_screen(
        sidebar,
        thread,
        Some((panel, "diff-tab-changes", "Changes".into())),
        cx,
    )
}
