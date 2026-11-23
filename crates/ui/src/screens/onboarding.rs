mod first_run;

use crate::{
    SANS,
    composer::popover,
    confirmation::{Confirmation, Outcome},
    focus,
    icon::icon,
    text,
};
use gpui::{
    Anchor, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
    Welcome,
    Onboarding,
    SelectWorkspace,
    FirstRun,
}

impl Page {
    pub fn path(self) -> &'static str {
        match self {
            Self::Welcome => "/welcome",
            Self::Onboarding => "/onboarding",
            Self::SelectWorkspace => "/select-workspace",
            Self::FirstRun => "/first-run",
        }
    }
}

/// The home folders the original's Local files capability asks for, in its order.
pub const FOLDERS: [&str; 3] = ["Desktop", "Documents", "Downloads"];

/// `electron.onboarding.capabilitiesSetup.localFiles.folderDescription` for the folders that
/// still need access; no folder left names all three, as the original's `other` branch does.
fn folder_description(missing: &[&str]) -> String {
    match missing {
        [one] => format!("Work with files in {one}"),
        [one, two] => format!("Work with files in {one} and {two}"),
        _ => "Work with files in Desktop, Documents, and Downloads".into(),
    }
}

/// One capability row's status, `Sd` and `sf` in the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Available,
    SettingUp,
    Enabled,
    Incomplete,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub root: String,
    pub label: String,
}

/// One card of the starter-task step, an entry of `OnboardingStarterTasks` in the original.
#[derive(Clone, Debug, PartialEq)]
pub struct StarterTask {
    pub id: String,
    pub label: String,
    pub icon: &'static str,
    /// The turn the task runs; `None` is the native Desktop note.
    pub prompt: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub roles: Vec<String>,
    pub personalized_suggestions: bool,
    pub skipped: bool,
    pub roots: Vec<String>,
    pub task: Option<StarterTask>,
}

impl Completion {
    pub fn representative_role(&self) -> Option<&str> {
        if self.skipped {
            return None;
        }
        self.roles
            .iter()
            .find(|role| matches!(role.as_str(), "engineering" | "data_science"))
            .or_else(|| {
                self.roles
                    .iter()
                    .find(|role| role.as_str() != "default" && role.as_str() != "something_else")
            })
            .or_else(|| {
                self.roles
                    .iter()
                    .find(|role| role.as_str() == "something_else")
            })
            .map(String::as_str)
    }
    pub fn work_mode(&self) -> &'static str {
        if self
            .roles
            .iter()
            .any(|role| matches!(role.as_str(), "default" | "engineering" | "data_science"))
        {
            "coding"
        } else {
            "non_coding"
        }
    }
}

