use super::*;
use crate::{composer::popover, icon::icon};
use gpui::{
    Anchor, AppContext as _, Focusable as _, KeyUpEvent, MouseButton, SharedString, anchored,
    deferred, point,
};

impl Voice {
    fn choice(
        &self,
        id: &'static str,
        label: String,
        options: Vec<(String, String, VoiceAction)>,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let disabled = (state.pending && id != "Microphone") || state.dispatch.is_none();
        let open = self.menu == Some(id);
        let current_label = label.clone();
        let handle = focus::handle(id, 0, window, cx);
        let trigger = div()
            .id(id)
            .h(px(28.))
            .px(px(13.))
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .role(gpui::Role::Button)
            .aria_label(id)
            .aria_disabled(disabled)
            .child(super::super::on_card(
                12.5,
                shapes::Corner::Squircle,
                t.primary_soft_alpha,
                Some(t.border),
            ))
            .child(text(label, 13., 18., 430., t.text))
            .child(icon("chevron-down", 14., t.text_tertiary))
            .when(disabled, |d| d.opacity(0.4))
            .on_click(cx.listener(move |this, _, _, cx| {
                if disabled {
                    return;
                }
                this.menu = if open { None } else { Some(id) };
                if id == "Microphone" && !open {
                    this.dispatch(VoiceAction::Microphones, cx);
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.menu = None;
                } else if !disabled
                    && matches!(e.keystroke.key.as_str(), "enter" | "space" | "down")
                {
                    this.menu = Some(id);
                    if id == "Microphone" {
                        this.dispatch(VoiceAction::Microphones, cx);
                    }
                } else {
                    return;
                }
                cx.notify();
                cx.stop_propagation();
            }));
        let trigger = focus::ring(trigger, &handle, px(12.5), px(2.), cx);
        let popup = open.then(|| {
            let rows = options
                .into_iter()
                .enumerate()
                .map(|(index, (_value, label, action))| {
                    let handle = focus::handle(format!("{id}-{index}"), 0, window, cx);
                    let key_action = action.clone();
                    let selected = label == current_label;
                    div()
                        .id((id, index))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(12.))
                        .hover(|d| d.bg(t.menu_item_hover))
                        .role(gpui::Role::MenuItem)
                        .aria_label(label.clone())
                        .track_focus(&handle.read(cx).handle)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(text(label, 13., 18.571_428, 430., t.text).flex_1())
                        .children(selected.then(|| icon("check", 14., t.text)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.menu = None;
                            this.dispatch(action.clone(), cx);
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            match event.keystroke.key.as_str() {
                                "enter" | "space" if !disabled => {
                                    this.menu = None;
                                    this.dispatch(key_action.clone(), cx);
                                }
                                "escape" => this.menu = None,
                                _ => return,
                            }
                            cx.notify();
                            cx.stop_propagation();
                        }))
                })
                .collect::<Vec<_>>();
            deferred(
                div().absolute().top_0().right_0().child(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(32.)))
                        .child(popover(&t, 280., |d| {
                            let note = (id == "Microphone")
                                .then(|| match &state.data.microphones {
                                    Load::Loading => Some("Loading microphones"),
                                    Load::Failed(_) => Some("Unable to load microphones"),
                                    Load::Ready(devices) if devices.is_empty() => {
                                        Some("No microphones found")
                                    }
                                    _ => None,
                                })
                                .flatten();
                            d.child(
                                div()
                                    .id("voice-options")
                                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                        this.menu_hovered = *hovered
                                    }))
                                    .children(rows)
                                    .children(note.map(|s| {
                                        text(s, 12., 16., 430., t.text_secondary).p(px(8.))
                                    })),
                            )
                        })),
                ),
            )
        });
        div()
            .id(SharedString::from(format!("voice-picker-{id}")))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.menu_hovered {
                    this.menu = None;
                    cx.notify();
                }
            }))
            .relative()
            .child(trigger)
            .children(popup)
            .into_any_element()
    }

    pub(super) fn microphone(
        &self,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let data = &state.data;
        let selected = data.values["microphoneInputDeviceId"].as_str();
        let mut options = vec![(
            String::new(),
            "System default".to_owned(),
            VoiceAction::SetPreference("microphoneInputDeviceId", Value::Null),
        )];
        let mut label = if selected.is_none() {
            "System default"
        } else {
            "Selected microphone"
        }
        .to_owned();
        match &data.microphones {
            Load::Ready(devices) => {
                if let Some(id) = selected {
                    label = devices
                        .iter()
                        .find(|d| d.device_id == id)
                        .map(|d| d.label.clone())
                        .unwrap_or_else(|| "Unavailable microphone".into());
                }
                for device in devices {
                    options.push((
                        device.device_id.clone(),
                        device.label.clone(),
                        VoiceAction::SetPreference(
                            "microphoneInputDeviceId",
                            Value::String(device.device_id.clone()),
                        ),
                    ));
                }
            }
            Load::Loading => {}
            Load::Failed(_) => {}
        }
        let t = cx.theme().clone();
        let control = if data.policy.microphone_selection {
            self.choice("Microphone", label, options, state, window, cx)
        } else {
            text(label, 13., 18., 430., t.text_secondary).into_any_element()
        };
        let detail = if data.policy.microphone_selection {
            "Used for voice chat and dictation"
        } else {
            "Microphone selection is not available on this device"
        };
        row("Microphone", Some(detail), control, &t)
    }

    pub(super) fn catalog(
        &self,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &state.data.catalog {
            Load::Failed(_) => self.action(
                "retry-voices",
                "Retry",
                VoiceAction::Refresh,
                state,
                window,
                cx,
            ),
            Load::Loading => {
                text("Loading…", 13., 18., 430., cx.theme().text_secondary).into_any_element()
            }
            Load::Ready(catalog) if !catalog.account_available => self.action(
                "retry-voices",
                "Retry",
                VoiceAction::Refresh,
                state,
                window,
                cx,
            ),
            Load::Ready(catalog) => self.choice(
                "Voice",
                voice_name(&catalog.selected),
                catalog
                    .voices
                    .iter()
                    .map(|voice| {
                        (
                            voice.clone(),
                            voice_name(voice),
                            VoiceAction::SetVoice(voice.clone()),
                        )
                    })
                    .collect(),
                state,
                window,
                cx,
            ),
        }
    }

    pub(super) fn language(
        &self,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let control = match &state.data.language {
            Load::Failed(_) => self.action(
                "retry-language",
                "Retry",
                VoiceAction::Refresh,
                state,
                window,
                cx,
            ),
            Load::Loading => text("Loading…", 13., 18., 430., t.text_secondary).into_any_element(),
            Load::Ready(selected) => {
                let label = if selected == "auto" {
                    "Auto-detect".into()
                } else {
                    selected.clone()
                };
                self.choice(
                    "Language",
                    label,
                    LANGUAGES
                        .iter()
                        .map(|&(value, label)| {
                            (
                                value.to_owned(),
                                label.to_owned(),
                                VoiceAction::SetLanguage(value.to_owned()),
                            )
                        })
                        .collect(),
                    state,
                    window,
                    cx,
                )
            }
        };
        row("Language", None, control, &t)
    }

    pub(super) fn hotkey(
        &self,
        kind: Hotkey,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let (label, detail) = match kind {
            Hotkey::Voice => (
                "Voice chat hotkey",
                "Start voice chat from anywhere on desktop",
            ),
            Hotkey::Hold => (
                "Dictation shortcut",
                "Hold to dictate, or double-tap for hands-free",
            ),
            Hotkey::Toggle => (
                "Single-tap shortcut",
                "Press once to start, and again to finish",
            ),
        };
        let capturing = self.capturing == Some(kind);
        let value = state.data.accelerator(kind);
        let disabled = state.pending || state.dispatch.is_none();
        let control = if capturing {
            button("Press keys…", t.text_alpha_5, Some(t.border), &t)
                .id("voice-hotkey-capture")
                .track_focus(&self.capture_focus)
                .role(gpui::Role::TextInput)
                .aria_label(format!("{label} capture"))
                .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                    if e.keystroke.key == "escape" {
                        this.capturing = None;
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    if let Some(value) = captured_key(e) {
                        this.capturing = None;
                        this.dispatch(VoiceAction::SetHotkey(kind, Some(value)), cx);
                        cx.notify();
                    }
                    cx.stop_propagation();
                }))
                .on_key_up(cx.listener(move |this, e: &KeyUpEvent, _, cx| {
                    let value = match e.keystroke.key.as_str() {
                        "shift" => Some("LeftShift"),
                        "control" => Some("LeftControl"),
                        "alt" => Some("LeftOption"),
                        "cmd" => Some("LeftCommand"),
                        "fn" => Some("Fn"),
                        _ => None,
                    };
                    if let Some(value) = value {
                        this.capturing = None;
                        this.dispatch(VoiceAction::SetHotkey(kind, Some(value.into())), cx);
                        cx.notify();
                    }
                    cx.stop_propagation();
                }))
                .into_any_element()
        } else {
            let formatted = value.map(display_hotkey).unwrap_or_else(|| "Off".into());
            div()
                .flex()
                .gap(px(8.))
                .child(
                    div()
                        .id(kind.command())
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .aria_disabled(disabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if disabled {
                                return;
                            }
                            this.capturing = Some(kind);
                            this.capture_focus.focus(window, cx);
                            cx.notify();
                        }))
                        .child(text(formatted, 13., 18., 430., t.text)),
                )
                .children(value.map(|_| {
                    self.action(
                        format!("clear-{}", kind.command()),
                        "Clear",
                        VoiceAction::SetHotkey(kind, None),
                        state,
                        window,
                        cx,
                    )
                }))
                .into_any_element()
        };
        let error = state
            .hotkey_error
            .as_ref()
            .filter(|(failed, _)| *failed == kind)
            .map(|(_, error)| error.clone());
        let base = row(label, Some(detail), control, &t).when(kind == Hotkey::Hold, |d| {
            d.child(
                div()
                    .id("voice-advanced")
                    .role(gpui::Role::Button)
                    .aria_label("Advanced")
                    .child(text("Advanced", 12., 16., 430., t.text_secondary))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
        });
        match error {
            Some(error) => div().relative().flex().flex_col().child(base).child(
                div()
                    .px(px(16.))
                    .pb(px(12.))
                    .child(text(error, 12., 16., 430., t.text_error)),
            ),
            None => base,
        }
    }

    pub(super) fn recordings(
        &self,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let t = cx.theme().clone();
        let date = |item: &super::Recording| {
            chrono::DateTime::from_timestamp_millis(item.created_at_ms)
                .map(|d| d.format("%b %-d, %-I:%M %p").to_string())
                .unwrap_or_default()
        };
        let focused = match &state.data.recordings {
            Load::Ready(items) => state
                .recording
                .as_ref()
                .and_then(|id| items.iter().find(|item| &item.id == id)),
            _ => None,
        };
        // A failure notice opens one recording; `Show all recordings` returns to the list.
        let header = match focused {
            Some(_) => button("Show all recordings", t.text_alpha_5, None, &t)
                .id("voice-all-recordings")
                .role(gpui::Role::Button)
                .aria_label("Show all recordings")
                .on_click(|_, _, cx| {
                    if let Some(state) = cx.try_global::<VoiceState>() {
                        let state = VoiceState {
                            recording: None,
                            ..state.clone()
                        };
                        cx.set_global(state);
                    }
                })
                .into_any_element(),
            None => div().into_any_element(),
        };
        let mut rows = vec![row(
            "Recent recordings",
            Some("Your last 20 recordings are saved on this device"),
            header,
            &t,
        )];
        if let Some(item) = focused {
            let mut controls = div().flex().items_center().gap(px(8.));
            if item.can_retry(state.data.policy.dictation) {
                controls = controls.child(self.action(
                    format!("retry-{}", item.id),
                    "Retry",
                    VoiceAction::TranscribeRecording(item.id.clone()),
                    state,
                    window,
                    cx,
                ));
            }
            if item.size_bytes > 0 {
                controls = controls.child(self.action(
                    format!("download-{}", item.id),
                    "Download audio",
                    VoiceAction::DownloadRecording(item.id.clone()),
                    state,
                    window,
                    cx,
                ));
            }
            rows.push(row(
                item.label().to_owned(),
                Some(&date(item)),
                controls.into_any_element(),
                &t,
            ));
            return rows;
        }
        match &state.data.recordings {
            Load::Loading => rows.push(row("Loading…", None, div().into_any_element(), &t)),
            Load::Failed(error) => rows.push(row(
                error.clone(),
                None,
                self.action(
                    "retry-recordings",
                    "Retry",
                    VoiceAction::Refresh,
                    state,
                    window,
                    cx,
                ),
                &t,
            )),
            Load::Ready(items) => {
                for item in items.iter().take(20) {
                    let date = date(item);
                    let mut controls = div().flex().items_center().gap(px(8.));
                    if !item.text.is_empty() {
                        let text = item.text.clone();
                        controls = controls.child(
                            button("Copy", t.text_alpha_5, None, &t)
                                .id(SharedString::from(format!("copy-{}", item.id)))
                                .aria_label("Copy dictation")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        text.clone(),
                                    ))
                                }),
                        );
                    }
                    if item.can_retry(state.data.policy.dictation) {
                        controls = controls.child(self.action(
                            format!("retry-{}", item.id),
                            "Retry",
                            VoiceAction::TranscribeRecording(item.id.clone()),
                            state,
                            window,
                            cx,
                        ));
                    }
                    if item.size_bytes > 0 {
                        controls = controls.child(self.action(
                            format!("download-{}", item.id),
                            "Download recording",
                            VoiceAction::DownloadRecording(item.id.clone()),
                            state,
                            window,
                            cx,
                        ));
                    }
                    if item.status != "recording" {
                        controls = controls.child(self.action(
                            format!("delete-{}", item.id),
                            "Delete recording",
                            VoiceAction::DeleteRecording(item.id.clone()),
                            state,
                            window,
                            cx,
                        ));
                    }
                    rows.push(row(
                        item.label().to_owned(),
                        Some(&date),
                        controls.into_any_element(),
                        &t,
                    ));
                }
            }
        }
        rows
    }

    pub(super) fn dictionary(
        &mut self,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        if state.data.account_dictionary {
            let control = button("Manage", t.text_alpha_5, None, &t)
                .id("manage-voice-dictionary")
                .aria_label("Manage")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dictionary_open = true;
                    this.dispatch(VoiceAction::Dictionary, cx);
                    cx.notify();
                }))
                .into_any_element();
            return group(
                "",
                vec![row(
                    "Voice dictionary",
                    Some("Words or phrases dictation should recognize"),
                    control,
                    &t,
                )],
                &t,
            );
        }
        if self.fields.is_empty() {
            let words = state.data.local_words();
            for word in if words.is_empty() {
                vec![String::new()]
            } else {
                words
            } {
                self.add_field(&word, window, cx);
            }
        }
        let control = button("Add entry", t.text_alpha_5, None, &t)
            .id("add-dictionary-entry")
            .aria_label("Add entry")
            .on_click(cx.listener(|this, _, window, cx| {
                this.add_field("", window, cx);
                if let Some(field) = this.fields.last() {
                    field.focus_handle(cx).focus(window, cx);
                }
                cx.notify();
            }))
            .into_any_element();
        let mut rows = vec![row(
            "Dictation dictionary",
            Some("Words or phrases dictation should recognize"),
            control,
            &t,
        )];
        for (index, field) in self.fields.iter().enumerate() {
            let remove = button("Remove", t.text_alpha_5, None, &t)
                .id(("remove-dictionary", index))
                .aria_label(format!("Remove dictionary entry {}", index + 1))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let words: Vec<_> = this
                        .fields
                        .iter()
                        .enumerate()
                        .filter(|(at, _)| *at != index)
                        .map(|(_, field)| field.read(cx).field_text().trim().to_owned())
                        .collect();
                    this.dispatch(
                        VoiceAction::SetPreference("dictationDictionary", serde_json::json!(words)),
                        cx,
                    );
                    if cx.global::<VoiceState>().error.is_none() {
                        this.fields.remove(index);
                    }
                    cx.notify();
                }));
            rows.push(
                div().child(
                    div()
                        .id(("voice-dictionary-row", index))
                        .p(px(16.))
                        .flex()
                        .gap(px(8.))
                        .role(gpui::Role::Group)
                        .aria_label(format!("Dictionary entry {}", index + 1))
                        .capture_key_down(cx.listener(
                            move |this, event: &KeyDownEvent, window, cx| {
                                if event.keystroke.key != "enter" {
                                    return;
                                }
                                this.add_field("", window, cx);
                                if let Some(field) = this.fields.pop() {
                                    field.focus_handle(cx).focus(window, cx);
                                    this.fields.insert(index + 1, field);
                                }
                                cx.notify();
                                cx.stop_propagation();
                            },
                        ))
                        .child(div().flex_1().child(field.clone()))
                        .child(remove),
                ),
            );
        }
        group("", rows, &t)
    }

    fn add_field(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let field = cx.new(|cx| Composer::settings_field("Jane Doe", value, false, cx));
        cx.on_blur(&field.focus_handle(cx), window, |this, _, cx| {
            this.save_words(cx)
        })
        .detach();
        self.fields.push(field);
    }
    fn save_words(&self, cx: &mut Context<Self>) {
        let words: Vec<_> = self
            .fields
            .iter()
            .map(|field| field.read(cx).field_text().trim())
            .filter(|s| !s.is_empty())
            .collect();
        self.dispatch(
            VoiceAction::SetPreference("dictationDictionary", serde_json::json!(words)),
            cx,
        );
    }

    pub(super) fn dictionary_dialog(
        &mut self,
        state: &VoiceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let field = self
            .cloud_field
            .get_or_insert_with(|| {
                cx.new(|cx| Composer::settings_field("Add a word or phrase", "", false, cx))
            })
            .clone();
        if !state.pending
            && state.error.is_none()
            && let Some(submitted) = &self.submitted_word
            && let Load::Ready(dictionary) = &state.data.dictionary
            && dictionary.words.iter().any(|word| word.text == *submitted)
        {
            if field.read(cx).field_text().trim() == submitted {
                field.update(cx, |field, cx| field.set_text("", cx));
            }
            self.submitted_word = None;
        }
        let mut content = div().flex().flex_col().gap(px(16.)).child(text(
            "Voice dictionary",
            20.,
            28.,
            500.,
            t.text,
        ));
        match &state.data.dictionary {
            Load::Loading => {
                content = content.child(text("Loading…", 13., 18., 430., t.text_secondary))
            }
            Load::Failed(error) => {
                content = content
                    .child(text(error.clone(), 13., 18., 430., t.text))
                    .child(self.action(
                        "retry-dictionary",
                        "Retry",
                        VoiceAction::Dictionary,
                        state,
                        window,
                        cx,
                    ))
            }
            Load::Ready(dictionary) => {
                content = content.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().flex_1().child(field.clone()))
                        .child(
                            button("Add", t.text_alpha_5, None, &t)
                                .id("add-voice-word")
                                .aria_label("Add")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let text = field.read(cx).field_text().trim().to_owned();
                                    if !text.is_empty() {
                                        this.submitted_word = Some(text.clone());
                                        this.dispatch(VoiceAction::AddWord(text), cx);
                                    }
                                })),
                        ),
                );
                for word in &dictionary.words {
                    content = content.child(row(
                        word.text.clone(),
                        None,
                        self.action(
                            format!("delete-word-{}", word.id),
                            "Remove",
                            VoiceAction::DeleteWord(word.id.clone()),
                            state,
                            window,
                            cx,
                        ),
                        &t,
                    ));
                }
                let local = state.data.local_words();
                if !local.is_empty() {
                    if let Some(selected) = &self.imported {
                        content = content
                            .child(text("Import local words", 16., 22., 500., t.text))
                            .child(text(
                                "Saved words will be available on your other devices",
                                13.,
                                18.,
                                430.,
                                t.text_secondary,
                            ));
                        for word in &local {
                            let word = word.clone();
                            let checked = selected.contains(&word);
                            content = content.child(
                                div()
                                    .id(SharedString::from(format!("import-{word}")))
                                    .role(gpui::Role::CheckBox)
                                    .aria_label(word.clone())
                                    .aria_toggled(if checked {
                                        gpui::Toggled::True
                                    } else {
                                        gpui::Toggled::False
                                    })
                                    .child(text(word.clone(), 13., 18., 430., t.text))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(words) = &mut this.imported {
                                            if checked {
                                                words.retain(|w| w != &word)
                                            } else {
                                                words.push(word.clone())
                                            }
                                        }
                                        cx.notify();
                                    })),
                            );
                        }
                        content = content.child(self.action(
                            "confirm-word-import",
                            "Import words",
                            VoiceAction::ImportWords(selected.clone()),
                            state,
                            window,
                            cx,
                        ));
                    } else {
                        content = content.child(
                            button("Import local words", t.text_alpha_5, None, &t)
                                .id("preview-word-import")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.imported = Some(local.clone());
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
        }
        content = content.children(
            state
                .error
                .as_ref()
                .map(|error| text(error.clone(), 13., 18., 430., t.text)),
        );
        content = content.child(
            button("Done", t.text_alpha_5, None, &t)
                .id("close-voice-dictionary")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dictionary_open = false;
                    this.imported = None;
                    cx.notify();
                })),
        );
        deferred(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.5))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(popover(&t, 512., |d| {
                    d.p(px(24.)).child(
                        div()
                            .id("voice-dictionary-content")
                            .max_h(px(600.))
                            .overflow_y_scroll()
                            .child(content),
                    )
                })),
        )
        .into_any_element()
    }
}

