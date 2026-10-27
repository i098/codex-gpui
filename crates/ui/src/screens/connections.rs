use crate::{focus, icon::icon, text};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Corners, Div, Entity, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

mod ssh;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tab {
    #[default]
    Computer,
    Devices,
    Ssh,
    Wsl,
}

#[derive(Clone, Debug, Default)]
pub struct Gates {
    pub slingshot: bool,
    pub available: Option<bool>,
    pub access_required: bool,
}

impl Gates {
    pub fn remote_control(&self) -> bool {
        self.slingshot && self.available != Some(false) && !self.access_required
    }

    pub fn tab(&self, requested: Tab, has_wsl: bool) -> Tab {
        match requested {
            Tab::Computer | Tab::Devices if !self.remote_control() => Tab::Ssh,
            Tab::Wsl if !has_wsl => Tab::Ssh,
            tab => tab,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Disabled,
    Connecting,
    Connected,
    Errored,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub status: Status,
    pub server_name: String,
    pub installation_id: String,
    pub environment_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub client_id: String,
    pub display_name: Option<String>,
    pub device_model: Option<String>,
    pub device_type: Option<String>,
    pub platform: Option<String>,
    pub last_seen_at: Option<i64>,
}

impl Client {
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.device_model.as_deref())
            .or(self.platform.as_deref())
            .unwrap_or(&self.client_id)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub environment_id: String,
    pub expires_at: i64,
    pub pairing_code: String,
    pub manual_pairing_code: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub host_id: String,
    pub display_name: String,
    pub source: String,
    pub alias: Option<String>,
    pub hostname: Option<String>,
    pub ssh_port: Option<u16>,
    pub identity: Option<String>,
    #[serde(default, skip_serializing)]
    pub auto_connect: bool,
    #[serde(default, skip_serializing)]
    pub state: String,
    #[serde(default, skip_serializing)]
    pub error: Option<String>,
    #[serde(default, skip_serializing)]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing)]
    pub online: bool,
    #[serde(default, skip_serializing)]
    pub app_server_version: Option<String>,
    #[serde(default, skip_serializing)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SshDraft {
    pub host_id: Option<String>,
    pub display_name: String,
    pub hostname: String,
    pub port: String,
    pub identity: String,
    pub use_identity: bool,
    pub alias: bool,
    pub color: Option<Option<String>>,
}

impl SshDraft {
    pub fn errors(&self, existing: &[Connection]) -> impl Iterator<Item = &'static str> {
        let name = self.display_name.trim();
        let hostname = self.hostname.trim();
        let port = self.port.trim();
        let numeric_port = port.bytes().all(|c| c.is_ascii_digit());
        [
            name.is_empty().then_some("Display name is required"),
            hostname.is_empty().then_some("Hostname is required"),
            (hostname.starts_with('-') || hostname.contains(['\n', '\r', '\0']))
                .then_some("Hostname is invalid"),
            (!port.is_empty() && !numeric_port).then_some("SSH port must be an integer"),
            (!port.is_empty()
                && numeric_port
                && port.parse::<u16>().ok().filter(|&p| p != 0).is_none())
            .then_some("SSH port must be between 1 and 65535"),
            (self.use_identity && self.identity.trim().is_empty())
                .then_some("Identity file path is required"),
            existing
                .iter()
                .any(|c| Some(&c.host_id) != self.host_id.as_ref() && c.display_name.trim() == name)
                .then_some("A connection with this display name already exists"),
        ]
        .into_iter()
        .flatten()
    }

