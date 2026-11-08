use super::LocalHost;
#[cfg(target_os = "macos")]
use super::native_apps;
use anyhow::{Context as _, Result, ensure};
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
        let custom = self.file_handlers(query.cwd.as_deref())?;
        let targets: Vec<_> = TARGETS.iter().filter(|target| target.supported()).collect();
        let mut rows: Vec<_> = targets.iter().map(|target| target.metadata()).collect();
        for (id, handler) in &custom {
            let target = format!("custom:{id}");
            rows.push(json!({"id": target, "target": target,
                "label": handler["label"], "icon": custom_icon(handler["icon"].as_str().unwrap()), "kind": "editor"}));
        }
        let configured = saved_preference(Some(settings), query.cwd.as_deref(), &custom);
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
        if joined.symlink_metadata().is_ok() {
            joined
        } else {
            let path = path
                .strip_prefix("a")
                .or_else(|_| path.strip_prefix("b"))
                .ok()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(&path);
            let joined = cwd.join(path);
            let name = cwd.file_name();
            let remainder = path
                .components()
                .skip_while(|part| Some(part.as_os_str()) != name)
                .skip(1)
                .collect::<PathBuf>();
            if joined.exists() || remainder.as_os_str().is_empty() {
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
        let home = crate::env::home().unwrap_or_default();
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRequest {
    path: String,
    cwd: Option<String>,
    target: Option<String>,
    app_path: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    persist_preferred_target_path: Option<String>,
}

#[derive(Clone, Copy, serde::Serialize)]
struct Location {
    line: u32,
    column: u32,
}

#[derive(Clone, Copy)]
struct Launch<'a> {
    target: &'a str,
    command: &'a Path,
    path: &'a str,
    app_path: Option<&'a str>,
    location: Option<Location>,
}

impl LocalHost {
    fn file_handlers(&self, cwd: Option<&str>) -> Result<Map<String, Value>> {
        if let Some(bridge) = &self.bridge
            && !bridge.is_acp()
        {
            let config: Value = pollster::block_on(
                bridge.request("config/read", json!({"includeLayers": true, "cwd": cwd})),
            )?;
            Ok(layered_handlers(config))
        } else {
            Ok(Map::new())
        }
    }

    pub(super) fn open_in(&self, params: &Value) -> Result<Value> {
        let request = OpenRequest::deserialize(params).context("invalid open-in request")?;
        let result = self.open_target(&request);
        if let Err(error) = &result {
            eprintln!("codex-gpui: cannot open target: {error:#}");
        }
        Ok(json!({"success": result.is_ok()}))
    }

    fn open_target(&self, request: &OpenRequest) -> Result<()> {
        ensure!(
            !request.path.is_empty() && !request.path.contains('\0'),
            "invalid open path"
        );
        let discovery = Discovery::new();
        if is_web_url(&request.path) {
            ensure!(
                request
                    .target
                    .as_deref()
                    .is_none_or(|target| target == "systemDefault"),
                "web URLs require the default-application target"
            );
            return self.launch(
                Launch {
                    target: "systemDefault",
                    command: &self.opener,
                    path: &request.path,
                    app_path: request.app_path.as_deref(),
                    location: None,
                },
                &discovery,
                &Map::new(),
            );
        }
        let path = checked_project_path(&request.path, request.cwd.as_deref())?;
        let path_text = path.to_str().context("open path must be UTF-8")?;
        let custom = self.file_handlers(request.cwd.as_deref())?;
        let resolve = |id: &str| -> Option<PathBuf> {
            if TARGETS
                .iter()
                .any(|target| target.id == id && target.supported())
            {
                discovery.command(id)
            } else {
                id.strip_prefix("custom:")
                    .and_then(|id| custom.get(id))
                    .and_then(|handler| discovery.executable(handler["command"].as_str()?))
            }
        };
        let explicit = request.target.is_some();
        let (mut target, mut command) = if let Some(id) = &request.target {
            (
                id.clone(),
                resolve(id).context("requested open target is unavailable")?,
            )
        } else {
            saved_preference(self.settings.as_ref(), request.cwd.as_deref(), &custom)
                .and_then(|id| resolve(&id).map(|command| (id, command)))
                .or_else(|| {
                    TARGETS
                        .iter()
                        .filter(|target| target.supported())
                        .find_map(|target| {
                            resolve(target.id).map(|command| (target.id.to_owned(), command))
                        })
                })
                .or_else(|| {
                    custom.keys().find_map(|id| {
                        let target = format!("custom:{id}");
                        resolve(&target).map(|command| (target, command))
                    })
                })
                .context("no available open target")?
        };
        let location = request.line.map(|line| Location {
            line,
            column: request.column.unwrap_or(1),
        });
        if !explicit {
            let file_manager = path.is_dir()
                || (cfg!(target_os = "macos")
                    && native_path(path_text)
                    && location.is_none()
                    && is_editor_target(&target, &custom));
            if file_manager {
                if let Some(found) = discovery.command("fileManager") {
                    target = "fileManager".into();
                    command = found;
                }
            } else if !cfg!(target_os = "macos") && native_path(path_text) {
                target = "systemDefault".into();
                command = self.opener.clone();
            }
        }
        if let Some(preference_path) = &request.persist_preferred_target_path
            && (explicit
                || saved_preference(self.settings.as_ref(), Some(preference_path), &custom)
                    .is_none())
        {
            let mut settings = self
                .settings
                .clone()
                .context("host settings are unavailable")?;
            settings.update("open-in-target-preferences", |previous| {
                let mut per_path = previous
                    .and_then(|value| value["perPath"].as_object())
                    .cloned()
                    .unwrap_or_default();
                per_path.insert(preference_path.clone(), json!(target));
                Ok(json!({"global": target, "perPath": per_path}))
            })?;
        }
        self.launch(
            Launch {
                target: &target,
                command: &command,
                path: path_text,
                app_path: request.app_path.as_deref(),
                location,
            },
            &discovery,
            &custom,
        )
    }

    fn launch(
        &self,
        launch: Launch<'_>,
        discovery: &Discovery,
        custom: &Map<String, Value>,
    ) -> Result<()> {
        if let Some(id) = launch.target.strip_prefix("custom:") {
            return launch_custom(
                launch,
                custom.get(id).context("custom target is unavailable")?,
            );
        }
        let target = TARGETS
            .iter()
            .find(|target| target.id == launch.target && target.supported())
            .context("unknown open target")?;
        if target.kind == "systemDefault" {
            return self.launch_default(launch, discovery);
        }
        if target.kind == "terminal" {
            return self.launch_terminal(launch, discovery);
        }
        if target.kind == "fileManager" {
            let mut command = Command::new(if cfg!(target_os = "macos") {
                &self.opener
            } else {
                launch.command
            });
            if cfg!(target_os = "macos") {
                command.arg("-R");
            }
            return run_open_command(command.arg(launch.path), None);
        }
        if cfg!(target_os = "linux")
            && launch
                .command
                .extension()
                .is_some_and(|extension| extension == "desktop")
        {
            let gio = discovery
                .executable("gio")
                .context("desktop application launcher is unavailable")?;
            return run_open_command(
                Command::new(gio)
                    .arg("launch")
                    .arg(launch.command)
                    .arg(launch.path),
                None,
            );
        }
        match launch.target {
            "vscode" | "vscodeInsiders" | "windsurf" | "antigravity" | "devin" => run_open_command(
                Command::new(launch.command)
                    .arg("--goto")
                    .arg(location_path(launch)),
                None,
            ),
            "cursor" if cfg!(target_os = "macos") => {
                let cli = launch
                    .command
                    .parent()
                    .and_then(Path::parent)
                    .context("invalid Cursor application")?
                    .join("Resources/app/out/cli.js");
                let mut command = Command::new(launch.command);
                if let Some(value) = std::env::var_os("NODE_OPTIONS") {
                    command.env("VSCODE_NODE_OPTIONS", value);
                }
                if let Some(value) = std::env::var_os("NODE_REPL_EXTERNAL_MODULE") {
                    command.env("VSCODE_NODE_REPL_EXTERNAL_MODULE", value);
                }
                command
                    .env_remove("NODE_OPTIONS")
                    .env_remove("NODE_REPL_EXTERNAL_MODULE")
                    .env("ELECTRON_RUN_AS_NODE", "1")
                    .arg(cli)
                    .arg("--goto")
                    .arg(location_path(launch));
                run_open_command(&mut command, None)
            }
            "cursor" => run_open_command(
                Command::new(launch.command)
                    .arg("--goto")
                    .arg(location_path(launch)),
                None,
            ),
            "bbedit" | "emacs" if cfg!(target_os = "macos") => run_open_command(
                Command::new(&self.opener)
                    .arg("-a")
                    .arg(if launch.target == "bbedit" {
                        "BBEdit"
                    } else {
                        "Emacs"
                    })
                    .arg(launch.path),
                None,
            ),
            "emacs" => {
                let mut command = Command::new(launch.command);
                if let Some(location) = launch.location {
                    command.arg(format!("+{}:{}", location.line, location.column));
                }
                run_open_command(command.arg(launch.path), None)
            }
            "textmate" => {
                let path = if let Some(location) = launch.location {
                    let mut uri = Url::parse("txmt://open/")?;
                    uri.query_pairs_mut()
                        .append_pair(
                            "url",
                            Url::from_file_path(launch.path)
                                .map_err(|_| anyhow::anyhow!("invalid TextMate path"))?
                                .as_str(),
                        )
                        .append_pair("line", &location.line.to_string())
                        .append_pair("column", &location.column.to_string());
                    uri.to_string()
                } else {
                    launch.path.to_owned()
                };
                run_open_command(
                    Command::new(&self.opener).args(["-a", "TextMate", &path]),
                    None,
                )
            }
            "sublimeText" => run_open_command(
                Command::new(launch.command).arg(location_path(launch)),
                None,
            ),
            "zed" => {
                if cfg!(target_os = "macos")
                    && let Some(app) = app_bundle(launch.command)
                {
                    run_open_command(
                        Command::new(&self.opener)
                            .arg("-a")
                            .arg(app)
                            .arg(launch.path),
                        None,
                    )?;
                    if launch.location.is_some() {
                        let command = discovery
                            .executable("zed")
                            .unwrap_or_else(|| launch.command.to_path_buf());
                        let _ = run_open_command(
                            Command::new(command).arg(location_path(launch)),
                            None,
                        );
                    }
                    Ok(())
                } else {
                    run_open_command(
                        Command::new(launch.command).arg(location_path(launch)),
                        None,
                    )
                }
            }
            "xcode" => {
                if launch
                    .command
                    .extension()
                    .is_some_and(|extension| extension == "app")
                {
                    return run_open_command(
                        Command::new(&self.opener)
                            .arg("-a")
                            .arg(launch.command)
                            .arg(launch.path),
                        None,
                    );
                }
                let mut command = Command::new(launch.command);
                if let Some(project) = xcode_container(Path::new(launch.path)) {
                    command.arg("--project").arg(project);
                }
                if let Some(location) = launch.location {
                    command.arg("--line").arg(location.line.to_string());
                }
                run_open_command(command.arg(launch.path), None)
            }
            _ if jetbrains_tool(launch.target).is_some() => {
                let mut command = Command::new(launch.command);
                if let Some(location) = launch.location {
                    command.args([
                        "--line",
                        &location.line.to_string(),
                        "--column",
                        &location.column.to_string(),
                    ]);
                }
                run_open_command(command.arg(launch.path), None)
            }
            _ => anyhow::bail!("unsupported open target"),
        }
    }

    fn launch_default(&self, launch: Launch<'_>, discovery: &Discovery) -> Result<()> {
        if cfg!(target_os = "macos") {
            let mut command = Command::new(&self.opener);
            if let Some(app) = launch.app_path {
                let app = Path::new(app);
                ensure!(
                    app.is_absolute()
                        && app.is_dir()
                        && app.extension().is_some_and(|extension| extension == "app"),
                    "invalid native application path"
                );
                command.arg("-a").arg(app);
            }
            return run_open_command(command.arg(launch.path), None);
        }
        if let Some(app) = launch.app_path {
            let app = Path::new(app);
            ensure!(
                app.is_absolute()
                    && app.is_file()
                    && app
                        .extension()
                        .is_some_and(|extension| extension == "desktop"),
                "invalid desktop application path"
            );
            let gio = discovery
                .executable("gio")
                .context("desktop application launcher is unavailable")?;
            return run_open_command(
                Command::new(gio).arg("launch").arg(app).arg(launch.path),
                None,
            );
        }
        if self.opener != Path::new("/usr/bin/open") || self.opener.exists() {
            return run_open_command(Command::new(&self.opener).arg(launch.path), None);
        }
        if let Some(opener) = discovery.executable("xdg-open") {
            run_open_command(Command::new(opener).arg(launch.path), None)
        } else {
            let gio = discovery
                .executable("gio")
                .context("default application launcher is unavailable")?;
            run_open_command(Command::new(gio).arg("open").arg(launch.path), None)
        }
    }

    fn launch_terminal(&self, launch: Launch<'_>, discovery: &Discovery) -> Result<()> {
        let path = Path::new(launch.path);
        let directory = if path.is_dir() {
            path
        } else {
            path.parent().context("file has no parent")?
        };
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "/bin/zsh"
                } else {
                    "/bin/sh"
                }
                .into()
            });
        let editor = editor_command(discovery);
        if cfg!(target_os = "macos") {
            if path.is_file()
                && let Some(editor) = editor
            {
                let script = editor_script(&editor, launch.path, launch.location)?;
                if let Some(mut command) =
                    mac_terminal_command(launch.target, &self.opener, &shell, &script)?
                {
                    if run_open_command(&mut command, None).is_ok() {
                        return Ok(());
                    }
                    eprintln!("codex-gpui: terminal editor command failed; opening the directory");
                }
            }
            let app = match launch.target {
                "terminal" => "Terminal",
                "iterm2" => "iTerm",
                "kitty" => "kitty",
                "ghostty" => "Ghostty",
                "warp" => "Warp",
                _ => anyhow::bail!("unsupported terminal"),
            };
            return run_open_command(
                Command::new(&self.opener).arg("-a").arg(app).arg(directory),
                None,
            );
        }
        let name = launch
            .command
            .file_name()
            .and_then(|name| name.to_str())
            .context("invalid terminal command")?;
        let mut command = Command::new(launch.command);
        match name {
            "kitty" => {
                command.arg("--directory").arg(directory);
            }
            "ghostty" => {
                command.arg(format!("--working-directory={}", directory.display()));
            }
            "konsole" => {
                command.arg("--workdir").arg(directory);
            }
            "alacritty" | "gnome-terminal" | "kgx" | "mate-terminal" | "tilix"
            | "xfce4-terminal" => {
                command.arg("--working-directory").arg(directory);
            }
            "x-terminal-emulator" | "zutty" => {}
            _ => anyhow::bail!("unsupported Linux terminal command"),
        }
        if path.is_file()
            && let Some(editor) = editor
        {
            let script = editor_script(&editor, launch.path, launch.location)?;
            match name {
                "kitty" => {
                    command.args([&shell, "-lc", &script]);
                }
                "mate-terminal" | "xfce4-terminal" => {
                    command.args(["--execute", &shell, "-lc", &script]);
                }
                "gnome-terminal" => {
                    command.args(["--", &shell, "-lc", &script]);
                }
                _ => {
                    command.args(["-e", &shell, "-lc", &script]);
                }
            }
        } else if matches!(name, "x-terminal-emulator" | "zutty") {
            let script = format!(
                "cd {} && exec {}",
                shlex::try_quote(
                    directory
                        .to_str()
                        .context("terminal directory must be UTF-8")?
                )?,
                shlex::try_quote(&shell)?
            );
            command.args(["-e", &shell, "-lc", &script]);
        }
        run_open_command(&mut command, None)
    }
}

