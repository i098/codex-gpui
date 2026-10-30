use super::{Layer, deco, on_card, probe};
use crate::screens::sidebar::Project;
use crate::{
    composer::Composer,
    confirmation::{Confirmation, Outcome},
    focus,
    icon::icon,
    text,
};
use bridge::types::Thread;
use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Div, Entity, Focusable as _,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde::Deserialize;
use serde_json::Value;
use shapes::Corner::Squircle;
use std::collections::HashSet;
use theme::ActiveTheme as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Load,
    Restore(String),
    RestoreCloud(String),
    Delete(DeleteScope),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteScope {
    Single(String),
    Project(Vec<String>),
    All,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = archived_chats, no_json)]
pub struct ArchiveAction {
    pub page: Entity<ArchivedChats>,
    pub operation: Operation,
}

#[derive(Clone, Deserialize)]
pub struct CloudArchive {
    pub id: String,
    pub title: Option<String>,
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    #[serde(default)]
    pub task_status_display: Value,
}

impl CloudArchive {
    fn title(&self) -> &str {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("Untitled chat")
    }

    fn created(&self) -> i64 {
        self.created_at
            .or(self.updated_at)
            .filter(|value| value.is_finite())
            .unwrap_or(0.) as i64
    }

    fn updated(&self) -> i64 {
        self.updated_at
            .or(self.created_at)
            .filter(|value| value.is_finite())
            .unwrap_or(0.) as i64
    }

    fn environment(&self) -> &str {
        self.task_status_display["environment_label"]
            .as_str()
            .unwrap_or("")
    }
}

pub struct ArchiveData {
    pub threads: anyhow::Result<Vec<Thread>>,
    pub projects: Vec<Project>,
    pub metadata: Value,
    pub metadata_error: Option<String>,
}

#[derive(Default)]
struct Metadata {
    values: Value,
    projectless: HashSet<String>,
    scheduled: HashSet<String>,
}

