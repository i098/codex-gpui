use super::*;

/// A project row menu item (`Tbs`); the app runs it against the host's project store.
#[derive(Clone, Copy, PartialEq)]
pub enum ProjectCommand {
    /// `Pin` (`true`) or `Unpin`: the `pinned-project-ids` atom.
    Pin(bool),
    Edit,
    MarkRead,
    Archive,
    Remove,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ProjectAction {
    pub project: String,
    pub command: ProjectCommand,
}