    pub fn validate(&self, existing: &[Connection]) -> Result<(), &'static str> {
        self.errors(existing).next().map_or(Ok(()), Err)
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    SetEnabled(bool),
    KeepAwake(bool),
    StartPairing,
    RefreshPairing,
    Revoke(String),
    DiscoverSsh,
    Save(SshDraft),
    SaveAliases(Vec<String>),
    ConnectBatch(Vec<String>),
    Delete(String),
    RenameRemote {
        environment_id: String,
        name: String,
    },
    DeleteRemote(Connection),
    Connect(String, bool),
    Restart(String),
    Logout(String),
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ConnectionsState {
    pub gates: Gates,
    pub requested_tab: Tab,
    pub setup_complete: bool,
    pub active_setup: Option<Entity<super::mobile::MobileSetupPage>>,
    pub status: Option<RemoteStatus>,
    pub clients: Option<Vec<Client>>,
    pub connections: Option<Vec<Connection>>,
    pub remote_devices: Option<Vec<Connection>>,
    pub remote_error: Option<String>,
    pub wsl_error: Option<String>,
    pub aliases: Vec<String>,
    pub aliases_loaded: bool,
    pub pairing: Option<Pairing>,
    pub pairing_error: Option<String>,
    pub pairing_connected: bool,
    pub keep_awake: bool,
    pub pending: bool,
    pub saved_revision: u64,
    pub error: Option<String>,
    pub dispatch: Option<Dispatch>,
    pub generation: u64,
}

impl Global for ConnectionsState {}

pub fn dispatch(action: Action, cx: &mut App) {
    if let Some(send) = cx
        .try_global::<ConnectionsState>()
        .and_then(|state| state.dispatch.clone())
    {
        send(action, cx);
    } else {
        let mut state = cx
            .try_global::<ConnectionsState>()
            .cloned()
            .unwrap_or_default();
        state.error = Some("Connections are unavailable".into());
        cx.set_global(state);
    }
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    let page = window.use_keyed_state("connections-settings", cx, |_, cx| {
        cx.observe_global::<ConnectionsState>(|_, cx| cx.notify())
            .detach();
        let tab = cx
            .try_global::<ConnectionsState>()
            .map_or(Tab::Ssh, |s| s.requested_tab);
        cx.defer(|cx| dispatch(Action::Refresh, cx));
        ConnectionsPage {
            tab,
            editor: None,
            discovery: None,
            menu: None,
            pairing: None,
            layer: Default::default(),
        }
    });
    page.into_any_element()
}

struct ConnectionsPage {
    tab: Tab,
    editor: Option<ssh::Editor>,
    discovery: Option<ssh::Discovery>,
    menu: Option<crate::screens::thread::view::menu::ContextMenu>,
    pairing: Option<Entity<super::mobile::MobileSetupPage>>,
    layer: super::settings::Layer,
}

impl ConnectionsPage {
    fn button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        action: Action,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.into();
        let t = cx.theme().clone();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            super::settings::button(label, t.text_alpha_5, None, &t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .when(disabled, |d| d.opacity(0.4)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if !disabled {
                this.activate(action.clone(), window, cx);
            }
        }))
        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
            if !disabled && matches!(e.keystroke.key.as_str(), "enter" | "space") {
                this.activate(key_action.clone(), window, cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(action, Action::StartPairing) {
            let setup = !cx.global::<ConnectionsState>().setup_complete;
            cx.update_global::<ConnectionsState, _>(|state, _| {
                state.pairing_connected = false;
                state.pairing = None;
                state.pairing_error = None;
            });
            let page = cx.new(if setup {
                super::mobile::MobileSetupPage::setup_dialog
            } else {
                super::mobile::MobileSetupPage::pairing_dialog
            });
            if let Some(focus) = page.read(cx).dialog_focus() {
                focus.focus(window, cx);
            }
            cx.update_global::<ConnectionsState, _>(|state, _| {
                state.active_setup = Some(page.clone())
            });
            self.pairing = Some(page);
            if setup {
                cx.notify();
                return;
            }
        }
        dispatch(action, cx);
    }

    fn close_pairing(&mut self, cx: &mut Context<Self>) {
        self.pairing = None;
        cx.update_global::<ConnectionsState, _>(|state, _| {
            state.pairing = None;
            state.pairing_connected = false;
            state.active_setup = None;
        });
        cx.notify();
    }

    fn toggle(
        &self,
        control: (&'static str, &'static str),
        checked: bool,
        action: Action,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, label) = control;
        let t = cx.theme().clone();
        let handle = focus::handle(id, 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            super::settings::switch(id, checked, disabled, &t)
                .role(gpui::Role::Switch)
                .aria_label(label)
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
        .on_click(move |_, _, cx| {
            if !disabled {
                dispatch(action.clone(), cx);
            }
        })
        .on_key_down(move |e: &KeyDownEvent, _, cx| {
            if !disabled && matches!(e.keystroke.key.as_str(), "enter" | "space") {
                dispatch(key_action.clone(), cx);
                cx.stop_propagation();
            }
        })
        .into_any_element()
    }

    fn computer(
        &self,
        state: &ConnectionsState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        if !state.setup_complete {
            return self.empty_connections(&t, cx);
        }
        let enabled = state
            .status
            .as_ref()
            .is_some_and(|s| matches!(s.status, Status::Connecting | Status::Connected));
        let toggle = self.toggle(
            ("connections-enabled", "Allow connections"),
            enabled,
            Action::SetEnabled(!enabled),
            state.pending,
            window,
            cx,
        );
        let mut rows = vec![row(
            "Allow connections",
            Some(
                "Authorized devices on your ChatGPT account can discover and control this device"
                    .into(),
            ),
            toggle,
            &t,
        )];
        if enabled {
            match &state.clients {
                None => rows.push(message("Loading device list", &t)),
                Some(clients) => {
                    for client in clients {
                        let detail = client
                            .last_seen_at
                            .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                            .map(|date| format!("Last connected {}", date.format("%b %-d, %Y")))
                            .unwrap_or_else(|| "Authorized device".into());
                        let button = self.button(
                            format!("revoke-{}", client.client_id),
                            "Revoke access",
                            Action::Revoke(client.client_id.clone()),
                            state.pending,
                            window,
                            cx,
                        );
                        rows.push(row(client.label(), Some(detail), button, &t));
                    }
                    let button = self.button(
                        "add-device",
                        "Add",
                        Action::StartPairing,
                        state.pending,
                        window,
                        cx,
                    );
                    if clients.is_empty() {
                        rows.push(row(
                            "Add device to control this Mac remotely",
                            Some("Control this Mac from your phone or other device".into()),
                            button,
                            &t,
                        ));
                    } else {
                        rows.push(row("Devices that can control this Mac", None, button, &t));
                    }
                }
            }
        }
        let awake = self.toggle(
            ("connections-keep-awake", "Keep this Mac awake"),
            state.keep_awake,
            Action::KeepAwake(!state.keep_awake),
            state.pending,
            window,
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(card("Devices that can control this Mac", rows, &t))
            .child(card(
                "Other settings",
                vec![row(
                    "Keep this Mac awake",
                    Some(
                        "Prevent sleep when computer is plugged in and remote access is enabled"
                            .into(),
                    ),
                    awake,
                    &t,
                )],
                &t,
            ))
    }

    fn activate_menu(
        &mut self,
        action: &'static str,
        host: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = cx.global::<ConnectionsState>();
        let connection = state
            .connections
            .iter()
            .chain(state.remote_devices.iter())
            .flat_map(|rows| rows.iter())
            .find(|row| row.host_id == host)
            .cloned();
        match action {
            "details" => {
                self.open_details(connection.as_ref(), window, cx);
            }
            "edit" => self.open_editor(connection.as_ref(), window, cx),
            "delete" => match connection {
                Some(connection) if connection.source == "remote-control" => {
                    dispatch(Action::DeleteRemote(connection), cx)
                }
                _ => dispatch(Action::Delete(host), cx),
            },
            "restart" => dispatch(Action::Restart(host), cx),
            "logout" => dispatch(Action::Logout(host), cx),
            _ => {}
        }
    }

    fn hosts(&self, state: &ConnectionsState, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut rows = Vec::new();
        let error = match self.tab {
            Tab::Devices => state.remote_error.as_ref(),
            Tab::Wsl => state.wsl_error.as_ref(),
            _ => None,
        };
        if let Some(error) = error {
            return div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text(
                    if self.tab == Tab::Wsl {
                        "Could not discover WSL connections"
                    } else {
                        "Couldn’t load devices"
                    },
                    14.,
                    21.,
                    500.,
                    t.text,
                ))
                .child(text(error.clone(), 13., 20., 430., t.text_error))
                .child(self.button(
                    "retry-remote-devices",
                    "Refresh",
                    Action::Refresh,
                    state.pending,
                    window,
                    cx,
                ));
        }
        if self.tab == Tab::Ssh
            && state
                .connections
                .as_ref()
                .is_some_and(|rows| rows.is_empty())
        {
            return self.empty_connections(&t, cx);
        }
        let connections = if self.tab == Tab::Devices {
            &state.remote_devices
        } else {
            &state.connections
        };
        match connections {
            None => rows.push(message("Loading SSH connections…", &t)),
            Some(connections) => {
                for connection in connections.iter().filter(|c| match self.tab {
                    Tab::Wsl => c.source == "wsl",
                    Tab::Devices => c.source == "remote-control",
                    _ => c.source != "wsl" && c.source != "remote-control",
                }) {
                    let id = connection.host_id.clone();
                    let checked = connection.state == "connected";
                    let state_label = match connection.state.as_str() {
                        "connected" => "Connected",
                        "connecting" => "Connecting",
                        "error" => "Connection failed",
                        _ => "Disconnected",
                    };
                    let control = self.button(
                        format!("connect-{id}"),
                        if checked { "Disconnect" } else { "Connect" },
                        Action::Connect(id.clone(), !checked),
                        state.pending,
                        window,
                        cx,
                    );
                    let menu_host = id.clone();
                    let remote = connection.source == "remote-control";
                    let can_delete = !remote || !connection.online;
                    let menu = div()
                        .id(SharedString::from(format!("actions-{id}")))
                        .role(gpui::Role::Button)
                        .aria_label(format!("{} actions", connection.display_name))
                        .focusable()
                        .tab_index(0)
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("more-horizontal", 16., t.text))
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, window, cx| {
                                use crate::screens::thread::view::menu::{ContextMenu, Item};
                                this.menu = Some(ContextMenu::open(
                                    vec![
                                        Item::new("details", "Details", true),
                                        Item::new(
                                            "edit",
                                            if remote { "Rename" } else { "Edit connection" },
                                            true,
                                        ),
                                        Item::new("delete", "Delete", can_delete),
                                        Item::new("restart", "Restart connection", !remote),
                                        Item::new("logout", "Log out", !remote),
                                    ],
                                    menu_host.clone(),
                                    event.position(),
                                    window,
                                    cx,
                                ));
                                cx.notify();
                            },
                        ));
                    rows.push(row(
                        &connection.display_name,
                        Some(
                            connection
                                .error
                                .clone()
                                .unwrap_or_else(|| state_label.into()),
                        ),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(control)
                            .child(menu)
                            .into_any_element(),
                        &t,
                    ));
                }
                if rows.is_empty() {
                    rows.push(message(
                        if self.tab == Tab::Devices {
                            "No devices added yet"
                        } else {
                            "No SSH connections found"
                        },
                        &t,
                    ));
                }
            }
        }
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(text(
                match self.tab {
                    Tab::Wsl => "WSL connections",
                    Tab::Devices => "Devices you can control from this Mac",
                    _ => "SSH connections from this Mac",
                },
                14.,
                21.,
                500.,
                t.text,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(self.button(
                        "refresh-connections",
                        "Refresh",
                        Action::Refresh,
                        state.pending,
                        window,
                        cx,
                    ))
                    .child(
                        super::settings::button("Add", t.text_alpha_5, None, &t)
                            .id("add-ssh-connection")
                            .role(gpui::Role::Button)
                            .aria_label("Add SSH connection")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_ssh_discovery(window, cx)
                            })),
                    ),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(header)
            .child(card("", rows, &t))
    }

    fn empty_connections(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let ssh = self.tab == Tab::Ssh;
        let (title, description, glyph, width) = if ssh {
            (
                "SSH connections from this Mac",
                "Connect to a remote device through SSH connection",
                "connections-empty-ssh",
                66.,
            )
        } else {
            (
                "Devices that can control this Mac",
                "Add device to control this Mac remotely",
                "connections-empty-devices",
                61. * 24. / 23.,
            )
        };
        let foreground = if t.appearance == theme::Appearance::Dark {
            gpui::rgb(0x2d2d2d).into()
        } else {
            t.text_inverse
        };
        let add = div()
            .id("connections-empty-add")
            .role(gpui::Role::Button)
            .aria_label("Add")
            .focusable()
            .tab_index(0)
            .h(px(24.))
            .px(px(9.))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(9999.)),
                fill: Some(t.button_primary_solid),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(text("Add", 13., 18., 430., foreground))
            .on_click(cx.listener(move |this, _, window, cx| {
                if ssh {
                    this.open_ssh_discovery(window, cx);
                } else {
                    this.activate(Action::StartPairing, window, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    if ssh {
                        this.open_ssh_discovery(window, cx);
                    } else {
                        this.activate(Action::StartPairing, window, cx);
                    }
                    cx.stop_propagation();
                }
            }));
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(46.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .child(text(title, 14., 21., 500., t.text)),
            )
            .child(
                div()
                    .relative()
                    .p(px(1.))
                    .child(super::settings::deco(
                        &self.layer,
                        20.,
                        shapes::Corner::Squircle,
                        t.background_panel,
                        Some(t.border),
                    ))
                    .child(
                        div()
                            .w_full()
                            .px(px(12.))
                            .py(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(576.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(crate::icon::icon_sized(glyph, width, 24., t.text))
                                    .child(text(
                                        description,
                                        13.,
                                        18.571_428,
                                        430.,
                                        gpui::Hsla { a: 0.65, ..t.text },
                                    ))
                                    .child(add),
                            ),
                    ),
            )
    }
}

