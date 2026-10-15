//! `thread` parity screen: the subtract thread as refs/thread shows it.

#[path = "../thread.rs"]
pub mod view;

use bridge::types::ThreadReadResponse;
use gpui::{AnyView, App, AppContext as _, Entity, Window};
use view::{Clock, ThreadView};

use super::{diff::panel::split_screen, sidebar::home_sidebar};

/// The refs were captured on 2026-10-15 08:00 in America/Los_Angeles (tools/route1).
pub const CAPTURE_CLOCK: Clock = Clock {
    utc_offset: -7 * 3600,
    pinned_now: Some(1_792_080_000),
};

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let thread = subtract_thread_beside_panel(window, cx, false);
    let id = thread.read(cx).id().to_owned();
    split_screen(home_sidebar(cx, Some(&id)), thread, None, cx)
}

/// The recorded subtract thread (second `thread/read` of readback.jsonl, the selected Recents
/// row); `beside_panel` when a side panel narrows it, so its composer starts with the compact
/// footer.
pub fn subtract_thread_beside_panel(
    window: &mut Window,
    cx: &mut App,
    beside_panel: bool,
) -> Entity<ThreadView> {
    let read: ThreadReadResponse = crate::fixture::response("readback.jsonl", "thread/read", 1);
    let composer = super::composer::thread_composer(window, cx);
    composer.update(cx, |c, cx| c.set_compact(beside_panel, cx));
    cx.new(|_| ThreadView::new(read.thread, CAPTURE_CLOCK, composer.into()))
}
