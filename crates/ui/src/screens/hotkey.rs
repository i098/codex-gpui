use crate::{
    composer::{Composer, Permission, pill, popover},
    icon::icon,
    text,
};
use gpui::{
    App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme as _, Theme};

pub fn init(cx: &mut App) {
    cx.bind_keys([gpui::KeyBinding::new(
        "escape",
        crate::composer::Dismiss,
        Some("HotkeySettings"),
    )]);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Local,
    Worktree,
    Cloud,
}

#[derive(Clone, Debug)]
pub enum Change {
    Project(Option<String>),
    Mode(Mode),
    Branch(String),
    Environment(Option<String>),
    Permission(Permission),
    EnvironmentSettings,
}

#[derive(Clone, Default)]
pub struct Options {
    pub projects: Vec<(SharedString, String)>,
    pub root: Option<String>,
    pub loading: bool,
    pub git_root: Option<String>,
    pub branches: Vec<String>,
    pub branch: Option<String>,
    pub mode: Mode,
    pub cloud_available: bool,
    pub environments: Vec<(String, String)>,
    pub environment: Option<String>,
    pub environment_error: Option<String>,
}

impl Options {
    pub fn placeholder(&self) -> String {
        let Some(root) = &self.root else {
            return "Ask ChatGPT anything locally".into();
        };
        let project = self
            .projects
            .iter()
            .find(|(_, path)| path == root)
            .map(|(name, _)| name.as_ref())
            .or_else(|| {
                std::path::Path::new(root)
                    .file_name()
                    .and_then(|name| name.to_str())
            })
            .unwrap_or("this project");
        match self.mode {
            Mode::Local => format!("Ask ChatGPT anything locally in {project}"),
            Mode::Worktree => format!("Ask ChatGPT anything in a worktree in {project}"),
            Mode::Cloud => "Ask ChatGPT anything in the cloud".into(),
        }
    }

    pub fn start_in_disabled(&self) -> Option<&'static str> {
        if self.loading {
            Some("Loading project")
        } else if self.root.is_none() {
            Some("Standalone chats run in a local folder")
        } else if self.git_root.is_none() {
            Some("Initialize a git repo to start in cloud or worktree mode")
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Settings,
    Project,
    Mode,
    Branch,
    Environment,
    Permission,
}

pub struct Controls {
    options: Options,
    composer: gpui::WeakEntity<Composer>,
    open: Option<Menu>,
    selected: usize,
    focus: FocusHandle,
}

impl EventEmitter<Change> for Controls {}
impl Focusable for Controls {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Controls {
    pub fn new(composer: Entity<Composer>, options: Options, cx: &mut Context<Self>) -> Self {
        Self {
            options,
            composer: composer.downgrade(),
            open: None,
            selected: 0,
            focus: cx.focus_handle(),
        }
    }

    pub fn update_options(&mut self, options: Options, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        if let Some(composer) = self.composer.upgrade() {
            window.focus(&composer.focus_handle(cx), cx);
        }
        cx.notify();
    }

