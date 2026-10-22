mod current;
pub mod model;

use crate::{
    composer::{Composer, popover},
    focus, text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use model::{Draft, Environment, RUNTIMES, Secret, UNIVERSAL};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Copy, Default)]
pub struct Access {
    pub cloud: bool,
    pub legacy: bool,
    pub workspace: bool,
    pub manage_workspace: bool,
    pub domain_secrets: bool,
    pub network_allowed: bool,
}

#[derive(Clone)]
pub enum Operation {
    Access,
    List {
        query: String,
        cursor: Option<String>,
    },
    Detail(String),
    Machines,
    Repositories(String),
    Save {
        original: Option<String>,
        body: Value,
    },
    Delete(String),
    Reset(String),
    Configs,
    Vault,
    Config(String),
    ConfigSave {
        id: String,
        config: Value,
        changes: Value,
        metadata: Value,
    },
    ConfigDelete(String),
    ConfigCreate(Value),
}

#[derive(Clone)]
pub struct Request {
    pub serial: u64,
    pub operation: Operation,
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    List,
    Detail,
    Editor,
}
#[derive(Clone, PartialEq)]
enum Menu {
    Machine,
    Sharing,
    Network,
    Runtime(usize),
}

struct Editor {
    draft: Draft,
    original: Option<Environment>,
    fields: BTreeMap<String, Entity<Composer>>,
    initial: Draft,
}

pub struct CloudPage {
    access: Access,
    pub access_loaded: bool,
    view: View,
    serial: u64,
    pub pending: bool,
    error: Option<String>,
    retry: Option<Operation>,
    environments: Vec<Environment>,
    cursor: Option<String>,
    selected: Option<Environment>,
    machines: Vec<Value>,
    machine_error: bool,
    repositories: Vec<Value>,
    query: Entity<Composer>,
    editor: Option<Editor>,
    menu: Option<Menu>,
    packages: bool,
    repository_picker: bool,
    confirm: Option<&'static str>,
    expanded_workspace: bool,
    discard_to_list: bool,
    button_focus: RefCell<BTreeMap<SharedString, FocusHandle>>,
    current: current::Current,
    current_mode: Option<bool>,
    route_pending: Option<String>,
    edit_requested: bool,
    unavailable: bool,
}

impl EventEmitter<Request> for CloudPage {}

