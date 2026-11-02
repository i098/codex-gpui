use super::computer_use::Load;
use super::{button, switch};
use crate::{composer::Composer, focus, text};
use gpui::{
    AnyElement, App, Context, Div, Entity, FocusHandle, Global, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
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
    pub hotkeys: Value,
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
            hotkeys: Value::Null,
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
    fn accelerator(&self, hotkey: Hotkey) -> Option<&str> {
        let key = match hotkey {
            Hotkey::Voice => "voice",
            Hotkey::Hold => "configuredHotkey",
            Hotkey::Toggle => "configuredToggleHotkey",
        };
        self.hotkeys[key].as_str()
    }
}

pub type Dispatch = Arc<dyn Fn(VoiceAction, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct VoiceState {
    pub data: Arc<VoiceData>,
    pub pending: bool,
    pub error: Option<String>,
    pub dispatch: Option<Dispatch>,
    pub epoch: u64,
    /// The recording a dictation failure notice opened the page on; `None` lists them all.
    pub recording: Option<String>,
    /// The last failed hotkey change, shown under that hotkey's row.
    pub hotkey_error: Option<(Hotkey, String)>,
}
impl Global for VoiceState {}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("voice-settings-page", cx, |_, cx| {
            cx.observe_global::<VoiceState>(|_, cx| cx.notify())
                .detach();
            Voice {
                menu: None,
                menu_hovered: false,
                capturing: None,
                capture_focus: cx.focus_handle(),
                advanced: false,
                dictionary_open: false,
                fields: Vec::new(),
                cloud_field: None,
                submitted_word: None,
                imported: None,
            }
        })
        .into_any_element()
}

struct Voice {
    menu: Option<&'static str>,
    menu_hovered: bool,
    capturing: Option<Hotkey>,
    capture_focus: FocusHandle,
    advanced: bool,
    dictionary_open: bool,
    fields: Vec<Entity<Composer>>,
    cloud_field: Option<Entity<Composer>>,
    submitted_word: Option<String>,
    imported: Option<Vec<String>>,
}

impl Voice {
    fn dispatch(&self, action: VoiceAction, cx: &mut Context<Self>) {
        let state = cx.try_global::<VoiceState>();
        if state.is_some_and(|state| state.pending)
            && !matches!(action, VoiceAction::SetPreference(_, _))
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
        let disabled = state.pending || state.dispatch.is_none();
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
            if data.policy.realtime == Access::Allowed && data.hotkeys["supported"] != false {
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
        if data.policy.global_dictation && data.hotkeys["supported"] != false {
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
        if data.account_dictionary
            || data.policy.global_dictation && data.hotkeys["supported"] != false
        {
            groups.push(self.dictionary(&state, window, cx));
        }
        let modal = self
            .dictionary_open
            .then(|| self.dictionary_dialog(&state, window, cx));
        div()
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
