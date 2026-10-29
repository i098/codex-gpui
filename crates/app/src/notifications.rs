//! Settings ▸ Notifications service (`system-permissions-service`, `notifications-service`
//! and the `/notifications/settings` queries): the system permission, the notification sound
//! files and the ChatGPT notification channels.

use crate::*;
use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use ui::screens::computer_use::Load;
#[cfg(target_os = "macos")]
use ui::screens::settings::notifications::Permission;
use ui::screens::settings::notifications::{self as view, Account, Action, State};

/// The original's import limits (`importSound`): 10 MB and under 30 seconds.
const MAX_BYTES: u64 = 10_485_760;
const MAX_SECONDS: f64 = 30.;
const BUILT_IN: &[(&str, &[u8])] = &[
    (
        "codex-notification.wav",
        include_bytes!("../../../assets/sounds/codex-notification.wav"),
    ),
    (
        "codex-classic.wav",
        include_bytes!("../../../assets/sounds/codex-classic.wav"),
    ),
];

/// The running preview; a new preview stops it first.
#[derive(Default)]
struct Preview(Option<Child>);

impl gpui::Global for Preview {}

pub fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(Preview::default());
    cx.set_global(State {
        // Off macOS the original never reports a permission (`notificationAuthorizationUnavailable`).
        permission: cfg!(target_os = "macos").then_some(Load::Loading),
        dispatch: Some(Arc::new(move |action, cx| {
            root.update(cx, |root, cx| root.notifications_action(action, cx))
                .ok();
        })),
        ..Default::default()
    });
}

/// `~/Library/Application Support/codex-gpui/notification-sounds`, next to `host-settings.json`.
fn sounds_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/codex-gpui/notification-sounds")
}

/// The file a sound setting plays (`Wh`): `None` for `none`.
fn sound_path(setting: &Value, dir: &Path) -> Result<Option<PathBuf>> {
    let file = match setting {
        Value::Object(custom) => {
            let name = custom["fileName"].as_str().context("missing sound file")?;
            ensure!(
                name.starts_with("codex-custom-") && !name.contains(['/', '\\']),
                "invalid sound file"
            );
            return Ok(Some(dir.join(name)));
        }
        Value::String(name) if name == "none" => return Ok(None),
        Value::String(name) if name == "classic" => "codex-classic.wav",
        Value::String(name) if name == "default" => "codex-notification.wav",
        Value::String(name) => {
            ensure!(
                name.chars().all(char::is_alphanumeric),
                "invalid sound name"
            );
            return Ok(Some(PathBuf::from(format!(
                "/System/Library/Sounds/{name}.aiff"
            ))));
        }
        _ => "codex-notification.wav",
    };
    let path = dir.join(file);
    if !path.exists() {
        let (_, bytes) = BUILT_IN
            .iter()
            .find(|(name, _)| *name == file)
            .context("unknown sound")?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(&path, bytes)?;
    }
    Ok(Some(path))
}

/// The length of a PCM WAV file, from its `fmt ` byte rate and `data` size.
fn wav_seconds(bytes: &[u8]) -> Option<f64> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut at, mut byte_rate) = (12, None);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        match id {
            b"fmt " => {
                byte_rate = Some(u32::from_le_bytes(
                    bytes.get(at + 16..at + 20)?.try_into().ok()?,
                ))
            }
            b"data" => return Some(size as f64 / f64::from(byte_rate.filter(|&rate| rate > 0)?)),
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    None
}

/// `afinfo -x -r`'s `<duration units="sec">` (macOS reads MP3, FLAC, AIFF and CAF this way).
fn afinfo_seconds(path: &Path) -> Option<f64> {
    let output = Command::new("/usr/bin/afinfo")
        .args(["-x", "-r"])
        .arg(path)
        .output()
        .ok()?;
    let xml = String::from_utf8_lossy(&output.stdout);
    let start = xml.find("<duration")?;
    let value = &xml[start..];
    value[value.find('>')? + 1..value.find("</duration>")?]
        .trim()
        .parse()
        .ok()
}

