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
    pub projectless: bool,
    pub loading: bool,
    pub git_root: Option<String>,
    pub branches: Vec<String>,
    pub branch: Option<String>,
    pub mode: Mode,
    pub cloud_available: bool,
    pub environments: Vec<(String, String)>,
    /// Cloud environments `(label, id)`: the `Environment` choices of a `Cloud` start.
    pub cloud_environments: Vec<(String, String)>,
    pub environment: Option<String>,
    pub environment_error: Option<String>,
}

type Entry = (String, Option<Change>, Option<Menu>, Option<&'static str>);

impl Options {
    pub fn placeholder(&self) -> String {
        if self.projectless {
            return "Ask ChatGPT anything locally".into();
        }
        let project = self
            .root
            .as_ref()
            .and_then(|root| {
                self.projects
                    .iter()
                    .find(|(_, path)| path == root)
                    .map(|(name, _)| name.as_ref())
                    .or_else(|| {
                        std::path::Path::new(root)
                            .file_name()
                            .and_then(|name| name.to_str())
                    })
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
        } else if self.projectless {
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
    selected: Option<usize>,
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
            selected: None,
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

    fn entries(&self, cx: &App) -> Vec<Entry> {
        match self.open.unwrap_or(Menu::Settings) {
            Menu::Settings => {
                let project = self
                    .options
                    .root
                    .as_ref()
                    .and_then(|root| self.options.projects.iter().find(|(_, path)| path == root))
                    .map_or("New chat".into(), |(name, _)| name.to_string());
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
                if self.options.mode != Mode::Local {
                    let choices = if self.options.mode == Mode::Cloud {
                        &self.options.cloud_environments
                    } else {
                        &self.options.environments
                    };
                    let environment = self
                        .options
                        .environment
                        .as_ref()
                        .and_then(|path| choices.iter().find(|(_, p)| p == path))
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
                    let permission = self
                        .composer
                        .upgrade()
                        .map_or(Permission::Custom, |composer| {
                            composer.read(cx).permission()
                        });
                    rows.push((
                        format!("Permissions\t{}", permission.short_label()),
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
            Menu::Environment if self.options.mode == Mode::Cloud => self
                .options
                .cloud_environments
                .iter()
                .map(|(label, id)| {
                    (
                        label.clone(),
                        Some(Change::Environment(Some(id.clone()))),
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
            Menu::Permission => {
                let mut entries: Vec<_> = [
                    ("Ask for approval", Permission::Ask),
                    ("Approve for me", Permission::ApproveForMe),
                    ("Full access", Permission::FullAccess),
                    ("Custom (config.toml)", Permission::Custom),
                ]
                .into_iter()
                .filter(|(_, permission)| {
                    *permission != Permission::FullAccess
                        || !cx
                            .try_global::<crate::composer::FullAccessHidden>()
                            .is_some_and(|hidden| hidden.0)
                })
                .map(|(label, permission)| {
                    (
                        label.into(),
                        Some(Change::Permission(permission)),
                        None,
                        None,
                    )
                })
                .collect();
                if let Some(composer) = self.composer.upgrade() {
                    entries.extend(composer.read(cx).named_permissions().map(|permission| {
                        (
                            permission.option_label().to_owned(),
                            Some(Change::Permission(permission.clone())),
                            None,
                            None,
                        )
                    }));
                }
                entries
            }
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
            self.selected = None;
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
                self.selected = None;
            }
            "down" => {
                self.selected = (self.selected.map_or(0, |index| index + 1)..rows.len())
                    .find(|&index| rows[index].3.is_none());
            }
            "up" => {
                self.selected = (0..self.selected.unwrap_or(rows.len()))
                    .rev()
                    .find(|&index| rows[index].3.is_none());
            }
            "enter" | "space" | "right" => {
                if let Some(index) = self.selected {
                    self.activate(index, window, cx);
                }
            }
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
            .rounded(px(9.375))
            .when(self.open.is_some(), |button| button.bg(t.ghost_hover))
            .justify_center()
            .child(icon("thread-chat-actions", 16., t.text_tertiary))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.open.is_some() {
                    this.close(window, cx);
                } else {
                    this.open = Some(Menu::Settings);
                    this.selected = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                    this.open = Some(Menu::Settings);
                    this.selected = Some(0);
                    window.focus(&this.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }));
        let mut root = div().relative().ml(px(4.)).child(button);
        if self.open.is_some() {
            let settings = self.open == Some(Menu::Settings);
            let rows =
                self.entries(cx)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, _, _, disabled))| {
                        let (title, value) = label.split_once('\t').unwrap_or((&label, ""));
                        let icon_size = if title == "Start in" { 14. } else { 16. };
                        let value_icon = match title {
                            "Project" => Some(if self.options.root.is_some() {
                                "folder"
                            } else {
                                "new-chat"
                            }),
                            "Start in" => Some("laptop"),
                            "Permissions" => Some("permissions"),
                            "Branch" => Some("settings-git"),
                            _ => None,
                        };
                        let color = if settings && title == "Project" {
                            t.text
                        } else {
                            t.text_tertiary
                        };
                        let choice = pill("hotkey-choice", &t)
                            .role(gpui::Role::MenuItem)
                            .aria_label(label.clone())
                            .aria_disabled(disabled.is_some())
                            .gap(px(4.))
                            .px(px(if title == "Project" { 8. } else { 6. }))
                            .when(!settings, |choice| choice.w_full())
                            .when(self.selected == Some(index), |choice| {
                                choice.bg(t.ghost_hover)
                            })
                            .opacity(if disabled.is_some() { 0.4 } else { 1. })
                            .when_some(value_icon.filter(|_| settings), |choice, name| {
                                choice.child(icon(name, icon_size, color))
                            })
                            .child(text(
                                if settings { value } else { title }.to_owned(),
                                13.,
                                18.,
                                if title == "Start in" { 430. } else { 400. },
                                color,
                            ))
                            .when(settings && title != "Permissions", |choice| {
                                choice.child(icon("chevron-down", icon_size, color))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate(index, window, cx)
                            }));
                        let row = div()
                            .id(("hotkey-setting", index))
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .when(settings, |row| {
                                row.child(text(title.to_owned(), 13., 18.571_428, 500., t.text))
                            })
                            .child(choice);
                        if let Some(reason) = disabled {
                            crate::tooltip::tooltip(row, reason, None, crate::tooltip::Side::Top)
                                .into_any_element()
                        } else {
                            row.into_any_element()
                        }
                    })
                    .collect::<Vec<_>>();
            let menu = if settings {
                div()
                    .relative()
                    .w(px(320.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(Theme::RADIUS_XL),
                        fill: Some(t.menu_background),
                        shadows: vec![
                            shapes::Shadow {
                                color: t.border,
                                offset: gpui::point(px(0.), px(0.)),
                                blur: px(0.),
                                spread: px(0.5),
                                inset: false,
                            },
                            shapes::Shadow {
                                color: gpui::rgba(0x0000001a).into(),
                                offset: gpui::point(px(0.), px(4.)),
                                blur: px(8.),
                                spread: px(-2.),
                                inset: false,
                            },
                        ],
                        ..Default::default()
                    }))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .p(px(16.))
                            .gap(px(16.))
                            .children(rows),
                    )
            } else {
                popover(&t, 320., |menu| menu.p(px(8.)).gap(px(4.)).children(rows))
            };
            root = root.child(gpui::deferred(
                gpui::anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .snap_to_window_with_margin(gpui::Edges::all(px(6.)))
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
                .rounded(Theme::RADIUS_4XL)
                .pt(px(18.))
                .px(px(16.))
                .pb(px(14.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(Theme::RADIUS_4XL),
                    fill: Some(t.surface_secondary.opacity(0.94)),
                    border: Some((px(1.), t.border.opacity(0.72))),
                    shadows: vec![shapes::Shadow {
                        color: gpui::rgba(0xffffff1a).into(),
                        offset: gpui::point(px(0.), px(1.)),
                        blur: px(0.),
                        spread: px(0.),
                        inset: true,
                    }],
                    ..Default::default()
                }))
                .child(div().absolute().inset(px(1.)).child(shapes::decoration(
                    shapes::Decoration {
                        radius: gpui::Corners::all(Theme::RADIUS_4XL),
                        fill: Some(t.surface_secondary.opacity(0.94)),
                        ..Default::default()
                    },
                )))
                .child(div().relative().child(composer)),
        )
}
