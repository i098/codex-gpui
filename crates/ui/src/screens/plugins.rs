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
};
use theme::ActiveTheme as _;

mod sidebar;
mod skills;
mod view;
pub use sidebar::Sidebar;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Directory {
    #[default]
    Public,
    Personal,
}

impl Directory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "Public",
            Self::Personal => "Personal",
        }
    }
    pub fn query_value(self) -> &'static str {
        match self {
            Self::Public => "openai",
            Self::Personal => "personal",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkillSection {
    #[default]
    Personal,
    System,
    Recommended,
}

impl SkillSection {
    pub const ALL: [Self; 3] = [Self::Personal, Self::System, Self::Recommended];
    pub fn label(self) -> &'static str {
        match self {
            Self::Personal => "Personal",
            Self::System => "System",
            Self::Recommended => "Recommended",
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
    SelectDirectory(Directory),
    CreatePlugin,
    Read(Arc<Plugin>),
    ReadRecommended,
    InstallRecommended(Value),
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
    pub directory: Directory,
    pub category: Option<String>,
    pub marketplace_path: Option<String>,
    pub add_open: bool,
    pub skill_section: SkillSection,
    pub installed_expanded: bool,
    pub section_expanded: bool,
    pub recommended: Resource<Vec<Value>>,
    pub recommended_error: Option<String>,
    pub share_on_load: bool,
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
    pub marketplace_form: Option<Entity<crate::screens::settings::integrations::MarketplaceForm>>,
    marketplace_added: u64,
    return_focus: Option<FocusHandle>,
    pub generation: u64,
    pub focus: FocusHandle,
    modal_focus: FocusHandle,
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
            Composer::inline_field(
                if tab == Tab::Skills {
                    "Search skills"
                } else {
                    "Search plugins"
                },
                "",
                cx,
            )
        });
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        if !cx.has_global::<crate::screens::settings::integrations::State>() {
            cx.set_global(crate::screens::settings::integrations::State::default());
        }
        cx.observe_global::<crate::screens::settings::integrations::State>(|this, cx| {
            let added = cx
                .global::<crate::screens::settings::integrations::State>()
                .marketplace_added;
            if this.marketplace_form.is_some() && added != this.marketplace_added {
                this.marketplace_form = None;
                this.modal = None;
                cx.emit(Action::Navigate("/plugins?directory=personal".into()));
            }
            this.marketplace_added = added;
            cx.notify();
        })
        .detach();
        Self {
            tab,
            directory: Directory::Public,
            category: None,
            marketplace_path: None,
            add_open: false,
            skill_section: SkillSection::Personal,
            installed_expanded: false,
            section_expanded: false,
            recommended: Resource::Loading,
            recommended_error: None,
            share_on_load: false,
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
            marketplace_form: None,
            marketplace_added: cx
                .global::<crate::screens::settings::integrations::State>()
                .marketplace_added,
            return_focus: None,
            generation: 0,
            focus: cx.focus_handle(),
            modal_focus: cx.focus_handle(),
        }
    }

    pub fn select_skill_section(&mut self, section: SkillSection, cx: &mut Context<Self>) {
        self.skill_section = section;
        self.section_expanded = false;
        if section == SkillSection::Recommended && !self.pending {
            cx.emit(Action::ReadRecommended);
        }
        cx.notify();
    }

    pub fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.pending {
            cx.emit(action);
        }
    }

    pub fn show_modal(&mut self, modal: Modal, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.add_open = false;
        self.return_focus = window.focused(cx);
        self.modal = Some(modal);
        if let Some(Modal::Skill(skill)) = &self.modal {
            cx.emit(Action::ReadSkill(string(skill, "path").to_owned()));
        }
        self.modal_focus.focus(window, cx);
        cx.notify();
    }

    pub fn open_marketplace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = cx.new(crate::screens::settings::integrations::MarketplaceForm::new);
        cx.subscribe_in(
            &form,
            window,
            |this, _, _: &crate::screens::settings::integrations::FormClosed, window, cx| {
                this.close_modal(window, cx);
            },
        )
        .detach();
        self.marketplace_added = cx
            .global::<crate::screens::settings::integrations::State>()
            .marketplace_added;
        self.return_focus = window.focused(cx);
        self.modal = None;
        self.add_open = false;
        self.error = None;
        let focus = form.update(cx, |form, cx| form.controls(window, cx).into_iter().next());
        if let Some(focus) = focus {
            focus.focus(window, cx);
        }
        self.marketplace_form = Some(form);
        cx.notify();
    }

    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending
            || (self.marketplace_form.is_some()
                && cx
                    .global::<crate::screens::settings::integrations::State>()
                    .pending)
        {
            return;
        }
        self.modal = None;
        self.marketplace_form = None;
        self.error = None;
        self.return_focus
            .take()
            .unwrap_or_else(|| self.focus.clone())
            .focus(window, cx);
        cx.notify();
    }
}

pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
pub fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}

