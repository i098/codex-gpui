use super::computer_use::Load;
use super::{Layer, button, deco, on_card, probe, switch};
use crate::{composer::popover, focus, icon::icon, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use shapes::Corner::Squircle;
use std::{
    collections::{BTreeMap, HashSet},
    rc::Rc,
    sync::Arc,
};
use theme::{ActiveTheme as _, Theme};

gpui::actions!(
    settings_integrations,
    [OpenPluginManager, OpenPluginCreator]
);

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = settings_integrations, no_json)]
pub struct OpenExtensionDirectory(pub &'static str);

#[path = "settings_mcp_form.rs"]
mod form;

#[path = "settings_marketplace_form.rs"]
mod marketplace_form;

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    ReloadHooks,
    ReloadServers,
    SetServerEnabled {
        key: String,
        enabled: bool,
    },
    SaveServer {
        key: String,
        config: Value,
        expected_version: String,
    },
    RemoveServer(String),
    Authenticate {
        name: String,
        registration: &'static str,
    },
    SetHookEnabled {
        key: String,
        enabled: bool,
    },
    TrustHooks(Vec<(String, String)>),
    SetPluginEnabled {
        plugin: Value,
        enabled: bool,
    },
    RemovePlugin(Value),
    RemoveMarketplace(String),
    UpgradeMarketplace(String),
    AddMarketplace(Value),
    SetAppEnabled {
        id: String,
        enabled: bool,
    },
    SetSkillEnabled {
        path: String,
        enabled: bool,
    },
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub configuration: Arc<Value>,
    pub plugin_creator: Option<String>,
    pub config_error: Option<String>,
    pub servers: Load<Arc<Value>>,
    pub plugins: Load<Arc<Value>>,
    pub marketplaces: Load<Arc<Value>>,
    pub hooks: Load<Arc<Value>>,
    pub apps: Load<Arc<Value>>,
    pub skills: Load<Arc<Value>>,
    pub roots: Arc<Vec<String>>,
    pub evaluations: Arc<Value>,
    pub hidden_plugins: Arc<HashSet<String>>,
    pub gates_loaded: bool,
    pub pending: bool,
    pub refresh_requested: bool,
    pub reload_required: bool,
    pub form_error: Option<String>,
    pub operation_error: Option<String>,
    pub form_saved: u64,
    pub marketplace_added: u64,
    pub oauth: BTreeMap<String, Option<String>>,
    pub suppressed_servers: HashSet<String>,
    pub removal_completed: Option<String>,
    pub dispatch: Option<Dispatch>,
}

pub fn render(
    section: &'static str,
    state: Entity<State>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let page = window.use_keyed_state(
        SharedString::from(format!("settings-integrations-{section}")),
        cx,
        move |_, cx| {
            cx.observe(&state, |this: &mut Manager, state, cx| {
                let state = state.read(cx);
                let saved = state.form_saved;
                if saved != this.form_saved {
                    this.form = None;
                    this.form_saved = saved;
                }
                let added = state.marketplace_added;
                if added != this.marketplace_added {
                    this.marketplace_form = None;
                    this.marketplace_added = added;
                    this.tab = Tab::Marketplace;
                }
                if this.marketplace.as_ref() == state.removal_completed.as_ref() {
                    this.marketplace = None;
                }
                cx.notify();
            })
            .detach();
            Manager {
                section,
                tab: Tab::Plugins,
                query: String::new(),
                focus: cx.focus_handle().tab_stop(true),
                layer: Layer::default(),
                menu: None,
                form: None,
                form_saved: state.read(cx).form_saved,
                marketplace_form: None,
                marketplace_added: state.read(cx).marketplace_added,
                marketplace: None,
                selected_hooks: None,
                modal_focus: cx.focus_handle(),
                return_focus: None,
                expanded_hooks: HashSet::new(),
                registrations: BTreeMap::new(),
                state,
            }
        },
    );
    page.into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Plugins,
    Apps,
    Mcps,
    Skills,
    Marketplace,
}

#[derive(Clone)]
enum Menu {
    Add,
    OAuth(String),
}

struct Manager {
    state: Entity<State>,
    section: &'static str,
    tab: Tab,
    query: String,
    focus: gpui::FocusHandle,
    layer: Layer,
    menu: Option<Menu>,
    form: Option<Entity<form::Form>>,
    form_saved: u64,
    marketplace_form: Option<Entity<marketplace_form::MarketplaceForm>>,
    marketplace_added: u64,
    marketplace: Option<String>,
    selected_hooks: Option<String>,
    modal_focus: gpui::FocusHandle,
    return_focus: Option<gpui::FocusHandle>,
    expanded_hooks: HashSet<String>,
    registrations: BTreeMap<String, &'static str>,
}

pub(super) fn dispatch(state: &Entity<State>, action: Action, cx: &mut App) {
    if let Some(dispatch) = state.read(cx).dispatch.clone() {
        dispatch(action, cx);
    }
}

pub fn server_hidden(name: &str) -> bool {
    matches!(name, "codex_app" | "cua_repl")
}

pub fn configured_servers(configuration: &Value) -> impl Iterator<Item = (&str, &Value)> {
    configuration["config"]["mcp_servers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, config)| (name.as_str(), config))
        .filter(|(name, _)| !server_hidden(name))
}

pub fn plugin_name(plugin: &Value) -> &str {
    plugin["interface"]["displayName"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| plugin["name"].as_str())
        .unwrap_or("")
}

pub fn visible_plugin(plugin: &Value, hidden: &HashSet<String>) -> bool {
    let id = plugin["id"].as_str().unwrap_or("");
    let name = id.split('@').next().unwrap_or("");
    !matches!(name, "chrome" | "chrome-dev" | "chrome-internal")
        && !matches!(
            id,
            "codex-app-tools@openai-bundled" | "unified-computer-use@openai-bundled"
        )
        && !hidden.contains(id)
        && !plugin["remotePluginId"]
            .as_str()
            .is_some_and(|id| hidden.contains(id))
}

pub fn needs_review(hook: &Value) -> bool {
    matches!(hook["trustStatus"].as_str(), Some("untrusted" | "modified"))
}

fn hook_enabled(hook: &Value) -> bool {
    hook["isManaged"] == true || (hook["enabled"] == true && !needs_review(hook))
}

pub fn server_read_only(configuration: &Value, key: &str) -> bool {
    let plain = format!("mcp_servers.{key}.");
    let quoted = format!(
        "mcp_servers.{}.",
        serde_json::to_string(key).unwrap_or_default()
    );
    configuration["origins"]
        .as_object()
        .into_iter()
        .flatten()
        .any(|(path, origin)| {
            (path.starts_with(&plain) || path.starts_with(&quoted))
                && origin["name"]["type"] == "project"
        })
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn plugins(value: &Value) -> impl Iterator<Item = &Value> {
    array(value, "marketplaces")
        .iter()
        .flat_map(|marketplace| array(marketplace, "plugins"))
}

fn standalone_skills(value: &Value) -> Vec<&Value> {
    let mut selected = BTreeMap::<&str, &Value>::new();
    let priority = |skill: &Value| match skill["scope"].as_str() {
        Some("repo") => 0,
        Some("user") => 1,
        Some("system") => 2,
        _ => 3,
    };
    for skill in array(value, "data")
        .iter()
        .flat_map(|entry| array(entry, "skills"))
        .filter(|skill| skill["pluginId"].is_null())
    {
        let Some(name) = skill["name"].as_str() else {
            continue;
        };
        let keep = selected.get(name).is_none_or(|current| {
            (priority(skill), skill["path"].as_str())
                < (priority(current), current["path"].as_str())
        });
        if keep {
            selected.insert(name, skill);
        }
    }
    selected.into_values().collect()
}

pub(super) fn action_button(
    id: impl Into<SharedString>,
    label: &'static str,
    t: &Theme,
) -> Stateful<Div> {
    button(label, t.primary_soft_alpha, None, t)
        .id(id.into())
        .role(gpui::Role::Button)
        .aria_label(label)
}

pub(super) trait Activate: Sized {
    fn on_activate<T: 'static>(
        self,
        window: &mut Window,
        cx: &mut Context<T>,
        handler: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Self;
}

impl Activate for Stateful<Div> {
    fn on_activate<T: 'static>(
        self,
        window: &mut Window,
        cx: &mut Context<T>,
        handler: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Self {
        let id = gpui::Element::id(&self).expect("stateful control has an ID");
        let focus = focus::handle(id, 0, window, cx);
        let click = Rc::new(handler);
        let key = click.clone();
        focus::ring(self, &focus, px(12.5), px(2.), cx)
            .on_click(cx.listener(move |this, _, window, cx| click(this, window, cx)))
            .on_key_down(cx.listener(move |this, event, window, cx| {
                focus::tab(event, window, cx);
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    key(this, window, cx);
                    cx.stop_propagation();
                }
            }))
    }
}

pub(super) fn card<E: IntoElement>(
    rows: impl IntoIterator<Item = E>,
    layer: &Layer,
    t: &Theme,
) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .p(px(1.))
        .child(deco(
            layer,
            16.,
            Squircle,
            t.background_panel,
            Some(t.border),
        ))
        .children(rows)
}

