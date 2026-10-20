//! One module per interaction slice (docs: refs/interactions). A slice registers its key
//! bindings and its [`Root`] actions here, in its own file, so that parallel slices do not
//! edit the same hunks of `lib.rs`:
//!
//! 1. add `mod <slice>;` below;
//! 2. call `<slice>::init(cx)` in [`init`] for key bindings and app-level `cx.on_action`s;
//! 3. call `<slice>::actions(root, cx)` in [`actions`] for `root.on_action(cx.listener(..))`.
//!
//! Slice modules are children of the crate root, so they reach the private fields of [`Root`].

use crate::Root;
use gpui::{App, Context, Div};

pub(super) mod chat_commands;
mod command_destinations;
pub(crate) use command_destinations::OpenThreadInNewWindow;

/// Called by [`crate::init`], after the app's own bindings.
pub(crate) fn init(cx: &mut App) {
    command_destinations::init(cx);
    chat_commands::init(cx);
}

/// Called by `Root::render` on the root element, after the app's own actions.
pub(crate) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    command_destinations::actions(
        chat_commands::actions(crate::windows::actions(root, cx), cx),
        cx,
    )
}