impl CloudPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| Composer::settings_search("Search environments", "", cx));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        Self {
            access: Access::default(),
            access_loaded: false,
            view: View::List,
            serial: 0,
            pending: false,
            error: None,
            retry: None,
            environments: Vec::new(),
            cursor: None,
            selected: None,
            machines: Vec::new(),
            machine_error: false,
            repositories: Vec::new(),
            query,
            editor: None,
            menu: None,
            packages: false,
            repository_picker: false,
            confirm: None,
            expanded_workspace: false,
            discard_to_list: false,
            button_focus: RefCell::new(BTreeMap::new()),
            current: current::Current::default(),
            current_mode: None,
            route_pending: None,
            edit_requested: false,
            unavailable: false,
        }
    }

    pub fn access(&self) -> Access {
        self.access
    }

    pub fn load(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if matches!(operation, Operation::Machines) {
            self.machine_error = false;
        }
        self.serial += 1;
        self.pending = true;
        self.error = None;
        self.retry = matches!(
            operation,
            Operation::Access
                | Operation::List { .. }
                | Operation::Detail(_)
                | Operation::Machines
                | Operation::Repositories(_)
                | Operation::Configs
                | Operation::Vault
                | Operation::Config(_)
        )
        .then(|| operation.clone());
        cx.emit(Request {
            serial: self.serial,
            operation,
        });
        cx.notify();
    }

    pub fn set_access(&mut self, access: Result<Access, String>, cx: &mut Context<Self>) {
        self.access_loaded = true;
        self.pending = false;
        match access {
            Ok(access) => self.access = access,
            Err(error) => {
                self.access = Access::default();
                self.error = Some(error);
            }
        }
        if let Some(path) = self.route_pending.take() {
            self.open_path(&path, cx);
        } else if self.current_mode.is_some() {
            self.refresh(cx);
        }
        cx.notify();
    }

    pub fn apply(
        &mut self,
        request: &Request,
        result: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        if request.serial != self.serial {
            return;
        }
        self.pending = false;
        match result.and_then(|body| self.receive(&request.operation, body, cx)) {
            Ok(()) => {}
            Err(_) if matches!(request.operation, Operation::Machines) => self.machine_error = true,
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn receive(
        &mut self,
        operation: &Operation,
        body: Value,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match operation {
            Operation::Configs
            | Operation::Vault
            | Operation::Config(_)
            | Operation::ConfigSave { .. }
            | Operation::ConfigDelete(_)
            | Operation::ConfigCreate(_) => return self.receive_current(operation, body, cx),
            Operation::List { cursor, .. } => {
                let items: Vec<Environment> = serde_json::from_value(body["items"].clone())
                    .map_err(|_| "Unable to load cloud environments")?;
                self.cursor = body["cursor"].as_str().map(str::to_owned);
                if cursor.is_none() {
                    self.environments.clear();
                }
                for item in items {
                    if let Some(old) = self.environments.iter_mut().find(|old| old.id == item.id) {
                        *old = item;
                    } else {
                        self.environments.push(item);
                    }
                }
                self.environments.sort_by(|a, b| {
                    b.task_count
                        .unwrap_or(0)
                        .cmp(&a.task_count.unwrap_or(0))
                        .then_with(|| a.label.cmp(&b.label))
                });
            }
            Operation::Detail(_) => {
                self.selected = Some(
                    serde_json::from_value(body)
                        .map_err(|_| "Unable to load this cloud environment")?,
                );
                self.view = View::Detail;
                if std::mem::take(&mut self.edit_requested) {
                    self.edit(self.selected.clone(), cx);
                }
            }
            Operation::Machines => {
                self.machines = body
                    .as_array()
                    .ok_or("Unable to load container images")?
                    .clone();
                self.machines.retain(|m| m["id"] != UNIVERSAL);
                self.machines
                    .push(json!({"id":UNIVERSAL,"label":"Universal"}));
            }
            Operation::Repositories(_) => {
                self.repositories = body["repositories"]
                    .as_array()
                    .or_else(|| body["items"].as_array())
                    .ok_or("Unable to load repositories")?
                    .clone()
            }
            Operation::Save { original, .. } => {
                let id = original
                    .as_deref()
                    .or_else(|| body["id"].as_str())
                    .ok_or("Unable to save cloud environment")?
                    .to_owned();
                self.editor = None;
                self.load(Operation::Detail(id), cx);
            }
            Operation::Delete(_) => {
                self.selected = None;
                self.view = View::List;
                self.confirm = None;
                self.refresh(cx);
            }
            Operation::Reset(id) => {
                self.confirm = None;
                self.load(Operation::Detail(id.clone()), cx);
            }
            Operation::Access => {}
        }
        Ok(())
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.current_mode == Some(true) && self.access.cloud {
            self.load(Operation::Configs, cx);
        } else if self.current_mode == Some(false) && self.access.legacy {
            self.load(
                Operation::List {
                    query: self.query.read(cx).field_text().trim().into(),
                    cursor: None,
                },
                cx,
            );
        }
    }

    fn edit(&mut self, original: Option<Environment>, cx: &mut Context<Self>) {
        let mut draft = original.as_ref().map(Draft::from).unwrap_or_default();
        if original.is_none() && self.access.workspace {
            draft.sharing = "workspace".into();
        }
        let fields = [
            ("name", "Environment name", draft.label.clone(), false),
            (
                "description",
                "1-2 sentence description",
                draft.description.clone(),
                true,
            ),
            (
                "workspace",
                "Workspace directory",
                draft.workspace.clone(),
                false,
            ),
            ("setup", "Setup script", draft.setup.clone(), true),
            (
                "maintenance",
                "Maintenance script",
                draft.maintenance.clone(),
                true,
            ),
            (
                "domains",
                "Allowed domains",
                draft.network["allowlist_domains"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                true,
            ),
            (
                "editors",
                "Additional editors",
                draft.editors.join("\n"),
                true,
            ),
        ]
        .into_iter()
        .map(|(key, label, value, multiline)| {
            (key.into(), self.field(label, &value, multiline, false, cx))
        })
        .collect();
        self.editor = Some(Editor {
            initial: draft.clone(),
            draft,
            original,
            fields,
        });
        self.view = View::Editor;
        self.expanded_workspace = false;
        self.menu = None;
        self.error = None;
        self.load(Operation::Machines, cx);
    }

    fn field(
        &self,
        label: &'static str,
        value: &str,
        multiline: bool,
        secret: bool,
        cx: &mut Context<Self>,
    ) -> Entity<Composer> {
        let field = cx.new(|cx| {
            if secret {
                Composer::settings_secret(label, value, cx)
            } else {
                Composer::settings_field(label, value, multiline, cx)
            }
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field
    }

    fn draft(&self, cx: &App) -> Option<Draft> {
        let editor = self.editor.as_ref()?;
        let mut draft = editor.draft.clone();
        let get = |key: &str| {
            editor
                .fields
                .get(key)
                .map(|f| f.read(cx).field_text().to_owned())
                .unwrap_or_default()
        };
        draft.label = get("name");
        draft.description = get("description");
        draft.workspace = get("workspace");
        draft.setup = get("setup");
        draft.maintenance = get("maintenance");
        draft.editors = get("editors").lines().map(str::to_owned).collect();
        if draft.network["mode"] != "off" {
            draft.network["allowlist_domains"] = get("domains").into();
        }
        for (index, (key, value)) in draft.variables.iter_mut().enumerate() {
            if let Some(field) = editor.fields.get(&format!("variable-name-{index}")) {
                *key = field.read(cx).field_text().into();
            }
            if let Some(field) = editor.fields.get(&format!("variable-value-{index}")) {
                *value = field.read(cx).field_text().into();
            }
        }
        for (index, secret) in draft.secrets.iter_mut().enumerate() {
            if let Some(field) = editor.fields.get(&format!("secret-name-{index}")) {
                secret.name = field.read(cx).field_text().into();
            }
            if let Some(field) = editor.fields.get(&format!("secret-value-{index}")) {
                secret.value = field.read(cx).field_text().into();
            }
            if let Some(field) = editor.fields.get(&format!("secret-domain-{index}")) {
                secret.domain = Some(field.read(cx).field_text().into());
            }
        }
        Some(draft)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(draft) = self.draft(cx) else {
            return;
        };
        let original = self.editor.as_ref().and_then(|e| e.original.as_ref());
        if let Some(error) = draft
            .errors(&self.environments, original.map(|e| e.id.as_str()))
            .values()
            .next()
        {
            self.error = Some((*error).into());
            cx.notify();
            return;
        }
        if !self.machines.iter().any(|m| m["id"] == draft.machine)
            && original.is_none_or(|e| e.machine_id != draft.machine)
        {
            self.error = Some("Unable to load container images".into());
            cx.notify();
            return;
        }
        self.load(
            Operation::Save {
                original: original.map(|e| e.id.clone()),
                body: draft.payload(original),
            },
            cx,
        );
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        if self
            .draft(cx)
            .as_ref()
            .zip(self.editor.as_ref())
            .is_some_and(|(draft, e)| *draft != e.initial)
        {
            self.confirm = Some("discard");
        } else {
            self.leave_editor();
        }
        cx.notify();
    }

    fn leave_editor(&mut self) {
        if std::mem::take(&mut self.discard_to_list) {
            self.selected = None;
        }
        self.editor = None;
        self.packages = false;
        self.repository_picker = false;
        self.confirm = None;
        self.view = if self.selected.is_some() {
            View::Detail
        } else {
            View::List
        };
    }

    fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        primary: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id = id.into();
        let label = label.into();
        let fill = if primary {
            t.text
        } else {
            t.primary_soft_alpha
        };
        let color = if primary { t.background_panel } else { t.text };
        let content = div()
            .h(px(32.))
            .px(px(12.))
            .rounded(px(12.))
            .bg(fill)
            .flex()
            .items_center()
            .gap(px(6.))
            .child(text(label.clone(), 13., 18., 500., color));
        self.wire_button(id, label, content, cx, action)
    }

    fn wire_button(
        &self,
        id: SharedString,
        label: SharedString,
        content: Div,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let action = std::rc::Rc::new(action);
        let click = action.clone();
        let disabled = self.pending;
        let handle = self
            .button_focus
            .borrow_mut()
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        content
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .when(!disabled, |d| d.track_focus(&handle))
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                if !disabled {
                    handle.focus(window, cx);
                }
            })
            .when(disabled, |d| d.opacity(0.5))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    click(this, window, cx);
                }
            }))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        action(this, window, cx);
                        cx.stop_propagation();
                    } else {
                        focus::tab(event, window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    fn legacy_header(&self, title: &str, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        div()
            .h(px(46.))
            .flex_none()
            .px(px(20.))
            .flex()
            .items_center()
            .when(self.view != View::List, |d| {
                let parent = div()
                    .h(px(28.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .child(text("Legacy Codex Cloud", 13., 18., 430., t.text_tertiary));
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(self.wire_button(
                            "cloud-breadcrumb".into(),
                            "Legacy Codex Cloud".into(),
                            parent,
                            cx,
                            |this, _, cx| {
                                if this.editor.is_some() {
                                    this.discard_to_list = true;
                                    this.cancel_edit(cx);
                                    if this.editor.is_none() {
                                        this.refresh(cx);
                                    }
                                } else {
                                    this.selected = None;
                                    this.view = View::List;
                                    this.refresh(cx);
                                }
                            },
                        ))
                        .child(crate::icon::icon("cloud-breadcrumb", 16., t.text_tertiary))
                        .child(
                            div()
                                .h(px(28.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .child(text(title.to_owned(), 14., 21., 430., t.text)),
                        ),
                )
            })
            .into_any_element()
    }

    fn group(title: &'static str, rows: Vec<AnyElement>, t: &Theme) -> AnyElement {
        let count = rows.len();
        div()
            .flex()
            .flex_col()
            .when(!title.is_empty(), |d| {
                d.child(
                    div()
                        .h(px(46.))
                        .pb(px(6.))
                        .flex()
                        .items_center()
                        .child(text(title, 14., 21., 500., t.text)),
                )
            })
            .child(
                div()
                    .relative()
                    .p(px(1.))
                    .child(super::settings::on_card(
                        20.,
                        shapes::Corner::Squircle,
                        t.background_panel,
                        Some(t.border),
                    ))
                    .child(div().overflow_hidden().flex().flex_col().children(
                        rows.into_iter().enumerate().map(|(index, row)| {
                            div().relative().child(row).when(index + 1 < count, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .bottom_0()
                                        .left(px(16.))
                                        .right(px(16.))
                                        .h(px(1.))
                                        .bg(t.border),
                                )
                            })
                        }),
                    )),
            )
            .into_any_element()
    }

    fn row(
        label: &'static str,
        detail: Option<&'static str>,
        control: AnyElement,
        t: &Theme,
    ) -> AnyElement {
        super::settings::settings_row(label, detail, control, t).into_any_element()
    }

    fn select(&self, target: Menu, value: String, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.menu.as_ref() == Some(&target);
        self.button(
            format!("cloud-select-{value}"),
            format!("{value}  ⌄"),
            false,
            cx,
            move |this, _, cx| {
                this.menu = (!shown).then(|| target.clone());
                cx.notify();
            },
        )
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().child(self.query.clone()))
            .child(
                self.button("cloud-search", "Search", false, cx, |this, _, cx| {
                    this.refresh(cx)
                }),
            )
            .child(self.button(
                "cloud-create",
                "Create environment",
                true,
                cx,
                |this, _, cx| {
                    this.selected = None;
                    this.edit(None, cx);
                },
            ));
        let rows = self
            .environments
            .iter()
            .map(|environment| {
                let id = environment.id.clone();
                self.button(
                    format!("environment-{id}"),
                    environment.label.clone(),
                    false,
                    cx,
                    move |this, _, cx| this.load(Operation::Detail(id.clone()), cx),
                )
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(toolbar)
            .when(
                self.environments.is_empty() && !self.pending && self.error.is_none(),
                |d| {
                    d.child(text(
                        if self.query.read(cx).field_text().is_empty() {
                            "No cloud environments"
                        } else {
                            "No cloud environments found"
                        },
                        14.,
                        21.,
                        430.,
                        t.text_secondary,
                    ))
                },
            )
            .children(rows)
            .children(self.cursor.as_ref().map(|cursor| {
                let cursor = cursor.clone();
                self.button("cloud-more", "Load more", false, cx, move |this, _, cx| {
                    this.load(
                        Operation::List {
                            query: this.query.read(cx).field_text().trim().into(),
                            cursor: Some(cursor.clone()),
                        },
                        cx,
                    )
                })
            }))
            .into_any_element()
    }

    fn detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let Some(environment) = &self.selected else {
            return div().into_any_element();
        };
        let rows = vec![
            Self::row(
                "Repository",
                None,
                text(environment.repository_name(), 13., 18., 430., t.text).into_any_element(),
                &t,
            ),
            Self::row(
                "Machine",
                None,
                text(
                    environment.machine["label"]
                        .as_str()
                        .unwrap_or(&environment.machine_id)
                        .to_owned(),
                    13.,
                    18.,
                    430.,
                    t.text,
                )
                .into_any_element(),
                &t,
            ),
            Self::row(
                "Created by",
                None,
                text(
                    environment.creator["name"]
                        .as_str()
                        .or_else(|| environment.creator["email"].as_str())
                        .unwrap_or("")
                        .to_owned(),
                    13.,
                    18.,
                    430.,
                    t.text,
                )
                .into_any_element(),
                &t,
            ),
            Self::row(
                "Network access",
                None,
                text(
                    environment.agent_network_access["mode"]
                        .as_str()
                        .unwrap_or("off")
                        .to_owned(),
                    13.,
                    18.,
                    430.,
                    t.text,
                )
                .into_any_element(),
                &t,
            ),
            Self::row(
                "Sharing",
                None,
                text(environment.share_settings.clone(), 13., 18., 430., t.text).into_any_element(),
                &t,
            ),
        ];
        let edit = environment.permissions["can_write"] == true;
        let delete = environment.permissions["can_delete"] == true;
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text(
                environment.description.clone(),
                14.,
                21.,
                430.,
                t.text_secondary,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(self.button("cloud-back", "Back", false, cx, |this, _, cx| {
                        this.view = View::List;
                        this.selected = None;
                        this.refresh(cx);
                    }))
                    .when(edit, |d| {
                        d.child(self.button("cloud-edit", "Edit", false, cx, |this, _, cx| {
                            this.edit(this.selected.clone(), cx)
                        }))
                    })
                    .when(
                        edit && environment.cache_settings["post_setup_cache_enabled"] != false,
                        |d| {
                            d.child(self.button(
                                "cloud-reset",
                                "Reset cache",
                                false,
                                cx,
                                |this, _, cx| {
                                    this.confirm = Some("reset");
                                    cx.notify();
                                },
                            ))
                        },
                    )
                    .when(delete, |d| {
                        d.child(self.button(
                            "cloud-delete",
                            "Delete environment",
                            false,
                            cx,
                            |this, _, cx| {
                                this.confirm = Some("delete");
                                cx.notify();
                            },
                        ))
                    }),
            )
            .child(Self::group("", rows, &t))
            .into_any_element()
    }

    fn input_row(
        &self,
        key: &str,
        label: &'static str,
        detail: Option<&'static str>,
        t: &Theme,
    ) -> AnyElement {
        let field = self
            .editor
            .as_ref()
            .and_then(|e| e.fields.get(key))
            .cloned();
        Self::row(
            label,
            detail,
            div().w(px(288.)).children(field).into_any_element(),
            t,
        )
    }

    fn editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        let machine = self
            .machines
            .iter()
            .find(|m| m["id"] == editor.draft.machine)
            .and_then(|m| m["label"].as_str())
            .unwrap_or(&editor.draft.machine)
            .to_owned();
        let repository = if editor.original.is_some() {
            text(editor.draft.repository_name.clone(), 13., 18., 430., t.text).into_any_element()
        } else {
            self.button(
                "cloud-repository",
                if editor.draft.repository_name.is_empty() {
                    "Select a repository".into()
                } else {
                    editor.draft.repository_name.clone()
                },
                false,
                cx,
                |this, _, cx| {
                    this.repository_picker = true;
                    this.load(Operation::Repositories(String::new()), cx);
                },
            )
        };
        let mut identity = vec![
            Self::row("Repository", None, repository, &t),
            self.input_row("name", "Name", None, &t),
            self.input_row(
                "description",
                "Description",
                Some("Explain to members of your organization what this environment is for"),
                &t,
            ),
            Self::row(
                "Container image",
                self.machine_error
                    .then_some("Unable to load container images"),
                if self.machine_error {
                    div()
                        .flex()
                        .gap(px(8.))
                        .children(editor.original.as_ref().map(|environment| {
                            text(
                                environment.machine["label"]
                                    .as_str()
                                    .unwrap_or(&environment.machine_id)
                                    .to_owned(),
                                13.,
                                18.,
                                430.,
                                t.text_secondary,
                            )
                        }))
                        .child(self.button(
                            "cloud-machine-retry",
                            "Retry",
                            false,
                            cx,
                            |this, _, cx| this.load(Operation::Machines, cx),
                        ))
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(self.select(Menu::Machine, machine, cx))
                        .when(editor.draft.machine == UNIVERSAL, |d| {
                            d.child(self.button(
                                "cloud-packages",
                                "Preinstalled packages",
                                false,
                                cx,
                                |this, _, cx| {
                                    this.packages = true;
                                    cx.notify();
                                },
                            ))
                        })
                        .into_any_element()
                },
                &t,
            ),
            Self::row(
                "",
                None,
                self.button(
                    "cloud-workspace",
                    "Edit workspace directory",
                    false,
                    cx,
                    |this, _, cx| {
                        this.expanded_workspace = !this.expanded_workspace;
                        cx.notify();
                    },
                ),
                &t,
            ),
        ];
        if self.expanded_workspace {
            identity.push(self.input_row("workspace","Workspace directory",Some("Directory where the repository is cloned; change this only when setup requires an absolute path"),&t));
        }
        let mut setup = vec![
            Self::row(
                "Container Caching",
                Some(
                    "Speed up container start by caching state after running the setup script. Maintenance scripts are run before every task.",
                ),
                self.button(
                    "cloud-cache",
                    if editor.draft.cache["post_setup_cache_enabled"] == true {
                        "On"
                    } else {
                        "Off"
                    },
                    false,
                    cx,
                    |this, _, cx| {
                        if let Some(e) = &mut this.editor {
                            e.draft.cache["post_setup_cache_enabled"] =
                                json!(e.draft.cache["post_setup_cache_enabled"] != true);
                        }
                        cx.notify();
                    },
                ),
                &t,
            ),
            Self::row(
                "Setup script",
                None,
                self.button(
                    "cloud-auto-setup",
                    if editor.draft.auto_setup["use_auto_setup"] == true {
                        "Automatic"
                    } else {
                        "Manual"
                    },
                    false,
                    cx,
                    |this, _, cx| {
                        if let Some(e) = &mut this.editor {
                            e.draft.auto_setup["use_auto_setup"] =
                                json!(e.draft.auto_setup["use_auto_setup"] != true);
                        }
                        cx.notify();
                    },
                ),
                &t,
            ),
        ];
        if editor.draft.auto_setup["use_auto_setup"] != true {
            setup.push(self.input_row("setup","Setup script",Some("Network access is available during setup, even when agent internet access is off"),&t));
            if editor.draft.cache["post_setup_cache_enabled"] == true {
                setup.push(self.input_row(
                    "maintenance",
                    "Maintenance script",
                    Some("Runs before every task when using a cached container"),
                    &t,
                ));
            }
        }
        let mut variables = vec![Self::row(
            "Environment variables",
            Some("Paste .env content into a variable name to add multiple values"),
            self.button("add-variable", "Add variable", false, cx, |this, _, cx| {
                if let Some(e) = &mut this.editor {
                    e.draft.variables.push((String::new(), String::new()));
                }
                cx.notify();
            }),
            &t,
        )];
        for (index, (key, value)) in editor.draft.variables.iter().enumerate() {
            if !value.trim().is_empty()
                && RUNTIMES.iter().any(|r| r.variable == key.trim())
                && editor.draft.machine == UNIVERSAL
                && !editor
                    .draft
                    .variables
                    .iter()
                    .enumerate()
                    .any(|(other, (name, _))| other != index && name.trim() == key.trim())
            {
                continue;
            }
            variables.push(self.variable_row(index, false, cx));
        }
        variables.push(Self::row(
            "Secrets",
            None,
            self.button("add-secret", "Add secret", false, cx, |this, _, cx| {
                if let Some(e) = &mut this.editor {
                    e.draft.secrets.push(Secret {
                        name: String::new(),
                        value: String::new(),
                        domain: None,
                        previous: None,
                    });
                }
                cx.notify();
            }),
            &t,
        ));
        if self.access.domain_secrets {
            variables.push(Self::row(
                "Domain-scoped secrets",
                Some("During tasks, secrets are sent only to their configured domain. Secrets without a domain are available only to setup scripts."),
                self.button("cloud-scoped-secrets", if editor.draft.authtranslator {"On"} else {"Off"}, false, cx, |this,_,cx|{
                    if let Some(editor)=&mut this.editor { editor.draft.authtranslator=!editor.draft.authtranslator; }
                    cx.notify();
                }),
                &t,
            ));
        }
        for index in 0..editor.draft.secrets.len() {
            variables.push(self.variable_row(index, true, cx));
        }
        let mut network = vec![Self::row(
            "Internet access",
            (!self.access.network_allowed)
                .then_some("Your workspace doesn’t allow internet access after setup"),
            if self.access.network_allowed {
                self.select(
                    Menu::Network,
                    editor.draft.network["mode"]
                        .as_str()
                        .unwrap_or("off")
                        .into(),
                    cx,
                )
            } else {
                text(
                    editor.draft.network["mode"]
                        .as_str()
                        .unwrap_or("off")
                        .to_owned(),
                    13.,
                    18.,
                    430.,
                    t.text_secondary,
                )
                .into_any_element()
            },
            &t,
        )];
        if self.access.network_allowed && editor.draft.network["mode"] != "off" {
            network.push(self.input_row("domains", "Allowed domains", None, &t));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(Self::group("", identity, &t))
            .when(self.access.workspace, |d| {
                d.child(Self::group(
                    "Sharing",
                    vec![
                        Self::row(
                            "Visibility",
                            None,
                            self.select(Menu::Sharing, editor.draft.sharing.clone(), cx),
                            &t,
                        ),
                        self.input_row(
                            "editors",
                            "Additional editors",
                            Some("Enter one email address per line"),
                            &t,
                        ),
                    ],
                    &t,
                ))
            })
            .child(Self::group("Code execution", setup, &t))
            .child(Self::group("Variables and secrets", variables, &t))
            .child(Self::group("Agent internet access", network, &t))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        self.button("cloud-cancel", "Cancel", false, cx, |this, _, cx| {
                            this.cancel_edit(cx)
                        }),
                    )
                    .child(self.button(
                        "cloud-save",
                        if editor.original.is_some() {
                            "Save changes"
                        } else {
                            "Create environment"
                        },
                        true,
                        cx,
                        |this, _, cx| this.save(cx),
                    )),
            )
            .into_any_element()
    }

    fn variable_row(&self, index: usize, secret: bool, cx: &mut Context<Self>) -> AnyElement {
        let prefix = if secret { "secret" } else { "variable" };
        let editor = self.editor.as_ref().unwrap();
        let fields = [
            format!("{prefix}-name-{index}"),
            format!("{prefix}-value-{index}"),
        ];
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .p(px(12.))
            .children(
                fields
                    .iter()
                    .filter_map(|key| editor.fields.get(key).cloned())
                    .map(|field| div().flex_1().child(field)),
            )
            .when(secret && editor.draft.authtranslator, |d| {
                d.children(
                    editor
                        .fields
                        .get(&format!("secret-domain-{index}"))
                        .cloned()
                        .map(|field| div().flex_1().child(field)),
                )
            })
            .child(self.button(
                format!("remove-{prefix}-{index}"),
                "Remove",
                false,
                cx,
                move |this, _, cx| {
                    if let Some(draft) = this.draft(cx) {
                        if let Some(e) = &mut this.editor {
                            e.draft = draft;
                            if secret {
                                e.draft.secrets.remove(index);
                            } else {
                                e.draft.variables.remove(index);
                            }
                            e.fields.retain(|k, _| !k.starts_with(prefix));
                        }
                    }
                    cx.notify();
                },
            ))
            .into_any_element()
    }

    fn prepare_fields(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let mut missing = Vec::new();
        for (index, (key, value)) in editor.draft.variables.iter().enumerate() {
            for (part, label, value) in [
                ("name", "Variable name", key.as_str()),
                ("value", "Variable value", value.as_str()),
            ] {
                let id = format!("variable-{part}-{index}");
                if !editor.fields.contains_key(&id) {
                    missing.push((id, label, value.to_owned(), false));
                }
            }
        }
        for (index, secret) in editor.draft.secrets.iter().enumerate() {
            for (part, label, value, masked) in [
                ("name", "Secret name", secret.name.as_str(), false),
                ("value", "Secret value", secret.value.as_str(), true),
                (
                    "domain",
                    "Secret domain",
                    secret.domain.as_deref().unwrap_or(""),
                    false,
                ),
            ] {
                let id = format!("secret-{part}-{index}");
                if !editor.fields.contains_key(&id) {
                    missing.push((id, label, value.to_owned(), masked));
                }
            }
        }
        for (id, label, value, secret) in missing {
            let field = self.field(label, &value, false, secret, cx);
            if label == "Variable name" || label == "Secret name" {
                let index = id
                    .rsplit('-')
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap();
                let secret = label == "Secret name";
                let weak = cx.entity().downgrade();
                field.update(cx, |field, _| {
                    field.on_settings_paste(move |text, cx| {
                        let Some(entries) = model::parse_assignments(text) else {
                            return false;
                        };
                        let weak = weak.clone();
                        cx.defer(move |cx| {
                            weak.update(cx, |this, cx| {
                                let Some(draft) = this.draft(cx) else {
                                    return;
                                };
                                let Some(editor) = &mut this.editor else {
                                    return;
                                };
                                editor.draft = draft;
                                if secret {
                                    if index >= editor.draft.secrets.len() {
                                        return;
                                    }
                                    let mut replacement: Vec<_> = entries
                                        .into_iter()
                                        .map(|(name, value)| Secret {
                                            name,
                                            value,
                                            domain: None,
                                            previous: None,
                                        })
                                        .collect();
                                    replacement[0].previous =
                                        editor.draft.secrets[index].previous.clone();
                                    replacement[0].domain =
                                        editor.draft.secrets[index].domain.clone();
                                    editor.draft.secrets.splice(index..index + 1, replacement);
                                } else {
                                    if index >= editor.draft.variables.len() {
                                        return;
                                    }
                                    editor.draft.variables.splice(index..index + 1, entries);
                                }
                                editor.fields.retain(|key, _| {
                                    !key.starts_with(if secret { "secret-" } else { "variable-" })
                                });
                                cx.notify();
                            })
                            .ok();
                        });
                        true
                    })
                });
            }
            self.editor.as_mut().unwrap().fields.insert(id, field);
        }
    }

    fn overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = cx.theme().clone();
        let content = if self.unavailable {
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(text(
                    "Cloud environments unavailable",
                    20.,
                    28.,
                    600.,
                    t.text,
                ))
                .child(text(
                    "Cloud environment setup isn't available in this workspace",
                    14.,
                    21.,
                    430.,
                    t.text_secondary,
                ))
                .child(self.button(
                    "cloud-unavailable-close",
                    "Close",
                    true,
                    cx,
                    |_, window, cx| {
                        window.dispatch_action(Box::new(crate::GoBack), cx);
                    },
                ))
                .into_any_element()
        } else if let Some(action) = self.confirm {
            let (title, description, button) = match action {
                "delete" => (
                    format!(
                        "Delete {}?",
                        self.selected
                            .as_ref()
                            .map(|e| e.label.as_str())
                            .unwrap_or("environment")
                    ),
                    "Existing chats will remain, but you will not be able to create follow-ups with this environment",
                    "Delete environment",
                ),
                "reset" => (
                    "Reset cached containers?".into(),
                    "This invalidates all cached containers for this environment and all users",
                    "Reset cache",
                ),
                "config-delete" => (
                    "Delete environment?".into(),
                    "Existing chats will remain, but you will not be able to create follow-ups with this environment",
                    "Delete environment",
                ),
                _ => (
                    "Discard changes?".into(),
                    "Your unsaved changes will be lost",
                    "Discard changes",
                ),
            };
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(text(title, 20., 28., 600., t.text))
                .child(text(description, 14., 21., 430., t.text_secondary))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            self.button("confirm-cancel", "Cancel", false, cx, |this, _, cx| {
                                this.confirm = None;
                                this.discard_to_list = false;
                                cx.notify();
                            }),
                        )
                        .child(self.button(
                            "confirm-action",
                            button,
                            true,
                            cx,
                            move |this, _, cx| {
                                match action {
                                    "discard" => {
                                        this.leave_editor();
                                        if this.view == View::List {
                                            this.refresh(cx);
                                        }
                                    }
                                    "delete" => {
                                        if let Some(e) = &this.selected {
                                            this.load(Operation::Delete(e.id.clone()), cx)
                                        }
                                    }
                                    "reset" => {
                                        if let Some(e) = &this.selected {
                                            this.load(Operation::Reset(e.id.clone()), cx)
                                        }
                                    }
                                    "config-delete" => {
                                        if let Some(id) = this.current.deleting.clone() {
                                            this.load(Operation::ConfigDelete(id), cx)
                                        }
                                    }
                                    _ => {}
                                }
                                cx.notify();
                            },
                        )),
                )
                .into_any_element()
        } else if self.packages {
            let draft = self.draft(cx)?;
            let rows = RUNTIMES
                .iter()
                .enumerate()
                .map(|(index, runtime)| {
                    Self::row(
                        runtime.name,
                        None,
                        self.select(
                            Menu::Runtime(index),
                            draft
                                .variables
                                .iter()
                                .find(|(k, _)| k.trim() == runtime.variable)
                                .map(|(_, v)| v.as_str())
                                .unwrap_or("Image default")
                                .into(),
                            cx,
                        ),
                        &t,
                    )
                })
                .collect();
            div().flex().flex_col().gap(px(16.)).child(text("Preinstalled packages",20.,28.,600.,t.text))
                .child(text("Choose language versions for the Universal image. Install additional packages in your setup script.",14.,21.,430.,t.text_secondary)).child(Self::group("",rows,&t))
                .child(self.button("packages-link","View all preinstalled packages",false,cx,|_,_,cx|cx.open_url("https://github.com/openai/codex-universal")))
                .child(self.button("packages-done","Done",true,cx,|this,_,cx|{this.packages=false;cx.notify();})).into_any_element()
        } else if self.repository_picker {
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text("Select repository", 20., 28., 600., t.text))
                .children(self.repositories.iter().filter_map(|repo| {
                    let id = repo["id"].as_str()?.to_owned();
                    let name = repo["repository_full_name"]
                        .as_str()
                        .or_else(|| repo["full_name"].as_str())?
                        .to_owned();
                    let repository = repo.clone();
                    Some(self.button(
                        format!("repo-{id}"),
                        name.clone(),
                        false,
                        cx,
                        move |this, _, cx| {
                            if this.current.creating {
                                if this.current.repositories.iter().any(|r| r["id"] == id) {
                                    this.current.repositories.retain(|r| r["id"] != id);
                                } else {
                                    this.current.repositories.push(repository.clone());
                                }
                            } else {
                                if let Some(e) = &mut this.editor {
                                    e.draft.repository = id.clone();
                                    e.draft.repository_name = name.clone();
                                    if e.fields["name"].read(cx).field_text().is_empty() {
                                        e.fields["name"]
                                            .update(cx, |field, cx| field.set_text(&name, cx));
                                    }
                                }
                                this.repository_picker = false;
                            }
                            cx.notify();
                        },
                    ))
                }))
                .when(self.current.creating, |d| {
                    d.child(
                        self.button("config-start", "Get started", true, cx, |this, _, cx| {
                            this.repository_picker = false;
                            this.current_create(cx);
                        }),
                    )
                })
                .child(
                    self.button("repository-cancel", "Cancel", false, cx, |this, _, cx| {
                        this.repository_picker = false;
                        this.current.creating = false;
                        cx.notify();
                    }),
                )
                .into_any_element()
        } else {
            return None;
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x00000060))
                .child(
                    div()
                        .w(px(640.))
                        .max_h(px(760.))
                        .p(px(24.))
                        .rounded(px(24.))
                        .bg(t.background_panel)
                        .child(content),
                )
                .into_any_element(),
        )
    }

    fn menu_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.clone()?;
        let t = cx.theme().clone();
        let options: Vec<(String, String)> = match menu {
            Menu::Machine => self
                .machines
                .iter()
                .filter_map(|m| Some((m["id"].as_str()?.into(), m["label"].as_str()?.into())))
                .collect(),
            Menu::Sharing => vec![
                ("private".into(), "Only me".into()),
                ("workspace".into(), "Shared with workspace".into()),
            ],
            Menu::Network => vec![
                ("off".into(), "Off".into()),
                ("on".into(), "On".into()),
                ("custom".into(), "Custom".into()),
            ],
            Menu::Runtime(index) => std::iter::once((String::new(), "Image default".into()))
                .chain(
                    RUNTIMES[index]
                        .versions
                        .iter()
                        .map(|v| ((*v).into(), (*v).into())),
                )
                .collect(),
        };
        let rows = options.into_iter().map(|(value, label)| {
            let target = menu.clone();
            self.button(
                format!("cloud-option-{value}"),
                label,
                false,
                cx,
                move |this, _, cx| {
                    if let Some(draft) = this.draft(cx) {
                        if let Some(e) = &mut this.editor {
                            e.draft = draft;
                            match target {
                                Menu::Machine => e.draft.machine = value.clone(),
                                Menu::Sharing => e.draft.sharing = value.clone(),
                                Menu::Network => {
                                    e.draft.network["mode"] = value.clone().into();
                                    e.draft.network["preset_allowlist"] =
                                        if value == "on" { "all" } else { "none" }.into();
                                }
                                Menu::Runtime(index) => {
                                    e.draft.set_runtime(
                                        &RUNTIMES[index],
                                        (!value.is_empty()).then_some(value.as_str()),
                                    );
                                    e.fields.retain(|k, _| !k.starts_with("variable-"));
                                }
                            }
                        }
                    }
                    this.menu = None;
                    cx.notify();
                },
            )
        });
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(popover(&t, 288., |d| d.children(rows)))
                .into_any_element(),
        )
    }
}