impl From<Value> for Metadata {
    fn from(mut values: Value) -> Self {
        let ids = |key: &str| {
            values[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let projectless = ids("projectless-thread-ids");
        let scheduled = ids("automationThreadIds");
        if let Some(object) = values.as_object_mut() {
            object.remove("projectless-thread-ids");
            object.remove("automationThreadIds");
        }
        Self {
            values,
            projectless,
            scheduled,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Sort {
    #[default]
    Updated,
    Created,
    Alphabetical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    All,
    Local,
    Cloud,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ProjectFilter {
    #[default]
    All,
    Project(String),
    Chats,
    Scheduled,
}

#[derive(Clone)]
struct Group {
    project: Option<usize>,
    rows: Vec<usize>,
}

#[derive(Clone, PartialEq)]
enum Menu {
    Filter,
    Projects,
    Project(usize),
}

#[derive(Clone, PartialEq)]
enum Control {
    Restore(String),
    RestoreCloud(String),
    DeleteRow(String),
    DeleteAll,
    Menu(Menu),
    Retry,
    ClearSearch,
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("archived-settings", cx, |window, cx| {
            ArchivedChats::new(window, cx)
        })
        .into_any_element()
}

pub struct ArchivedChats {
    pub threads: Option<anyhow::Result<Vec<Thread>>>,
    cloud: Vec<CloudArchive>,
    cloud_pending: HashSet<String>,
    cloud_loading: bool,
    cloud_error: Option<String>,
    projects: Vec<Project>,
    metadata: Metadata,
    metadata_error: Option<String>,
    query: Entity<Composer>,
    kind: Kind,
    project_filter: ProjectFilter,
    sort: Sort,
    groups: Vec<Group>,
    pending: HashSet<String>,
    removed: HashSet<String>,
    menu: Option<Menu>,
    menu_hovered: bool,
    menu_items: Vec<gpui::FocusHandle>,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl gpui::EventEmitter<Operation> for ArchivedChats {}

impl ArchivedChats {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx
            .new(|cx| Composer::embedded_field("Search archived chats", "", px(14.), px(18.), cx));
        cx.observe(&query, |this, _, cx| {
            this.rebuild(cx);
            cx.notify();
        })
        .detach();
        let page = cx.entity();
        window.defer(cx, move |window, cx| {
            window.dispatch_action(
                Box::new(ArchiveAction {
                    page,
                    operation: Operation::Load,
                }),
                cx,
            );
        });
        Self {
            threads: None,
            cloud: Vec::new(),
            cloud_pending: HashSet::new(),
            cloud_loading: true,
            cloud_error: None,
            projects: Vec::new(),
            metadata: Metadata::default(),
            metadata_error: None,
            query,
            kind: Kind::All,
            project_filter: ProjectFilter::All,
            sort: Sort::Updated,
            groups: Vec::new(),
            pending: HashSet::new(),
            removed: HashSet::new(),
            menu: None,
            menu_hovered: false,
            menu_items: Vec::new(),
            confirmation: None,
            layer: Layer::default(),
        }
    }

    pub fn confirmation(&self) -> Option<Entity<Confirmation>> {
        self.confirmation.clone()
    }

    pub fn loaded(&mut self, data: ArchiveData, cx: &mut Context<Self>) {
        self.projects = data.projects;
        self.metadata = data.metadata.into();
        self.metadata_error = data.metadata_error;
        self.threads = Some(data.threads.map(|mut threads| {
            threads.retain(|thread| !self.removed.contains(&thread.id));
            threads
        }));
        self.rebuild(cx);
        cx.notify();
    }

    pub fn cloud_loaded(
        &mut self,
        result: anyhow::Result<Vec<CloudArchive>>,
        cx: &mut Context<Self>,
    ) {
        self.cloud_loading = false;
        match result {
            Ok(mut cloud) => {
                cloud.retain(|task| !self.removed.contains(&task.id));
                self.cloud = cloud;
                self.cloud_error = None;
            }
            Err(error) => self.cloud_error = Some(error.to_string()),
        }
        self.rebuild(cx);
        cx.notify();
    }

    pub fn finished(&mut self, ids: &[String], success: bool, cx: &mut Context<Self>) {
        for id in ids {
            self.pending.remove(id);
        }
        if success {
            self.removed.extend(ids.iter().cloned());
            if let Some(Ok(threads)) = &mut self.threads {
                let removed: HashSet<&str> = ids.iter().map(String::as_str).collect();
                threads.retain(|thread| !removed.contains(thread.id.as_str()));
            }
        }
        self.rebuild(cx);
        cx.notify();
    }

    pub fn deletion_finished(&mut self, ids: &[String], cx: &mut Context<Self>) {
        self.pending.clear();
        self.finished(ids, true, cx);
    }

    pub fn cloud_finished(&mut self, id: &str, success: bool, cx: &mut Context<Self>) {
        self.cloud_pending.remove(id);
        if success {
            self.removed.insert(id.to_owned());
            self.cloud.retain(|task| task.id != id);
        }
        self.rebuild(cx);
        cx.notify();
    }

    fn local_threads(&self) -> &[Thread] {
        self.threads
            .as_ref()
            .and_then(|threads| threads.as_ref().ok())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn restore_cloud(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.pending.is_empty() || !self.cloud_pending.insert(id.clone()) {
            return;
        }
        cx.emit(Operation::RestoreCloud(id));
        cx.notify();
    }

    fn rebuild(&mut self, cx: &App) {
        self.groups = project_groups(
            self.local_threads(),
            &self.cloud,
            &self.projects,
            &self.metadata,
            self.query.read(cx).field_text(),
            (self.kind, &self.project_filter, self.sort),
            &self.pending,
        );
    }

    fn restore(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.pending.is_empty() {
            return;
        }
        self.pending.insert(id.clone());
        self.rebuild(cx);
        cx.emit(Operation::Restore(id));
        cx.notify();
    }

    fn confirm_delete(
        &mut self,
        project: Option<usize>,
        row: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pending.is_empty() {
            return;
        }
        let Some(Ok(threads)) = &self.threads else {
            return;
        };
        let (title, description, ids, scope) = if let Some(id) = row.as_deref() {
            let Some(thread) = threads.iter().find(|thread| thread.id == id) else {
                return;
            };
            (
                "Delete archived chat?",
                "This permanently deletes the archived chat".to_owned(),
                vec![thread.id.clone()],
                DeleteScope::Single(thread.id.clone()),
            )
        } else if let Some(project) = project {
            let ids: Vec<_> = threads
                .iter()
                .filter(|thread| {
                    project_for(
                        &thread.id,
                        Some(&thread.cwd),
                        &self.projects,
                        &self.metadata,
                    ) == Some(project)
                })
                .map(|thread| thread.id.clone())
                .collect();
            let noun = if ids.len() == 1 { "chat" } else { "chats" };
            let scope = DeleteScope::Project(ids.clone());
            (
                "Delete all in project?",
                format!(
                    "This permanently deletes {} local archived {noun} in this project",
                    ids.len()
                ),
                ids,
                scope,
            )
        } else {
            (
                "Delete all archived local chats?",
                "This permanently deletes all local archived chats".to_owned(),
                threads.iter().map(|thread| thread.id.clone()).collect(),
                DeleteScope::All,
            )
        };
        if ids.is_empty() {
            return;
        }
        self.menu = None;
        let confirmation = cx.new(|cx| {
            Confirmation::new(title, description, "Delete", window, cx)
                .destructive()
                .settings_style()
        });
        cx.subscribe_in(&confirmation, window, move |this, _, outcome, _, cx| {
            this.confirmation = None;
            if *outcome == Outcome::Confirm {
                this.pending.extend(ids.iter().cloned());
                this.rebuild(cx);
                cx.emit(Operation::Delete(scope.clone()));
            }
            cx.notify();
        })
        .detach();
        self.confirmation = Some(confirmation);
        cx.notify();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.threads = None;
        self.cloud_loading = true;
        self.cloud_error = None;
        self.metadata_error = None;
        self.rebuild(cx);
        cx.emit(Operation::Load);
        cx.notify();
    }

    fn control_body(command: &Control, label: &SharedString, open: bool, cx: &App) -> Div {
        let t = cx.theme();
        if matches!(command, Control::Menu(Menu::Filter | Menu::Projects)) {
            let leading = if command == &Control::Menu(Menu::Filter) {
                "archive-filter"
            } else {
                "archive-folder"
            };
            div()
                .relative()
                .h(px(28.))
                .px(px(13.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(4.))
                .child(on_card(
                    12.5,
                    Squircle,
                    if open {
                        t.ghost_hover
                    } else {
                        t.primary_soft_alpha
                    },
                    Some(t.border),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(leading, 16., t.text))
                        .child(text(label.clone(), 13., 18., 430., t.text).truncate()),
                )
                .child(icon("chevron-down", 14., t.text_tertiary))
        } else if command == &Control::ClearSearch {
            div()
                .size(px(16.))
                .child(icon("shortcut-clear", 16., t.text_secondary))
        } else {
            let destructive = command == &Control::DeleteAll;
            let (height, color, fill) = if destructive {
                (32., t.text_error, t.text_error.opacity(0.1))
            } else {
                (28., t.text, t.text_alpha_5)
            };
            let mut content = div()
                .relative()
                .h(px(height))
                .px(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.));
            if matches!(
                command,
                Control::DeleteRow(_) | Control::Menu(Menu::Project(_))
            ) {
                content = content.w(px(28.)).px_0().child(icon(
                    if matches!(command, Control::DeleteRow(_)) {
                        "archive-delete"
                    } else {
                        "diff-more"
                    },
                    16.,
                    t.text_tertiary,
                ));
            } else {
                content = content.child(on_card(12.5, Squircle, fill, None));
                if destructive {
                    content = content.child(icon("archive-delete", 16., color));
                }
                content = content.child(text(label.clone(), 13., 18., 430., color));
            }
            content
        }
    }

    fn control_menu_key(
        &mut self,
        event: &KeyDownEvent,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        focus::tab(event, window, cx);
        if event.keystroke.key == "escape" && self.menu.is_some() {
            self.menu = None;
            cx.notify();
        } else if event.keystroke.key == "down" && open {
            self.move_menu(1, window, cx);
        } else {
            return false;
        }
        cx.stop_propagation();
        true
    }

    fn control(
        &self,
        id: SharedString,
        label: SharedString,
        command: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let disabled = (!self.pending.is_empty()
            && !matches!(command, Control::Menu(_) | Control::ClearSearch))
            || matches!(&command, Control::RestoreCloud(id) if self.cloud_pending.contains(id));
        let focus = focus::handle(id.clone(), 0, window, cx);
        let handle = focus.read(cx).handle.clone();
        let open = matches!(&command, Control::Menu(menu) if self.menu.as_ref() == Some(menu));
        let mut body = Self::control_body(&command, &label, open, cx)
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(disabled);
        if command == Control::Menu(Menu::Filter) {
            body = body.aria_label("Filter archived chats");
        } else if command == Control::Menu(Menu::Projects) {
            body = body.aria_label("Filter archived chats by project");
        }
        let popup = open.then(|| self.popup(window, cx)).flatten();
        let click = command.clone();
        let key_focus = handle.clone();
        focus::ring(body.children(popup), &focus, px(12.5), px(0.), cx)
            .when(disabled, |d| d.opacity(0.4))
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                // Keys on a focused menu row belong to the popup inside this control.
                if open && !key_focus.is_focused(window) {
                    return;
                }
                let key = event.keystroke.key.as_str();
                if !this.control_menu_key(event, open, window, cx)
                    && !disabled
                    && (matches!(key, "enter" | "space")
                        || (key == "down" && matches!(command, Control::Menu(_))))
                {
                    this.activate(command.clone(), window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                handle.focus(window, cx)
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    this.activate(click.clone(), window, cx);
                }
            }))
            .when(open, |d| {
                d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if !this.menu_hovered {
                        this.menu = None;
                        cx.notify();
                    }
                }))
            })
    }

    fn activate(&mut self, command: Control, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Control::Restore(id) => self.restore(id, cx),
            Control::RestoreCloud(id) => self.restore_cloud(id, cx),
            Control::DeleteRow(row) => self.confirm_delete(None, Some(row), window, cx),
            Control::DeleteAll => self.confirm_delete(None, None, window, cx),
            Control::Retry => self.retry(cx),
            Control::ClearSearch => {
                self.query.update(cx, |query, cx| query.set_text("", cx));
                self.query.focus_handle(cx).focus(window, cx);
            }
            Control::Menu(menu) => {
                self.menu_hovered = false;
                self.menu = if self.menu.as_ref() == Some(&menu) {
                    None
                } else {
                    let items = self.menu_entries(&menu).len();
                    self.menu_items = (0..items).map(|_| cx.focus_handle()).collect();
                    Some(menu)
                };
                cx.notify();
            }
        }
    }

    /// Moves focus by `delta` among the open menu's selectable rows, wrapping at the ends.
    fn move_menu(&self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.menu else {
            return;
        };
        let rows: Vec<_> = self
            .menu_entries(menu)
            .iter()
            .zip(&self.menu_items)
            .filter(|((_, _, choice), _)| choice.is_some())
            .map(|(_, handle)| handle)
            .collect();
        if rows.is_empty() {
            return;
        }
        let next = match rows.iter().position(|handle| handle.is_focused(window)) {
            Some(current) => (current as isize + delta).rem_euclid(rows.len() as isize) as usize,
            None if delta < 0 => rows.len() - 1,
            None => 0,
        };
        rows[next].focus(window, cx);
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let locals = self.local_threads();
        let (label, date, id, cloud) = if let Some(thread) = locals.get(index) {
            (
                title(thread).to_owned(),
                row_date(thread.updated_at, thread.created_at),
                &thread.id,
                false,
            )
        } else {
            let task = &self.cloud[index - locals.len()];
            (
                task.title().to_owned(),
                row_date(task.updated(), task.created()),
                &task.id,
                true,
            )
        };
        let restoring = cloud && self.cloud_pending.contains(id);
        let operation = if cloud {
            Control::RestoreCloud(id.clone())
        } else {
            Control::Restore(id.clone())
        };
        let restore = self
            .control(
                format!("unarchive-{id}").into(),
                "Unarchive".into(),
                operation,
                window,
                cx,
            )
            .when(restoring, |button| {
                button.child(icon("sidebar-cmd-spinner", 12., t.text))
            });
        let delete = (!cloud).then(|| {
            self.control(
                format!("delete-archived-{id}").into(),
                "Delete archived chat".into(),
                Control::DeleteRow(id.clone()),
                window,
                cx,
            )
        });
        div()
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .px(px(16.))
            .py(px(12.))
            .when(cloud, |row| {
                row.child(icon("cloud-web", 16., t.text_secondary))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(label, 13., 18.571_428, 500., t.text).truncate())
                    .child(text(date, 12., 16., 430., t.text_secondary).truncate()),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(px(160.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .children(delete)
                    .child(restore),
            )
    }

    fn card(&self, t: &theme::Theme) -> Div {
        div().relative().p(px(1.)).child(deco(
            &self.layer,
            20.,
            Squircle,
            t.background_panel,
            Some(t.border),
        ))
    }

    fn error_row(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let control = self.control(
            "archive-retry".into(),
            "Retry".into(),
            Control::Retry,
            window,
            cx,
        );
        self.card(&t).child(crate::rounded_clip(
            px(19.),
            Squircle,
            super::settings_row(
                "Could not load archived chats",
                None,
                control.into_any_element(),
                &t,
            ),
        ))
    }

    fn message_row(&self, label: &'static str, glyph: Option<&'static str>, cx: &App) -> Div {
        let t = cx.theme();
        self.card(t).child(
            div()
                .px(px(16.))
                .py(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .children(glyph.map(|glyph| icon(glyph, 16., t.text_secondary)))
                .child(text(label, 13., 18.571_428, 400., t.text_secondary)),
        )
    }

    fn filter_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let kind = match self.kind {
            Kind::All => "All chats",
            Kind::Local => "Local",
            Kind::Cloud => "Cloud",
        };
        let project: SharedString = match &self.project_filter {
            ProjectFilter::All => "All projects".into(),
            ProjectFilter::Chats => "Chats".into(),
            ProjectFilter::Scheduled => "Scheduled tasks".into(),
            ProjectFilter::Project(id) => self
                .projects
                .iter()
                .find(|p| p.id.as_ref() == id)
                .map(|p| p.label.clone())
                .unwrap_or_else(|| "All projects".into()),
        };
        let kind_button = self
            .control(
                "archive-filter".into(),
                kind.into(),
                Control::Menu(Menu::Filter),
                window,
                cx,
            )
            .w(px(144.));
        let project_button = self
            .control(
                "archive-projects".into(),
                project,
                Control::Menu(Menu::Projects),
                window,
                cx,
            )
            .w(px(176.));
        let t = cx.theme();
        let search = div()
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .px(px(11.))
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(on_card(
                9999.,
                shapes::Corner::Round,
                t.surface_elevated_secondary,
                Some(t.border_strong),
            ))
            .child(icon("shortcut-search", 18., t.text_secondary))
            .child(div().flex_1().min_w_0().child(self.query.clone()));
        let clear = (!self.query.read(cx).field_text().is_empty()).then(|| {
            self.control(
                "clear-archive-search".into(),
                "Clear search".into(),
                Control::ClearSearch,
                window,
                cx,
            )
        });
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(search.children(clear))
            .child(kind_button)
            .child(project_button)
    }

    fn menu_entries(&self, menu: &Menu) -> Vec<(SharedString, bool, Option<MenuChoice>)> {
        let mut entries: Vec<(SharedString, bool, Option<MenuChoice>)> = Vec::new();
        match menu {
            Menu::Filter => {
                entries.push(("Type".into(), false, None));
                for (kind, label) in [
                    (Kind::All, "All chats"),
                    (Kind::Local, "Local"),
                    (Kind::Cloud, "Cloud"),
                ] {
                    entries.push((
                        label.into(),
                        self.kind == kind,
                        Some(MenuChoice::Kind(kind)),
                    ));
                }
                entries.push(("".into(), false, None));
                entries.push(("Sort by".into(), false, None));
                for (sort, label) in [
                    (Sort::Updated, "Updated"),
                    (Sort::Created, "Created"),
                    (Sort::Alphabetical, "Alphabetical"),
                ] {
                    entries.push((
                        label.into(),
                        self.sort == sort,
                        Some(MenuChoice::Sort(sort)),
                    ));
                }
            }
            Menu::Projects => {
                entries.push((
                    "All projects".into(),
                    self.project_filter == ProjectFilter::All,
                    Some(MenuChoice::Project(ProjectFilter::All)),
                ));
                entries.push(("".into(), false, None));
                for project in &self.projects {
                    let filter = ProjectFilter::Project(project.id.to_string());
                    entries.push((
                        project.label.clone(),
                        self.project_filter == filter,
                        Some(MenuChoice::Project(filter)),
                    ));
                }
                entries.push(("".into(), false, None));
                entries.push((
                    "Chats".into(),
                    self.project_filter == ProjectFilter::Chats,
                    Some(MenuChoice::Project(ProjectFilter::Chats)),
                ));
                entries.push((
                    "Scheduled tasks".into(),
                    self.project_filter == ProjectFilter::Scheduled,
                    Some(MenuChoice::Project(ProjectFilter::Scheduled)),
                ));
            }
            Menu::Project(project) => entries.push((
                "Delete all in project".into(),
                false,
                Some(MenuChoice::DeleteProject(*project)),
            )),
        }
        entries
    }

    fn menu_row(
        &self,
        entry: (usize, (SharedString, bool, Option<MenuChoice>)),
        t: &theme::Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (index, (label, checked, choice)) = entry;
        if label.is_empty() {
            return div()
                .px(px(8.))
                .py(px(4.))
                .child(div().h(px(1.)).bg(t.border))
                .into_any_element();
        }
        let destructive = matches!(choice, Some(MenuChoice::DeleteProject(_)));
        let color = if destructive { t.text_error } else { t.text };
        let leading = match choice.as_ref() {
            Some(MenuChoice::DeleteProject(_)) => Some("archive-delete"),
            Some(MenuChoice::Project(ProjectFilter::Chats)) => Some("archive-chat"),
            Some(MenuChoice::Project(ProjectFilter::Scheduled)) => {
                Some("sidebar-cmd-manage-scheduled-tasks")
            }
            Some(MenuChoice::Project(ProjectFilter::Project(_))) => Some("archive-folder"),
            _ => None,
        };
        let focus = self.menu_items.get(index).filter(|_| choice.is_some());
        let focused = focus.is_some_and(|handle| handle.is_focused(window));
        div()
            .id(("archive-menu-item", index))
            .when_some(focus, |d, handle| {
                d.role(gpui::Role::MenuItem).track_focus(handle)
            })
            .rounded(px(15.))
            .px(px(8.))
            .py(px(if choice.is_some() { 5. } else { 4. }))
            .flex()
            .items_center()
            .gap(px(6.))
            .when(choice.is_some(), |d| d.hover(|s| s.bg(t.menu_item_hover)))
            .when(focused, |d| d.bg(t.menu_item_hover))
            .when_some(leading, |d, glyph| {
                d.child(icon(glyph, 16., color).opacity(0.75))
            })
            .child(
                text(
                    label,
                    13.,
                    18.571_428,
                    430.,
                    if choice.is_some() {
                        color
                    } else {
                        t.text_tertiary
                    },
                )
                .flex_1(),
            )
            .when(checked, |d| {
                d.child(icon("archive-check", 16., t.text).opacity(0.75))
            })
            .on_key_down(cx.listener({
                let choice = choice.clone();
                move |this, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.choose(choice.as_ref(), window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.choose(choice.as_ref(), window, cx);
                cx.stop_propagation();
            }))
            .into_any_element()
    }

    fn popup(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let t = cx.theme().clone();
        let entries = self.menu_entries(menu);
        let rows = entries
            .into_iter()
            .enumerate()
            .map(|entry| self.menu_row(entry, &t, window, cx));
        let width = match menu {
            Menu::Filter => 208.,
            Menu::Projects => 260.,
            Menu::Project(_) => 220.,
        };
        Some(
            gpui::deferred(
                div().absolute().top_0().right_0().child(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .offset(gpui::point(px(19.), px(30.)))
                        .child(
                            crate::composer::popover(&t, width, |d| d.children(rows))
                                .id("archive-menu")
                                .on_hover(
                                    cx.listener(|this, hovered, _, _| this.menu_hovered = *hovered),
                                )
                                .capture_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        match event.keystroke.key.as_str() {
                                            "escape" => {
                                                this.menu = None;
                                                cx.notify();
                                            }
                                            "down" => this.move_menu(1, window, cx),
                                            "up" => this.move_menu(-1, window, cx),
                                            _ => return,
                                        }
                                        cx.stop_propagation();
                                    },
                                )),
                        ),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn choose(&mut self, choice: Option<&MenuChoice>, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            Some(MenuChoice::Kind(kind)) => self.kind = *kind,
            Some(MenuChoice::Sort(sort)) => self.sort = *sort,
            Some(MenuChoice::Project(project)) => self.project_filter = project.clone(),
            Some(MenuChoice::DeleteProject(project)) => {
                self.confirm_delete(Some(*project), None, window, cx)
            }
            None => return,
        }
        self.menu = None;
        self.menu_hovered = false;
        self.rebuild(cx);
        cx.notify();
    }
}

#[derive(Clone)]
enum MenuChoice {
    Kind(Kind),
    Sort(Sort),
    Project(ProjectFilter),
    DeleteProject(usize),
}

impl ArchivedChats {
    fn load_failed(&self) -> bool {
        (self.kind != Kind::Cloud && self.threads.as_ref().is_some_and(Result::is_err))
            || (self.kind != Kind::Local && self.cloud_error.is_some())
            || self.metadata_error.is_some()
    }

    fn is_loading(&self) -> bool {
        self.groups.is_empty()
            && (self.threads.is_none() || (self.kind != Kind::Local && self.cloud_loading))
    }

    fn group_section(
        &self,
        group: &Group,
        t: &theme::Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut section = div().flex().flex_col();
        if self.project_filter == ProjectFilter::All {
            let label = group
                .project
                .map(|i| self.projects[i].label.clone())
                .unwrap_or_else(|| "No project".into());
            let count = group.rows.len();
            let mut header = div()
                .h(px(46.))
                .pb(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("archive-folder", 16., t.text))
                .child(text(label, 14., 21., 500., t.text).flex_1())
                .child(text(
                    format!("{count} {}", if count == 1 { "chat" } else { "chats" }),
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ));
            if let Some(project) = group.project
                && group
                    .rows
                    .iter()
                    .any(|index| *index < self.local_threads().len())
            {
                header = header.child(self.control(
                    format!("archive-project-{project}").into(),
                    "Project actions".into(),
                    Control::Menu(Menu::Project(project)),
                    window,
                    cx,
                ));
            }
            section = section.child(header);
        }
        let count = group.rows.len();
        let rows = group.rows.iter().enumerate().map(|(i, &index)| {
            self.row(index, window, cx).when(i + 1 < count, |row| {
                row.child(
                    div()
                        .absolute()
                        .left(px(16.))
                        .right(px(16.))
                        .bottom_0()
                        .h(px(1.))
                        .bg(crate::over(t.border, t.background_panel)),
                )
            })
        });
        section.child(self.card(t).child(crate::rounded_clip(
            px(19.),
            Squircle,
            div().flex().flex_col().children(rows),
        )))
    }
}

impl Render for ArchivedChats {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let has_local = !self.local_threads().is_empty();
        let has_threads = has_local || !self.cloud.is_empty();
        let show_error = self.load_failed();
        let controls = has_threads.then(|| self.filter_controls(window, cx));
        let mut content = div().flex().flex_col().gap(px(40.));
        match &self.threads {
            _ if self.is_loading() => {
                content = content.child(self.message_row(
                    "Loading archived chats…",
                    Some("sidebar-cmd-spinner"),
                    cx,
                ))
            }
            Some(_) if !has_threads && show_error => {
                content = content.child(self.error_row(window, cx));
            }
            Some(Ok(threads)) if threads.is_empty() && self.cloud.is_empty() => {
                content = content.child(self.message_row("No archived chats", None, cx))
            }
            Some(_) if self.groups.is_empty() && self.pending.is_empty() && !show_error => {
                content = content.child(self.message_row("No matching archived chats", None, cx))
            }
            _ => {
                for group in &self.groups {
                    content = content.child(self.group_section(group, &t, window, cx));
                }
            }
        }
        if has_threads && show_error {
            content = content.child(self.error_row(window, cx));
        }
        let content = content.when(has_threads, |d| {
            d.pt(px(20.)).layer().child(shapes::top_fade(px(32.)))
        });
        let content = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(controls)
            .child(content);
        let delete_all = has_local.then(|| {
            self.control(
                "archive-delete-all".into(),
                "Delete all".into(),
                Control::DeleteAll,
                window,
                cx,
            )
        });
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("archived-chats")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
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
                                    .min_h(px(60.))
                                    .py(px(12.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(text("Archived chats", 28., 33.6, 500., t.text))
                                    .children(delete_all),
                            ),
                    )
                    .child(
                        div()
                            .pt(px(20.))
                            .pb(px(20.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(content),
                    ),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.menu_hovered {
                    this.menu = None;
                    cx.notify();
                }
            }))
    }
}

fn normalized(path: &str) -> &str {
    path.trim_end_matches('/')
}

fn title(thread: &Thread) -> &str {
    let title = thread
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| thread.preview.trim());
    if title.is_empty() {
        "Untitled chat"
    } else {
        title
    }
}

fn project_for(
    thread_id: &str,
    cwd: Option<&str>,
    projects: &[Project],
    metadata: &Metadata,
) -> Option<usize> {
    if metadata.projectless.contains(thread_id) {
        return None;
    }
    if let Some(id) = metadata.values["thread-project-assignments"][thread_id]["projectId"].as_str()
        && metadata.values["thread-project-assignments"][thread_id]["projectKind"] == "local"
        && let Some(index) = projects
            .iter()
            .position(|project| project.id.as_ref() == id)
    {
        return Some(index);
    }
    let cwd = cwd?;
    for path in [
        metadata.values["thread-workspace-root-hints"][thread_id].as_str(),
        Some(cwd),
    ]
    .into_iter()
    .flatten()
    {
        let mut matches = projects.iter().enumerate().filter(|(_, project)| {
            project
                .root
                .as_deref()
                .is_some_and(|root| normalized(root) == normalized(path))
        });
        if let Some((index, _)) = matches.next()
            && matches.next().is_none()
        {
            return Some(index);
        }
    }
    None
}

struct Row<'a> {
    id: &'a str,
    title: &'a str,
    location: &'a str,
    path: &'a str,
    created: i64,
    updated: i64,
    project: Option<usize>,
    kind: Kind,
    scheduled: bool,
}