    fn entries(
        &self,
        cx: &App,
    ) -> Vec<(String, Option<Change>, Option<Menu>, Option<&'static str>)> {
        match self.open.unwrap_or(Menu::Settings) {
            Menu::Settings => {
                let project = self
                    .options
                    .root
                    .as_ref()
                    .and_then(|root| self.options.projects.iter().find(|(_, path)| path == root))
                    .map_or("Standalone".into(), |(name, _)| name.to_string());
                let mode = match self.options.mode {
                    Mode::Local => "Local project",
                    Mode::Worktree => "Worktree",
                    Mode::Cloud => "Cloud",
                };
                let mut rows = vec![
                    (
                        format!("Project\t{project}"),
                        None,
                        Some(Menu::Project),
                        None,
                    ),
                    (
                        format!("Start in\t{mode}"),
                        None,
                        Some(Menu::Mode),
                        self.options.start_in_disabled(),
                    ),
                ];
                if self.options.mode == Mode::Worktree {
                    let environment = self
                        .options
                        .environment
                        .as_ref()
                        .and_then(|path| self.options.environments.iter().find(|(_, p)| p == path))
                        .map_or("None", |(name, _)| name.as_str());
                    rows.push((
                        format!("Environment\t{environment}"),
                        None,
                        Some(Menu::Environment),
                        None,
                    ));
                }
                if self.options.git_root.is_some() {
                    rows.push((
                        format!(
                            "Branch\t{}",
                            self.options.branch.as_deref().unwrap_or("HEAD")
                        ),
                        None,
                        Some(Menu::Branch),
                        None,
                    ));
                }
                if self.options.mode != Mode::Cloud {
                    let permission = match self
                        .composer
                        .upgrade()
                        .map_or(Permission::Custom, |composer| {
                            composer.read(cx).permission()
                        }) {
                        Permission::Ask => "Ask for approval",
                        Permission::ApproveForMe => "Approve for me",
                        Permission::FullAccess => "Full access",
                        Permission::Custom => "Custom",
                    };
                    rows.push((
                        format!("Permissions\t{permission}"),
                        None,
                        Some(Menu::Permission),
                        None,
                    ));
                }
                rows
            }
            Menu::Project => {
                std::iter::once(("Standalone".into(), Some(Change::Project(None)), None, None))
                    .chain(self.options.projects.iter().map(|(name, path)| {
                        (
                            name.to_string(),
                            Some(Change::Project(Some(path.clone()))),
                            None,
                            None,
                        )
                    }))
                    .collect()
            }
            Menu::Mode => vec![
                (
                    "Work locally".into(),
                    Some(Change::Mode(Mode::Local)),
                    None,
                    None,
                ),
                (
                    "Cloud".into(),
                    Some(Change::Mode(Mode::Cloud)),
                    None,
                    (!self.options.cloud_available).then_some("Cloud is unavailable"),
                ),
                (
                    "Worktree".into(),
                    Some(Change::Mode(Mode::Worktree)),
                    None,
                    self.options
                        .git_root
                        .is_none()
                        .then_some("Initialize a git repo to run tasks in worktrees"),
                ),
            ],
            Menu::Branch => self
                .options
                .branches
                .iter()
                .map(|branch| {
                    (
                        branch.clone(),
                        Some(Change::Branch(branch.clone())),
                        None,
                        None,
                    )
                })
                .collect(),
            Menu::Environment => {
                std::iter::once(("None".into(), Some(Change::Environment(None)), None, None))
                    .chain(self.options.environments.iter().map(|(name, path)| {
                        (
                            name.clone(),
                            Some(Change::Environment(Some(path.clone()))),
                            None,
                            None,
                        )
                    }))
                    .chain(std::iter::once((
                        "Open settings".into(),
                        Some(Change::EnvironmentSettings),
                        None,
                        None,
                    )))
                    .collect()
            }
            Menu::Permission => [
                ("Ask for approval", Permission::Ask),
                ("Approve for me", Permission::ApproveForMe),
                ("Full access", Permission::FullAccess),
                ("Custom (config.toml)", Permission::Custom),
            ]
            .into_iter()
            .map(|(label, permission)| {
                (
                    label.into(),
                    Some(Change::Permission(permission)),
                    None,
                    None,
                )
            })
            .collect(),
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, change, submenu, disabled)) = self.entries(cx).into_iter().nth(index) else {
            return;
        };
        if disabled.is_some() {
            return;
        }
        if let Some(menu) = submenu {
            self.open = Some(menu);
            self.selected = 0;
            cx.notify();
        } else if let Some(change) = change {
            cx.emit(change);
            self.close(window, cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.entries(cx);
        match event.keystroke.key.as_str() {
            "escape" => self.close(window, cx),
            "left" if self.open != Some(Menu::Settings) => {
                self.open = Some(Menu::Settings);
                self.selected = 0;
            }
            "down" => {
                if let Some(next) = (self.selected + 1..rows.len()).find(|&i| rows[i].3.is_none()) {
                    self.selected = next;
                }
            }
            "up" => {
                if let Some(previous) = (0..self.selected).rev().find(|&i| rows[i].3.is_none()) {
                    self.selected = previous;
                }
            }
            "enter" | "space" | "right" => self.activate(self.selected, window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let button = pill("hotkey-chat-settings", &t)
            .role(gpui::Role::Button)
            .aria_label("Chat settings")
            .aria_expanded(self.open.is_some())
            .focusable()
            .tab_index(0)
            .w(px(28.))
            .justify_center()
            .child(icon("settings-general", 16., t.text_tertiary))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.open.is_some() {
                    this.close(window, cx);
                } else {
                    this.open = Some(Menu::Settings);
                    this.selected = 0;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                    this.open = Some(Menu::Settings);
                    this.selected = 0;
                    window.focus(&this.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }));
        let mut root = div().relative().child(button);
        if self.open.is_some() {
            let rows = self.entries(cx);
            let menu = popover(&t, 320., |menu| {
                menu.gap(px(4.))
                    .p(px(8.))
                    .children(rows.into_iter().enumerate().map(
                        |(index, (label, _, _, disabled))| {
                            let (title, value) = label.split_once('\t').unwrap_or((&label, ""));
                            let row = div()
                                .id(("hotkey-setting", index))
                                .role(gpui::Role::MenuItem)
                                .aria_label(label.clone())
                                .aria_disabled(disabled.is_some())
                                .h(px(28.))
                                .px(px(4.))
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(16.))
                                .when(index == self.selected, |row| row.bg(t.ghost_hover))
                                .opacity(if disabled.is_some() { 0.5 } else { 1. })
                                .child(text(title.to_owned(), 13., 18., 500., t.text))
                                .child(text(value.to_owned(), 13., 18., 400., t.text_tertiary))
                                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                    this.selected = index;
                                    cx.notify();
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.activate(index, window, cx)
                                }));
                            if let Some(reason) = disabled {
                                crate::tooltip::tooltip(
                                    row,
                                    reason,
                                    None,
                                    crate::tooltip::Side::Top,
                                )
                                .into_any_element()
                            } else {
                                row.into_any_element()
                            }
                        },
                    ))
            });
            root = root.child(gpui::deferred(
                gpui::anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(gpui::point(px(0.), px(-38.)))
                    .child(
                        div()
                            .id("hotkey-settings-menu")
                            .role(gpui::Role::Menu)
                            .track_focus(&self.focus)
                            .key_context("HotkeySettings")
                            .on_action(cx.listener(
                                |this, _: &crate::composer::Dismiss, window, cx| {
                                    this.close(window, cx)
                                },
                            ))
                            .capture_key_down(cx.listener(Self::key))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_down_out(
                                cx.listener(|this, _, window, cx| this.close(window, cx)),
                            )
                            .child(menu),
                    ),
            ));
        }
        root
    }
}

pub fn home(composer: Entity<Composer>, t: &Theme) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .justify_end()
        .p(px(4.))
        .child(
            div()
                .relative()
                .rounded(px(32.))
                .border_1()
                .border_color(t.border.opacity(0.72))
                .bg(t.surface_elevated_secondary.opacity(0.94))
                .pt(px(17.))
                .px(px(15.))
                .pb(px(13.))
                .child(composer),
        )
}
