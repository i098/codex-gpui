use crate::{Root, Route, host_settings};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bridge::{Bridge, Event};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
    canvas, div, prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use theme::ActiveTheme as _;
use ui::{
    Shell,
    screens::terminal::{TerminalEvent, TerminalStyle, TerminalSurface},
    text,
};

static NEXT_PROCESS: AtomicU64 = AtomicU64::new(1);
const HOME_TERMINAL: &str = "home";
gpui::actions!(terminal, [ToggleBottomPanel, OpenTerminal]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-j", ToggleBottomPanel, None),
        KeyBinding::new("ctrl-`", OpenTerminal, None),
    ]);
}

pub(crate) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &ToggleBottomPanel, window, cx| {
        this.toggle_terminal(window, cx);
    }))
    .on_action(cx.listener(|this, _: &OpenTerminal, window, cx| {
        this.toggle_terminal(window, cx);
    }))
}

#[derive(Default)]
pub struct Terminals {
    panels: HashMap<String, Entity<TerminalPanel>>,
    visible: bool,
}

pub(crate) fn source_directories(state: Option<&Value>, thread: &str, cwd: &str) -> Vec<String> {
    let roots = state.and_then(|state| {
        let projects = state["local-projects"].as_object()?;
        let project = if thread == HOME_TERMINAL {
            projects.values().find(|project| {
                project["rootPaths"]
                    .as_array()
                    .is_some_and(|roots| roots.iter().any(|root| root.as_str() == Some(cwd)))
            })?
        } else {
            let assignment = &state["thread-project-assignments"][thread];
            if assignment["projectKind"] != "local" {
                return None;
            }
            projects.get(assignment["projectId"].as_str()?)?
        };
        project["rootPaths"].as_array()
    });
    let roots: Vec<String> = roots
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|path| std::path::Path::new(path).is_absolute())
        .map(str::to_owned)
        .collect();
    if roots.is_empty() {
        vec![cwd.to_owned()]
    } else {
        roots
    }
}

impl Root {
    pub fn terminal_panel(&self) -> Option<&Entity<TerminalPanel>> {
        let id = match &self.state.route {
            Route::Home => HOME_TERMINAL,
            Route::Thread(id) => id,
            _ => return None,
        };
        self.terminal.panels.get(id)
    }

    fn terminal_target(&self, cx: &App) -> Option<(String, String)> {
        match &self.state.route {
            Route::Home => Some((
                HOME_TERMINAL.into(),
                self.state
                    .project_root
                    .clone()
                    .unwrap_or_else(|| self.cwd.clone()),
            )),
            Route::Thread(id) => self
                .thread
                .as_ref()
                .filter(|thread| thread.read(cx).id() == id)
                .map(|thread| (id.clone(), thread.read(cx).cwd().to_owned())),
            _ => None,
        }
    }