impl Row<'_> {
    fn matches(
        &self,
        projects: &[Project],
        query: &str,
        selection: (Kind, &ProjectFilter),
        pending: &HashSet<String>,
    ) -> bool {
        let (kind, filter) = selection;
        if (self.kind == Kind::Local && pending.contains(self.id))
            || (kind != Kind::All && kind != self.kind)
        {
            return false;
        }
        let selected = match filter {
            ProjectFilter::All => true,
            ProjectFilter::Project(id) => {
                self.project.is_some_and(|i| projects[i].id.as_ref() == id)
            }
            ProjectFilter::Chats => self.project.is_none() && !self.scheduled,
            ProjectFilter::Scheduled => self.scheduled,
        };
        let project_label = self
            .project
            .map(|i| projects[i].label.as_ref())
            .unwrap_or("");
        selected
            && (query.is_empty()
                || [self.title, project_label, self.location, self.path]
                    .iter()
                    .any(|field| fuzzy_match(field, query)))
    }
}

fn project_groups(
    threads: &[Thread],
    cloud: &[CloudArchive],
    projects: &[Project],
    metadata: &Metadata,
    query: &str,
    selection: (Kind, &ProjectFilter, Sort),
    pending: &HashSet<String>,
) -> Vec<Group> {
    let rows: Vec<_> = threads
        .iter()
        .map(|thread| Row {
            id: &thread.id,
            title: title(thread),
            location: &thread.cwd,
            path: thread.path.as_deref().unwrap_or(""),
            created: thread.created_at,
            updated: thread.updated_at,
            project: project_for(&thread.id, Some(&thread.cwd), projects, metadata),
            kind: Kind::Local,
            scheduled: metadata.scheduled.contains(&thread.id),
        })
        .chain(cloud.iter().map(|task| {
            let project = project_for(&task.id, None, projects, metadata);
            Row {
                id: &task.id,
                title: task.title(),
                location: task.environment(),
                path: "",
                created: task.created(),
                updated: task.updated(),
                project,
                kind: Kind::Cloud,
                scheduled: false,
            }
        }))
        .collect();
    let (kind, filter, sort) = selection;
    let query = query.trim().to_lowercase();
    let mut groups: Vec<Group> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if !row.matches(projects, &query, (kind, filter), pending) {
            continue;
        }
        let key = if *filter == ProjectFilter::All {
            row.project
        } else {
            None
        };
        if let Some(group) = groups.iter_mut().find(|group| group.project == key) {
            group.rows.push(index);
        } else {
            groups.push(Group {
                project: key,
                rows: vec![index],
            });
        }
    }
    sort_groups(&mut groups, &rows, projects, sort);
    groups
}

