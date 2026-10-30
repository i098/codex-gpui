mod first_run;

use crate::{
    SANS,
    composer::popover,
    confirmation::{Confirmation, Outcome},
    focus,
    icon::icon,
    screens::computer_use::{ComputerUseAction, ComputerUseState, Load},
    text,
};
use gpui::{
    Anchor, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

#[derive(Clone, Debug)]
pub struct Project {
    pub root: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub roles: Vec<String>,
    pub personalized_suggestions: bool,
    pub skipped: bool,
    pub roots: Vec<String>,
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
    /// Entering the capabilities step: check Computer use and memories.
    ReadCapabilities,
    /// Computer history: turn on memories (`setMemoryFeatureEnabled` + `writeMemoryConfig`).
    EnableHistory,
    /// Entering the connectors step, or `Check connection`: read `app/list`.
    ReadApps,
    /// `Connect` on an `app/list` entry: open its browser setup.
    ConnectApp(Value),
    /// A starter card: complete setup, then start a chat with the card's prompt.
    StartTask(Completion, &'static str),
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
    SetUpComputerUse,
    SetUpHistory,
    ReloadApps,
    Connect(usize),
    Starter(usize),
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

/// `chatgpt.tpp.onboarding.starter.{personal_website,manage_inbox,personalized_presentation}`
/// and `skill_fallback.repeatable_work` cards: label, description, prompt.
const STARTERS: &[(&str, &str, &str)] = &[
    (
        "Build a personal website",
        "Create a personalized website based on what you have shared",
        "Based on everything you know about me and what we’ve discussed in past conversations, choose the most useful subject for a personal website. If there isn’t enough context to choose honestly, ask me one focused question about what the site should be for. Then design, build, and host it for me. Make confident choices about the content, structure, and visual style you think I’ll like best, and start creating it now.",
    ),
    (
        "Help manage my inbox",
        "Learn your writing style and help draft replies to unread emails",
        "Use my connected email app. If I haven’t connected one, ask which email provider I use and help me connect it before continuing. Then review the emails I’ve sent over the last three months and learn how my writing changes for different people and situations. Create a reusable email-writing skill that captures those styles, patterns, and preferences. Set up a daily automation that reviews my unread emails and drafts a response to each one in the appropriate style. Never send an email without my explicit confirmation.",
    ),
    (
        "Make my work repeatable",
        "Turn recurring tasks into skills you can run",
        "Find a workflow I repeat and turn it into a reusable skill",
    ),
    (
        "Make a presentation for me",
        "Create a polished slide presentation based on your past conversations",
        "Review our past conversations and choose the topic that would be most useful to turn into a polished slide presentation for me right now. If there isn’t enough context to choose honestly, ask me one focused question about the subject. Then pull together the relevant context, decide on a strong story and structure, write the content, and create the complete presentation with useful visuals. Make confident choices and start creating it now. When it’s ready, give me the finished file and a brief note explaining what you chose and why.",
    ),
];

/// `app/list` entries shown on the connectors step.
pub const CONNECTOR_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Loading,
    Roles,
    Import,
    Workspace,
    Error,
    FirstRun(usize),
    Capabilities,
    Connectors,
    Starter,
}

/// The setup step after `state`: Import when detected, then the original's capabilities,
/// connectors and starter-task steps; `None` completes setup.
fn next_step(state: State, import_available: bool) -> Option<State> {
    match state {
        State::Roles if import_available => Some(State::Import),
        State::Roles | State::Import => Some(State::Capabilities),
        State::Capabilities => Some(State::Connectors),
        State::Connectors => Some(State::Starter),
        _ => None,
    }
}

fn previous_step(state: State, import_available: bool) -> Option<State> {
    match state {
        State::Capabilities if import_available => Some(State::Import),
        State::Capabilities => Some(State::Roles),
        State::Connectors => Some(State::Capabilities),
        State::Starter => Some(State::Connectors),
        _ => None,
    }
}

/// A capability row's status (`electron.onboarding.capabilitiesSetup.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    Checking,
    Ready,
    SettingUp,
    Incomplete,
    Done,
    Unavailable,
}

/// The Computer use setup this page started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComputerUseStep {
    Install,
    Enable,
    Failed,
}

