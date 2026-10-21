use super::LocalHost;
#[cfg(target_os = "macos")]
use super::native_apps;
use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use url::Url;

struct Target {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    kind: &'static str,
    mac: bool,
    linux: bool,
}

const TARGETS: &[Target] = &[
    Target {
        id: "vscode",
        label: "VS Code",
        icon: "/apps/vscode.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "vscodeInsiders",
        label: "VS Code Insiders",
        icon: "/apps/vscode-insiders.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "cursor",
        label: "Cursor",
        icon: "/apps/cursor.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "devin",
        label: "Devin Desktop",
        icon: "/apps/devin.svg",
        kind: "editor",
        mac: true,
        linux: false,
    },
    Target {
        id: "bbedit",
        label: "BBEdit",
        icon: "/apps/bbedit.png",
        kind: "editor",
        mac: true,
        linux: false,
    },
    Target {
        id: "emacs",
        label: "Emacs",
        icon: "/apps/emacs.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "sublimeText",
        label: "Sublime Text",
        icon: "/apps/sublime-text.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "zed",
        label: "Zed",
        icon: "/apps/zed.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "windsurf",
        label: "Windsurf",
        icon: "/apps/windsurf.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "antigravity",
        label: "Antigravity",
        icon: "/apps/antigravity.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "systemDefault",
        label: "Default app",
        icon: "/apps/file-explorer.png",
        kind: "systemDefault",
        mac: true,
        linux: true,
    },
    Target {
        id: "fileManager",
        label: "Finder",
        icon: "/apps/finder.png",
        kind: "fileManager",
        mac: true,
        linux: true,
    },
    Target {
        id: "terminal",
        label: "Terminal",
        icon: "/apps/terminal.png",
        kind: "terminal",
        mac: true,
        linux: true,
    },
    Target {
        id: "iterm2",
        label: "iTerm2",
        icon: "/apps/iterm2.png",
        kind: "terminal",
        mac: true,
        linux: false,
    },
    Target {
        id: "kitty",
        label: "Kitty",
        icon: "/apps/kitty.png",
        kind: "terminal",
        mac: true,
        linux: true,
    },
    Target {
        id: "ghostty",
        label: "Ghostty",
        icon: "/apps/ghostty.png",
        kind: "terminal",
        mac: true,
        linux: true,
    },
    Target {
        id: "warp",
        label: "Warp",
        icon: "/apps/warp.png",
        kind: "terminal",
        mac: true,
        linux: false,
    },
    Target {
        id: "xcode",
        label: "Xcode",
        icon: "/apps/xcode.png",
        kind: "editor",
        mac: true,
        linux: false,
    },
    Target {
        id: "androidStudio",
        label: "Android Studio",
        icon: "/apps/android-studio.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "intellij",
        label: "IntelliJ IDEA",
        icon: "/apps/intellij.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "rider",
        label: "Rider",
        icon: "/apps/rider.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "goland",
        label: "GoLand",
        icon: "/apps/goland.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "rustrover",
        label: "RustRover",
        icon: "/apps/rustrover.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "pycharm",
        label: "PyCharm",
        icon: "/apps/pycharm.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "webstorm",
        label: "WebStorm",
        icon: "/apps/webstorm.svg",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "phpstorm",
        label: "PhpStorm",
        icon: "/apps/phpstorm.png",
        kind: "editor",
        mac: true,
        linux: true,
    },
    Target {
        id: "textmate",
        label: "TextMate",
        icon: "/apps/textmate.png",
        kind: "editor",
        mac: true,
        linux: false,
    },
];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Query {
    cwd: Option<String>,
    path: Option<String>,
    defer_enrichment: bool,
    native_browser_discovery: Option<String>,
}

impl Target {
    fn supported(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.mac
        } else if cfg!(target_os = "linux") {
            self.linux
        } else {
            false
        }
    }

    fn metadata(&self) -> Value {
        let mut value = json!({"id": self.id, "target": self.id, "label": self.label, "icon": self.icon, "kind": self.kind});
        if self.id == "systemDefault" {
            value["hidden"] = true.into();
        }
        if cfg!(target_os = "linux") && self.id == "fileManager" {
            value["label"] = "File Manager".into();
            value["icon"] = "/apps/file-explorer.png".into();
        }
        value
    }
}

