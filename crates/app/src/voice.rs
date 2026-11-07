use crate::{
    Root,
    host::{self, Host, Identity, Request},
};
use anyhow::{Context as _, Result, bail, ensure};
use gpui::{App, Context};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::{
    computer_use::Load,
    settings::voice::{Catalog, Policy, VoiceAction, VoiceData, VoiceState},
};

impl Root {
    pub(crate) fn load_voice(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let epoch = cx.try_global::<VoiceState>().map_or(1, |s| s.epoch + 1);
        let mut policy = cx.try_global::<Policy>().cloned().unwrap_or_default();
        policy.microphone_selection = cfg!(target_os = "macos");
        let data = VoiceData {
            policy,
            ..Default::default()
        };
        let state = VoiceState {
            data: Arc::new(data),
            epoch,
            dispatch: Some(Arc::new(move |action, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| this.voice_action(action, cx));
            })),
            ..Default::default()
        };
        cx.set_global(state);
        self.voice_action(VoiceAction::Refresh, cx);
    }

    fn voice_action(&mut self, action: VoiceAction, cx: &mut Context<Self>) {
        let Some(mut state) = cx.try_global::<VoiceState>().cloned() else {
            return;
        };
        if let VoiceAction::SetPreference(key, value) = &action {
            state.error = set_preference(&mut state, key, value.clone(), cx)
                .err()
                .map(|error| error.to_string());
            cx.set_global(state);
            return;
        }
        if state.pending {
            return;
        }
        let imported = match &action {
            VoiceAction::ImportWords(words) => Some(words.clone()),
            _ => None,
        };
        let hotkey = match &action {
            VoiceAction::SetHotkey(kind, _) => Some(*kind),
            _ => None,
        };
        state.pending = true;
        state.error = None;
        state.hotkey_error = None;
        state.epoch += 1;
        let epoch = state.epoch;
        let previous = state.data.clone();
        cx.set_global(state);
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let task = cx
            .background_executor()
            .spawn(async move { apply(host.as_ref(), &bridge, action, &previous) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |_, cx| {
                let Some(mut state) = cx.try_global::<VoiceState>().cloned() else {
                    return;
                };
                if state.epoch != epoch {
                    return;
                }
                state.pending = false;
                match result {
                    Ok(mut data) => {
                        if let Some(settings) = cx.try_global::<crate::host_settings::Settings>() {
                            data.values = Value::Object(settings.read_all().clone());
                        }
                        if let Some(words) = &imported {
                            let result = finish_import(&mut data, words, cx);
                            if let Err(error) = result {
                                state.error = Some(error.to_string());
                            }
                        }
                        state.data = Arc::new(data);
                    }
                    Err(error) => match hotkey {
                        Some(kind) => state.hotkey_error = Some((kind, error.to_string())),
                        None => state.error = Some(error.to_string()),
                    },
                }
                cx.set_global(state);
            });
        })
        .detach();
    }
}

fn set_preference(state: &mut VoiceState, key: &str, value: Value, cx: &mut App) -> Result<()> {
    if key == "realtimeVoiceScreenContextEnabled" {
        ensure!(
            state.data.policy.screen_context,
            "Screen context is unavailable"
        );
    }
    let value = host::voice::Settings::normalize(key, value)?;
    ensure!(
        cx.has_global::<crate::host_settings::Settings>(),
        "Host settings are unavailable"
    );
    let settings = cx.global_mut::<crate::host_settings::Settings>();
    settings.write(key, value)?;
    Arc::make_mut(&mut state.data).values = Value::Object(settings.read_all().clone());
    Ok(())
}

/// The original's message for a hotkey service reply with `success: false`.
fn hotkey_error(kind: ui::screens::settings::voice::Hotkey, result: &Value) -> String {
    use ui::screens::settings::voice::Hotkey;
    if kind != Hotkey::Voice && result["errorCode"] == "shortcut-conflict" {
        return "Choose a different shortcut for single-tap dictation".into();
    }
    let fallback = match kind {
        Hotkey::Voice => "Failed to update voice chat hotkey",
        Hotkey::Hold => "Failed to update dictation shortcut",
        Hotkey::Toggle => "Failed to update single-tap dictation shortcut",
    };
    result["error"].as_str().unwrap_or(fallback).to_owned()
}

