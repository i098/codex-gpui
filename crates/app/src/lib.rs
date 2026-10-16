//! The app's root view: routes, persisted window state, and the live `codex app-server`
//! connection that feeds the screens (Recents, model picker, threads, approvals, sign-in),
//! plus the key bindings and menu bar. `main.rs` only opens the window;
//! `examples/live_turn.rs` and `tests/flows.rs` drive the same view headless.

mod features;
pub mod host;
pub mod host_settings;

use anyhow::{Context as _, Result};
use async_channel::Receiver;
use bridge::types::{
    ApprovalDecision, CommandApprovalRequest, ConfigReadResponse, FileChangeApprovalRequest,
    Thread, ThreadItem, ThreadReadResponse, ThreadStartResponse, ThreadStartedNotification,
    TurnStatus,
};
use bridge::{Bridge, Event};
use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, AsyncWindowContext, Context, Entity,
    FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyBinding, Menu, MenuItem,
    OsAction, ParentElement as _, Pixels, Render, SharedString, Styled as _, SystemMenuType, Task,
    WeakEntity, Window, WindowAppearance, actions, div, px,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::{fs, path::PathBuf, sync::Arc};
use theme::Theme;
use ui::composer::{Approval, Composer, Permission, Submission};
use ui::dialog::{Dialog, Kind as DialogKind, Outcome as DialogOutcome};
use ui::screens::diff::panel::{
    ActivateSideTab, CloseSideTab, DiffPanel, FileView, OpenFile, SplitScreen, Tab, with_side_tabs,
};
use ui::screens::home::HomeMain;
use ui::screens::settings::{self, AppServer, Login, SettingChanged};
use ui::screens::sidebar::{
    ArchiveThread, CommandMenu, CommandMenuEvent, HomeSidebar, OpenThread, UnpinThread,
};
use ui::screens::terminal::Xterm;
use ui::screens::thread::view::{Clock, EditMessage, ForkThread, RevertTurn, ThreadView};
use ui::screens::{automations, mobile};
use ui::toast::{Toast, Toaster};
use ui::{
    Copy, Cut, GoBack, GoForward, NewChat, OpenBackgroundTerminal, OpenChangesPanel,
    OpenCommandMenu, OpenMobileSetup, OpenScheduled, OpenSettings, Paste, Redo, SelectAll,
    SetTheme, Shell, ShowKeyboardShortcuts, ToggleChangesPanel, ToggleSidebar, Undo,
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
        FindInThread,
        FindNext,
        FindPrevious,
        /// The Recents row above the open thread (`previousThread`).
        PreviousChat,
        /// The Recents row below the open thread (`nextThread`).
        NextChat,
        /// `⌘⇧A`: archive the open chat (`archiveThread`).
        ArchiveChat,
    ]
);

/// `thread1..9`: open the Nth Recents row (0-based).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = app, no_json)]
pub struct GoToChat(pub usize);

/// `recentThread1..6`: open the Nth most recently updated chat (0-based).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = app, no_json)]
pub struct GoToRecentChat(pub usize);

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
        KeyBinding::new("cmd-/", ShowKeyboardShortcuts, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-f", FindInThread, None),
        KeyBinding::new("cmd-g", FindNext, None),
        KeyBinding::new("cmd-shift-g", FindPrevious, None),
        KeyBinding::new("cmd-shift-s", ToggleSidebar, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-]", GoForward, None),
        KeyBinding::new("cmd-shift-[", PreviousChat, None),
        KeyBinding::new("cmd-alt-left", PreviousChat, None),
        KeyBinding::new("cmd-shift-]", NextChat, None),
        KeyBinding::new("cmd-alt-right", NextChat, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-k", OpenCommandMenu, None),
        KeyBinding::new("cmd-shift-p", OpenCommandMenu, None),
        KeyBinding::new("alt-cmd-b", ToggleChangesPanel, None),
        KeyBinding::new("cmd-shift-a", ArchiveChat, None),
    ]);
    cx.bind_keys((1..=9).map(|n| KeyBinding::new(&format!("cmd-{n}"), GoToChat(n - 1), None)));
    cx.bind_keys(
        (1..=6).map(|n| KeyBinding::new(&format!("cmd-alt-{n}"), GoToRecentChat(n - 1), None)),
    );
    ui::composer::bind_keys(cx);
    features::init(cx);
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
            MenuItem::separator(),
            MenuItem::action("Find", FindInThread),
            MenuItem::action("Find Next", FindNext),
            MenuItem::action("Find Previous", FindPrevious),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Changes Panel", ToggleChangesPanel),
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
        Menu::new("Help").items([
            MenuItem::action("Documentation", OpenDocumentation),
            MenuItem::action("Keyboard Shortcuts", ShowKeyboardShortcuts),
        ]),
    ]
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Route {
    #[default]
    Home,
    Thread(String),
    Settings,
    /// Rail `Scheduled` (`/scheduled`): the Automations page.
    Scheduled,
    /// A sidebar task row (`/scheduled?automationId=<id>`): the task's thread.
    Automation(String),
    /// Profile menu ▸ `Set up mobile` (`/codex-mobile`).
    Mobile,
}

/// The thread route's side panel tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SidePanel {
    /// `Changes`: the last turn's diff.
    Changes,
    /// The first background command's terminal.
    Terminal,
    /// A file link in an answer: the file viewer ([`Root::file`]).
    File,
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

/// Window layout, route, theme and close preference, persisted to `path` when set.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub width: f32,
    pub height: f32,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub route: Route,
    pub theme: ThemeSetting,
    pub confirm_before_close: ConfirmBeforeClose,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            width: 1280.,
            height: 832.,
            sidebar_open: true,
            sidebar_width: 340.,
            route: Route::Home,
            theme: ThemeSetting::System,
            confirm_before_close: ConfirmBeforeClose::default(),
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmBeforeClose {
    Never,
    #[default]
    KeyboardOnly,
    Always,
}

pub fn theme_for(appearance: WindowAppearance) -> Theme {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::dark(),
        WindowAppearance::Light | WindowAppearance::VibrantLight => Theme::light(),
    }
}

