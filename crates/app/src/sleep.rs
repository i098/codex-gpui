//! "Prevent sleep while running": a power assertion held while any local thread runs and
//! `desktop.preventSleepWhileRunning` is on (the original's `powerSaveBlocker`).

use gpui::WindowId;
use std::process::Child;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;
use std::sync::Mutex;

static STATE: Mutex<(Vec<WindowId>, Option<Child>)> = Mutex::new((Vec::new(), None));

/// Records whether window `owner` wants the machine kept awake. The assertion is held
/// while any window wants it, and released with the last one.
pub(crate) fn set(owner: WindowId, wanted: bool) {
    let Ok(mut state) = STATE.lock() else { return };
    state.0.retain(|id| *id != owner);
    if wanted {
        state.0.push(owner);
    }
    if state
        .1
        .as_mut()
        .is_some_and(|child| child.try_wait().ok().flatten().is_some())
    {
        state.1 = None;
    }
    if state.0.is_empty() {
        if let Some(mut child) = state.1.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    } else if state.1.is_none() {
        match assertion() {
            Ok(child) => state.1 = Some(child),
            Err(error) => eprintln!("codex-gpui: cannot prevent sleep: {error:#}"),
        }
    }
}

/// A child that holds the assertion and exits by itself when this process dies.
fn assertion() -> std::io::Result<Child> {
    let pid = std::process::id().to_string();
    #[cfg(target_os = "macos")]
    return Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &pid])
        .spawn();
    #[cfg(target_os = "linux")]
    return Command::new("systemd-inhibit")
        .args([
            "--what=sleep:idle",
            "--who=Codex",
            "--why=A task is running",
            "tail",
            "-f",
            "/dev/null",
            "--pid",
            &pid,
        ])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        Err(std::io::Error::other("not available on this platform"))
    }
}

impl crate::Root {
    /// Re-evaluates the assertion from the setting and the running threads.
    pub(crate) fn sync_prevent_sleep(&self, cx: &gpui::App) {
        let enabled = self.config["desktop"]["preventSleepWhileRunning"] == true;
        let running = self.sidebar.read(cx).any_running();
        set(self.context_window.window_id(), enabled && running);
    }
}

impl Drop for crate::Root {
    fn drop(&mut self) {
        set(self.context_window.window_id(), false);
    }
}
