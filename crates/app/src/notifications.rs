use crate::{
    Root,
    host::{Host, Identity, Request, json_body},
    host_settings::Settings,
};
use anyhow::{Context as _, Result, ensure};
use gpui::{App, AppContext as _, BorrowAppContext as _, Context, PathPromptOptions, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::{
    screens::settings::notifications::{
        AccountGate, Action, Contacts, Load, Preferences, SOUND, State,
    },
    toast::Toast,
};

#[path = "notification_sound.rs"]
mod sound;

pub(crate) fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(State {
        dispatch: Some(Arc::new(move |action, cx| {
            let _ = root.update(cx, |root, cx| root.notification_action(action, cx));
        })),
        ..State::default()
    });
}

pub fn request(identity: &Identity, method: &str, path: &str, body: Option<Value>) -> Request {
    let mut request = json!({
        "url": path, "method": method,
        "headers": {
            "x-openai-web-frontend": "codex_webview", "x-oai-mcp-form-version": "2",
            "x-openai-codex-window-type": "electron", "OAI-Language": "en-US",
            "oai-did": identity.device_id, "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1", "X-OpenAI-Attach-Integrity-State": "1",
            "ChatGPT-Account-ID": identity.account_id
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "returnErrorResponse": true
    });
    if let Some(body) = body {
        request["body"] = json!(body.to_string());
    }
    Request::Http(request)
}

fn read(host: &dyn Host, identity: &Identity, path: &str) -> Result<Value> {
    json_body(&host.fetch(&request(identity, "GET", path, None))?)
}

fn identity_matches(state: &State, identity: &Identity, generation: u64) -> bool {
    state.generation == generation
        && state.identity.as_ref().is_some_and(|(account, user)| {
            account == &identity.account_id && user == &identity.user_id
        })
}

impl Root {
    pub(crate) fn load_notifications(&mut self, cx: &mut Context<Self>) {
        let desktop = cx
            .try_global::<Settings>()
            .map(|settings| {
                settings
                    .read_all()
                    .iter()
                    .filter(|(key, value)| valid_preference(key, value))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        cx.update_global::<State, _>(|state, _| state.desktop = desktop);
        self.refresh_notification_permission(cx);
    }

    fn notification_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::RefreshAccount => self.refresh_notification_account(cx),
            Action::LeavePage => cx.update_global::<State, _>(|state, _| {
                state.generation += 1;
                state.account = AccountGate::Hidden;
                state.identity = None;
                state.mutations.clear();
            }),
            Action::RefreshPreferences => self.refresh_notification_preferences(cx),
            Action::RefreshContacts => self.refresh_notification_contacts(cx),
            Action::RefreshPermission => self.refresh_notification_permission(cx),
            Action::SetPreference(key, value) => self.save_notification_preference(key, value, cx),
            Action::ImportSound => self.import_notification_sound(cx),
            Action::UpdateChannel {
                category,
                channel,
                enabled,
            } => self.update_notification_channel(category, channel, enabled, cx),
            Action::OpenSystemSettings => {
                let result = cx.background_spawn(async { sound::open_settings() });
                cx.spawn(async move |root, cx| {
                    if let Err(error) = result.await {
                        eprintln!("notification system settings: {error:#}");
                        let _ = root.update(cx, |root, cx| {
                            root.toast(Toast::danger("Couldn't open System Settings"), cx)
                        });
                    }
                })
                .detach();
            }
        }
    }

    fn refresh_notification_permission(&mut self, cx: &mut Context<Self>) {
        let read = cx.background_spawn(async { sound::permission() });
        cx.spawn(async move |root, cx| {
            let permission = read.await;
            let _ = root.update(cx, |_, cx| match permission {
                Ok(allowed) => {
                    cx.update_global::<State, _>(|state, _| state.permission = Some(allowed))
                }
                Err(error) => eprintln!("notification permission: {error:#}"),
            });
        })
        .detach();
    }

    fn refresh_notification_account(&mut self, cx: &mut Context<Self>) {
        let generation = cx.update_global::<State, _>(|state, _| {
            state.generation += 1;
            state.account = AccountGate::Loading;
            state.identity = None;
            state.preferences = Load::Loading;
            state.contacts = Load::Loading;
            state.mutations.clear();
            state.generation
        });
        let host = self.host.clone();
        let load = cx.background_spawn(async move {
            let Some(identity) = host.identity() else {
                return Ok(None);
            };
            let accounts = read(host.as_ref(), &identity, "/accounts/check/v4-2023-04-27")?;
            let account = &accounts["accounts"][&identity.account_id]["account"];
            ensure!(
                account.is_object(),
                "authenticated account is absent from account check"
            );
            Ok::<_, anyhow::Error>((account["structure"] == "personal").then_some(identity))
        });
        cx.spawn(async move |root, cx| {
            let result = load.await;
            let _ = root.update(cx, |root, cx| {
                if cx.global::<State>().generation != generation {
                    return;
                }
                let allowed = matches!(&result, Ok(Some(_)));
                cx.update_global::<State, _>(|state, _| match result {
                    Ok(Some(identity)) => {
                        state.account = AccountGate::Allowed;
                        state.identity = Some((identity.account_id, identity.user_id));
                    }
                    Ok(None) => state.account = AccountGate::Hidden,
                    Err(error) => {
                        eprintln!("notification account: {error:#}");
                        state.account = AccountGate::Failed;
                    }
                });
                if allowed {
                    root.refresh_notification_preferences(cx);
                }
            });
        })
        .detach();
    }

    fn refresh_notification_preferences(&mut self, cx: &mut Context<Self>) {
        if cx.global::<State>().account != AccountGate::Allowed {
            return;
        }
        let state = cx.global::<State>();
        let generation = state.generation;
        let expected = state.identity.clone();
        cx.update_global::<State, _>(|state, _| state.preferences = Load::Loading);
        let host = self.host.clone();
        let load = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("notification identity is unavailable")?;
            ensure!(
                expected == Some((identity.account_id.clone(), identity.user_id.clone())),
                "notification identity changed"
            );
            let preferences: Preferences =
                serde_json::from_value(read(host.as_ref(), &identity, "/notifications/settings")?)?;
            Ok::<_, anyhow::Error>((identity, preferences))
        });
        cx.spawn(async move |root, cx| {
            let result = load.await;
            let _ = root.update(cx, |root, cx| {
                if cx.global::<State>().generation != generation {
                    return;
                }
                let mut contacts = false;
                cx.update_global::<State, _>(|state, _| match result {
                    Ok((identity, preferences))
                        if identity_matches(state, &identity, generation) =>
                    {
                        contacts = preferences
                            .settings
                            .iter()
                            .any(|setting| setting.category == "teen_safety");
                        state.preferences = Load::Ready(preferences);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("notification preferences: {error:#}");
                        state.preferences = Load::Failed;
                    }
                });
                if contacts {
                    root.refresh_notification_contacts(cx);
                }
            });
        })
        .detach();
    }

    fn refresh_notification_contacts(&mut self, cx: &mut Context<Self>) {
        let state = cx.global::<State>();
        if state.account != AccountGate::Allowed {
            return;
        }
        let generation = state.generation;
        let expected = state.identity.clone();
        cx.update_global::<State, _>(|state, _| state.contacts = Load::Loading);
        let host = self.host.clone();
        let load = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("notification identity is unavailable")?;
            ensure!(
                expected == Some((identity.account_id.clone(), identity.user_id.clone())),
                "notification identity changed"
            );
            let contacts =
                serde_json::from_value::<Contacts>(read(host.as_ref(), &identity, "/me")?)?;
            Ok::<_, anyhow::Error>((identity, contacts))
        });
        cx.spawn(async move |root, cx| {
            let result = load.await;
            let _ = root.update(cx, |_, cx| {
                if cx.global::<State>().generation != generation {
                    return;
                }
                cx.update_global::<State, _>(|state, _| match result {
                    Ok((identity, contacts)) if identity_matches(state, &identity, generation) => {
                        state.contacts = Load::Ready(contacts)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("notification contacts: {error:#}");
                        state.contacts = Load::Failed;
                    }
                });
            });
        })
        .detach();
    }

    fn update_notification_channel(
        &mut self,
        category: String,
        channel: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let change = cx.update_global::<State, _>(|state, _| {
            if state.account != AccountGate::Allowed {
                return None;
            }
            if category == "teen_safety"
                && !matches!(&state.contacts, Load::Ready(contacts) if contacts.allows(&channel))
            {
                return None;
            }
            let option = state.channel_mut(&category, &channel)?;
            let previous = option.enabled;
            option.enabled = enabled;
            state.next_mutation += 1;
            let version = state.next_mutation;
            state
                .mutations
                .insert((category.clone(), channel.clone()), version);
            Some((previous, state.identity.clone(), state.generation, version))
        });
        let Some((previous, expected, generation, version)) = change else {
            return;
        };
        let host = self.host.clone();
        let category_request = category.clone();
        let channel_request = channel.clone();
        let save = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("notification identity is unavailable")?;
            ensure!(
                expected == Some((identity.account_id.clone(), identity.user_id.clone())),
                "notification identity changed"
            );
            let response = host.fetch(&request(
                &identity,
                "PATCH",
                "/notifications/settings",
                Some(json!({"updates": {category_request.as_str(): {channel_request.as_str(): enabled}}})),
            ))?;
            let preferences: Preferences = serde_json::from_value(json_body(&response)?)?;
            let enabled = preferences
                .settings
                .iter()
                .find(|item| item.category == category_request)
                .and_then(|item| {
                    item.options
                        .iter()
                        .find(|item| item.channel == channel_request)
                })
                .map_or(enabled, |item| item.enabled);
            Ok::<_, anyhow::Error>((identity, enabled))
        });
        cx.spawn(async move |root, cx| {
            let result = save.await;
            let _ = root.update(cx, |root, cx| {
                let current = cx.global::<State>();
                if current.generation != generation
                    || current.mutations.get(&(category.clone(), channel.clone())) != Some(&version)
                {
                    return;
                }
                let failed = result.is_err();
                cx.update_global::<State, _>(|state, _| {
                    let value = match result {
                        Ok((identity, value)) if identity_matches(state, &identity, generation) => {
                            value
                        }
                        Ok(_) => return,
                        Err(error) => {
                            eprintln!("notification update: {error:#}");
                            previous
                        }
                    };
                    if let Some(option) = state.channel_mut(&category, &channel) {
                        option.enabled = value;
                    }
                    state.mutations.remove(&(category, channel));
                });
                if failed {
                    root.toast(
                        Toast::danger("Failed to update ChatGPT notification settings"),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn save_notification_preference(
        &mut self,
        key: &'static str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        if cx.global::<State>().permission != Some(true) {
            return;
        }
        if !valid_preference(key, &value) {
            return;
        }
        let result = if cx.has_global::<Settings>() {
            cx.global_mut::<Settings>().write(key, value.clone())
        } else {
            Err(anyhow::anyhow!("host settings unavailable"))
        };
        if let Err(error) = result {
            eprintln!("notification preference: {error:#}");
            self.toast(
                Toast::danger(if key == SOUND {
                    "Couldn't change the notification sound. Try again"
                } else {
                    "Could not save notification preference"
                }),
                cx,
            );
            return;
        }
        cx.update_global::<State, _>(|state, _| {
            state.desktop.insert(key.into(), value.clone());
        });
        if key == SOUND {
            self.preview_notification_sound(value, cx);
        }
    }

    fn preview_notification_sound(&mut self, value: Value, cx: &mut Context<Self>) {
        sound::stop_preview();
        let selected = value.clone();
        let prepare = cx.background_spawn(async move { sound::prepare_preview(&selected) });
        cx.spawn(async move |root, cx| {
            let preview = prepare.await;
            let _ = root.update(cx, |root, cx| {
                if cx.global::<State>().desktop.get(SOUND) != Some(&value) {
                    return;
                }
                if let Err(error) = preview.and_then(sound::play_preview) {
                    eprintln!("notification preview: {error:#}");
                    root.toast(Toast::danger("Couldn't play the sound preview"), cx);
                }
            });
        })
        .detach();
    }

    fn import_notification_sound(&mut self, cx: &mut Context<Self>) {
        if cx.global::<State>().permission != Some(true) || cx.global::<State>().saving_sound {
            return;
        }
        cx.update_global::<State, _>(|state, _| state.saving_sound = true);
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose notification sound".into()),
        });
        cx.spawn(async move |root, cx| {
            let result = async {
                let Some(paths) = picker.await?? else { return Ok(None) };
                let path = paths.first().context("sound picker returned no file")?.clone();
                cx.background_spawn(async move { sound::import(&path).map(Some) }).await
            }.await;
            let _ = root.update(cx, |root, cx| {
                cx.update_global::<State, _>(|state, _| state.saving_sound = false);
                match result {
                    Ok(Some(value)) => root.save_notification_preference(SOUND, value, cx),
                    Ok(None) => {},
                    Err(error) => { eprintln!("notification import: {error:#}"); root.toast(Toast::danger("Couldn't import this sound. Choose an MP3, WAV, FLAC, or Ogg file under 30 seconds and 10 MB"), cx); }
                }
            });
        }).detach();
    }
}

pub fn valid_preference(key: &str, value: &Value) -> bool {
    use ui::screens::settings::notifications::{DOT, PERMISSIONS, QUESTIONS, TURN};
    match key {
        TURN => value
            .as_str()
            .is_some_and(|value| ["off", "unfocused", "always"].contains(&value)),
        PERMISSIONS | QUESTIONS | DOT => value.is_boolean(),
        SOUND => sound::valid(value),
        _ => false,
    }
}
