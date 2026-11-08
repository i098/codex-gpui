use std::{ffi::OsString, path::PathBuf};

fn non_empty(key: &str) -> Option<OsString> {
    std::env::var_os(key).filter(|value| !value.is_empty())
}

pub fn home() -> Option<PathBuf> {
    non_empty("HOME")
        .or_else(|| non_empty("USERPROFILE"))
        .map(PathBuf::from)
}

/// `$CODEX_HOME`, else `~/.codex`.
pub fn codex_home() -> Option<PathBuf> {
    non_empty("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".codex")))
}
