//! The app's root view: routes, persisted window state, and the live `codex app-server`
//! connection that feeds the screens (Recents, model picker, threads, approvals, sign-in),
//! plus the key bindings and menu bar. `main.rs` only opens the window;
//! `examples/live_turn.rs` and `tests/flows.rs` drive the same view headless.

use anyhow::Result;
use async_channel::Receiver;
use bridge::types::{
    ApprovalDecision, CommandApprovalRequest, ConfigReadResponse, FileChangeApprovalRequest,
    Thread, ThreadItem, ThreadStartResponse, ThreadStartedNotification,
};
use bridge::{Bridge, Event};
use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Context, Entity, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, KeyBinding, Menu, MenuItem, OsAction,
    ParentElement as _, Pixels, Render, Styled as _, SystemMenuType, Task, Window,
    WindowAppearance, actions, div, px,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::{fs, path::PathBuf, sync::Arc};
use theme::Theme;
use ui::composer::{Approval, Composer, Permission, Submission};
use ui::screens::home::HomeMain;
use ui::screens::settings::{self, AppServer, Login};
use ui::screens::sidebar::{HomeSidebar, OpenThread};
use ui::screens::thread::view::{Clock, ThreadView};
use ui::{
    Copy, Cut, GoBack, GoForward, NewChat, OpenSettings, Paste, Redo, SelectAll, Shell,
    ToggleSidebar, Undo,
};

actions!(
    app,
    [
        CloseWindow,
        Minimize,
        Zoom,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        OpenDocumentation,
        /// The Recents row above the open thread (`previousThread`).
        PreviousChat,
        /// The Recents row below the open thread (`nextThread`).
        NextChat,
    ]
);

/// `thread1..9`: open the Nth Recents row (0-based).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = app, no_json)]
pub struct GoToChat(pub usize);

/// `(min-width: 768px)`: below it the original hides the sidebar panel.
const SIDEBAR_BREAKPOINT: Pixels = px(768.);

/// Key bindings (docs/inventory.md §12) and the app-level commands.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-n", NewChat, None),
        KeyBinding::new("cmd-shift-o", NewChat, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-shift-s", ToggleSidebar, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-]", GoForward, None),
        KeyBinding::new("cmd-shift-[", PreviousChat, None),
        KeyBinding::new("cmd-alt-left", PreviousChat, None),
        KeyBinding::new("cmd-shift-]", NextChat, None),
        KeyBinding::new("cmd-alt-right", NextChat, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    cx.bind_keys((1..=9).map(|n| KeyBinding::new(&format!("cmd-{n}"), GoToChat(n - 1), None)));
    ui::composer::bind_keys(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &OpenDocumentation, cx| {
        cx.open_url("https://developers.openai.com/codex/app")
    });
}

/// Menu bar of the original (docs/inventory.md §13), limited to commands with a handler.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Codex").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Codex", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Codex", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Chat", NewChat),
            MenuItem::separator(),
            MenuItem::action("Close", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Previous Chat", PreviousChat),
            MenuItem::action("Next Chat", NextChat),
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
        Menu::new("Help").items([MenuItem::action("Documentation", OpenDocumentation)]),
    ]
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Route {
    #[default]
    Home,
    Thread(String),
    Settings,
}

/// Settings ▸ Appearance ▸ Theme (the original's `appearanceTheme`): follow the system, or
/// force light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeSetting {
    /// The theme of a window whose system appearance is `appearance`.
    pub fn theme(self, appearance: WindowAppearance) -> Theme {
        match self {
            Self::System => theme_for(appearance),
            Self::Light => Theme::light(),
            Self::Dark => Theme::dark(),
        }
    }
}

/// Window size, sidebar, route and theme, persisted to `path` (none: not persisted).
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub width: f32,
    pub height: f32,
    pub sidebar_open: bool,
    pub route: Route,
    pub theme: ThemeSetting,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            width: 1280.,
            height: 832.,
            sidebar_open: true,
            route: Route::Home,
            theme: ThemeSetting::System,
            path: None,
        }
    }
}