fn sort_groups(groups: &mut [Group], rows: &[Row<'_>], projects: &[Project], sort: Sort) {
    for group in groups.iter_mut() {
        group.rows.sort_by(|&a, &b| {
            let (a, b) = (&rows[a], &rows[b]);
            let title_order = || {
                a.title
                    .chars()
                    .flat_map(char::to_lowercase)
                    .cmp(b.title.chars().flat_map(char::to_lowercase))
            };
            match sort {
                Sort::Alphabetical => title_order().then(b.updated.cmp(&a.updated)),
                Sort::Created => b.created.cmp(&a.created).then_with(title_order),
                Sort::Updated => b.updated.cmp(&a.updated).then_with(title_order),
            }
        });
    }
    groups.sort_by(|a, b| {
        let labels = || {
            a.project
                .map(|i| projects[i].label.as_ref())
                .unwrap_or("")
                .cmp(b.project.map(|i| projects[i].label.as_ref()).unwrap_or(""))
        };
        if sort == Sort::Alphabetical {
            labels()
        } else {
            let newest = |group: &Group| {
                group
                    .rows
                    .iter()
                    .map(|&i| {
                        if sort == Sort::Created {
                            rows[i].created
                        } else {
                            rows[i].updated
                        }
                    })
                    .max()
                    .unwrap_or(0)
            };
            newest(b).cmp(&newest(a)).then_with(labels)
        }
    });
}

fn fuzzy_match(field: &str, query: &str) -> bool {
    let field = field.to_lowercase();
    if field.contains(query) {
        return true;
    }
    let needle: Vec<_> = query.chars().collect();
    let limit = needle.len() * 2 / 5;
    let mut previous: Vec<_> = (0..=needle.len()).collect();
    let mut current = vec![0; needle.len() + 1];
    for character in field.chars() {
        current[0] = 0;
        for (index, wanted) in needle.iter().enumerate() {
            current[index + 1] = (previous[index] + usize::from(character != *wanted))
                .min(previous[index + 1] + 1)
                .min(current[index] + 1);
        }
        if current[needle.len()] <= limit {
            return true;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    false
}

fn row_date(updated: i64, created: i64) -> String {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_opt(updated, 0)
        .single()
        .or_else(|| Local.timestamp_opt(created, 0).single())
        .map(|date| {
            format!(
                "{}, {}",
                date.format("%b %-d, %Y"),
                date.format("%-I:%M %p")
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge::types::ThreadListResponse;
    use serde_json::json;

    fn recorded_threads() -> Vec<Thread> {
        include_str!("../../../../fixtures/sessions/archived-chats.jsonl")
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|line| {
                serde_json::from_value::<ThreadListResponse>(line["msg"]["result"].clone()).ok()
            })
            .expect("original archived thread response")
            .data
    }

    fn project(id: &str, root: &str) -> Project {
        Project {
            id: id.to_owned().into(),
            label: id.to_owned().into(),
            root: Some(root.to_owned().into()),
            updated_at: None,
            new_chat_label: id.to_owned().into(),
            worktree: false,
            missing: false,
            expanded: true,
        }
    }

    #[test]
    fn explicit_assignment_and_projectless_state_override_cwd() {
        let mut thread = recorded_threads().remove(0);
        thread.cwd = "/repo".into();
        let projects = vec![project("one", "/repo"), project("two", "/elsewhere")];
        let mut metadata = json!({"thread-project-assignments": {thread.id.clone(): {"projectId": "two", "projectKind": "local"}}});
        assert_eq!(
            project_for(
                &thread.id,
                Some(&thread.cwd),
                &projects,
                &Metadata::from(metadata.clone())
            ),
            Some(1)
        );
        metadata["projectless-thread-ids"] = json!([thread.id]);
        assert_eq!(
            project_for(
                &thread.id,
                Some(&thread.cwd),
                &projects,
                &Metadata::from(metadata)
            ),
            None
        );
        assert_eq!(
            project_for(
                &thread.id,
                Some(&thread.cwd),
                &[project("one", "/repo"), project("two", "/repo/")],
                &Metadata::default()
            ),
            None
        );
    }

    #[test]
    fn filtering_keeps_scheduled_chats_out_of_projectless_results() {
        let threads = recorded_threads();
        let metadata = Metadata::from(json!({"automationThreadIds": [threads[0].id]}));
        let pending = HashSet::from([threads[1].id.clone()]);
        let groups = project_groups(
            &threads,
            &[],
            &[],
            &metadata,
            "",
            (Kind::All, &ProjectFilter::Chats, Sort::Updated),
            &pending,
        );
        assert_eq!(
            groups
                .iter()
                .flat_map(|group| &group.rows)
                .copied()
                .collect::<Vec<_>>(),
            vec![2]
        );
        let groups = project_groups(
            &threads,
            &[],
            &[],
            &metadata,
            "",
            (Kind::All, &ProjectFilter::Scheduled, Sort::Updated),
            &HashSet::new(),
        );
        assert_eq!(groups[0].rows, vec![0]);
        assert!(
            project_groups(
                &threads,
                &[],
                &[],
                &metadata,
                "",
                (Kind::Cloud, &ProjectFilter::All, Sort::Updated),
                &HashSet::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn search_accepts_small_typos_and_sort_uses_the_selected_timestamp() {
        assert!(fuzzy_match("Archived math chat", "mth"));
        assert!(!fuzzy_match("Archived math chat", "zzzzz"));
        let mut threads = recorded_threads();
        threads[0].created_at = 10;
        threads[1].created_at = 30;
        threads[2].created_at = 20;
        let groups = project_groups(
            &threads,
            &[],
            &[],
            &Metadata::default(),
            "",
            (Kind::All, &ProjectFilter::All, Sort::Created),
            &HashSet::new(),
        );
        assert_eq!(groups[0].rows, vec![1, 2, 0]);
    }

    #[test]
    fn cloud_filter_uses_cloud_rows_and_searches_the_environment() {
        let threads = recorded_threads();
        let cloud: Vec<CloudArchive> = serde_json::from_value(json!([{
            "id": "archived-cloud", "title": "Review a change",
            "created_at": 1, "updated_at": 2,
            "task_status_display": {"environment_label": "Review environment"}
        }]))
        .unwrap();
        let groups = project_groups(
            &threads,
            &cloud,
            &[],
            &Metadata::default(),
            "environment",
            (Kind::Cloud, &ProjectFilter::All, Sort::Updated),
            &HashSet::new(),
        );
        assert_eq!(groups[0].rows, vec![threads.len()]);
        let groups = project_groups(
            &threads,
            &cloud,
            &[],
            &Metadata::default(),
            "environment",
            (Kind::Local, &ProjectFilter::All, Sort::Updated),
            &HashSet::new(),
        );
        assert!(groups.is_empty());
    }

    #[test]
    fn cloud_assignment_ignores_local_workspace_hints() {
        let projects = vec![project("one", "/repo")];
        let mut values = json!({"thread-workspace-root-hints": {"cloud": "/repo"}});
        assert_eq!(
            project_for("cloud", None, &projects, &Metadata::from(values.clone())),
            None
        );
        values["thread-project-assignments"] = json!({
            "cloud": {"projectKind": "local", "projectId": "one"}
        });
        assert_eq!(
            project_for("cloud", None, &projects, &Metadata::from(values)),
            Some(0)
        );
    }

    #[test]
    fn retry_after_partial_failure_clears_stale_row_indices() -> anyhow::Result<()> {
        use gpui::{AppContext as _, HeadlessAppContext, size};
        use std::sync::Arc;
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut app =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        app.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
        });
        let window = app.open_window(size(px(1280.), px(832.)), |window, cx| {
            cx.new(|cx| ArchivedChats::new(window, cx))
        })?;
        let page = app.read_window(&window, |page, _| page)?;
        app.update(|cx| {
            page.update(cx, |page, cx| {
                page.loaded(
                    ArchiveData {
                        threads: Ok(recorded_threads()),
                        projects: Vec::new(),
                        metadata: Value::Null,
                        metadata_error: Some("project metadata unavailable".into()),
                    },
                    cx,
                );
                page.cloud_loaded(Err(anyhow::anyhow!("offline")), cx);
                assert_eq!(page.local_threads().len(), 3);
                assert_eq!(
                    page.groups
                        .iter()
                        .map(|group| group.rows.len())
                        .sum::<usize>(),
                    3
                );
            })
        });
        app.run_until_parked();
        app.update_window(window.into(), |_, _, cx| {
            page.update(cx, |page, cx| page.retry(cx))
        })?;
        app.run_until_parked();
        page.read_with(&app, |page, _| {
            assert!(page.threads.is_none() && page.groups.is_empty() && page.cloud_error.is_none());
        });
        Ok(())
    }
}