impl LocalHost {
    pub(super) fn open_in_targets(&self, params: &Value) -> Result<Value> {
        let query = Query::deserialize(params).context("invalid open-in target query")?;
        let settings = self
            .settings
            .as_ref()
            .context("host settings are unavailable")?;
        let custom = if let Some(bridge) = &self.bridge {
            let mut config: Value =
                pollster::block_on(bridge.request("config/read", json!({"includeLayers": false})))?;
            custom_handlers(config["config"]["custom_file_handlers"].take())
        } else {
            Map::new()
        };
        let targets: Vec<_> = TARGETS.iter().filter(|target| target.supported()).collect();
        let mut rows: Vec<_> = targets.iter().map(|target| target.metadata()).collect();
        for (id, handler) in &custom {
            let target = format!("custom:{id}");
            rows.push(json!({"id": target, "target": target,
                "label": handler["label"], "icon": custom_icon(handler["icon"].as_str().unwrap()), "kind": "editor"}));
        }
        let configured = {
            let values = settings.read_all();
            let prefs = values
                .get("open-in-target-preferences")
                .unwrap_or(&Value::Null);
            let known = |value: &Value| {
                let value = value.as_str()?;
                let value = if value == "finder" {
                    "fileManager"
                } else {
                    value
                };
                rows.iter()
                    .any(|row| row["id"] == value)
                    .then(|| value.to_owned())
            };
            query
                .cwd
                .as_ref()
                .and_then(|cwd| known(&prefs["perPath"][cwd]))
                .or_else(|| known(&prefs["global"]))
        };
        let deferred = query.defer_enrichment && query.path.is_none();
        let mut available = Vec::new();
        if !deferred {
            let discovery = Discovery::new();
            for target in &targets {
                if discovery.command(target.id).is_some() {
                    available.push(target.id.to_owned());
                }
            }
            for (id, handler) in &custom {
                if discovery
                    .executable(handler["command"].as_str().unwrap())
                    .is_some()
                {
                    available.push(format!("custom:{id}"));
                }
            }
        }
        let available_set: HashSet<_> = available.iter().map(String::as_str).collect();
        let preferred = if deferred {
            configured
        } else {
            configured
                .filter(|target| available_set.contains(target.as_str()))
                .or_else(|| available.first().cloned())
        };
        for row in &mut rows {
            if !deferred {
                row["available"] = available_set.contains(row["id"].as_str().unwrap()).into();
            }
            if preferred.as_ref().is_some_and(|id| row["id"] == *id) {
                row["default"] = true.into();
            }
        }
        let path = query
            .path
            .as_ref()
            .map(|path| resolve_path(path, query.cwd.as_deref()))
            .transpose()?;
        let native_mode = path.as_ref().is_some_and(|path| native_path(path));
        if let Some(path) = path.as_ref().filter(|_| native_mode) {
            if office_path(path) {
                rows.retain(|row| row["id"] != "systemDefault");
            }
            rows.extend(native_targets(
                path,
                query
                    .native_browser_discovery
                    .as_deref()
                    .is_some_and(|value| value != "scan"),
            )?);
        }
        Ok(
            json!({"preferredTarget": preferred, "availableTargets": available,
            "mode": if native_mode { "native" } else { "editor" }, "targets": rows}),
        )
    }
}

fn custom_handlers(value: Value) -> Map<String, Value> {
    let Value::Object(mut handlers) = value else {
        return Map::new();
    };
    let valid = handlers.values().all(|handler| {
        ["label", "icon", "command"].iter().all(|key| {
            handler[*key]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        }) && handler.get("args").is_none_or(|args| {
            args.as_array()
                .is_some_and(|args| args.iter().all(Value::is_string))
        }) && handler.get("input").is_none_or(|input| {
            matches!(
                input.as_str(),
                Some("path" | "json_argument" | "json_stdin")
            )
        }) && handler.get("supports_ssh").is_none_or(Value::is_boolean)
    });
    if !valid {
        eprintln!("codex-gpui: invalid custom file handlers configuration");
        return Map::new();
    }
    handlers.retain(|id, _| valid_custom_id(id));
    handlers
}

fn valid_custom_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(byte))
}