pub enum OnboardingEvent {
    Reload,
    Complete(Completion),
    PickFolder,
    CreateProject,
    OpenImport(Completion),
    FirstRunComplete,
    /// Ask the host for access to these `FOLDERS`, in order; answer with `set_local_files`.
    RequestLocalFiles(Vec<&'static str>),
    /// Fetch the starter tasks for this role; answer with `set_tasks`.
    LoadTasks(String),
}
impl EventEmitter<OnboardingEvent> for Onboarding {}

#[derive(Clone, Copy)]
enum Action {
    Continue,
    Back,
    Skip,
    Role(usize),
    Suggestions,
    AddProject,
    PickFolder,
    CreateProject,
    ToggleProject(usize),
    SelectAll,
    Import,
    NotNow,
    Reload,
    LocalFiles,
    Task(usize),
}

const ROLES: &[(&str, &str)] = &[
    ("engineering", "Engineering"),
    ("data_science", "Data science"),
    ("product_management", "Product"),
    ("design", "Design"),
    ("marketing", "Marketing"),
    ("sales", "Sales"),
    ("finance", "Finance"),
    ("operations", "Operations"),
    ("people_hr", "People & HR"),
    ("legal", "Legal"),
    ("student", "Student"),
    ("something_else", "Something else"),
];

enum State {
    Loading,
    Roles,
    Import,
    Workspace,
    Tasks,
    Capabilities,
    Error,
    FirstRun(usize),
}

pub struct Onboarding {
    page: Page,
    chatgpt: bool,
    state: State,
    roles: Vec<String>,
    suggestions: bool,
    import_available: bool,
    projects: Vec<Project>,
    roots: Vec<String>,
    add_project: bool,
    hovered_button: Option<gpui::ElementId>,
    menu_hovered: bool,
    menu_focus: [FocusHandle; 2],
    project_trigger: Option<FocusHandle>,
    backdrop: Option<first_run::Backdrop>,
    busy: bool,
    error: Option<String>,
    confirmation: Option<Entity<Confirmation>>,
    /// Which `FOLDERS` the host reported accessible.
    folders: [bool; 3],
    local_files_pending: bool,
    local_files_failed: bool,
    /// `None` while the host fetches them.
    tasks: Option<Vec<StarterTask>>,
    task: Option<StarterTask>,
}

impl Onboarding {
    pub fn new(
        page: Page,
        chatgpt: bool,
        clock: Option<crate::screens::thread::view::Clock>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            page,
            chatgpt,
            state: if page == Page::FirstRun {
                State::FirstRun(if chatgpt { 0 } else { 3 })
            } else {
                State::Loading
            },
            roles: Vec::new(),
            suggestions: true,
            import_available: false,
            projects: Vec::new(),
            roots: Vec::new(),
            add_project: false,
            hovered_button: None,
            menu_hovered: false,
            menu_focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            project_trigger: None,
            backdrop: (page == Page::FirstRun).then(|| first_run::Backdrop::new(clock)),
            busy: false,
            error: None,
            confirmation: None,
            folders: [false; 3],
            local_files_pending: false,
            local_files_failed: false,
            tasks: None,
            task: None,
        }
    }
    pub fn page(&self) -> Page {
        self.page
    }
    pub fn loading(&self) -> bool {
        matches!(self.state, State::Loading)
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn confirming_skip(&self) -> bool {
        self.confirmation.is_some()
    }
    pub fn project_menu_open(&self) -> bool {
        self.add_project
    }
    pub fn set_context(
        &mut self,
        roles: Vec<String>,
        suggestions: bool,
        import_available: bool,
        projects: Vec<Project>,
        cx: &mut Context<Self>,
    ) {
        self.roles = roles;
        self.suggestions = suggestions;
        self.import_available = import_available;
        self.projects = projects;
        self.state = if self.page == Page::SelectWorkspace {
            State::Workspace
        } else {
            State::Roles
        };
        self.error = None;
        self.busy = false;
        cx.notify();
    }
    pub fn set_projects(
        &mut self,
        projects: Vec<Project>,
        selected: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let auto_continue =
            self.page == Page::SelectWorkspace && self.projects.is_empty() && selected.is_some();
        self.projects = projects;
        if let Some(root) = selected
            && !self.roots.contains(&root)
        {
            self.roots.push(root);
        }
        self.add_project = false;
        self.busy = false;
        cx.notify();
        if auto_continue {
            self.complete(false, cx);
        }
    }
    pub fn project_created(
        &mut self,
        projects: Vec<Project>,
        root: String,
        cx: &mut Context<Self>,
    ) {
        self.roots.clear();
        self.set_projects(projects, Some(root), cx);
        self.complete(false, cx);
    }
    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = Some(error);
        if matches!(self.state, State::Loading) {
            self.state = State::Error;
        }
        cx.notify();
    }
    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        cx.notify();
    }
    /// The host's answer to `RequestLocalFiles`: one flag per requested folder, stopping at the
    /// first folder it could not open.
    pub fn set_local_files(&mut self, accessible: Vec<bool>, cx: &mut Context<Self>) {
        let missing = self.missing_folders();
        for (folder, accessible) in missing.iter().zip(&accessible) {
            if let Some(index) = FOLDERS.iter().position(|value| value == folder) {
                self.folders[index] = *accessible;
            }
        }
        self.local_files_pending = false;
        self.local_files_failed = !self.missing_folders().is_empty();
        cx.notify();
    }
    pub fn set_tasks(&mut self, tasks: Vec<StarterTask>, cx: &mut Context<Self>) {
        if !matches!(self.state, State::Tasks) {
            return;
        }
        if tasks.is_empty() {
            self.state = State::Capabilities;
        }
        self.tasks = Some(tasks);
        cx.notify();
    }
    /// The original asks for a starter task only with a ChatGPT account.
    fn enter_tasks(&mut self, cx: &mut Context<Self>) {
        if !self.chatgpt {
            self.state = State::Capabilities;
            return;
        }
        self.state = State::Tasks;
        self.tasks = None;
        self.task = None;
        let completion = self.completion(false);
        let role = completion.representative_role().unwrap_or("something_else");
        cx.emit(OnboardingEvent::LoadTasks(role.into()));
    }
    fn select_task(&mut self, index: usize) {
        self.task = self
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(index))
            .cloned();
        self.state = State::Capabilities;
    }
    fn missing_folders(&self) -> Vec<&'static str> {
        FOLDERS
            .iter()
            .zip(self.folders)
            .filter(|(_, accessible)| !accessible)
            .map(|(folder, _)| *folder)
            .collect()
    }
    fn local_files_status(&self) -> Status {
        if self.local_files_pending {
            Status::SettingUp
        } else if self.folders.iter().all(|accessible| *accessible) {
            Status::Enabled
        } else if self.local_files_failed {
            Status::Incomplete
        } else {
            Status::Available
        }
    }
    fn completion(&self, skipped: bool) -> Completion {
        Completion {
            roles: if skipped {
                vec!["something_else".into()]
            } else {
                self.roles.clone()
            },
            personalized_suggestions: !skipped && self.suggestions,
            skipped,
            roots: self.roots.clone(),
            task: if skipped { None } else { self.task.clone() },
        }
    }
    fn complete(&mut self, skipped: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.emit(OnboardingEvent::Complete(self.completion(skipped)));
        cx.notify();
    }
    fn action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match action {
            Action::Reload => {
                self.state = State::Loading;
                self.error = None;
                cx.emit(OnboardingEvent::Reload);
            }
            Action::Continue => match self.state {
                State::Roles if !self.roles.is_empty() => {
                    if self.import_available {
                        self.state = State::Import;
                    } else {
                        self.enter_tasks(cx);
                    }
                }
                State::Import | State::Capabilities => self.complete(false, cx),
                State::Workspace if !self.roots.is_empty() => self.complete(false, cx),
                State::FirstRun(index) if index < 3 => self.state = State::FirstRun(index + 1),
                State::FirstRun(_) => {
                    self.busy = true;
                    cx.emit(OnboardingEvent::FirstRunComplete);
                }
                _ => {}
            },
            Action::Back => {
                if let State::FirstRun(index) = self.state {
                    self.state = State::FirstRun(index.saturating_sub(1));
                }
            }
            Action::Role(index) => {
                let role = ROLES[index].0;
                if let Some(index) = self.roles.iter().position(|value| value == role) {
                    self.roles.remove(index);
                } else {
                    self.roles.push(role.into());
                }
            }
            Action::Suggestions => self.suggestions = !self.suggestions,
            Action::Skip => {
                // The original skips this step without asking and keeps the chosen roles
                // and task (`capabilitiesSkipped`).
                if matches!(self.state, State::Capabilities | State::Tasks) {
                    self.complete(false, cx);
                } else if self.page == Page::Welcome {
                    self.complete(true, cx);
                } else {
                    let dialog = cx.new(|cx| {
                        Confirmation::new(
                            "Skip setup?",
                            "You’ll go straight to ChatGPT",
                            "Skip",
                            window,
                            cx,
                        )
                        .cancel_label("Keep setting up")
                    });
                    cx.subscribe(&dialog, |this, _, outcome: &Outcome, cx| {
                        this.confirmation = None;
                        if *outcome == Outcome::Confirm {
                            this.complete(true, cx);
                        }
                        cx.notify();
                    })
                    .detach();
                    self.confirmation = Some(dialog);
                }
            }
            Action::AddProject => {
                self.add_project = !self.add_project;
                if self.add_project {
                    self.menu_focus[0].focus(window, cx);
                } else {
                    self.close_project_menu(window, cx);
                }
            }
            Action::PickFolder => {
                self.close_project_menu(window, cx);
                cx.emit(OnboardingEvent::PickFolder);
            }
            Action::CreateProject => {
                self.close_project_menu(window, cx);
                self.busy = true;
                cx.emit(OnboardingEvent::CreateProject);
            }
            Action::ToggleProject(index) => {
                let root = &self.projects[index].root;
                if let Some(index) = self.roots.iter().position(|value| value == root) {
                    self.roots.remove(index);
                } else {
                    self.roots.push(root.clone());
                }
            }
            Action::SelectAll => {
                if self.roots.len() == self.projects.len() {
                    self.roots.clear();
                } else {
                    self.roots = self
                        .projects
                        .iter()
                        .map(|project| project.root.clone())
                        .collect();
                }
            }
            Action::Import => {
                self.busy = true;
                self.error = None;
                cx.emit(OnboardingEvent::OpenImport(self.completion(false)));
            }
            Action::NotNow => self.enter_tasks(cx),
            Action::Task(index) => self.select_task(index),
            Action::LocalFiles => {
                if matches!(
                    self.local_files_status(),
                    Status::Available | Status::Incomplete
                ) {
                    self.local_files_pending = true;
                    cx.emit(OnboardingEvent::RequestLocalFiles(self.missing_folders()));
                }
            }
        }
        cx.notify();
    }
    fn button(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        action: Action,
        primary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme();
        let workspace = self.page == Page::SelectWorkspace;
        let first_run = self.page == Page::FirstRun;
        let pill = workspace || first_run;
        let link = workspace && matches!(action, Action::Skip | Action::CreateProject);
        let add_empty =
            workspace && self.projects.is_empty() && matches!(action, Action::AddProject);
        let id = id.into();
        let hovered = self.hovered_button.as_ref() == Some(&id)
            || (workspace && self.add_project && matches!(action, Action::AddProject));
        let disabled = self.busy
            || (matches!(action, Action::Back) && matches!(self.state, State::FirstRun(0)))
            || (matches!(action, Action::Continue)
                && match self.state {
                    State::Roles => self.roles.is_empty(),
                    State::Workspace => self.roots.is_empty(),
                    State::Loading | State::Error => true,
                    State::Import | State::Tasks | State::Capabilities | State::FirstRun(_) => {
                        false
                    }
                });
        let border = t.border;
        let (background, foreground) = if primary {
            (
                if hovered {
                    t.text.opacity(0.8)
                } else {
                    t.button_primary_solid
                },
                t.surface_elevated_secondary,
            )
        } else if link {
            (
                if hovered {
                    t.ghost_hover
                } else {
                    gpui::transparent_black()
                },
                t.text.opacity(0.498),
            )
        } else if first_run {
            (
                if hovered {
                    t.ghost_hover
                } else {
                    t.text.opacity(0.03)
                },
                t.text,
            )
        } else {
            (
                if workspace {
                    t.surface_secondary
                } else {
                    t.surface
                },
                t.text,
            )
        };
        let label = label.into();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let hover_id = id.clone();
        let control = div()
            .id(id)
            .relative()
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .aria_disabled(disabled)
            .h(px(if add_empty {
                40.
            } else if pill {
                36.
            } else {
                44.
            }))
            .px(px(if first_run { 21. } else { 16. }))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .text_color(foreground)
            .cursor_pointer()
            .when(pill, |button| {
                button
                    .child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(Theme::RADIUS_FULL),
                        corner: shapes::Corner::Round,
                        fill: (!link || hovered).then_some(background),
                        border: (!link).then_some((px(1.), border)),
                        ..Default::default()
                    }))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hovered_button = hovered.then(|| hover_id.clone());
                        cx.notify();
                    }))
            })
            .when(!pill, |button| {
                button
                    .bg(background)
                    .when(!primary, |button| button.border_1().border_color(border))
            })
            .when(disabled, |button| {
                button.opacity(if pill { 0.4 } else { 0.5 })
            })
            .on_click(cx.listener(move |this, _, window, cx| this.action(action, window, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.action(action, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(text(
                label,
                if add_empty { 13. } else { 14. },
                if pill { 18. } else { 20. },
                if add_empty || first_run {
                    430.
                } else if matches!(action, Action::Role(_)) {
                    400.
                } else {
                    500.
                },
                foreground,
            ));
        if disabled {
            control
        } else {
            focus::ring(control, &handle, Theme::RADIUS_FULL, px(2.), cx)
        }
    }
    fn role_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let columns = if f32::from(window.viewport_size().width) < 540. {
            2
        } else {
            3
        };
        let mut grid = div().w_full().flex().flex_col().gap(px(8.));
        for (row, roles) in ROLES.chunks(columns).enumerate() {
            let mut line = div().w_full().flex().gap(px(8.));
            for (col, (_, label)) in roles.iter().enumerate() {
                let index = row * columns + col;
                let selected = self.roles.iter().any(|role| role == ROLES[index].0);
                let mut button = self
                    .button(
                        ("onboarding-role", index),
                        *label,
                        Action::Role(index),
                        false,
                        window,
                        cx,
                    )
                    .flex_1()
                    .rounded(px(15.))
                    .h(px(40.))
                    .aria_toggled(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    });
                if selected {
                    button = button
                        .bg(t.text.opacity(0.06))
                        .border_color(t.text.opacity(0.2))
                        .child(div().absolute().left(px(8.)).top(px(12.)).child(icon(
                            "menu-check",
                            16.,
                            t.text,
                        )));
                }
                line = line.child(button);
            }
            grid = grid.child(line);
        }
        div()
            .w(px(560.))
            .max_w_full()
            .p(px(40.))
            .flex()
            .flex_col()
            .items_center()
            .child(
                text("What type of work do you do?", 28., 34., 400., t.text)
                    .whitespace_normal()
                    .text_center(),
            )
            .child(
                text(
                    "Customize ChatGPT to fit the way you work",
                    16.,
                    24.,
                    400.,
                    t.text_description,
                )
                .mt(px(8.))
                .whitespace_normal()
                .text_center(),
            )
            .child(grid.mt(px(32.)))
            .child(
                self.button(
                    "personalized-suggestions",
                    "Suggest personalized tasks",
                    Action::Suggestions,
                    false,
                    window,
                    cx,
                )
                .mt(px(28.))
                .aria_toggled(if self.suggestions {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .child(icon(
                    if self.suggestions {
                        "menu-check"
                    } else {
                        "plus"
                    },
                    14.,
                    t.text,
                )),
            )
            .child(
                self.button(
                    "onboarding-continue",
                    "Continue",
                    Action::Continue,
                    true,
                    window,
                    cx,
                )
                .mt(px(32.))
                .w(px(320.))
                .max_w_full()
                .when(self.roles.is_empty(), |button| button.opacity(0.3)),
            )
            .child(
                self.button("onboarding-skip", "Skip", Action::Skip, false, window, cx)
                    .mt(px(12.)),
            )
    }
    /// The original's starter-task step: its introduction, then a grid of task cards.
    fn tasks_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let width = f32::from(window.viewport_size().width);
        let columns = if width < 640. {
            1
        } else if width < 1024. {
            2
        } else {
            4
        };
        let tasks = self.tasks.as_deref().unwrap_or_default();
        // The original's `loading-block` while the tasks load.
        let mut grid = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .when(self.tasks.is_none(), |grid| {
                grid.child(div().h(px(120.)).rounded(px(16.)).bg(t.surface_secondary))
            });
        for (row, chunk) in tasks.chunks(columns).enumerate() {
            let mut line = div().w_full().flex().gap(px(12.));
            for (col, task) in chunk.iter().enumerate() {
                let index = row * columns + col;
                let id = gpui::ElementId::from(("onboarding-task", index));
                let hovered = self.hovered_button.as_ref() == Some(&id);
                let hover_id = id.clone();
                let handle = focus::handle(id.clone(), 0, window, cx);
                let card = div()
                    .id(id)
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(task.label.clone()))
                    .flex_1()
                    .min_w_0()
                    .h(px(120.))
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(16.))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(t.border)
                    .bg(if hovered { t.ghost_hover } else { t.surface })
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hovered_button = hovered.then(|| hover_id.clone());
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.action(Action::Task(index), window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.action(Action::Task(index), window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(icon(task.icon, 24., t.text))
                    .child(
                        text(task.label.clone(), 18., 24., 400., t.text)
                            .w_full()
                            .whitespace_normal()
                            .line_clamp(2),
                    );
                line = line.child(focus::ring(card, &handle, px(16.), px(2.), cx));
            }
            for _ in chunk.len()..columns {
                line = line.child(div().flex_1());
            }
            grid = grid.child(line);
        }
        div()
            .w(px(820.))
            .max_w_full()
            .p(px(40.))
            .flex()
            .flex_col()
            .child(
                text(
                    "Here in the desktop app, I can work directly with your computer’s files, folders, and apps you choose.",
                    18.,
                    24.,
                    400.,
                    t.text,
                )
                .whitespace_normal(),
            )
            .child(
                text(
                    "Start with a simple task to see how it works:",
                    18.,
                    24.,
                    400.,
                    t.text,
                )
                .mt(px(24.)),
            )
            .child(grid.mt(px(24.)))
            .child(
                div().mt(px(40.)).flex().justify_center().child(
                    self.button(
                        "onboarding-tasks-skip",
                        "Skip",
                        Action::Skip,
                        false,
                        window,
                        cx,
                    )
                    .w(px(320.))
                    .max_w_full(),
                ),
            )
    }
    /// `_d` and `vd` in the original: the introduction, then a card of capability rows.
    fn capabilities_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let status = self.local_files_status();
        let detail = match status {
            Status::SettingUp => "Setting up…".into(),
            _ => folder_description(&self.missing_folders()),
        };
        let on = status == Status::Enabled;
        let disabled = matches!(status, Status::SettingUp | Status::Enabled);
        let handle = focus::handle("onboarding-local-files", 0, window, cx);
        let toggle = focus::ring(
            crate::screens::settings::switch("onboarding-local-files", on, disabled, &t)
                .relative()
                .role(gpui::Role::Switch)
                .aria_label("Local files")
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .aria_disabled(disabled)
                .on_click(
                    cx.listener(|this, _, window, cx| this.action(Action::LocalFiles, window, cx)),
                )
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.action(Action::LocalFiles, window, cx);
                        cx.stop_propagation();
                    }
                })),
            &handle,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        );
        let row = div()
            .min_w_0()
            .px(px(16.))
            .py(px(6.))
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .size(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("folder", 24., t.text)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(text("Local files", 18., 24., 400., t.text))
                    .child(
                        div()
                            .id("onboarding-local-files-status")
                            .role(gpui::Role::Status)
                            .child(
                                text(detail, 16., 20., 400., t.text_tertiary).whitespace_normal(),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(8.))
                    .child(toggle)
                    .when(status == Status::Incomplete, |column| {
                        column.child(
                            self.button(
                                "onboarding-local-files-retry",
                                "Retry",
                                Action::LocalFiles,
                                false,
                                window,
                                cx,
                            )
                            .h(px(28.))
                            .px(px(10.)),
                        )
                    }),
            );
        let card = div()
            .w_full()
            .overflow_hidden()
            .rounded(px(24.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(text(
                        "Give ChatGPT access to your computer",
                        16.,
                        20.,
                        500.,
                        t.text,
                    )),
            )
            .child(div().py(px(10.)).flex().flex_col().child(row));
        div()
            .w(px(616.))
            .max_w_full()
            .p(px(40.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                text(
                    "Let ChatGPT use your apps, browser, files, and work history to understand what you’re working on and help you get things done with less explaining",
                    18.,
                    28.,
                    400.,
                    t.text,
                )
                .whitespace_normal(),
            )
            .child(card)
            .child(
                div()
                    .mt(px(16.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.button(
                        "onboarding-capabilities-skip",
                        "Skip",
                        Action::Skip,
                        false,
                        window,
                        cx,
                    ))
                    .child(self.button(
                        "onboarding-capabilities-continue",
                        "Continue",
                        Action::Continue,
                        true,
                        window,
                        cx,
                    )),
            )
    }
    fn close_project_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_project = false;
        self.menu_hovered = false;
        if let Some(trigger) = &self.project_trigger {
            trigger.focus(window, cx);
        }
    }

    fn project_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = cx.theme().clone();
        let rows = [
            (
                "onboarding-new-project",
                "Start from scratch",
                "onboarding-project-new",
                Action::CreateProject,
            ),
            (
                "onboarding-existing-project",
                "Use an existing folder",
                "onboarding-project-folder",
                Action::PickFolder,
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (id, label, asset, action))| {
            let active = self.hovered_button.as_ref() == Some(&id.into())
                || (self.menu_focus[index].is_focused(window) && window.last_input_was_keyboard());
            div()
                .id(id)
                .role(gpui::Role::MenuItem)
                .aria_label(label)
                .track_focus(&self.menu_focus[index])
                .px(px(8.))
                .py(px(5.))
                .rounded(px(15.))
                .flex()
                .items_center()
                .gap(px(6.))
                .when(active, |row| row.bg(t.menu_item_hover))
                .child(
                    div()
                        .size(px(16.))
                        .opacity(if active { 1. } else { 0.75 })
                        .child(icon(asset, 16., t.text)),
                )
                .child(text(label, 13., 18.5714, 430., t.text))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.hovered_button = hovered.then(|| id.into());
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.action(action, window, cx)))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.action(action, window, cx);
                        cx.stop_propagation();
                    }
                }))
        });
        div()
            .id("onboarding-project-menu")
            .role(gpui::Role::Menu)
            .on_hover(cx.listener(|this, hovered: &bool, _, _| this.menu_hovered = *hovered))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.close_project_menu(window, cx),
                    "up" | "down" | "tab" => {
                        let next = usize::from(this.menu_focus[0].is_focused(window));
                        this.menu_focus[next].focus(window, cx);
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(popover(&t, 220., |menu| menu.children(rows)))
    }

    fn workspace_choice(
        &self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let (id, label, description, selected, partial, action) = match index {
            Some(index) => {
                let project = &self.projects[index];
                (
                    ("workspace-choice", index + 1),
                    project.label.clone(),
                    Some(project.root.clone()),
                    self.roots.contains(&project.root),
                    false,
                    Action::ToggleProject(index),
                )
            }
            None => (
                ("workspace-choice", 0),
                "Select all".to_owned(),
                None,
                self.roots.len() == self.projects.len(),
                !self.roots.is_empty() && self.roots.len() < self.projects.len(),
                Action::SelectAll,
            ),
        };
        let handle = focus::handle(id, 0, window, cx);
        let checkbox = div()
            .size(px(18.))
            .flex_none()
            .rounded(px(4.))
            .border_1()
            .border_color(if selected || partial {
                t.text
            } else {
                t.border_strong
            })
            .when(selected || partial, |checkbox| checkbox.bg(t.text))
            .flex()
            .items_center()
            .justify_center()
            .when(selected && !partial, |checkbox| {
                checkbox.child(icon("thread-check", 12., t.surface_secondary))
            })
            .when(partial, |checkbox| {
                checkbox.child(div().w(px(8.)).h(px(1.5)).bg(t.surface_secondary))
            });
        let label_column = div()
            .min_w_0()
            .flex()
            .flex_col()
            .child(text(label.clone(), 14., 20., 430., t.text))
            .children(description.map(|description| {
                text(description, 12., 16., 430., t.text.opacity(0.498)).truncate()
            }));
        let control = div()
            .id(id)
            .role(gpui::Role::CheckBox)
            .aria_label(label)
            .aria_disabled(self.busy)
            .aria_toggled(if partial {
                gpui::Toggled::Mixed
            } else if selected {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .w_full()
            .py(px(8.))
            .flex()
            .items_start()
            .gap(px(8.))
            .child(checkbox)
            .child(label_column)
            .on_click(cx.listener(move |this, _, window, cx| this.action(action, window, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.action(action, window, cx);
                    cx.stop_propagation();
                }
            }));
        focus::ring(control, &handle, px(4.), px(2.), cx)
    }

    fn workspace_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let loading = matches!(self.state, State::Loading);
        let empty = self.projects.is_empty() && !loading;
        self.project_trigger = Some(
            focus::handle("onboarding-add-project", 0, window, cx)
                .read(cx)
                .handle
                .clone(),
        );
        let mut add_project = div()
            .id("workspace-project-trigger")
            .relative()
            .w_full()
            .child(
                self.button(
                    "onboarding-add-project",
                    "Add project",
                    Action::AddProject,
                    empty,
                    window,
                    cx,
                )
                .w_full(),
            )
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if this.add_project && !this.menu_hovered {
                    this.close_project_menu(window, cx);
                    cx.notify();
                }
            }));
        if self.add_project {
            add_project = add_project.child(deferred(
                div().absolute().top_0().left_0().child(
                    anchored()
                        .anchor(Anchor::TopLeft)
                        .offset(point(px(1.), px(if empty { 42. } else { 38. })))
                        .child(self.project_menu(window, cx)),
                ),
            ));
        }
        let mut content = div().w_full().max_w(px(330.)).flex().flex_col().items_center().gap(px(24.))
            .when(empty, |column| column.child(icon("onboarding-workspace", 40., t.text)))
            .child(div().w_full().flex().flex_col().items_center().gap(px(24.))
                .child(text("Select a project", 24., 36., 600., t.text).text_center())
                .child(text("ChatGPT will be able to edit files and run commands in selected folders", 16., 24., 430., t.text.opacity(0.498)).w_full().whitespace_normal().text_center()));
        let mut choices = div().w_full().flex().flex_col().gap(px(16.));
        if loading {
            choices = choices.child(text(
                "Loading projects…",
                13.,
                18.5714,
                430.,
                t.text.opacity(0.498),
            ));
        } else if !empty {
            let mut list = div()
                .id("workspace-project-options")
                .w_full()
                .h(px(240.))
                .px(px(20.))
                .py(px(16.))
                .border_1()
                .border_color(t.border)
                .rounded(px(20.))
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .child(self.workspace_choice(None, window, cx));
            for index in 0..self.projects.len() {
                list = list.child(self.workspace_choice(Some(index), window, cx));
            }
            choices = choices.child(list);
        }
        let mut actions = div().w_full().flex().flex_col().gap(px(12.));
        if empty {
            actions = actions.child(add_project);
        } else {
            actions = actions.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(add_project.flex_1())
                    .child(
                        self.button(
                            "workspace-continue",
                            "Continue",
                            Action::Continue,
                            true,
                            window,
                            cx,
                        )
                        .flex_1()
                        .aria_disabled(self.busy || loading || self.roots.is_empty())
                        .when(loading || self.roots.is_empty(), |button| {
                            button.opacity(0.4)
                        }),
                    ),
            );
        }
        let skip = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .children(self.error.clone().map(|error| {
                text(error, 13., 18.5714, 430., t.text_error)
                    .whitespace_normal()
                    .text_center()
            }))
            .child(
                self.button(
                    "workspace-skip",
                    if self.busy {
                        "Creating a new project…"
                    } else {
                        "Skip"
                    },
                    Action::CreateProject,
                    false,
                    window,
                    cx,
                )
                .w_full(),
            );
        choices = choices.child(actions.child(skip));
        content = content.child(choices);
        div()
            .size_full()
            .pt(px(44.))
            .pb(px(32.))
            .px(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .child(content)
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body=match self.state {
            State::Loading if self.page == Page::SelectWorkspace => self.workspace_view(window,cx),
            State::Loading=>div().child(icon("signin-chatgpt",40.,t.text)),
            State::Error=>div().flex().flex_col().items_center().gap(px(16.))
                .child(text("Couldn’t load setup. Please reload the app",14.,20.,400.,t.text_description))
                .child(self.button("onboarding-reload","Reload app",Action::Reload,false,window,cx)),
            State::Roles=>self.role_view(window,cx),
            State::Workspace=>self.workspace_view(window,cx),
            State::FirstRun(index)=>self.first_run_view(index,window,cx),
            State::Tasks=>self.tasks_view(window,cx),
            State::Capabilities=>self.capabilities_view(window,cx),
            State::Import=>div().w(px(640.)).max_w_full().p(px(40.)).flex().flex_col().gap(px(24.))
                .child(text("Import work from other AI apps",28.,34.,400.,t.text))
                .child(text("Before we get started, would you like to bring over your work from other AI apps? You can review your setup, projects, and chats before importing.",18.,28.,400.,t.text).whitespace_normal())
                .child(self.button("onboarding-import","Bring over my setup",Action::Import,true,window,cx))
                .child(self.button("onboarding-not-now","Not now",Action::NotNow,false,window,cx)),
        };
        div()
            .size_full()
            .relative()
            .when(self.page != Page::FirstRun, |page| {
                page.bg(if self.page == Page::SelectWorkspace {
                    t.surface_secondary
                } else {
                    t.surface
                })
            })
            .when(self.page == Page::FirstRun, |page| page.pt(px(46.)))
            .font_family(SANS)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .on_key_down(focus::tab)
            .child(body)
            .when(self.page != Page::SelectWorkspace, |page| {
                page.children(self.error.clone().map(|error| {
                    text(error, 14., 20., 400., t.text_error)
                        .max_w(px(560.))
                        .whitespace_normal()
                }))
            })
            .children(self.confirmation.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn role_priority_and_skip_match_completion_contract() {
        let mut value = Completion {
            roles: vec!["marketing".into(), "engineering".into()],
            personalized_suggestions: true,
            skipped: false,
            roots: Vec::new(),
            task: None,
        };
        assert_eq!(value.representative_role(), Some("engineering"));
        assert_eq!(value.work_mode(), "coding");
        value.skipped = true;
        assert_eq!(value.representative_role(), None);
        value.roles = vec!["marketing".into()];
        assert_eq!(value.work_mode(), "non_coding");
    }

    #[test]
    fn a_chosen_starter_task_reaches_completion_after_capabilities() {
        let task = StarterTask {
            id: "desktop_note".into(),
            label: "Leave a note on my Desktop".into(),
            icon: "folder",
            prompt: None,
        };
        let mut app = gpui::HeadlessAppContext::new(textsys::text_system());
        let view = app.new(|cx| Onboarding::new(Page::Onboarding, true, None, cx));
        view.update(&mut app, |view, cx| {
            view.roles = vec!["marketing".into()];
            view.enter_tasks(cx);
            assert!(matches!(view.state, State::Tasks));
            view.set_tasks(vec![task.clone()], cx);
            view.select_task(0);
            assert!(matches!(view.state, State::Capabilities));
            assert_eq!(view.completion(false).task, Some(task));
            assert_eq!(view.completion(true).task, None);
            // No tasks: the step falls through to the capabilities step.
            view.enter_tasks(cx);
            view.set_tasks(Vec::new(), cx);
            assert!(matches!(view.state, State::Capabilities));
        });
        // Without a ChatGPT account the original has no starter-task step.
        let view = app.new(|cx| Onboarding::new(Page::Onboarding, false, None, cx));
        view.update(&mut app, |view, cx| {
            view.enter_tasks(cx);
            assert!(matches!(view.state, State::Capabilities));
        });
    }

    #[test]
    fn local_files_asks_only_for_folders_still_missing() {
        assert_eq!(
            folder_description(&["Desktop", "Downloads"]),
            "Work with files in Desktop and Downloads"
        );
        assert_eq!(
            folder_description(&[]),
            "Work with files in Desktop, Documents, and Downloads"
        );
        let mut app = gpui::HeadlessAppContext::new(textsys::text_system());
        let view = app.new(|cx| Onboarding::new(Page::Onboarding, true, None, cx));
        view.update(&mut app, |view, cx| {
            assert_eq!(view.local_files_status(), Status::Available);
            // The host stops at the first folder it cannot open.
            view.set_local_files(vec![true, false], cx);
            assert_eq!(view.local_files_status(), Status::Incomplete);
            assert_eq!(view.missing_folders(), ["Documents", "Downloads"]);
            view.set_local_files(vec![true, true], cx);
            assert_eq!(view.local_files_status(), Status::Enabled);
        });
    }

    #[test]
    fn a_created_project_replaces_the_previous_selection() {
        let mut app = gpui::HeadlessAppContext::new(textsys::text_system());
        let view = app.new(|cx| Onboarding::new(Page::SelectWorkspace, false, None, cx));
        view.update(&mut app, |view, cx| {
            view.set_context(
                Vec::new(),
                true,
                false,
                vec![Project {
                    root: "/existing".into(),
                    label: "Existing".into(),
                }],
                cx,
            );
            view.roots.push("/existing".into());
            view.project_created(
                vec![
                    Project {
                        root: "/existing".into(),
                        label: "Existing".into(),
                    },
                    Project {
                        root: "/new".into(),
                        label: "New".into(),
                    },
                ],
                "/new".into(),
                cx,
            );
            assert_eq!(view.completion(false).roots, ["/new"]);
        });
    }
}