fn apply(
    host: &dyn Host,
    bridge: &bridge::Bridge,
    action: VoiceAction,
    previous: &VoiceData,
) -> Result<VoiceData> {
    let settings = host.voice_settings()?;
    let identity = host.identity();
    let mut hotkeys = previous.hotkeys.clone();
    match &action {
        VoiceAction::Refresh
        | VoiceAction::Microphones
        | VoiceAction::Dictionary
        | VoiceAction::SetPreference(_, _) => {}
        VoiceAction::SetHotkey(kind, value) => {
            let request = match kind {
                ui::screens::settings::voice::Hotkey::Voice => Request::post(
                    "set-codex-command-keybinding",
                    Some(
                        json!({"commandId":kind.command(),"update":value.as_ref().map_or(json!({"type":"clear"}),|v|json!({"type":"set","accelerator":v}))}),
                    ),
                ),
                ui::screens::settings::voice::Hotkey::Hold => {
                    Request::post("global-dictation-set-hotkey", Some(json!({"hotkey":value})))
                }
                ui::screens::settings::voice::Hotkey::Toggle => Request::post(
                    "global-dictation-set-toggle-hotkey",
                    Some(json!({"hotkey":value})),
                ),
            };
            let result = host::json_body(&host.fetch(&request)?)?;
            ensure!(
                result["success"] != false,
                "{}",
                hotkey_error(*kind, &result)
            );
            let key = match kind {
                ui::screens::settings::voice::Hotkey::Voice => "voice",
                ui::screens::settings::voice::Hotkey::Hold => "configuredHotkey",
                ui::screens::settings::voice::Hotkey::Toggle => "configuredToggleHotkey",
            };
            hotkeys[key] = json!(value);
        }
        VoiceAction::SetVoice(slug) => {
            ensure!(
                previous.policy.realtime == ui::screens::settings::voice::Access::Allowed,
                "Voice chat unavailable"
            );
            patch_setting(host, identity.as_ref(), "voice_name", account_voice(slug))?;
        }
        VoiceAction::SetLanguage(language) => {
            patch_setting(host, identity.as_ref(), "voice_main_language", language)?
        }
        VoiceAction::DeleteRecording(id) => settings.delete(id)?,
        VoiceAction::DownloadRecording(id) => {
            let home = crate::env::home().context("The Downloads directory is unavailable")?;
            settings.download(id, &home.join("Downloads"))?;
        }
        VoiceAction::TranscribeRecording(id) => {
            ensure!(previous.policy.dictation, "Dictation unavailable");
            let (recording, audio) = settings.audio(id)?;
            ensure!(
                recording.can_retry(true),
                "This recording cannot be transcribed"
            );
            let request = transcription_request(
                identity
                    .as_ref()
                    .context("No authenticated workspace is available")?,
                &recording.mime_type,
                recording.language.as_deref(),
                &audio,
            )?;
            let response = host::json_body(&host.fetch(&request)?)?;
            let text = response["text"]
                .as_str()
                .context("Invalid transcription response")?;
            settings.set_transcript(id, text)?;
        }
        VoiceAction::AddWord(text) => {
            validate_word(previous, text)?;
            request(
                host,
                identity.as_ref(),
                "POST",
                "/dictation/custom_dictionary/words",
                Some(json!({"text":text})),
            )?;
        }
        VoiceAction::DeleteWord(id) => {
            let encoded: String = url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
            request(
                host,
                identity.as_ref(),
                "DELETE",
                &format!("/dictation/custom_dictionary/words/{encoded}"),
                None,
            )?;
        }
        VoiceAction::ImportWords(words) => {
            for word in import_words(previous, words)? {
                request(
                    host,
                    identity.as_ref(),
                    "POST",
                    "/dictation/custom_dictionary/words",
                    Some(json!({"text":word})),
                )?;
            }
        }
    }
    let mut data = read(host, previous)?;
    if matches!(action, VoiceAction::Refresh) {
        hotkeys = host
            .fetch(&Request::post("global-dictation-hotkey-state", None))
            .and_then(|v| host::json_body(&v))
            .unwrap_or_else(|_| json!({"supported":false}));
    }
    data.hotkeys = hotkeys;
    if matches!(action, VoiceAction::Microphones) {
        data.microphones = load(host::voice::microphones());
    }
    if matches!(
        action,
        VoiceAction::Dictionary
            | VoiceAction::AddWord(_)
            | VoiceAction::DeleteWord(_)
            | VoiceAction::ImportWords(_)
    ) {
        data.dictionary = load(
            request(
                host,
                identity.as_ref(),
                "GET",
                "/dictation/custom_dictionary",
                None,
            )
            .and_then(|v| Ok(serde_json::from_value(v)?)),
        );
    }
    if matches!(action, VoiceAction::Refresh | VoiceAction::SetVoice(_))
        && data.policy.realtime == ui::screens::settings::voice::Access::Allowed
    {
        data.catalog = load(catalog(host, bridge, identity.as_ref()));
    }
    if matches!(action, VoiceAction::Refresh | VoiceAction::SetLanguage(_)) && data.policy.dictation
    {
        data.language = load(
            request(host, identity.as_ref(), "GET", "/settings/user", None).map(|value| {
                value["settings"]["voice_main_language"]
                    .as_str()
                    .unwrap_or("auto")
                    .to_owned()
            }),
        );
    }
    Ok(data)
}

