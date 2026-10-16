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

/// Called by [`crate::init`], after the app's own bindings.
pub(crate) fn init(_cx: &mut App) {}

/// Called by `Root::render` on the root element, after the app's own actions.
pub(crate) fn actions(root: Div, _cx: &mut Context<Root>) -> Div {
    root
}