fn normalize_target<'a>(target: &'a str, custom: &Map<String, Value>) -> Option<&'a str> {
    let target = if target == "finder" {
        "fileManager"
    } else {
        target
    };
    (TARGETS
        .iter()
        .any(|candidate| candidate.id == target && candidate.supported())
        || target
            .strip_prefix("custom:")
            .is_some_and(|id| custom.contains_key(id)))
    .then_some(target)
}

fn saved_preference(
    settings: Option<&crate::host_settings::Settings>,
    cwd: Option<&str>,
    custom: &Map<String, Value>,
) -> Option<String> {
    let settings = settings?;
    let values = settings.read_all();
    let prefs = values.get("open-in-target-preferences")?;
    cwd.and_then(|cwd| prefs["perPath"][cwd].as_str())
        .and_then(|target| normalize_target(target, custom))
        .or_else(|| {
            prefs["global"]
                .as_str()
                .and_then(|target| normalize_target(target, custom))
        })
        .map(str::to_owned)
}

fn is_editor_target(target: &str, custom: &Map<String, Value>) -> bool {
    TARGETS
        .iter()
        .any(|candidate| candidate.id == target && candidate.kind == "editor")
        || target
            .strip_prefix("custom:")
            .is_some_and(|id| custom.contains_key(id))
}