/// The local wall clock's offset from UTC, in seconds.
fn utc_offset() -> i64 {
    chrono::Local::now().offset().local_minus_utc().into()
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
    /// Recents in sidebar order, for `⌘1`…`⌘9`, Previous/Next Chat and the command menu.
    threads: Vec<Thread>,
    /// The window is at least [`SIDEBAR_BREAKPOINT`] wide; narrower hides the sidebar.
    wide: bool,
    /// Holds key focus when no composer shows (Settings, a loading thread, sign-in), so
    /// shortcuts still reach the root's actions.
    focus: FocusHandle,
    home: Entity<HomeMain>,
    /// Docked under every open thread.
    thread_composer: Entity<Composer>,
    thread: Option<Entity<ThreadView>>,
    diff: Option<Entity<DiffPanel>>,
    /// The file viewer of [`SidePanel::File`]; cleared with the thread.
    file: Option<Entity<FileView>>,
    /// The open thread in the main shell, with its side panel if any.
    split: Option<Entity<SplitScreen>>,
    /// Side panel tabs of the thread route in opening order, and the selected one (always in
    /// `tabs`); kept across threads like the original's panel.
    tabs: Vec<SidePanel>,
    side: Option<SidePanel>,
    /// The route's own shell (Settings, Scheduled, Mobile).
    page: Option<Entity<Shell>>,
    /// The Settings nav item the next Settings route opens at (`General` when `None`).
    settings_section: Option<&'static str>,
    /// `⌘K` command menu over the window.
    command_menu: Option<Entity<CommandMenu>>,
    dialog: Option<Entity<Dialog>>,
    /// Signed out: replaces the whole window.
    login: Option<Entity<Login>>,
    bridge: Arc<Bridge>,
    host: Arc<dyn host::Host>,
    automations: Option<Result<Value, String>>,
    automation_load: Option<Task<()>>,
    /// The Scheduled routes' sidebar, fed the host's list when the read completes.
    scheduled: Option<Entity<automations::ScheduledSidebar>>,
    /// The Scheduled empty page: its suggestion cards start guided conversations.
    suggestions: Option<Entity<automations::AutomationsPage>>,
    cwd: String,
    config: Value,
    skills: Value,
    ready: bool,
    /// Threads started or resumed by this app-server; others need `thread/resume` first.
    loaded: HashSet<String>,
    /// Id of the server request the composer's approval card answers.
    approval: Option<Value>,
    /// The `Notifications` region over the window.
    toaster: Entity<Toaster>,
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
        if !cx.has_global::<host_settings::Settings>() {
            let settings = match state.path.as_ref() {
                Some(path) => {
                    host_settings::Settings::load(path.with_file_name("host-settings.json"))
                }
                None => Ok(host_settings::Settings::default()),
            };
            match settings {
                Ok(settings) => cx.set_global(settings),
                Err(error) => eprintln!("codex-gpui: cannot load host settings: {error:#}"),
            }
        }
        let this = cx.weak_entity();
        // Both pickers write the user's `config.toml`, like the original's `writeModel`.
        let on_model = |this: WeakEntity<Self>| {
            move |model: &str, effort: &str, cx: &mut App| {
                this.update(cx, |this, cx| this.write_model(model, effort, cx))
                    .ok();
            }
        };
        let home_composer = cx.new(|cx| {
            let mut composer = Composer::home(window, cx);
            composer.on_model(on_model(this.clone()));
            let this = this.clone();
            composer.on_submit(move |s, window, cx| {
                this.update(cx, |this, cx| this.start_thread(s.clone(), window, cx))
                    .ok();
            });
            composer
        });
        let thread_composer = cx.new(|cx| {
            let mut composer = Composer::thread(window, cx);
            composer.on_model(on_model(this.clone()));
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
        for composer in [&home_composer, &thread_composer] {
            cx.subscribe_in(composer, window, |this, _, command: &ui::composer::SlashCommand, window, cx| {
                match command {
                    ui::composer::SlashCommand::Fast => {
                        this.write_config("config/batchWrite", json!({
                            "edits": [{"keyPath": "service_tier", "value": "priority", "mergeStrategy": "upsert"}],
                            "filePath": null, "expectedVersion": null, "reloadUserConfig": true,
                        }), cx, |config| config["service_tier"] = "priority".into());
                    }
                    ui::composer::SlashCommand::Archive => {
                        if let Route::Thread(id) = this.state.route.clone() {
                            this.archive(id, window, cx);
                        }
                    }
                    ui::composer::SlashCommand::NewChat => this.new_chat(window, cx),
                    ui::composer::SlashCommand::Usage => {
                        if let Some(section) = settings::section("Usage & billing") {
                            this.open_settings_section(section, window, cx);
                        }
                    }
                }
            }).detach();
        }
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
            shell.sidebar_width = px(if state.sidebar_width.is_finite() {
                state.sidebar_width.clamp(290., 520.)
            } else {
                340.
            });
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
            threads: Vec::new(),
            wide,
            focus: cx.focus_handle(),
            home: cx.new(|_| HomeMain {
                composer: home_composer,
            }),
            thread_composer,
            thread: None,
            diff: None,
            file: None,
            split: None,
            tabs: Vec::new(),
            side: None,
            page: None,
            settings_section: None,
            command_menu: None,
            dialog: None,
            login: None,
            host: Arc::new(host::CloudHost::from_env(host::app_server_credentials(
                bridge.clone(),
            ))),
            bridge,
            automations: None,
            automation_load: None,
            scheduled: None,
            suggestions: None,
            cwd,
            config: Value::Null,
            skills: Value::Null,
            ready: false,
            loaded: HashSet::new(),
            approval: None,
            toaster: cx.new(|_| Toaster::new()),
            _events,
            state,
        };
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| {
                if this.state.confirm_before_close != ConfirmBeforeClose::Always {
                    return true;
                }
                this.open_dialog(DialogKind::CloseWindow, window, cx);
                false
            })
            .unwrap_or(true)
        });
        window.focus(&root.focus, cx);
        // The DOM's body: when the focused element goes (a closed find bar or menu blurs, a
        // node leaves the tree), the root takes focus so the key bindings keep working.
        cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx))
            .detach();
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
                    composer.update(cx, |c, cx| {
                        c.set_models(&models.data, &config.config, cx);
                        c.set_slash_skills(&skills, cx);
                    });
                }
                this.config = config.config;
                this.config["host"]["confirm-before-close"] =
                    serde_json::to_value(this.state.confirm_before_close)
                        .expect("close preference");
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

    /// The shell on screen: the route's own (Settings, Scheduled, Mobile), else the main one.
    pub fn shell(&self) -> &Entity<Shell> {
        self.page.as_ref().unwrap_or(&self.shell)
    }

    /// The Home sidebar of the chat routes.
    pub fn sidebar(&self) -> &Entity<HomeSidebar> {
        &self.sidebar
    }

    /// The thread route's selected side panel tab.
    pub fn side_panel(&self) -> Option<SidePanel> {
        self.side
    }

    /// The thread route's side panel tabs in strip order.
    pub fn side_tabs(&self) -> &[SidePanel] {
        &self.tabs
    }

    pub fn diff_panel(&self) -> Option<&Entity<DiffPanel>> {
        self.diff.as_ref()
    }

    /// The file viewer of the open thread, once a file link was clicked.
    pub fn file_view(&self) -> Option<&Entity<FileView>> {
        self.file.as_ref()
    }

    /// The `⌘K` command menu, while open.
    pub fn command_menu(&self) -> Option<&Entity<CommandMenu>> {
        self.command_menu.as_ref()
    }

    /// The composer of the route: Home's, or the one docked under the open thread.
    pub fn composer(&self, cx: &App) -> Entity<Composer> {
        match self.state.route {
            Route::Thread(_) => self.thread_composer.clone(),
            _ => self.home.read(cx).composer.clone(),
        }
    }

    /// Recents thread ids in sidebar order.
    pub fn recents(&self) -> Vec<String> {
        self.threads.iter().map(|t| t.id.clone()).collect()
    }

    /// The `config/read` config with the writes the server took since.
    pub fn config(&self) -> &Value {
        &self.config
    }

    /// Replaces the desktop host and cancels any pending host read.
    pub fn set_host(&mut self, host: Arc<dyn host::Host>, cx: &mut Context<Self>) {
        self.automation_load = None;
        self.host = host;
        self.automations = None;
        if self.scheduled_route() {
            self.load_automations(cx);
        }
        cx.notify();
    }

    /// The host's automation list, or its error; `None` means that the read has not completed.
    pub fn automations(&self) -> Option<&Result<Value, String>> {
        self.automations.as_ref()
    }

    fn scheduled_route(&self) -> bool {
        matches!(self.state.route, Route::Scheduled | Route::Automation(_))
    }

    fn load_automations(&mut self, cx: &mut Context<Self>) {
        self.automations = None;
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            host.fetch(&host::Request::post("list-automations", None))
                .and_then(|response| host::json_body(&response))
                .map_err(|error| error.to_string())
        });
        self.automation_load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if result.is_err() && this.scheduled_route() {
                    this.toast(Toast::danger("Failed to load automations"), cx);
                }
                let tasks = result
                    .as_ref()
                    .map_or_else(|_| Vec::new(), automations::tasks);
                if let Some(sidebar) = &this.scheduled {
                    sidebar.update(cx, |sidebar, cx| sidebar.set_tasks(tasks, cx));
                }
                this.automations = Some(result);
                cx.notify();
            })
            .ok();
        }));
    }

    /// The host's tasks as read so far.
    fn tasks(&self) -> Vec<automations::Task> {
        self.automations
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map_or_else(Vec::new, automations::tasks)
    }

    /// The Scheduled empty page, when the route shows it.
    pub fn suggestions(&self) -> Option<&Entity<automations::AutomationsPage>> {
        self.suggestions.as_ref()
    }

    /// A suggestion card click (`refs/interactions/s035b.json`): the card is already
    /// `Starting…` and every card disabled. The original's guided start first reads the TPP
    /// models through the desktop host, then prepares and streams the conversation and
    /// navigates into it; when any step fails it shows `Couldn’t start this suggested task`
    /// and re-enables the cards.
    fn start_suggested_task(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let identity = host.identity().context("no ChatGPT sign-in")?;
            host.fetch(&host::tpp_models(&identity))
                .and_then(|response| host::json_body(&response))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if let Some(page) = &this.suggestions {
                    page.update(cx, |page, cx| page.set_pending(None, cx));
                }
                // ponytail: with the models in hand the original prepares the ChatGPT
                // conversation (`/f/conversation/prepare`) and streams it; neither is recorded
                // (the oracle refuses the start), so the port stops here and reports it.
                if result.is_ok() {
                    eprintln!("codex-gpui: suggested task: the conversation stream is not wired");
                }
                this.toast(Toast::danger("Couldn’t start this suggested task"), cx);
            })
            .ok();
        })
        .detach();
    }

    fn set_threads(&mut self, threads: &[Thread], cx: &mut Context<Self>) {
        self.threads = threads.to_vec();
        self.sidebar.update(cx, |s, cx| s.set_threads(threads, cx));
    }

    /// Puts `thread` at the top of Recents.
    fn add_thread(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        self.threads.retain(|t| t.id != thread.id);
        self.threads.insert(0, thread.clone());
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
            Route::Thread(id) => self.threads.iter().position(|t| t.id == *id),
            _ => None,
        };
        let index = current.map_or(Some(0), |i| i.checked_add_signed(delta));
        self.go_to_chat(index, window, cx);
    }

    fn go_to_chat(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = index
            .and_then(|i| self.threads.get(i))
            .map(|t| t.id.clone())
        {
            self.navigate(Route::Thread(id), window, cx);
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
        self.page = match self.state.route.clone() {
            Route::Settings => {
                // The Appearance page shows the theme the app keeps.
                let mut config = self.config.clone();
                config["host"] = cx
                    .try_global::<host_settings::Settings>()
                    .map(|settings| Value::Object(settings.read_all().clone()))
                    .unwrap_or(Value::Null);
                let (store, key) = settings::THEME.split_once('.').expect("store.key");
                config[store][key] = json!(self.state.theme);
                config["host"]["confirm-before-close"] = json!(self.state.confirm_before_close);
                let section = self.settings_section.take();
                let (shell, page) = settings::shell(&config, &self.skills, section, cx);
                cx.subscribe(&page, |this, page, e: &SettingChanged, cx| {
                    this.write_setting(e, cx);
                    if e.path.starts_with("host.") && e.path != settings::THEME {
                        let mut values = cx
                            .try_global::<host_settings::Settings>()
                            .map(|settings| Value::Object(settings.read_all().clone()))
                            .unwrap_or(Value::Null);
                        values["appearance-theme"] = json!(this.state.theme);
                        values["confirm-before-close"] = json!(this.state.confirm_before_close);
                        page.update(cx, |page, cx| page.refresh_host(values, cx));
                    }
                })
                .detach();
                Some(shell)
            }
            Route::Scheduled => {
                let tasks = self.tasks();
                self.load_automations(cx);
                let (shell, sidebar, page) = automations::shell(tasks, None, utc_offset(), cx);
                self.scheduled = Some(sidebar);
                self.suggestions = page;
                Some(shell)
            }
            Route::Automation(id) => {
                // The list is the one the Scheduled page read; a cold start reads it now.
                if self.automations.is_none() && self.automation_load.is_none() {
                    self.load_automations(cx);
                }
                let tasks = self.tasks();
                let selected = tasks.iter().find(|t| t.id == id).cloned().map(|task| {
                    let composer: AnyView = cx.new(|cx| Composer::automation(window, cx)).into();
                    (task, composer)
                });
                let (shell, sidebar, page) = automations::shell(tasks, selected, utc_offset(), cx);
                self.scheduled = Some(sidebar);
                self.suggestions = page;
                Some(shell)
            }
            Route::Mobile => Some(mobile::shell(cx)),
            Route::Home | Route::Thread(_) => None,
        };
        if let Some(page) = &self.page {
            page.update(cx, |shell, _| {
                shell.can_go_back = can_back;
                shell.can_go_forward = can_forward;
            });
        }
        let thread_id = match &self.state.route {
            Route::Thread(id) => Some(id.clone()),
            _ => None,
        };
        let view = self
            .thread
            .clone()
            .filter(|v| Some(v.read(cx).id()) == thread_id.as_deref());
        let main: AnyView = match (&thread_id, &view) {
            (Some(_), Some(view)) => view.clone().into(),
            (Some(id), None) => {
                self.load_thread(id.clone(), window, cx);
                cx.new(|_| ui::EmptyMain).into()
            }
            (None, _) => self.home.clone().into(),
        };
        window.focus(&self.route_focus(cx), cx);
        self.sidebar
            .update(cx, |s, cx| s.select(thread_id.as_deref(), cx));
        let home = self.state.route == Route::Home;
        self.shell.update(cx, |shell, cx| {
            shell.selected_rail = home.then_some("home");
            shell.can_go_back = can_back;
            shell.can_go_forward = can_forward;
            // Thread routes: `with_side_panel` below sets its own.
            shell.new_tab = true;
            shell.main_composited = false;
            shell.set_main(main, cx);
        });
        self.split = view.map(|view| {
            let panel = self.side_tabs_view(&view, cx);
            with_side_tabs(self.shell.clone(), view, panel, cx)
        });
        self.save(window, cx);
        cx.notify();
    }

    /// Keys need a focused element in the frame: the route's composer, else the root (the
    /// route's own shell, a loading thread, the sign-in view, which take no keys of their own).
    fn route_focus(&self, cx: &App) -> FocusHandle {
        let open = |id: &str| self.thread.as_ref().is_some_and(|v| v.read(cx).id() == id);
        match &self.state.route {
            _ if self.login.is_some() || self.page.is_some() => self.focus.clone(),
            Route::Thread(id) if open(id) => self.thread_composer.focus_handle(cx),
            Route::Thread(_) => self.focus.clone(),
            _ => self.home.read(cx).composer.focus_handle(cx),
        }
    }

    /// The strip tab of `side` in the open thread `view`: `(icon, label)`. No terminal tab
    /// without a background command, no file tab without a file viewer.
    fn side_tab(
        side: SidePanel,
        view: &Entity<ThreadView>,
        file: Option<&Entity<FileView>>,
        cx: &App,
    ) -> Option<Tab> {
        match side {
            SidePanel::Changes => Some(("diff-tab-changes", "Changes".into())),
            SidePanel::Terminal => {
                let (command, _) = view.read(cx).background_terminals().into_iter().next()?;
                Some(("thread-terminal", command.into()))
            }
            SidePanel::File => {
                let file = file?.read(cx);
                Some((file.tab_icon(), file.name()))
            }
        }
    }

    /// The open tabs of thread `view` and the selected tab's view, dropping the tabs the
    /// thread cannot show: `(view, tabs, selected index)`.
    fn side_tabs_view(
        &mut self,
        view: &Entity<ThreadView>,
        cx: &mut Context<Self>,
    ) -> Option<(AnyView, Vec<Tab>, usize)> {
        let file = self.file.clone();
        self.tabs
            .retain(|side| Self::side_tab(*side, view, file.as_ref(), cx).is_some());
        let tabs: Vec<Tab> = self
            .tabs
            .iter()
            .filter_map(|side| Self::side_tab(*side, view, file.as_ref(), cx))
            .collect();
        let active = self
            .side
            .and_then(|side| self.tabs.iter().position(|t| *t == side))
            .or(self.tabs.len().checked_sub(1));
        self.side = active.map(|ix| self.tabs[ix]);
        let active = active?;
        let view: AnyView = match self.tabs[active] {
            SidePanel::Changes => self.diff.as_ref()?.clone().into(),
            SidePanel::Terminal => cx.new(|cx| Xterm::new(view.clone(), cx)).into(),
            SidePanel::File => self.file.as_ref()?.clone().into(),
        };
        Some((view, tabs, active))
    }

    /// Opens (a new tab after the others) or switches to the thread route's side panel tab.
    fn set_side(&mut self, side: SidePanel, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.state.route, Route::Thread(_)) || self.side == Some(side) {
            return;
        }
        if !self.tabs.contains(&side) {
            self.tabs.push(side);
        }
        self.side = Some(side);
        self.sync(window, cx);
    }

    /// Closes tab `side` (`Close {title} tab`, `⌘W` on the selected one): the next tab, else
    /// the previous, is selected; closing the last tab closes the panel.
    fn close_side(&mut self, side: SidePanel, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|t| *t == side) else {
            return;
        };
        self.tabs.remove(ix);
        if self.side == Some(side) {
            self.side = self.tabs.get(ix).or(self.tabs.last()).copied();
        }
        self.sync(window, cx);
    }

    /// A file link in an answer: the file viewer tab, filled with the local file.
    fn open_file(&mut self, action: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let cwd = view.read(cx).cwd().trim_end_matches('/').to_owned();
        let path = if action.path.starts_with('/') {
            action.path.clone()
        } else {
            format!("{cwd}/{}", action.path.trim_start_matches("./"))
        };
        let file = cx.new(|_| FileView::new(&cwd, path.clone(), action.line));
        // The original asks its Electron main for `vscode://codex/read-file`; the port's local
        // host is this process, so it reads the file itself.
        let read = cx.background_spawn(async move {
            std::fs::read_to_string(&path).map_err(|error| error.to_string())
        });
        let target = file.clone();
        cx.spawn(async move |this, cx| {
            let contents = read.await;
            let failed = contents.is_err();
            target.update(cx, |file, cx| file.set_contents(contents, cx));
            if failed {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Could not open file"), cx)
                })
                .ok();
            }
        })
        .detach();
        self.file = Some(file);
        if self.side == Some(SidePanel::File) {
            // The tab is selected already: show the new file in it.
            self.sync(window, cx);
        } else {
            self.set_side(SidePanel::File, window, cx);
        }
    }

    /// `⌘K`, `⌘⇧P` and the sidebar `Search`: the command menu over live Recents, focused.
    fn open_command_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let menu = cx.new(|cx| CommandMenu::with_threads(&self.threads, "", cx));
        cx.subscribe_in(&menu, window, |this, _, event, window, cx| {
            this.run_command(event, window, cx)
        })
        .detach();
        window.focus(&menu.focus_handle(cx), cx);
        self.command_menu = Some(menu);
        cx.notify();
    }

    pub fn dialog(&self) -> Option<&Entity<Dialog>> {
        self.dialog.as_ref()
    }

    fn open_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        self.command_menu = None;
        let dialog = cx.new(|cx| Dialog::new(kind, cx));
        cx.subscribe_in(&dialog, window, |this, _, outcome, window, cx| {
            this.dialog = None;
            match outcome {
                DialogOutcome::Dismiss => window.focus(&this.focus, cx),
                DialogOutcome::CloseWindow { do_not_ask_again } => {
                    if *do_not_ask_again {
                        this.state.confirm_before_close = ConfirmBeforeClose::Never;
                        this.config["host"]["confirm-before-close"] = "never".into();
                        this.state.save();
                    }
                    window.remove_window();
                }
            }
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.dialog = Some(dialog);
        cx.notify();
    }

    /// `⌘W`: Electron main runs `closeTab` (`close-active-app-shell-tab`) while the thread
    /// route's panel shows a tab, else the window closes.
    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        if let Some(side) = self.side.filter(|_| self.split.is_some()) {
            return self.close_side(side, window, cx);
        }
        if self.state.confirm_before_close == ConfirmBeforeClose::Never {
            window.remove_window();
        } else {
            self.open_dialog(DialogKind::CloseWindow, window, cx);
        }
    }

    /// Closes the command menu and runs its row: a chat opens, `New chat`, `Manage scheduled
    /// tasks` and the settings pages navigate.
    /// ponytail: commands whose feature the port lacks (Open folder, Show Mini, Log out, ...)
    /// only close the menu; run them when the features land.
    fn run_command(
        &mut self,
        event: &CommandMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_menu = None;
        match event {
            CommandMenuEvent::Dismiss => {}
            CommandMenuEvent::OpenThread(id) => {
                self.navigate(Route::Thread(id.clone()), window, cx)
            }
            CommandMenuEvent::Command(label) => match label.as_ref() {
                "New chat" => self.new_chat(window, cx),
                "Manage scheduled tasks" => self.navigate(Route::Scheduled, window, cx),
                label => {
                    if let Some(section) = settings::section(label) {
                        self.open_settings_section(section, window, cx);
                    }
                }
            },
        }
        // Give the keys back to the route (navigating already did).
        window.focus(&self.route_focus(cx), cx);
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
        let running = thread
            .turns
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress);
        // The composer is shared: the new thread's queue arrives with `thread/queue/changed`.
        self.thread_composer.update(cx, |c, cx| {
            c.set_running(running, cx);
            c.set_queued(Vec::new(), cx);
            c.set_search_roots(vec![thread.cwd.clone()]);
        });
        let route = Route::Thread(thread.id.clone());
        let clock = Clock {
            utc_offset: utc_offset(),
            pinned_now: None,
        };
        let composer = self.thread_composer.clone().into();
        self.diff = Some(cx.new(|_| DiffPanel::last_turn(&self.cwd, &thread.cwd, &thread.turns)));
        self.file = None;
        let view = cx.new(|_| ThreadView::new(thread, clock, composer));
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

    /// Thread composer: `turn/start` on the open thread, resuming it first if needed. While a
    /// turn runs (CP-108), `Follow-up behavior` decides: `Queue` adds the message to the
    /// app-server's queue (`thread/queue/add`, drained by the server when the turn ends),
    /// `Steer` sends it into the turn (`turn/steer`). A refusal shows `Error submitting
    /// message` and puts the text back.
    fn send(&mut self, s: Submission, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let view = view.read(cx);
        let id = view.id().to_owned();
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let running = view
            .turns()
            .iter()
            .rev()
            .find(|turn| turn.status == TurnStatus::InProgress)
            .map(|turn| turn.id.clone());
        let queue = self.config["desktop"]["followUpQueueMode"] == "queue";
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            resume_if(resume, &bridge, &this, &id, cx).await?;
            let Some(turn_id) = running else {
                return turn_start(&bridge, &id, &s).await;
            };
            let input = json!([{ "type": "text", "text": s.text, "text_elements": [] }]);
            let (method, params) = if queue {
                let params = json!({
                    "threadId": id, "input": input, "clientUserMessageId": uuid_v4(),
                });
                ("thread/queue/add", params)
            } else {
                let params = json!({ "threadId": id, "input": input, "expectedTurnId": turn_id });
                ("turn/steer", params)
            };
            let result = bridge.request::<_, Value>(method, params).await.map(drop);
            this.update(cx, |this, cx| match &result {
                Ok(()) if queue => this.list_queue(id, cx),
                Ok(()) => {}
                Err(_) => {
                    this.thread_composer
                        .update(cx, |c, cx| c.restore_text(&s.text, cx));
                    this.toast(Toast::danger("Error submitting message"), cx);
                }
            })?;
            result
        });
        log_err(task, cx);
    }

    /// `thread/queue/list`: the queued follow-ups of thread `id`, shown over the composer.
    fn list_queue(&self, id: String, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let list: Value = bridge
                .request(
                    "thread/queue/list",
                    json!({ "threadId": id, "cursor": null }),
                )
                .await?;
            let rows = list["data"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| {
                    row["input"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| part["text"].as_str())
                        .collect::<String>()
                        .trim_end()
                        .to_owned()
                        .into()
                })
                .collect();
            this.update(cx, |this, cx| {
                if this.thread.as_ref().is_some_and(|v| v.read(cx).id() == id) {
                    this.thread_composer
                        .update(cx, |c, cx| c.set_queued(rows, cx));
                }
            })
        });
        log_err(task, cx);
    }

    fn edit_message(&mut self, edit: EditMessage, window: &mut Window, cx: &mut Context<Self>) {
        let valid = self.thread.as_ref().is_some_and(|view| {
            let view = view.read(cx);
            view.id() == edit.thread_id && view.can_edit(&edit.turn_id)
        });
        if !valid || edit.message.trim().is_empty() {
            edit.editor.update(cx, |editor, cx| editor.edit_failed(cx));
            if !valid {
                self.toast(Toast::danger("Failed to edit message"), cx);
            }
            return;
        }
        let resume = !self.loaded.contains(&edit.thread_id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let mut reverted = false;
            let result =
                async {
                    resume_if(resume, &bridge, &this, &edit.thread_id, cx).await?;
                    let read: ThreadReadResponse = bridge
                        .request(
                            "thread/revert",
                            json!({
                                "threadId": edit.thread_id, "beforeTurnId": edit.turn_id,
                            }),
                        )
                        .await?;
                    reverted = true;
                    let cwd = read.thread.cwd.clone();
                    this.update_in(cx, |this, window, cx| {
                        if this.state.route == Route::Thread(edit.thread_id.clone()) {
                            this.show_thread(read.thread, window, cx);
                        }
                    })?;
                    bridge.request::<_, Value>("turn/start", json!({
                    "threadId": edit.thread_id,
                    "turnTrigger": "edit_user_message",
                    "input": [{"type": "text", "text": edit.message, "text_elements": []}],
                    "cwd": cwd,
                })).await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
            if result.is_err() {
                this.update(cx, |this, cx| {
                    edit.editor.update(cx, |editor, cx| editor.edit_failed(cx));
                    if reverted {
                        this.thread_composer
                            .update(cx, |composer, cx| composer.restore_text(&edit.message, cx));
                    }
                    this.toast(Toast::danger("Failed to edit message"), cx);
                })?;
            }
            result
        });
        log_err(task, cx);
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Route::Thread(id) = &self.state.route else {
            return;
        };
        let Some(view) = &self.thread else { return };
        let view = view.read(cx);
        if view.id() != id {
            return;
        }
        let Some(turn) = view
            .turns()
            .iter()
            .rev()
            .find(|turn| turn.status == TurnStatus::InProgress)
        else {
            return;
        };
        let (thread_id, turn_id) = (id.clone(), turn.id.clone());
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = bridge.turn_interrupt(&thread_id, &turn_id).await;
            if result.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Failed to stop chat"), cx)
                })?;
            }
            result
        });
        log_err(task, cx);
    }

    /// A file change card's `Undo`: `thread/revert` to before `turn_id`, then shows the
    /// reverted thread.
    fn revert(&mut self, turn_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let id = view.read(cx).id().to_owned();
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            resume_if(resume, &bridge, &this, &id, cx).await?;
            let params = json!({ "threadId": id, "beforeTurnId": turn_id });
            let read: ThreadReadResponse = bridge.request("thread/revert", params).await?;
            this.update_in(cx, |this, window, cx| {
                if this.state.route == Route::Thread(read.thread.id.clone()) {
                    this.show_thread(read.thread, window, cx);
                }
            })
        });
        log_err(task, cx);
    }

    /// `Fork chat from here` under `turn_id`: `thread/fork`, then `thread/rollback` of the
    /// later turns, and opens the new chat.
    fn fork(&mut self, turn_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let view = view.read(cx);
        let id = view.id().to_owned();
        let Some(later) = view.turns().iter().rev().position(|t| t.id == turn_id) else {
            return;
        };
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            // Both answer `{ thread }`, like `thread/read`.
            let mut read: ThreadReadResponse = bridge
                .request("thread/fork", json!({ "threadId": id }))
                .await?;
            if later > 0 {
                let params = json!({ "threadId": read.thread.id, "numTurns": later });
                read = bridge.request("thread/rollback", params).await?;
            }
            this.update_in(cx, |this, window, cx| {
                this.loaded.insert(read.thread.id.clone());
                this.add_thread(&read.thread, cx);
                this.show_thread(read.thread, window, cx);
            })
        });
        log_err(task, cx);
    }

    fn unpin(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == id) else {
            return;
        };
        let old_section = thread.section.take();
        self.sidebar
            .update(cx, |s, cx| s.set_threads(&self.threads, cx));
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let done = bridge
                .request::<_, Value>(
                    "thread/section/move",
                    json!({ "threadId": id, "sectionId": null, "beforeThreadId": null }),
                )
                .await;
            this.update(cx, |this, cx| {
                if done.is_err() {
                    if let Some(thread) = this.threads.iter_mut().find(|thread| thread.id == id) {
                        thread.section = old_section;
                    }
                    this.sidebar
                        .update(cx, |s, cx| s.set_threads(&this.threads, cx));
                    this.toast(Toast::danger("Couldn't save your changes. Try again"), cx);
                }
            })?;
            done.map(drop)
        });
        log_err(task, cx);
    }

    /// `thread/archive`: the Recents row leaves at once (an archived open chat leaves for Home),
    /// then `Archived chat` offers View and Undo; a refused write puts the row back with
    /// `Failed to archive chat`.
    fn archive(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.threads.iter().position(|t| t.id == id);
        let thread = at.map(|i| self.threads.remove(i));
        self.sidebar
            .update(cx, |s, cx| s.set_threads(&self.threads, cx));
        if self.state.route == Route::Thread(id.clone()) {
            self.new_chat(window, cx);
        }
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let done = bridge
                .request::<_, Value>("thread/archive", json!({ "threadId": id }))
                .await;
            this.update(cx, |this, cx| match &done {
                Ok(_) => {
                    let (view, undo) = (cx.weak_entity(), cx.weak_entity());
                    let toast = Toast::info("Archived chat")
                        .icon("sidebar-archive")
                        .action("View", move |window, cx| {
                            view.update(cx, |this, cx| this.open_archived_chats(window, cx))
                                .ok();
                        })
                        .action("Undo", move |window, cx| {
                            let id = id.clone();
                            undo.update(cx, |this, cx| this.unarchive(id, at, window, cx))
                                .ok();
                        });
                    this.toast(toast, cx);
                }
                Err(_) => {
                    if let Some(thread) = thread {
                        this.threads
                            .insert(at.unwrap_or(0).min(this.threads.len()), thread);
                        this.sidebar
                            .update(cx, |s, cx| s.set_threads(&this.threads, cx));
                    }
                    this.toast(Toast::danger("Failed to archive chat"), cx);
                }
            })?;
            done.map(drop)
        });
        log_err(task, cx);
    }

    /// The toast's Undo: `thread/unarchive` puts the row back where it was.
    fn unarchive(
        &mut self,
        id: String,
        at: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let done = bridge
                .request::<_, Value>("thread/unarchive", json!({ "threadId": id }))
                .await;
            this.update(cx, |this, cx| {
                let thread = done
                    .as_ref()
                    .ok()
                    .and_then(|r| serde_json::from_value::<Thread>(r["thread"].clone()).ok());
                match thread {
                    Some(thread) => {
                        this.threads.retain(|t| t.id != thread.id);
                        this.threads
                            .insert(at.unwrap_or(0).min(this.threads.len()), thread);
                        this.sidebar
                            .update(cx, |s, cx| s.set_threads(&this.threads, cx));
                    }
                    None => this.toast(Toast::danger("Failed to unarchive conversation"), cx),
                }
            })?;
            done.map(drop)
        });
        log_err(task, cx);
    }

    /// Shows `toast` in the `Notifications` region.
    pub fn toast(&mut self, toast: Toast, cx: &mut Context<Self>) {
        self.toaster.update(cx, |t, cx| t.push(toast, cx));
    }

    /// Titles of the toasts shown, newest first.
    pub fn toasts(&self, cx: &App) -> Vec<SharedString> {
        self.toaster.read(cx).titles()
    }

    /// Settings at `section`, whether or not Settings is open.
    fn open_settings_section(
        &mut self,
        section: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_section = Some(section);
        if self.state.route == Route::Settings {
            self.sync(window, cx);
        } else {
            self.navigate(Route::Settings, window, cx);
        }
    }

    /// The archive toast's View.
    /// ponytail: the original's destination is not recorded; Settings ▸ Archived chats lists them.
    fn open_archived_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(section) = settings::section("Archived chats") {
            self.open_settings_section(section, window, cx);
        }
    }

    /// `recentThread1..6`: the `n`th chat by last update, newest first.
    fn go_to_recent_chat(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut recent: Vec<&Thread> = self.threads.iter().collect();
        recent.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        if let Some(id) = recent.get(n).map(|t| t.id.clone()) {
            self.navigate(Route::Thread(id), window, cx);
        }
    }

    /// A model picker choice: `config/batchWrite` of `model` and `model_reasoning_effort`
    /// (the original's `writeModel`).
    fn write_model(&mut self, model: &str, effort: &str, cx: &mut Context<Self>) {
        let params = json!({
            "edits": [
                { "keyPath": "model", "value": model, "mergeStrategy": "upsert" },
                { "keyPath": "model_reasoning_effort", "value": effort, "mergeStrategy": "upsert" },
            ],
            "filePath": null,
            "expectedVersion": null,
            "reloadUserConfig": true,
        });
        let (model, effort) = (model.to_owned(), effort.to_owned());
        self.write_config("config/batchWrite", params, cx, move |config| {
            config["model"] = model.into();
            config["model_reasoning_effort"] = effort.into();
        });
    }

    /// Window preferences keep their state storage; other host settings use the desktop service.
    fn write_setting(&mut self, e: &SettingChanged, cx: &mut Context<Self>) {
        if e.path == settings::THEME {
            return;
        }
        if e.path == "host.confirm-before-close" {
            if let Ok(value) = serde_json::from_value(e.value.clone()) {
                self.state.confirm_before_close = value;
                self.config["host"]["confirm-before-close"] = e.value.clone();
                self.state.save();
            }
            return;
        }
        if e.path == "desktop.followUpQueueMode" {
            let queue = e.value == "queue";
            self.thread_composer
                .update(cx, |c, cx| c.set_queue_mode(queue, cx));
        }
        let (store, key) = e.path.split_once('.').unwrap_or((e.path, ""));
        if store == "host" {
            let result = if cx.has_global::<host_settings::Settings>() {
                cx.global_mut::<host_settings::Settings>()
                    .write(key, e.value.clone())
            } else {
                Err(anyhow::anyhow!("host settings could not be loaded"))
            };
            if let Err(error) = result {
                eprintln!("codex-gpui: settings.write({key}): {error:#}");
                self.toast(Toast::danger("Couldn't save your changes. Try again"), cx);
            }
            return;
        }
        let value = e.value.clone();
        let apply = move |config: &mut Value| config[store][key] = value;
        if !matches!(store, "desktop" | "features") {
            return apply(&mut self.config);
        }
        let params = json!({
            "keyPath": e.path,
            "value": e.value,
            "mergeStrategy": "upsert",
            "filePath": null,
            "expectedVersion": null,
        });
        self.write_config("config/value/write", params, cx, apply);
    }

    /// Sends a config write and, once the server took it, applies it to the kept config.
    fn write_config(
        &self,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Value) + 'static,
    ) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            bridge.request::<_, Value>(method, params).await?;
            this.update(cx, |this, _| apply(&mut this.config))
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
    pub fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if let Some(view) = &self.thread {
            view.update(cx, |view, cx| {
                view.apply(&event, cx);
                if let Some(panel) = &self.diff
                    && let Event::Notification { params, .. } = &event
                    && params["threadId"].as_str() == Some(view.id())
                {
                    panel.update(cx, |panel, cx| {
                        panel.apply(&event, &self.cwd, view.turns(), cx);
                    });
                }
            });
            if matches!(&event, Event::Notification { method, .. } if matches!(method.as_str(), "turn/started" | "turn/completed"))
            {
                let running = view
                    .read(cx)
                    .turns()
                    .iter()
                    .any(|turn| turn.status == TurnStatus::InProgress);
                self.thread_composer
                    .update(cx, |c, cx| c.set_running(running, cx));
            }
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
                "thread/queue/changed" => {
                    if let Some(id) = params["threadId"].as_str()
                        && self.thread.as_ref().is_some_and(|v| v.read(cx).id() == id)
                    {
                        self.list_queue(id.to_owned(), cx);
                    }
                }
                "fuzzyFileSearch/sessionUpdated" | "fuzzyFileSearch/sessionCompleted" => {
                    let completed = method == "fuzzyFileSearch/sessionCompleted";
                    self.thread_composer
                        .update(cx, |c, cx| c.apply_file_search(completed, &params, cx));
                }
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
        self.state.sidebar_width = self.shell.read(cx).sidebar_width.into();
        self.state.save();
    }
}

