use crate::{composer::Composer, icon::icon, text};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use theme::ActiveTheme as _;

mod nav;
mod view;

pub use nav::Nav;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Plugins,
    Skills,
}

impl Tab {
    pub const ALL: [Self; 2] = [Self::Plugins, Self::Skills];

    pub fn label(self) -> &'static str {
        match self {
            Self::Plugins => "Plugins",
            Self::Skills => "Skills",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub summary: Value,
    pub marketplace_name: String,
    pub marketplace_path: Option<String>,
    pub remote_marketplace_name: Option<String>,
}

impl Plugin {
    pub fn id(&self) -> &str {
        string(&self.summary, "id")
    }
    pub fn name(&self) -> &str {
        string(&self.summary, "name")
    }
    pub fn title(&self) -> &str {
        self.summary["interface"]["displayName"]
            .as_str()
            .unwrap_or_else(|| self.name())
    }
    pub fn description(&self) -> &str {
        self.summary["interface"]["shortDescription"]
            .as_str()
            .unwrap_or("")
    }
    pub fn installed(&self) -> bool {
        self.summary["installed"] == true
    }
    pub fn visible(&self) -> bool {
        !matches!(
            self.id(),
            "codex-app-tools@openai-bundled" | "unified-computer-use@openai-bundled"
        )
    }
    pub fn enabled(&self) -> bool {
        self.summary["enabled"] == true
    }
    pub fn disabled_reason(&self) -> Option<&'static str> {
        match self.summary["disabledReason"].as_str() {
            Some("plan_not_eligible") => Some("This plugin is not available on your plan"),
            Some("required_app_unavailable") => Some("A required app is unavailable"),
            _ if self.summary["availability"] == "DISABLED_BY_ADMIN"
                || self.summary["installPolicy"] == "NOT_AVAILABLE"
                || self.summary["disabledReason"] == "disabled_by_admin" =>
            {
                Some("Disabled by your workspace admin")
            }
            _ => None,
        }
    }
    pub fn can_uninstall(&self) -> bool {
        self.installed()
            && self.id() != "code-review@openai-bundled"
            && !(self.summary["source"]["type"] == "remote"
                && self.summary["installPolicy"] == "INSTALLED_BY_DEFAULT")
    }
    pub fn local_path(&self) -> Option<&str> {
        (self.summary["source"]["type"] == "local")
            .then(|| self.summary["source"]["path"].as_str())
            .flatten()
    }
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || [self.name(), self.title(), self.description()]
                .iter()
                .any(|value| value.to_lowercase().contains(query))
    }
}

#[derive(Clone, Debug)]
pub struct Marketplace {
    pub name: String,
    pub path: Option<String>,
    pub title: String,
    pub plugins: Vec<Arc<Plugin>>,
}

pub fn marketplaces(value: &Value) -> Vec<Marketplace> {
    array(value, "marketplaces")
        .iter()
        .map(|marketplace| {
            let name = string(marketplace, "name").to_owned();
            let path = marketplace["path"].as_str().map(str::to_owned);
            let remote = marketplace["remoteMarketplaceName"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| path.is_none().then(|| name.clone()));
            let plugins = array(marketplace, "plugins")
                .iter()
                .map(|summary| {
                    Arc::new(Plugin {
                        summary: summary.clone(),
                        marketplace_name: name.clone(),
                        marketplace_path: path.clone(),
                        remote_marketplace_name: remote.clone(),
                    })
                })
                .collect();
            Marketplace {
                title: marketplace["interface"]["displayName"]
                    .as_str()
                    .unwrap_or(&name)
                    .to_owned(),
                name,
                path,
                plugins,
            }
        })
        .collect()
}

/// The original `use-search-plugins` debounce and page size.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);
pub const SEARCH_LIMIT: u32 = 50;

/// The Plugins tab's server `plugin/search` results: the pages fetched so far and the cursor
/// for the next one.
#[derive(Debug)]
pub struct Search {
    pub results: Resource<Vec<Arc<Plugin>>>,
    pub next: Option<String>,
    pub loading_more: bool,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            results: Resource::Loading,
            next: None,
            loading_more: false,
        }
    }
}

impl Search {
    /// Appends one page; the first page replaces `Loading`, and `next` closes the paging.
    pub fn push(&mut self, plugins: Vec<Arc<Plugin>>, next: Option<String>) {
        match &mut self.results {
            Resource::Ready(list) => list.extend(plugins),
            _ => self.results = Resource::Ready(plugins),
        }
        self.next = next;
        self.loading_more = false;
    }
}

#[derive(Debug, Default)]
pub enum Resource<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Resource<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Data {
    pub plugins: Resource<Vec<Marketplace>>,
    pub skills: Resource<Vec<Value>>,
    pub apps: Resource<Vec<Value>>,
    pub personal_marketplace: Option<PathBuf>,
}

