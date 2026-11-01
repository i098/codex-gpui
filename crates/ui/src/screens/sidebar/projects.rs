use super::*;

/// A project row menu item that acts on the sidebar's own state; `Edit`, `Move up` and
/// `Remove project` go through [`super::ProjectAction`].
#[derive(Clone, Copy, PartialEq)]
pub enum ProjectCommand {
    /// `Pin` (`true`) or `Unpin`: the global state's `pinned-project-ids`.
    Pin(bool),
    MarkRead,
    Archive,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ProjectMenuAction {
    pub project: String,
    pub command: ProjectCommand,
}
