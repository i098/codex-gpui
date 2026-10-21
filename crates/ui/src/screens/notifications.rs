use super::{self as settings, SettingsPane};
use crate::{composer::popover, focus, icon::icon, text};
use gpui::{
    Anchor, AnyElement, App, Context, Div, Global, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred,
    div, point, prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, sync::Arc};
use theme::Theme;

pub const TURN: &str = "notifications-turn-mode";
pub const PERMISSIONS: &str = "notifications-permissions-enabled";
pub const QUESTIONS: &str = "notifications-questions-enabled";
pub const DOT: &str = "notifications-dot-enabled";
pub const SOUND: &str = "notifications-sound";

#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Ready(T),
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccountGate {
    #[default]
    Hidden,
    Loading,
    Allowed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Channel {
    pub channel: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Category {
    pub category: String,
    pub name: String,
    pub description: String,
    pub options: Vec<Channel>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preferences {
    pub settings: Vec<Category>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Contacts {
    pub email: Option<String>,
    pub phone_number: Option<String>,
}

impl Contacts {
    pub fn allows(&self, channel: &str) -> bool {
        match channel {
            "email" => self.email.as_ref().is_some_and(|v| !v.is_empty()),
            "sms" => self.phone_number.as_ref().is_some_and(|v| !v.is_empty()),
            _ => true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    RefreshAccount,
    LeavePage,
    RefreshPreferences,
    RefreshContacts,
    RefreshPermission,
    OpenSystemSettings,
    SetPreference(&'static str, Value),
    ImportSound,
    UpdateChannel {
        category: String,
        channel: String,
        enabled: bool,
    },
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub permission: Option<bool>,
    pub desktop: Map<String, Value>,
    pub saving_sound: bool,
    pub account: AccountGate,
    pub preferences: Load<Preferences>,
    pub contacts: Load<Contacts>,
    pub generation: u64,
    pub identity: Option<(String, String)>,
    pub next_mutation: u64,
    pub mutations: HashMap<(String, String), u64>,
    pub dispatch: Option<Dispatch>,
}

impl Global for State {}

impl State {
    pub fn channel_mut(&mut self, category: &str, channel: &str) -> Option<&mut Channel> {
        let Load::Ready(preferences) = &mut self.preferences else {
            return None;
        };
        preferences
            .settings
            .iter_mut()
            .find(|item| item.category == category)?
            .options
            .iter_mut()
            .find(|item| item.channel == channel)
    }
}

fn dispatch(action: Action, cx: &mut App) {
    if let Some(dispatch) = cx
        .try_global::<State>()
        .and_then(|state| state.dispatch.clone())
    {
        dispatch(action, cx);
    }
}

pub(super) struct Menu {
    pub key: String,
    pub selected: usize,
}

struct OptionItem {
    label: String,
    checked: bool,
    action: Action,
}

impl SettingsPane {
    fn notification_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: Action,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handle = focus::handle(id, 0, window, cx);
        let focus = handle.read(cx).handle.clone();
        let keyboard_action = action.clone();
        let border = matches!(action, Action::OpenSystemSettings).then_some(t.border);
        focus::ring(
            settings::button(label, t.primary_soft_alpha, border, t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(move |_, window, cx| {
            focus.focus(window, cx);
            dispatch(action.clone(), cx);
        })
        .on_key_down(move |event, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                dispatch(keyboard_action.clone(), cx);
            }
        })
        .into_any_element()
    }

    fn notification_retry(
        &self,
        message: &'static str,
        action: Action,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        settings::settings_row(
            message,
            None,
            self.notification_button("notifications-retry", "Retry", action, t, window, cx),
            t,
        )
    }

    pub(super) fn observe_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.page.read(cx).section == "Notifications";
        if active != self.notifications_page_active {
            self.notifications_page_active = active;
            cx.defer(move |cx| {
                dispatch(
                    if active {
                        Action::RefreshAccount
                    } else {
                        Action::LeavePage
                    },
                    cx,
                )
            });
        }
        if self.notifications_observed {
            return;
        }
        self.notifications_observed = true;
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                dispatch(Action::RefreshPermission, cx);
            }
        })
        .detach();
    }

    pub(super) fn desktop_notifications(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let state = cx.try_global::<State>().cloned().unwrap_or_else(|| State {
            permission: Some(false),
            ..State::default()
        });
        let disabled = state.permission != Some(true);
        let row = |label, description, control| {
            settings::settings_row(label, Some(description), control, t)
        };
        let mut rows = Vec::new();
        if state.permission == Some(false) {
            let control = self.notification_button(
                "notifications-system-settings",
                "Open System Settings",
                Action::OpenSystemSettings,
                t,
                window,
                cx,
            );
            rows.push(row(
                "Notifications are off",
                "Turn on notifications in System Settings to use the options below",
                control,
            ));
        }
        let current = state
            .desktop
            .get(TURN)
            .and_then(Value::as_str)
            .unwrap_or("unfocused");
        let options = [
            ("off", "Never"),
            ("unfocused", "Only when unfocused"),
            ("always", "Always"),
        ];
        let label = options
            .iter()
            .find(|(value, _)| *value == current)
            .map_or("Only when unfocused", |(_, label)| *label);
        rows.push(row(
            "Turn completion notifications",
            "Set when ChatGPT alerts you that it's finished",
            self.notification_select(
                (TURN, label.into()),
                options
                    .into_iter()
                    .map(|(value, label)| OptionItem {
                        label: label.into(),
                        checked: current == value,
                        action: Action::SetPreference(TURN, json!(value)),
                    })
                    .collect(),
                disabled,
                t,
                window,
                cx,
            ),
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
            let checked = state
                .desktop
                .get(key)
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let handle = focus::handle(key, 0, window, cx);
            let toggle = focus::ring(
                settings::switch(key, checked, disabled, t)
                    .role(gpui::Role::Switch)
                    .aria_label(title)
                    .aria_disabled(disabled)
                    .aria_toggled(if checked {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    }),
                &handle,
                Theme::RADIUS_FULL,
                px(2.),
                cx,
            )
            .on_click(move |_, _, cx| {
                if !disabled {
                    dispatch(Action::SetPreference(key, json!(!checked)), cx);
                }
            })
            .on_key_down(move |event, _, cx| {
                if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    dispatch(Action::SetPreference(key, json!(!checked)), cx);
                    cx.stop_propagation();
                }
            });
            rows.push(row(title, detail, toggle.into_any_element()));
        }
        let sound = state
            .desktop
            .get(SOUND)
            .cloned()
            .unwrap_or(json!("default"));
        let mut sounds = vec![
            ("default", "Default"),
            ("classic", "Classic"),
            ("none", "None"),
        ];
        if cfg!(target_os = "macos") {
            sounds.extend([
                ("Basso", "Basso"),
                ("Glass", "Glass"),
                ("Ping", "Ping"),
                ("Pop", "Pop"),
                ("Purr", "Purr"),
                ("Tink", "Tink"),
            ]);
        }
        let label = sound["name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                sounds
                    .iter()
                    .find(|(value, _)| sound == *value)
                    .map_or("Default", |(_, label)| *label)
                    .to_owned()
            });
        let mut options: Vec<_> = sounds
            .into_iter()
            .map(|(value, label)| OptionItem {
                label: label.into(),
                checked: sound == value,
                action: Action::SetPreference(SOUND, json!(value)),
            })
            .collect();
        if let Some(name) = sound["name"].as_str() {
            options.push(OptionItem {
                label: name.into(),
                checked: true,
                action: Action::SetPreference(SOUND, sound.clone()),
            });
        }
        options.push(OptionItem {
            label: "Choose custom sound…".into(),
            checked: false,
            action: Action::ImportSound,
        });
        rows.push(row(
            "Notification sound",
            "Sound for task completion, permission requests, and questions",
            self.notification_select(
                (SOUND, label),
                options,
                disabled || state.saving_sound,
                t,
                window,
                cx,
            ),
        ));
        rows
    }

    pub(super) fn chatgpt_notifications(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        if state.account == AccountGate::Hidden {
            return Vec::new();
        }
        let loading = || {
            div().child(
                div()
                    .id("notifications-loading")
                    .role(gpui::Role::Status)
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("sidebar-cmd-spinner", 14., t.text_secondary))
                    .child(text(
                        "Loading ChatGPT notification settings",
                        13.,
                        18.571_428,
                        430.,
                        t.text_secondary,
                    )),
            )
        };
        let rows = match state.account {
            AccountGate::Loading => vec![loading()],
            AccountGate::Failed => vec![self.notification_retry("Your account information couldn’t be loaded", Action::RefreshAccount, t, window, cx)],
            AccountGate::Hidden => Vec::new(),
            AccountGate::Allowed => match &state.preferences {
                Load::Loading => vec![loading()],
                Load::Failed => vec![self.notification_retry("ChatGPT notification settings could not be loaded", Action::RefreshPreferences, t, window, cx)],
                Load::Ready(preferences) => preferences.settings.iter().map(|category| {
                    if category.category == "teen_safety" {
                        match &state.contacts {
                            Load::Loading => return settings::settings_row(category.name.clone(), None, icon("sidebar-cmd-spinner", 14., t.text_secondary).into_any_element(), t),
                            Load::Failed => return settings::settings_row(category.name.clone(), Some("Your contact methods couldn’t be loaded"),
                                self.notification_button("notification-contacts-retry", "Retry", Action::RefreshContacts, t, window, cx), t),
                            Load::Ready(_) => {}
                        }
                    }
                    let channels: Vec<_> = category.options.iter().filter(|option| category.category != "teen_safety" || matches!(&state.contacts, Load::Ready(contacts) if contacts.allows(&option.channel))).collect();
                    let labels: Vec<_> = channels.iter().filter(|option| option.enabled).map(|option| option.name.as_str()).collect();
                    let label = if labels.is_empty() { "Off".into() } else { labels.join(", ") };
                    let options = channels.into_iter().map(|option| OptionItem { label: option.name.clone(), checked: option.enabled, action: Action::UpdateChannel { category: category.category.clone(), channel: option.channel.clone(), enabled: !option.enabled } }).collect();
                    let select = self.notification_select((&category.category, label), options, false, t, window, cx);
                    let row = settings::settings_row(category.name.clone(), Some(&category.description), select, t);
                    if category.category == "tasks" {
                        row.pb(px(30.)).child(div().absolute().left(px(16.)).bottom(px(12.)).child(text("Manage tasks", 12., 16., 430., t.text).underline())
                            .id("notification-manage-tasks").role(gpui::Role::Link).aria_label("Manage tasks")
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::OpenScheduled), cx)))
                    } else { row }
                }).collect(),
            },
        };
        vec![("ChatGPT", rows)]
    }

    fn notification_select(
        &self,
        selection: (&str, String),
        options: Vec<OptionItem>,
        disabled: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, label) = selection;
        let key = key.to_owned();
        let open = self
            .notification_menu
            .as_ref()
            .is_some_and(|menu| menu.key == key);
        let handle = focus::handle(
            SharedString::from(format!("notification-{key}")),
            0,
            window,
            cx,
        );
        let focus = handle.read(cx).handle.clone();
        let key_click = key.clone();
        let key_keyboard = key.clone();
        let actions: Vec<_> = options.iter().map(|option| option.action.clone()).collect();
        let menu = open.then(|| {
            let rows = options.into_iter().enumerate().map(|(index, option)| {
                let action = option.action;
                div()
                    .id(("notification-option", index))
                    .role(gpui::Role::MenuItemCheckBox)
                    .aria_label(option.label.clone())
                    .aria_toggled(if option.checked {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(
                        self.notification_menu
                            .as_ref()
                            .is_some_and(|menu| menu.selected == index),
                        |d| d.bg(t.menu_item_hover),
                    )
                    .hover(|s| s.bg(t.menu_item_hover))
                    .child(text(option.label, 13., 18.571_428, 430., t.text).flex_1())
                    .child(
                        div()
                            .size(px(16.))
                            .children(option.checked.then(|| icon("menu-check", 16., t.text))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let keep_open = matches!(action, Action::UpdateChannel { .. });
                        dispatch(action.clone(), cx);
                        if !keep_open {
                            this.notification_menu = None;
                        }
                        cx.notify();
                    }))
            });
            deferred(
                div().absolute().top_0().right_0().child(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(32.)))
                        .child(
                            div()
                                .id("notification-menu")
                                .role(gpui::Role::Menu)
                                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                    this.menu_hovered = *hovered
                                }))
                                .child(popover(t, 280., |d| d.children(rows))),
                        ),
                ),
            )
        });
        focus::ring(
            settings::select(&label, None, disabled, t)
                .id(SharedString::from(format!("notification-{key}")))
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if !disabled {
                focus.focus(window, cx);
                this.notification_menu = (!open).then(|| Menu {
                    key: key_click.clone(),
                    selected: 0,
                });
                cx.notify();
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if disabled {
                return;
            }
            let key = event.keystroke.key.as_str();
            if !matches!(key, "enter" | "space" | "up" | "down" | "escape") {
                return;
            }
            cx.stop_propagation();
            if key == "escape" {
                this.notification_menu = None;
            } else if this
                .notification_menu
                .as_ref()
                .is_none_or(|menu| menu.key != key_keyboard)
            {
                this.notification_menu = Some(Menu {
                    key: key_keyboard.clone(),
                    selected: 0,
                });
            } else if let Some(menu) = &mut this.notification_menu {
                if key == "up" {
                    menu.selected = menu.selected.saturating_sub(1);
                } else if key == "down" {
                    menu.selected = (menu.selected + 1).min(actions.len().saturating_sub(1));
                } else if let Some(action) = actions.get(menu.selected) {
                    let keep_open = matches!(action, Action::UpdateChannel { .. });
                    dispatch(action.clone(), cx);
                    if !keep_open {
                        this.notification_menu = None;
                    }
                }
            } else {
                this.notification_menu = Some(Menu {
                    key: key_keyboard.clone(),
                    selected: 0,
                });
            }
            cx.notify();
        }))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if !this.menu_hovered {
                this.notification_menu = None;
                cx.notify();
            }
        }))
        .children(menu)
        .into_any_element()
    }
}