impl Data {
    pub fn can_use(&self, plugin: &Plugin, detail: Option<&Value>) -> bool {
        if !plugin.installed() || !plugin.enabled() || plugin.disabled_reason().is_some() {
            return false;
        }
        if plugin.summary["authPolicy"] != "ON_INSTALL" {
            return true;
        }
        let (Some(apps), Some(detail)) = (self.apps.value(), detail) else {
            return false;
        };
        array(detail, "apps").iter().all(|required| {
            apps.iter()
                .any(|app| app["id"] == required["id"] && app["isAccessible"] == true)
        })
    }

    pub fn personal_path<'a>(&self, plugin: &'a Plugin) -> Option<&'a str> {
        let expected = self.personal_marketplace.as_deref()?;
        let actual = Path::new(plugin.marketplace_path.as_deref()?);
        (actual == expected).then(|| plugin.local_path()).flatten()
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Manage,
    Navigate(String),
    /// Searches the server catalog for this term.
    Search(String),
    /// Loads the next `plugin/search` page for this term and cursor.
    SearchMore(String, String),
    Read(Arc<Plugin>),
    Install(Arc<Plugin>),
    EnablePlugin(Arc<Plugin>, bool),
    Uninstall(Arc<Plugin>),
    EnableSkill {
        name: String,
        path: String,
        enabled: bool,
    },
    EnableApp {
        id: String,
        enabled: bool,
    },
    Share(Arc<Plugin>),
    CreateShare(Arc<Plugin>),
    UpdateSharedVersion(Arc<Plugin>),
    TryPlugin(Arc<Plugin>),
    EditPlugin(Arc<Plugin>),
    NewSkill,
    ReadSkill(String),
    OpenSkill(String),
    ConnectApp(Value),
    UpdateShareTargets(Arc<Plugin>),
    DeleteShare(Arc<Plugin>),
    DisconnectApp(String),
}

#[derive(Clone, Debug)]
pub enum Modal {
    Install(Arc<Plugin>),
    Connections {
        plugin: Arc<Plugin>,
        apps: Vec<Value>,
    },
    Uninstall(Arc<Plugin>),
    Share(Arc<Plugin>),
    Skill(Value),
    Connect(Value),
    Connecting(Value),
    Disconnect(String),
}

pub struct Page {
    pub tab: Tab,
    pub data: Data,
    pub available: bool,
    pub query: Entity<Composer>,
    pub detail: Option<Arc<Plugin>>,
    pub detail_data: Resource<Value>,
    pub pending: bool,
    pub error: Option<String>,
    pub modal: Option<Modal>,
    pub share_url: Option<String>,
    pub skill_file: Option<Entity<crate::screens::diff::panel::FileView>>,
    pub share_targets: Vec<Value>,
    pub share_discoverability: String,
    /// The selected pill under the heading (`Public`, `Personal`, `System`, ...).
    pub section: &'static str,
    /// Lists expanded past their first rows by `See ...`.
    pub expanded: std::collections::HashSet<&'static str>,
    /// Cards shown per plugin marketplace section, raised by its `See more`.
    pub plugin_rows: std::collections::HashMap<String, usize>,
    /// The server search behind a non-empty query; `None` filters the loaded catalog locally.
    pub search: Option<Search>,
    /// Bumped on every query and section change, so a late page cannot land on a newer search.
    pub search_gen: usize,
    pub generation: u64,
    pub focus: FocusHandle,
    modal_focus: FocusHandle,
    modal_return_focus: Option<FocusHandle>,
}

impl EventEmitter<Action> for Page {}
impl Focusable for Page {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Page {
    pub fn new(route: &str, available: bool, cx: &mut Context<Self>) -> Self {
        let tab = if route.starts_with("/skills") {
            Tab::Skills
        } else {
            Tab::Plugins
        };
        let query = cx.new(|cx| {
            Composer::embedded_field(
                if tab == Tab::Skills {
                    "Search skills"
                } else {
                    "Search plugins"
                },
                "",
                px(13.),
                px(18.),
                cx,
            )
        });
        cx.observe(&query, |page, _, cx| page.query_changed(cx))
            .detach();
        Self {
            tab,
            data: Data::default(),
            available,
            query,
            detail: None,
            detail_data: Resource::Loading,
            pending: false,
            error: None,
            modal: None,
            share_url: None,
            skill_file: None,
            share_targets: Vec::new(),
            share_discoverability: "PRIVATE".into(),
            section: if tab == Tab::Skills {
                "Personal"
            } else {
                "Public"
            },
            expanded: Default::default(),
            plugin_rows: Default::default(),
            search: None,
            search_gen: 0,
            generation: 0,
            focus: cx.focus_handle(),
            modal_focus: cx.focus_handle(),
            modal_return_focus: None,
        }
    }

    pub fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.pending {
            cx.emit(action);
        }
    }