    pub(crate) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, cwd)) = self.terminal_target(cx) else {
            return;
        };
        if self.terminal.visible
            && self
                .terminal_panel()
                .is_some_and(|p| !p.read(cx).sessions.is_empty())
        {
            self.terminal.visible = false;
            self.sync_terminal(cx);
            self.composer(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        self.terminal.visible = true;
        if !self.terminal.panels.contains_key(&id) {
            let bridge = self.bridge.clone();
            let panel = cx.new(|cx| TerminalPanel::new(bridge, cwd, cx));
            cx.subscribe(&panel, |root, _, _: &PanelChanged, cx| {
                root.sync_terminal(cx)
            })
            .detach();
            self.terminal.panels.insert(id, panel);
        }
        if let Some(panel) = self.terminal_panel().cloned()
            && panel.read(cx).sessions.is_empty()
        {
            panel.update(cx, |panel, cx| panel.open(cx));
        }
        self.sync_terminal(cx);
        if let Some(surface) = self
            .terminal_panel()
            .and_then(|panel| panel.read(cx).surface().cloned())
        {
            surface.focus_handle(cx).focus(window, cx);
        }
    }

    pub(crate) fn sync_terminal(&mut self, cx: &mut Context<Self>) {
        let theme_key = if cx.theme().appearance == theme::Appearance::Dark {
            "appearanceDarkChromeTheme"
        } else {
            "appearanceLightChromeTheme"
        };
        let (size, face) = cx
            .try_global::<host_settings::Settings>()
            .map(|settings| {
                let values = settings.read_all();
                let size = values.get("codeFontSize").and_then(Value::as_f64);
                let fonts = values
                    .get(theme_key)
                    .map_or(Value::Null, |v| v["fonts"].clone());
                let face = fonts["codeFace"]["postscriptName"]
                    .as_str()
                    .or_else(|| {
                        fonts["code"]
                            .as_str()
                            .and_then(|name| name.split(',').next())
                    })
                    .map(|name| name.trim().trim_matches(['\"', '\'']).to_owned());
                (size, face)
            })
            .unwrap_or_default();
        ui::set_code_font(
            face.as_deref()
                .filter(|name| !name.is_empty() && !matches!(*name, "monospace" | "ui-monospace"))
                .unwrap_or(ui::MONO),
        );
        let mut style = TerminalStyle::default();
        if let Some(value) = size
            && value.is_finite()
            && (6.0..=72.0).contains(&value)
        {
            style.size = value as f32;
        }
        if let Some(name) = face
            && !name.is_empty()
            && !matches!(name.as_str(), "monospace" | "ui-monospace")
        {
            style.font = name.into();
        }
        cx.set_global(style);
        let panel = self.terminal_panel().cloned();
        if let Some(panel) = &panel
            && let Some((id, cwd)) = self.terminal_target(cx)
            && (panel.read(cx).cwd != cwd || !panel.read(cx).workspaces_loaded)
        {
            panel.update(cx, |panel, cx| {
                panel.set_workspace(cwd.clone(), vec![cwd.clone()], cx);
                panel.workspaces_loaded = true;
            });
            let host = self.host.clone();
            let expected_cwd = cwd.clone();
            let target = panel.clone();
            let read = cx.background_spawn(async move { host.terminal_workspaces(&id, &cwd) });
            cx.spawn(async move |_, cx| {
                match read.await {
                    Ok(roots) => target.update(cx, |panel, cx| {
                        if panel.cwd == expected_cwd {
                            panel.set_workspace(expected_cwd, roots, cx);
                        }
                    }),
                    Err(error) => {
                        eprintln!("codex-gpui: cannot read terminal source directories: {error:#}");
                    }
                };
            })
            .detach();
        }
        let bottom = panel
            .filter(|panel| self.terminal.visible && !panel.read(cx).sessions.is_empty())
            .map(Into::into);
        let docked = bottom.is_some();
        if let Some(thread) = &self.thread {
            thread.update(cx, |thread, _| thread.docked_below = docked);
        }
        let shell = if self.sidebar_automation.is_some() {
            self.page.as_ref().unwrap_or(&self.shell)
        } else {
            &self.shell
        };
        shell.update(cx, |shell: &mut Shell, cx| {
            shell.bottom = bottom;
            cx.notify();
        });
    }

    pub(crate) fn terminal_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        for panel in self.terminal.panels.values() {
            panel.update(cx, |panel, cx| panel.apply(event, cx));
        }
    }

    pub(crate) fn close_focused_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self
            .terminal_panel()
            .cloned()
            .filter(|p| p.read(cx).focused(window, cx))
        else {
            return false;
        };
        panel.update(cx, |panel, cx| panel.close(panel.active, cx));
        self.sync_terminal(cx);
        true
    }
}

struct Session {
    id: String,
    cwd: String,
    surface: Entity<TerminalSurface>,
    error: Option<String>,
    exited: bool,
    warning_dismissed: bool,
    input: async_channel::Sender<Value>,
    ready: bool,
    pending: Vec<Value>,
}

pub struct PanelChanged;
impl EventEmitter<PanelChanged> for TerminalPanel {}

