use super::computer_use::Load;
use super::{Layer, button, select};
use crate::{
    composer::{Composer, popover},
    icon::icon,
    text,
};
use gpui::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, Global, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, point, prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

#[derive(Clone)]
pub enum Action {
    Refresh,
    AddProject,
    Read {
        path: String,
    },
    Save {
        root: String,
        path: String,
        revision: Option<String>,
        definition: Value,
    },
}

pub type Dispatch = Arc<dyn Fn(Action, &mut Window, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub projects: Load<Arc<[Value]>>,
    pub selected: Load<Arc<Value>>,
    pub selected_path: Option<Arc<str>>,
    pub pending: bool,
    pub error: Option<String>,
    pub conflict: bool,
    pub saved_generation: usize,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

pub fn render(
    key: SharedString,
    layer: &Layer,
    composited: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let view = window.use_keyed_state(key, cx, |window, cx| {
        cx.observe_global::<State>(|_, cx| cx.notify()).detach();
        if let Some(dispatch) = cx
            .try_global::<State>()
            .and_then(|state| state.dispatch.clone())
        {
            dispatch(Action::Refresh, window, cx);
        }
        Environments {
            layer: layer.clone(),
            composited,
            root: None,
            path: None,
            editing: false,
            fields: None,
            inherited: Vec::new(),
            generation: 0,
            menu: None,
            variables: false,
            preview_platforms: [0; 2],
            expanded_actions: Vec::new(),
        }
    });
    view.update(cx, |this, cx| {
        if this.composited != composited {
            this.composited = composited;
            cx.notify();
        }
    });
    view.into_any_element()
}

struct Fields {
    initial: Value,
    name: Entity<Composer>,
    setup: [Entity<Composer>; 4],
    cleanup: [Entity<Composer>; 4],
    setup_platform: usize,
    cleanup_platform: usize,
    actions: Vec<ActionFields>,
}
struct ActionFields {
    name: Entity<Composer>,
    command: Entity<Composer>,
    icon: usize,
    platform: usize,
}
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Icon(usize),
    Platform(usize),
}
struct Environments {
    layer: Layer,
    composited: bool,
    root: Option<String>,
    path: Option<String>,
    editing: bool,
    fields: Option<Fields>,
    inherited: Vec<String>,
    generation: usize,
    menu: Option<Menu>,
    variables: bool,
    preview_platforms: [usize; 2],
    expanded_actions: Vec<usize>,
}
const PLATFORMS: [&str; 4] = ["default", "darwin", "linux", "win32"];
const PLATFORM_LABELS: [&str; 4] = ["Default", "macOS", "Linux", "Windows"];
const ICONS: [&str; 4] = ["tool", "run", "debug", "test"];

fn field(
    placeholder: &'static str,
    value: &str,
    rows: usize,
    cx: &mut Context<Environments>,
) -> Entity<Composer> {
    let field = cx.new(|cx| {
        if rows > 1 {
            Composer::settings_field(placeholder, value, true, cx)
                .with_field_min_height(px(18.571_428 * rows as f32))
                .monospace_field()
                .input_style()
        } else {
            Composer::settings_compact_field(placeholder, value, cx)
        }
    });
    cx.observe(&field, |_, _, cx| cx.notify()).detach();
    field
}