impl State {
    /// `~/Library/Application Support/codex-gpui/state.json`, saved back on every change.
    pub fn load() -> Self {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/codex-gpui/state.json");
        let state: Self = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            ..state
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let result = fs::create_dir_all(path.parent().expect("state path has a parent"))
            .and_then(|()| fs::write(path, serde_json::to_vec_pretty(self)?));
        if let Err(err) = result {
            eprintln!("codex-gpui: cannot save {}: {err}", path.display());
        }
    }
}

pub fn theme_for(appearance: WindowAppearance) -> Theme {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::dark(),
        WindowAppearance::Light | WindowAppearance::VibrantLight => Theme::light(),
    }
}

/// Runs `task` to the end and prints its error.
fn log_err(task: Task<Result<()>>, cx: &App) {
    cx.foreground_executor()
        .spawn(async move {
            if let Err(err) = task.await {
                eprintln!("codex-gpui: {err:#}");
            }
        })
        .detach();
}

pub struct Root {
    /// Window size, sidebar and route, saved back on every change.
    state: State,
    back: Vec<Route>,
    forward: Vec<Route>,
    shell: Entity<Shell>,
    sidebar: Entity<HomeSidebar>,
    /// Recents thread ids in sidebar order, for `⌘1`…`⌘9` and Previous/Next Chat.
    recents: Vec<String>,
    /// The window is at least [`SIDEBAR_BREAKPOINT`] wide; narrower hides the sidebar.
    wide: bool,
    /// Holds key focus when no composer shows (Settings, a loading thread, sign-in), so
    /// shortcuts still reach the root's actions.
    focus: FocusHandle,
    home: Entity<HomeMain>,
    /// Docked under every open thread.
    thread_composer: Entity<Composer>,
    thread: Option<Entity<ThreadView>>,
    /// The Settings route's own shell: settings nav in the sidebar panel.
    settings: Option<Entity<Shell>>,
    /// Signed out: replaces the whole window.
    login: Option<Entity<Login>>,
    bridge: Arc<Bridge>,
    cwd: String,
    config: Value,
    skills: Value,
    ready: bool,
    /// Threads started or resumed by this app-server; others need `thread/resume` first.
    loaded: HashSet<String>,
    /// Id of the server request the composer's approval card answers.
    approval: Option<Value>,
    _events: Task<()>,
}

