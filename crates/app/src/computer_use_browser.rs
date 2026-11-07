//! Browser-family rows for Computer use settings (`computer-use/:browserFamily`).
//! The original's `chromiumBrowser` host service reports whether the Codex extension is installed
//! in a browser profile, and `browser-family-enabled:<family>` (default on) shows or hides the
//! `@browser` reference in the composer. Extension ids and store URLs are the bundle's prod channel.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use ui::screens::computer_use::{BrowserControl, BrowserFamily};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub const CURRENT: Option<Self> = if cfg!(target_os = "macos") {
        Some(Self::Mac)
    } else if cfg!(target_os = "windows") {
        Some(Self::Windows)
    } else if cfg!(target_os = "linux") {
        Some(Self::Linux)
    } else {
        None
    };

    /// Families the original lists by default: Edge only on Windows.
    fn families(self) -> &'static [BrowserFamily] {
        match self {
            Self::Windows => &[BrowserFamily::Chrome, BrowserFamily::Edge],
            Self::Mac | Self::Linux => &[BrowserFamily::Chrome],
        }
    }
}

struct Extension {
    id: &'static str,
    store_url: &'static str,
}

fn extension(family: BrowserFamily) -> Extension {
    match family {
        BrowserFamily::Chrome => Extension {
            id: "hehggadaopoacecdllhhajmbjkdcmajg",
            store_url: "https://chromewebstore.google.com/detail/chatgpt/hehggadaopoacecdllhhajmbjkdcmajg",
        },
        BrowserFamily::Edge => Extension {
            id: "odlomjlbamekndcpllcnffbgeohgkmjh",
            store_url: "https://microsoftedge.microsoft.com/addons/detail/odlomjlbamekndcpllcnffbgeohgkmjh",
        },
    }
}

/// Where the family keeps its profiles. `local` is `%LOCALAPPDATA%`, `config` the XDG config home.
fn user_data_dirs(
    family: BrowserFamily,
    platform: Platform,
    home: &Path,
    local: Option<&Path>,
    config: Option<&Path>,
) -> Vec<PathBuf> {
    let (mac, windows, linux): (&[&str], &[&str], &str) = match family {
        BrowserFamily::Chrome => (
            &["Google", "Chrome"],
            &["Google", "Chrome", "User Data"],
            "google-chrome",
        ),
        BrowserFamily::Edge => (
            &["Microsoft Edge"],
            &["Microsoft", "Edge", "User Data"],
            "microsoft-edge",
        ),
    };
    match platform {
        Platform::Mac => vec![
            mac.iter()
                .fold(home.join("Library/Application Support"), |p, s| p.join(s)),
        ],
        Platform::Windows => [
            local.map_or_else(|| home.join("AppData/Local"), Path::to_path_buf),
            home.join("AppData/Roaming"),
        ]
        .into_iter()
        .map(|base| windows.iter().fold(base, |p, s| p.join(s)))
        .collect(),
        Platform::Linux => vec![
            config
                .map_or_else(|| home.join(".config"), Path::to_path_buf)
                .join(linux),
        ],
    }
}

/// Whether `profile` (a user-data dir or one of its profile dirs) records `id` as installed.
/// Mirrors the original: a preferences entry with a live path wins; a profile whose preferences
/// cannot be read falls back to an `Extensions/<id>` directory.
fn profile_has_extension(profile: &Path, id: &str) -> bool {
    let mut readable = false;
    for name in ["Secure Preferences", "Preferences"] {
        let Ok(text) = std::fs::read(profile.join(name)) else {
            continue;
        };
        let Ok(prefs) = serde_json::from_slice::<Value>(&text) else {
            continue;
        };
        readable = true;
        for group in ["settings", "opsettings"] {
            let Some(entry) = prefs["extensions"][group].get(id).filter(|e| e.is_object()) else {
                continue;
            };
            match entry["path"].as_str() {
                None => {
                    if profile.join("Extensions").join(id).exists() {
                        return true;
                    }
                }
                Some(path) => {
                    if [profile.to_path_buf(), profile.join("Extensions")]
                        .iter()
                        .any(|base| base.join(path).is_dir())
                    {
                        return true;
                    }
                }
            }
        }
    }
    !readable && profile.join("Extensions").join(id).exists()
}

fn extension_installed(dirs: &[PathBuf], id: &str) -> bool {
    dirs.iter().filter(|dir| dir.exists()).any(|dir| {
        std::iter::once(dir.clone())
            .chain(
                std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|entry| entry.path().is_dir())
                    .map(|entry| entry.path()),
            )
            .any(|profile| profile_has_extension(&profile, id))
    })
}