impl Fields {
    fn new(value: &Value, cx: &mut Context<Environments>) -> Self {
        let scripts = |key: &str, cx: &mut Context<Environments>| {
            std::array::from_fn(|i| {
                field(
                    "",
                    if i == 0 {
                        value[key]["script"].as_str()
                    } else {
                        value[key][PLATFORMS[i]]["script"].as_str()
                    }
                    .unwrap_or_default(),
                    6,
                    cx,
                )
            })
        };
        let mut fields = Self {
            initial: Value::Null,
            name: field("Name", value["name"].as_str().unwrap_or("local"), 1, cx),
            setup: scripts("setup", cx),
            cleanup: scripts("cleanup", cx),
            setup_platform: 0,
            cleanup_platform: 0,
            actions: value["actions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|action| ActionFields {
                    name: field("Name", action["name"].as_str().unwrap_or_default(), 1, cx),
                    command: field(
                        "npm run dev",
                        action["command"].as_str().unwrap_or_default(),
                        4,
                        cx,
                    ),
                    icon: ICONS
                        .iter()
                        .position(|name| action["icon"] == *name)
                        .unwrap_or(0),
                    platform: PLATFORMS
                        .iter()
                        .position(|name| action["platform"] == *name)
                        .unwrap_or(0),
                })
                .collect(),
        };
        fields.initial = fields.definition(cx);
        fields
    }

    fn definition(&self, cx: &App) -> Value {
        let scripts = |fields: &[Entity<Composer>; 4]| {
            let mut value = json!({"script": fields[0].read(cx).field_text()});
            for i in 1..4 {
                let script = fields[i].read(cx).field_text();
                if !script.is_empty() {
                    value[PLATFORMS[i]] = json!({"script": script});
                }
            }
            value
        };
        let actions: Vec<_> = self
            .actions
            .iter()
            .filter_map(|action| {
                let name = action.name.read(cx).field_text().trim();
                let command = action.command.read(cx).field_text().trim();
                if name.is_empty() && command.is_empty() {
                    return None;
                }
                let mut value =
                    json!({"name": name, "command": command, "icon": ICONS[action.icon]});
                if action.platform != 0 {
                    value["platform"] = PLATFORMS[action.platform].into();
                }
                Some(value)
            })
            .collect();
        json!({"version": 1, "name": self.name.read(cx).field_text().trim(), "setup": scripts(&self.setup), "cleanup": scripts(&self.cleanup), "actions": actions})
    }
}