/// Copies an accepted sound into `dir` as `codex-custom-<sha256>.<ext>` and returns the setting.
fn import_sound(path: &Path, dir: &Path) -> Result<Value> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let accepted: &[&str] = if cfg!(target_os = "macos") {
        &["mp3", "wav", "flac", "ogg", "oga", "aif", "aiff", "caf"]
    } else {
        &["mp3", "wav", "flac", "ogg", "oga"]
    };
    ensure!(accepted.contains(&ext.as_str()), "unsupported sound type");
    let metadata = std::fs::metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BYTES,
        "sound file too large"
    );
    let bytes = std::fs::read(path)?;
    // shortcut: Ogg and non-WAV files off macOS have no native decoder here, so they are
    // refused; add a decoder when Ogg import is needed.
    let seconds = wav_seconds(&bytes)
        .or_else(|| {
            cfg!(target_os = "macos")
                .then(|| afinfo_seconds(path))
                .flatten()
        })
        .context("unreadable sound")?;
    ensure!(seconds > 0. && seconds < MAX_SECONDS, "sound too long");
    let name = format!("codex-custom-{:x}.{ext}", Sha256::digest(&bytes));
    std::fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".codex-import-{name}"));
    std::fs::write(&staged, &bytes)?;
    if let Err(error) = std::fs::rename(&staged, dir.join(&name)) {
        std::fs::remove_file(&staged).ok();
        return Err(error.into());
    }
    let label = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    Ok(json!({ "fileName": name, "name": label.unwrap_or_else(|| name.clone()) }))
}

fn play(path: &Path) -> Result<Child> {
    let player = if cfg!(target_os = "macos") {
        "/usr/bin/afplay"
    } else {
        "paplay"
    };
    Ok(Command::new(player)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?)
}

#[cfg(target_os = "macos")]
mod system {
    use super::Permission;
    use objc2_foundation::NSBundle;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings,
        UNUserNotificationCenter,
    };
    use std::ptr::NonNull;

    /// `None` outside an app bundle, where `UNUserNotificationCenter` aborts.
    pub fn bundle_id() -> Option<String> {
        NSBundle::mainBundle()
            .bundleIdentifier()
            .map(|id| id.to_string())
    }

    pub async fn permission() -> Option<Permission> {
        bundle_id()?;
        let (tx, rx) = async_channel::bounded(1);
        let block = block2::RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            tx.try_send(status).ok();
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&block);
        Some(match rx.recv().await.ok()? {
            UNAuthorizationStatus::Denied => Permission::Disabled,
            UNAuthorizationStatus::NotDetermined => Permission::NotDetermined,
            _ => Permission::Enabled,
        })
    }

    pub fn request_authorization() {
        if bundle_id().is_none() {
            return;
        }
        let block = block2::RcBlock::new(|_, _| {});
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert
                    | UNAuthorizationOptions::Sound
                    | UNAuthorizationOptions::Badge,
                &block,
            );
    }
}

fn update(cx: &mut App, f: impl FnOnce(&mut State)) {
    cx.update_global::<State, _>(|state, _| f(state));
}

fn chat_notice_enabled(
    kind: &str,
    settings: &serde_json::Map<String, Value>,
    focused: bool,
) -> bool {
    match kind {
        "completion" => match settings
            .get("notifications-turn-mode")
            .and_then(Value::as_str)
        {
            Some("off") => false,
            Some("always") => true,
            _ => !focused,
        },
        "permission" | "question" => {
            !focused
                && settings
                    .get(if kind == "permission" {
                        "notifications-permissions-enabled"
                    } else {
                        "notifications-questions-enabled"
                    })
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
        }
        _ => false,
    }
}

pub(crate) fn destination(tag: &str) -> Option<(String, String)> {
    if let Some(id) = tag.strip_prefix("automation:").filter(|id| !id.is_empty()) {
        return Some(("local".into(), id.into()));
    }
    serde_json::from_str(tag.strip_prefix("chat:")?).ok()
}