fn is_web_url(path: &str) -> bool {
    Url::parse(path).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn checked_project_path(path: &str, cwd: Option<&str>) -> Result<PathBuf> {
    let cwd = cwd
        .map(PathBuf::from)
        .map_or_else(std::env::current_dir, Ok)?;
    ensure!(cwd.is_absolute(), "project directory must be absolute");
    let output = Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output();
    let root = output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| PathBuf::from(path.trim_end_matches(['\n', '\r'])))
        .unwrap_or_else(|| cwd.clone())
        .canonicalize()
        .context("resolve the project folder")?;
    let path = PathBuf::from(resolve_path(path, cwd.to_str())?)
        .canonicalize()
        .context("open path no longer exists")?;
    ensure!(path.starts_with(root), "open path is outside the project");
    ensure!(
        path.is_file() || path.is_dir(),
        "open path is not a file or folder"
    );
    Ok(path)
}

fn location_path(launch: Launch<'_>) -> String {
    launch.location.map_or_else(
        || launch.path.to_owned(),
        |location| format!("{}:{}:{}", launch.path, location.line, location.column),
    )
}

fn app_bundle(command: &Path) -> Option<PathBuf> {
    let value = command.to_str()?;
    value
        .find(".app/Contents/MacOS/")
        .map(|index| PathBuf::from(&value[..index + 4]))
}