impl Environments {
    fn dispatch(&self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dispatch) = cx
            .try_global::<State>()
            .and_then(|state| state.dispatch.clone())
        {
            dispatch(action, window, cx);
        }
    }

    fn open(
        &mut self,
        root: String,
        path: String,
        editing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fields = None;
        self.root = Some(root);
        self.path = Some(path.clone());
        self.editing = editing;
        self.preview_platforms = [0; 2];
        self.expanded_actions.clear();
        self.dispatch(Action::Read { path }, window, cx);
        cx.notify();
    }

    fn projects(&self, state: &State, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let action = button("Add project", t.text_alpha_5, None, t)
            .id("add-environment-project")
            .on_click(
                cx.listener(|this, _, window, cx| this.dispatch(Action::AddProject, window, cx)),
            );
        let heading = div()
            .min_h(px(46.))
            .pb(px(6.))
            .flex()
            .justify_between()
            .items_center()
            .child(text("Select a project", 14., 21., 500., t.text))
            .child(action);
        let rows = match &state.projects {
            Load::Loading => vec![div().p(px(12.)).child(text(
                "Loading projects…",
                14.,
                20.,
                430.,
                t.text_secondary,
            ))],
            Load::Failed(error) => vec![
                div()
                    .p(px(12.))
                    .child(text(error.clone(), 14., 20., 430., t.text_secondary))
                    .child(
                        button("Retry", t.text_alpha_5, None, t)
                            .id("retry-environments")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dispatch(Action::Refresh, window, cx)
                            })),
                    ),
            ],
            Load::Ready(projects) if projects.is_empty() => vec![super::worktrees::card(
                vec![
                    div()
                        .p(px(12.))
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .child(text(
                            "No projects yet. Add one to configure local environments.",
                            13.,
                            18.571_428,
                            430.,
                            t.text_secondary,
                        ))
                        .child(
                            div().flex().child(
                                div()
                                    .id("add-first-environment-project")
                                    .h(px(28.))
                                    .px(px(9.))
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: gpui::Corners::all(px(12.5)),
                                        corner: shapes::Corner::Squircle,
                                        fill: Some(t.button_primary_solid),
                                        border: Some((px(1.), t.border)),
                                        ..Default::default()
                                    }))
                                    .child(text(
                                        "Add project",
                                        13.,
                                        18.,
                                        430.,
                                        t.control_background,
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.dispatch(Action::AddProject, window, cx)
                                    })),
                            ),
                        ),
                ],
                self.composited.then_some(&self.layer),
                t,
            )],
            Load::Ready(projects) => projects
                .iter()
                .map(|project| {
                    let root = project["root"].as_str().unwrap_or_default().to_owned();
                    let name = project["name"].as_str().unwrap_or(&root).to_owned();
                    let path = project["nextConfigPath"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    let create_root = root.clone();
                    let inherited: Vec<_> = project["environments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|entry| entry["inherited"] == true)
                        .collect();
                    let show_inherited = self.inherited.contains(&root);
                    let mut rows = vec![
                        div()
                            .p(px(12.))
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(text(name, 14., 20., 500., t.text))
                            .child(
                                button("+", t.text_alpha_5, None, t)
                                    .id(SharedString::from(format!("add-environment-{root}")))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open(
                                            create_root.clone(),
                                            path.clone(),
                                            true,
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    ];
                    for entry in project["environments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|entry| entry["inherited"] != true)
                    {
                        rows.push(self.environment_row(&root, entry, t, cx));
                    }
                    if !inherited.is_empty() {
                        let toggle_root = root.clone();
                        rows.push(
                            div().child(
                                div()
                                    .id(SharedString::from(format!("inherited-{root}")))
                                    .px(px(16.))
                                    .py(px(12.))
                                    .flex()
                                    .justify_between()
                                    .child(text(
                                        format!("Inherited environments ({})", inherited.len()),
                                        13.,
                                        18.,
                                        430.,
                                        t.text,
                                    ))
                                    .child(icon(
                                        if show_inherited {
                                            "chevron-down"
                                        } else {
                                            "chevron-right"
                                        },
                                        12.,
                                        t.text_secondary,
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(index) = this
                                            .inherited
                                            .iter()
                                            .position(|root| root == &toggle_root)
                                        {
                                            this.inherited.remove(index);
                                        } else {
                                            this.inherited.push(toggle_root.clone());
                                        }
                                        cx.notify();
                                    })),
                            ),
                        );
                        if show_inherited {
                            for entry in inherited {
                                rows.push(self.environment_row(&root, entry, t, cx));
                            }
                        }
                    }
                    super::worktrees::card(rows, self.composited.then_some(&self.layer), t)
                })
                .collect(),
        };
        div().flex().flex_col().child(heading).children(rows)
    }

    fn environment_row(
        &self,
        root: &str,
        entry: &Value,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let path = entry["configPath"].as_str().unwrap_or_default().to_owned();
        let name = if entry["type"] == "error" {
            "Environment needs attention".to_owned()
        } else {
            entry["environment"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .or_else(|| entry["fileName"].as_str())
                .unwrap_or("Environment needs attention")
                .to_owned()
        };
        let detail = entry["source"].as_str().or_else(|| {
            entry["fileName"]
                .as_str()
                .filter(|file| *file != name.as_str())
        });
        let root = root.to_owned();
        div().child(
            div()
                .id(SharedString::from(format!("environment-{path}")))
                .px(px(16.))
                .py(px(12.))
                .flex()
                .justify_between()
                .items_center()
                .child(div().child(text(name, 13., 18., 430., t.text)).children(
                    detail.map(|source| text(source.to_owned(), 12., 16., 430., t.text_secondary)),
                ))
                .child(icon("chevron-right", 12., t.text_secondary))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open(root.clone(), path.clone(), false, window, cx)
                })),
        )
    }

    fn platforms(
        &self,
        setup: bool,
        selected: usize,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let hover = t.list_hover;
        div()
            .flex()
            .gap(px(4.))
            .children(PLATFORM_LABELS.iter().enumerate().map(|(index, label)| {
                div()
                    .id(SharedString::from(format!("platform-{setup}-{index}")))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(8.))
                    .when(index == selected, |d| d.bg(t.text_alpha_5))
                    .hover(move |s| s.bg(hover))
                    .child(text(*label, 13., 18., 430., t.text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.preview_platforms[usize::from(!setup)] = index;
                        if let Some(fields) = &mut this.fields {
                            if setup {
                                fields.setup_platform = index;
                            } else {
                                fields.cleanup_platform = index;
                            }
                        }
                        cx.notify();
                    }))
            }))
    }

    fn preview_script(
        &self,
        setup: bool,
        lifecycle: &Value,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let index = self.preview_platforms[usize::from(!setup)];
        let default = lifecycle["script"].as_str().unwrap_or_default();
        let override_script = (index != 0)
            .then(|| lifecycle[PLATFORMS[index]]["script"].as_str())
            .flatten()
            .filter(|script| !script.is_empty());
        let script = override_script.unwrap_or(default);
        let configured = !default.is_empty()
            || PLATFORMS.iter().skip(1).any(|platform| {
                lifecycle[*platform]["script"]
                    .as_str()
                    .is_some_and(|script| !script.is_empty())
            });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(text(
                        if setup {
                            "Setup script"
                        } else {
                            "Cleanup script"
                        },
                        14.,
                        21.,
                        500.,
                        t.text,
                    ))
                    .when(setup, |heading| heading.child(self.variables_button(t, cx))),
            )
            .child(text(
                if setup {
                    "This script runs on worktree creation"
                } else {
                    "Runs at the project root before worktree cleanup"
                },
                12.,
                16.,
                430.,
                t.text_secondary,
            ))
            .when(configured, |section| {
                section.child(self.platforms(setup, index, t, cx))
            })
            .when(
                index != 0 && override_script.is_none() && !default.is_empty(),
                |section| {
                    section.child(text(
                        "No platform override. Using the default script",
                        13.,
                        20.,
                        430.,
                        t.text_secondary,
                    ))
                },
            )
            .child(div().font_family(crate::MONO).child(text(
                if script.is_empty() {
                    "No script configured".to_owned()
                } else {
                    script.to_owned()
                },
                13.,
                20.,
                430.,
                t.text_secondary,
            )))
    }

    fn preview_actions(&self, actions: &Value, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let actions = actions.as_array().map(Vec::as_slice).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text("Actions", 14., 21., 500., t.text))
            .child(text(
                "These actions can run any command and will be displayed in the header",
                12.,
                16.,
                430.,
                t.text_secondary,
            ))
            .when(actions.is_empty(), |section| {
                section.child(text(
                    "Add an action to run commands from the local toolbar",
                    13.,
                    20.,
                    430.,
                    t.text_secondary,
                ))
            })
            .children(actions.iter().enumerate().map(|(index, action)| {
                let command = action["command"].as_str().unwrap_or_default();
                let multiline = command.trim_end().contains('\n');
                let expanded = self.expanded_actions.contains(&index);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .child(text(
                                        action["name"].as_str().unwrap_or_default().to_owned(),
                                        13.,
                                        20.,
                                        500.,
                                        t.text,
                                    ))
                                    .child(div().font_family(crate::MONO).child(text(
                                        command.lines().next().unwrap_or_default().to_owned(),
                                        12.,
                                        18.,
                                        430.,
                                        t.text_secondary,
                                    ))),
                            )
                            .when(multiline, |row| {
                                row.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "expand-environment-action-{index}"
                                        )))
                                        .role(gpui::Role::Button)
                                        .aria_label(if expanded {
                                            "Hide full command"
                                        } else {
                                            "Show full command"
                                        })
                                        .child(icon(
                                            if expanded {
                                                "chevron-down"
                                            } else {
                                                "chevron-right"
                                            },
                                            12.,
                                            t.text_secondary,
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(position) = this
                                                .expanded_actions
                                                .iter()
                                                .position(|candidate| *candidate == index)
                                            {
                                                this.expanded_actions.remove(position);
                                            } else {
                                                this.expanded_actions.push(index);
                                            }
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .when(multiline && expanded, |row| {
                        row.child(div().font_family(crate::MONO).child(text(
                            command.to_owned(),
                            12.,
                            18.,
                            430.,
                            t.text,
                        )))
                    })
            }))
    }

    fn variables_button(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let popup = self.variables.then(|| {
            let content = popover(t, 320., |d| {
                d.child(text(
                    "Setup script environment variables",
                    14.,
                    20.,
                    500.,
                    t.text,
                ))
                .child(text(
                    "Source workspace path",
                    13.,
                    20.,
                    430.,
                    t.text_secondary,
                ))
                .child(text("CODEX_SOURCE_TREE_PATH", 12., 18., 500., t.text))
                .child(text("New worktree path", 13., 20., 430., t.text_secondary))
                .child(text("CODEX_WORKTREE_PATH", 12., 18., 500., t.text))
            });
            deferred(
                anchored()
                    .anchor(Anchor::TopRight)
                    .offset(point(px(0.), px(28.)))
                    .child(content),
            )
        });
        button("Variables", t.text_alpha_5, None, t)
            .id("setup-variables")
            .role(gpui::Role::Button)
            .aria_label("Setup script environment variables")
            .on_click(cx.listener(|this, _, _, cx| {
                this.variables = !this.variables;
                cx.notify();
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.variables = false;
                cx.notify();
            }))
            .children(popup)
    }

    fn script_section(&self, setup: bool, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let fields = self.fields.as_ref().expect("loaded editor fields");
        let index = if setup {
            fields.setup_platform
        } else {
            fields.cleanup_platform
        };
        let editor = if setup {
            &fields.setup[index]
        } else {
            &fields.cleanup[index]
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(text(
                        if setup {
                            "Setup script"
                        } else {
                            "Cleanup script"
                        },
                        14.,
                        21.,
                        500.,
                        t.text,
                    ))
                    .when(setup, |heading| heading.child(self.variables_button(t, cx))),
            )
            .child(text(
                if setup {
                    "Runs at the project root on worktree creation"
                } else {
                    "Runs at the project root before worktree cleanup"
                },
                12.,
                16.,
                430.,
                t.text_secondary,
            ))
            .child(self.platforms(setup, index, t, cx))
            .child(div().font_family(crate::MONO).child(editor.clone()))
    }

    fn action_menu(
        &self,
        menu: Menu,
        current: usize,
        labels: &[&'static str],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let opened = self.menu == Some(menu);
        let hover = t.list_hover;
        let rows = labels.iter().enumerate().map(|(index, label)| {
            div()
                .id(SharedString::from(format!("choice-{index}")))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(8.))
                .hover(move |s| s.bg(hover))
                .child(text(*label, 14., 20., 430., t.text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(fields) = &mut this.fields {
                        match menu {
                            Menu::Icon(row) => fields.actions[row].icon = index,
                            Menu::Platform(row) => fields.actions[row].platform = index,
                        }
                    }
                    this.menu = None;
                    cx.notify();
                }))
        });
        select(labels[current], None, false, false, t)
            .id(SharedString::from(format!(
                "action-menu-{}",
                match menu {
                    Menu::Icon(index) => index * 2,
                    Menu::Platform(index) => index * 2 + 1,
                }
            )))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.menu = if opened { None } else { Some(menu) };
                cx.notify();
            }))
            .children(opened.then(|| {
                deferred(
                    anchored()
                        .anchor(Anchor::TopLeft)
                        .offset(point(px(0.), px(28.)))
                        .child(popover(t, 200., |d| d.children(rows))),
                )
            }))
            .into_any_element()
    }

    fn editor(&self, state: &State, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let fields = self.fields.as_ref().expect("loaded editor fields");
        let value = fields.definition(cx);
        let valid = value["name"].as_str().is_some_and(|name| !name.is_empty())
            && value["actions"].as_array().is_some_and(|actions| {
                actions
                    .iter()
                    .all(|action| action["name"] != "" && action["command"] != "")
            });
        let changed = value != fields.initial
            || matches!(&state.selected, Load::Ready(data) if data["exists"] != true || data["environment"]["type"] == "error");
        let add = button("Add action", t.text_alpha_5, None, t)
            .id("add-environment-action")
            .on_click(cx.listener(|this, _, _, cx| {
                let action = ActionFields {
                    name: field("Name", "", 1, cx),
                    command: field("npm run dev", "", 4, cx),
                    icon: 0,
                    platform: 0,
                };
                if let Some(fields) = &mut this.fields {
                    fields.actions.push(action);
                }
                cx.notify();
            }));
        let setup = self.script_section(true, t, cx);
        let cleanup = self.script_section(false, t, cx);
        let actions = fields.actions.iter().enumerate().map(|(index, action)| {
            div()
                .relative()
                .p(px(12.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.border)
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(text("Name", 12., 16., 500., t.text_secondary))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(self.action_menu(
                            Menu::Icon(index),
                            action.icon,
                            &["Tool", "Run", "Debug", "Test"],
                            t,
                            cx,
                        ))
                        .child(div().flex_1().child(action.name.clone())),
                )
                .child(text("Action script", 12., 16., 500., t.text_secondary))
                .child(div().font_family(crate::MONO).child(action.command.clone()))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_end()
                        .child(
                            div()
                                .child(text("Platforms", 12., 16., 500., t.text_secondary))
                                .child(self.action_menu(
                                    Menu::Platform(index),
                                    action.platform,
                                    &["All platforms", "macOS", "Linux", "Windows"],
                                    t,
                                    cx,
                                )),
                        )
                        .child(
                            button("Delete", t.text_alpha_5, None, t)
                                .id(SharedString::from(format!("remove-action-{index}")))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(fields) = &mut this.fields {
                                        fields.actions.remove(index);
                                    }
                                    cx.notify();
                                })),
                        ),
                )
        });
        let save_label = if state.conflict {
            "Discard edits"
        } else if state.error.is_some() {
            "Retry save"
        } else {
            "Save"
        };
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(text("Name", 13., 18., 500., t.text))
            .child(fields.name.clone())
            .when(value["name"] == "", |d| {
                d.child(text("Enter an environment name", 12., 16., 430., t.text))
            })
            .child(setup)
            .child(cleanup)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Actions", 14., 21., 500., t.text))
                    .child(add),
            )
            .child(text(
                "These actions can run any command and will be displayed in the header",
                12.,
                16.,
                430.,
                t.text_secondary,
            ))
            .when(fields.actions.is_empty(), |d| {
                d.child(text(
                    "Add an action to run commands from the local toolbar",
                    13.,
                    20.,
                    430.,
                    t.text_secondary,
                ))
            })
            .children(actions)
            .children(
                state
                    .error
                    .as_ref()
                    .map(|error| text(error.clone(), 13., 20., 430., t.text)),
            )
            .child(
                div().flex().justify_end().child(
                    button(
                        if state.pending {
                            "Saving…"
                        } else {
                            save_label
                        },
                        t.text_alpha_5,
                        None,
                        t,
                    )
                    .id("save-environment")
                    .when(
                        (valid && changed || state.conflict) && !state.pending,
                        |button| {
                            button.on_click(cx.listener(|this, _, window, cx| {
                                let state = cx.global::<State>().clone();
                                let (Some(root), Some(path), Some(fields)) =
                                    (&this.root, &this.path, &this.fields)
                                else {
                                    return;
                                };
                                let action = if state.conflict {
                                    Action::Read { path: path.clone() }
                                } else {
                                    Action::Save {
                                        root: root.clone(),
                                        path: path.clone(),
                                        revision: match &state.selected {
                                            Load::Ready(value) => {
                                                value["revision"].as_str().map(str::to_owned)
                                            }
                                            _ => None,
                                        },
                                        definition: fields.definition(cx),
                                    }
                                };
                                if state.conflict {
                                    this.fields = None;
                                }
                                this.dispatch(action, window, cx);
                            }))
                        },
                    ),
                ),
            )
    }
}

