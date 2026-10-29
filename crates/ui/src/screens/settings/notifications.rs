//! Settings ▸ Notifications (`notifications-settings-*.js`): the desktop alert rows in
//! General (`lt`, `Ve`) and the account-gated ChatGPT channel group of the Notifications page
//! (`ct`, `Ye`). The app owns the permission check, sound files and ChatGPT requests.

use super::{SettingsPane, button, focus_on_pointer, settings_row, settings_row_content, switch};
use crate::screens::computer_use::Load;
use crate::settings_kit::{Choice, MultiSelect, Select, Selected, SelectionChanged};
use crate::{focus, icon::icon, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Global, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use std::sync::Arc;
use theme::Theme;

pub const TURN_MODE: &str = "host.notifications-turn-mode";
pub const PERMISSIONS: &str = "host.notifications-permissions-enabled";
pub const QUESTIONS: &str = "host.notifications-questions-enabled";
pub const DOT: &str = "host.notifications-dot-enabled";
pub const SOUND: &str = "host.notifications-sound";

/// `notifications.turnMode.*`, in menu order; `unfocused` is the default.
const TURN_MODES: &[(&str, &str)] = &[
    ("off", "Never"),
    ("unfocused", "Only when unfocused"),
    ("always", "Always"),
];

/// `Ve`'s sound list; only `default`, `classic` and `none` exist off macOS.
const SOUNDS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("classic", "Classic"),
    ("none", "None"),
    ("Basso", "Basso"),
    ("Glass", "Glass"),
    ("Ping", "Ping"),
    ("Pop", "Pop"),
    ("Purr", "Purr"),
    ("Tink", "Tink"),
];
const CHOOSE_SOUND: &str = "\0choose";
const CUSTOM_SOUND: &str = "\0custom";

/// macOS's answer for this app (`getNotificationPermissionStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Enabled,
    Disabled,
    NotDetermined,
}

/// The personal-account gate of the ChatGPT group (`oNc`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Account {
    /// No ChatGPT sign-in, or a workspace account.
    #[default]
    Hidden,
    Loading,
    Error,
    Personal,
}

#[derive(Clone, Debug)]
pub enum Action {
    RefreshPermission,
    OpenSystemSettings,
    /// Previews a chosen sound setting value.
    PreviewSound(Value),
    /// Opens the file picker and imports a custom sound.
    ImportSound,
    RefreshAccount,
    RefreshChannels,
    RefreshContacts,
    SetChannel {
        category: String,
        channel: String,
        enabled: bool,
    },
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    /// `None` where the platform reports no notification permission (only macOS does).
    pub permission: Option<Load<Permission>>,
    pub opening_settings: bool,
    pub importing_sound: bool,
    pub account: Account,
    /// `GET /notifications/settings`.
    pub channels: Load<Arc<Value>>,
    /// `GET /me`: the email and phone number the Family channels need.
    pub contacts: Load<Arc<Value>>,
    pub fetching: bool,
    pub dispatch: Option<Dispatch>,
}

impl Global for State {}

impl State {
    /// `p`: the desktop controls wait for, or are blocked by, the system permission.
    pub fn controls_disabled(&self) -> bool {
        matches!(
            self.permission,
            Some(Load::Loading | Load::Ready(Permission::Disabled | Permission::NotDetermined))
        )
    }

    fn system_off(&self) -> bool {
        matches!(
            self.permission,
            Some(Load::Ready(
                Permission::Disabled | Permission::NotDetermined
            ))
        )
    }
}

pub fn dispatch(action: Action, cx: &mut App) {
    if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
        dispatch(action, cx);
    }
}

/// The sound setting's menu value: a built-in name, or [`CUSTOM_SOUND`] for an imported file.
fn sound_value(setting: &Value) -> &str {
    match setting {
        Value::String(name) => name,
        Value::Object(_) => CUSTOM_SOUND,
        _ => "default",
    }
}