fn custom_icon(icon: &str) -> String {
    let asset = icon.strip_prefix('/').unwrap_or(icon);
    if asset.starts_with("apps/") && !asset.split('/').any(|part| part == "..") {
        return format!("/{asset}");
    }
    if icon.split_once(',').is_some_and(|(header, _)| {
        let header = header.to_ascii_lowercase();
        matches!(
            header.as_str(),
            "data:image/png;base64"
                | "data:image/jpg;base64"
                | "data:image/jpeg;base64"
                | "data:image/gif;base64"
                | "data:image/webp;base64"
                | "data:image/bmp;base64"
                | "data:image/x-icon;base64"
                | "data:image/vnd.microsoft.icon;base64"
        )
    }) {
        return icon.to_owned();
    }
    let path = if icon.starts_with("file:") {
        Url::parse(icon)
            .ok()
            .and_then(|url| url.to_file_path().ok())
    } else if Path::new(icon).is_absolute() {
        Some(PathBuf::from(icon))
    } else {
        None
    };
    path.and_then(|path| {
        let mut url = Url::parse("app://fs").ok()?;
        url.set_path(&format!("/@fs{}", path.to_str()?));
        Some(url.to_string())
    })
    .unwrap_or_else(|| "/apps/vscode.png".to_owned())
}

fn resolve_path(path: &str, cwd: Option<&str>) -> Result<String> {
    if Url::parse(path).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        return Ok(path.to_owned());
    }
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    let path = if path.starts_with("file:") {
        Url::parse(path)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("invalid file URL"))?
    } else {
        PathBuf::from(path)
    };
    let path = if path.is_absolute() {
        path
    } else {
        let cwd = match cwd {
            Some(cwd) => PathBuf::from(cwd),
            None => std::env::current_dir()?,
        };
        let joined = cwd.join(&path);
        if joined.exists() {
            joined
        } else {
            let name = cwd.file_name();
            let remainder = path
                .components()
                .skip_while(|part| Some(part.as_os_str()) != name)
                .skip(1)
                .collect::<PathBuf>();
            if remainder.as_os_str().is_empty() {
                joined
            } else {
                cwd.join(remainder)
            }
        }
    };
    path.to_str()
        .map(str::to_owned)
        .context("open-in path must be UTF-8")
}

fn office_path(path: &str) -> bool {
    if Url::parse(path).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        return false;
    }
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["doc", "docx", "ppt", "pptx", "xls", "xlsm", "xlsx"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

fn native_path(path: &str) -> bool {
    if Url::parse(path).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        return true;
    }
    let path = Path::new(path);
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm") {
        return true;
    }
    path.is_file()
        && [
            "pdf", "png", "jpg", "jpeg", "gif", "bmp", "tiff", "ico", "webp", "mov", "mp4", "m4v",
            "avi", "mkv", "webm", "mpg", "mpeg", "mp3", "wav", "m4a", "aac", "aiff", "aif", "flac",
            "caf", "zip", "tar", "gz", "tgz", "bz2", "tbz", "xz", "txz", "7z", "rar", "sketch",
            "fig", "psd", "ai", "indd", "xd", "doc", "docx", "xls", "xlsm", "xlsx", "csv", "ppt",
            "pptx", "key", "pages", "numbers",
        ]
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

struct Discovery {
    home: PathBuf,
    paths: Vec<PathBuf>,
    apps: Vec<(String, PathBuf)>,
    desktop_roots: Vec<PathBuf>,
}

