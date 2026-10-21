use crate::{
    MONO, SANS,
    confirmation::{Confirmation, Outcome},
    focus,
    icon::icon,
    text,
};
use gpui::{
    AppContext as _, Context, Div, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px,
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
    FirstRunComplete,
    PickFolder,
    CreateProject,
    OpenImport(Completion),
}
impl EventEmitter<OnboardingEvent> for Onboarding {}

#[derive(Clone, Copy)]
enum Action {
    Continue,
    Skip,
    Back,
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
    FirstRun(usize),
    Error,
}

pub struct Onboarding {
    page: Page,
    state: State,
    roles: Vec<String>,
    suggestions: bool,
    import_available: bool,
    projects: Vec<Project>,
    roots: Vec<String>,
    add_project: bool,
    busy: bool,
    error: Option<String>,
    confirmation: Option<Entity<Confirmation>>,
}

impl Onboarding {
    pub fn new(page: Page, chatgpt: bool) -> Self {
        Self {
            page,
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
            busy: false,
            error: None,
            confirmation: None,
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
                State::Roles if !self.roles.is_empty() => {
                    if self.import_available {
                        self.state = State::Import;
                    } else {
                        self.complete(false, cx);
                    }
                }
                State::Import => self.complete(false, cx),
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
            Action::AddProject => self.add_project = !self.add_project,
            Action::PickFolder => {
                self.add_project = false;
                cx.emit(OnboardingEvent::PickFolder);
            }
            Action::CreateProject => {
                self.add_project = false;
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
            Action::Import => cx.emit(OnboardingEvent::OpenImport(self.completion(false))),
            Action::NotNow => self.complete(false, cx),
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
        let border = t.border;
        let (background, foreground) = if primary {
            (t.button_primary_solid, t.text_inverse)
        } else {
            (t.surface, t.text)
        };
        let label = label.into();
        let id = id.into();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let control = div()
            .id(id)
            .relative()
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .h(px(44.))
            .px(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(background)
            .text_color(foreground)
            .cursor_pointer()
            .when(!primary, |button| button.border_1().border_color(border))
            .when(self.busy, |button| button.opacity(0.5))
            .on_click(cx.listener(move |this, _, window, cx| this.action(action, window, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.action(action, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(text(label, 14., 20., 500., foreground));
        focus::ring(control, &handle, Theme::RADIUS_FULL, px(2.), cx)
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
                        .child(icon("check", 14., t.text));
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
                    if self.suggestions { "check" } else { "plus" },
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
    fn workspace_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut body = div()
            .w(px(560.))
            .max_w_full()
            .p(px(40.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text("Select a project", 28., 34., 400., t.text))
            .child(
                text(
                    "ChatGPT will be able to edit files and run commands in selected folders",
                    14.,
                    20.,
                    400.,
                    t.text_description,
                )
                .whitespace_normal(),
            )
            .child(self.button(
                "onboarding-add-project",
                "Add project",
                Action::AddProject,
                false,
                window,
                cx,
            ));
        if self.add_project {
            body = body.child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(self.button(
                        "onboarding-new-project",
                        "Start from scratch",
                        Action::CreateProject,
                        false,
                        window,
                        cx,
                    ))
                    .child(self.button(
                        "onboarding-existing-project",
                        "Use an existing folder",
                        Action::PickFolder,
                        false,
                        window,
                        cx,
                    )),
            );
        }
        if !self.projects.is_empty() {
            body = body
                .child(text("Available projects", 14., 20., 500., t.text))
                .child(self.button(
                    "onboarding-select-all",
                    "Select all",
                    Action::SelectAll,
                    false,
                    window,
                    cx,
                ));
            for (index, project) in self.projects.iter().enumerate() {
                let selected = self.roots.contains(&project.root);
                body = body.child(
                    self.button(
                        ("onboarding-project", index),
                        project.label.clone(),
                        Action::ToggleProject(index),
                        false,
                        window,
                        cx,
                    )
                    .aria_toggled(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .when(selected, |button| button.child(icon("check", 14., t.text))),
                );
            }
        } else {
            body = body.child(text(
                "Add a project to continue.",
                14.,
                20.,
                400.,
                t.text_description,
            ));
        }
        body.child(
            self.button(
                "workspace-continue",
                "Continue",
                Action::Continue,
                true,
                window,
                cx,
            )
            .when(self.roots.is_empty(), |button| button.opacity(0.3)),
        )
        .child(self.button(
            "workspace-skip",
            "Skip",
            Action::CreateProject,
            false,
            window,
            cx,
        ))
    }
    fn first_run_view(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let titles = [
            "Codex in your IDE",
            "Hand off to Codex in the cloud",
            "Turn TODOs into Codex tasks",
            "Decide how much autonomy you want to grant",
        ];
        let descriptions = [
            "Codex navigates, edits, runs commands, and executes tests directly in your repo. Powered by your ChatGPT account.",
            "Send tasks to Codex to run in the background so you can stay focused and move faster.",
            "Write a TODO comment and convert it into a Codex task with a single click.",
            "For more details, see the Codex docs",
        ];
        let mut body = div()
            .w(px(560.))
            .max_w_full()
            .p(px(40.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(24.))
            .child(
                text(titles[index], 28., 34., 500., t.text)
                    .whitespace_normal()
                    .text_center(),
            )
            .child(
                text(descriptions[index], 14., 20., 400., t.text_description)
                    .whitespace_normal()
                    .text_center(),
            );
        if index < 3 {
            let sample = match index {
                0 => "Ask Codex to do anything",
                1 => {
                    "Explain repository to a new designer\nopenai/agi · Oct 12\n\nFix an onboarding bug\nopenai/agi · Oct 9\n\nCreate a darkmode theme\nopenai/codex · Oct 8"
                }
                _ => "// TODO: implement schema",
            };
            body = body.child(
                text(sample, 14., 22., 400., t.text)
                    .font_family(MONO)
                    .whitespace_normal()
                    .p(px(24.))
                    .w_full()
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(16.)),
            );
        } else {
            body = body.child(
                div()
                    .id("first-run-docs")
                    .role(gpui::Role::Link)
                    .child(text("Codex docs", 14., 20., 400., t.text).underline())
                    .on_click(|_, _, cx| cx.open_url("https://developers.openai.com/codex/ide")),
            );
            body = body
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(text("Codex can make mistakes", 14., 20., 500., t.text))
                        .child(
                            text(
                                "Review the code it writes and commands it runs",
                                14.,
                                20.,
                                400.,
                                t.text_description,
                            )
                            .whitespace_normal(),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(text(
                            "Powered by your ChatGPT account",
                            14.,
                            20.,
                            500.,
                            t.text,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(4.))
                                .child(text(
                                    "Uses your plan’s rate limits and",
                                    14.,
                                    20.,
                                    400.,
                                    t.text_description,
                                ))
                                .child(
                                    div()
                                        .id("first-run-data-controls")
                                        .role(gpui::Role::Link)
                                        .child(
                                            text(
                                                "training data preferences",
                                                14.,
                                                20.,
                                                400.,
                                                t.text_description,
                                            )
                                            .underline(),
                                        )
                                        .on_click(|_, _, cx| {
                                            cx.open_url(
                                                "https://chatgpt.com/#settings/DataControls",
                                            )
                                        }),
                                ),
                        ),
                );
        }
        body.child(
            div()
                .flex()
                .gap(px(12.))
                .when(index > 0, |row| {
                    row.child(self.button(
                        "first-run-back",
                        "Back",
                        Action::Back,
                        false,
                        window,
                        cx,
                    ))
                })
                .child(self.button(
                    "first-run-next",
                    if index < 3 { "Next" } else { "Continue" },
                    Action::Continue,
                    true,
                    window,
                    cx,
                )),
        )
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body=match self.state {
            State::Loading=>div().child(text("Loading…",14.,20.,400.,t.text_description)),
            State::Error=>div().flex().flex_col().items_center().gap(px(16.))
                .child(text("Couldn’t load setup. Please reload the app",14.,20.,400.,t.text_description))
                .child(self.button("onboarding-reload","Reload app",Action::Reload,false,window,cx)),
            State::Roles=>self.role_view(window,cx),
            State::Workspace=>self.workspace_view(window,cx),
            State::FirstRun(index)=>self.first_run_view(index,window,cx),
            State::Import=>div().w(px(640.)).max_w_full().p(px(40.)).flex().flex_col().gap(px(24.))
                .child(text("Import work from other AI apps",28.,34.,400.,t.text))
                .child(text("Before we get started, would you like to bring over your work from other AI apps? You can review your setup, projects, and chats before importing.",18.,28.,400.,t.text).whitespace_normal())
                .child(self.button("onboarding-import","Bring over my setup",Action::Import,true,window,cx))
                .child(self.button("onboarding-not-now","Not now",Action::NotNow,false,window,cx)),
        };
        div()
            .size_full()
            .relative()
            .bg(t.surface)
            .font_family(SANS)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .on_key_down(|event, window, cx| focus::tab(event, window, cx))
            .child(body)
            .children(self.error.clone().map(|error| {
                text(error, 14., 20., 400., t.text_error)
                    .max_w(px(560.))
                    .whitespace_normal()
            }))
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
        };
        assert_eq!(value.representative_role(), Some("engineering"));
        assert_eq!(value.work_mode(), "coding");
        value.skipped = true;
        assert_eq!(value.representative_role(), None);
        value.roles = vec!["marketing".into()];
        assert_eq!(value.work_mode(), "non_coding");
    }
}