fn read(host: &dyn Host, previous: &VoiceData) -> Result<VoiceData> {
    let settings = host.voice_settings()?;
    Ok(VoiceData {
        policy: previous.policy.clone(),
        values: settings.values()?,
        hotkeys: previous.hotkeys.clone(),
        catalog: previous.catalog.clone(),
        language: previous.language.clone(),
        microphones: previous.microphones.clone(),
        recordings: load(settings.recordings()),
        dictionary: previous.dictionary.clone(),
        account_dictionary: host.identity().is_some(),
    })
}

fn finish_import(data: &mut VoiceData, words: &[String], cx: &mut App) -> Result<()> {
    ensure!(
        cx.has_global::<crate::host_settings::Settings>(),
        "Host settings are unavailable"
    );
    let settings = cx.global_mut::<crate::host_settings::Settings>();
    let remaining: Vec<_> = settings
        .read_all()
        .get("dictationDictionary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|word| !words.iter().any(|s| word.as_str() == Some(s)))
        .cloned()
        .collect();
    let value = json!(remaining);
    settings.write("dictationDictionary", value.clone())?;
    data.values["dictationDictionary"] = value;
    Ok(())
}

fn load<T>(result: Result<T>) -> Load<T> {
    match result {
        Ok(value) => Load::Ready(value),
        Err(error) => Load::Failed(error.to_string()),
    }
}

fn validate_word(data: &VoiceData, text: &str) -> Result<()> {
    let Load::Ready(dictionary) = &data.dictionary else {
        bail!("Voice dictionary is not loaded");
    };
    ensure!(
        !text.trim().is_empty() && text.encode_utf16().count() <= dictionary.max_entry_utf16_units,
        "Dictionary entry is too long or empty"
    );
    ensure!(
        dictionary.words.len() < dictionary.max_words,
        "Voice dictionary is full"
    );
    Ok(())
}

fn import_words<'a>(data: &VoiceData, words: &'a [String]) -> Result<Vec<&'a str>> {
    let Load::Ready(dictionary) = &data.dictionary else {
        bail!("Voice dictionary is not loaded");
    };
    let mut unique = std::collections::HashSet::new();
    let mut pending = Vec::new();
    for word in words {
        ensure!(
            !word.trim().is_empty()
                && word.encode_utf16().count() <= dictionary.max_entry_utf16_units,
            "Dictionary entry is too long or empty"
        );
        if unique.insert(word.as_str())
            && !dictionary.words.iter().any(|stored| stored.text == *word)
        {
            pending.push(word.as_str());
        }
    }
    ensure!(
        pending.len() <= dictionary.max_words.saturating_sub(dictionary.words.len()),
        "Voice dictionary is full"
    );
    Ok(pending)
}

fn request(
    host: &dyn Host,
    identity: Option<&Identity>,
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<Value> {
    let identity = identity.context("No authenticated workspace is available")?;
    let mut value = http_request(identity, method, url);
    if let Some(body) = body {
        value["body"] = body.to_string().into();
        value["headers"]["Content-Type"] = "application/json".into();
    }
    host::json_body(&host.fetch(&Request::Http(value))?)
}

fn http_request(identity: &Identity, method: &str, url: &str) -> Value {
    json!({"url":url,"method":method,"headers":{"X-OpenAI-Attach-Auth":"1","X-OpenAI-Attach-Desktop-Surface":"1","X-OpenAI-Attach-Integrity-State":"1","ChatGPT-Account-ID":identity.account_id,"oai-did":identity.device_id,"OAI-Language":"en-US"},"expectedIdentity":{"accountId":identity.account_id,"userId":identity.user_id},"returnErrorResponse":true})
}

fn patch_setting(
    host: &dyn Host,
    identity: Option<&Identity>,
    feature: &str,
    value: &str,
) -> Result<()> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("feature", feature)
        .append_pair("value", value)
        .finish();
    request(
        host,
        identity,
        "PATCH",
        &format!("/settings/account_user_setting?{query}"),
        None,
    )?;
    Ok(())
}

fn catalog(
    host: &dyn Host,
    bridge: &bridge::Bridge,
    identity: Option<&Identity>,
) -> Result<Catalog> {
    let response: Value =
        pollster::block_on(bridge.request("thread/realtime/listVoices", json!({})))?;
    let voices = &response["voices"];
    let available: Vec<String> = serde_json::from_value(voices["v1"].clone())?;
    let default = voices["defaultV1"]
        .as_str()
        .context("Invalid default voice")?;
    ensure!(
        available.iter().any(|v| v == default),
        "The default voice is not supported"
    );
    let account = request(
        host,
        identity,
        "GET",
        "/settings/voices?spoken_language=en&voice_mode=advanced",
        None,
    )
    .ok();
    let selected = account.as_ref().and_then(|v| v["selected"].as_str());
    let selected = selected.map(realtime_voice);
    let selected = selected
        .filter(|s| available.iter().any(|v| v == s))
        .unwrap_or(default)
        .to_owned();
    Ok(Catalog {
        voices: available,
        selected,
        account_available: account.is_some(),
    })
}

