use super::computer_use::Load;
use super::keyboard_shortcuts::{self as keyboard, KeyboardState};
use super::{button, switch};
use crate::{composer::Composer, focus, text};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Div, Entity, FocusHandle,
    Global, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, Modifiers, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

#[path = "voice_controls.rs"]
mod controls;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    #[default]
    Loading,
    Allowed,
    Denied,
    Error,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub realtime_visible: bool,
    pub realtime: Access,
    pub dictation: bool,
    pub global_dictation: bool,
    pub screen_context: bool,
    pub microphone_selection: bool,
}

impl Global for Policy {}

impl Policy {
    pub fn from_original(
        gates: &Value,
        capabilities: &Value,
        platform: &str,
        appshots: bool,
    ) -> Self {
        let gate = |key: &str| gates[key].as_bool() == Some(true);
        let capable = |key: &str| capabilities[key]["isCapable"].as_bool() == Some(true);
        let realtime_visible = gate("2380644311") && !gate("1697652030");
        let access = &capabilities["voice.realtime"];
        let realtime = if access["isLoading"] == true {
            Access::Loading
        } else if access["isError"] == true {
            Access::Error
        } else if realtime_visible && capable("voice.realtime") {
            Access::Allowed
        } else {
            Access::Denied
        };
        Self {
            realtime_visible,
            realtime,
            dictation: capable("dictation.composer") || capable("dictation.global"),
            global_dictation: capable("dictation.global"),
            screen_context: realtime == Access::Allowed && platform == "macOS" && appshots,
            microphone_selection: platform == "macOS",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphone {
    pub device_id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    pub created_at_ms: i64,
    pub mime_type: String,
    pub size_bytes: u64,
    pub status: String,
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
}

impl Recording {
    pub fn can_retry(&self, dictation: bool) -> bool {
        dictation && self.text.is_empty() && self.size_bytes > 0 && self.status != "recording"
    }
    pub fn label(&self) -> &str {
        if !self.text.is_empty() {
            return &self.text;
        }
        match self.status.as_str() {
            "recording" => "Recording in progress",
            "cancelled" => "Recording cancelled",
            "interrupted" => "Recording interrupted",
            _ => "Recording saved",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub voices: Vec<String>,
    pub selected: String,
    pub account_available: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Word {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Dictionary {
    pub words: Vec<Word>,
    pub max_words: usize,
    pub max_entry_utf16_units: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Voice,
    Hold,
    Toggle,
}

impl Hotkey {
    pub fn command(self) -> &'static str {
        match self {
            Self::Voice => "realtimeVoice",
            Self::Hold => "globalDictationHold",
            Self::Toggle => "globalDictationSingleTap",
        }
    }
}

#[derive(Clone, Debug)]
pub enum VoiceAction {
    Refresh,
    SetPreference(&'static str, Value),
    Microphones,
    SetHotkey(Hotkey, Option<String>),
    SetVoice(String),
    SetLanguage(String),
    DeleteRecording(String),
    DownloadRecording(String),
    TranscribeRecording(String),
    Dictionary,
    AddWord(String),
    DeleteWord(String),
    ImportWords(Vec<String>),
}

#[derive(Clone)]
pub struct VoiceData {
    pub policy: Policy,
    pub values: Value,
    pub catalog: Load<Catalog>,
    pub language: Load<String>,
    pub microphones: Load<Vec<Microphone>>,
    pub recordings: Load<Vec<Recording>>,
    pub dictionary: Load<Dictionary>,
    pub account_dictionary: bool,
}

impl Default for VoiceData {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            values: Value::Null,
            catalog: Load::Loading,
            language: Load::Loading,
            microphones: Load::Loading,
            recordings: Load::Loading,
            dictionary: Load::Loading,
            account_dictionary: false,
        }
    }
}

impl VoiceData {
    fn flag(&self, key: &str, default: bool) -> bool {
        self.values[key].as_bool().unwrap_or(default)
    }
    fn local_words(&self) -> Vec<String> {
        self.values["dictationDictionary"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }
}

fn keymap_supported(cx: &App) -> bool {
    cx.try_global::<KeyboardState>()
        .is_some_and(|state| state.loaded && state.dispatch.is_some())
}

fn accelerator(kind: Hotkey, cx: &App) -> Option<String> {
    let state = cx.try_global::<KeyboardState>()?;
    let command = keyboard::commands()
        .iter()
        .find(|command| command.id == kind.command())?;
    state.preferences.keys(command).next().map(str::to_owned)
}

pub type Dispatch = Arc<dyn Fn(VoiceAction, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct VoiceState {
    pub data: Arc<VoiceData>,
    pub pending: bool,
    pub error: Option<String>,
    pub dispatch: Option<Dispatch>,
    pub epoch: u64,
}
impl Global for VoiceState {}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("voice-settings-page", cx, Voice::new)
        .into_any_element()
}

struct Voice {
    menu: Option<&'static str>,
    menu_hovered: bool,
    capturing: Option<Hotkey>,
    capture_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    held_modifiers: Option<Modifiers>,
    _key_interceptor: gpui::Subscription,
    advanced: bool,
    dictionary_open: bool,
    fields: Vec<Entity<Composer>>,
    cloud_field: Option<Entity<Composer>>,
    submitted_word: Option<String>,
    imported: Option<Vec<String>>,
}

impl Voice {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<VoiceState>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<KeyboardState>(|_, cx| cx.notify())
            .detach();
        let capture_focus = cx.focus_handle();
        let owner = cx.entity_id();
        cx.on_release(move |_, cx| {
            if cx
                .try_global::<KeyboardState>()
                .is_some_and(|state| state.capture_owner == Some(owner))
            {
                cx.update_global::<KeyboardState, _>(|state, _| state.capture_owner = None);
            }
        })
        .detach();
        cx.on_blur(&capture_focus, window, |this, window, cx| {
            if this.capturing.is_some() {
                this.finish_capture(None, window, cx);
            }
        })
        .detach();
        let weak = cx.weak_entity();
        let key_interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            if matches!(
                event.keystroke.key.as_str(),
                "shift" | "control" | "alt" | "platform" | "function" | "cmd" | "fn"
            ) {
                return;
            }
            if let Some(page) = weak.upgrade() {
                let state = page.read(cx);
                if state.capturing.is_some() && state.capture_focus.is_focused(window) {
                    page.update(cx, |this, cx| {
                        this.capture_key(&event.keystroke, window, cx)
                    });
                    cx.stop_propagation();
                }
            }
        });
        Self {
            menu: None,
            menu_hovered: false,
            capturing: None,
            capture_focus,
            return_focus: None,
            held_modifiers: None,
            _key_interceptor: key_interceptor,
            advanced: false,
            dictionary_open: false,
            fields: Vec::new(),
            cloud_field: None,
            submitted_word: None,
            imported: None,
        }
    }

    fn publish_capture(&self, active: bool, cx: &mut Context<Self>) {
        let owner = cx.entity_id();
        if let Some(state) = cx.try_global::<KeyboardState>() {
            let next = if active {
                Some(owner)
            } else {
                state.capture_owner.filter(|id| *id != owner)
            };
            cx.update_global::<KeyboardState, _>(|state, _| state.capture_owner = next);
        }
    }

    fn begin_capture(
        &mut self,
        kind: Hotkey,
        return_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_global::<VoiceState>() {
            cx.update_global::<VoiceState, _>(|state, _| state.error = None);
        }
        self.capturing = Some(kind);
        self.return_focus = Some(return_focus);
        self.held_modifiers = None;
        self.publish_capture(true, cx);
        self.capture_focus.focus(window, cx);
        cx.notify();
    }

    fn finish_capture(
        &mut self,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = self.capturing.take();
        self.held_modifiers = None;
        self.publish_capture(false, cx);
        if let Some(focus) = self.return_focus.take() {
            focus.focus(window, cx);
        }
        if let (Some(kind), Some(value)) = (kind, value) {
            self.dispatch(VoiceAction::SetHotkey(kind, Some(value)), cx);
        }
        cx.notify();
    }

    fn capture_key(&mut self, stroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        if stroke.key == "escape" {
            self.finish_capture(None, window, cx);
        } else if let Some(value) = keyboard::capture(stroke) {
            self.finish_capture(Some(value), window, cx);
        }
        cx.stop_propagation();
    }

    fn modifiers(&mut self, modifiers: Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        if self.capturing.is_none() || !self.capture_focus.is_focused(window) {
            return;
        }
        if modifiers == Modifiers::default() {
            if let Some(held) = self.held_modifiers.take() {
                self.finish_capture(Some(keyboard::modifier_accelerator(held)), window, cx);
            }
        } else {
            *self.held_modifiers.get_or_insert_default() |= modifiers;
        }
    }
    fn dispatch(&self, action: VoiceAction, cx: &mut Context<Self>) {
        let state = cx.try_global::<VoiceState>();
        if state.is_some_and(|state| state.pending)
            && !matches!(
                action,
                VoiceAction::SetPreference(_, _) | VoiceAction::SetHotkey(_, _)
            )
        {
            return;
        }
        if let Some(dispatch) = state.and_then(|state| state.dispatch.clone()) {
            dispatch(action, cx);
        }
    }

    fn action(
        &self,
        id: impl Into<gpui::SharedString>,
        label: &'static str,
        action: VoiceAction,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.into();
        let t = cx.theme().clone();
        let disabled = (state.pending && !matches!(action, VoiceAction::SetHotkey(_, _)))
            || state.dispatch.is_none();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            button(label, t.text_alpha_5, None, &t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .when(disabled, |d| d.opacity(0.6)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                this.dispatch(action.clone(), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.dispatch(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn toggle(
        &self,
        id: &'static str,
        checked: bool,
        key: &'static str,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let disabled = state.dispatch.is_none();
        let handle = focus::handle(id, 0, window, cx);
        focus::ring(
            switch(id, checked, disabled, &t)
                .role(gpui::Role::Switch)
                .aria_label(id)
                .aria_disabled(disabled)
                .aria_toggled(if checked {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                }),
            &handle,
            px(10.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                this.dispatch(VoiceAction::SetPreference(key, Value::Bool(!checked)), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.dispatch(VoiceAction::SetPreference(key, Value::Bool(!checked)), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }
}

impl Render for Voice {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.try_global::<VoiceState>().cloned().unwrap_or_default();
        let data = &state.data;
        let mut groups = Vec::new();
        if data.policy.dictation || data.policy.realtime == Access::Allowed {
            let mut rows = vec![self.microphone(&state, window, cx)];
            if data.policy.dictation {
                rows.push(self.language(&state, window, cx));
            }
            groups.push(group("General", rows, &t));
        }
        if data.policy.realtime_visible {
            let (label, description) = match data.policy.realtime {
                Access::Loading => ("Checking voice chat access…", None),
                Access::Error => ("Unable to verify voice chat access", None),
                Access::Denied => (
                    "Voice chat unavailable",
                    Some("Your account or workspace does not have access to voice chat"),
                ),
                Access::Allowed => (
                    "Voice",
                    Some("Choose the voice Codex uses for new voice chats"),
                ),
            };
            let control = if data.policy.realtime == Access::Allowed {
                self.catalog(&state, window, cx)
            } else {
                div().into_any_element()
            };
            let mut rows = vec![row(label, description, control, &t)];
            if data.policy.realtime == Access::Allowed && keymap_supported(cx) {
                rows.push(self.hotkey(Hotkey::Voice, &state, window, cx));
            }
            if data.policy.screen_context {
                rows.push(row("Screen context", Some("Let Codex inspect the foreground app when you refer to what's on screen. macOS asks for access the first time Codex needs it"),
                    self.toggle("Enable screen context for voice chat", data.flag("realtimeVoiceScreenContextEnabled", true), "realtimeVoiceScreenContextEnabled", &state, window, cx), &t));
            }
            groups.push(group("Voice chat", rows, &t));
        }
        let mut rows = Vec::new();
        if data.policy.dictation {
            rows.push(row(
                "Dictation sounds",
                Some("Play sounds when dictation starts and stops"),
                self.toggle(
                    "Toggle dictation sounds",
                    data.flag("dictationSoundsEnabled", true),
                    "dictationSoundsEnabled",
                    &state,
                    window,
                    cx,
                ),
                &t,
            ));
        } else {
            rows.push(row(
                "Dictation unavailable",
                Some("Dictation is not available in your current configuration"),
                div().into_any_element(),
                &t,
            ));
        }
        if data.policy.global_dictation && keymap_supported(cx) {
            rows.push(self.hotkey(Hotkey::Hold, &state, window, cx));
            if self.advanced {
                rows.push(self.hotkey(Hotkey::Toggle, &state, window, cx));
            }
        }
        if data.policy.dictation
            || matches!(&data.recordings, Load::Ready(items) if !items.is_empty())
        {
            rows.extend(self.recordings(&state, window, cx));
        }
        groups.push(group("Dictation", rows, &t));
        if data.account_dictionary || data.policy.global_dictation && keymap_supported(cx) {
            groups.push(self.dictionary(&state, window, cx));
        }
        let modal = self
            .dictionary_open
            .then(|| self.dictionary_dialog(&state, window, cx));
        div()
            .id("voice-settings-root")
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, window, cx| {
                    this.modifiers(event.modifiers, window, cx)
                },
            ))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("voice-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .px(px(20.))
                            .pt(px(28.))
                            .child(text("Voice", 28., 33.6, 500., t.text)),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(40.))
                            .children(groups)
                            .children(state.error.map(|error| text(error, 13., 18., 430., t.text))),
                    ),
            )
            .children(modal)
    }
}

fn row(
    label: impl Into<gpui::SharedString>,
    description: Option<&str>,
    control: AnyElement,
    t: &Theme,
) -> Div {
    div()
        .relative()
        .px(px(16.))
        .py(px(12.))
        .flex()
        .items_center()
        .gap(px(24.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(text(label, 13., 18.571_428, 500., t.text))
                .children(
                    description
                        .map(|detail| text(detail.to_owned(), 12., 16., 430., t.text_secondary)),
                ),
        )
        .child(control)
}

fn group(label: &'static str, rows: Vec<Div>, t: &Theme) -> Div {
    if label.is_empty() {
        super::computer_use::card(rows, None, t)
    } else {
        super::computer_use::group(label, rows, None, t)
    }
}