impl Discovery {
    fn new() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let apps = if cfg!(target_os = "macos") {
            [PathBuf::from("/Applications"), home.join("Applications")]
                .iter()
                .flat_map(|dir| {
                    fs::read_dir(dir)
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                })
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().to_lowercase(),
                        entry.path(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut desktop_roots = vec![
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share")),
        ];
        desktop_roots.extend(std::env::split_paths(
            &std::env::var_os("XDG_DATA_DIRS")
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
        ));
        Self {
            home,
            paths: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect(),
            apps,
            desktop_roots,
        }
    }

    fn executable(&self, command: &str) -> Option<PathBuf> {
        let command = Path::new(command);
        if command.is_absolute() {
            return command.exists().then(|| command.to_path_buf());
        }
        self.paths
            .iter()
            .map(|dir| dir.join(command))
            .find(|path| executable(path))
    }

    fn app_path(&self, path: &str) -> Option<PathBuf> {
        let system = PathBuf::from(path);
        if system.exists() {
            return Some(system);
        }
        path.strip_prefix("/Applications/")
            .map(|relative| self.home.join("Applications").join(relative))
            .filter(|path| path.exists())
    }

    fn app_prefix(&self, prefix: &str) -> Option<PathBuf> {
        self.apps
            .iter()
            .find(|(name, path)| {
                name.starts_with(prefix) && name.ends_with(".app") && path.exists()
            })
            .map(|(_, path)| path.clone())
    }

    fn app_binary(&self, prefix: &str, relative: &str) -> Option<PathBuf> {
        self.apps
            .iter()
            .filter(|(name, _)| name.starts_with(prefix) && name.ends_with(".app"))
            .map(|(_, path)| path.join(relative))
            .find(|path| path.exists())
    }

    fn cursor(&self) -> Option<PathBuf> {
        let names = ["cursor.app", "cursor preview.app", "cursor nightly.app"];
        let preferred = names
            .into_iter()
            .flat_map(|wanted| self.apps.iter().filter(move |(name, _)| name == wanted));
        preferred
            .chain(
                self.apps
                    .iter()
                    .filter(|(name, _)| name.starts_with("cursor"))
                    .take(1),
            )
            .find_map(|(_, app)| {
                let binary = app.join("Contents/MacOS/Cursor");
                (binary.exists() && app.join("Contents/Resources/app/out/cli.js").exists())
                    .then_some(binary)
            })
    }

    fn desktop(&self, names: &[&str]) -> Option<PathBuf> {
        for name in names {
            for root in &self.desktop_roots {
                let path = root.join("applications").join(name);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn command(&self, id: &str) -> Option<PathBuf> {
        if id == "systemDefault" {
            return Some("system-default".into());
        }
        if cfg!(target_os = "macos") {
            self.mac_command(id)
        } else {
            self.linux_command(id)
        }
    }

    fn mac_command(&self, id: &str) -> Option<PathBuf> {
        let candidates: &[&str] = match id {
            "vscode" => &[
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                "/Applications/Code.app/Contents/Resources/app/bin/code",
            ],
            "vscodeInsiders" => &[
                "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code",
                "/Applications/Code - Insiders.app/Contents/Resources/app/bin/code",
            ],
            "devin" => &["/Applications/Devin.app/Contents/Resources/app/bin/devin-desktop"],
            "windsurf" => &["/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf"],
            "antigravity" => {
                &["/Applications/Antigravity.app/Contents/Resources/app/bin/antigravity"]
            }
            "kitty" => &["/Applications/kitty.app"],
            "ghostty" => &["/Applications/Ghostty.app"],
            "warp" => &["/Applications/Warp.app"],
            "iterm2" => &["/Applications/iTerm.app", "/Applications/iTerm2.app"],
            "terminal" => &["/System/Applications/Utilities/Terminal.app"],
            _ => &[],
        };
        if let Some(path) = candidates.iter().find_map(|path| self.app_path(path)) {
            return Some(
                if matches!(id, "kitty" | "ghostty" | "warp" | "iterm2" | "terminal") {
                    "/usr/bin/open".into()
                } else {
                    path
                },
            );
        }
        match id {
            "fileManager" => Some("/usr/bin/open".into()),
            "bbedit" | "emacs" | "textmate" => self
                .app_prefix(match id {
                    "bbedit" => "bbedit",
                    "emacs" => "emacs",
                    _ => "textmate",
                })
                .map(|_| "/usr/bin/open".into()),
            "cursor" => self.cursor(),
            "sublimeText" => self
                .executable("subl")
                .or_else(|| self.app_binary("sublime text", "Contents/SharedSupport/bin/subl")),
            "zed" => self
                .executable("zed")
                .or_else(|| {
                    [
                        "/Applications/Zed.app/Contents/MacOS/zed",
                        "/Applications/Zed Preview.app/Contents/MacOS/zed",
                        "/Applications/Zed Nightly.app/Contents/MacOS/zed",
                    ]
                    .iter()
                    .find_map(|path| self.app_path(path))
                })
                .or_else(|| self.app_binary("zed", "Contents/MacOS/zed")),
            "xcode" => self.xcode(),
            "androidStudio" | "intellij" | "rider" | "goland" | "rustrover" | "pycharm"
            | "webstorm" | "phpstorm" => self.jetbrains(id),
            _ => None,
        }
    }

    fn linux_command(&self, id: &str) -> Option<PathBuf> {
        let (commands, desktops): (&[&str], &[&str]) = match id {
            "vscode" => (
                &["code", "code-oss"],
                &[
                    "code.desktop",
                    "code-oss.desktop",
                    "com.visualstudio.code.desktop",
                    "com.visualstudio.code-oss.desktop",
                    "code_code.desktop",
                ],
            ),
            "vscodeInsiders" => (
                &["code-insiders"],
                &[
                    "code-insiders.desktop",
                    "com.visualstudio.code.insiders.desktop",
                    "code-insiders_code-insiders.desktop",
                ],
            ),
            "cursor" => (&["cursor"], &["cursor.desktop"]),
            "emacs" => (
                &["emacs", "emacsclient"],
                &[
                    "emacs.desktop",
                    "emacsclient.desktop",
                    "org.gnu.emacs.desktop",
                ],
            ),
            "sublimeText" => (
                &["subl", "sublime_text"],
                &["sublime_text.desktop", "sublime-text.desktop"],
            ),
            "zed" => (
                &["zed"],
                &[
                    "dev.zed.Zed.desktop",
                    "dev.zed.Zed-Preview.desktop",
                    "zed.desktop",
                ],
            ),
            "windsurf" => (&["windsurf"], &["windsurf.desktop"]),
            "antigravity" => (&["antigravity-ide"], &["antigravity-ide.desktop"]),
            "fileManager" => (&["xdg-open"], &[]),
            "terminal" => (
                &[
                    "kgx",
                    "gnome-terminal",
                    "konsole",
                    "xfce4-terminal",
                    "mate-terminal",
                    "tilix",
                    "alacritty",
                    "x-terminal-emulator",
                ],
                &[],
            ),
            "kitty" => (&["kitty"], &[]),
            "ghostty" => (&["ghostty"], &[]),
            _ => (&[], &[]),
        };
        commands
            .iter()
            .find_map(|command| self.executable(command))
            .or_else(|| self.desktop(desktops))
            .or_else(|| {
                let (command, _) = jetbrains_tool(id)?;
                self.executable(command)
                    .or_else(|| self.executable(&format!("{command}.sh")))
                    .or_else(|| {
                        (id == "intellij")
                            .then(|| self.executable("intellij-idea"))
                            .flatten()
                    })
                    .or_else(|| {
                        let path = self
                            .home
                            .join(".local/share/JetBrains/Toolbox/scripts")
                            .join(command);
                        path.exists().then_some(path)
                    })
            })
    }

    fn xcode(&self) -> Option<PathBuf> {
        let app = self.app_prefix("xcode");
        let selected = Command::new("/usr/bin/xcode-select")
            .arg("-p")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok());
        if let Some(directory) = selected {
            let xed = Path::new(directory.trim()).join("usr/bin/xed");
            if xed.exists() {
                return Some(xed);
            }
        }
        app
    }

    fn jetbrains(&self, id: &str) -> Option<PathBuf> {
        let target = TARGETS.iter().find(|target| target.id == id)?;
        let (command, prefix) = jetbrains_tool(id)?;
        let candidate = format!(
            "/Applications/{}.app/Contents/MacOS/{command}",
            target.label
        );
        self.app_path(&candidate)
            .or_else(|| self.app_binary(prefix, &format!("Contents/MacOS/{command}")))
            .or_else(|| {
                let root = self
                    .home
                    .join("Library/Application Support/JetBrains/Toolbox/apps");
                toolbox_apps(&root).into_iter().find_map(|app| {
                    let name = app.file_name()?.to_string_lossy().to_lowercase();
                    let prefixes: &[&str] = match id {
                        "intellij" => &["intellij", "idea"],
                        "androidStudio" => &["android studio", "androidstudio"],
                        _ => &[id],
                    };
                    let binary = app.join("Contents/MacOS").join(command);
                    (prefixes.iter().any(|prefix| name.starts_with(prefix)) && binary.exists())
                        .then_some(binary)
                })
            })
    }
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    path.is_file()
}

fn jetbrains_tool(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "androidStudio" => ("studio", "android studio"),
        "intellij" => ("idea", "intellij idea"),
        "rider" => ("rider", "rider"),
        "goland" => ("goland", "goland"),
        "rustrover" => ("rustrover", "rustrover"),
        "pycharm" => ("pycharm", "pycharm"),
        "webstorm" => ("webstorm", "webstorm"),
        "phpstorm" => ("phpstorm", "phpstorm"),
        _ => return None,
    })
}

fn toolbox_apps(root: &Path) -> Vec<PathBuf> {
    let mut layer = vec![root.to_path_buf()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for directory in layer {
            let mut children: Vec<_> = fs::read_dir(directory)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect();
            children.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
            next.extend(children);
        }
        layer = next;
    }
    layer
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .collect()
}

fn native_targets(path: &str, known_browsers: bool) -> Result<Vec<Value>> {
    #[cfg(target_os = "macos")]
    return mac::targets(path, known_browsers);
    #[cfg(target_os = "linux")]
    return linux::targets(path, known_browsers);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, known_browsers);
        Ok(Vec::new())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use base64::Engine as _;
    use objc2::{class, msg_send, rc::autoreleasepool, runtime::AnyObject};
    use objc2_foundation::NSString;