impl Root {
    /// `bridge` is a freshly spawned app-server; `cwd` is the project new threads start in.
    pub fn new(
        state: State,
        bridge: Bridge,
        events: Receiver<Event>,
        cwd: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let bridge = Arc::new(bridge);
        cx.set_global(AppServer(bridge.clone()));
        let this = cx.weak_entity();
        let home_composer = cx.new(|cx| {
            let mut composer = Composer::home(window, cx);
            let this = this.clone();
            composer.on_submit(move |s, window, cx| {
                this.update(cx, |this, cx| this.start_thread(s.clone(), window, cx))
                    .ok();
            });
            composer
        });
        let thread_composer = cx.new(|cx| {
            let mut composer = Composer::thread(window, cx);
            let send = this.clone();
            composer.on_submit(move |s, window, cx| {
                send.update(cx, |this, cx| this.send(s.clone(), window, cx))
                    .ok();
            });
            composer.on_approval(move |decision, _, cx| {
                this.update(cx, |this, _| this.answer(decision)).ok();
            });
            composer
        });
        let sidebar = cx.new(|_| HomeSidebar::new(&[], None));
        cx.subscribe_in(
            &sidebar,
            window,
            |this, _, open: &OpenThread, window, cx| {
                this.navigate(Route::Thread(open.0.clone()), window, cx)
            },
        )
        .detach();
        let main = cx.new(|_| ui::EmptyMain).into();
        let wide = window.viewport_size().width >= SIDEBAR_BREAKPOINT;
        let sidebar_open = state.sidebar_open && wide;
        let shell = cx.new(|_| {
            let mut shell = Shell::new(sidebar.clone().into(), main);
            shell.sidebar_open = sidebar_open;
            shell
        });
        cx.observe_in(&shell, window, |this, _, window, cx| {
            this.save(window, cx);
            cx.notify();
        })
        .detach();
        cx.observe_window_bounds(window, |this, window, cx| {
            this.fit(window, cx);
            this.save(window, cx);
        })
        .detach();
        cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx))
            .detach();
        let _events = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let root = Self {
            back: Vec::new(),
            forward: Vec::new(),
            shell,
            sidebar,
            recents: Vec::new(),
            wide,
            focus: cx.focus_handle(),
            home: cx.new(|_| HomeMain {
                composer: home_composer,
            }),
            thread_composer,
            thread: None,
            settings: None,
            login: None,
            bridge,
            cwd,
            config: Value::Null,
            skills: Value::Null,
            ready: false,
            loaded: HashSet::new(),
            approval: None,
            _events,
            state,
        };
        window.focus(&root.focus, cx);
        root.connect(window, cx);
        root
    }

    /// `initialize`, then account, Recents, models and config; shows the route once ready.
    fn connect(&self, window: &Window, cx: &mut Context<Self>) {
        let (bridge, cwd) = (self.bridge.clone(), self.cwd.clone());
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            bridge
                .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                .await?;
            let account = bridge.account_read().await?;
            let threads = bridge.thread_list().await?;
            let models = bridge.model_list().await?;
            let config: ConfigReadResponse = bridge
                .request("config/read", json!({ "includeLayers": false, "cwd": cwd }))
                .await?;
            // Only the settings page reads it (home directory), so a failure is not fatal.
            let skills = bridge
                .request("skills/list", json!({ "cwds": [cwd] }))
                .await
                .unwrap_or(Value::Null);
            this.update_in(cx, |this, window, cx| {
                if account.account.is_none() && account.requires_openai_auth {
                    this.login = Some(cx.new(Login::new));
                }
                this.set_threads(&threads.data, cx);
                let composers = [
                    this.home.read(cx).composer.clone(),
                    this.thread_composer.clone(),
                ];
                for composer in composers {
                    composer.update(cx, |c, cx| c.set_models(&models.data, &config.config, cx));
                }
                this.config = config.config;
                this.skills = skills;
                this.ready = true;
                this.sync(window, cx);
            })
        });
        log_err(task, cx);
    }

    /// `initialize` and the startup reads are done and the route is showing.
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// The open thread's view.
    pub fn thread(&self) -> Option<&Entity<ThreadView>> {
        self.thread.as_ref()
    }

    /// No account: the sign-in view replaces the window.
    pub fn signed_out(&self) -> bool {
        self.login.is_some()
    }

    pub fn route(&self) -> &Route {
        &self.state.route
    }

    /// The shell on screen: the Settings route's own, else the main one.
    pub fn shell(&self) -> &Entity<Shell> {
        self.settings.as_ref().unwrap_or(&self.shell)
    }

    /// The composer of the route: Home's, or the one docked under the open thread.
    pub fn composer(&self, cx: &App) -> Entity<Composer> {
        match self.state.route {
            Route::Thread(_) => self.thread_composer.clone(),
            _ => self.home.read(cx).composer.clone(),
        }
    }

    pub fn recents(&self) -> &[String] {
        &self.recents
    }

    fn set_threads(&mut self, threads: &[Thread], cx: &mut Context<Self>) {
        self.recents = threads.iter().map(|t| t.id.clone()).collect();
        self.sidebar.update(cx, |s, cx| s.set_threads(threads, cx));
    }

    /// Puts `thread` at the top of Recents.
    fn add_thread(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        self.recents.retain(|id| *id != thread.id);
        self.recents.insert(0, thread.id.clone());
        self.sidebar.update(cx, |s, cx| s.add_thread(thread, cx));
    }

    /// `⌘N`, the `New chat` row and rail Home: Home with an empty, focused composer.
    fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Home, window, cx);
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |c, cx| c.set_text("", cx));
        window.focus(&composer.focus_handle(cx), cx);
    }

    /// Opens the Recents row `delta` rows from the open thread; from any other route,
    /// the first row.
    fn step_chat(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let current = match &self.state.route {
            Route::Thread(id) => self.recents.iter().position(|r| r == id),
            _ => None,
        };
        let index = current.map_or(Some(0), |i| i.checked_add_signed(delta));
        self.go_to_chat(index, window, cx);
    }

    fn go_to_chat(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = index.and_then(|i| self.recents.get(i)) {
            self.navigate(Route::Thread(id.clone()), window, cx);
        }
    }

    /// Below [`SIDEBAR_BREAKPOINT`] the sidebar hides; wider again it shows as the user
    /// left it.
    fn fit(&mut self, window: &Window, cx: &mut Context<Self>) {
        let wide = window.viewport_size().width >= SIDEBAR_BREAKPOINT;
        if wide != self.wide {
            self.wide = wide;
            let open = wide && self.state.sidebar_open;
            self.shell.update(cx, |shell, cx| {
                shell.sidebar_open = open;
                cx.notify();
            });
        }
    }

    fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if route != self.state.route {
            self.back
                .push(std::mem::replace(&mut self.state.route, route));
            self.forward.clear();
            self.sync(window, cx);
        }
    }

    fn step(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (from, to) = if back {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        if let Some(route) = from.pop() {
            to.push(std::mem::replace(&mut self.state.route, route));
            self.sync(window, cx);
        }
    }

    /// Shows the route (loading its thread if needed), pushes history into the shell and
    /// persists the state.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (can_back, can_forward) = (!self.back.is_empty(), !self.forward.is_empty());
        self.settings = (self.state.route == Route::Settings).then(|| {
            let shell = settings::shell(&self.config, &self.skills, cx);
            shell.update(cx, |shell, _| {
                shell.can_go_back = can_back;
                shell.can_go_forward = can_forward;
            });
            shell
        });
        let thread_id = match &self.state.route {
            Route::Thread(id) => Some(id.clone()),
            _ => None,
        };
        // Keys need a focused element in the frame: the route's composer, else the root.
        let (main, focus): (AnyView, _) = match (&thread_id, &self.thread) {
            (Some(id), Some(view)) if view.read(cx).id() == id => {
                (view.clone().into(), self.thread_composer.focus_handle(cx))
            }
            (Some(id), _) => {
                self.load_thread(id.clone(), window, cx);
                (cx.new(|_| ui::EmptyMain).into(), self.focus.clone())
            }
            (None, _) if self.settings.is_some() => (self.home.clone().into(), self.focus.clone()),
            (None, _) => (
                self.home.clone().into(),
                self.home.read(cx).composer.focus_handle(cx),
            ),
        };
        // The sign-in view takes no keys of its own.
        window.focus(
            if self.login.is_some() {
                &self.focus
            } else {
                &focus
            },
            cx,
        );
        self.sidebar
            .update(cx, |s, cx| s.select(thread_id.as_deref(), cx));
        let home = self.state.route == Route::Home;
        self.shell.update(cx, |shell, cx| {
            shell.selected_rail = home.then_some("home");
            shell.can_go_back = can_back;
            shell.can_go_forward = can_forward;
            shell.set_main(main, cx);
        });
        self.save(window, cx);
        cx.notify();
    }

    /// `thread/read` with turns, then shows it if the route still points at it.
    fn load_thread(&self, id: String, window: &Window, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let read = bridge.thread_read(&id, true).await?;
            this.update_in(cx, |this, window, cx| {
                if this.state.route == Route::Thread(id) {
                    this.show_thread(read.thread, window, cx);
                }
            })
        });
        log_err(task, cx);
    }

    fn show_thread(&mut self, thread: Thread, window: &mut Window, cx: &mut Context<Self>) {
        let route = Route::Thread(thread.id.clone());
        let clock = Clock {
            utc_offset: chrono::Local::now().offset().local_minus_utc().into(),
            pinned_now: None,
        };
        let composer = self.thread_composer.clone().into();
        let view = cx.new(|_| ThreadView::new(thread, clock, composer));
        // The titlebar header reads the view.
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        self.thread = Some(view);
        if self.state.route == route {
            self.sync(window, cx);
        } else {
            self.navigate(route, window, cx);
        }
    }

    /// Home composer: `thread/start` with the chosen model and permissions, open it, then
    /// `turn/start`; the turn streams in through [`Self::on_event`].
    fn start_thread(&mut self, s: Submission, window: &mut Window, cx: &mut Context<Self>) {
        let (bridge, cwd) = (self.bridge.clone(), self.cwd.clone());
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let mut params = json!({ "cwd": cwd, "model": model(&s) });
            let (policy, sandbox, reviewer) = match s.permission {
                Permission::Ask => ("on-request", "workspace-write", "user"),
                Permission::ApproveForMe => ("on-request", "workspace-write", "auto_review"),
                Permission::FullAccess => ("never", "danger-full-access", "user"),
                // `config.toml` decides.
                Permission::Custom => ("", "", ""),
            };
            if !policy.is_empty() {
                params["approvalPolicy"] = policy.into();
                params["sandbox"] = sandbox.into();
                params["approvalsReviewer"] = reviewer.into();
            }
            let mut thread = bridge
                .request::<_, ThreadStartResponse>("thread/start", params)
                .await?
                .thread;
            // Title the new chat (Recents row, header) with its prompt until it has a preview.
            if thread.preview.is_empty() {
                thread.preview = s.text.clone();
            }
            let id = thread.id.clone();
            this.update_in(cx, |this, window, cx| {
                this.loaded.insert(id.clone());
                this.add_thread(&thread, cx);
                this.show_thread(thread, window, cx);
            })?;
            turn_start(&bridge, &id, &s).await
        });
        log_err(task, cx);
    }

    /// Thread composer: `turn/start` on the open thread, resuming it first if needed.
    fn send(&mut self, s: Submission, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let id = view.read(cx).id().to_owned();
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            if resume {
                bridge
                    .request::<_, Value>("thread/resume", json!({ "threadId": id }))
                    .await?;
                this.update(cx, |this, _| this.loaded.insert(id.clone()))?;
            }
            turn_start(&bridge, &id, &s).await
        });
        log_err(task, cx);
    }

    fn answer(&mut self, decision: ApprovalDecision) {
        if let Some(id) = self.approval.take()
            && let Err(err) = self.bridge.respond(id, json!({ "decision": decision }))
        {
            eprintln!("codex-gpui: approval: {err:#}");
        }
    }

    /// Every server message: streamed into the open thread and the sign-in view, plus
    /// Recents updates and approval prompts.
    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if let Some(view) = &self.thread {
            view.update(cx, |view, cx| view.apply(&event, cx));
        }
        if let Some(login) = &self.login {
            login.update(cx, |login, cx| login.apply(&event, cx));
        }
        match event {
            Event::Notification { method, params } => match method.as_str() {
                "thread/started" => {
                    if let Ok(n) = serde_json::from_value::<ThreadStartedNotification>(params) {
                        self.add_thread(&n.thread, cx);
                    }
                }
                // Recents rows show the thread preview, known once a turn ran.
                "turn/completed" => self.refresh_threads(cx),
                "account/login/completed" if params["success"] == true => {
                    self.login = None;
                    cx.notify();
                }
                _ => {}
            },
            // ponytail: other server requests (user input, permissions) stay unanswered;
            // answer them when their UI lands.
            Event::Request { id, method, params } => {
                let approval = match method.as_str() {
                    "item/commandExecution/requestApproval" => {
                        serde_json::from_value::<CommandApprovalRequest>(params)
                            .ok()
                            .map(|r| Approval::command(&r))
                    }
                    "item/fileChange/requestApproval" => serde_json::from_value::<
                        FileChangeApprovalRequest,
                    >(params)
                    .ok()
                    .map(|r| {
                        let paths: Vec<String> = self
                            .thread
                            .as_ref()
                            .and_then(|v| {
                                v.read(cx)
                                    .turns()
                                    .iter()
                                    .flat_map(|t| &t.items)
                                    .find_map(|i| match i {
                                        ThreadItem::FileChange { id, changes, .. }
                                            if *id == r.item_id =>
                                        {
                                            Some(changes.iter().map(|c| c.path.clone()).collect())
                                        }
                                        _ => None,
                                    })
                            })
                            .unwrap_or_default();
                        Approval::file_change(&r, &paths)
                    }),
                    _ => None,
                };
                if let Some(approval) = approval {
                    self.approval = Some(id);
                    self.thread_composer
                        .update(cx, |c, cx| c.set_approval(Some(approval), cx));
                }
            }
        }
    }

    fn refresh_threads(&self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let list = bridge.thread_list().await?;
            this.update(cx, |this, cx| this.set_threads(&list.data, cx))
        });
        log_err(task, cx);
    }

    /// Settings ▸ Appearance ▸ Theme: applies `theme` at once and persists it.
    /// ponytail: the native window chrome keeps following the system; force it with
    /// `cx.set_window_appearance` if a mismatched titlebar edge shows.
    pub fn set_theme(&mut self, theme: ThemeSetting, window: &mut Window, cx: &mut Context<Self>) {
        self.state.theme = theme;
        self.apply_theme(window, cx);
        self.save(window, cx);
    }

    fn apply_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.set_global(self.state.theme.theme(window.appearance()));
        window.refresh();
    }

    fn save(&mut self, window: &Window, cx: &App) {
        let size = window.window_bounds().get_bounds().size;
        self.state.width = size.width.into();
        self.state.height = size.height.into();
        // A narrow window hides the sidebar without changing the user's choice.
        if self.wide {
            self.state.sidebar_open = self.shell.read(cx).sidebar_open;
        }
        self.state.save();
    }
}

