use crate::{Root, Route, windows};
use anyhow::{Context as _, Result, ensure};
use gpui::{App, AppContext as _, AsyncApp, Entity, Focusable as _, Global, WindowHandle, actions};
use std::{collections::BTreeMap, path::Path, process::Command};

mod platform;
use platform::{Native, Shortcut};

actions!(screenshot, [ScreenshotCurrent, ScreenshotNew]);

#[derive(Default)]
struct Runtime {
    native: Option<Native>,
    listener: Option<gpui::Task<()>>,
    bindings: Vec<platform::Binding>,
    sender: Option<async_channel::Sender<Shortcut>>,
    capturing: bool,
    waiting: Option<(gpui::WindowId, gpui::Subscription)>,
}
impl Global for Runtime {}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(Runtime::default());
    cx.on_action(|_: &ScreenshotCurrent, cx| capture(Shortcut::ScreenshotCurrent, cx));
    cx.on_action(|_: &ScreenshotNew, cx| capture(Shortcut::ScreenshotNew, cx));
    cx.on_window_closed(|cx, id| {
        let runtime = cx.global_mut::<Runtime>();
        if runtime
            .waiting
            .as_ref()
            .is_some_and(|(owner, _)| *owner == id)
        {
            runtime.waiting = None;
            runtime.capturing = false;
        }
    })
    .detach();
}

pub fn replace_global_bindings(
    overrides: &BTreeMap<String, Vec<String>>,
    cx: &mut App,
) -> Result<()> {
    let mut bindings = Vec::new();
    for (id, shortcut, default) in [
        (
            "screenshotToCurrentChat",
            Shortcut::ScreenshotCurrent,
            "Control+Shift+4",
        ),
        (
            "screenshotToNewChat",
            Shortcut::ScreenshotNew,
            "Control+Alt+Shift+4",
        ),
    ] {
        if let Some(values) = overrides.get(id) {
            for value in values {
                bindings.push(platform::binding(shortcut, value)?);
            }
        } else {
            bindings.push(platform::binding(shortcut, default)?);
        }
    }
    let (sender, receiver) = async_channel::unbounded();
    let runtime = cx.global_mut::<Runtime>();
    drop(runtime.native.take());
    let native = match Native::new(&bindings, sender.clone()) {
        Ok(native) => native,
        Err(error) => {
            if let Some(previous) = runtime.sender.clone() {
                runtime.native = Some(
                    Native::new(&runtime.bindings, previous)
                        .context("restore screenshot shortcuts")?,
                );
            }
            return Err(error);
        }
    };
    let listener = cx.spawn(async move |cx| {
        while let Ok(shortcut) = receiver.recv().await {
            cx.update(|cx| capture(shortcut, cx));
        }
    });
    let runtime = cx.global_mut::<Runtime>();
    runtime.native = Some(native);
    runtime.listener = Some(listener);
    runtime.bindings = bindings;
    runtime.sender = Some(sender);
    Ok(())
}

pub fn register(cx: &mut App) {
    if crate::qa::ENABLED || !cfg!(target_os = "macos") {
        return;
    }
    let overrides = cx
        .global::<crate::host_settings::Settings>()
        .read_all()
        .get("codex-command-keybindings")
        .cloned();
    let result = overrides
        .map(serde_json::from_value)
        .transpose()
        .context("read screenshot shortcuts")
        .and_then(|values| replace_global_bindings(&values.unwrap_or_default(), cx));
    if let Err(error) = result {
        eprintln!("codex-gpui: cannot register screenshot shortcuts: {error}");
    }
}

fn capture(destination: Shortcut, cx: &mut App) {
    capture_with(destination, capture_selection, cx);
}

fn capture_with(
    destination: Shortcut,
    selection: impl FnOnce() -> Result<Option<Vec<u8>>> + Send + 'static,
    cx: &mut App,
) {
    if cx.global::<Runtime>().capturing {
        return;
    }
    if let Err(error) =
        windows::primary(cx).and_then(|owner| begin(owner, destination, selection, cx))
    {
        eprintln!("codex-gpui: cannot open screenshot destination: {error:#}");
    }
}