fn xcode_container(path: &Path) -> Option<PathBuf> {
    let mut directory = if path.is_dir() { path } else { path.parent()? };
    let mut package = None;
    loop {
        let entries: Vec<_> = fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        if let Some(workspace) = entries.iter().find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "xcworkspace")
        }) {
            return Some(workspace.clone());
        }
        if let Some(project) = entries.iter().find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "xcodeproj")
        }) {
            return Some(project.clone());
        }
        if package.is_none()
            && entries
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == "Package.swift"))
        {
            package = Some(directory.to_path_buf());
        }
        let Some(parent) = directory.parent() else {
            return package;
        };
        directory = parent;
    }
}

fn run_open_command(command: &mut Command, input: Option<&[u8]>) -> Result<()> {
    use std::{io::Write as _, process::Stdio};
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().context("start open target")?;
    let written = if let Some(input) = input {
        child
            .stdin
            .take()
            .context("open target has no stdin")?
            .write_all(input)
    } else {
        Ok(())
    };
    let status = child.wait().context("wait for open target")?;
    written.context("write open target input")?;
    ensure!(status.success(), "open target exited unsuccessfully");
    Ok(())
}

fn launch_custom(launch: Launch<'_>, handler: &Value) -> Result<()> {
    let mut command = Command::new(launch.command);
    if let Some(args) = handler["args"].as_array() {
        command.args(args.iter().filter_map(Value::as_str));
    }
    let payload = || json!({"target": launch.target, "path": launch.path, "appPath": launch.app_path, "location": launch.location});
    match handler["input"].as_str().unwrap_or("path") {
        "path" => run_open_command(command.arg(launch.path), None),
        "json_argument" => run_open_command(command.arg(payload().to_string()), None),
        "json_stdin" => {
            let mut payload = payload();
            payload["hostConfig"] = json!({"id": "local", "kind": "local"});
            payload["remoteWorkspaceRoot"] = Value::Null;
            payload["remotePath"] = Value::Null;
            run_open_command(&mut command, Some(payload.to_string().as_bytes()))
        }
        _ => anyhow::bail!("unsupported custom target input"),
    }
}

