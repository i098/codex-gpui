use crate::{Root, Route};
use anyhow::{Context as _, Result};
use gpui::{AnyView, App, Context, Window};
use std::path::{Path, PathBuf};
use ui::screens::settings::subtrees::{self, OpenSubtree};

impl Root {
    pub(crate) fn open_settings_subtree(
        &mut self,
        action: &OpenSubtree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.path == subtrees::LICENSES_PATH {
            self.navigate(Route::SettingsSubtree(action.path.clone()), window, cx);
        } else if action.path == subtrees::GENERAL_PATH {
            self.state.route = Route::Settings;
            self.settings_section = None;
            self.sync(window, cx);
        }
    }

    pub(crate) fn open_settings_parent(
        &mut self,
        action: &subtrees::OpenSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(section) = ui::screens::settings::section(&action.label) {
            self.settings_section = Some(section);
            self.navigate(Route::Settings, window, cx);
        }
    }

    pub(crate) fn settings_subtree(&self, path: &str, cx: &mut App) -> Option<AnyView> {
        match path {
            subtrees::LICENSES_PATH => Some(
                subtrees::licenses(
                    async {
                        let executable =
                            std::env::current_exe().map_err(|error| error.to_string())?;
                        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
                        read_notices(&notice_paths(&executable, &cwd))
                            .map_err(|error| error.to_string())
                    },
                    cx,
                )
                .into(),
            ),
            _ => None,
        }
    }
}

fn notice_paths(executable: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(3);
    if let Some(contents) = executable.parent().and_then(Path::parent) {
        paths.push(contents.join("Resources/THIRD_PARTY_NOTICES.txt"));
    }
    paths.push(cwd.join("assets/THIRD_PARTY_NOTICES.txt"));
    paths.push(cwd.join("electron/assets/THIRD_PARTY_NOTICES.txt"));
    paths
}

fn read_notices(paths: &[PathBuf]) -> Result<Option<String>> {
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(text) => return Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_use_first_existing_file_without_hiding_read_failures() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let executable = directory.path().join("Codex.app/Contents/MacOS/codex-gpui");
        let paths = notice_paths(&executable, directory.path());
        assert_eq!(read_notices(&paths)?, None);
        std::fs::create_dir_all(paths[1].parent().unwrap())?;
        std::fs::write(&paths[1], "Local dependency\nCopyright holder\n")?;
        assert_eq!(
            read_notices(&paths)?.as_deref(),
            Some("Local dependency\nCopyright holder\n")
        );
        std::fs::create_dir_all(paths[0].parent().unwrap())?;
        std::fs::write(&paths[0], "Bundled dependency")?;
        assert_eq!(read_notices(&paths)?.as_deref(), Some("Bundled dependency"));
        std::fs::write(&paths[0], [0xff])?;
        assert!(read_notices(&paths).is_err());
        Ok(())
    }
}