const VOICE_ALIASES: &[(&str, &str)] =
    &[("fathom", "arbor"), ("glimmer", "sol"), ("orbit", "spruce")];

fn account_voice(voice: &str) -> &str {
    VOICE_ALIASES
        .iter()
        .find(|(_, realtime)| *realtime == voice)
        .map_or(voice, |(account, _)| *account)
}

fn realtime_voice(voice: &str) -> &str {
    VOICE_ALIASES
        .iter()
        .find(|(account, _)| *account == voice)
        .map_or(voice, |(_, realtime)| *realtime)
}

pub(crate) fn transcription_request(
    identity: &Identity,
    mime_type: &str,
    language: Option<&str>,
    audio: &[u8],
) -> Result<Request> {
    ensure!(
        !mime_type.is_empty() && !mime_type.contains(['\r', '\n']),
        "Invalid recording content type"
    );
    ensure!(!audio.is_empty(), "Dictation recording has no audio");
    let boundary = format!("----codex-transcribe-{}", host::device_id());
    let extension = mime_type
        .split(['/', ';'])
        .nth(1)
        .unwrap_or("webm")
        .replace('"', "");
    let mut body=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.{extension}\"\r\nContent-Type: {mime_type}\r\n\r\n").into_bytes();
    body.extend_from_slice(audio);
    body.extend_from_slice(b"\r\n");
    if let Some(language) = language {
        ensure!(
            !language.contains(['\r', '\n']),
            "Invalid recording language"
        );
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{language}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let mut request = http_request(identity, "POST", "/transcribe");
    request["headers"]["Content-Type"] = format!("multipart/form-data; boundary={boundary}").into();
    request["retry"] = "never".into();
    request["bodyBase64"] =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body).into();
    Ok(Request::Http(request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcription_rejects_injected_headers_and_empty_audio() {
        let identity = Identity {
            account_id: "account".into(),
            user_id: "user".into(),
            device_id: "device".into(),
        };
        assert!(
            transcription_request(&identity, "audio/webm\r\nX-Test: injected", None, &[1]).is_err()
        );
        assert!(transcription_request(&identity, "audio/webm", Some("en\r\n"), &[1]).is_err());
        assert!(transcription_request(&identity, "audio/webm", None, &[]).is_err());
    }

    #[test]
    fn account_voice_aliases_round_trip_without_changing_other_voices() {
        assert_eq!(account_voice("arbor"), "fathom");
        assert_eq!(account_voice("sol"), "glimmer");
        assert_eq!(account_voice("spruce"), "orbit");
        assert_eq!(realtime_voice("fathom"), "arbor");
        assert_eq!(realtime_voice("glimmer"), "sol");
        assert_eq!(realtime_voice("orbit"), "spruce");
        assert_eq!(account_voice("cove"), "cove");
        assert_eq!(realtime_voice("cove"), "cove");
    }

    #[test]
    fn local_preferences_save_while_network_work_is_pending() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("preferences.json");
        let mut cx = gpui::HeadlessAppContext::with_asset_source(
            textsys::text_system(),
            Arc::new(theme::Assets),
        );
        cx.update(|cx| -> Result<()> {
            cx.set_global(crate::host_settings::Settings::load(path.clone())?);
            let mut state = VoiceState {
                pending: true,
                ..Default::default()
            };
            set_preference(
                &mut state,
                "dictationDictionary",
                json!(["saved during refresh"]),
                cx,
            )?;
            assert!(state.pending);
            assert_eq!(
                state.data.values["dictationDictionary"],
                json!(["saved during refresh"])
            );
            let saved: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            assert_eq!(
                saved["dictationDictionary"],
                json!(["saved during refresh"])
            );
            Ok(())
        })
    }

    #[test]
    fn imports_validate_the_whole_batch_before_any_upload() {
        use ui::screens::settings::voice::{Dictionary, Word};
        let data = VoiceData {
            dictionary: Load::Ready(Dictionary {
                words: vec![Word {
                    id: "existing".into(),
                    text: "existing".into(),
                }],
                max_words: 2,
                max_entry_utf16_units: 10,
            }),
            ..Default::default()
        };
        assert!(import_words(&data, &["first".into(), "second".into()]).is_err());
        assert!(import_words(&data, &["first".into(), "too many characters".into()]).is_err());
        let words = ["existing".into(), "new".into(), "new".into()];
        assert_eq!(import_words(&data, &words).unwrap(), ["new"]);
    }
}