impl Render for Environments {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        if state.saved_generation != self.generation {
            self.generation = state.saved_generation;
            self.editing = false;
            self.fields = None;
        }
        if self.root.is_none() {
            return self.projects(&state, &t, cx);
        }
        let back = button("Environments", t.text_alpha_5, None, &t)
            .id("environments-back")
            .on_click(cx.listener(|this, _, _, cx| {
                this.root = None;
                this.path = None;
                this.fields = None;
                this.menu = None;
                cx.notify();
            }));
        let mut view = div().flex().flex_col().gap(px(24.)).child(back);
        let data = match &state.selected {
            Load::Loading => {
                return view.child(text(
                    "Fetching your project configuration…",
                    14.,
                    20.,
                    430.,
                    t.text_secondary,
                ));
            }
            Load::Failed(error) => {
                let path = self.path.clone();
                return view
                    .child(text(error.clone(), 14., 20., 430., t.text))
                    .child(
                        button("Retry", t.text_alpha_5, None, &t)
                            .id("retry-environment-read")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(path) = &path {
                                    this.dispatch(Action::Read { path: path.clone() }, window, cx);
                                }
                            })),
                    );
            }
            Load::Ready(data) => data,
        };
        let definition = &data["environment"]["environment"];
        if self.editing {
            if self.fields.is_none() {
                let default = json!({"name": self.root.as_deref().and_then(|root| std::path::Path::new(root).file_name()).and_then(|name| name.to_str()).unwrap_or("local")});
                self.fields = Some(Fields::new(
                    if definition.is_null() {
                        &default
                    } else {
                        definition
                    },
                    cx,
                ));
            }
            return view
                .child(text("Edit local environment", 28., 34., 500., t.text))
                .child(self.editor(&state, &t, cx));
        }
        let exists = data["exists"] == true;
        view = view.child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(text(
                    definition["name"]
                        .as_str()
                        .unwrap_or("Local environment")
                        .to_owned(),
                    28.,
                    34.,
                    500.,
                    t.text,
                ))
                .child(
                    button(
                        if exists {
                            "Edit"
                        } else {
                            "Create local environment"
                        },
                        t.text_alpha_5,
                        None,
                        &t,
                    )
                    .id("edit-environment")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.editing = true;
                        cx.notify();
                    })),
                ),
        );
        if !exists {
            return view.child(text(
                "No local environment is configured for this project yet",
                14.,
                20.,
                430.,
                t.text_secondary,
            ));
        }
        if data["environment"]["type"] == "error" {
            return view.child(text(
                "Could not parse the local environment configuration",
                14.,
                20.,
                430.,
                t.text,
            ));
        }
        view.child(self.preview_script(true, &definition["setup"], &t, cx))
            .child(self.preview_script(false, &definition["cleanup"], &t, cx))
            .child(self.preview_actions(&definition["actions"], &t, cx))
    }
}