impl Render for ConnectionsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx
            .try_global::<ConnectionsState>()
            .cloned()
            .unwrap_or_default();
        self.sync_ssh_dialogs(&state, window, cx);
        let t = cx.theme().clone();
        let has_wsl = state
            .connections
            .as_ref()
            .is_some_and(|rows| rows.iter().any(|c| c.source == "wsl"));
        self.tab = state.gates.tab(state.requested_tab, has_wsl);
        let mut tabs = vec![(Tab::Ssh, "SSH")];
        if state.gates.remote_control() {
            tabs.splice(
                0..0,
                [
                    (Tab::Computer, "Control this Mac"),
                    (Tab::Devices, "Control other devices"),
                ],
            );
        }
        if has_wsl {
            tabs.push((Tab::Wsl, "WSL"));
        }
        let tabbar = div()
            .flex()
            .gap(px(2.))
            .children(tabs.into_iter().map(|(tab, label)| {
                connection_tab(label, self.tab == tab, &t)
                    .id(label)
                    .role(gpui::Role::Tab)
                    .aria_label(label)
                    .aria_selected(self.tab == tab)
                    .focusable()
                    .tab_index(0)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.update_global::<ConnectionsState, _>(|state, _| {
                            state.requested_tab = tab
                        });
                        dispatch(Action::Refresh, cx);
                        cx.notify();
                    }))
            }));
        let content = if self.tab == Tab::Computer {
            self.computer(&state, window, cx)
        } else {
            self.hosts(&state, window, cx)
        };
        let menu = self.menu.as_ref().map(|menu| {
            menu.overlay(
                "connection-menu",
                "Connection actions",
                |this| &mut this.menu,
                Self::activate_menu,
                window,
                cx,
            )
            .into_any_element()
        });
        let editor = self.editor(window, cx);
        let discovery = self.discovery_dialog(window, cx);
        let pairing = self.pairing.as_ref().map(|page| {
            let viewport = window.viewport_size();
            let frame = div()
                .id("connections-setup-frame")
                .relative()
                .w(px(420.))
                .max_w(viewport.width * 0.92)
                .max_h(viewport.height - px(32.))
                .role(gpui::Role::Dialog)
                .aria_label("Connect a device to this Mac")
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(shapes::backdrop_blur(
                    px(24.),
                    shapes::Fill::Color,
                    false,
                    shapes::Decoration {
                        radius: Corners::all(px(25.)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(gpui::Hsla {
                            a: 0.9,
                            ..t.menu_background
                        }),
                        shadows: vec![
                            crate::composer::shadow(t.border, 0., 0., 0.5, false),
                            crate::composer::shadow(
                                gpui::rgba(0x0000001a).into(),
                                4.,
                                8.,
                                -2.,
                                false,
                            ),
                        ],
                        ..Default::default()
                    },
                ))
                .child(crate::rounded_clip(
                    px(25.),
                    shapes::Corner::Squircle,
                    page.clone(),
                ));
            gpui::deferred(
                gpui::anchored()
                    .position(gpui::point(px(0.), px(0.)))
                    .child(
                        div()
                            .w(viewport.width)
                            .h(viewport.height)
                            .occlude()
                            .bg(gpui::rgba(0x00000022))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_action(cx.listener(
                                |this, _: &super::mobile::FinishSetup, _, cx| {
                                    this.close_pairing(cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if event.keystroke.key == "escape" {
                                    this.close_pairing(cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.close_pairing(cx)),
                            )
                            .child(frame),
                    ),
            )
            .with_priority(10)
            .into_any_element()
        });
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("connections-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(super::settings::probe(&self.layer))
                    .child(
                        div()
                            .pt(px(16.))
                            .pb(px(1.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div().w_full().max_w(px(768.)).px(px(20.)).child(
                                    div()
                                        .min_h(px(60.))
                                        .py(px(12.))
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .child(text("Connections", 28., 33.6, 500., t.text)),
                                ),
                            ),
                    )
                    .child(
                        div().flex().flex_col().items_center().child(
                            div()
                                .w_full()
                                .max_w(px(768.))
                                .p(px(20.))
                                .flex()
                                .flex_col()
                                .gap(px(40.))
                                .child(tabbar)
                                .children(
                                    state.error.as_ref().map(|error| {
                                        text(error.clone(), 13., 18., 430., t.text_error)
                                    }),
                                )
                                .child(content),
                        ),
                    ),
            )
            .children(menu)
            .children(editor)
            .children(discovery)
            .children(pairing)
    }
}

fn connection_tab(label: &'static str, selected: bool, t: &Theme) -> Div {
    div()
        .h(px(28.))
        .px(px(9.))
        .relative()
        .flex()
        .items_center()
        .when(selected, |tab| {
            tab.child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(12.5)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.text_alpha_5),
                ..Default::default()
            }))
        })
        .child(text(
            label,
            13.,
            18.,
            430.,
            if selected { t.text } else { t.text_tertiary },
        ))
}