fn section_card(title: &str, rows: impl IntoIterator<Item = Div>, layer: &Layer, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(text(title.to_owned(), 14., 21., 500., t.text))
        .child(card(rows, layer, t))
}

fn message(title: &str, detail: Option<&str>, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .items_center()
        .py(px(32.))
        .child(text(title.to_owned(), 14., 21., 500., t.text))
        .children(detail.map(|detail| text(detail.to_owned(), 13., 18., 430., t.text_secondary)))
}

impl Manager {
    fn search_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.platform || event.keystroke.modifiers.control {
            return;
        }
        match (event.keystroke.key.as_str(), &event.keystroke.key_char) {
            ("backspace", _) => {
                self.query.pop();
            }
            ("escape", _) => self.query.clear(),
            (_, Some(value)) => self.query.push_str(value),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.query.clear();
        self.form = None;
        self.menu = None;
        cx.notify();
    }

    fn open_form(&mut self, key: Option<String>, config: Value, cx: &mut Context<Self>) {
        self.tab = Tab::Mcps;
        self.menu = None;
        let form = cx.new(|cx| form::Form::new(key, config, self.state.clone(), cx));
        cx.subscribe(&form, |this, _, _: &form::Close, cx| {
            this.form = None;
            cx.notify();
        })
        .detach();
        self.form = Some(form);
        cx.notify();
    }

