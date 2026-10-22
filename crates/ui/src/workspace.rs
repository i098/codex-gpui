#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceLayout {
    Chat,
    #[default]
    Split,
    Full,
    FullChat,
}

impl WorkspaceLayout {
    pub fn toggle_full(self) -> Self {
        if matches!(self, Self::Full | Self::FullChat) {
            Self::Split
        } else {
            Self::Full
        }
    }

    pub fn step(self, persistent: bool) -> Self {
        match self {
            Self::Full | Self::FullChat | Self::Chat => Self::Split,
            Self::Split if persistent => Self::Full,
            Self::Split => Self::Chat,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_tabs_return_to_split_before_full_view() {
        assert_eq!(WorkspaceLayout::Full.step(false), WorkspaceLayout::Split);
        assert_eq!(WorkspaceLayout::Split.step(false), WorkspaceLayout::Chat);
        assert_eq!(WorkspaceLayout::Chat.step(false), WorkspaceLayout::Split);
        assert_eq!(WorkspaceLayout::Chat.toggle_full(), WorkspaceLayout::Full);
        assert_eq!(WorkspaceLayout::Full.toggle_full(), WorkspaceLayout::Split);
    }

    #[test]
    fn persistent_workspace_never_hides_tabs() {
        assert_eq!(WorkspaceLayout::Full.step(true), WorkspaceLayout::Split);
        assert_eq!(WorkspaceLayout::Split.step(true), WorkspaceLayout::Full);
    }
}
