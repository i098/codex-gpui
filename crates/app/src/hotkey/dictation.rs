use super::{
    Runtime, audio, desktop,
    platform::{Shortcut, ShortcutEvent},
};
use crate::{Root, host, windows};
use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, Bounds, ClipboardItem, Context, Entity, Task, WindowHandle};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use ui::screens::{
    dictation::{Action, Phase, Recovery, View},
    settings::voice::{Policy, Recording},
};

struct Session {
    id: String,
    started: Instant,
    clipboard: Option<ClipboardItem>,
    clipboard_stamp: isize,
    application: Option<i32>,
}

pub(super) struct Controller {
    view: Entity<View>,
    window: WindowHandle<View>,
    owner: WindowHandle<Root>,
    host: Arc<dyn host::Host>,
    recorder: Option<audio::Recorder>,
    session: Option<Session>,
    last_id: Option<String>,
    latched: bool,
    last_press: Option<Instant>,
    region: Option<Bounds<gpui::Pixels>>,
    pointer_enabled: bool,
    operation: Option<Task<()>>,
    _pointer: desktop::Monitor,
    _pointer_task: Task<()>,
}

fn hint(cx: &App) -> String {
    use ui::screens::settings::keyboard_shortcuts::{KeyboardState, key_label};
    let Some(state) = cx.try_global::<KeyboardState>() else {
        return String::new();
    };
    let first = |id: &str| {
        state
            .preferences
            .bindings
            .get(id)
            .and_then(|keys| keys.first())
            .map(|key| key_label(key))
    };
    match (
        first("globalDictationHold"),
        first("globalDictationSingleTap"),
    ) {
        (Some(hold), Some(toggle)) => {
            format!("Hold or double-tap {hold}, or press {toggle} to dictate")
        }
        (Some(hold), None) => format!("Hold or double-tap {hold} to dictate"),
        (None, Some(toggle)) => format!("Press {toggle} to dictate"),
        _ => String::new(),
    }
}

fn ensure(cx: &mut App) -> Result<Entity<Controller>> {
    if let Some(controller) = &cx.global::<Runtime>().dictation {
        return Ok(controller.clone());
    }
    let owner = cx
        .global::<Runtime>()
        .primary
        .context("No primary window for dictation")?;
    let host = owner.update(cx, |root, _, _| root.host.clone())?;
    let shortcut = hint(cx);
    let mut spec = windows::Spec::new(windows::Kind::GlobalDictation, "Dictation");
    spec.presentation = windows::Presentation::Background;
    spec.owner = Some(owner.into());
    let window = windows::open(spec, cx, |_, cx| cx.new(|cx| View::new(shortcut, cx)))?;
    let view = window.update(cx, |_, _, cx| cx.entity())?;
    let (pointer, events) = desktop::Monitor::pointer()?;
    let controller = cx.new(|cx| {
        cx.subscribe(&view, |this: &mut Controller, _, action: &Action, cx| {
            this.action(*action, cx)
        })
        .detach();
        let pointer_task = cx.spawn(async move |this, cx| {
            while events.recv().await.is_ok() {
                if this.update(cx, |this, cx| this.pointer(cx)).is_err() {
                    break;
                }
            }
        });
        Controller {
            view,
            window,
            owner,
            host,
            recorder: None,
            session: None,
            last_id: None,
            latched: false,
            last_press: None,
            region: None,
            pointer_enabled: false,
            operation: None,
            _pointer: pointer,
            _pointer_task: pointer_task,
        }
    });
    cx.global_mut::<Runtime>().dictation = Some(controller.clone());
    Ok(controller)
}

pub(super) fn registration_changed(cx: &mut App) -> Result<()> {
    let runtime = cx.global::<Runtime>();
    let enabled = runtime.native.is_some()
        && runtime.bindings.iter().any(|binding| {
            matches!(
                binding.0,
                Shortcut::DictationHold | Shortcut::DictationToggle
            )
        });
    if enabled {
        let controller = ensure(cx)?;
        controller.update(cx, |controller, cx| {
            let shortcut = hint(cx);
            controller
                .view
                .update(cx, |view, cx| view.set_shortcut(shortcut, cx));
            let _ = windows::show(
                controller.window.into(),
                windows::Presentation::Background,
                cx,
            );
        });
    } else if let Some(controller) = &cx.global::<Runtime>().dictation {
        controller
            .clone()
            .update(cx, |controller, cx| controller.dismiss(cx));
    }
    Ok(())
}

