//! The per-OS user data directory, the native equivalent of Electron's `app.getPath("userData")`.
use std::ffi::OsString;
use std::path::PathBuf;

/// `~/Library/Application Support` on macOS, `%APPDATA%` on Windows, and
/// `$XDG_CONFIG_HOME` (else `~/.config`) elsewhere.
pub fn user_data_root() -> PathBuf {
    root_for(
        cfg!(target_os = "macos"),
        cfg!(windows),
        std::env::var_os("HOME"),
        std::env::var_os("APPDATA"),
        std::env::var_os("XDG_CONFIG_HOME"),
    )
}

/// The directory of one app, such as `codex-gpui`, under [`user_data_root`].
pub fn user_data_dir(app: &str) -> PathBuf {
    user_data_root().join(app)
}

fn root_for(
    macos: bool,
    windows: bool,
    home: Option<OsString>,
    appdata: Option<OsString>,
    xdg: Option<OsString>,
) -> PathBuf {
    let home = PathBuf::from(home.unwrap_or_default());
    if macos {
        return home.join("Library/Application Support");
    }
    if windows && let Some(appdata) = appdata.filter(|value| !value.is_empty()) {
        return appdata.into();
    }
    // XDG requires an absolute path; a relative value is ignored.
    match xdg.map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => home.join(".config"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(macos: bool, windows: bool, appdata: Option<&str>, xdg: Option<&str>) -> PathBuf {
        root_for(
            macos,
            windows,
            Some("/h".into()),
            appdata.map(Into::into),
            xdg.map(Into::into),
        )
    }

    #[cfg(unix)] // POSIX path fixtures
    #[test]
    fn resolves_per_os() {
        assert_eq!(
            root(true, false, None, Some("/x")),
            PathBuf::from("/h/Library/Application Support")
        );
        assert_eq!(root(false, true, Some("C:/A"), None), PathBuf::from("C:/A"));
        assert_eq!(root(false, false, None, Some("/x")), PathBuf::from("/x"));
        assert_eq!(
            root(false, false, None, Some("rel")),
            PathBuf::from("/h/.config")
        );
        assert_eq!(
            root(false, false, Some("C:/A"), None),
            PathBuf::from("/h/.config")
        );
    }
}