impl Root {
    pub(super) fn chat_notification(
        &mut self,
        kind: &'static str,
        id: &str,
        body: &str,
        cx: &mut Context<Self>,
    ) {
        let title = self
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .map(thread_title)
            .unwrap_or_else(|| "Codex".into());
        let notice = gpui::SystemNotification {
            tag: format!("chat:{}", json!([self.host_id, id])).into(),
            title,
            body: body.into(),
            actions: Vec::new(),
            reply_placeholder: None,
        };
        // Read permission again: System Settings can change while this app is in the background.
        cx.spawn(async move |this, cx| {
            #[cfg(target_os = "macos")]
            if matches!(
                system::permission().await,
                Some(Permission::Disabled | Permission::NotDetermined)
            ) {
                return;
            }
            this.update(cx, |this, cx| {
                let focused = this
                    .context_window
                    .update(cx, |_, window, _| window.is_window_active())
                    .unwrap_or(true);
                let sound = {
                    let settings = cx
                        .try_global::<host_settings::Settings>()
                        .map(host_settings::Settings::read_all);
                    let empty = serde_json::Map::new();
                    let settings = settings.as_deref().unwrap_or(&empty);
                    if !chat_notice_enabled(kind, settings, focused) {
                        return;
                    }
                    settings
                        .get("notifications-sound")
                        .cloned()
                        .unwrap_or_default()
                };
                cx.show_system_notification(notice);
                cx.background_spawn(async move {
                    let result = sound_path(&sound, &sounds_dir()).and_then(|path| {
                        if let Some(path) = path {
                            play(&path)?.wait()?;
                        }
                        Ok(())
                    });
                    if let Err(error) = result {
                        eprintln!("codex-gpui: notification sound: {error:#}");
                    }
                })
                .detach();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn approval_notification(
        &mut self,
        pending: &PendingApproval,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = pending.params["threadId"].as_str() else {
            return;
        };
        let (kind, body) = match pending.method.as_str() {
            "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval" => {
                ("permission", "Approval is needed to continue.")
            }
            "item/tool/requestUserInput" | "mcpServer/elicitation/request" => {
                ("question", "A question needs your answer.")
            }
            _ => return,
        };
        self.chat_notification(kind, id, body, cx);
    }

    pub(crate) fn notifications_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::RefreshPermission => self.refresh_notification_permission(cx),
            Action::OpenSystemSettings => self.open_notification_settings(cx),
            Action::PreviewSound(setting) => self.preview_sound(&setting, cx),
            Action::ImportSound => self.choose_sound(cx),
            Action::RefreshAccount => self.refresh_notification_account(cx),
            Action::RefreshChannels => self.notification_get("/notifications/settings", cx),
            Action::RefreshContacts => self.notification_get("/me", cx),
            Action::SetChannel {
                category,
                channel,
                enabled,
            } => self.set_notification_channel(category, channel, enabled, cx),
        }
    }

    fn refresh_notification_permission(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        cx.spawn(async move |_, cx| {
            let permission = system::permission().await;
            cx.update(|cx| update(cx, |state| state.permission = permission.map(Load::Ready)));
        })
        .detach();
        #[cfg(not(target_os = "macos"))]
        let _ = cx;
    }

    /// `openNotificationSettings`: asks first when macOS has never asked, then opens the pane.
    fn open_notification_settings(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        {
            let Some(bundle_id) = system::bundle_id() else {
                return;
            };
            let undetermined = !matches!(
                cx.global::<State>().permission,
                Some(Load::Ready(Permission::Enabled | Permission::Disabled))
            );
            update(cx, |state| state.opening_settings = true);
            cx.spawn(async move |this, cx| {
                if undetermined {
                    system::request_authorization();
                    let wait = std::time::Duration::from_secs(2);
                    cx.background_executor().timer(wait).await;
                }
                this.update(cx, |this, cx| {
                    update(cx, |state| state.opening_settings = false);
                    cx.open_url(&format!(
                        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={bundle_id}"
                    ));
                    this.refresh_notification_permission(cx);
                })
                .ok();
            })
            .detach();
        }
        #[cfg(target_os = "windows")]
        cx.open_url("ms-settings:notifications");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        self.toast(Toast::danger("Couldn't open System Settings"), cx);
    }

    fn preview_sound(&mut self, setting: &Value, cx: &mut Context<Self>) {
        if let Some(mut child) = cx.global_mut::<Preview>().0.take() {
            child.kill().ok();
            child.wait().ok();
        }
        let played = sound_path(setting, &sounds_dir())
            .and_then(|path| path.map(|path| play(&path)).transpose());
        match played {
            Ok(child) => cx.global_mut::<Preview>().0 = child,
            Err(error) => {
                eprintln!("codex-gpui: notification sound preview: {error:#}");
                self.toast(Toast::danger("Couldn't play the sound preview"), cx);
            }
        }
    }

    fn choose_sound(&mut self, cx: &mut Context<Self>) {
        if cx.global::<State>().importing_sound {
            return;
        }
        update(cx, |state| state.importing_sound = true);
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose sound".into()),
        });
        cx.spawn(async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let imported = match path {
                Some(path) => Some(
                    cx.background_executor()
                        .spawn(async move { import_sound(&path, &sounds_dir()) })
                        .await,
                ),
                None => None,
            };
            this.update(cx, |this, cx| {
                update(cx, |state| state.importing_sound = false);
                match imported {
                    None => {}
                    Some(Err(error)) => {
                        eprintln!("codex-gpui: notification sound import: {error:#}");
                        this.toast(
                            Toast::danger(
                                "Couldn't import this sound. Choose an MP3, WAV, FLAC, or Ogg file under 30 seconds and 10 MB",
                            ),
                            cx,
                        );
                    }
                    Some(Ok(setting)) => {
                        if let Some(page) = this.settings_page.clone() {
                            page.update(cx, |page, cx| page.set(view::SOUND, setting.clone(), cx));
                        }
                        this.preview_sound(&setting, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// `oNc`: the ChatGPT group shows only for a personal ChatGPT account.
    fn refresh_notification_account(&mut self, cx: &mut Context<Self>) {
        let Some(identity) = self.host.identity() else {
            update(cx, |state| state.account = Account::Hidden);
            return;
        };
        update(cx, |state| {
            if state.account != Account::Personal {
                state.account = Account::Loading;
            }
            state.fetching = true;
        });
        let host = self.host.clone();
        let task = cx.background_executor().spawn(async move {
            let accounts = usage::http(
                host.as_ref(),
                &identity,
                "GET",
                "/accounts/check/v4-2023-04-27",
                None,
            )?;
            Ok::<_, anyhow::Error>(
                accounts["accounts"][&identity.account_id]["account"]["structure"] == "personal",
            )
        });
        cx.spawn(async move |this, cx| {
            let personal = task.await;
            this.update(cx, |this, cx| {
                update(cx, |state| {
                    state.fetching = false;
                    state.account = match &personal {
                        Ok(true) => Account::Personal,
                        Ok(false) => Account::Hidden,
                        Err(_) => Account::Error,
                    };
                });
                if matches!(personal, Ok(true)) {
                    this.notification_get("/notifications/settings", cx);
                    this.notification_get("/me", cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Loads the channel settings or the contact methods (`/me`).
    fn notification_get(&mut self, path: &'static str, cx: &mut Context<Self>) {
        let Some(identity) = self.host.identity() else {
            return;
        };
        let set = move |state: &mut State, value: Load<Arc<Value>>| match path {
            "/me" => state.contacts = value,
            _ => state.channels = value,
        };
        update(cx, |state| {
            state.fetching = true;
            set(state, Load::Loading);
        });
        let host = self.host.clone();
        let task = cx
            .background_executor()
            .spawn(async move { usage::http(host.as_ref(), &identity, "GET", path, None) });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            cx.update(|cx| {
                update(cx, |state| {
                    state.fetching = false;
                    set(
                        state,
                        match result {
                            Ok(value) => Load::Ready(Arc::new(value)),
                            Err(error) => Load::Failed(error.to_string()),
                        },
                    );
                })
            });
        })
        .detach();
    }

    /// `Xe`/`Ze`: shows the change at once, then `PATCH`es it; a failure undoes that option.
    fn set_notification_channel(
        &mut self,
        category: String,
        channel: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(identity) = self.host.identity() else {
            return;
        };
        if !set_channel(cx, &category, &channel, enabled) {
            return;
        }
        let host = self.host.clone();
        let body = json!({ "updates": { &category: { &channel: enabled } } });
        let task = cx.background_executor().spawn(async move {
            usage::http(
                host.as_ref(),
                &identity,
                "PATCH",
                "/notifications/settings",
                Some(body),
            )
        });
        cx.spawn(async move |this, cx| {
            if task.await.is_ok() {
                return;
            }
            this.update(cx, |this, cx| {
                // Only this option: another update may have changed the rest since.
                set_channel(cx, &category, &channel, !enabled);
                this.toast(
                    Toast::danger("Failed to update ChatGPT notification settings"),
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }
}

/// Sets one channel of the loaded settings; `false` when none are loaded.
fn set_channel(cx: &mut App, category: &str, channel: &str, enabled: bool) -> bool {
    let Load::Ready(settings) = &cx.global::<State>().channels else {
        return false;
    };
    let mut settings = (**settings).clone();
    for setting in settings["settings"].as_array_mut().into_iter().flatten() {
        if setting["category"] != category {
            continue;
        }
        for option in setting["options"].as_array_mut().into_iter().flatten() {
            if option["channel"] == channel {
                option["enabled"] = enabled.into();
            }
        }
    }
    update(cx, |state| state.channels = Load::Ready(Arc::new(settings)));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_preferences_gate_each_notification_kind() {
        let mut settings = serde_json::Map::new();
        for kind in ["completion", "permission", "question"] {
            assert!(chat_notice_enabled(kind, &settings, false));
            assert!(!chat_notice_enabled(kind, &settings, true));
        }
        settings.insert("notifications-turn-mode".into(), json!("always"));
        assert!(chat_notice_enabled("completion", &settings, true));
        settings.insert("notifications-turn-mode".into(), json!("off"));
        assert!(!chat_notice_enabled("completion", &settings, false));
        for (kind, key) in [
            ("permission", "notifications-permissions-enabled"),
            ("question", "notifications-questions-enabled"),
        ] {
            settings.insert(key.into(), json!(false));
            assert!(!chat_notice_enabled(kind, &settings, false));
            settings.insert(key.into(), json!(true));
            assert!(chat_notice_enabled(kind, &settings, false));
        }
    }

    fn wav(seconds: u32) -> Vec<u8> {
        let (rate, block) = (8_000u32, 2u16);
        let data = rate * u32::from(block) * seconds;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * u32::from(block)).to_le_bytes());
        bytes.extend(block.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data.to_le_bytes());
        bytes.resize(bytes.len() + data as usize, 0);
        bytes
    }

    #[test]
    fn import_checks_type_length_and_names_the_copy_by_content() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("Chime.WAV");
        std::fs::write(&source, wav(2))?;
        let sounds = dir.path().join("sounds");
        let setting = import_sound(&source, &sounds)?;
        let file = setting["fileName"].as_str().context("file name")?;
        assert!(file.starts_with("codex-custom-") && file.ends_with(".wav"));
        assert_eq!(setting["name"], "Chime.WAV");
        assert_eq!(sound_path(&setting, &sounds)?, Some(sounds.join(file)));
        assert!(sounds.join(file).is_file());

        let long = dir.path().join("long.wav");
        std::fs::write(&long, wav(30))?;
        assert!(import_sound(&long, &sounds).is_err());
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, b"hi")?;
        assert!(import_sound(&text, &sounds).is_err());
        Ok(())
    }

    #[test]
    fn sound_settings_resolve_to_files_and_reject_paths() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert_eq!(sound_path(&json!("none"), dir.path())?, None);
        let default = sound_path(&json!("default"), dir.path())?.context("default")?;
        assert_eq!(
            wav_seconds(&std::fs::read(&default)?).map(|s| s > 0.),
            Some(true)
        );
        assert_eq!(
            sound_path(&json!("Glass"), dir.path())?,
            Some(PathBuf::from("/System/Library/Sounds/Glass.aiff"))
        );
        assert!(sound_path(&json!({ "fileName": "../x.wav" }), dir.path()).is_err());
        Ok(())
    }
}