/// `settings` is the host `settings.readAll` map; the toggle defaults to shown.
pub fn controls(
    platform: Platform,
    home: &Path,
    local: Option<&Path>,
    config: Option<&Path>,
    settings: &Map<String, Value>,
) -> Vec<BrowserControl> {
    platform
        .families()
        .iter()
        .map(|&family| {
            let ext = extension(family);
            let dirs = user_data_dirs(family, platform, home, local, config);
            let installed = extension_installed(&dirs, ext.id);
            BrowserControl {
                family,
                installed,
                install_url: Some(ext.store_url.to_owned()),
                extension_id: installed.then(|| ext.id.to_owned()),
                enabled: settings.get(family.setting_key()) != Some(&Value::Bool(false)),
            }
        })
        .collect()
}

/// Reads the current host's rows; empty on an unsupported platform.
pub fn current_controls(settings: &Map<String, Value>) -> Vec<BrowserControl> {
    let (Some(platform), Some(home)) = (Platform::CURRENT, std::env::var_os("HOME")) else {
        return Vec::new();
    };
    let path = |name| std::env::var_os(name).map(PathBuf::from);
    let (local, config) = (path("LOCALAPPDATA"), path("XDG_CONFIG_HOME"));
    controls(
        platform,
        Path::new(&home),
        local.as_deref(),
        config.as_deref(),
        settings,
    )
}

/// Opens `url` in `family`'s browser, as the original's `chromiumBrowser.openUrl` does.
pub fn open_url(family: BrowserFamily, url: &str) -> anyhow::Result<()> {
    use anyhow::{Context as _, ensure};
    let mut command = match Platform::CURRENT {
        Some(Platform::Mac) => {
            let mut c = std::process::Command::new("/usr/bin/open");
            c.args(["-b", family.mac_bundle_id()]);
            c
        }
        Some(Platform::Windows) => {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", "", family.windows_command()]);
            c
        }
        Some(Platform::Linux) => std::process::Command::new(family.linux_command()),
        None => {
            anyhow::bail!("Opening browser URLs is only supported on macOS, Windows, and Linux")
        }
    };
    let missing = || format!("{} is not installed", family.title());
    if Platform::CURRENT == Some(Platform::Mac) {
        // `open` returns at once; Linux and Windows launchers can outlive the call, so only spawn.
        let status = command.arg(url).status().with_context(missing)?;
        ensure!(
            status.success(),
            "{} could not open the page",
            family.title()
        );
    } else {
        command.arg(url).spawn().with_context(missing)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    const ID: &str = "hehggadaopoacecdllhhajmbjkdcmajg";

    fn mac_profile(home: &Path, name: &str) -> PathBuf {
        let dir = home
            .join("Library/Application Support/Google/Chrome")
            .join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn chrome(home: &Path, settings: &Map<String, Value>) -> BrowserControl {
        controls(Platform::Mac, home, None, None, settings).remove(0)
    }

    #[test]
    fn extension_in_any_profile_marks_installed() {
        let home = tempfile::tempdir().unwrap();
        mac_profile(home.path(), "Default");
        assert!(!chrome(home.path(), &Map::new()).installed);
        let profile = mac_profile(home.path(), "Profile 2");
        let prefs = json!({"extensions": {"settings": {ID: {"path": ID}}}});
        fs::write(profile.join("Preferences"), prefs.to_string()).unwrap();
        fs::create_dir_all(profile.join("Extensions").join(ID)).unwrap();
        let row = chrome(home.path(), &Map::new());
        assert!(row.installed);
        assert_eq!(row.extension_id.as_deref(), Some(ID));
    }

    #[test]
    fn uninstalled_extension_entry_with_missing_path_is_not_installed() {
        let home = tempfile::tempdir().unwrap();
        let profile = mac_profile(home.path(), "Default");
        let prefs = json!({"extensions": {"settings": {ID: {"path": "gone"}}}});
        fs::write(profile.join("Preferences"), prefs.to_string()).unwrap();
        assert!(!chrome(home.path(), &Map::new()).installed);
    }

    #[test]
    fn unreadable_preferences_fall_back_to_the_extensions_directory() {
        let home = tempfile::tempdir().unwrap();
        let profile = mac_profile(home.path(), "Default");
        fs::create_dir_all(profile.join("Extensions").join(ID)).unwrap();
        assert!(chrome(home.path(), &Map::new()).installed);
    }

    #[test]
    fn composer_toggle_defaults_on_and_only_false_hides() {
        let home = tempfile::tempdir().unwrap();
        assert!(chrome(home.path(), &Map::new()).enabled);
        let hidden = json!({"browser-family-enabled:chrome": false});
        assert!(!chrome(home.path(), hidden.as_object().unwrap()).enabled);
    }

    #[test]
    fn edge_is_listed_only_on_windows() {
        let home = tempfile::tempdir().unwrap();
        let families = |platform| -> Vec<_> {
            controls(platform, home.path(), None, None, &Map::new())
                .iter()
                .map(|row| row.family)
                .collect()
        };
        assert_eq!(families(Platform::Linux), [BrowserFamily::Chrome]);
        assert_eq!(
            families(Platform::Windows),
            [BrowserFamily::Chrome, BrowserFamily::Edge]
        );
    }
}