/// `None` (the server's default) when `model/list` gave the picker nothing.
fn model(s: &Submission) -> Option<&str> {
    (!s.model.is_empty()).then_some(s.model.as_str())
}

/// `turn/start` with the prompt, attachments, model, effort and Fast mode.
async fn turn_start(bridge: &Bridge, thread_id: &str, s: &Submission) -> Result<()> {
    const IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
    let (images, files): (Vec<_>, Vec<_>) = s.attachments.iter().partition(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGES.contains(&e.to_lowercase().as_str()))
    });
    // Non-image files go in as paths the agent can open.
    let text = files
        .iter()
        .fold(s.text.clone(), |text, p| format!("{text}\n{}", p.display()));
    let mut input = vec![json!({ "type": "text", "text": text, "text_elements": [] })];
    input.extend(
        images
            .iter()
            .map(|p| json!({ "type": "localImage", "path": p })),
    );
    let mut params = json!({
        "threadId": thread_id,
        "input": input,
        "model": model(s),
        "effort": s.effort,
    });
    if s.fast {
        params["serviceTier"] = "priority".into();
    }
    bridge.request::<_, Value>("turn/start", params).await?;
    Ok(())
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = match (&self.login, &self.settings) {
            (Some(login), _) => login.clone().into_any_element(),
            (None, Some(settings)) => settings.clone().into_any_element(),
            (None, None) => self.shell.clone().into_any_element(),
        };
        // The open thread's titlebar header (open-sidebar layout only).
        let header = match (&self.state.route, &self.thread) {
            (Route::Thread(_), Some(view))
                if self.login.is_none() && self.shell.read(cx).sidebar_open =>
            {
                Some(view.read(cx).titlebar_header(cx))
            }
            _ => None,
        };
        div()
            .size_full()
            .relative()
            .font_family(ui::SANS)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.navigate(Route::Settings, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.step(false, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousChat, window, cx| this.step_chat(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextChat, window, cx| this.step_chat(1, window, cx)))
            .on_action(
                cx.listener(|this, a: &GoToChat, window, cx| {
                    this.go_to_chat(Some(a.0), window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.shell.update(cx, |shell, cx| shell.toggle_sidebar(cx))
            }))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .child(content)
            .children(header)
    }
}