fn editor_command(discovery: &Discovery) -> Option<String> {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            ["nvim", "vim", "nano", "less"]
                .into_iter()
                .find_map(|name| discovery.executable(name))
                .map(|path| path.to_string_lossy().into_owned())
        })
}

fn editor_script(editor: &str, path: &str, location: Option<Location>) -> Result<String> {
    let directory = Path::new(path)
        .parent()
        .context("editor path has no parent")?
        .to_str()
        .context("editor directory must be UTF-8")?;
    let command = if let Some(mut args) = shlex::split(editor).filter(|args| !args.is_empty()) {
        let executable = Path::new(&args[0])
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .trim_end_matches(".exe")
            .to_lowercase();
        if let Some(location) = location {
            let argument = match executable.as_str() {
                "nvim" | "vim" => Some(format!(
                    "+call cursor({},{})",
                    location.line, location.column
                )),
                "nano" => Some(format!("+{},{}", location.line, location.column)),
                "less" => Some(format!("+{}g", location.line)),
                _ => None,
            };
            if let Some(argument) = argument {
                let index = args
                    .iter()
                    .position(|argument| argument == "--")
                    .unwrap_or(args.len());
                args.insert(index, argument);
            }
        }
        args.push(path.to_owned());
        shlex::try_join(args.iter().map(String::as_str))?
    } else {
        format!("{editor} {}", shlex::try_quote(path)?)
    };
    Ok(format!("cd {} && {command}", shlex::try_quote(directory)?))
}