    struct Application {
        path: String,
        bundle: String,
        default: bool,
    }

    unsafe fn string(value: *mut AnyObject) -> Option<String> {
        if value.is_null() {
            None
        } else {
            Some(unsafe { &*(value as *const NSString) }.to_string())
        }
    }

    pub(super) fn targets(path: &str, known_browsers: bool) -> Result<Vec<Value>> {
        let web = Url::parse(path).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            || Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
                });
        let mut applications = autoreleasepool(|_| unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let value = NSString::from_str(if web { "https://example.com" } else { path });
            let url: *mut AnyObject = if web {
                msg_send![class!(NSURL), URLWithString: &*value]
            } else {
                msg_send![class!(NSURL), fileURLWithPath: &*value]
            };
            if url.is_null() {
                return Vec::new();
            }
            let default: *mut AnyObject = msg_send![workspace, URLForApplicationToOpenURL: url];
            let default_path: Option<PathBuf> = if default.is_null() {
                None
            } else {
                let raw: *mut AnyObject = msg_send![default, path];
                string(raw)
                    .map(PathBuf::from)
                    .map(|path| path.canonicalize().unwrap_or(path))
            };
            let urls: *mut AnyObject = msg_send![workspace, URLsForApplicationsToOpenURL: url];
            if urls.is_null() {
                return Vec::new();
            }
            let count: usize = msg_send![urls, count];
            let mut seen = HashSet::new();
            let mut applications = Vec::new();
            for index in 0..count {
                let url: *mut AnyObject = msg_send![urls, objectAtIndex: index];
                let raw: *mut AnyObject = msg_send![url, path];
                let Some(path) = string(raw) else {
                    continue;
                };
                let bundle: *mut AnyObject = msg_send![class!(NSBundle), bundleWithURL: url];
                if bundle.is_null() {
                    continue;
                }
                let raw: *mut AnyObject = msg_send![bundle, bundleIdentifier];
                let Some(bundle) = string(raw) else {
                    continue;
                };
                if [
                    "com.openai.codex.agent",
                    "com.openai.codex.dev",
                    "com.openai.codex.nightly",
                    "com.openai.codex.alpha",
                    "com.openai.codex.beta",
                    "com.openai.codex",
                ]
                .contains(&bundle.as_str())
                {
                    continue;
                }
                if web
                    && known_browsers
                    && ![
                        "com.apple.Safari",
                        "com.google.Chrome",
                        "com.google.Chrome.canary",
                        "org.mozilla.firefox",
                        "org.mozilla.nightly",
                        "company.thebrowser.Browser",
                        "com.brave.Browser",
                        "com.microsoft.edgemac",
                        "com.operasoftware.Opera",
                        "com.vivaldi.Vivaldi",
                        "com.kagi.kagimacOS",
                        "app.zen-browser.zen",
                    ]
                    .contains(&bundle.as_str())
                {
                    continue;
                }
                if !seen.insert(bundle.clone()) {
                    continue;
                }
                let canonical = Path::new(&path)
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(&path));
                applications.push(Application {
                    path,
                    bundle,
                    default: default_path.as_ref() == Some(&canonical),
                });
            }
            applications
        });
        let office = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        applications.sort_by_key(|app| {
            (
                !app.default,
                match office.as_deref() {
                    Some("pptx") if app.bundle == "com.microsoft.Powerpoint" => 0,
                    Some("xlsx") if app.bundle == "com.microsoft.Excel" => 0,
                    _ => 1,
                },
            )
        });
        applications.truncate(5);
        Ok(applications.into_iter().map(|app| {
            let mut target = json!({"id": format!("app:{}", app.path), "target": "systemDefault",
                "label": native_apps::display_name(&app.path), "icon": icon(&app.path).unwrap_or_else(|| "/apps/finder.png".into()),
                "kind": "native", "appPath": app.path, "bundleId": app.bundle, "available": true});
            if app.default { target["default"] = true.into(); }
            target
        }).collect())
    }

    fn icon(path: &str) -> Option<String> {
        let app = Path::new(path);
        let resources = app.join("Contents/Resources");
        let plist = app.join("Contents/Info.plist");
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIconFile", "raw", "-o", "-"])
            .arg(plist)
            .output()
            .ok()?;
        let name = String::from_utf8(output.stdout).ok()?;
        let name = name.trim();
        let icon = [
            resources.join(if name.ends_with(".icns") {
                name.to_owned()
            } else {
                format!("{name}.icns")
            }),
            resources.join(name),
            resources.join("AppIcon.icns"),
        ]
        .into_iter()
        .find(|path| path.is_file())?;
        let bytes = if icon
            .extension()
            .is_some_and(|extension| extension == "icns")
        {
            let temporary = tempfile::tempdir().ok()?;
            let status = Command::new("/usr/bin/qlmanage")
                .args(["-t", "-s", "32", "-o"])
                .arg(temporary.path())
                .arg(&icon)
                .output()
                .ok()?;
            if !status.status.success() {
                return None;
            }
            fs::read(
                temporary
                    .path()
                    .join(format!("{}.png", icon.file_name()?.to_string_lossy())),
            )
            .ok()?
        } else {
            png(&icon)?
        };
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }

    fn png(path: &Path) -> Option<Vec<u8>> {
        autoreleasepool(|_| unsafe {
            let path = NSString::from_str(path.to_str()?);
            let data: *mut AnyObject = msg_send![class!(NSData), dataWithContentsOfFile: &*path];
            if data.is_null() {
                return None;
            }
            let image: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: data];
            if image.is_null() {
                return None;
            }
            let bitmap: bool = msg_send![image, isKindOfClass: class!(NSBitmapImageRep)];
            if !bitmap {
                return None;
            }
            let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let png: *mut AnyObject =
                msg_send![image, representationUsingType: 4usize, properties: properties];
            if png.is_null() {
                return None;
            }
            let length: usize = msg_send![png, length];
            let bytes: *const u8 = msg_send![png, bytes];
            if bytes.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(bytes, length).to_vec())
        })
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use base64::Engine as _;

    pub(super) fn targets(path: &str, _: bool) -> Result<Vec<Value>> {
        let discovery = Discovery::new();
        let Some(gio) = discovery.executable("gio") else {
            return Ok(Vec::new());
        };
        let web = Url::parse(path).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        let mime = if web {
            "text/html".to_owned()
        } else {
            let Some(command) = discovery.executable("xdg-mime") else {
                return Ok(Vec::new());
            };
            let output = Command::new(command)
                .args(["query", "filetype"])
                .arg(path)
                .env("LC_ALL", "C")
                .output()?;
            if !output.status.success() {
                return Ok(Vec::new());
            }
            String::from_utf8(output.stdout)?.trim().to_owned()
        };
        let output = Command::new(gio)
            .args(["mime", &mime])
            .env("LC_ALL", "C")
            .output()?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let output = String::from_utf8(output.stdout)?;
        let default = output
            .lines()
            .find(|line| line.starts_with("Default application for "))
            .and_then(|line| line.rsplit_once(": ").map(|(_, id)| id.trim()));
        let mut ids = Vec::new();
        if let Some(id) = default {
            ids.push(id);
        }
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| line.ends_with(".desktop"))
        {
            if !ids.contains(&line) {
                ids.push(line);
            }
        }
        let mut targets = Vec::new();
        for id in ids {
            if id.starts_with("com.openai.codex") {
                continue;
            }
            let Some(app) = discovery.desktop(&[id]) else {
                continue;
            };
            let contents = fs::read_to_string(&app)?;
            let mut section = false;
            let mut name = None;
            let mut icon_path = None;
            for line in contents.lines() {
                if line.starts_with('[') {
                    section = line == "[Desktop Entry]";
                }
                if !section {
                    continue;
                }
                if let Some(value) = line.strip_prefix("Name=") {
                    name = Some(value);
                }
                if let Some(value) = line.strip_prefix("Icon=") {
                    icon_path = Some(PathBuf::from(value));
                }
            }
            let icon = icon_path
                .filter(|path| path.is_absolute())
                .and_then(|path| {
                    let extension = path.extension()?.to_str()?;
                    if !matches!(extension, "png" | "jpg" | "jpeg") {
                        return None;
                    }
                    fs::read(&path).ok().map(|bytes| {
                        format!(
                            "data:image/{extension};base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        )
                    })
                })
                .unwrap_or_else(|| "/apps/file-explorer.png".into());
            let app_path = app.to_str().context("desktop file path must be UTF-8")?;
            let mut target = json!({"id": format!("app:{app_path}"), "target": "systemDefault", "label": name.unwrap_or(id),
                "icon": icon, "kind": "native", "appPath": app_path, "bundleId": id, "available": true});
            if Some(id) == default {
                target["default"] = true.into();
            }
            targets.push(target);
            if targets.len() == 5 {
                break;
            }
        }
        Ok(targets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Host, Request, json_body};
    use crate::host_settings::Settings;

    #[test]
    fn catalog_observes_shared_preferences_and_rejects_invalid_custom_handlers() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let directory = tempfile::tempdir()?;
        let mut settings = Settings::default();
        let host = LocalHost::new(directory.path()).with_settings(Some(settings.clone()));
        settings.write(
            "open-in-target-preferences",
            json!({
                "global": "vscode", "perPath": {"/project": "cursor"}
            }),
        )?;
        let query = Request::post(
            "open-in-targets",
            Some(json!({
                "cwd": "/project", "deferEnrichment": true
            })),
        );
        let catalog = json_body(&host.fetch(&query)?)?;
        assert_eq!(catalog["preferredTarget"], "cursor");
        assert_eq!(catalog["availableTargets"], json!([]));
        let rows = catalog["targets"].as_array().unwrap();
        assert_eq!(rows[0]["id"], "vscode");
        assert_eq!(
            rows.iter().find(|row| row["id"] == "cursor").unwrap()["default"],
            true
        );
        assert!(rows.iter().all(|row| row.get("available").is_none()));
        settings.write(
            "open-in-target-preferences",
            json!({
                "global": "finder", "perPath": {"/project": "missing-target"}
            }),
        )?;
        let catalog = json_body(&host.fetch(&query)?)?;
        assert_eq!(catalog["preferredTarget"], "fileManager");
        settings.write(
            "open-in-target-preferences",
            json!({"global": "missing-target"}),
        )?;
        assert!(json_body(&host.fetch(&query)?)?["preferredTarget"].is_null());
        let enriched = json_body(&host.fetch(&Request::post("open-in-targets", Some(json!({}))))?)?;
        assert_eq!(enriched["mode"], "editor");
        let fallback = enriched["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["id"] == "systemDefault")
            .unwrap();
        assert_eq!(fallback["available"], true);
        assert_eq!(fallback["hidden"], true);
        assert!(
            host.fetch(&Request::post(
                "open-in-targets",
                Some(json!({"deferEnrichment": "yes"}))
            ))
            .is_err()
        );
        let handler = json!({"label": "Open safely", "icon": "apps/vscode.png", "command": "/bin/echo", "args": [], "input": "path"});
        let custom = custom_handlers(json!({"safe-tool": handler, "../escape": handler}));
        assert_eq!(
            custom.keys().map(String::as_str).collect::<Vec<_>>(),
            ["safe-tool"]
        );
        assert!(
            custom_handlers(json!({"safe-tool": handler, "invalid": {"label": ""}})).is_empty()
        );
        assert_eq!(
            custom_icon("/tmp/icon #1.png"),
            "app://fs/@fs/tmp/icon%20%231.png"
        );
        assert_eq!(custom_icon("apps/../private.png"), "/apps/vscode.png");
        let image = directory.path().join("screen.PNG");
        fs::write(&image, [])?;
        assert!(native_path(image.to_str().unwrap()));
        assert!(!native_path(
            directory.path().join("missing.rs").to_str().unwrap()
        ));
        assert!(native_path("https://example.com"));
        assert!(!office_path("https://example.com/file.docx"));
        Ok(())
    }
}