impl SettingsPane {
    /// General ▸ Notifications.
    pub(super) fn notifications(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        // `refetchOnWindowFocus: always`: the app refreshes on activation; this covers opening.
        window.use_keyed_state("notification-permission", cx, |_, cx| {
            dispatch(Action::RefreshPermission, cx);
        });
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        let disabled = state.controls_disabled();
        let mut rows = Vec::new();
        if state.system_off() {
            let open = focus::handle("open-notification-settings", 0, window, cx);
            let pending = state.opening_settings;
            let control = focus::ring(
                button(
                    "Open System Settings",
                    t.primary_soft_alpha,
                    Some(t.border),
                    t,
                )
                .id("open-notification-settings")
                .role(gpui::Role::Button)
                .aria_label("Open System Settings")
                .aria_disabled(pending)
                .when(pending, |d| d.opacity(0.5)),
                &open,
                theme::Theme::RADIUS_LG,
                px(2.),
                cx,
            )
            .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(open))
            .on_click(move |_, _, cx| {
                if !pending {
                    dispatch(Action::OpenSystemSettings, cx);
                }
            });
            rows.push(settings_row(
                "Notifications are off",
                Some("Turn on notifications in System Settings to use the options below"),
                control.into_any_element(),
                t,
            ));
        }
        let turn_mode = self.notification_select(TURN_MODE, disabled, cx);
        rows.push(settings_row(
            "Turn completion notifications",
            Some("Set when ChatGPT alerts you that it's finished"),
            turn_mode,
            t,
        ));
        for (key, title, detail) in [
            (
                PERMISSIONS,
                "Enable permission notifications",
                "Show alerts when notification permissions are required",
            ),
            (
                QUESTIONS,
                "Enable question notifications",
                "Show alerts when input is needed to continue",
            ),
            (
                DOT,
                "Enable dot notifications",
                "Show alerts when your dot sends you a message",
            ),
        ] {
            let control = self.notification_switch(key, disabled, t, window, cx);
            rows.push(settings_row(title, Some(detail), control, t));
        }
        let sound = self.notification_select(SOUND, disabled || state.importing_sound, cx);
        rows.push(settings_row(
            "Notification sound",
            Some("Sound for task completion, permission requests, and questions"),
            sound,
            t,
        ));
        rows
    }

    fn notification_switch(
        &self,
        key: &'static str,
        disabled: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.page.read(cx).flag(key, true);
        let focus = focus::handle(key, 0, window, cx);
        focus::ring(
            switch(key, on, disabled, t)
                .relative()
                .role(gpui::Role::Switch)
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .aria_disabled(disabled),
            &focus,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                this.page
                    .update(cx, |p, cx| p.set(key, Value::Bool(!on), cx));
            }
        }))
        .into_any_element()
    }

    /// The turn mode and sound menus (`d` with `contentWidth: menuWide`).
    fn notification_select(
        &mut self,
        key: &'static str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let setting = self.page.read(cx).get(key).clone();
        let (value, choices) = if key == SOUND {
            let mut choices: Vec<_> = SOUNDS
                .iter()
                .filter(|(value, _)| {
                    cfg!(target_os = "macos") || matches!(*value, "default" | "classic" | "none")
                })
                .map(|&(value, label)| (value.into(), label.into()))
                .collect();
            if let Some(name) = setting["name"].as_str() {
                choices.push((CUSTOM_SOUND.into(), name.to_owned().into()));
            }
            choices.push((CHOOSE_SOUND.into(), "Choose custom sound…".into()));
            (sound_value(&setting).to_owned(), choices)
        } else {
            let value = setting.as_str().unwrap_or("unfocused").to_owned();
            let choices = TURN_MODES
                .iter()
                .map(|&(value, label)| (value.into(), label.into()))
                .collect();
            (value, choices)
        };
        let choices: Vec<(SharedString, SharedString)> = choices;
        let page = self.page.clone();
        let select = self.selects.entry(key).or_insert_with(|| {
            let select = cx.new(|cx| Select::new(value.clone(), Vec::new(), None, cx));
            cx.subscribe(&select, move |_, _, Selected(selected), cx| {
                let current = page.read(cx).get(key).clone();
                let value = match selected.as_ref() {
                    CHOOSE_SOUND => return dispatch(Action::ImportSound, cx),
                    CUSTOM_SOUND => current,
                    value => json!(value),
                };
                page.update(cx, |page, cx| page.set(key, value.clone(), cx));
                if key == SOUND {
                    dispatch(Action::PreviewSound(value), cx);
                }
            })
            .detach();
            select
        });
        select.update(cx, |select, cx| {
            let stale = select.choices().len() != choices.len()
                || select
                    .choices()
                    .iter()
                    .zip(&choices)
                    .any(|(choice, (value, label))| {
                        choice.value != *value || choice.label != *label
                    });
            if stale {
                select.set_choices(
                    choices
                        .into_iter()
                        .map(|(value, label)| Choice {
                            value,
                            label,
                            checked: false,
                            disabled: false,
                        })
                        .collect(),
                    cx,
                );
            }
            select.set_value(value, cx);
            select.set_disabled(disabled, cx);
        });
        select.clone().into_any_element()
    }

    /// The Notifications page (`ct`): the ChatGPT group for a personal ChatGPT account.
    pub(super) fn notifications_page(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        window.use_keyed_state("chatgpt-notifications", cx, |_, cx| {
            dispatch(Action::RefreshAccount, cx);
        });
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        let rows = match state.account {
            Account::Hidden => return Vec::new(),
            Account::Loading => vec![loading_row(t)],
            Account::Error => vec![retry_row(
                "Your account information couldn’t be loaded",
                Action::RefreshAccount,
                state.fetching,
                t,
            )],
            Account::Personal => match &state.channels {
                Load::Loading => vec![loading_row(t)],
                Load::Failed(_) => vec![retry_row(
                    "ChatGPT notification settings could not be loaded",
                    Action::RefreshChannels,
                    state.fetching,
                    t,
                )],
                Load::Ready(channels) => channels["settings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|category| self.category_row(category, &state.contacts, t, cx))
                    .collect(),
            },
        };
        vec![("ChatGPT", rows)]
    }

    /// One category (`Qe`); `teen_safety` (`tt`) offers email and SMS only with a contact.
    fn category_row(
        &mut self,
        category: &Value,
        contacts: &Load<Arc<Value>>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = category["category"].as_str().unwrap_or_default().to_owned();
        let name = category["name"].as_str().unwrap_or_default().to_owned();
        let description = category["description"].as_str().map(str::to_owned);
        let mut options: Vec<&Value> = category["options"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        if id == "teen_safety" {
            match contacts {
                Load::Loading => {
                    return settings_row_content(
                        name,
                        None,
                        icon("sidebar-cmd-spinner", 16., t.text_secondary).into_any_element(),
                        t,
                    );
                }
                Load::Failed(_) => {
                    return settings_row_content(
                        name,
                        Some(alert("Your contact methods couldn’t be loaded", t)),
                        retry(Action::RefreshContacts, false, t),
                        t,
                    );
                }
                Load::Ready(me) => options.retain(|option| match option["channel"].as_str() {
                    Some("email") => me["email"].as_str().is_some_and(|s| !s.is_empty()),
                    Some("sms") => me["phone_number"].as_str().is_some_and(|s| !s.is_empty()),
                    _ => true,
                }),
            }
        }
        let choices: Vec<(String, String, bool)> = options
            .iter()
            .map(|option| {
                (
                    option["channel"].as_str().unwrap_or_default().to_owned(),
                    option["name"].as_str().unwrap_or_default().to_owned(),
                    option["enabled"].as_bool().unwrap_or(false),
                )
            })
            .collect();
        let channels: Vec<String> = choices
            .iter()
            .map(|(channel, ..)| channel.clone())
            .collect();
        let select = match self.channel_selects.get(&id) {
            Some((known, select)) if *known == channels => select.clone(),
            _ => {
                let label = name.clone();
                let menu_choices = choices
                    .iter()
                    .map(|(channel, name, enabled)| Choice {
                        value: channel.clone().into(),
                        label: name.clone().into(),
                        checked: *enabled,
                        disabled: false,
                    })
                    .collect();
                let select =
                    cx.new(|cx| MultiSelect::new(label, menu_choices, cx).summarize("Off"));
                let category = id.clone();
                cx.subscribe(&select, move |_, _, changed: &SelectionChanged, cx| {
                    dispatch(
                        Action::SetChannel {
                            category: category.clone(),
                            channel: changed.value.to_string(),
                            enabled: changed.checked,
                        },
                        cx,
                    );
                })
                .detach();
                self.channel_selects
                    .insert(id.clone(), (channels, select.clone()));
                select
            }
        };
        select.update(cx, |select, cx| {
            for (channel, _, enabled) in &choices {
                select.set_checked(channel, *enabled, cx);
            }
        });
        let detail = description.map(|description| {
            let detail = div()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(t.text_secondary)
                .child(description);
            if id == "tasks" {
                detail
                    .child(
                        div()
                            .id("manage-tasks")
                            .role(gpui::Role::Link)
                            .cursor_pointer()
                            .underline()
                            .child("Manage tasks")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(crate::OpenScheduled), cx)
                            }),
                    )
                    .into_any_element()
            } else {
                detail.into_any_element()
            }
        });
        settings_row_content(name, detail, select.into_any_element(), t)
    }
}