fn begin(
    owner: WindowHandle<Root>,
    destination: Shortcut,
    selection: impl FnOnce() -> Result<Option<Vec<u8>>> + Send + 'static,
    cx: &mut App,
) -> Result<()> {
    let root = owner.update(cx, |_, _, cx| cx.entity())?;
    if !root.read(cx).ready {
        let mut selection = Some(selection);
        let subscription = cx.observe(&root, move |root, cx| {
            if root.read(cx).ready
                && let Some(selection) = selection.take()
            {
                let runtime = cx.global_mut::<Runtime>();
                runtime.waiting = None;
                runtime.capturing = false;
                if let Err(error) = begin(owner, destination, selection, cx) {
                    eprintln!("codex-gpui: cannot open screenshot destination: {error:#}");
                }
            }
        });
        let runtime = cx.global_mut::<Runtime>();
        runtime.capturing = true;
        runtime.waiting = Some((owner.window_id(), subscription));
        return Ok(());
    }
    let Some(pending) = Pending::new(owner, destination, cx)? else {
        return Ok(());
    };
    cx.global_mut::<Runtime>().capturing = true;
    let selection = cx.background_spawn(async move { selection() });
    cx.spawn(async move |cx| {
        let result = selection.await;
        cx.update(|cx| cx.global_mut::<Runtime>().capturing = false);
        pending.complete(result, cx);
    })
    .detach();
    Ok(())
}

struct Pending {
    owner: WindowHandle<Root>,
    composer: Entity<ui::composer::Composer>,
    generation: u64,
    navigation: u64,
}

impl Pending {
    fn new(owner: WindowHandle<Root>, destination: Shortcut, cx: &mut App) -> Result<Option<Self>> {
        owner.update(cx, |root, window, cx| {
            if root.login.is_some() || root.logout_pending {
                return None;
            }
            if destination == Shortcut::ScreenshotNew
                || !matches!(root.state.route, Route::Home | Route::Thread(_))
                || (matches!(root.state.route, Route::Thread(_)) && root.thread.is_none())
            {
                root.new_chat(window, cx);
            }
            let composer = root.composer(cx);
            let generation = composer.read(cx).attachment_generation();
            Some(Self {
                owner,
                composer,
                generation,
                navigation: root.navigation_epoch,
            })
        })
    }

    fn is_current(&self, root: &Root, cx: &App) -> bool {
        root.navigation_epoch == self.navigation
            && root.login.is_none()
            && !root.logout_pending
            && root.composer(cx) == self.composer
            && self.composer.read(cx).attachment_generation() == self.generation
    }

    fn complete(self, result: Result<Option<Vec<u8>>>, cx: &mut AsyncApp) {
        let accepted = self.owner.update(cx, |root, window, cx| {
            if !self.is_current(root, cx) {
                return false;
            }
            match result {
                Ok(Some(bytes)) => {
                    self.composer
                        .update(cx, |composer, cx| composer.attach_png(bytes, cx));
                    window.focus(&self.composer.focus_handle(cx), cx);
                    true
                }
                Ok(None) => false,
                Err(error) => {
                    root.toast(
                        ui::toast::Toast::danger("Failed to attach screenshot")
                            .description(error.to_string()),
                        cx,
                    );
                    false
                }
            }
        });
        if matches!(accepted, Ok(true)) {
            cx.update(|cx| {
                if let Err(error) =
                    windows::show(self.owner.into(), windows::Presentation::Foreground, cx)
                {
                    eprintln!("codex-gpui: cannot show screenshot destination: {error:#}");
                }
            });
        }
    }
}

fn capture_selection() -> Result<Option<Vec<u8>>> {
    ensure!(
        cfg!(target_os = "macos"),
        "Screenshot selection requires macOS"
    );
    let directory = tempfile::tempdir()?;
    run_capture(Path::new("/usr/sbin/screencapture"), directory.path()).map_err(Into::into)
}

fn run_capture(program: &Path, directory: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let path = directory.join("screenshot.png");
    let output = Command::new(program)
        .args(["-i", "-s", "-x", "-t", "png"])
        .arg(&path)
        .output()?;
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "Screenshot selection failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