/// The Computer use row from the shared Computer Use state and the setup this page started:
/// the row status, the setup still running, and the action to send next.
fn computer_use_setup(
    state: &ComputerUseState,
    step: Option<ComputerUseStep>,
) -> (Setup, Option<ComputerUseStep>, Option<ComputerUseAction>) {
    if !state.available || matches!(state.plugin, Load::Ready(None)) {
        return (Setup::Unavailable, None, None);
    }
    let installed = match &state.plugin {
        Load::Ready(Some(plugin)) if plugin.enabled && !state.pending => {
            return (Setup::Done, None, None);
        }
        Load::Ready(Some(plugin)) => plugin.installed,
        _ => false,
    };
    if step == Some(ComputerUseStep::Failed) {
        return (Setup::Incomplete, step, None);
    }
    if state.pending || matches!(state.plugin, Load::Loading) {
        let setup = if step.is_some() {
            Setup::SettingUp
        } else {
            Setup::Checking
        };
        return (setup, step, None);
    }
    match step {
        // Install reloaded the plugin; turn it on next, as the Settings toggle does.
        Some(ComputerUseStep::Install) if installed && state.error.is_none() => (
            Setup::SettingUp,
            Some(ComputerUseStep::Enable),
            Some(ComputerUseAction::SetEnabled(true)),
        ),
        Some(_) => (Setup::Incomplete, Some(ComputerUseStep::Failed), None),
        None if matches!(state.plugin, Load::Failed(_)) => (Setup::Incomplete, None, None),
        None => (Setup::Ready, None, None),
    }
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
    computer_use: Option<ComputerUseStep>,
    history: Setup,
    apps: Option<Result<Vec<Value>, String>>,
    /// App ids whose browser setup is open; their button checks the connection.
    connecting: Vec<String>,
}