/// `settings-loading-row`: a status row with a spinner and secondary text.
fn loading_row(t: &Theme) -> Div {
    div().child(
        div()
            .id("chatgpt-notifications-loading")
            .role(gpui::Role::Status)
            .px(px(16.))
            .py(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(icon("sidebar-cmd-spinner", 12., t.text_secondary))
            .child(text(
                "Loading ChatGPT notification settings",
                13.,
                18.571_428,
                430.,
                t.text_secondary,
            )),
    )
}

fn alert(message: &'static str, t: &Theme) -> AnyElement {
    div()
        .id(message)
        .role(gpui::Role::Alert)
        .child(text(message, 12., 16., 430., t.text_secondary))
        .into_any_element()
}

/// An error row (`span role=alert` label) with the toolbar `Retry` button (`it`).
fn retry_row(message: &'static str, action: Action, fetching: bool, t: &Theme) -> Div {
    settings_row_content(message, None, retry(action, fetching, t), t)
}

fn retry(action: Action, fetching: bool, t: &Theme) -> AnyElement {
    button("Retry", t.text_alpha_5, None, t)
        .id(SharedString::from(format!("retry-{action:?}")))
        .role(gpui::Role::Button)
        .aria_label("Retry")
        .aria_disabled(fetching)
        .when(fetching, |d| d.opacity(0.5))
        .on_click(move |_, _, cx| {
            if !fetching {
                dispatch(action.clone(), cx);
            }
        })
        .into_any_element()
}