fn mac_terminal_command(
    target: &str,
    opener: &Path,
    shell: &str,
    script: &str,
) -> Result<Option<Command>> {
    if matches!(target, "terminal" | "iterm2") {
        ensure!(
            !script.chars().any(char::is_control),
            "control characters are unsafe in an interactive terminal command"
        );
    }
    let mut command = match target {
        "kitty" | "ghostty" => Command::new(opener),
        "terminal" | "iterm2" => Command::new("/usr/bin/osascript"),
        _ => return Ok(None),
    };
    match target {
        "kitty" => {
            command.args(["-na", "kitty.app", "--args", shell, "-lc", script]);
        }
        "ghostty" => {
            command.args(["-na", "Ghostty.app", "--args", "-e", shell, "-lc", script]);
        }
        "terminal" | "iterm2" => {
            let mut escaped = String::with_capacity(script.len());
            for ch in script.chars() {
                if matches!(ch, '\\' | '"') {
                    escaped.push('\\');
                }
                escaped.push(ch);
            }
            let source = if target == "terminal" {
                format!("tell application \"Terminal\" to do script \"{escaped}\"")
            } else {
                format!(
                    "tell application \"iTerm\"\ncreate window with default profile\ntell current session of current window to write text \"{escaped}\"\nend tell"
                )
            };
            command.arg("-e").arg(source);
        }
        _ => return Ok(None),
    }
    Ok(Some(command))
}

#[cfg(test)]
mod launch_tests {
    use super::*;
    use crate::{
        host::{Host, Request, json_body},
        host_settings::Settings,
    };
    use bridge::Bridge;
    use std::sync::Arc;