    fn tab_counts(&self, state: &State) -> [(Tab, &'static str, usize); 5] {
        let plugin_count = match &state.plugins {
            Load::Ready(value) => plugins(value)
                .filter(|plugin| visible_plugin(plugin, &state.hidden_plugins))
                .count(),
            _ => 0,
        };
        let app_count = match &state.apps {
            Load::Ready(value) => array(value, "data")
                .iter()
                .filter(|app| app["isAccessible"] == true)
                .count(),
            _ => 0,
        };
        let skill_count = match &state.skills {
            Load::Ready(value) => standalone_skills(value).len(),
            _ => 0,
        };
        [
            (Tab::Plugins, "Plugins", plugin_count),
            (Tab::Apps, "Apps", app_count),
            (
                Tab::Mcps,
                "MCPs",
                configured_servers(&state.configuration).count(),
            ),
            (Tab::Skills, "Skills", skill_count),
            (
                Tab::Marketplace,
                "Marketplace",
                self.marketplaces(state).len(),
            ),
        ]
    }

    fn toolbar(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let counts = self.tab_counts(state);
        let plugins_enabled = state.configuration["config"]["features"]["plugins"] != false;
        let tabs = counts
            .into_iter()
            .filter(|(tab, _, count)| *tab == Tab::Mcps || (plugins_enabled && *count > 0))
            .map(|(tab, label, count)| {
                div()
                    .id(label)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .aria_selected(self.tab == tab)
                    .h(px(28.))
                    .px(px(9.))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(self.tab == tab, |d| {
                        d.child(on_card(12.5, Squircle, t.primary_soft_alpha, None))
                    })
                    .child(text(
                        label,
                        13.,
                        18.,
                        430.,
                        if self.tab == tab {
                            t.text
                        } else {
                            t.text_tertiary
                        },
                    ))
                    .child(text(count.to_string(), 13., 18., 430., t.text_tertiary))
                    .on_activate(window, cx, move |this, _, cx| this.select_tab(tab, cx))
            });
        let placeholder = match self.tab {
            Tab::Plugins => "Search plugins",
            Tab::Apps => "Search apps",
            Tab::Mcps => "Search MCP servers",
            Tab::Skills => "Search skills",
            Tab::Marketplace => "Search marketplaces",
        };
        div()
            .flex()
            .justify_between()
            .items_center()
            .gap(px(12.))
            .child(div().flex().items_center().children(tabs))
            .child(
                div()
                    .id("integrations-search")
                    .role(gpui::Role::TextInput)
                    .aria_label(placeholder)
                    .track_focus(&self.focus)
                    .w(px(224.))
                    .h(px(32.))
                    .relative()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(on_card(12., Squircle, t.primary_soft_alpha, Some(t.border)))
                    .child(icon("search", 16., t.text_tertiary))
                    .child(text(
                        if self.query.is_empty() {
                            placeholder.to_owned()
                        } else {
                            self.query.clone()
                        },
                        14.,
                        18.,
                        430.,
                        if self.query.is_empty() {
                            t.text_tertiary
                        } else {
                            t.text
                        },
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
                    .on_key_down(cx.listener(Self::search_key)),
            )
    }

    fn server_row(
        &self,
        key: &str,
        details: (Option<&Value>, Option<&Value>),
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (config, status) = details;
        let name = config.and_then(|c| c["name"].as_str()).unwrap_or(key);
        let enabled = config.is_none_or(|config| config["enabled"] != false);
        let read_only = server_read_only(&state.configuration, key);
        let mut controls = self.authentication_controls(key, status, state, t, window, cx);
        if let Some(config) = config {
            let settings_key = key.to_owned();
            let value = config.clone();
            controls = controls.child(
                div()
                    .id(SharedString::from(format!("server-settings-{key}")))
                    .role(gpui::Role::Button)
                    .aria_label(format!("Settings for {name}"))
                    .aria_disabled(read_only)
                    .when(read_only, |d| d.opacity(0.4))
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("settings-configuration", 16., t.text_secondary))
                    .on_activate(window, cx, move |this, _, cx| {
                        if !read_only {
                            this.open_form(Some(settings_key.clone()), value.clone(), cx);
                        }
                    }),
            );
            let key = key.to_owned();
            let control = switch(
                SharedString::from(format!("server-enabled-{key}")),
                enabled,
                read_only || state.pending,
                t,
            )
            .role(gpui::Role::Switch)
            .aria_label(format!("Enable {name}"))
            .aria_toggled(if enabled {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .aria_disabled(read_only || state.pending)
            .on_activate(window, cx, move |this, _, cx| {
                if !read_only {
                    dispatch(
                        &this.state,
                        Action::SetServerEnabled {
                            key: key.clone(),
                            enabled: !enabled,
                        },
                        cx,
                    );
                }
            });
            controls = controls.child(control);
        }
        div()
            .min_h(px(if config.is_some() { 52. } else { 42.571_43 }))
            .px(px(16.))
            .py(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(text(name.to_owned(), 13., 18.571_428, 500., t.text))
            .child(controls)
    }

    fn authentication_controls(
        &self,
        key: &str,
        status: Option<&Value>,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let authenticate = status.is_some_and(|status| status["authStatus"] == "notLoggedIn");
        let mut controls = div().flex().items_center().gap(px(12.));
        if authenticate {
            let registration = self.registrations.get(key).copied().unwrap_or("auto");
            let label = match registration {
                "cimd" => "CIMD",
                "dcr" => "DCR",
                _ => "Auto",
            };
            let server = key.to_owned();
            controls = controls.child(
                action_button(format!("registration-{key}"), label, t)
                    .aria_label("OAuth client registration")
                    .child(icon("chevron-down", 14., t.text_secondary))
                    .on_activate(window, cx, move |this, _, cx| {
                        this.menu = Some(Menu::OAuth(server.clone()));
                        cx.notify();
                    }),
            );
            let server = key.to_owned();
            let pending = state.oauth.get(key).is_some_and(Option::is_none);
            controls = controls.child(
                action_button(
                    format!("authenticate-{key}"),
                    if pending {
                        "Starting…"
                    } else {
                        "Authenticate"
                    },
                    t,
                )
                .aria_disabled(pending)
                .when(pending, |d| d.opacity(0.4))
                .on_activate(window, cx, move |this, _, cx| {
                    if !pending {
                        dispatch(
                            &this.state,
                            Action::Authenticate {
                                name: server.clone(),
                                registration,
                            },
                            cx,
                        );
                    }
                }),
            );
        }
        controls
    }

    fn mcp_list(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        if let Some(error) = &state.config_error {
            return self.load_error("MCP server settings are unavailable", error, t, window, cx);
        }
        let (custom, provided) = self.server_rows(state, t, window, cx);
        let mut body = div().flex().flex_col().gap(px(32.));
        match &state.servers {
            Load::Loading => return body.child(message("Loading MCP servers…", None, t)),
            Load::Failed(error) => {
                return body.child(self.load_error(
                    "Could not load MCP servers",
                    error,
                    t,
                    window,
                    cx,
                ));
            }
            Load::Ready(_) => {}
        }
        let empty = custom.is_empty() && provided.is_empty();
        if !custom.is_empty() {
            body = body.child(section_card("Servers", custom, &self.layer, t));
        } else if empty {
            body = body.child(section_card(
                "Servers",
                [div()
                    .p(px(16.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text(
                        if self.query.is_empty() {
                            "No MCP servers connected"
                        } else {
                            "No MCP servers found"
                        },
                        13.,
                        18.571_428,
                        500.,
                        t.text,
                    ))
                    .child(
                        action_button("add-mcp-server", "Add server", t).on_activate(
                            window,
                            cx,
                            |this, _, cx| {
                                this.open_form(None, serde_json::json!({"command":""}), cx)
                            },
                        ),
                    )],
                &self.layer,
                t,
            ));
        }
        if !provided.is_empty() {
            body = body.child(section_card("From plugins", provided, &self.layer, t));
        }
        if state.reload_required {
            body = body.child(action_button("reload-mcp", "Restart", t).on_activate(
                window,
                cx,
                |this, _, cx| dispatch(&this.state, Action::ReloadServers, cx),
            ));
        }
        body
    }

    fn server_rows(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Vec<Div>, Vec<Div>) {
        let query = self.query.to_lowercase();
        let mut configured = configured_servers(&state.configuration).collect::<Vec<_>>();
        configured.sort_by_key(|(key, config)| {
            (
                config["enabled"] == false,
                config["name"].as_str().unwrap_or(key).to_lowercase(),
            )
        });
        let statuses = match &state.servers {
            Load::Ready(value) => array(value, "data"),
            _ => &[],
        };
        let custom = configured
            .iter()
            .filter(|(key, config)| {
                key.to_lowercase().contains(&query)
                    || config["name"]
                        .as_str()
                        .is_some_and(|name| name.to_lowercase().contains(&query))
            })
            .map(|(key, config)| {
                self.server_row(
                    key,
                    (Some(config), statuses.iter().find(|s| s["name"] == *key)),
                    state,
                    t,
                    window,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let provided = statuses
            .iter()
            .filter(|status| {
                let name = status["name"].as_str().unwrap_or("");
                !server_hidden(name)
                    && !state.suppressed_servers.contains(name)
                    && !configured
                        .iter()
                        .any(|(key, config)| *key == name || config["name"] == name)
                    && name.to_lowercase().contains(&query)
            })
            .map(|status| {
                self.server_row(
                    status["name"].as_str().unwrap_or(""),
                    (None, Some(status)),
                    state,
                    t,
                    window,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        (custom, provided)
    }

    fn load_error(
        &self,
        title: &str,
        error: &str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        message(title, Some(error), t).child(
            action_button("integrations-retry", "Retry", t).on_activate(
                window,
                cx,
                |this, _, cx| dispatch(&this.state, Action::Refresh, cx),
            ),
        )
    }

    fn plugin_list(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let value = match &state.plugins {
            Load::Loading => return message("Loading installed plugins…", None, t),
            Load::Failed(error) => {
                return self.load_error("Unable to load installed plugins", error, t, window, cx);
            }
            Load::Ready(value) => value,
        };
        let query = self.query.to_lowercase();
        let mut plugins = plugins(value)
            .filter(|plugin| {
                visible_plugin(plugin, &state.hidden_plugins)
                    && (plugin_name(plugin).to_lowercase().contains(&query)
                        || plugin["interface"]["shortDescription"]
                            .as_str()
                            .is_some_and(|description| description.to_lowercase().contains(&query)))
            })
            .collect::<Vec<_>>();
        plugins.sort_by(|a, b| {
            b["installedAt"]
                .as_str()
                .cmp(&a["installedAt"].as_str())
                .then_with(|| plugin_name(a).cmp(plugin_name(b)))
        });
        if plugins.is_empty() {
            return message("No plugins found", None, t);
        }
        let rows = plugins.into_iter().map(|plugin| {
            let name = plugin_name(plugin).to_owned();
            let enabled = plugin["enabled"] == true;
            let value = plugin.clone();
            let id = plugin["id"].as_str().unwrap_or("").to_owned();
            let toggle = switch(
                SharedString::from(format!("plugin-enabled-{id}")),
                enabled,
                state.pending,
                t,
            )
            .role(gpui::Role::Switch)
            .aria_label(format!("Enable {name}"))
            .aria_toggled(if enabled {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .aria_disabled(state.pending)
            .on_activate(window, cx, move |this, _, cx| {
                dispatch(
                    &this.state,
                    Action::SetPluginEnabled {
                        plugin: value.clone(),
                        enabled: !enabled,
                    },
                    cx,
                )
            });
            let name_for_menu = id.clone();
            let mut logo = div()
                .size(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center();
            if let Some(url) = plugin["iconDataUrl"].as_str()
                && let Some(image) = super::computer_use::data_image(
                    url,
                    format!("plugin-icon-{id}").into(),
                    window,
                    cx,
                )
            {
                logo = logo.child(gpui::img(image).size_full());
            } else {
                logo = logo.child(icon("settings-plugins", 28., t.text_secondary));
            }
            div()
                .id(SharedString::from(format!("plugin-{id}")))
                .min_h(px(68.))
                .px(px(8.))
                .py(px(12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(logo)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(text(name.clone(), 14., 20., 500., t.text))
                        .child(text(
                            plugin["interface"]["shortDescription"]
                                .as_str()
                                .unwrap_or("")
                                .to_owned(),
                            13.,
                            18.,
                            430.,
                            t.text_secondary,
                        )),
                )
                .child(toggle)
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, _, window, cx| {
                        this.begin_modal(window, cx);
                        this.menu = None;
                        this.marketplace = Some(format!("plugin:{name_for_menu}"));
                        cx.notify();
                    }),
                )
        });
        div().flex().flex_col().children(rows)
    }

    fn app_list(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let value = match &state.apps {
            Load::Loading => return message("Loading apps…", None, t),
            Load::Failed(error) => {
                return self.load_error("Unable to load apps", error, t, window, cx);
            }
            Load::Ready(value) => value,
        };
        let query = self.query.to_lowercase();
        let apps = array(value, "data")
            .iter()
            .filter(|app| {
                app["isAccessible"] == true
                    && [
                        app["name"].as_str().unwrap_or(""),
                        app["description"].as_str().unwrap_or(""),
                    ]
                    .join(" ")
                    .to_lowercase()
                    .contains(&query)
            })
            .collect::<Vec<_>>();
        if apps.is_empty() {
            return message("No installed apps", None, t);
        }
        div()
            .flex()
            .flex_col()
            .children(apps.into_iter().map(|app| {
                let id = app["id"].as_str().unwrap_or("").to_owned();
                let name = app["name"].as_str().unwrap_or("").to_owned();
                let enabled = app["isEnabled"] != false;
                let toggle = switch(
                    SharedString::from(format!("app-enabled-{id}")),
                    enabled,
                    state.pending,
                    t,
                )
                .role(gpui::Role::Switch)
                .aria_label(format!("Enable {name}"))
                .aria_disabled(state.pending)
                .aria_toggled(if enabled {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .on_activate(window, cx, move |this, _, cx| {
                    dispatch(
                        &this.state,
                        Action::SetAppEnabled {
                            id: id.clone(),
                            enabled: !enabled,
                        },
                        cx,
                    )
                });
                div()
                    .min_h(px(68.))
                    .px(px(8.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon("settings-connections", 28., t.text_secondary))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(text(name, 14., 20., 500., t.text))
                            .child(text(
                                app["description"].as_str().unwrap_or("").to_owned(),
                                13.,
                                18.,
                                430.,
                                t.text_secondary,
                            )),
                    )
                    .child(toggle)
            }))
    }

    fn skill_list(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let value = match &state.skills {
            Load::Loading => return message("Loading skills…", None, t),
            Load::Failed(error) => {
                return self.load_error("Unable to load skills", error, t, window, cx);
            }
            Load::Ready(value) => value,
        };
        let query = self.query.to_lowercase();
        let skills = standalone_skills(value)
            .into_iter()
            .filter(|skill| {
                [
                    skill["name"].as_str().unwrap_or(""),
                    skill["description"].as_str().unwrap_or(""),
                ]
                .join(" ")
                .to_lowercase()
                .contains(&query)
            })
            .collect::<Vec<_>>();
        if skills.is_empty() {
            return message("No skills found", None, t);
        }
        div()
            .flex()
            .flex_col()
            .children(skills.into_iter().map(|skill| {
                let path = skill["path"].as_str().unwrap_or("").to_owned();
                let name = skill["interface"]["displayName"]
                    .as_str()
                    .or_else(|| skill["name"].as_str())
                    .unwrap_or("")
                    .to_owned();
                let description = skill["interface"]["shortDescription"]
                    .as_str()
                    .or_else(|| skill["shortDescription"].as_str())
                    .or_else(|| skill["description"].as_str())
                    .unwrap_or("")
                    .to_owned();
                let enabled = skill["enabled"] == true;
                let managed = skill["scope"] == "admin" || skill["scope"] == "system";
                let toggle = switch(
                    SharedString::from(format!("skill-enabled-{path}")),
                    enabled,
                    managed || state.pending,
                    t,
                )
                .role(gpui::Role::Switch)
                .aria_label(format!("Enable {name}"))
                .aria_disabled(managed || state.pending)
                .aria_toggled(if enabled {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .on_activate(window, cx, move |this, _, cx| {
                    if !managed {
                        dispatch(
                            &this.state,
                            Action::SetSkillEnabled {
                                path: path.clone(),
                                enabled: !enabled,
                            },
                            cx,
                        );
                    }
                });
                div()
                    .min_h(px(68.))
                    .px(px(8.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon("settings-configuration", 28., t.text_secondary))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(text(name, 14., 20., 500., t.text))
                            .child(text(description, 13., 18., 430., t.text_secondary)),
                    )
                    .child(toggle)
            }))
    }

    fn marketplaces<'a>(&self, state: &'a State) -> Vec<&'a Value> {
        match &state.marketplaces {
            Load::Ready(value) => array(value, "marketplaces")
                .iter()
                .filter(|marketplace| {
                    marketplace["isBuiltIn"] != true
                        && marketplace["name"].as_str().is_some_and(|name| {
                            state.configuration["config"]["marketplaces"]
                                .get(name)
                                .is_some()
                        })
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn marketplace_list(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match &state.marketplaces {
            Load::Loading => return message("Loading marketplaces…", None, t),
            Load::Failed(error) => {
                return self.load_error("Unable to load marketplaces", error, t, window, cx);
            }
            _ => {}
        }
        let entries = self.marketplaces(state);
        if entries.is_empty() {
            return message("No marketplaces found", None, t);
        }
        div().flex().flex_col().gap(px(8.)).children(
            entries
                .into_iter()
                .filter(|entry| {
                    entry["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&self.query.to_lowercase())
                })
                .map(|entry| {
                    let name = entry["name"].as_str().unwrap_or("").to_owned();
                    let label = entry["interface"]["displayName"]
                        .as_str()
                        .unwrap_or(&name)
                        .to_owned();
                    let upgrade_name = name.clone();
                    let configured = &state.configuration["config"]["marketplaces"][&name];
                    let can_upgrade = configured["source_type"] == "git"
                        && configured["source"]
                            .as_str()
                            .is_some_and(|source| !source.is_empty());
                    div()
                        .px(px(8.))
                        .py(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(icon("settings-plugins", 24., t.text_secondary))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(text(label, 14., 20., 500., t.text))
                                .child(text(
                                    format!("{} plugins", array(entry, "plugins").len()),
                                    13.,
                                    18.,
                                    430.,
                                    t.text_secondary,
                                )),
                        )
                        .child(
                            action_button(
                                format!("upgrade-marketplace-{upgrade_name}"),
                                "Upgrade",
                                t,
                            )
                            .aria_disabled(!can_upgrade || state.pending)
                            .when(!can_upgrade || state.pending, |d| d.opacity(0.4))
                            .on_activate(
                                window,
                                cx,
                                move |this, _, cx| {
                                    if can_upgrade {
                                        dispatch(
                                            &this.state,
                                            Action::UpgradeMarketplace(upgrade_name.clone()),
                                            cx,
                                        );
                                    }
                                },
                            ),
                        )
                        .child(
                            action_button(
                                format!("remove-marketplace-{name}"),
                                "Remove marketplace",
                                t,
                            )
                            .on_activate(
                                window,
                                cx,
                                move |this, window, cx| {
                                    this.begin_modal(window, cx);
                                    this.marketplace = Some(name.clone());
                                    cx.notify();
                                },
                            ),
                        )
                }),
        )
    }

    fn hooks(&self, state: &State, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let value = match &state.hooks {
            Load::Loading => return message("Loading hooks…", None, t),
            Load::Failed(error) => {
                return self.load_error("Could not load hooks", error, t, window, cx);
            }
            Load::Ready(value) => value,
        };
        let hooks = unique_hooks(value);
        if hooks.is_empty()
            && array(value, "data").iter().all(|entry| {
                array(entry, "errors").is_empty() && array(entry, "warnings").is_empty()
            })
        {
            return card(
                [div()
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text("No hooks found", 13., 18.571_428, 500., t.text))
                    .child(text(
                        "Configured hooks will appear here",
                        12.,
                        16.,
                        430.,
                        t.text_secondary,
                    ))],
                &self.layer,
                t,
            );
        }
        let mut sources = BTreeMap::<String, usize>::new();
        for hook in hooks {
            *sources.entry(hook_source(hook)).or_default() += 1;
        }
        let mut body = div().flex().flex_col().gap(px(24.));
        for group in [
            "From Config",
            "From Plugins",
            "From Projects",
            "Other sources",
        ] {
            let rows = sources
                .iter()
                .filter(|(source, _)| hook_source_group(source) == group)
                .map(|(source, count)| {
                    let target = source.clone();
                    div()
                        .id(SharedString::from(format!("hook-source-{source}")))
                        .role(gpui::Role::Button)
                        .aria_label(hook_source_title(source, state))
                        .flex()
                        .items_center()
                        .p(px(16.))
                        .gap(px(12.))
                        .child(icon("settings-hooks", 20., t.text_secondary))
                        .child(div().flex_1().child(text(
                            hook_source_title(source, state),
                            14.,
                            21.,
                            500.,
                            t.text,
                        )))
                        .child(text(
                            format!("{count} {}", if *count == 1 { "hook" } else { "hooks" }),
                            13.,
                            18.,
                            430.,
                            t.text_secondary,
                        ))
                        .child(icon("chevron-right", 16., t.text_secondary))
                        .on_activate(window, cx, move |this, window, cx| {
                            this.begin_modal(window, cx);
                            this.selected_hooks = Some(target.clone());
                            this.expanded_hooks.clear();
                            cx.notify();
                        })
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                body = body.child(section_card(
                    group,
                    rows.into_iter().map(|row| div().child(row)),
                    &self.layer,
                    t,
                ));
            }
        }
        for entry in array(value, "data") {
            for warning in array(entry, "warnings") {
                body = body.child(text(
                    warning.as_str().unwrap_or("").to_owned(),
                    13.,
                    18.,
                    430.,
                    t.text_secondary,
                ));
            }
            for error in array(entry, "errors") {
                body = body.child(text(
                    format!(
                        "{}: {}",
                        error["path"].as_str().unwrap_or(""),
                        error["message"].as_str().unwrap_or("")
                    ),
                    13.,
                    18.,
                    430.,
                    t.text_error,
                ));
            }
        }
        body
    }

    fn begin_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.return_focus = window.focused(cx);
        self.modal_focus.focus(window, cx);
        self.state.update(cx, |state, cx| {
            state.operation_error = None;
            state.removal_completed = None;
            cx.notify();
        });
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marketplace = None;
        self.selected_hooks = None;
        self.marketplace_form = None;
        if let Some(focus) = self.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn hook_details(
        &self,
        source: &str,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Div, Vec<String>) {
        let hooks = match &state.hooks {
            Load::Ready(value) => unique_hooks(value),
            _ => Vec::new(),
        };
        let mut hooks = hooks
            .into_iter()
            .filter(|hook| hook_source(hook) == source)
            .collect::<Vec<_>>();
        hooks.sort_by_key(|hook| hook["displayOrder"].as_i64().unwrap_or_default());
        let mut controls = vec!["hooks-close".to_owned()];
        let mut body = div()
            .w(px(896.))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text(
                        hook_source_title(source, state),
                        20.,
                        28.,
                        500.,
                        t.text,
                    ))
                    .child(action_button("hooks-close", "Close", t).on_activate(
                        window,
                        cx,
                        |this, window, cx| this.close_modal(window, cx),
                    )),
            );
        let trust = hooks
            .iter()
            .filter(|hook| hook["isManaged"] != true && needs_review(hook))
            .filter_map(|hook| {
                Some((
                    hook["key"].as_str()?.to_owned(),
                    hook["currentHash"].as_str()?.to_owned(),
                ))
            })
            .collect::<Vec<_>>();
        if !trust.is_empty() {
            controls.push("hooks-trust-all".into());
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(
                        text(
                            "Hooks run outside of the sandbox and may be unsafe",
                            13.,
                            18.,
                            430.,
                            t.text_secondary,
                        )
                        .whitespace_normal(),
                    )
                    .child(
                        action_button("hooks-trust-all", "Trust all", t)
                            .aria_disabled(state.pending)
                            .on_activate(window, cx, move |this, _, cx| {
                                dispatch(&this.state, Action::TrustHooks(trust.clone()), cx)
                            }),
                    ),
            );
        }
        let mut events = Vec::new();
        for hook in &hooks {
            let event = hook["eventName"].as_str().unwrap_or("");
            if !events.contains(&event) {
                events.push(event);
            }
        }
        for event in events {
            let (label, description) = hook_event(event);
            let mut rows = Vec::new();
            for (index, hook) in hooks
                .iter()
                .filter(|hook| hook["eventName"] == event)
                .enumerate()
            {
                rows.push(self.hook_row((index, hook), &mut controls, state, t, window, cx));
            }
            body = body.child(card(
                [div()
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text(label.to_owned(), 13., 18., 500., t.text))
                    .child(text(description, 12., 16., 430., t.text_secondary))]
                .into_iter()
                .chain(rows),
                &self.layer,
                t,
            ));
        }
        if let Some(error) = &state.operation_error {
            body =
                body.child(text(error.clone(), 13., 18., 430., t.text_error).whitespace_normal());
        }
        (body, controls)
    }

    fn hook_row(
        &self,
        entry: (usize, &Value),
        controls: &mut Vec<String>,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (index, hook) = entry;
        let key = hook["key"].as_str().unwrap_or("").to_owned();
        let expanded = self.expanded_hooks.contains(&key);
        let title = hook["statusMessage"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map_or_else(
                || format!("Hook {}", index + 1),
                |status| format!("{} - {status}", index + 1),
            );
        let expand_id = format!("hook-expand-{key}");
        controls.push(expand_id.clone());
        let expand_key = key.clone();
        let controls_row = Self::hook_controls(
            (key.as_str(), hook, title.as_str()),
            controls,
            state,
            t,
            window,
            cx,
        );
        let mut row = div().flex().flex_col().px(px(12.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(8.))
                .child(
                    div()
                        .id(SharedString::from(expand_id))
                        .role(gpui::Role::Button)
                        .aria_label(title.clone())
                        .aria_expanded(expanded)
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(text(title, 13., 18., 430., t.text))
                        .child(icon(
                            if expanded {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                            14.,
                            t.text_secondary,
                        ))
                        .on_activate(window, cx, move |this, _, cx| {
                            if !this.expanded_hooks.remove(&expand_key) {
                                this.expanded_hooks.insert(expand_key.clone());
                            }
                            cx.notify();
                        }),
                )
                .child(controls_row),
        );
        if expanded {
            row = row.child(Self::hook_body(hook, t));
        }
        row
    }

    fn hook_controls(
        details: (&str, &Value, &str),
        controls: &mut Vec<String>,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (key, hook, title) = details;
        let managed = hook["isManaged"] == true;
        let review = needs_review(hook);
        let enabled = hook_enabled(hook);
        let disabled = managed || review || state.pending;
        let toggle_id = format!("hook-enabled-{key}");
        controls.push(toggle_id.clone());
        let toggle_key = key.to_owned();
        let toggle = switch(SharedString::from(toggle_id), enabled, disabled, t)
            .role(gpui::Role::Switch)
            .aria_label(title.to_owned())
            .aria_disabled(disabled)
            .aria_toggled(if enabled {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .on_activate(window, cx, move |this, _, cx| {
                if !managed && !review {
                    dispatch(
                        &this.state,
                        Action::SetHookEnabled {
                            key: toggle_key.clone(),
                            enabled: !enabled,
                        },
                        cx,
                    );
                }
            });
        let mut controls_row = div().flex().items_center().gap(px(8.));
        if !managed && review {
            let id = format!("trust-{key}");
            controls.push(id.clone());
            let hash = hook["currentHash"].as_str().unwrap_or("").to_owned();
            let trust_key = key.to_owned();
            controls_row = controls_row.child(
                action_button(id, "Trust", t)
                    .aria_disabled(state.pending)
                    .on_activate(window, cx, move |this, _, cx| {
                        dispatch(
                            &this.state,
                            Action::TrustHooks(vec![(trust_key.clone(), hash.clone())]),
                            cx,
                        )
                    }),
            );
        }
        controls_row = controls_row.child(toggle);
        controls_row
    }

    fn hook_body(hook: &Value, t: &Theme) -> Div {
        let detail = if hook["handlerType"] == "command" {
            format!("Command: {}", hook["command"].as_str().unwrap_or(""))
        } else if hook["handlerType"] == "mcpTool" {
            format!(
                "MCP server: {}\\nTool: {}",
                hook["server"].as_str().unwrap_or(""),
                hook["tool"].as_str().unwrap_or("")
            )
        } else {
            format!("Handler: {}", hook["handlerType"].as_str().unwrap_or(""))
        };
        div()
            .px(px(28.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                text(detail, 12., 16., 430., t.text)
                    .font_family(crate::MONO)
                    .whitespace_normal(),
            )
            .children(hook["matcher"].as_str().map(|matcher| {
                text(
                    format!("Matcher: {matcher}"),
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                )
            }))
            .child(text(
                format!(
                    "Timeout: {}s",
                    hook["timeoutSec"].as_u64().unwrap_or_default()
                ),
                12.,
                16.,
                430.,
                t.text_secondary,
            ))
    }

    fn modal(
        &self,
        content: (String, Div),
        controls: Vec<gpui::FocusHandle>,
        pending: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, content) = content;
        gpui::deferred(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(gpui::black().opacity(0.4))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if !pending {
                            this.close_modal(window, cx);
                        }
                    }),
                )
                .child(
                    div()
                        .id("settings-manager-dialog")
                        .role(gpui::Role::Dialog)
                        .aria_label(title)
                        .max_w_full()
                        .max_h(px(f32::from(window.viewport_size().height) - 64.))
                        .relative()
                        .track_focus(&self.modal_focus)
                        .overflow_y_scroll()
                        .capture_key_down(cx.listener(
                            move |this, event: &KeyDownEvent, window, cx| {
                                let key = &event.keystroke;
                                if key.key == "escape" && !pending {
                                    this.close_modal(window, cx);
                                    cx.stop_propagation();
                                }
                                if key.key == "tab"
                                    && !key.modifiers.platform
                                    && !key.modifiers.control
                                    && !key.modifiers.alt
                                    && !controls.is_empty()
                                {
                                    let current = controls
                                        .iter()
                                        .position(|control| control.is_focused(window));
                                    let next = if key.modifiers.shift {
                                        current.map_or(controls.len() - 1, |index| {
                                            (index + controls.len() - 1) % controls.len()
                                        })
                                    } else {
                                        current.map_or(0, |index| (index + 1) % controls.len())
                                    };
                                    controls[next].focus(window, cx);
                                    cx.stop_propagation();
                                }
                            },
                        ))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(on_card(24., Squircle, t.background_panel, Some(t.border)))
                        .child(content),
                ),
        )
        .into_any_element()
    }

    fn overlay(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some(form) = &self.marketplace_form {
            let controls = form.update(cx, |form, cx| form.controls(window, cx));
            return Some(self.modal(
                ("Add plugin marketplace".into(), div().child(form.clone())),
                controls,
                state.pending,
                t,
                window,
                cx,
            ));
        }
        if let Some(source) = &self.selected_hooks {
            let (content, controls) = self.hook_details(source, state, t, window, cx);
            return Some(self.modal(
                (hook_source_title(source, state), content),
                control_handles(controls, window, cx),
                state.pending,
                t,
                window,
                cx,
            ));
        }
        if let Some(name) = &self.marketplace {
            return Some(self.removal_dialog(name, state, t, window, cx));
        }
        self.menu
            .as_ref()
            .map(|menu| self.menu_overlay(menu, state, t, window, cx))
    }

    fn removal_dialog(
        &self,
        name: &str,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let plugin_id = name.strip_prefix("plugin:");
        let listed = match &state.plugins {
            Load::Ready(value) => Some(value),
            _ => None,
        };
        let affected = listed
            .into_iter()
            .flat_map(|value| array(value, "marketplaces"))
            .filter(|marketplace| plugin_id.is_some() || marketplace["name"] == name)
            .flat_map(|marketplace| array(marketplace, "plugins"))
            .filter(|plugin| plugin_id.is_none_or(|id| plugin["id"] == id))
            .collect::<Vec<_>>();
        let title = if let Some(id) = plugin_id {
            format!(
                "Uninstall {}?",
                affected
                    .iter()
                    .find(|p| p["id"] == id)
                    .map_or(id, |p| plugin_name(p))
            )
        } else {
            format!("Remove \"{name}\"?")
        };
        let empty = affected.is_empty();
        let action = match (state.operation_error.is_some(), plugin_id.is_some(), empty) {
            (true, _, _) => "Retry",
            (false, true, _) => "Uninstall",
            (false, false, true) => "Remove",
            (false, false, false) => "Uninstall & remove",
        };
        let target = name.to_owned();
        let plugin = affected.first().map(|value| (*value).clone());
        let title_label = title.clone();
        let pending = state.pending;
        let confirm = div().w(px(448.)).p(px(24.)).flex().flex_col().gap(px(16.)).relative()
                .children(state.operation_error.as_ref().map(|error| text(error.clone(), 13., 18., 430., t.text_error).whitespace_normal()))
                .child(text(title, 18., 24., 600., t.text))
                .child(text(if empty { "Its plugins will not appear or be available for installation unless you add it again" }
                    else { "The following plugins will be uninstalled, and can't be re-installed unless you add this marketplace again." }, 13., 18., 430., t.text_secondary).whitespace_normal())
                .children(affected.iter().map(|plugin| text(plugin_name(plugin).to_owned(), 14., 20., 430., t.text)))
                .child(div().flex().justify_end().gap(px(8.))
                    .child(action_button("cancel-removal", "Cancel", t).aria_disabled(pending).on_activate(window, cx, move |this, window, cx| { if !pending { this.close_modal(window, cx); } }))
                    .child(action_button("confirm-removal", action, t).aria_disabled(pending).on_activate(window, cx, move |this, _, cx| {
                        if pending { return; }
                        this.dispatch_removal(&target, plugin.as_ref(), cx);
                    })));
        self.modal(
            (title_label, confirm),
            control_handles(
                vec!["cancel-removal".into(), "confirm-removal".into()],
                window,
                cx,
            ),
            pending,
            t,
            window,
            cx,
        )
    }

    fn dispatch_removal(&self, target: &str, plugin: Option<&Value>, cx: &mut App) {
        let action = match plugin {
            Some(plugin) if target.starts_with("plugin:") => Action::RemovePlugin(plugin.clone()),
            _ => Action::RemoveMarketplace(target.to_owned()),
        };
        dispatch(&self.state, action, cx);
    }

    fn menu_overlay(
        &self,
        menu: &Menu,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = match menu {
            Menu::Add => vec![
                ("Create plugin", None),
                ("Add a marketplace", None),
                ("Add MCP server", None),
            ],
            Menu::OAuth(name) => vec![
                ("Automatic", Some((name.clone(), "auto"))),
                (
                    "Client ID metadata document (CIMD)",
                    Some((name.clone(), "cimd")),
                ),
                (
                    "Dynamic client registration (DCR)",
                    Some((name.clone(), "dcr")),
                ),
            ],
        };
        let can_create_plugin = state.plugin_creator.is_some();
        let content =
            div()
                .w(px(260.))
                .flex()
                .flex_col()
                .p(px(5.))
                .children(options.into_iter().map(|(label, oauth)| {
                    div()
                        .id(label)
                        .role(gpui::Role::MenuItem)
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(10.))
                        .hover(|style| style.bg(t.menu_item_hover))
                        .child(text(label, 13., 18., 430., t.text))
                        .aria_disabled(label == "Create plugin" && !can_create_plugin)
                        .when(label == "Create plugin" && !can_create_plugin, |d| {
                            d.opacity(0.4)
                        })
                        .on_activate(window, cx, move |this, window, cx| {
                            if label == "Create plugin" && !can_create_plugin {
                                return;
                            }
                            this.menu = None;
                            if label == "Create plugin" {
                                window.dispatch_action(Box::new(OpenPluginCreator), cx);
                            } else if let Some((name, registration)) = &oauth {
                                this.registrations.insert(name.clone(), *registration);
                            } else if label == "Add MCP server" {
                                this.open_form(None, serde_json::json!({"command":""}), cx);
                            } else {
                                this.begin_modal(window, cx);
                                let form = cx.new(|cx| {
                                    marketplace_form::MarketplaceForm::new(this.state.clone(), cx)
                                });
                                cx.subscribe(&form, |this, _, _: &form::Close, cx| {
                                    this.marketplace_form = None;
                                    cx.notify();
                                })
                                .detach();
                                this.marketplace_form = Some(form);
                            }
                            cx.notify();
                        })
                }));
        div()
            .absolute()
            .top(px(120.))
            .right(px(40.))
            .child(popover(t, 260., |d| d.child(content)))
            .into_any_element()
    }

    fn select_available_tab(&mut self, state: &State) {
        let counts = self.tab_counts(state);
        let plugins_enabled = state.configuration["config"]["features"]["plugins"] != false;
        if !plugins_enabled {
            self.tab = Tab::Mcps;
        } else if self.tab != Tab::Mcps
            && counts
                .iter()
                .any(|(tab, _, count)| *tab == self.tab && *count == 0)
            && !matches!(&state.plugins, Load::Loading)
        {
            self.tab = counts
                .iter()
                .find(|(_, _, count)| *count > 0)
                .map_or(Tab::Mcps, |(tab, _, _)| *tab);
        }
    }

    fn header_actions(
        &self,
        hooks: bool,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut actions = div().flex().items_center().gap(px(8.));
        if hooks {
            let disabled = state.pending || state.roots.is_empty();
            actions = actions.child(
                div()
                    .id("reload-hooks")
                    .role(gpui::Role::Button)
                    .aria_label("Reload hooks")
                    .aria_disabled(disabled)
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(disabled, |d| d.opacity(0.4))
                    .child(icon("diff-refresh", 16., t.text_secondary))
                    .on_activate(window, cx, move |this, _, cx| {
                        if !disabled {
                            dispatch(&this.state, Action::ReloadHooks, cx);
                        }
                    }),
            );
        } else {
            actions =
                actions
                    .child(
                        action_button("browse-extension-directory", "Browse directory", t)
                            .on_activate(window, cx, {
                                let path = if self.tab == Tab::Skills {
                                    "/skills"
                                } else {
                                    "/plugins"
                                };
                                move |_, window, cx| {
                                    window
                                        .dispatch_action(Box::new(OpenExtensionDirectory(path)), cx)
                                }
                            }),
                    )
                    .child(action_button("integrations-add", "Add", t).on_activate(
                        window,
                        cx,
                        |this, _, cx| {
                            this.menu = Some(Menu::Add);
                            cx.notify();
                        },
                    ));
        }
        actions
    }

    fn extension_body(
        &self,
        state: &State,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(form) = &self.form {
            form.clone().into_any_element()
        } else {
            match self.tab {
                Tab::Plugins => self.plugin_list(state, t, window, cx),
                Tab::Apps => self.app_list(state, t, window, cx),
                Tab::Mcps => self.mcp_list(state, t, window, cx),
                Tab::Skills => self.skill_list(state, t, window, cx),
                Tab::Marketplace => self.marketplace_list(state, t, window, cx),
            }
            .into_any_element()
        }
    }
}

fn control_handles(ids: Vec<String>, window: &mut Window, cx: &mut App) -> Vec<gpui::FocusHandle> {
    ids.into_iter()
        .map(|id| {
            focus::handle(SharedString::from(id), 0, window, cx)
                .read(cx)
                .handle
                .clone()
        })
        .collect()
}

pub fn plugin_creator_path(skills: &Value) -> Option<String> {
    array(skills, "data")
        .iter()
        .flat_map(|entry| array(entry, "skills"))
        .find(|skill| skill["name"] == "plugin-creator" && skill["scope"] == "system")
        .and_then(|skill| skill["path"].as_str())
        .map(str::to_owned)
}

fn unique_hooks(value: &Value) -> Vec<&Value> {
    let mut seen = HashSet::new();
    array(value, "data")
        .iter()
        .flat_map(|entry| array(entry, "hooks"))
        .filter(|hook| seen.insert(hook["key"].as_str().unwrap_or("")))
        .collect()
}

fn hook_source(hook: &Value) -> String {
    match hook["source"].as_str().unwrap_or("unknown") {
        "plugin" => format!("plugin:{}", hook["pluginId"].as_str().unwrap_or("")),
        "user" => "user".into(),
        "project" => format!("project:{}", hook["sourcePath"].as_str().unwrap_or("")),
        "sessionFlags" => "sessionFlags".into(),
        "unknown" => "unknown".into(),
        _ => "admin".into(),
    }
}

fn hook_source_group(source: &str) -> &'static str {
    if source.starts_with("plugin:") {
        "From Plugins"
    } else if source.starts_with("project:") {
        "From Projects"
    } else if matches!(source, "user" | "admin") {
        "From Config"
    } else {
        "Other sources"
    }
}

fn hook_source_title(source: &str, state: &State) -> String {
    match source {
        "user" => "User config".into(),
        "admin" => "Admin config".into(),
        "sessionFlags" => "Session flags".into(),
        "unknown" => "Unknown source".into(),
        _ => {
            if let Some(id) = source.strip_prefix("plugin:") {
                if let Load::Ready(value) = &state.plugins
                    && let Some(plugin) = plugins(value).find(|plugin| plugin["id"] == id)
                {
                    return plugin_name(plugin).to_owned();
                }
                return if id.is_empty() {
                    "Unknown plugin".into()
                } else {
                    id.split('@').next().unwrap_or(id).to_owned()
                };
            }
            source.strip_prefix("project:").unwrap_or(source).to_owned()
        }
    }
}

fn hook_event(event: &str) -> (&str, &'static str) {
    match event {
        "preToolUse" => ("PreToolUse", "Before a tool executes"),
        "permissionRequest" => ("PermissionRequest", "When permission is requested"),
        "postToolUse" => ("PostToolUse", "After a tool executes"),
        "preCompact" => ("PreCompact", "Before ChatGPT compacts the conversation"),
        "postCompact" => ("PostCompact", "After ChatGPT compacts the conversation"),
        "sessionStart" => ("SessionStart", "When a new session starts"),
        "sessionEnd" => ("SessionEnd", "When a session ends"),
        "userPromptSubmit" => ("UserPromptSubmit", "When the user submits a prompt"),
        "subagentStart" => ("SubagentStart", "When a subagent starts"),
        "subagentStop" => ("SubagentStop", "When a subagent stops"),
        "stop" => ("Stop", "Right before ChatGPT ends its turn"),
        "interrupt" => ("Interrupt", "When a turn is interrupted"),
        _ => (event, ""),
    }
}

impl Render for Manager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx).clone();
        let t = cx.theme().clone();
        let hooks = self.section == "Hooks";
        if self.marketplace.is_none()
            && self.selected_hooks.is_none()
            && self.marketplace_form.is_none()
            && let Some(focus) = self.return_focus.take()
        {
            focus.focus(window, cx);
        }
        if !hooks && !state.pending {
            self.select_available_tab(&state);
        }
        let heading = if hooks { "Hooks" } else { "Plugins" };
        let subtitle = if hooks {
            div()
                .flex()
                .items_center()
                .child(text(
                    "Manage lifecycle hooks from config and enabled plugins. ",
                    14.,
                    24.,
                    430.,
                    t.text_secondary,
                ))
                .child(
                    div()
                        .id("hooks-learn-more")
                        .role(gpui::Role::Link)
                        .child(text("Learn more", 14., 24., 430., t.text_accent))
                        .on_activate(window, cx, |_, _, cx| {
                            cx.open_url("https://developers.openai.com/codex/hooks")
                        }),
                )
        } else {
            text(
                "Manage plugins, skills, and MCPs",
                14.,
                24.,
                430.,
                t.text_secondary,
            )
        };
        let actions = self.header_actions(hooks, &state, &t, window, cx);
        let mut content = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .pt(px(28.))
            .pb(px(40.))
            .flex()
            .flex_col()
            .gap(px(40.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(text(heading, 28., 33.6, 500., t.text))
                            .child(subtitle),
                    )
                    .child(actions),
            );
        if !hooks {
            content = content.child(self.toolbar(&state, &t, window, cx));
        }
        let body = if hooks {
            self.hooks(&state, &t, window, cx).into_any_element()
        } else {
            self.extension_body(&state, &t, window, cx)
        };
        content = content.child(body);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("settings-integrations-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(div().flex().justify_center().child(content)),
            )
            .children(self.overlay(&state, &t, window, cx))
    }
}

#[cfg(test)]
fn test_panic_hook() {
    #[cfg(target_os = "macos")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                eprintln!("{info}");
                std::process::exit(101);
            }))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_and_changed_hooks_stay_off_until_trusted() {
        test_panic_hook();
        for status in ["untrusted", "modified"] {
            assert!(!hook_enabled(
                &json!({"enabled":true,"trustStatus":status,"isManaged":false})
            ));
        }
        assert!(hook_enabled(
            &json!({"enabled":true,"trustStatus":"trusted","isManaged":false})
        ));
        assert!(hook_enabled(
            &json!({"enabled":false,"trustStatus":"managed","isManaged":true})
        ));
    }

    #[test]
    fn repository_skill_shadows_user_skill_without_hiding_other_names() {
        test_panic_hook();
        let value = json!({"data":[{"skills":[
            {"name":"review","path":"/user/review/SKILL.md","scope":"user","pluginId":null},
            {"name":"review","path":"/repo/review/SKILL.md","scope":"repo","pluginId":null},
            {"name":"compile","path":"/user/compile/SKILL.md","scope":"user","pluginId":null},
            {"name":"plugin-skill","path":"/plugin/SKILL.md","scope":"user","pluginId":"tools@local"}
        ]}]});
        let selected = standalone_skills(&value);
        assert_eq!(
            selected
                .iter()
                .map(|skill| skill["path"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["/user/compile/SKILL.md", "/repo/review/SKILL.md"]
        );
    }

    #[test]
    fn project_server_origin_disables_only_the_matching_server() {
        test_panic_hook();
        let configuration = json!({"origins":{
            "mcp_servers.\"work.search\".command":{"name":{"type":"project"}},
            "mcp_servers.personal.command":{"name":{"type":"user"}}
        }});
        assert!(server_read_only(&configuration, "work.search"));
        assert!(!server_read_only(&configuration, "work"));
        assert!(!server_read_only(&configuration, "personal"));
    }
}