pub fn skill_title(skill: &Value) -> String {
    if let Some(title) = skill["interface"]["displayName"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
    {
        return title.trim().to_owned();
    }
    string(skill, "name")
        .split(':')
        .map(|part| {
            part.split(|c: char| c == '_' || c == '-' || c.is_whitespace())
                .filter(|word| !word.is_empty())
                .enumerate()
                .map(|(index, word)| title_word(word, index))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(": ")
}

fn title_word(word: &str, index: usize) -> String {
    const ACRONYMS: &[&str] = &[
        "GH", "IA", "MCP", "API", "CI", "CLI", "LLM", "PDF", "PR", "RCS", "UI", "URL", "SQL", "TW",
        "GPU", "CPU",
    ];
    let upper = word.to_uppercase();
    if ACRONYMS.contains(&upper.as_str()) {
        return upper;
    }
    if upper.ends_with('S') && ACRONYMS.contains(&&upper[..upper.len() - 1]) {
        return format!("{}s", &upper[..upper.len() - 1]);
    }
    let lower = word.to_lowercase();
    match lower.as_str() {
        "openai" => "OpenAI".into(),
        "openaideveloperdocs" => "OpenAI Developer Docs".into(),
        "openapi" => "OpenAPI".into(),
        "github" => "GitHub".into(),
        "imessage" => "iMessage".into(),
        "pagerduty" => "PagerDuty".into(),
        "datadog" => "DataDog".into(),
        "sharepoint" => "SharePoint".into(),
        "sqlite" => "SQLite".into(),
        "fastapi" => "FastAPI".into(),
        "and" | "or" | "to" | "up" | "with" if index > 0 => lower,
        _ => {
            let mut chars = lower.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

pub fn skill_description(skill: &Value) -> &str {
    skill["interface"]["shortDescription"]
        .as_str()
        .filter(|text| !text.is_empty())
        .or_else(|| {
            skill["shortDescription"]
                .as_str()
                .filter(|text| !text.is_empty())
        })
        .unwrap_or_else(|| string(skill, "description"))
}

pub fn standalone_skills(skills: impl IntoIterator<Item = Value>) -> Vec<Value> {
    use std::collections::{HashMap, hash_map::Entry};
    let rank = |skill: &Value| match string(skill, "scope") {
        "repo" => 0,
        "user" => 1,
        "system" => 2,
        "admin" => 3,
        _ => 4,
    };
    let mut by_name: HashMap<String, Value> = HashMap::new();
    for skill in skills {
        if !skill["pluginId"].is_null() || plugin_skill_path(string(&skill, "path")) {
            continue;
        }
        match by_name.entry(string(&skill, "name").to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(skill);
            }
            Entry::Occupied(mut entry) => {
                let old = entry.get();
                if (rank(&skill), string(&skill, "path")) < (rank(old), string(old, "path")) {
                    entry.insert(skill);
                }
            }
        }
    }
    let mut skills: Vec<_> = by_name.into_values().collect();
    skills.sort_by_cached_key(|skill| {
        (
            skill_title(skill).to_lowercase(),
            string(skill, "name").to_owned(),
        )
    });
    skills
}

fn plugin_skill_path(path: &str) -> bool {
    let parts: Vec<_> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.windows(2).any(|pair| {
        (pair[0].eq_ignore_ascii_case(".agents") || pair[0].eq_ignore_ascii_case(".codex"))
            && pair[1].eq_ignore_ascii_case("skills")
    }) {
        return false;
    }
    for (index, part) in parts.iter().enumerate() {
        if !part.eq_ignore_ascii_case("plugins") {
            continue;
        }
        let cached = parts
            .get(index + 1)
            .is_some_and(|part| part.eq_ignore_ascii_case("cache"));
        let plugin = index + if cached { 3 } else { 1 };
        if parts.get(plugin).is_none() {
            continue;
        }
        let root = plugin + usize::from(cached);
        if parts.iter().enumerate().any(|(index, part)| {
            index > root && part.eq_ignore_ascii_case("skills") && parts.get(index + 1).is_some()
        }) {
            return true;
        }
        if parts.len() == root + 2 && parts[root + 1].eq_ignore_ascii_case("skill.md") {
            return true;
        }
    }
    false
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

    #[test]
    fn standalone_skill_projection_keeps_the_nearest_scope_and_hides_plugin_skills() {
        let skill =
            |name: &str, scope: &str, path: &str| json!({"name":name,"scope":scope,"path":path});
        let selected = standalone_skills([
            skill("same", "user", "/home/user/.agents/skills/same/SKILL.md"),
            skill("same", "repo", "/work/.agents/skills/same/SKILL.md"),
            skill(
                "same",
                "system",
                "/home/user/.codex/skills/.system/same/SKILL.md",
            ),
            skill("first", "user", "/home/user/.agents/skills/first/SKILL.md"),
            skill(
                "hidden",
                "user",
                "/home/user/.codex/plugins/cache/vendor/plugin/1/skills/hidden/SKILL.md",
            ),
            json!({"name":"explicit","path":"/work/skill.md","pluginId":"plugin"}),
        ]);
        assert_eq!(
            selected
                .iter()
                .map(|skill| string(skill, "path"))
                .collect::<Vec<_>>(),
            [
                "/home/user/.agents/skills/first/SKILL.md",
                "/work/.agents/skills/same/SKILL.md",
            ]
        );
    }
}