    #[cfg(unix)]
    #[test]
    fn custom_launch_modes_preserve_paths_preferences_and_failure_results() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let layered = layered_handlers(json!({
            "config": {},
            "layers": [
                {"disabledReason": "project is not trusted", "config": {"custom_file_handlers": {
                    "tool": {"command": "/blocked"}
                }}},
                {"config": {"custom_file_handlers": {"tool": {"command": "/user-tool"}}}},
                {"config": {"custom_file_handlers": {"tool": {
                    "label": "System tool", "icon": "apps/vscode.png", "command": "/system-tool", "args": ["kept"]
                }}}}
            ]
        }));
        assert_eq!(layered["tool"]["command"], "/user-tool");
        assert_eq!(layered["tool"]["label"], "System tool");
        assert_eq!(layered["tool"]["args"], json!(["kept"]));
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        fs::create_dir(&project)?;
        let marker = directory.path().join("injected");
        let file = project.join(format!("name;touch {}", marker.display()));
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, "keep")?;
        let file = file.canonicalize()?;
        let handler = directory.path().join("handler.sh");
        fs::write(
            &handler,
            "#!/bin/sh\ncase \"$2\" in\nstdin) cat > \"$1\";;\nfail) exit 3;;\n*) printf '%s' \"$3\" > \"$1\";;\nesac\n",
        )?;
        fs::set_permissions(&handler, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
        let mut settings = Settings::default();
        settings.write(
            "open-in-target-preferences",
            json!({"perPath": {"/kept": "vscode", "/other-project": "custom:elsewhere"}}),
        )?;
        for (mode, input) in [
            ("path", "path"),
            ("argument", "json_argument"),
            ("stdin", "json_stdin"),
            ("fail", "path"),
        ] {
            let output = directory.path().join(format!("{mode}.json"));
            let script = directory.path().join(format!("{mode}.jsonl"));
            let config = json!({"config": {"custom_file_handlers": {
                (mode): {"label": mode, "icon": "apps/vscode.png", "command": handler, "args": [output, mode], "input": input}
            }}});
            fs::write(
                &script,
                format!(
                    "{}\n{}\n",
                    json!({"dir": "send", "msg": {"id": 1, "method": "config/read", "params": {"includeLayers": true, "cwd": project}}}),
                    json!({"dir": "recv", "msg": {"id": 1, "result": config}})
                ),
            )?;
            let (bridge, _) = Bridge::replay(&script)?;
            let host = LocalHost::new(directory.path())
                .with_settings(Some(settings.clone()))
                .with_bridge(Arc::new(bridge));
            let target = format!("custom:{mode}");
            let response = json_body(&host.fetch(&Request::post(
                "open-in",
                Some(json!({
                    "cwd": project, "path": file, "target": target, "line": 4, "column": 2,
                    "persistPreferredTargetPath": project
                })),
            ))?)?;
            assert_eq!(response["success"], mode != "fail");
            assert_eq!(
                settings.read_all()["open-in-target-preferences"]["global"],
                target
            );
            assert_eq!(
                settings.read_all()["open-in-target-preferences"]["perPath"]["/kept"],
                "vscode"
            );
            assert_eq!(
                settings.read_all()["open-in-target-preferences"]["perPath"]["/other-project"],
                "custom:elsewhere"
            );
            if mode == "fail" {
                assert!(!output.exists());
            } else if mode == "path" {
                assert_eq!(fs::read_to_string(&output)?, file.to_str().unwrap());
            } else {
                let payload: Value = serde_json::from_slice(&fs::read(&output)?)?;
                assert_eq!(payload["path"], file.to_str().unwrap());
                assert_eq!(payload["target"], target);
                assert_eq!(payload["location"], json!({"line": 4, "column": 2}));
                if mode == "stdin" {
                    assert_eq!(
                        payload["hostConfig"],
                        json!({"id": "local", "kind": "local"})
                    );
                    assert!(payload["remotePath"].is_null());
                }
            }
            assert!(!marker.exists(), "a file path executed as shell input");
        }
        let shell = editor_script("/usr/bin/printf '%s'", file.to_str().unwrap(), None)?;
        let output = Command::new("/bin/sh").arg("-c").arg(shell).output()?;
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout)?, file.to_str().unwrap());
        assert!(!marker.exists());
        for target in ["terminal", "iterm2"] {
            assert!(
                mac_terminal_command(
                    target,
                    Path::new("/usr/bin/open"),
                    "/bin/sh",
                    "unsafe\u{3}input"
                )
                .is_err()
            );
        }
        assert_eq!(fs::read_to_string(file)?, "keep");
        Ok(())
    }

    #[test]
    fn literal_project_paths_win_over_diff_prefixes() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let project = tempfile::tempdir()?;
        let cwd = project.path().to_str().unwrap();
        fs::write(project.path().join("file.txt"), "wrong file")?;
        for prefix in ["a", "b"] {
            fs::create_dir(project.path().join(prefix))?;
            fs::write(project.path().join(prefix).join("file.txt"), prefix)?;
            let path = checked_project_path(&format!("{prefix}/file.txt"), Some(cwd))?;
            assert_eq!(fs::read_to_string(path)?, prefix);
        }
        fs::write(project.path().join("fallback.txt"), "diff fallback")?;
        let path = checked_project_path("a/fallback.txt", Some(cwd))?;
        assert_eq!(fs::read_to_string(path)?, "diff fallback");
        Ok(())
    }
}

fn layered_handlers(mut config: Value) -> Map<String, Value> {
    let mut handlers = config["config"]["custom_file_handlers"].take();
    if let Value::Array(layers) = config["layers"].take() {
        // The app-server lists higher-priority layers first.
        for mut layer in layers.into_iter().rev() {
            if layer["disabledReason"].is_null()
                && layer["config"].get("custom_file_handlers").is_some()
            {
                merge_handler_layer(
                    &mut handlers,
                    layer["config"]["custom_file_handlers"].take(),
                );
            }
        }
    }
    custom_handlers(handlers)
}

fn merge_handler_layer(base: &mut Value, incoming: Value) {
    match (base, incoming) {
        (Value::Object(base), Value::Object(incoming)) => {
            for (key, value) in incoming {
                merge_handler_layer(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, incoming) => *base = incoming,
    }
}
