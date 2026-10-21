use super::*;

pub(super) const TARGETS: &[(&str, &str, &str)] = &[
    ("vscode", "VS Code", "Visual Studio Code"),
    (
        "vscodeInsiders",
        "VS Code Insiders",
        "Visual Studio Code - Insiders",
    ),
    ("cursor", "Cursor", "Cursor"),
    ("devin", "Devin Desktop", "Devin Desktop"),
    ("bbedit", "BBEdit", "BBEdit"),
    ("emacs", "Emacs", "Emacs"),
    ("sublimeText", "Sublime Text", "Sublime Text"),
    ("zed", "Zed", "Zed"),
    ("windsurf", "Windsurf", "Windsurf"),
    ("antigravity", "Antigravity", "Antigravity"),
    ("systemDefault", "Default app", ""),
    ("fileManager", "Finder", "Finder"),
    ("terminal", "Terminal", "Terminal"),
    ("iterm2", "iTerm2", "iTerm"),
    ("kitty", "Kitty", "kitty"),
    ("ghostty", "Ghostty", "Ghostty"),
    ("warp", "Warp", "Warp"),
    ("xcode", "Xcode", "Xcode"),
    ("androidStudio", "Android Studio", "Android Studio"),
    ("intellij", "IntelliJ IDEA", "IntelliJ IDEA"),
    ("rider", "Rider", "Rider"),
    ("goland", "GoLand", "GoLand"),
    ("rustrover", "RustRover", "RustRover"),
    ("pycharm", "PyCharm", "PyCharm"),
    ("webstorm", "WebStorm", "WebStorm"),
    ("phpstorm", "PhpStorm", "PhpStorm"),
    ("textmate", "TextMate", "TextMate"),
];

#[cfg(target_os = "macos")]
fn installed(application: &str) -> bool {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::NSString;
    let workspace: Retained<AnyObject> = unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] };
    let path: Option<Retained<NSString>> = unsafe {
        msg_send![&*workspace, fullPathForApplication: &*NSString::from_str(application)]
    };
    path.is_some()
}

#[cfg(not(target_os = "macos"))]
fn installed(_: &str) -> bool {
    false
}

impl LocalHost {
    pub(super) fn open_in_targets(&self) -> Result<Value> {
        let settings = crate::host_settings::Settings::load(self.host_settings_path.clone())?;
        let preference = settings
            .read_all()
            .get("open-in-target-preferences")
            .and_then(|value| value["global"].as_str());
        let targets = TARGETS.iter().map(|&(id, label, application)| {
            json!({"id": id, "target": id, "label": label, "available": application.is_empty() || installed(application)})
        }).collect::<Vec<_>>();
        let available = targets
            .iter()
            .filter(|target| target["available"] == true)
            .filter_map(|target| target["id"].as_str())
            .collect::<Vec<_>>();
        let preferred = preference
            .filter(|target| available.contains(target))
            .or_else(|| available.first().copied());
        Ok(json!({"preferredTarget": preferred, "availableTargets": available, "targets": targets}))
    }

    pub(super) fn open_with_preference(&self, params: &Value) -> Result<Value> {
        if params.get("target").is_some_and(|value| !value.is_null()) {
            return super::open_file(&self.opener, params);
        }
        let settings = crate::host_settings::Settings::load(self.host_settings_path.clone())?;
        let preferred = settings
            .read_all()
            .get("open-in-target-preferences")
            .and_then(|value| value["global"].as_str());
        if let Some(preferred) = preferred {
            let mut params = params.clone();
            params["target"] = preferred.into();
            super::open_file(&self.opener, &params)
        } else {
            super::open_file(&self.opener, params)
        }
    }
}

pub(super) fn notices() -> Value {
    json!({"text": include_str!("../../../../assets/app/THIRD_PARTY_NOTICES.txt")})
}