pub struct TerminalPanel {
    bridge: Arc<Bridge>,
    sessions: Vec<Session>,
    active: usize,
    hovered: Option<usize>,
    strip_width: f32,
    strip_left: f32,
    cwd: String,
    roots: Vec<String>,
    shell: String,
    workspaces_loaded: bool,
    focus_pending: bool,
}

impl TerminalPanel {
    fn new(bridge: Arc<Bridge>, cwd: String, cx: &mut Context<Self>) -> Self {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| s.starts_with('/'))
            .unwrap_or_else(|| "/bin/sh".into());
        let mut panel = Self {
            bridge,
            sessions: Vec::new(),
            active: 0,
            hovered: None,
            strip_width: 0.,
            strip_left: 0.,
            cwd: cwd.clone(),
            roots: vec![cwd],
            shell,
            workspaces_loaded: false,
            focus_pending: false,
        };
        panel.open(cx);
        panel
    }

    pub fn set_workspace(&mut self, cwd: String, roots: Vec<String>, cx: &mut Context<Self>) {
        if self.cwd != cwd || self.roots != roots {
            self.cwd = cwd;
            self.roots = roots;
            cx.notify();
        }
    }

    pub fn surface(&self) -> Option<&Entity<TerminalSurface>> {
        self.sessions.get(self.active).map(|s| &s.surface)
    }
    pub fn error(&self) -> Option<&str> {
        self.sessions
            .get(self.active)
            .and_then(|s| s.error.as_deref())
    }
    pub fn process_id(&self) -> Option<&str> {
        self.sessions.get(self.active).map(|s| s.id.as_str())
    }

    pub fn workspace_warning(&self) -> bool {
        self.sessions.get(self.active).is_some_and(|session| {
            !session.warning_dismissed
                && session.cwd != self.cwd
                && !self
                    .roots
                    .iter()
                    .any(|root| std::path::Path::new(&session.cwd).starts_with(root))
        })
    }

    fn focused(&self, window: &Window, cx: &App) -> bool {
        self.surface()
            .is_some_and(|surface| surface.focus_handle(cx).is_focused(window))
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(surface) = self.surface() {
            surface.focus_handle(cx).focus(window, cx);
        }
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let id = format!(
            "gpui-terminal-{}-{}",
            std::process::id(),
            NEXT_PROCESS.fetch_add(1, Ordering::Relaxed)
        );
        let surface = cx.new(|cx| TerminalSurface::new(true, cx));
        let process = id.clone();
        cx.subscribe(&surface, move |panel, _, event: &TerminalEvent, cx| {
            let Some(index) = panel.sessions.iter().position(|s| s.id == process) else {
                return;
            };
            match event {
                TerminalEvent::Input(bytes) => panel.send(
                    index,
                    "command/exec/write",
                    json!({"deltaBase64":STANDARD.encode(bytes)}),
                    cx,
                ),
                TerminalEvent::Resize { rows, cols } => panel.send(
                    index,
                    "command/exec/resize",
                    json!({"size":{"rows":rows,"cols":cols}}),
                    cx,
                ),
                TerminalEvent::New => panel.open(cx),
                TerminalEvent::Directory(number) => panel.change_directory(*number, cx),
            }
        })
        .detach();
        let (input, receiver) = async_channel::unbounded::<Value>();
        let bridge = self.bridge.clone();
        let input_id = id.clone();
        cx.spawn(async move |panel, cx| {
            while let Ok(mut request) = receiver.recv().await {
                let method = request["method"]
                    .take()
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if let Err(error) = bridge.request::<_, Value>(&method, request).await {
                    panel
                        .update(cx, |panel, cx| {
                            panel.set_error(&input_id, error.to_string(), cx)
                        })
                        .ok();
                }
            }
        })
        .detach();
        let bridge = self.bridge.clone();
        let params = json!({"command":[self.shell,"-l"],"cwd":self.cwd,"processId":id,
            "tty":true,"disableTimeout":true,"disableOutputCap":true,"size":{"rows":24,"cols":80},
            "env":{"TERM":"xterm-256color"}});
        self.active = self.sessions.len();
        self.sessions.push(Session {
            id: id.clone(),
            cwd: self.cwd.clone(),
            surface,
            error: None,
            exited: false,
            warning_dismissed: false,
            input,
            ready: false,
            pending: Vec::new(),
        });
        cx.spawn(async move |panel, cx| {
            let result = bridge.request::<_, Value>("command/exec", params).await;
            panel
                .update(cx, |panel, cx| {
                    if let Some(session) = panel.sessions.iter_mut().find(|s| s.id == id) {
                        session.exited = true;
                        session.surface.update(cx, |surface, cx| {
                            surface.writable = false;
                            cx.notify();
                        });
                        if let Err(error) = result {
                            session.error = Some(error.to_string());
                        }
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
        self.focus_pending = true;
        cx.emit(PanelChanged);
        cx.notify();
    }

    fn send(&mut self, index: usize, method: &str, mut params: Value, _: &mut Context<Self>) {
        if let Some(session) = self.sessions.get_mut(index).filter(|s| !s.exited) {
            params["processId"] = session.id.clone().into();
            params["method"] = method.into();
            if session.ready {
                let _ = session.input.try_send(params);
            } else {
                session.pending.push(params);
            }
        }
    }

    fn set_error(&mut self, id: &str, message: String, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.error = Some(message);
            cx.notify();
        }
    }

    fn apply(&mut self, event: &Event, cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        if method != "command/exec/outputDelta" {
            return;
        }
        let Some(session) = self
            .sessions
            .iter_mut()
            .find(|s| params["processId"] == s.id)
        else {
            return;
        };
        match params["deltaBase64"]
            .as_str()
            .and_then(|data| STANDARD.decode(data).ok())
        {
            Some(bytes) => {
                session.ready = true;
                for request in session.pending.drain(..) {
                    let _ = session.input.try_send(request);
                }
                session
                    .surface
                    .update(cx, |surface, cx| surface.append(&bytes, cx));
            }
            None => session.error = Some("The terminal returned invalid output".into()),
        }
        cx.notify();
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.sessions.len() {
            return;
        }
        let session = self.sessions.remove(index);
        if !session.exited {
            let bridge = self.bridge.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = bridge
                        .request::<_, Value>(
                            "command/exec/terminate",
                            json!({"processId":session.id}),
                        )
                        .await
                    {
                        eprintln!("codex-gpui: cannot close terminal: {error:#}");
                    }
                })
                .detach();
        }
        self.active = if index < self.active {
            self.active - 1
        } else {
            self.active
        };
        self.active = self.active.min(self.sessions.len().saturating_sub(1));
        cx.emit(PanelChanged);
        cx.notify();
    }

    fn change_directory(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(root) = self.roots.get(index).cloned() else {
            return;
        };
        let Some(session) = self.sessions.get(self.active) else {
            return;
        };
        if session.surface.read(cx).interacted {
            return;
        }
        session.surface.update(cx, |surface, cx| {
            surface.interacted = true;
            cx.notify();
        });
        let command = format!("cd -- '{}'\r", root.replace('\'', "'\\''"));
        self.send(
            self.active,
            "command/exec/write",
            json!({"deltaBase64":STANDARD.encode(command)}),
            cx,
        );
        cx.notify();
    }
}

#[derive(Clone)]
enum PanelAction {
    Activate(usize),
    New,
    Close(usize),
    Hide,
    Dismiss,
    Reload,
    Directory(usize),
}

impl TerminalPanel {
    fn perform(&mut self, action: &PanelAction, window: &mut Window, cx: &mut Context<Self>) {
        match *action {
            PanelAction::Activate(index) if index < self.sessions.len() => self.active = index,
            PanelAction::New => self.open(cx),
            PanelAction::Close(index) => self.close(index, cx),
            PanelAction::Hide => {
                window.dispatch_action(Box::new(ToggleBottomPanel), cx);
                return;
            }
            PanelAction::Dismiss => {
                if let Some(session) = self.sessions.get_mut(self.active) {
                    session.warning_dismissed = true;
                }
            }
            PanelAction::Directory(index) => self.change_directory(index, cx),
            PanelAction::Reload => {
                if self
                    .sessions
                    .get(self.active)
                    .is_some_and(|session| session.exited)
                {
                    self.close(self.active, cx);
                    self.open(cx);
                } else if let Some(session) = self.sessions.get_mut(self.active) {
                    session.error = None;
                }
            }
            PanelAction::Activate(_) => {}
        }
        self.focus(window, cx);
        cx.notify();
    }
}

fn control(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    action: PanelAction,
    window: &mut Window,
    cx: &mut Context<TerminalPanel>,
) -> gpui::Stateful<gpui::Div> {
    let id = id.into();
    let label = label.into();
    let focus = ui::focus::handle(id.clone(), 0, window, cx);
    let key_action = action.clone();
    let button = div()
        .id(id)
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .when(!label.is_empty(), |d| {
            d.child(text(label, 12., 20., 400., cx.theme().text))
        })
        .on_click(cx.listener(move |panel, _, window, cx| {
            panel.perform(&action, window, cx);
            cx.stop_propagation();
        }))
        .on_key_down(
            cx.listener(move |panel, event: &gpui::KeyDownEvent, window, cx| {
                ui::focus::tab(event, window, cx);
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    panel.perform(&key_action, window, cx);
                    cx.stop_propagation();
                }
            }),
        );
    ui::focus::ring(button, &focus, px(6.), px(0.), cx)
}

impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_pending) {
            self.focus(window, cx);
        }
        let theme = cx.theme().clone();
        let available = (self.strip_width - 89.).round();
        let slot_width = if self.strip_width == 0. {
            240.
        } else {
            (available / self.sessions.len().max(1) as f32).clamp(90., 240.)
        };
        let tab_width = slot_width - 2.;
        let anchored = slot_width * self.sessions.len() as f32 >= available;
        let measure = cx.entity();
        let measure = canvas(
            move |bounds, _, cx| {
                measure.update(cx, |panel, cx| {
                    let width = f32::from(bounds.size.width);
                    let left = f32::from(bounds.origin.x);
                    if (panel.strip_left - left).abs() > 0.01 {
                        panel.strip_left = left;
                        cx.notify();
                    }
                    if (panel.strip_width - width).abs() > 0.1 {
                        panel.strip_width = width;
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let mut tabs = div()
            .h(px(40.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(8.))
            .pr(px(6.))
            .border_b_1()
            .border_color(theme.border);
        let mut items = div()
            .id("terminal-tabs")
            .h_full()
            .min_w_0()
            .pr(px(2.))
            .flex()
            .items_center()
            .gap(px(2.))
            .overflow_x_scroll();
        for (index, session) in self.sessions.iter().enumerate() {
            let selected = index == self.active;
            let hovered = self.hovered == Some(index);
            let name = std::path::Path::new(&session.cwd)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let label = if self.sessions.len() == 1 {
                name.into_owned()
            } else {
                format!("{name} {}", index + 1)
            };
            let content = ui::screens::terminal::panel_tab(
                label.clone().into(),
                selected,
                hovered,
                tab_width,
                window,
                cx,
            );
            let mut tab = control(
                ("terminal-tab", index),
                "",
                PanelAction::Activate(index),
                window,
                cx,
            )
            .role(gpui::Role::Tab)
            .aria_label(label.clone())
            .relative()
            .w(px(tab_width))
            .h(px(32.))
            .px_0()
            .flex_shrink_0()
            .child(content)
            .on_hover(cx.listener(move |panel, hovered: &bool, _, cx| {
                panel.hovered = hovered.then_some(index);
                cx.notify();
            }));
            if selected || hovered {
                tab = tab.child(
                    control(
                        ("close-terminal", index),
                        "",
                        PanelAction::Close(index),
                        window,
                        cx,
                    )
                    .aria_label(format!("Close {label} tab"))
                    .absolute()
                    .top(px(6.))
                    .right(px(7.))
                    .size(px(20.))
                    .px_0()
                    .justify_center()
                    .child(ui::icon::icon(
                        "diff-tab-close",
                        14.,
                        theme.text_tertiary,
                    )),
                );
            }
            let slot_origin = self.strip_left + 8. + slot_width * index as f32;
            let slot_snap = (slot_origin * 2. + 0.5).floor() / 2. - slot_origin;
            if index + 1 < self.sessions.len()
                && !selected
                && !hovered
                && self.active != index + 1
                && self.hovered != Some(index + 1)
            {
                tab = tab.child(
                    div()
                        .absolute()
                        // Chromium snaps the slot's origin to the device grid, then adds the
                        // divider's offset inside it.
                        .right(px(-2. - slot_snap))
                        .top(px(10.))
                        .w(px(1.))
                        .h(px(12.))
                        .bg(theme.border),
                );
            }
            items = items.child(tab);
        }
        tabs = tabs
            .child(
                div()
                    .min_w_0()
                    .h_full()
                    .flex_1()
                    .flex()
                    .items_center()
                    .child(items)
                    .child(
                        control("new-terminal", "", PanelAction::New, window, cx)
                            .aria_label("Open bottom panel tab")
                            .ml(px(6.))
                            .size(px(28.))
                            .rounded(theme::Theme::RADIUS_LG)
                            .px_0()
                            .flex_shrink_0()
                            .justify_center()
                            .child(ui::icon::icon("diff-new-tab", 16., theme.text_tertiary)),
                    ),
            )
            .child(
                control("hide-terminal", "", PanelAction::Hide, window, cx)
                    .aria_label("Hide bottom panel")
                    .relative()
                    .when(anchored, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(-7.))
                                .top(px(6.))
                                .w(px(1.))
                                .h(px(16.))
                                .bg(theme.border),
                        )
                    })
                    .ml(px(7.))
                    .size(px(28.))
                    .rounded(theme::Theme::RADIUS_LG)
                    .px_0()
                    .flex_shrink_0()
                    .justify_center()
                    .child(ui::icon::icon(
                        "chevron-down-medium",
                        16.,
                        theme.text_tertiary,
                    )),
            );
        let mut panel = div()
            .id("interactive-terminal-panel")
            .relative()
            .h(px(280.))
            .min_h(px(120.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .child(measure)
            .child(tabs);
        let Some(session) = self.sessions.get(self.active) else {
            return panel;
        };
        if self.workspace_warning() {
            let foreground = if theme.appearance == theme::Appearance::Dark {
                gpui::rgb(0x2d2d2d).into()
            } else {
                theme.text_inverse
            };
            let actions = div()
                .flex()
                .gap(px(8.))
                .flex_shrink_0()
                .when(self.strip_width < 448., |d| {
                    d.w_full().flex_wrap().justify_center()
                })
                .child(
                    control(
                        "dismiss-terminal-warning",
                        "",
                        PanelAction::Dismiss,
                        window,
                        cx,
                    )
                    .aria_label("Dismiss")
                    .px(px(6.))
                    .rounded_full()
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .child(text("Dismiss", 13., 18., 430., theme.text)),
                )
                .child(
                    control("new-workspace-terminal", "", PanelAction::New, window, cx)
                        .aria_label("Open new terminal")
                        .px(px(6.))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.button_primary_solid)
                        .child(text("Open new terminal", 13., 18., 430., foreground)),
                );
            panel = panel.child(ui::screens::terminal::workspace_warning(
                actions,
                self.strip_width < 608.,
                cx,
            ));
        }
        if session.error.is_some() {
            return panel.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.))
                            .child(text(
                                "The terminal encountered an error",
                                16.,
                                24.,
                                500.,
                                theme.text,
                            ))
                            .child(text(
                                "Try reloading the terminal to continue",
                                13.,
                                18.5714,
                                430.,
                                theme.text_secondary,
                            )),
                    )
                    .child(
                        control("reload-terminal", "", PanelAction::Reload, window, cx)
                            .aria_label("Reload")
                            .h(px(24.))
                            .rounded_full()
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .bg(theme.text_alpha_5)
                            .child(text("Reload", 13., 18., 430., theme.text)),
                    ),
            );
        }
        let mut body = div()
            .relative()
            .min_h_0()
            .flex_1()
            .pl(px(16.))
            .pt(px(8.))
            .pb(px(12.))
            .child(session.surface.clone());
        if !session.exited && self.roots.len() > 1 && !session.surface.read(cx).interacted {
            let font = cx
                .try_global::<TerminalStyle>()
                .map(|style| style.font.clone())
                .unwrap_or_else(|| ui::MONO.into());
            let mut color = if theme.appearance == theme::Appearance::Dark {
                gpui::rgb(0xdfdfdf).into()
            } else {
                theme.text
            };
            color.a = 0.65;
            let hint = div()
                .absolute()
                .left(px(16.))
                .right(px(16.))
                .top(px(24.4))
                .pt(px(8.))
                .font_family(font)
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text("Source directories", 13., 20., 430., color));
            let mut folders = div().flex().flex_col().items_start().gap(px(4.));
            for (index, root) in self.roots.iter().enumerate() {
                let name = std::path::Path::new(root)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                folders = folders.child(
                    control(
                        ("terminal-root", index),
                        "",
                        PanelAction::Directory(index),
                        window,
                        cx,
                    )
                    .aria_label(format!("{} {name}", index + 1))
                    .h(px(20.))
                    .px_0()
                    .max_w_full()
                    .min_w_0()
                    .gap(px(12.))
                    .when(*root == session.cwd, |d| d.underline())
                    .hover(|d| d.underline())
                    .child(
                        text((index + 1).to_string(), 13., 20., 430., color)
                            .w(px(16.))
                            .text_right(),
                    )
                    .child(
                        text(name.into_owned(), 13., 20., 430., color)
                            .min_w_0()
                            .text_ellipsis(),
                    ),
                );
            }
            body = body.child(hint.child(folders).child(text(
                if cfg!(target_os = "macos") {
                    "Press ⌥ Opt + Number to change directory"
                } else {
                    "Press Alt + Number to change directory"
                },
                13.,
                20.,
                430.,
                color,
            )));
        }
        panel = panel.child(body);
        if session.exited {
            panel =
                panel.child(text("Shell exited", 12., 24., 400., theme.text_secondary).px(px(8.)));
        }
        panel
    }
}

impl Drop for TerminalPanel {
    fn drop(&mut self) {
        let ids: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| !s.exited)
            .map(|s| s.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        let bridge = self.bridge.clone();
        std::thread::spawn(move || {
            for id in ids {
                if let Err(error) = pollster::block_on(
                    bridge.request::<_, Value>("command/exec/terminate", json!({"processId": id})),
                ) {
                    eprintln!("codex-gpui: cannot close terminal: {error:#}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::source_directories;
    use serde_json::json;

    #[test]
    fn terminal_directories_stay_with_the_assigned_project() {
        let state = json!({
            "thread-project-assignments": {"chat": {"projectKind":"local","projectId":"selected"}},
            "local-projects": {
                "selected": {"rootPaths":["/project/one","/project/two"]},
                "unrelated": {"rootPaths":["/private/project"]}
            }
        });
        assert_eq!(
            source_directories(Some(&state), "chat", "/project/one"),
            ["/project/one", "/project/two"]
        );
        assert_eq!(
            source_directories(Some(&state), "home", "/project/one"),
            ["/project/one", "/project/two"]
        );
        assert_eq!(
            source_directories(Some(&state), "other", "/project/current"),
            ["/project/current"]
        );
    }
}