pub(super) fn voice_name(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}
fn display_hotkey(value: &str) -> String {
    value
        .replace("Command", "⌘")
        .replace("Ctrl", "⌃")
        .replace("Alt", "⌥")
        .replace("Shift", "⇧")
        .replace('+', " ")
}
fn captured_key(event: &KeyDownEvent) -> Option<String> {
    let key = &event.keystroke.key;
    if matches!(
        key.as_str(),
        "shift" | "control" | "alt" | "cmd" | "fn" | "escape"
    ) {
        return None;
    }
    let mut parts = Vec::new();
    let m = &event.keystroke.modifiers;
    if m.control {
        parts.push("Ctrl".to_owned())
    }
    if m.platform {
        parts.push("Command".to_owned())
    }
    if m.alt {
        parts.push("Alt".to_owned())
    }
    if m.shift {
        parts.push("Shift".to_owned())
    }
    parts.push(key.to_uppercase());
    Some(parts.join("+"))
}
const LANGUAGES: &[(&str, &str)] = &[
    ("auto", "Auto-detect"),
    ("af", "Afrikaans"),
    ("amh", "Amharic"),
    ("ar", "Arabic"),
    ("hy", "Armenian"),
    ("az", "Azerbaijani"),
    ("bn", "Bangla"),
    ("be", "Belarusian"),
    ("bs", "Bosnian"),
    ("bg", "Bulgarian"),
    ("mya", "Burmese"),
    ("yue", "Cantonese (Traditional Chinese)"),
    ("ca", "Catalan"),
    ("hr", "Croatian"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("en", "English"),
    ("et", "Estonian"),
    ("fil", "Filipino"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("gl", "Galician"),
    ("ka", "Georgian"),
    ("de", "German"),
    ("el", "Greek"),
    ("gu", "Gujarati"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("is", "Icelandic"),
    ("id", "Indonesian"),
    ("gle", "Irish"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("kn", "Kannada"),
    ("kk", "Kazakh"),
    ("ko", "Korean"),
    ("lv", "Latvian"),
    ("lt", "Lithuanian"),
    ("mk", "Macedonian"),
    ("ms", "Malay"),
    ("ml", "Malayalam"),
    ("zh", "Mandarin Chinese"),
    ("mi", "Māori"),
    ("mr", "Marathi"),
    ("mon", "Mongolian"),
    ("ne", "Nepali"),
    ("no", "Norwegian"),
    ("nb", "Norwegian Bokmål"),
    ("fa", "Persian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sr", "Serbian"),
    ("zh-cn", "Simplified Chinese"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("som", "Somali"),
    ("es", "Spanish"),
    ("sw", "Swahili"),
    ("sv", "Swedish"),
    ("tl", "Tagalog"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("th", "Thai"),
    ("zh-hk", "Traditional Chinese (Hong Kong)"),
    ("zh-tw", "Traditional Chinese (Taiwan)"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("vi", "Vietnamese"),
    ("cy", "Welsh"),
];