    /// The query or section changed: drop the old results and search the server after the
    /// original's debounce. An empty query filters the loaded catalog locally instead.
    pub fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.search_gen += 1;
        self.search = None;
        let term = self.query.read(cx).field_text().trim().to_owned();
        if term.is_empty() || self.tab != Tab::Plugins || !self.available {
            cx.notify();
            return;
        }
        let generation = self.search_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if this.search_gen != generation {
                    return;
                }
                this.search = Some(Search::default());
                cx.emit(Action::Search(term.clone()));
                cx.notify();
            });
        })
        .detach();
    }

    pub fn show_modal(&mut self, modal: Modal, window: &mut Window, cx: &mut Context<Self>) {
        self.modal_return_focus = window.focused(cx);
        self.error = None;
        self.modal = Some(modal);
        if let Some(Modal::Skill(skill)) = &self.modal {
            cx.emit(Action::ReadSkill(string(skill, "path").to_owned()));
        }
        self.modal_focus.focus(window, cx);
        cx.notify();
    }

    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.modal = None;
        self.error = None;
        if let Some(focus) = self.modal_return_focus.take() {
            focus.focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
}

pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
pub fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_page(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn policy_blocks_take_precedence_over_installed_state() {
        let mut plugin = Plugin {
            summary: json!({"id":"example@remote","name":"example","installed":true,
                "enabled":true,"source":{"type":"remote"},"installPolicy":"INSTALLED_BY_DEFAULT",
                "availability":"DISABLED_BY_ADMIN"}),
            marketplace_name: "remote".into(),
            marketplace_path: None,
            remote_marketplace_name: Some("remote".into()),
        };
        assert!(plugin.disabled_reason().is_some());
        assert!(!plugin.can_uninstall());
        plugin.summary["availability"] = json!("AVAILABLE");
        plugin.summary["installPolicy"] = json!("AVAILABLE");
        assert!(plugin.disabled_reason().is_none());
        assert!(plugin.can_uninstall());
        plugin.summary["id"] = json!("code-review@openai-bundled");
        assert!(!plugin.can_uninstall());
    }

    #[test]
    fn install_time_auth_requires_confirmed_app_connections() {
        let mut plugin = Plugin {
            summary: json!({"id":"example","name":"example","installed":true,
                "enabled":true,"authPolicy":"ON_INSTALL"}),
            marketplace_name: String::new(),
            marketplace_path: None,
            remote_marketplace_name: None,
        };
        let detail = json!({"apps":[{"id":"required-app"}]});
        let mut data = Data::default();
        assert!(!data.can_use(&plugin, Some(&detail)));
        data.apps = Resource::Ready(vec![json!({"id":"required-app","isAccessible":false})]);
        assert!(!data.can_use(&plugin, Some(&detail)));
        data.apps = Resource::Ready(vec![json!({"id":"required-app","isAccessible":true})]);
        assert!(data.can_use(&plugin, Some(&detail)));
        plugin.summary["authPolicy"] = json!("ON_USE");
        assert!(Data::default().can_use(&plugin, None));
    }

    #[test]
    fn search_pages_append_and_drop_the_finished_cursor() {
        let plugin = |id: &str| {
            Arc::new(Plugin {
                summary: json!({"id":id,"name":id}),
                marketplace_name: "market".into(),
                marketplace_path: None,
                remote_marketplace_name: Some("market".into()),
            })
        };
        let mut search = Search::default();
        assert!(matches!(search.results, Resource::Loading));
        search.push(vec![plugin("a")], Some("page-2".into()));
        search.loading_more = true;
        search.push(vec![plugin("b")], None);
        let Resource::Ready(list) = &search.results else {
            panic!("search results are not ready");
        };
        let ids: Vec<_> = list.iter().map(|plugin| plugin.id()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(search.next, None);
        assert!(!search.loading_more);
        let mut failed = Search {
            results: Resource::Failed("gone".into()),
            ..Search::default()
        };
        failed.push(vec![plugin("c")], None);
        assert!(matches!(failed.results, Resource::Ready(_)));
    }

    #[test]
    fn sharing_requires_the_personal_marketplace_not_a_matching_filename() {
        let mut plugin = Plugin {
            summary: json!({"id":"example","name":"example","source":{"type":"local","path":"/home/user/plugin"}}),
            marketplace_name: "personal".into(),
            marketplace_path: Some("/home/user/.agents/plugins/marketplace.json".into()),
            remote_marketplace_name: None,
        };
        let data = Data {
            personal_marketplace: Some("/home/user/.agents/plugins/marketplace.json".into()),
            ..Data::default()
        };
        assert_eq!(data.personal_path(&plugin), Some("/home/user/plugin"));
        plugin.marketplace_path = Some("/tmp/imported/.agents/plugins/marketplace.json".into());
        assert_eq!(data.personal_path(&plugin), None);
    }
}
