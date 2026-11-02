//! Appshots (the original's `appshot-hotkeys` service): pressing both keys of the chosen
//! modifier captures the frontmost window into a chat. Settings ▸ Appshots stores the
//! choices in host settings under the original's keys.

use super::{Shortcut, begin};
use crate::{Root, windows};
use anyhow::{Context as _, Result, ensure};
use gpui::{App, WindowHandle};
use serde_json::Value;
use std::{process::Command, time::Duration};

pub const HOTKEY: &str = "appshotHotkey";
const DESTINATION: &str = "appshotDestination";
const MINI: &str = "appshotAlwaysSendToMini";
const SOUND: &str = "appshotSoundEnabled";
/// `YZe`: how recently the primary window was active for `automatic` to use its chat.
const RECENT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Runtime {
    _monitor: Option<crate::hotkey::desktop::Monitor>,
    _listener: Option<gpui::Task<()>>,
}
impl gpui::Global for Runtime {}

fn setting(key: &str, cx: &App) -> Value {
    cx.try_global::<crate::host_settings::Settings>()
        .and_then(|settings| settings.read_all().get(key).cloned())
        .unwrap_or(Value::Null)
}

/// The `NX_DEVICE*KEYMASK` pair of a hotkey value; `None` for no hotkey.
fn device_mask(hotkey: &str) -> Option<usize> {
    match hotkey {
        "DoubleCommand" => Some(0x08 | 0x10),
        "DoubleOption" => Some(0x20 | 0x40),
        "DoubleShift" => Some(0x02 | 0x04),
        _ => None,
    }
}

/// Installs the stored hotkey (default `DoubleCommand`, as on macOS in the original),
/// replacing the previous one.
pub fn register(cx: &mut App) -> Result<()> {
    if crate::qa::ENABLED {
        return Ok(());
    }
    cx.set_global(Runtime::default());
    let hotkey = setting(HOTKEY, cx);
    let Some(mask) = device_mask(hotkey.as_str().unwrap_or("DoubleCommand")) else {
        return Ok(());
    };
    let (sender, receiver) = async_channel::bounded(1);
    let monitor = crate::hotkey::desktop::Monitor::both(mask, move || {
        let _ = sender.try_send(());
    })?;
    let listener = cx.spawn(async move |cx| {
        while receiver.recv().await.is_ok() {
            cx.update(take);
        }
    });
    cx.set_global(Runtime {
        _monitor: Some(monitor),
        _listener: Some(listener),
    });
    Ok(())
}

/// The original's `#r`: Mini first when preferred (or when `automatic` and the primary
/// window was not used recently), else the primary window's current or a new chat.
fn take(cx: &mut App) {
    let window = crate::hotkey::desktop::frontmost_window();
    let destination = setting(DESTINATION, cx);
    let destination = destination.as_str().unwrap_or("automatic");
    let recent = windows::primary_active_within(RECENT, cx);
    if (setting(MINI, cx) == true || destination == "automatic" && !recent)
        && let Some(mini) = crate::hotkey::visible_mini(cx)
    {
        // In Mini an appshot always starts a new chat.
        return start(mini, Shortcut::ScreenshotNew, window, cx);
    }
    let current = match destination {
        "last-chat" => true,
        "new-chat" => false,
        _ => recent,
    };
    match windows::primary_or_open(cx) {
        Ok(owner) => start(
            owner,
            if current {
                Shortcut::ScreenshotCurrent
            } else {
                Shortcut::ScreenshotNew
            },
            window,
            cx,
        ),
        Err(error) => eprintln!("codex-gpui: cannot open appshot destination: {error:#}"),
    }
}

fn start(owner: WindowHandle<Root>, shortcut: Shortcut, window: Option<u32>, cx: &mut App) {
    if cx.global::<super::Runtime>().capturing {
        return;
    }
    let sound = setting(SOUND, cx) != false;
    if let Err(error) = begin(owner, shortcut, move || capture(window, sound), cx) {
        eprintln!("codex-gpui: cannot open appshot destination: {error:#}");
    }
}

/// Captures one window without its shadow; with sound on, `screencapture` plays the
/// system capture sound.
fn capture(window: Option<u32>, sound: bool) -> Result<Option<Vec<u8>>> {
    let window = window.context("There is no frontmost window to capture")?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("appshot.png");
    let mut command = Command::new("/usr/sbin/screencapture");
    command
        .args(["-o", "-t", "png", "-l"])
        .arg(window.to_string());
    if !sound {
        command.arg("-x");
    }
    let output = command.arg(&path).output()?;
    ensure!(
        output.status.success(),
        "Appshot capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(Some(std::fs::read(path)?))
}