pub(super) fn event(event: ShortcutEvent, cx: &mut App) {
    if !cx
        .try_global::<Policy>()
        .is_some_and(|policy| policy.global_dictation)
    {
        return;
    }
    match ensure(cx) {
        Ok(controller) => controller.update(cx, |controller, cx| controller.key(event, cx)),
        Err(error) => {
            if let Some(owner) = cx.global::<Runtime>().primary {
                let _ = owner.update(cx, |root, _, cx| {
                    root.toast(
                        ui::toast::Toast::danger("Could not start dictation")
                            .description(error.to_string()),
                        cx,
                    )
                });
            }
        }
    }
}

impl Controller {
    fn phase(&mut self, phase: Phase, cx: &mut Context<Self>) {
        self.view.update(cx, |view, cx| view.set_phase(phase, cx));
        let _ = windows::show(self.window.into(), windows::Presentation::Background, cx);
        self.pointer(cx);
    }

    fn pointer(&mut self, cx: &mut Context<Self>) {
        let passive = matches!(
            self.view.read(cx).phase,
            Phase::Initializing | Phase::Listening | Phase::Transcribing
        );
        let interactive = if passive {
            false
        } else if let Some(region) = self.region {
            self.window
                .update(cx, |_, window, _| desktop::pointer_inside(window, region))
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false)
        } else {
            false
        };
        if interactive != self.pointer_enabled
            && windows::set_pointer_interactive(self.window.into(), interactive, cx).is_ok()
        {
            self.pointer_enabled = interactive;
        }
    }

    fn key(&mut self, event: ShortcutEvent, cx: &mut Context<Self>) {
        if event.shortcut == Shortcut::DictationToggle {
            if event.pressed {
                if self.session.is_some() {
                    self.stop(false, false, cx);
                } else {
                    self.latched = true;
                    self.start(cx);
                }
            }
            return;
        }
        if event.pressed {
            let now = Instant::now();
            let double = self
                .last_press
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(250));
            self.last_press = Some(now);
            if self.session.is_none() {
                self.latched = double;
                self.start(cx);
            } else if self.latched {
                self.stop(false, false, cx);
            } else if double {
                self.latched = true;
            }
        } else if !self.latched {
            self.stop(false, false, cx);
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.session.is_some() {
            return;
        }
        let id = host::device_id();
        self.session = Some(Session {
            id: id.clone(),
            started: Instant::now(),
            clipboard: cx.read_from_clipboard(),
            clipboard_stamp: desktop::clipboard_stamp(),
            application: desktop::frontmost_application(),
        });
        self.phase(Phase::Initializing, cx);
        let host = self.host.clone();
        self.operation = Some(cx.spawn(async move |this, cx| {
            if let Err(error) = audio::request_access().await {
                let _ = this.update(cx, |this, cx| {
                    this.failed(&id, error.to_string(), false, cx)
                });
                return;
            }
            if !this
                .update(cx, |this, _| this.matches(&id))
                .unwrap_or(false)
            {
                return;
            }
            let recording_id = id.clone();
            let prepare_host = host.clone();
            let prepared = cx
                .background_spawn(async move {
                    let settings = prepare_host.voice_settings()?;
                    let values = settings.values()?;
                    let path = settings.begin_capture(&Recording {
                        id: recording_id,
                        created_at_ms: chrono::Utc::now().timestamp_millis(),
                        mime_type: "audio/wav".into(),
                        size_bytes: 0,
                        status: "recording".into(),
                        text: String::new(),
                        language: None,
                    })?;
                    Ok::<_, anyhow::Error>((
                        path,
                        values["microphoneInputDeviceId"]
                            .as_str()
                            .map(str::to_owned),
                    ))
                })
                .await;
            match prepared {
                Ok((path, device)) => {
                    let attached = this
                        .update(cx, |this, cx| {
                            if !this.matches(&id) {
                                return false;
                            }
                            match audio::Recorder::start(&path, device.as_deref()) {
                                Ok(recorder) => {
                                    this.recorder = Some(recorder);
                                    this.session.as_mut().unwrap().started = Instant::now();
                                    this.last_id = Some(id.clone());
                                    this.phase(Phase::Listening, cx);
                                    this.meter(id.clone(), cx);
                                    true
                                }
                                Err(error) => {
                                    this.failed(&id, error.to_string(), false, cx);
                                    false
                                }
                            }
                        })
                        .unwrap_or(false);
                    if !attached {
                        let _ = cx
                            .background_spawn(async move {
                                host.voice_settings()?.finish_capture(&id, "cancelled")
                            })
                            .await;
                    }
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.failed(&id, error.to_string(), false, cx)
                    });
                }
            }
        }));
    }

    fn matches(&self, id: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.id == id)
    }

    fn meter(&mut self, id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if !this.matches(&id) {
                            return false;
                        }
                        let Some(recorder) = &this.recorder else {
                            return false;
                        };
                        if recorder.error().is_some() {
                            this.stop(false, true, cx);
                            return false;
                        }
                        let level = recorder.level();
                        this.view.update(cx, |view, cx| view.push_level(level, cx));
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn stop(&mut self, cancel: bool, recovery: bool, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        let id = session.id.clone();
        let too_short = session.started.elapsed() < Duration::from_millis(250);
        let Some(recorder) = self.recorder.take() else {
            self.operation = None;
            self.session = None;
            self.phase(Phase::Idle, cx);
            return;
        };
        let result = recorder.stop();
        let empty = result.as_ref().is_ok_and(|bytes| *bytes == 0);
        if cancel || too_short || empty {
            self.session = None;
            self.phase(Phase::Idle, cx);
            let host = self.host.clone();
            cx.background_spawn(async move {
                let _ = host
                    .voice_settings()
                    .and_then(|settings| settings.finish_capture(&id, "cancelled"));
            })
            .detach();
        } else {
            self.last_id = Some(id.clone());
            self.transcribe(id, recovery || result.is_err(), cx);
        }
    }

    fn transcribe(&mut self, id: String, recovery: bool, cx: &mut Context<Self>) {
        self.phase(
            if recovery {
                Phase::Recovery {
                    state: Recovery::Recovering,
                    saved: true,
                }
            } else {
                Phase::Transcribing
            },
            cx,
        );
        let host = self.host.clone();
        self.operation = Some(cx.spawn(async move |this, cx| {
            let task_id = id.clone();
            let result = cx
                .background_spawn(async move {
                    let settings = host.voice_settings()?;
                    settings.finish_capture(
                        &task_id,
                        if recovery { "interrupted" } else { "completed" },
                    )?;
                    let (recording, audio) = settings.audio(&task_id)?;
                    let identity = host
                        .identity()
                        .context("No authenticated workspace is available")?;
                    let request = crate::voice::transcription_request(
                        &identity,
                        &recording.mime_type,
                        recording.language.as_deref(),
                        &audio,
                    )?;
                    let response = host::json_body(&host.fetch(&request)?)?;
                    let text = response["text"]
                        .as_str()
                        .context("Invalid transcription response")?
                        .trim()
                        .to_owned();
                    if !text.is_empty() {
                        settings.set_transcript(&task_id, &text)?;
                    }
                    Ok::<_, anyhow::Error>(text)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.matches(&id) {
                    return;
                }
                match result {
                    Ok(text) if recovery => {
                        this.session = None;
                        this.phase(
                            Phase::Recovery {
                                state: if text.is_empty() {
                                    Recovery::Failed
                                } else {
                                    Recovery::Recovered(text)
                                },
                                saved: true,
                            },
                            cx,
                        );
                    }
                    Ok(text) => this.insert(text, cx),
                    Err(error) => {
                        this.session = None;
                        this.phase(
                            if recovery {
                                Phase::Recovery {
                                    state: Recovery::Failed,
                                    saved: true,
                                }
                            } else {
                                Phase::Error {
                                    message: error.to_string(),
                                    retry: true,
                                }
                            },
                            cx,
                        );
                    }
                }
            });
        }));
    }

    fn insert(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.take() else {
            return;
        };
        if text.is_empty() {
            self.phase(Phase::Idle, cx);
            return;
        }
        if desktop::clipboard_stamp() != session.clipboard_stamp {
            self.phase(
                Phase::PasteFailed {
                    text,
                    copied: false,
                    accessibility: false,
                },
                cx,
            );
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        let same_application = session.application.is_some()
            && session.application == desktop::frontmost_application();
        if !same_application {
            self.phase(
                Phase::PasteFailed {
                    text,
                    copied: true,
                    accessibility: false,
                },
                cx,
            );
            return;
        }
        match desktop::paste() {
            Ok(true) => {
                let stamp = desktop::clipboard_stamp();
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    cx.update(|cx| {
                        if desktop::clipboard_stamp() == stamp {
                            match session.clipboard {
                                Some(item) => cx.write_to_clipboard(item),
                                None => desktop::clear_clipboard(),
                            }
                        }
                    });
                })
                .detach();
                self.phase(Phase::Idle, cx);
            }
            result => self.phase(
                Phase::PasteFailed {
                    text,
                    copied: true,
                    accessibility: matches!(result, Ok(false)),
                },
                cx,
            ),
        }
    }

    fn failed(&mut self, id: &str, message: String, retry: bool, cx: &mut Context<Self>) {
        if self.matches(id) {
            self.session = None;
            self.phase(Phase::Error { message, retry }, cx);
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.stop(true, false, cx);
        self.session = None;
        self.operation = None;
        self.view
            .update(cx, |view, cx| view.set_phase(Phase::Idle, cx));
        let _ = windows::show(self.window.into(), windows::Presentation::Hidden, cx);
    }

    fn action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Region(region) => {
                self.region = Some(region);
                let controller = cx.entity();
                cx.defer(move |cx| controller.update(cx, Self::pointer));
            }
            Action::Dismiss => self.dismiss(cx),
            Action::AccessibilitySettings => cx.open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            ),
            Action::Recordings => {
                let _ = self.owner.update(cx, |root, window, cx| {
                    root.open_settings_section("Voice", window, cx)
                });
            }
            Action::Copy => {
                let text = match &self.view.read(cx).phase {
                    Phase::PasteFailed { text, .. }
                    | Phase::Recovery {
                        state: Recovery::Recovered(text),
                        ..
                    } => Some(text.clone()),
                    _ => None,
                };
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    let accessibility = match &self.view.read(cx).phase {
                        Phase::PasteFailed { accessibility, .. } => Some(*accessibility),
                        _ => None,
                    };
                    if let Some(accessibility) = accessibility {
                        self.phase(
                            Phase::PasteFailed {
                                text,
                                copied: true,
                                accessibility,
                            },
                            cx,
                        );
                    }
                }
            }
            Action::Retry => {
                if let Some(id) = self.last_id.clone() {
                    self.session = Some(Session {
                        id: id.clone(),
                        started: Instant::now(),
                        clipboard: cx.read_from_clipboard(),
                        clipboard_stamp: desktop::clipboard_stamp(),
                        application: desktop::frontmost_application(),
                    });
                    let recovery = matches!(self.view.read(cx).phase, Phase::Recovery { .. });
                    self.transcribe(id, recovery, cx);
                }
            }
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            let _ = recorder.stop();
            if let Some(session) = &self.session
                && let Err(error) = self
                    .host
                    .voice_settings()
                    .and_then(|settings| settings.finish_capture(&session.id, "interrupted"))
            {
                eprintln!("codex-gpui: cannot retain interrupted dictation: {error:#}");
            }
        }
    }
}