impl Onboarding {
    pub fn new(
        page: Page,
        chatgpt: bool,
        clock: Option<crate::screens::thread::view::Clock>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_global::<ComputerUseState>(|this, cx| this.sync_computer_use(cx))
            .detach();
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
            computer_use: None,
            history: Setup::Checking,
            apps: None,
            connecting: Vec::new(),
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
    pub fn set_history(&mut self, setup: Setup, cx: &mut Context<Self>) {
        self.history = setup;
        cx.notify();
    }
    /// `app/list` result for the connectors step, already trimmed to [`CONNECTOR_LIMIT`].
    pub fn set_apps(&mut self, apps: Result<Vec<Value>, String>, cx: &mut Context<Self>) {
        self.apps = Some(apps);
        cx.notify();
    }
    /// The browser setup for app `id` opened.
    pub fn app_opened(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.connecting.contains(&id) {
            self.connecting.push(id);
        }
        cx.notify();
    }
    /// Follows the shared Computer Use state; sends the next setup action this page owes.
    fn sync_computer_use(&mut self, cx: &mut Context<Self>) {
        let Some(state) = cx.try_global::<ComputerUseState>() else {
            return;
        };
        let (_, step, action) = computer_use_setup(state, self.computer_use);
        let dispatch = state.dispatch.clone();
        self.computer_use = step;
        if let (Some(action), Some(dispatch)) = (action, dispatch) {
            dispatch(action, cx);
        }
        cx.notify();
    }
    fn advance(&mut self, cx: &mut Context<Self>) {
        let Some(next) = next_step(self.state, self.import_available) else {
            self.complete(false, cx);
            return;
        };
        self.state = next;
        if next == State::Capabilities && self.history == Setup::Checking {
            cx.emit(OnboardingEvent::ReadCapabilities);
        }
        if next == State::Connectors && self.apps.is_none() {
            cx.emit(OnboardingEvent::ReadApps);
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
                State::Roles if self.roles.is_empty() => {}
                State::Roles
                | State::Import
                | State::Capabilities
                | State::Connectors
                | State::Starter => self.advance(cx),
                State::Workspace if !self.roots.is_empty() => self.complete(false, cx),
                State::FirstRun(index) if index < 3 => self.state = State::FirstRun(index + 1),
                State::FirstRun(_) => {
                    self.busy = true;
                    cx.emit(OnboardingEvent::FirstRunComplete);
                }
                _ => {}
            },
            Action::Back => match self.state {
                State::FirstRun(index) => self.state = State::FirstRun(index.saturating_sub(1)),
                state => {
                    if let Some(previous) = previous_step(state, self.import_available) {
                        self.state = previous;
                    }
                }
            },
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
                if self.page == Page::Welcome {
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
            Action::NotNow => self.advance(cx),
            Action::SetUpComputerUse => {
                let Some(state) = cx.try_global::<ComputerUseState>() else {
                    return;
                };
                if state.pending {
                    return;
                }
                let installed =
                    matches!(&state.plugin, Load::Ready(Some(plugin)) if plugin.installed);
                let dispatch = state.dispatch.clone();
                let (step, action) = if installed {
                    (ComputerUseStep::Enable, ComputerUseAction::SetEnabled(true))
                } else {
                    (ComputerUseStep::Install, ComputerUseAction::Install)
                };
                self.computer_use = Some(step);
                if let Some(dispatch) = dispatch {
                    dispatch(action, cx);
                }
            }
            Action::SetUpHistory => {
                self.history = Setup::SettingUp;
                cx.emit(OnboardingEvent::EnableHistory);
            }
            Action::ReloadApps => {
                self.apps = None;
                cx.emit(OnboardingEvent::ReadApps);
            }
            Action::Connect(index) => {
                let Some(Ok(apps)) = &self.apps else {
                    return;
                };
                let app = apps[index].clone();
                if self
                    .connecting
                    .iter()
                    .any(|id| app["id"].as_str() == Some(id.as_str()))
                {
                    cx.emit(OnboardingEvent::ReadApps);
                } else {
                    cx.emit(OnboardingEvent::ConnectApp(app));
                }
            }
            Action::Starter(index) => {
                self.busy = true;
                self.error = None;
                cx.emit(OnboardingEvent::StartTask(
                    self.completion(false),
                    STARTERS[index].2,
                ));
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
                    State::Import
                    | State::FirstRun(_)
                    | State::Capabilities
                    | State::Connectors
                    | State::Starter => false,
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
    /// Frame of the steps after Roles. The forward button keeps the `onboarding-continue` id,
    /// so keyboard focus stays on it from step to step.
    fn step_view(
        &self,
        title: &'static str,
        introduction: Option<&'static str>,
        body: Div,
        forward: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        div()
            .w(px(640.))
            .max_w_full()
            .p(px(40.))
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(text(title, 28., 34., 400., t.text).whitespace_normal())
            .children(
                introduction.map(|introduction| {
                    text(introduction, 18., 28., 400., t.text).whitespace_normal()
                }),
            )
            .child(body)
            .child(self.button(
                "onboarding-continue",
                forward,
                Action::Continue,
                true,
                window,
                cx,
            ))
            .child(self.button("onboarding-back", "Back", Action::Back, false, window, cx))
    }
    /// One `capabilitiesSetup` row: the description, or the status while not set up.
    fn capability(
        &self,
        label: &'static str,
        description: &'static str,
        setup: Setup,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let status = match setup {
            Setup::Checking => "Checking…",
            Setup::SettingUp => "Setting up…",
            Setup::Incomplete => "Setup incomplete",
            Setup::Unavailable => "Unavailable",
            Setup::Ready | Setup::Done => description,
        };
        let control = match setup {
            Setup::Ready => Some(self.button(label, "Set up", action, false, window, cx)),
            Setup::Incomplete => Some(self.button(label, "Retry", action, false, window, cx)),
            _ => None,
        };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(text(label, 18., 24., 400., t.text))
                    .child(text(status, 16., 20., 400., t.text_description).whitespace_normal()),
            )
            .children(control)
            .when(setup == Setup::Done, |row| {
                row.child(icon("menu-check", 16., t.text))
            })
    }
    fn capabilities_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let computer_use = cx
            .try_global::<ComputerUseState>()
            .map_or(Setup::Unavailable, |state| {
                computer_use_setup(state, self.computer_use).0
            });
        let rows = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(self.capability(
                "Computer use",
                "Control apps to complete tasks",
                computer_use,
                Action::SetUpComputerUse,
                window,
                cx,
            ))
            .child(self.capability(
                "Computer history",
                "Find past work and pick up where you left off",
                self.history,
                Action::SetUpHistory,
                window,
                cx,
            ));
        self.step_view(
            "Give ChatGPT access to your computer",
            Some(
                "Let ChatGPT use your apps, browser, files, and work history to understand what you’re working on and help you get things done with less explaining",
            ),
            rows,
            "Continue",
            window,
            cx,
        )
    }
    /// `orbit.onboarding.connectors.*` over the first `app/list` entries.
    fn connectors_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut connected = false;
        let body = match &self.apps {
            None => text("Checking your tools", 16., 24., 400., t.text_description),
            Some(Err(_)) => div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(text(
                    "We couldn’t load your tools",
                    16.,
                    24.,
                    400.,
                    t.text_description,
                ))
                .child(self.button(
                    "onboarding-apps-retry",
                    "Retry",
                    Action::ReloadApps,
                    false,
                    window,
                    cx,
                )),
            Some(Ok(apps)) => {
                let mut list = div().w_full().flex().flex_col().gap(px(12.));
                for (index, app) in apps.iter().enumerate() {
                    let accessible = app["isAccessible"] == true;
                    connected |= accessible;
                    let opened = self
                        .connecting
                        .iter()
                        .any(|id| app["id"].as_str() == Some(id.as_str()));
                    let row = div().w_full().flex().items_center().gap(px(16.)).child(
                        text(
                            app["name"].as_str().unwrap_or_default().to_owned(),
                            18.,
                            24.,
                            400.,
                            t.text,
                        )
                        .min_w_0()
                        .flex_1()
                        .truncate(),
                    );
                    list = list.child(if accessible {
                        row.child(text("Connected", 16., 20., 400., t.text_description))
                    } else {
                        row.child(self.button(
                            ("onboarding-connect", index),
                            if opened {
                                "Check connection"
                            } else {
                                "Connect"
                            },
                            Action::Connect(index),
                            false,
                            window,
                            cx,
                        ))
                    });
                }
                list
            }
        };
        self.step_view(
            "Make your ChatGPT more helpful",
            None,
            body,
            if connected { "Continue" } else { "Not now" },
            window,
            cx,
        )
    }
    fn starter_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut cards = div().w_full().flex().flex_col().gap(px(8.));
        for (index, (label, description, _)) in STARTERS.iter().enumerate() {
            cards = cards.child(
                self.button(
                    ("onboarding-starter", index),
                    *label,
                    Action::Starter(index),
                    false,
                    window,
                    cx,
                )
                .w_full()
                .h_auto()
                .py(px(12.))
                .flex_col()
                .gap(px(4.))
                .rounded(px(15.))
                .child(
                    text(*description, 13., 18., 400., t.text_description)
                        .whitespace_normal()
                        .text_center(),
                ),
            );
        }
        self.step_view(
            "Start with a simple task to see how it works",
            None,
            cards,
            "Skip",
            window,
            cx,
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
            State::Capabilities=>self.capabilities_view(window,cx),
            State::Connectors=>self.connectors_view(window,cx),
            State::Starter=>self.starter_view(window,cx),
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
    use crate::screens::computer_use::Plugin;

    #[test]
    fn setup_steps_follow_roles_and_import_and_back_returns() {
        let mut forward = vec![State::Roles];
        while let Some(next) = next_step(*forward.last().unwrap(), true) {
            forward.push(next);
        }
        assert_eq!(
            forward,
            [
                State::Roles,
                State::Import,
                State::Capabilities,
                State::Connectors,
                State::Starter
            ]
        );
        assert_eq!(next_step(State::Roles, false), Some(State::Capabilities));
        assert_eq!(
            previous_step(State::Capabilities, false),
            Some(State::Roles)
        );
        assert_eq!(
            previous_step(State::Capabilities, true),
            Some(State::Import)
        );
        assert_eq!(previous_step(State::Starter, true), Some(State::Connectors));
        assert_eq!(next_step(State::Workspace, true), None);
    }

    #[test]
    fn computer_use_setup_installs_then_enables_once() {
        let mut state = ComputerUseState {
            available: true,
            plugin: Load::Ready(Some(Plugin {
                installed: false,
                enabled: false,
            })),
            ..Default::default()
        };
        assert_eq!(computer_use_setup(&state, None).0, Setup::Ready);
        state.pending = true;
        let step = Some(ComputerUseStep::Install);
        assert_eq!(computer_use_setup(&state, step).0, Setup::SettingUp);
        assert_eq!(computer_use_setup(&state, None).0, Setup::Checking);
        state.pending = false;
        state.plugin = Load::Ready(Some(Plugin {
            installed: true,
            enabled: false,
        }));
        let (setup, step, action) = computer_use_setup(&state, step);
        assert_eq!(setup, Setup::SettingUp);
        assert_eq!(step, Some(ComputerUseStep::Enable));
        assert!(matches!(action, Some(ComputerUseAction::SetEnabled(true))));
        // Enable finished without registering the provider: incomplete, never a loop.
        let (setup, step, action) = computer_use_setup(&state, step);
        assert_eq!(
            (setup, step),
            (Setup::Incomplete, Some(ComputerUseStep::Failed))
        );
        assert!(action.is_none());
        assert_eq!(computer_use_setup(&state, step).0, Setup::Incomplete);
        state.plugin = Load::Ready(Some(Plugin {
            installed: true,
            enabled: true,
        }));
        assert_eq!(computer_use_setup(&state, step).0, Setup::Done);
        state.available = false;
        assert_eq!(computer_use_setup(&state, None).0, Setup::Unavailable);
    }
    #[test]
    fn role_priority_and_skip_match_completion_contract() {
        let mut value = Completion {
            roles: vec!["marketing".into(), "engineering".into()],
            personalized_suggestions: true,
            skipped: false,
            roots: Vec::new(),
        };
        assert_eq!(value.representative_role(), Some("engineering"));
        assert_eq!(value.work_mode(), "coding");
        value.skipped = true;
        assert_eq!(value.representative_role(), None);
        value.roles = vec!["marketing".into()];
        assert_eq!(value.work_mode(), "non_coding");
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