fn message(label: &str, t: &Theme) -> Div {
    div()
        .p(px(24.))
        .child(text(label.to_owned(), 13., 20., 430., t.text_secondary))
}
fn row(title: &str, detail: Option<String>, control: AnyElement, t: &Theme) -> Div {
    div()
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
                .child(text(title.to_owned(), 13., 18.571_428, 500., t.text))
                .children(detail.map(|text_value| {
                    text(text_value, 12., 16., 430., t.text_secondary).whitespace_normal()
                })),
        )
        .child(control)
}
fn card(title: &str, rows: Vec<Div>, t: &Theme) -> Div {
    let last = rows.len().saturating_sub(1);
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .when(!title.is_empty(), |d| {
            d.child(text(title.to_owned(), 14., 21., 500., t.text))
        })
        .child(
            div()
                .relative()
                .p(px(1.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(20.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.background_panel),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .children(rows.into_iter().enumerate().map(|(i, r)| {
                            r.when(i < last, |r| r.border_b(px(1.)).border_color(t.border))
                        })),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_editor_rejects_invalid_ports_and_duplicate_names() {
        let existing = vec![Connection {
            host_id: "one".into(),
            display_name: "Build".into(),
            ..Default::default()
        }];
        let mut draft = SshDraft {
            display_name: "Build".into(),
            hostname: "example.invalid".into(),
            ..Default::default()
        };
        assert_eq!(
            draft.validate(&existing),
            Err("A connection with this display name already exists")
        );
        draft.host_id = Some("one".into());
        for port in ["0", "65536", "99999999999999999999"] {
            draft.port = port.into();
            assert_eq!(
                draft.validate(&existing),
                Err("SSH port must be between 1 and 65535")
            );
        }
        draft.port = "22.5".into();
        assert_eq!(
            draft.validate(&existing),
            Err("SSH port must be an integer")
        );
        draft.port = "65535".into();
        assert!(draft.validate(&existing).is_ok());
        draft.hostname = "-oProxyCommand=bad".into();
        assert_eq!(draft.validate(&existing), Err("Hostname is invalid"));
    }
    #[test]
    fn remote_tabs_obey_the_original_access_gate() {
        let mut gates = Gates {
            slingshot: true,
            ..Default::default()
        };
        assert_eq!(gates.tab(Tab::Computer, false), Tab::Computer);
        gates.access_required = true;
        assert_eq!(gates.tab(Tab::Computer, false), Tab::Ssh);
        gates.access_required = false;
        gates.available = Some(false);
        assert_eq!(gates.tab(Tab::Devices, true), Tab::Ssh);
        assert_eq!(gates.tab(Tab::Wsl, true), Tab::Wsl);
        assert_eq!(gates.tab(Tab::Wsl, false), Tab::Ssh);
    }

    #[test]
    fn new_pairing_dialog_takes_focus_without_previous_success() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        use gpui::{HeadlessAppContext, size};
        let mut cx =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(Theme::dark());
            cx.set_global(ConnectionsState {
                pairing_connected: true,
                ..Default::default()
            });
        });
        let page = cx.update(|cx| {
            cx.new(|_| ConnectionsPage {
                tab: Tab::Computer,
                editor: None,
                discovery: None,
                menu: None,
                pairing: None,
                layer: Default::default(),
            })
        });
        let window = cx
            .open_window(size(px(840.), px(650.)), |_, _| page.clone())
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                page.activate(Action::StartPairing, window, cx);
                let focus = page
                    .pairing
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .dialog_focus()
                    .unwrap();
                assert!(focus.is_focused(window));
                assert!(!cx.global::<ConnectionsState>().pairing_connected);
                page.close_pairing(cx);
                assert!(!cx.global::<ConnectionsState>().pairing_connected);
            })
        })
        .unwrap();
    }
}
