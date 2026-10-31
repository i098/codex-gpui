//! One module per interaction slice (docs: refs/interactions). A slice registers its key
//! bindings and its [`Root`] actions here, in its own file, so that parallel slices do not
//! edit the same hunks of `lib.rs`:
//!
//! 1. add `mod <slice>;` below;
//! 2. call `<slice>::init(cx)` in [`init`] for key bindings and app-level `cx.on_action`s;
//! 3. call `<slice>::actions(root, cx)` in [`actions`] for `root.on_action(cx.listener(..))`.
//!
//! Page-family actions and activation belong in `crate::routes`, not in interaction slices.
//!
//! Slice modules are children of the crate root, so they reach the private fields of [`Root`].

use crate::Root;
use gpui::{App, Context, Div};

mod command_destinations;
mod copy_directory;
pub(crate) mod feedback;
mod keymap;
pub(crate) mod memories;
pub(crate) mod shortcuts;
mod thread_menu;
mod workspace;
pub(crate) use command_destinations::OpenThreadInNewWindow;

/// Called by [`crate::init`], after the app's own bindings.
pub(crate) fn init(cx: &mut App) {
    shortcuts::init(cx);
}

pub(crate) fn install_shortcuts(cx: &mut App) {
    keymap::install(cx);
}

/// Called by `Root::render` on the root element, after the app's own actions.
pub(crate) fn actions(root: Div, owner: &Root, cx: &mut Context<Root>) -> Div {
    workspace::actions(
        shortcuts::actions(
            copy_directory::actions(
                thread_menu::actions(
                    command_destinations::actions(crate::windows::actions(root, cx), cx),
                    cx,
                ),
                cx,
            ),
            owner,
            cx,
        ),
        cx,
    )
}