impl Render for CloudPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_fields(cx);
        if self.current_mode == Some(true) {
            return self.current_screen(cx);
        }
        let t = cx.theme().clone();
        let title = match self.view {
            View::List => "Legacy Codex Cloud".to_owned(),
            View::Detail => self
                .selected
                .as_ref()
                .map(|e| e.label.clone())
                .unwrap_or_else(|| "Cloud environment".into()),
            View::Editor => {
                if self.editor.as_ref().is_some_and(|e| e.original.is_some()) {
                    "Edit cloud environment".into()
                } else {
                    "Create cloud environment".into()
                }
            }
        };
        let body = match self.view {
            View::List => self.list(cx),
            View::Detail => self.detail(cx),
            View::Editor => self.editor(cx),
        };
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(self.legacy_header(&title, cx))
            .child(
                div()
                    .id("cloud-environments")
                    .overflow_y_scroll()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .layer()
                    .child(
                        div()
                            .w_full()
                            .pt(px(16.))
                            .pb(px(1.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(768.))
                                    .px(px(20.))
                                    .py(px(12.))
                                    .child(text(title, 28., 33.6, 500., t.text)),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .px(px(20.))
                            .pt(px(20.))
                            .pb(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(20.))
                            .when(self.access.legacy, |d| d.child(body))
                            .when(!self.access_loaded, |d| {
                                d.child(text(
                                    "Loading cloud access…",
                                    14.,
                                    21.,
                                    430.,
                                    t.text_secondary,
                                ))
                            })
                            .when(self.pending, |d| {
                                d.child(text("Loading…", 14., 21., 430., t.text_secondary))
                            })
                            .children(self.error.as_ref().map(|error| {
                                div()
                                    .child(text(error.clone(), 14., 21., 430., t.text_secondary))
                                    .children(self.retry.as_ref().map(|request| {
                                        let request = request.clone();
                                        self.button(
                                            "cloud-retry",
                                            "Retry",
                                            false,
                                            cx,
                                            move |this, _, cx| this.load(request.clone(), cx),
                                        )
                                    }))
                            })),
                    ),
            )
            .children(self.overlay(cx))
            .children(self.menu_overlay(cx))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_failure_stays_in_the_editor_and_retry_can_recover() {
        let mut app = gpui::HeadlessAppContext::with_asset_source(
            textsys::text_system(),
            std::sync::Arc::new(theme::Assets),
        );
        app.update(|cx| {
            let page = cx.new(CloudPage::new);
            page.update(cx, |page, cx| {
                page.edit(None, cx);
                let failed = Request {
                    serial: page.serial,
                    operation: Operation::Machines,
                };
                page.apply(&failed, Err("machine request failed".into()), cx);
                assert!(page.machine_error);
                assert!(page.error.is_none());
                assert!(page.editor.is_some());
                assert!(!page.pending);

                page.load(Operation::Machines, cx);
                let retried = Request {
                    serial: page.serial,
                    operation: Operation::Machines,
                };
                page.apply(&failed, Err("stale failure".into()), cx);
                page.apply(&retried, Ok(json!([])), cx);
                assert!(!page.machine_error);
                assert!(page.error.is_none());
                assert!(page.editor.is_some());
                assert!(!page.pending);
            });
        });
    }
}