/// A random v4 UUID (`clientUserMessageId`) from the std hasher's per-instance random keys.
/// ponytail: not a CSPRNG; the `uuid` crate if a collision ever matters.
fn uuid_v4() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let word = |salt: u64| RandomState::new().hash_one(salt);
    let (a, b) = (word(1), word(2));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        0x8000 | ((b >> 48) & 0x3fff),
        b & 0xffff_ffff_ffff
    )
}

/// `thread/resume` when this app-server has not loaded thread `id` yet (`resume`).
async fn resume_if(
    resume: bool,
    bridge: &Bridge,
    this: &WeakEntity<Root>,
    id: &str,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    if resume {
        bridge
            .request::<_, Value>("thread/resume", json!({ "threadId": id }))
            .await?;
        this.update(cx, |this, _| this.loaded.insert(id.to_owned()))?;
    }
    Ok(())
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
        let content: AnyElement = match (&self.login, &self.page, &self.split) {
            (Some(login), ..) => login.clone().into_any_element(),
            (None, Some(page), _) => page.clone().into_any_element(),
            (None, None, Some(split)) => split.clone().into_any_element(),
            (None, None, None) => self.shell.clone().into_any_element(),
        };
        let root = div()
            .size_full()
            .relative()
            .font_family(ui::SANS)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FindInThread, window, cx| {
                if matches!(this.state.route, Route::Thread(_))
                    && let Some(thread) = &this.thread
                {
                    thread.update(cx, |thread, cx| thread.open_find(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| {
                if let Some(thread) = &this.thread {
                    thread.update(cx, |thread, cx| thread.next_find(false, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| {
                if let Some(thread) = &this.thread {
                    thread.update(cx, |thread, cx| thread.next_find(true, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
            .on_action(cx.listener(|this, _: &ui::composer::Stop, _, cx| this.stop(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.navigate(Route::Settings, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenScheduled, window, cx| {
                this.navigate(Route::Scheduled, window, cx)
            }))
            .on_action(
                cx.listener(|this, a: &automations::OpenAutomation, window, cx| {
                    this.navigate(Route::Automation(a.id.clone()), window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &automations::NewTask, window, cx| {
                this.open_dialog(DialogKind::ScheduleTask, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &automations::StartSuggestedTask, _, cx| {
                    this.start_suggested_task(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenMobileSetup, window, cx| {
                this.navigate(Route::Mobile, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleChangesPanel, window, cx| {
                if this.side == Some(SidePanel::Changes) {
                    this.close_side(SidePanel::Changes, window, cx)
                } else {
                    this.set_side(SidePanel::Changes, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &OpenChangesPanel, window, cx| {
                this.set_side(SidePanel::Changes, window, cx)
            }))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(|this, _: &OpenBackgroundTerminal, window, cx| {
                this.set_side(SidePanel::Terminal, window, cx)
            }))
            .on_action(cx.listener(|this, a: &ActivateSideTab, window, cx| {
                if let Some(side) = this.tabs.get(a.0).copied() {
                    this.set_side(side, window, cx)
                }
            }))
            .on_action(cx.listener(|this, a: &CloseSideTab, window, cx| {
                if let Some(side) = this.tabs.get(a.0).copied() {
                    this.close_side(side, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &OpenCommandMenu, window, cx| {
                this.open_command_menu(window, cx)
            }))
            .on_action(cx.listener(|this, a: &SetTheme, window, cx| {
                let theme = serde_json::from_value(json!(a.0)).unwrap_or_default();
                this.set_theme(theme, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.step(false, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousChat, window, cx| this.step_chat(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextChat, window, cx| this.step_chat(1, window, cx)))
            .on_action(cx.listener(|this, a: &GoToRecentChat, window, cx| {
                this.go_to_recent_chat(a.0, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ArchiveChat, window, cx| {
                if let Route::Thread(id) = this.state.route.clone() {
                    this.archive(id, window, cx)
                }
            }))
            .on_action(cx.listener(|this, a: &ArchiveThread, window, cx| {
                this.archive(a.0.clone(), window, cx)
            }))
            .on_action(
                cx.listener(|this, a: &UnpinThread, window, cx| {
                    this.unpin(a.0.clone(), window, cx)
                }),
            )
            .on_action(cx.listener(|this, a: &ForkThread, window, cx| this.fork(&a.0, window, cx)))
            .on_action(cx.listener(|this, edit: &EditMessage, window, cx| {
                this.edit_message(edit.clone(), window, cx)
            }))
            .on_action(
                cx.listener(|this, a: &RevertTurn, window, cx| {
                    this.revert(a.0.clone(), window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, a: &GoToChat, window, cx| {
                    this.go_to_chat(Some(a.0), window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.shell.update(cx, |shell, cx| shell.toggle_sidebar(cx))
            }))
            .on_action(cx.listener(|this, _: &ShowKeyboardShortcuts, window, cx| {
                if this
                    .dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.read(cx).kind() == DialogKind::Shortcuts)
                {
                    this.dialog = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                } else {
                    this.open_dialog(DialogKind::Shortcuts, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &CloseWindow, window, cx| this.close_window(window, cx)),
            )
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .child(content)
            .children(self.command_menu.clone())
            .child(self.toaster.clone())
            .children(self.dialog.clone());
        features::actions(root, cx)
    }
}
