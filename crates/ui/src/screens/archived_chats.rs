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
    Action, AnyElement, App, AppContext as _, Context, Div, Entity, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use shapes::Corner::Squircle;
use std::collections::HashSet;
use theme::ActiveTheme as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Load,
    Restore(String),
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

gpui::actions!(archived_chats, [OpenArchivedChats]);

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
    DeleteRow(usize),
    DeleteAll,
    Menu(Menu),
    Retry,
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
    projects: Vec<Project>,
    metadata: Value,
    query: Entity<Composer>,
    kind: Kind,
    project_filter: ProjectFilter,
    sort: Sort,
    groups: Vec<Group>,
    pending: HashSet<String>,
    removed: HashSet<String>,
    menu: Option<Menu>,
    menu_hovered: bool,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl ArchivedChats {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| Composer::page_search("Search archived chats", cx));
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
            projects: Vec::new(),
            metadata: Value::Null,
            query,
            kind: Kind::All,
            project_filter: ProjectFilter::All,
            sort: Sort::Updated,
            groups: Vec::new(),
            pending: HashSet::new(),
            removed: HashSet::new(),
            menu: None,
            menu_hovered: false,
            confirmation: None,
            layer: Layer::default(),
        }
    }

    fn dispatch(&self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(
            Box::new(ArchiveAction {
                page: cx.entity(),
                operation,
            }),
            cx,
        );
    }

    pub fn confirmation(&self) -> Option<Entity<Confirmation>> {
        self.confirmation.clone()
    }

    pub fn loaded(
        &mut self,
        result: anyhow::Result<(Vec<Thread>, Vec<Project>, Value)>,
        cx: &mut Context<Self>,
    ) {
        self.threads = Some(result.map(|(mut threads, projects, metadata)| {
            self.projects = projects;
            self.metadata = metadata;
            threads.retain(|thread| !self.removed.contains(&thread.id));
            threads
        }));
        self.rebuild(cx);
        cx.notify();
    }

    pub fn finished(&mut self, ids: &[String], success: bool, cx: &mut Context<Self>) {
        for id in ids {
            self.pending.remove(id);
        }
        if success {
            self.removed.extend(ids.iter().cloned());
        }
        if success && let Some(Ok(threads)) = &mut self.threads {
            threads.retain(|thread| !ids.contains(&thread.id));
        }
        self.rebuild(cx);
        cx.notify();
    }

    pub fn deletion_finished(&mut self, ids: &[String], cx: &mut Context<Self>) {
        self.pending.clear();
        self.finished(ids, true, cx);
    }

    fn rebuild(&mut self, cx: &App) {
        self.groups = self
            .threads
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|threads| {
                project_groups(
                    threads,
                    &self.projects,
                    &self.metadata,
                    self.query.read(cx).field_text(),
                    (self.kind, &self.project_filter, self.sort),
                    &self.pending,
                )
            })
            .unwrap_or_default();
    }

    fn restore(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending.is_empty() {
            return;
        }
        self.pending.insert(id.clone());
        self.rebuild(cx);
        self.dispatch(Operation::Restore(id), window, cx);
        cx.notify();
    }

    fn confirm_delete(
        &mut self,
        project: Option<usize>,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pending.is_empty() {
            return;
        }
        let Some(Ok(threads)) = &self.threads else {
            return;
        };
        let (title, description, ids) = if let Some(row) = row {
            (
                "Delete archived chat?",
                "This permanently deletes the archived chat".to_owned(),
                vec![threads[row].id.clone()],
            )
        } else if let Some(project) = project {
            let ids: Vec<_> = self
                .groups
                .iter()
                .filter(|group| group.project == Some(project))
                .flat_map(|group| &group.rows)
                .map(|&index| threads[index].id.clone())
                .collect();
            let noun = if ids.len() == 1 { "chat" } else { "chats" };
            (
                "Delete all in project?",
                format!(
                    "This permanently deletes {} local archived {noun} in this project",
                    ids.len()
                ),
                ids,
            )
        } else {
            (
                "Delete all archived local chats?",
                "This permanently deletes all local archived chats".to_owned(),
                threads.iter().map(|thread| thread.id.clone()).collect(),
            )
        };
        if ids.is_empty() {
            return;
        }
        let scope = if row.is_some() {
            DeleteScope::Single(ids[0].clone())
        } else if project.is_some() {
            DeleteScope::Project(ids.clone())
        } else {
            DeleteScope::All
        };
        self.menu = None;
        let confirmation =
            cx.new(|cx| Confirmation::new(title, description, "Delete", window, cx).destructive());
        cx.subscribe_in(
            &confirmation,
            window,
            move |this, _, outcome, window, cx| {
                this.confirmation = None;
                if *outcome == Outcome::Confirm {
                    this.pending.extend(ids.iter().cloned());
                    this.rebuild(cx);
                    this.dispatch(Operation::Delete(scope.clone()), window, cx);
                }
                cx.notify();
            },
        )
        .detach();
        self.confirmation = Some(confirmation);
        cx.notify();
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.threads = None;
        self.dispatch(Operation::Load, window, cx);
        cx.notify();
    }

    fn control(
        &self,
        id: SharedString,
        label: SharedString,
        command: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let disabled = !self.pending.is_empty() && !matches!(command, Control::Menu(_));
        let focus = focus::handle(id.clone(), 0, window, cx);
        let handle = focus.read(cx).handle.clone();
        let icon_only = matches!(
            command,
            Control::DeleteRow(_) | Control::Menu(Menu::Project(_))
        );
        let mut body = div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .aria_disabled(disabled)
            .relative()
            .h(px(28.))
            .px(px(9.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.));
        if icon_only {
            body = body.w(px(28.)).px_0().child(icon(
                if matches!(command, Control::DeleteRow(_)) {
                    "archive-delete"
                } else {
                    "diff-more"
                },
                16.,
                if matches!(command, Control::DeleteRow(_)) {
                    t.text_error
                } else {
                    t.text
                },
            ));
        } else {
            let outline = matches!(command, Control::Menu(Menu::Filter | Menu::Projects));
            let destructive = command == Control::DeleteAll;
            let color = if destructive { t.text_error } else { t.text };
            let fill = if outline {
                t.primary_soft_alpha
            } else if destructive {
                t.text_error.opacity(0.1)
            } else {
                t.text_alpha_5
            };
            body = body.child(on_card(12.5, Squircle, fill, outline.then_some(t.border)));
            body = match command {
                Control::DeleteAll => body.child(icon("archive-delete", 16., color)),
                Control::Menu(Menu::Filter) => body.child(icon("archive-filter", 16., color)),
                Control::Menu(Menu::Projects) => body.child(icon("folder", 16., color)),
                _ => body,
            };
            body = body.child(text(label, 13., 18., 430., color));
        }
        let open = matches!(&command, Control::Menu(menu) if self.menu.as_ref() == Some(menu));
        let popup = open.then(|| self.popup(window, cx)).flatten();
        let click = command.clone();
        focus::ring(body.children(popup), &focus, px(12.5), px(0.), cx)
            .when(disabled, |d| d.opacity(0.4))
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                focus::tab(event, window, cx);
                if event.keystroke.key == "escape" && this.menu.is_some() {
                    this.menu = None;
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "down" && open {
                    window.focus_next(cx);
                    cx.stop_propagation();
                } else if !disabled
                    && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down")
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
            Control::Restore(id) => self.restore(id, window, cx),
            Control::DeleteRow(row) => self.confirm_delete(None, Some(row), window, cx),
            Control::DeleteAll => self.confirm_delete(None, None, window, cx),
            Control::Retry => self.retry(window, cx),
            Control::Menu(menu) => {
                self.menu = if self.menu.as_ref() == Some(&menu) {
                    None
                } else {
                    Some(menu)
                };
                cx.notify();
            }
        }
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let thread = &self
            .threads
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .expect("loaded rows")[index];
        let id = thread.id.clone();
        let restore = self.control(
            format!("unarchive-{id}").into(),
            "Unarchive".into(),
            Control::Restore(id),
            window,
            cx,
        );
        let delete = self.control(
            format!("delete-archived-{}", thread.id).into(),
            "Delete archived chat".into(),
            Control::DeleteRow(index),
            window,
            cx,
        );
        div()
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .px(px(16.))
            .py(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(title(thread).to_owned(), 13., 18.571_428, 500., t.text).truncate())
                    .child(text(thread_date(thread), 12., 16., 430., t.text_secondary).truncate()),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(px(160.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child(delete)
                    .child(restore),
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
            .w(px(144.))
            .justify_between()
            .child(icon("chevron-down", 14., cx.theme().text_tertiary));
        let project_button = self
            .control(
                "archive-projects".into(),
                project,
                Control::Menu(Menu::Projects),
                window,
                cx,
            )
            .w(px(176.))
            .justify_between()
            .child(icon("chevron-down", 14., cx.theme().text_tertiary));
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(self.query.clone()))
            .child(kind_button)
            .child(project_button)
    }

    fn popup(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let t = cx.theme().clone();
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
                for project in &self.projects {
                    let filter = ProjectFilter::Project(project.id.to_string());
                    entries.push((
                        project.label.clone(),
                        self.project_filter == filter,
                        Some(MenuChoice::Project(filter)),
                    ));
                }
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
        let rows = entries
            .into_iter()
            .enumerate()
            .map(|(index, (label, checked, choice))| {
                let focus = focus::handle(
                    SharedString::from(format!("archive-menu-{index}")),
                    0,
                    window,
                    cx,
                );
                div()
                    .id(("archive-menu-item", index))
                    .when(choice.is_some(), |d| {
                        d.role(gpui::Role::MenuItem)
                            .track_focus(&focus.read(cx).handle)
                    })
                    .rounded(px(8.))
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(choice.is_some(), |d| d.hover(|s| s.bg(t.menu_item_hover)))
                    .child(
                        text(
                            label,
                            13.,
                            18.,
                            if choice.is_some() { 430. } else { 500. },
                            if choice.is_some() {
                                t.text
                            } else {
                                t.text_tertiary
                            },
                        )
                        .flex_1(),
                    )
                    .when(checked, |d| d.child(icon("menu-check", 14., t.text)))
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
            });
        Some(
            gpui::deferred(
                div().absolute().top_0().right_0().child(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .offset(gpui::point(px(0.), px(32.)))
                        .child(
                            crate::composer::popover(&t, 220., |d| d.children(rows))
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
                                            "down" => window.focus_next(cx),
                                            "up" => window.focus_prev(cx),
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

impl Render for ArchivedChats {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let has_threads = matches!(&self.threads, Some(Ok(threads)) if !threads.is_empty());
        let mut content = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .flex()
            .flex_col()
            .gap(px(40.));
        if has_threads {
            content = content.child(self.filter_controls(window, cx));
        }
        match &self.threads {
            None => {
                content = content.child(text(
                    "Loading archived chats…",
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ))
            }
            Some(Err(_)) => {
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(text(
                            "Could not load archived chats",
                            13.,
                            18.571_428,
                            430.,
                            t.text,
                        ))
                        .child(self.control(
                            "archive-retry".into(),
                            "Retry".into(),
                            Control::Retry,
                            window,
                            cx,
                        )),
                );
            }
            Some(Ok(threads)) if threads.is_empty() => {
                content = content.child(text(
                    "No archived chats",
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ))
            }
            Some(Ok(_)) if self.groups.is_empty() => {
                content = content.child(text(
                    "No matching archived chats",
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ))
            }
            Some(Ok(_)) => {
                for group in &self.groups {
                    let rows: Vec<_> = group
                        .rows
                        .iter()
                        .map(|&index| self.row(index, window, cx))
                        .collect();
                    let mut section = div().flex().flex_col();
                    if self.project_filter == ProjectFilter::All {
                        let label = group
                            .project
                            .map(|i| self.projects[i].label.clone())
                            .unwrap_or_else(|| "No project".into());
                        let mut header = div()
                            .h(px(46.))
                            .pb(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("folder", 16., t.text))
                            .child(text(label, 14., 21., 500., t.text).flex_1())
                            .child(text(
                                format!(
                                    "{} {}",
                                    group.rows.len(),
                                    if group.rows.len() == 1 {
                                        "chat"
                                    } else {
                                        "chats"
                                    }
                                ),
                                13.,
                                18.571_428,
                                430.,
                                t.text_secondary,
                            ));
                        if let Some(project) = group.project {
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
                    let count = rows.len();
                    section = section.child(
                        div()
                            .relative()
                            .p(px(1.))
                            .child(deco(
                                &self.layer,
                                20.,
                                Squircle,
                                t.background_panel,
                                Some(t.border),
                            ))
                            .child(crate::rounded_clip(
                                px(19.),
                                Squircle,
                                div()
                                    .flex()
                                    .flex_col()
                                    .children(rows.into_iter().enumerate().map(|(i, row)| {
                                        row.when(i + 1 < count, |row| {
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
                                    })),
                            )),
                    );
                    content = content.child(section);
                }
            }
        }
        let delete_all = has_threads.then(|| {
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
                        div().pt(px(16.)).flex().flex_col().items_center().child(
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

fn project_for(thread: &Thread, projects: &[Project], metadata: &Value) -> Option<usize> {
    if metadata["projectless-thread-ids"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&thread.id)))
    {
        return None;
    }
    if let Some(id) = metadata["thread-project-assignments"][&thread.id]["projectId"].as_str()
        && metadata["thread-project-assignments"][&thread.id]["projectKind"] == "local"
        && let Some(index) = projects
            .iter()
            .position(|project| project.id.as_ref() == id)
    {
        return Some(index);
    }
    for path in [
        metadata["thread-workspace-root-hints"][&thread.id].as_str(),
        Some(thread.cwd.as_str()),
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

fn project_groups(
    threads: &[Thread],
    projects: &[Project],
    metadata: &Value,
    query: &str,
    selection: (Kind, &ProjectFilter, Sort),
    pending: &HashSet<String>,
) -> Vec<Group> {
    let (kind, filter, sort) = selection;
    if kind == Kind::Cloud {
        return Vec::new();
    }
    let query = query.trim().to_lowercase();
    let mut groups: Vec<Group> = Vec::new();
    for (index, thread) in threads.iter().enumerate() {
        if pending.contains(&thread.id) {
            continue;
        }
        let project = project_for(thread, projects, metadata);
        let scheduled = metadata["automationThreadIds"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&thread.id)));
        let selected = match filter {
            ProjectFilter::All => true,
            ProjectFilter::Project(id) => project.is_some_and(|i| projects[i].id.as_ref() == id),
            ProjectFilter::Chats => project.is_none() && !scheduled,
            ProjectFilter::Scheduled => scheduled,
        };
        if !selected {
            continue;
        }
        let label = project.map(|i| projects[i].label.as_ref()).unwrap_or("");
        if !query.is_empty()
            && ![
                title(thread),
                label,
                &thread.cwd,
                thread.path.as_deref().unwrap_or(""),
            ]
            .iter()
            .any(|field| fuzzy_match(field, &query))
        {
            continue;
        }
        let key = if *filter == ProjectFilter::All {
            project
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
    for group in &mut groups {
        group.rows.sort_by(|&a, &b| {
            let (a, b) = (&threads[a], &threads[b]);
            match sort {
                Sort::Alphabetical => title(a)
                    .chars()
                    .flat_map(char::to_lowercase)
                    .cmp(title(b).chars().flat_map(char::to_lowercase))
                    .then(b.updated_at.cmp(&a.updated_at)),
                Sort::Created => b
                    .created_at
                    .cmp(&a.created_at)
                    .then_with(|| title(a).cmp(title(b))),
                Sort::Updated => b
                    .updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| title(a).cmp(title(b))),
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
                            threads[i].created_at
                        } else {
                            threads[i].updated_at
                        }
                    })
                    .max()
                    .unwrap_or(0)
            };
            newest(b).cmp(&newest(a)).then_with(labels)
        }
    });
    groups
}

fn fuzzy_match(field: &str, query: &str) -> bool {
    let field = field.to_lowercase();
    if field.contains(query) {
        return true;
    }
    let needle: Vec<_> = query.chars().collect();
    if needle.is_empty() {
        return true;
    }
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

fn thread_date(thread: &Thread) -> String {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_opt(thread.updated_at, 0)
        .single()
        .or_else(|| Local.timestamp_opt(thread.created_at, 0).single())
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
        assert_eq!(project_for(&thread, &projects, &metadata), Some(1));
        metadata["projectless-thread-ids"] = json!([thread.id]);
        assert_eq!(project_for(&thread, &projects, &metadata), None);
        assert_eq!(
            project_for(
                &thread,
                &[project("one", "/repo"), project("two", "/repo/")],
                &Value::Null
            ),
            None
        );
    }

    #[test]
    fn filtering_keeps_scheduled_chats_out_of_projectless_results() {
        let threads = recorded_threads();
        let metadata = json!({"automationThreadIds": [threads[0].id]});
        let pending = HashSet::from([threads[1].id.clone()]);
        let groups = project_groups(
            &threads,
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
            &Value::Null,
            "",
            (Kind::All, &ProjectFilter::All, Sort::Created),
            &HashSet::new(),
        );
        assert_eq!(groups[0].rows, vec![1, 2, 0]);
    }
}
