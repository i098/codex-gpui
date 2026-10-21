//! The app's root view: routes, persisted window state, and the live `codex app-server`
//! connection that feeds the screens (Recents, model picker, threads, approvals, sign-in),
//! plus the key bindings and menu bar. `main.rs` only opens the window;
//! `examples/live_turn.rs` and `tests/flows.rs` drive the same view headless.

pub mod computer_use_requests;
mod features;
pub mod host;
pub mod host_settings;
mod onboarding;
pub mod qa;
mod routes;
pub use routes::Route;
mod search;
mod section_actions;
mod signin;
pub mod windows;

use anyhow::{Context as _, Result, ensure};
use async_channel::Receiver;
use bridge::types::{
    Account, AccountReadResponse, CommandApprovalRequest, ConfigReadResponse,
    FileChangeApprovalRequest, Model, PermissionsApprovalRequest, QueuedSubmission, Thread,
    ThreadItem, ThreadListResponse, ThreadQueueListResponse, ThreadReadResponse, ThreadSection,
    ThreadStartResponse, ThreadStartedNotification, ThreadStatus, TurnStatus,
};
use bridge::{Bridge, Event};
use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, AsyncWindowContext, BorrowAppContext as _,
    Context, Entity, EntityInputHandler as _, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, KeyDownEvent, Menu, MenuItem, OsAction, ParentElement as _,
    PathPromptOptions, Pixels, Render, SharedString, Styled as _, SystemMenuType, Task, WeakEntity,
    Window, WindowAppearance, actions, div, px,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use theme::Theme;
use ui::composer::{Approval, Composer, Permission, QueueAction, Submission};
use ui::dialog::{
    CommitRepository, CommitStep, Dialog, Kind as DialogKind, Outcome as DialogOutcome, Selection,
};
use ui::screens::computer_use::{
    AppApproval as ComputerUseAppApproval, ComputerUseAction, ComputerUseState, Load,
    Plugin as ComputerUsePlugin, SoundMode as ComputerUseSoundMode,
};
use ui::screens::diff::panel::{
    ActivateSideTab, CloseSideTab, DiffPanel, DiffRequest, FileView, OpenFile, SplitScreen, Tab,
    with_side_tabs,
};
use ui::screens::home::{HomeMain, OpenFolder};
use ui::screens::settings::{self, AppServer, SettingChanged};
use ui::screens::sidebar::{
    ArchiveThread, ArchivedChats, CommandMenu, CommandMenuEvent, HomeSidebar, LoadMore, OpenThread,
    PINNED_SECTION_ID, PinThread, RenameThread, SEARCH_LIMIT, SearchHit, SearchRequest,
    SelectProject, title as thread_title,
};
use ui::screens::signin::Login;
use ui::screens::terminal::{OpenTerminalItem, StopBackgroundTerminals, Xterm};
use ui::screens::thread::view::{
    Clock, EditMessage, ForkThread, GitAction, RevertTurn, ThreadView,
};
use ui::screens::{automations, mobile};
use ui::toast::{Toast, Toaster};
use ui::{
    Copy, Cut, GoBack, GoForward, LogOut, NewChat, OpenBackgroundTerminal, OpenChangesPanel,
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
        KeyBinding::new("cmd-o", OpenFolder, None),
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
            MenuItem::action("Log Out", LogOut),
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
            MenuItem::action("Open Folder…", OpenFolder),
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
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub width: f32,
    pub height: f32,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub route: Route,
    pub theme: ThemeSetting,
    pub confirm_before_close: ConfirmBeforeClose,
    pub project_root: Option<String>,
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
            project_root: None,
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
/// The original shows `Signed in as <email>` for ChatGPT sign-ins only.
fn chatgpt_email(account: &AccountReadResponse) -> Option<String> {
    match &account.account {
        Some(Account::Chatgpt { email, .. }) => email.clone(),
        _ => None,
    }
}

fn log_err(task: Task<Result<()>>, cx: &App) {
    cx.foreground_executor()
        .spawn(async move {
            if let Err(err) = task.await {
                eprintln!("codex-gpui: {err:#}");
            }
        })
        .detach();
}
#[derive(Deserialize)]
struct ModelCatalog {
    data: Vec<CatalogModel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogModel {
    #[serde(flatten)]
    model: Model,
    availability_nux: Option<ModelAvailability>,
}

#[derive(Deserialize)]
struct ModelAvailability {
    message: String,
}

const LEGACY_MODEL_ANNOUNCEMENTS: [&str; 2] = ["gpt-5.4", "gpt-5.5"];

#[derive(Clone)]
struct DeletedQueuedMessage {
    thread_id: String,
    submission: QueuedSubmission,
    previous: Option<String>,
    next: Option<String>,
}

pub struct Root {
    /// Window size, sidebar and route, saved back on every change.
    state: State,
    back: Vec<(Route, Option<String>)>,
    forward: Vec<(Route, Option<String>)>,
    navigation_epoch: u64,
    sidebar_automation: Option<String>,
    thread_run_settings: HashMap<String, host::run_now::heartbeat::RendererState>,
    shell: Entity<Shell>,
    sidebar: Entity<HomeSidebar>,
    /// Recents in sidebar order, for `⌘1`…`⌘9`, Previous/Next Chat and the command menu.
    threads: Vec<Thread>,
    new_chat_section: Option<String>,
    /// `thread/list` cursor of the page after `threads`, while the sidebar's end asks for it.
    next_cursor: Option<String>,
    loading_more: bool,
    /// The window is at least [`SIDEBAR_BREAKPOINT`] wide; narrower hides the sidebar.
    wide: bool,
    /// Holds key focus when no composer shows (Settings, a loading thread, sign-in), so
    /// shortcuts still reach the root's actions.
    focus: FocusHandle,
    home: Entity<HomeMain>,
    /// Docked under every open thread.
    thread_composer: Entity<Composer>,
    /// The queued submission `Edit message` took back into the composer, until it is sent.
    queue_edit: Option<QueueEdit>,
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
    /// The route's own shell.
    page: Option<Entity<Shell>>,
    archive_page: Option<Entity<ArchivedChats>>,
    /// The Mobile route's setup page.
    mobile_page: Option<Entity<mobile::MobileSetupPage>>,
    /// The Settings nav item the next Settings route opens at (`General` when `None`).
    settings_section: Option<&'static str>,
    settings_page: Option<Entity<settings::SettingsPage>>,
    onboarding_page: Option<Entity<ui::screens::onboarding::Onboarding>>,
    onboarding_load: Option<Task<()>>,
    onboarding_context: Option<onboarding::ContextData>,
    /// `⌘K` command menu over the window.
    command_menu: Option<Entity<CommandMenu>>,
    dialog: Option<Entity<Dialog>>,
    /// The open dialog came from the composer: its close returns focus there.
    dialog_to_composer: bool,
    /// Signed out: replaces the whole window.
    login: Option<Entity<Login>>,
    logout_pending: bool,
    /// The signed-in ChatGPT account's email (`account/read`), shown in the logout dialog.
    account_email: Option<String>,
    chatgpt_account: bool,
    bridge: Arc<Bridge>,
    host: Arc<dyn host::Host>,
    project_load: Option<Task<()>>,
    picking_project: bool,
    automations: Option<Result<Value, String>>,
    automation_load: Option<Task<()>>,
    /// Tasks whose `automation-run-now` request has not answered.
    running_automations: HashSet<String>,
    automation_updates: HashMap<String, Task<()>>,
    suggestion_load: Option<Task<()>>,
    /// The Scheduled routes' sidebar, fed the host's list when the read completes.
    scheduled: Option<Entity<automations::ScheduledSidebar>>,
    /// The Scheduled empty page: its suggestion cards start guided conversations.
    suggestions: Option<Entity<automations::AutomationsPage>>,
    cwd: String,
    clock: Option<Clock>,
    config: Value,
    skills: Value,
    models: Vec<Model>,
    ready: bool,
    qa_work_started: bool,
    /// Threads started or resumed by this app-server; others need `thread/resume` first.
    loaded: HashSet<String>,
    queue_load: Option<Task<()>>,
    queue_pending: bool,
    deleted_queued: Vec<DeletedQueuedMessage>,
    /// A successful steer must never be sent again when its queue deletion fails.
    steered_queued: HashSet<(String, String)>,
    /// Server approval requests waiting for a decision, oldest first. The thread composer
    /// shows the open thread's oldest one.
    approvals: Vec<PendingApproval>,
    interrupting: HashSet<(String, String)>,
    /// A commit, push or branch workflow is running (the original's single workflow state).
    git_busy: Option<&'static str>,
    git_status_load: Option<Task<()>>,
    /// The repository the open commit form read; a later chat switch cannot redirect it.
    commit_cwd: Option<String>,
    /// `Include unstaged changes`, kept from one commit form to the next.
    commit_include_unstaged: bool,
    /// The ephemeral thread that writes a commit message.
    generation: Option<Generation>,
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
        Self::with_connection(state, Arc::new(bridge), events, cwd, window, cx)
    }

    fn with_connection(
        state: State,
        bridge: Arc<Bridge>,
        events: Receiver<Event>,
        cwd: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let computer_use = this.clone();
        cx.set_global(ComputerUseState {
            available: cfg!(target_os = "macos") && !bridge.is_acp(),
            permission_guidance: Some(
                "macOS controls Accessibility and Screen Recording access.".into(),
            ),
            dispatch: Some(Arc::new(move |action, cx| {
                computer_use
                    .update(cx, |root, cx| root.computer_use_action(action, cx))
                    .ok();
            })),
            ..Default::default()
        });
        // Both pickers write the user's `config.toml`, like the original's `writeModel`.
        let on_model = |this: WeakEntity<Self>| {
            move |model: &str, effort: &str, cx: &mut App| {
                this.update(cx, |this, cx| this.write_model(model, effort, cx))
                    .ok();
            }
        };
        let home_composer = cx.new(|cx| {
            let mut composer = Composer::home(window, cx);
            if let Some(root) = &state.project_root {
                composer.set_search_roots(vec![root.clone()], cx);
            }
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
            composer.on_approval(move |id, thread, response, _, cx| {
                // Deferred: answering refreshes this composer's card.
                let this = this.clone();
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| this.answer(id, &thread, response, cx))
                        .ok();
                });
            });
            composer
        });
        cx.subscribe_in(
            &thread_composer,
            window,
            |this, _, action: &QueueAction, window, cx| {
                this.queue_action(action, window, cx);
            },
        )
        .detach();
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
                    ui::composer::SlashCommand::Compact => this.compact(window, cx),
                    ui::composer::SlashCommand::Fork => this.fork_chat(0, window, cx),
                    ui::composer::SlashCommand::Rename => {
                        if let Route::Thread(id) = this.state.route.clone() {
                            this.open_rename(id, true, window, cx);
                        }
                    }
                    ui::composer::SlashCommand::Pin => {
                        if let Route::Thread(id) = this.state.route.clone() {
                            let pinned = this.threads.iter().any(|t| {
                                t.id == id
                                    && t.section.as_ref().is_some_and(|s| s.id == PINNED_SECTION_ID)
                            });
                            this.set_pinned(id, !pinned, window, cx);
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
        cx.subscribe_in(&sidebar, window, |this, _, _: &LoadMore, _, cx| {
            this.load_more_threads(cx)
        })
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
        cx.observe(&sidebar, |this, sidebar, cx| {
            let attention = !sidebar.read(cx).unread.is_empty();
            if this.shell.read(cx).attention != attention {
                this.shell.update(cx, |shell, cx| {
                    shell.attention = attention;
                    cx.notify();
                });
            }
        })
        .detach();
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
        if qa::ENABLED {
            // ponytail: defer used sessions until manual close; prove state restoration before relaxing this guard.
            cx.observe_window_activation(window, |this, window, _| {
                this.qa_work_started |= window.is_window_active();
            })
            .detach();
        }
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
        let mut root = Self {
            back: Vec::new(),
            forward: Vec::new(),
            navigation_epoch: 0,
            sidebar_automation: None,
            thread_run_settings: HashMap::new(),
            shell,
            sidebar,
            threads: Vec::new(),
            new_chat_section: None,
            next_cursor: None,
            loading_more: false,
            wide,
            focus: cx.focus_handle(),
            home: cx.new(|_| HomeMain {
                composer: home_composer,
                project: state.project_root.as_deref().map(project_name),
            }),
            thread_composer,
            queue_edit: None,
            thread: None,
            diff: None,
            file: None,
            split: None,
            tabs: Vec::new(),
            side: None,
            page: None,
            archive_page: None,
            mobile_page: None,
            settings_section: None,
            settings_page: None,
            onboarding_page: None,
            onboarding_load: None,
            onboarding_context: None,
            command_menu: None,
            dialog: None,
            dialog_to_composer: false,
            login: None,
            logout_pending: false,
            account_email: None,
            chatgpt_account: false,
            host: Arc::new(
                host::CloudHost::from_env(host::app_server_credentials(bridge.clone()))
                    .with_bridge(bridge.clone()),
            ),
            bridge,
            automations: None,
            automation_load: None,
            running_automations: HashSet::new(),
            project_load: None,
            picking_project: false,
            automation_updates: HashMap::new(),
            suggestion_load: None,
            scheduled: None,
            suggestions: None,
            cwd,
            clock: None,
            config: Value::Null,
            skills: Value::Null,
            models: Vec::new(),
            ready: false,
            qa_work_started: false,
            loaded: HashSet::new(),
            queue_load: None,
            queue_pending: false,
            deleted_queued: Vec::new(),
            steered_queued: HashSet::new(),
            approvals: Vec::new(),
            interrupting: HashSet::new(),
            git_busy: None,
            git_status_load: None,
            commit_cwd: None,
            commit_include_unstaged: true,
            generation: None,
            toaster: cx.new(|_| Toaster::new()),
            _events,
            state,
        };
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| {
                if this.state.confirm_before_close != ConfirmBeforeClose::Always {
                    this.flush_settings(cx);
                    return windows::allow_close(window, cx);
                }
                this.open_dialog(DialogKind::CloseWindow, window, cx);
                false
            })
            .unwrap_or(true)
        });
        cx.on_app_quit(|this, cx| {
            this.flush_settings(cx);
            async {}
        })
        .detach();
        window.focus(&root.focus, cx);
        // The DOM's body: when the focused element goes (a closed find bar or menu blurs, a
        // node leaves the tree), the root takes focus so the key bindings keep working.
        cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx))
            .detach();
        root.connect(window, cx);
        root.load_projects(cx);
        windows::bind_root(window, cx);
        root
    }

    /// Uses `clock` for thread timestamps. Set it before opening a thread.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// `initialize`, then account, Recents, models and config; shows the route once ready.
    /// An ACP agent has no Codex account, model catalog, config or skills: the app asks only
    /// for Recents, and a toast says which features work (README `ACP backend`).
    fn connect(&self, window: &Window, cx: &mut Context<Self>) {
        let (bridge, cwd) = (self.bridge.clone(), self.cwd.clone());
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let server = bridge
                .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                .await?;
            let acp = bridge.is_acp();
            let account = if acp {
                None
            } else {
                Some(bridge.account_read().await?)
            };
            let threads = bridge.thread_list().await?;
            let (mut catalog, config, skills) = if acp {
                // The picker's one row says the agent picks: an empty id sends no model, and
                // an empty effort shows no effort label.
                let model = Model {
                    id: String::new(),
                    model: String::new(),
                    display_name: "Agent model".into(),
                    description: "Set on the agent's command line".into(),
                    hidden: false,
                    default_reasoning_effort: String::new(),
                    supported_reasoning_efforts: Vec::new(),
                    is_default: true,
                };
                let data = vec![CatalogModel {
                    model,
                    availability_nux: None,
                }];
                let config = ConfigReadResponse {
                    config: json!({ "model_reasoning_effort": "" }),
                };
                (ModelCatalog { data }, config, Value::Null)
            } else {
                let catalog: ModelCatalog = bridge.request("model/list", json!({})).await?;
                let config: ConfigReadResponse = bridge
                    .request("config/read", json!({ "includeLayers": false, "cwd": cwd }))
                    .await?;
                // Only the settings page reads it (home directory), so a failure is not fatal.
                let skills = bridge
                    .request("skills/list", json!({ "cwds": [cwd] }))
                    .await
                    .unwrap_or(Value::Null);
                (catalog, config, skills)
            };
            this.update_in(cx, |this, window, cx| {
                this.account_email = account.as_ref().and_then(chatgpt_email);
                this.chatgpt_account = account.as_ref().is_some_and(|account| {
                    matches!(account.account, Some(Account::Chatgpt { .. }))
                });
                if account.is_some_and(|a| a.account.is_none() && a.requires_openai_auth) {
                    this.login = Some(this.new_login(window, cx));
                }
                this.set_threads(&threads.data, cx);
                this.set_next_cursor(threads.next_cursor, cx);
                if !acp && this.login.is_none() {
                    this.load_sections(cx);
                }
                let announcements_allowed = this.login.is_none()
                    && matches!(this.state.route, Route::Home | Route::Thread(_));
                let current_model = config.config["model"].as_str().or_else(|| {
                    catalog
                        .data
                        .iter()
                        .find(|entry| !entry.model.hidden && entry.model.is_default)
                        .map(|entry| entry.model.model.as_str())
                });
                if announcements_allowed
                    && let Some(current) = current_model
                    && !LEGACY_MODEL_ANNOUNCEMENTS.contains(&current)
                    && catalog.data.iter().any(|entry| {
                        entry.model.model == current && entry.availability_nux.is_some()
                    })
                {
                    this.remember_model_announcement(current, cx);
                }
                let announcement = announcements_allowed
                    .then(|| {
                        catalog.data.iter().position(|entry| {
                            entry.availability_nux.is_some()
                                && Some(entry.model.model.as_str()) != current_model
                                && !LEGACY_MODEL_ANNOUNCEMENTS.contains(&entry.model.model.as_str())
                                && !Self::model_announcement_history(cx)
                                    .iter()
                                    .any(|seen| seen.as_str() == Some(entry.model.model.as_str()))
                        })
                    })
                    .flatten()
                    .map(|index| {
                        let message = catalog.data[index]
                            .availability_nux
                            .take()
                            .expect("eligible model announcement")
                            .message;
                        (index, message)
                    });
                let models: Vec<Model> =
                    catalog.data.into_iter().map(|entry| entry.model).collect();
                let composers = [
                    this.home.read(cx).composer.clone(),
                    this.thread_composer.clone(),
                ];
                for composer in composers {
                    composer.update(cx, |c, cx| {
                        c.set_models(&models, &config.config, cx);
                        c.set_slash_skills(&skills, cx);
                    });
                }
                this.config = config.config;
                this.config["host"]["confirm-before-close"] =
                    serde_json::to_value(this.state.confirm_before_close)
                        .expect("close preference");
                this.skills = skills;
                this.models = models;
                this.ready = true;
                this.select_route(cx);
                this.sync(window, cx);
                this.maybe_onboard(window, cx);
                if bridge.is_acp() {
                    let note = format!(
                        "{} over ACP: only chats, streaming, Stop and approvals work",
                        server.user_agent
                    );
                    this.toast(Toast::info(note), cx);
                }
                if let Some((index, message)) = announcement {
                    let dialog =
                        cx.new(|cx| Dialog::model_announcement(&this.models[index], message, cx));
                    this.show_dialog(dialog, window, cx);
                }
            })
        });
        log_err(task, cx);
    }

    /// `initialize` and the startup reads are done and the route is showing.
    pub fn ready(&self) -> bool {
        self.ready
    }

    pub fn qa_restart_ready(&self, window: &Window, cx: &App) -> bool {
        self.ready
            && !window.is_window_active()
            && !self.qa_work_started
            && self.state.route == Route::Home
            && self.loaded.is_empty()
            && self
                .threads
                .iter()
                .all(|thread| thread.status != ThreadStatus::Active)
            && self.dialog.is_none()
            && self.command_menu.is_none()
            && self.approvals.is_empty()
            && self.automation_load.is_none()
            && self.state.confirm_before_close != ConfirmBeforeClose::Always
            && self.home.read(cx).composer.read(cx).restart_ready()
            && self.thread_composer.read(cx).restart_ready()
            && self
                .login
                .as_ref()
                .is_none_or(|login| login.read(cx).restart_ready())
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

    /// The shell on screen: the route's own shell, else the main shell.
    pub fn shell(&self) -> &Entity<Shell> {
        self.page.as_ref().unwrap_or(&self.shell)
    }

    pub fn archived_chats(&self) -> Option<&Entity<ArchivedChats>> {
        self.archive_page.as_ref()
    }

    pub fn mobile_page(&self) -> Option<&Entity<mobile::MobileSetupPage>> {
        self.mobile_page.as_ref()
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

    /// Replaces the desktop host and cancels pending UI callbacks for its reads and updates.
    pub fn set_host(&mut self, host: Arc<dyn host::Host>, cx: &mut Context<Self>) {
        self.automation_load = None;
        self.project_load = None;
        self.suggestion_load = None;
        self.git_status_load = None;
        self.automation_updates.clear();
        if let Some(sidebar) = &self.scheduled {
            for task in self.tasks() {
                sidebar.update(cx, |sidebar, cx| sidebar.set_pending(&task.id, false, cx));
            }
        }
        if let Some(thread) = &self.thread {
            thread.update(cx, |view, cx| view.set_git_repository(None, cx));
        }
        self.host = host;
        self.automations = None;
        self.load_projects(cx);
        if self.scheduled_route() {
            self.load_automations(cx);
            self.load_suggestions(cx);
        }
        self.refresh_git_actions(cx);
        cx.notify();
    }

    fn load_projects(&mut self, cx: &mut Context<Self>) {
        self.set_projects(Vec::new(), cx);
        let host = self.host.clone();
        let read = cx.background_spawn(async move { host.projects() });
        self.project_load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| match result {
                Ok(projects) => this.set_projects(projects, cx),
                Err(error) => eprintln!("codex-gpui: cannot read sidebar projects: {error:#}"),
            })
            .ok();
        }));
    }

    /// The sidebar's projects, and Home's `/project` choices: the ones with a root on disk.
    fn set_projects(
        &mut self,
        projects: Vec<ui::screens::sidebar::Project>,
        cx: &mut Context<Self>,
    ) {
        let choices = projects
            .iter()
            .filter(|project| !project.missing)
            .filter_map(|project| Some((project.label.clone(), project.root.as_ref()?.to_string())))
            .collect();
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, _| composer.set_slash_projects(choices));
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_projects(projects, cx));
    }

    fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picking_project {
            return;
        }
        self.picking_project = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project".into()),
        });
        let host = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(paths) = paths.await?? else {
                    return Ok(None);
                };
                let Some(path) = paths.into_iter().next() else {
                    return Ok(None);
                };
                cx.background_spawn(async move {
                    let root = host.add_project(&path)?;
                    Ok::<_, anyhow::Error>(Some((root, host.projects()?)))
                })
                .await
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                this.picking_project = false;
                match result {
                    Ok(Some((root, projects))) => {
                        this.project_load = None;
                        this.sidebar
                            .update(cx, |sidebar, cx| sidebar.set_projects(projects, cx));
                        this.select_project(root, window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("codex-gpui: cannot choose project: {error:#}");
                        this.toast(Toast::danger("Cannot open project folder"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Makes `root` the folder new chats start in (`thread/start` cwd) and shows it on Home.
    /// Open chats keep the cwd they were started in.
    fn select_project(&mut self, root: String, window: &mut Window, cx: &mut Context<Self>) {
        self.qa_work_started = true;
        self.new_chat_section = None;
        self.home.update(cx, |home, cx| {
            home.project = Some(project_name(&root));
            home.composer.update(cx, |composer, cx| {
                composer.set_search_roots(vec![root.clone()], cx);
            });
            cx.notify();
        });
        self.state.project_root = Some(root);
        self.navigate(Route::Home, window, cx);
        self.save(window, cx);
    }

    fn repository_request(
        &mut self,
        panel: Entity<DiffPanel>,
        cwd: &str,
        request: &DiffRequest,
        cx: &mut Context<Self>,
    ) {
        if panel.read(cx).pending() {
            return;
        }
        let (endpoint, mut params, reverted) = match request {
            DiffRequest::Refresh => ("git-changes", json!({"cwd": cwd}), None),
            DiffRequest::Stage(path) => ("git-stage", json!({"cwd": cwd, "path": path}), None),
            DiffRequest::Unstage(path) => ("git-unstage", json!({"cwd": cwd, "path": path}), None),
            DiffRequest::CreateRepository => ("git-init-repo", json!({"cwd": cwd}), None),
            DiffRequest::Changes { action, files } => (
                match *action {
                    "stage" => "git-stage",
                    "unstage" => "git-unstage",
                    _ => "git-revert",
                },
                json!({"cwd": cwd, "source": files.first().map(|f| f.source), "files": files}),
                (*action == "revert").then(|| "section".to_owned()),
            ),
            DiffRequest::Revert(change) => (
                "git-revert",
                json!({"cwd": cwd, "path": change.path, "source": change.source,
                    "diff": change.diff, "hunk": change.hunk}),
                Some(match change.hunk {
                    Some(index) => format!("hunk {} in {}", index + 1, change.path),
                    None => change.path.clone(),
                }),
            ),
            DiffRequest::Open(_) => return,
        };
        if let Some(review) = panel.read(cx).review_params().as_object() {
            params.as_object_mut().unwrap().extend(review.clone());
        }
        let request = host::Request::post(endpoint, Some(params));
        let host = self.host.clone();
        panel.update(cx, |panel, cx| panel.begin_repository_request(cx));
        let read = cx.background_spawn(async move {
            let body = host
                .fetch(&request)
                .and_then(|response| host::json_body(&response));
            body.and_then(|body| {
                let status = if endpoint == "git-revert" {
                    match body["status"].as_str() {
                        Some("success") => Some(false),
                        Some("partial-success") => Some(true),
                        _ => anyhow::bail!("Git revert returned an invalid result"),
                    }
                } else {
                    None
                };
                Ok((
                    serde_json::from_value::<ui::screens::diff::panel::RepositoryChanges>(body)?,
                    status,
                ))
            })
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if let Some(target) = reverted {
                    let toast = match &result {
                        Ok((_, Some(true))) => Toast::info(format!("Partially reverted {target}")),
                        Ok((_, Some(false))) => Toast::info(format!("Reverted {target}")),
                        _ => Toast::danger(format!("Failed to revert {target}")),
                    };
                    this.toast(toast, cx);
                }
                panel.update(cx, |panel, cx| {
                    panel.set_repository(result.map(|(changes, _)| changes), cx)
                });
                this.refresh_git_actions(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The header icon and `Open in editor`: the host opens the file in the editor chosen by the
    /// original's `open-in-target-preferences` (`perPath` for the project, else `global`).
    fn open_in_editor(&mut self, cwd: &str, path: &str, cx: &mut Context<Self>) {
        let target = cx
            .try_global::<host_settings::Settings>()
            .and_then(|settings| settings.read_all().get("open-in-target-preferences"))
            .and_then(|preferences| {
                preferences["perPath"][cwd]
                    .as_str()
                    .or(preferences["global"].as_str())
            })
            .map(str::to_owned);
        let request = host::Request::post(
            "open-file",
            Some(json!({"hostId": "local", "cwd": cwd, "path": path, "target": target})),
        );
        let host = self.host.clone();
        let path = path.to_owned();
        let opened = cx.background_spawn(async move { host.fetch(&request) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = opened.await {
                eprintln!("codex-gpui: cannot open {path} in the editor: {error:#}");
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Unable to open in editor"), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    fn refresh_git_actions(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.thread.clone() else {
            return;
        };
        let cwd = thread.read(cx).cwd().to_owned();
        let host = self.host.clone();
        let lookup = host.clone();
        thread.update(cx, |view, cx| view.set_git_repository(None, cx));
        self.sync_git_availability(cx);
        let read = cx.background_spawn(async move {
            git_call(&*lookup, "git-commit-status", json!({"cwd": cwd}))
                .and_then(|status| commit_repository(&status, true, String::new()))
                .map_err(|error| error.to_string())
        });
        self.git_status_load = Some(cx.spawn(async move |this, cx| {
            let repository = read.await;
            this.update(cx, |this, cx| {
                if this.thread.as_ref() != Some(&thread) || !Arc::ptr_eq(&host, &this.host) {
                    return;
                }
                thread.update(cx, |view, cx| view.set_git_repository(Some(repository), cx));
                this.sync_git_availability(cx);
            })
            .ok();
        }));
    }

    fn sync_git_availability(&self, cx: &mut Context<Self>) {
        if let Some(menu) = &self.command_menu {
            let available = self.git_busy.is_none()
                && self
                    .thread
                    .as_ref()
                    .is_some_and(|view| view.read(cx).git_command_available());
            menu.update(cx, |menu, cx| menu.set_git_available(available, cx));
        }
    }

    fn set_git_phase(&mut self, phase: Option<&'static str>, cx: &mut Context<Self>) {
        self.git_busy = phase;
        if let Some(thread) = &self.thread {
            thread.update(cx, |view, cx| view.set_git_phase(phase, cx));
        }
        self.sync_git_availability(cx);
    }

    /// Both original header actions open the form, including a push with no local changes.
    fn open_git_form(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.thread.clone() else {
            return;
        };
        if self.git_busy.is_some() || self.dialog.is_some() {
            return;
        }
        let view = thread.read(cx);
        let (cwd, title) = (view.cwd().to_owned(), view.header_title().to_string());
        let prefix = git_setting(cx, "git-branch-prefix").unwrap_or_else(|| "codex/".into());
        let name = branch_name(&prefix, &title);
        let include = self.commit_include_unstaged;
        let host = self.host.clone();
        let repository = cwd.clone();
        let request_host = host.clone();
        let read = cx.background_spawn(async move {
            git_call(&*host, "git-commit-status", json!({"cwd": repository}))
        });
        cx.spawn_in(window, async move |this, cx| {
            let status = read
                .await
                .and_then(|status| commit_repository(&status, include, name));
            this.update_in(cx, |this, window, cx| {
                if this.thread.as_ref() != Some(&thread)
                    || !Arc::ptr_eq(&this.host, &request_host)
                    || this.git_busy.is_some()
                    || this.dialog.is_some()
                {
                    return;
                }
                match status {
                    Ok(repo) => {
                        if kind != DialogKind::Commit
                            || repo.all.files > 0
                            || repo.branch.is_some() && repo.ahead > 0
                        {
                            let dialog = cx.new(|cx| {
                                if kind == DialogKind::Commit {
                                    Dialog::commit(repo, cx)
                                } else {
                                    Dialog::branches(repo, kind, cx)
                                }
                            });
                            this.commit_cwd = Some(cwd);
                            this.show_dialog(dialog, window, cx);
                        } else {
                            this.toast(Toast::info("No changes to commit"), cx);
                        }
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: cannot read Git status: {error:#}");
                        this.toast(
                            Toast::danger(if kind == DialogKind::Commit {
                                "Commit is unavailable right now"
                            } else {
                                "Branch information unavailable"
                            }),
                            cx,
                        );
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn checkout_branch(
        &mut self,
        dialog: Entity<Dialog>,
        name: String,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_busy.is_some() {
            return;
        }
        let Some(cwd) = self.commit_cwd.clone() else {
            return;
        };
        dialog.update(cx, |dialog, cx| dialog.begin_checkout(name.clone(), cx));
        self.set_git_phase(
            Some(if create {
                "Creating branch…"
            } else {
                "Switching branch…"
            }),
            cx,
        );
        let host = self.host.clone();
        let repository = cwd.clone();
        let mut params = json!({"cwd": repository});
        params[if create { "newBranch" } else { "branch" }] = name.into();
        let read = cx.background_spawn(async move {
            let result = git_call(&*host, "git-checkout", params);
            let repository = if result
                .as_ref()
                .is_ok_and(|body| body["status"] == "success")
            {
                None
            } else {
                git_call(&*host, "git-commit-status", json!({"cwd": repository}))
                    .and_then(|status| commit_repository(&status, true, String::new()))
                    .ok()
            };
            (result, repository)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (result, repository) = read.await;
            this.update_in(cx, |this, window, cx| {
                if let Some(repo) = repository {
                    dialog.update(cx, |dialog, cx| dialog.set_branch_repository(repo, cx));
                }
                match result {
                    Ok(body) if body["status"] == "success" => {
                        if this.dialog.as_ref() == Some(&dialog) {
                            this.dialog = None;
                            this.commit_cwd = None;
                            window.focus(&this.composer(cx).focus_handle(cx), cx);
                        }
                        let title = body["branch"].as_str().map_or_else(
                            || "Checked out detached HEAD".to_owned(),
                            |branch| format!("Switched to {branch}"),
                        );
                        this.finish_git_workflow(GitOutcome::Done(title), &cwd, cx);
                    }
                    result => {
                        let (error, conflicts) = match result {
                            Ok(body) => (
                                body["error"]
                                    .as_str()
                                    .unwrap_or("Git checkout returned an invalid result")
                                    .to_owned(),
                                if body["errorType"] == "blocked-by-working-tree-changes" {
                                    body["conflictedPaths"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter_map(|path| path.as_str().map(str::to_owned))
                                        .collect()
                                } else {
                                    Vec::new()
                                },
                            ),
                            Err(error) => (error.to_string(), Vec::new()),
                        };
                        dialog.update(cx, |dialog, cx| {
                            dialog.checkout_error(error.clone(), conflicts, cx)
                        });
                        if this.dialog.as_ref() == Some(&dialog) {
                            window.focus(&dialog.focus_handle(cx), cx);
                        }
                        this.finish_git_workflow(
                            GitOutcome::failed(
                                if create {
                                    "Failed to create branch"
                                } else {
                                    "Failed to check out branch"
                                },
                                Some(error),
                            ),
                            &cwd,
                            cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn commit_before_checkout(
        &mut self,
        dialog: Entity<Dialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self.commit_cwd.clone() else {
            return;
        };
        let Some(target) = dialog.read(cx).checkout_target() else {
            return;
        };
        dialog.update(cx, |dialog, cx| dialog.begin_checkout(target, cx));
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            git_call(&*host, "git-commit-status", json!({"cwd": cwd}))
                .and_then(|status| commit_repository(&status, true, String::new()))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            this.update_in(cx, |this, window, cx| {
                if this.dialog.as_ref() != Some(&dialog) {
                    return;
                }
                match result {
                    Ok(repo) => {
                        dialog.update(cx, |dialog, cx| dialog.prepare_branch_commit(repo, cx));
                        window.focus(&dialog.focus_handle(cx), cx);
                    }
                    Err(error) => {
                        dialog.update(cx, |dialog, cx| {
                            dialog.schedule_result(error.to_string(), cx)
                        });
                        this.toast(
                            Toast::danger(format!("Failed to read changes: {error}")),
                            cx,
                        );
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Creates the branch, commits and pushes in `cwd` as the submitted form chose, then
    /// toasts the result like the original and refreshes the Changes panel.
    fn git_workflow(&mut self, request: GitRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_busy.is_some() {
            return;
        }
        let phase = if request.step == CommitStep::Push {
            "Pushing changes…"
        } else if request.message.is_empty() {
            "Generating messages…"
        } else if request.new_branch.is_some() {
            "Creating branch…"
        } else {
            "Committing…"
        };
        self.set_git_phase(Some(phase), cx);
        let run = GitRun {
            host: self.host.clone(),
            bridge: self.bridge.clone(),
            model: generation_model(&self.models),
            cwd: request.cwd.clone(),
            instructions: git_setting(cx, "git-commit-instructions").unwrap_or_default(),
        };
        let continuation = self.dialog.clone().filter(|dialog| {
            dialog.read(cx).kind() == DialogKind::Commit
                && dialog.read(cx).checkout_target().is_some()
        });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = commit_workflow(&this, cx, &run, request).await;
            this.update_in(cx, |this, window, cx| {
                if let Some(dialog) = continuation {
                    match &outcome {
                        GitOutcome::Done(_) => {
                            let target = dialog.read(cx).checkout_target().unwrap();
                            dialog.update(cx, |dialog, cx| dialog.resume_checkout(cx));
                            this.finish_git_workflow(outcome, &run.cwd, cx);
                            this.checkout_branch(dialog, target, false, window, cx);
                            return;
                        }
                        GitOutcome::Failed { title, detail } => {
                            dialog.update(cx, |dialog, cx| {
                                dialog.schedule_result(detail.as_ref().unwrap_or(title).clone(), cx)
                            });
                        }
                    }
                }
                this.finish_git_workflow(outcome, &run.cwd, cx)
            })
            .ok();
        })
        .detach();
    }

    fn finish_git_workflow(&mut self, outcome: GitOutcome, cwd: &str, cx: &mut Context<Self>) {
        self.set_git_phase(None, cx);
        match outcome {
            GitOutcome::Done(title) => self.toast(Toast::info(title), cx),
            GitOutcome::Failed { title, detail } => {
                eprintln!(
                    "codex-gpui: {title}: {}",
                    detail.as_deref().unwrap_or_default()
                );
                let mut toast = Toast::danger(title);
                if let Some(detail) = detail {
                    toast = toast.action("Copy error", move |_, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(detail.clone()))
                    });
                }
                self.toast(toast, cx);
            }
        }
        if let (Some(panel), Some(thread)) = (self.diff.clone(), self.thread.clone())
            && thread.read(cx).cwd() == cwd
        {
            self.repository_request(panel, cwd, &DiffRequest::Refresh, cx);
        } else {
            self.refresh_git_actions(cx);
        }
    }

    fn confirm_revert(
        &mut self,
        panel: Entity<DiffPanel>,
        cwd: &str,
        request: &DiffRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if panel.read(cx).pending() || self.dialog.is_some() {
            return;
        }
        let request = request.clone();
        if cx
            .try_global::<host_settings::Settings>()
            .and_then(|settings| settings.read_all().get("skip-revert-confirmation"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            self.repository_request(panel, cwd, &request, cx);
            return;
        }
        let cwd = cwd.to_owned();
        let dialog = cx.new(|cx| Dialog::new(DialogKind::RevertChanges, cx));
        // Handle Confirm before show_dialog drops the event source.
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            if let DialogOutcome::RevertChanges { do_not_ask_again } = outcome {
                if *do_not_ask_again {
                    this.write_setting(
                        &SettingChanged {
                            path: "host.skip-revert-confirmation",
                            value: Value::Bool(true),
                        },
                        cx,
                    );
                }
                this.repository_request(panel.clone(), &cwd, &request, cx);
            }
        })
        .detach();
        self.show_dialog(dialog, window, cx);
    }

    /// The host's automation list, or its error; `None` means that the read has not completed.
    pub fn automations(&self) -> Option<&Result<Value, String>> {
        self.automations.as_ref()
    }

    fn scheduled_route(&self) -> bool {
        matches!(self.state.route, Route::Scheduled | Route::Automation(_))
    }

    fn load_automations(&mut self, cx: &mut Context<Self>) {
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

    fn toggle_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.automation_updates.contains_key(id) {
            return;
        }
        let item = self
            .automations
            .as_ref()
            .and_then(|list| list.as_ref().ok())
            .and_then(|list| list["items"].as_array())
            .and_then(|items| items.iter().find(|item| item["id"] == id));
        let Some(item) = item else { return };
        let status = match item["status"].as_str() {
            Some("ACTIVE") => "PAUSED",
            Some("PAUSED") => "ACTIVE",
            _ => return,
        };
        let body = automation_update_body(item, status);
        let id = id.to_owned();
        self.automation_load = None;
        if let Some(sidebar) = &self.scheduled {
            sidebar.update(cx, |sidebar, cx| sidebar.set_pending(&id, true, cx));
        }
        let host = self.host.clone();
        let expected_id = id.clone();
        let read = cx.background_spawn(async move {
            let response = host.fetch(&host::Request::post("automation-update", Some(body)))?;
            let response = host::json_body(&response)?;
            let item = response
                .get("item")
                .context("host response has no updated automation")?;
            anyhow::ensure!(
                item["id"] == expected_id && item["status"] == status,
                "host returned a different automation or status"
            );
            serde_json::from_value::<automations::Task>(item.clone())?;
            Ok::<_, anyhow::Error>(item.clone())
        });
        let update_key = id.clone();
        let update = cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                this.automation_updates.remove(&id);
                if let Some(sidebar) = &this.scheduled {
                    sidebar.update(cx, |sidebar, cx| sidebar.set_pending(&id, false, cx));
                }
                match result {
                    Ok(item) => {
                        if let Some(Ok(list)) = &mut this.automations
                            && let Some(items) = list["items"].as_array_mut()
                            && let Some(stored) = items.iter_mut().find(|item| item["id"] == id)
                        {
                            *stored = item;
                        }
                        let tasks = this.tasks();
                        if let Some(sidebar) = &this.scheduled {
                            sidebar.update(cx, |sidebar, cx| sidebar.set_tasks(tasks, cx));
                        }
                        this.toast(
                            Toast::success(if status == "PAUSED" {
                                "Successfully paused automation"
                            } else {
                                "Successfully resumed automation"
                            }),
                            cx,
                        );
                    }
                    Err(error) => this.toast(
                        Toast::danger("Could not update scheduled task")
                            .description(error.to_string()),
                        cx,
                    ),
                }
                this.load_automations(cx);
                cx.notify();
            })
            .ok();
        });
        self.automation_updates.insert(update_key, update);
    }

    fn load_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.suggestions.clone() else {
            return;
        };
        page.update(cx, |page, cx| {
            page.set_pending(None, cx);
            page.set_suggestions(Vec::new(), cx);
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let response = host.fetch(&host::suggested_automations())?;
            Ok::<Vec<automations::Suggestion>, anyhow::Error>(serde_json::from_value(
                host::json_body(&response)?,
            )?)
        });
        self.suggestion_load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |_, cx| match result {
                Ok(server) => page.update(cx, |page, cx| page.set_suggestions(server, cx)),
                Err(error) => eprintln!("codex-gpui: failed to load task suggestions: {error:#}"),
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

    fn sync_starting(&self, cx: &mut Context<Self>) {
        if let Some(sidebar) = &self.scheduled {
            let running = self.running_automations.clone();
            sidebar.update(cx, |sidebar, cx| sidebar.set_starting(running, cx));
        }
    }

    fn remember_thread_settings(&mut self, cx: &App) {
        if let Route::Thread(id) = &self.state.route
            && self
                .thread
                .as_ref()
                .is_some_and(|view| view.read(cx).id() == id)
        {
            let (collaboration_mode, permission) = self.thread_composer.read(cx).run_settings();
            self.thread_run_settings.insert(
                id.clone(),
                host::run_now::heartbeat::RendererState {
                    collaboration_mode,
                    permission,
                },
            );
        }
    }

    /// Uses the target thread's renderer settings and keeps navigation bound to the starting location.
    fn run_automation_now(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.running_automations.insert(id.clone()) {
            return;
        }
        self.remember_thread_settings(cx);
        let target = self
            .automations
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|v| v["items"].as_array())
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["id"] == id && item["kind"] == "heartbeat")
            })
            .and_then(|item| item["targetThreadId"].as_str())
            .map(str::to_owned);
        let settings = target
            .as_ref()
            .and_then(|t| self.thread_run_settings.get(t))
            .cloned();
        self.sync_starting(cx);
        self.toast(Toast::info("Starting run…"), cx);
        let epoch = self.navigation_epoch;
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let task_id = id.clone();
        let target_id = target.clone();
        let run = cx.background_spawn(async move {
            let params = if let Some(thread) = target_id {
                pollster::block_on(host::run_now::heartbeat::request_params(
                    &bridge, &task_id, &thread, settings,
                ))?
            } else {
                json!({ "id": task_id, "collaborationMode": null, "permissions": null })
            };
            host.fetch(&host::Request::post("automation-run-now", Some(params)))
                .and_then(|response| host::json_body(&response))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = run.await;
            this.update_in(cx, |this, window, cx| {
                this.running_automations.remove(&id);
                this.sync_starting(cx);
                let title = "Could not start scheduled task";
                match result {
                    Ok(body) if body["success"] == true => {
                        if let Some(target) = target {
                            this.loaded.insert(target.clone());
                            if this.navigation_epoch == epoch {
                                let reload = this
                                    .thread
                                    .as_ref()
                                    .is_some_and(|view| view.read(cx).id() == target);
                                this.navigate_with_sidebar(
                                    Route::Thread(target.clone()),
                                    Some(id),
                                    window,
                                    cx,
                                );
                                if reload {
                                    this.load_thread(target, window, cx);
                                }
                            }
                        }
                    }
                    Ok(_) => this.toast(Toast::danger(title), cx),
                    Err(error) => {
                        this.toast(Toast::danger(title).description(error.to_string()), cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// The Scheduled routes' task list, when a route shows it.
    pub fn scheduled_sidebar(&self) -> Option<&Entity<automations::ScheduledSidebar>> {
        self.scheduled.as_ref()
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

    /// Mobile `Get started` (`codex-mobile-setup-flow` `P`, `XNn`): the initial step shows its
    /// loading button while the host reads the account's MFA requirement. A non-`required`
    /// answer opens the allow-host step (whose `Allow` is disabled); `required` also reads
    /// `/accounts/mfa_info` and opens the MFA step unless `mfa_enabled_v2` is already on. A
    /// failed read or a body without the expected field returns to the initial step with its
    /// error.
    fn start_mobile_setup(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.mobile_page.clone() else {
            return;
        };
        page.update(cx, |page, cx| page.set_step(mobile::SetupStep::Pending, cx));
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let body = host::json_body(&host.fetch(&host::mfa_requirement())?)?;
            let requirement = body["requirement"]
                .as_str()
                .context("MFA requirement response has no requirement")?;
            if requirement != "required" {
                return Ok(mobile::SetupStep::AllowHost);
            }
            let info = host::json_body(&host.fetch(&host::mfa_info())?)?;
            let enabled = info["mfa_enabled_v2"]
                .as_bool()
                .context("MFA info response has no mfa_enabled_v2")?;
            anyhow::Ok(if enabled {
                mobile::SetupStep::AllowHost
            } else {
                mobile::SetupStep::MfaRequired
            })
        });
        cx.spawn(async move |_, cx| {
            let step = read.await.unwrap_or_else(|error| {
                eprintln!("codex-gpui: mobile setup: MFA read failed: {error:#}");
                mobile::SetupStep::Error
            });
            page.update(cx, |page, cx| page.set_step(step, cx));
        })
        .detach();
    }

    fn set_threads(&mut self, threads: &[Thread], cx: &mut Context<Self>) {
        self.threads = threads.to_vec();
        self.sidebar.update(cx, |s, cx| s.set_threads(threads, cx));
    }

    fn set_next_cursor(&mut self, cursor: Option<String>, cx: &mut Context<Self>) {
        self.loading_more = false;
        self.sidebar
            .update(cx, |s, cx| s.set_has_more(cursor.is_some(), cx));
        self.next_cursor = cursor;
    }

    /// The sidebar's end came into view past the loaded pages: the next `thread/list` page.
    fn load_more_threads(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = self.next_cursor.clone() else {
            return;
        };
        if self.loading_more {
            return;
        }
        self.loading_more = true;
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let page = bridge
                .request::<_, ThreadListResponse>(
                    "thread/list",
                    json!({ "cursor": cursor, "limit": 10 }),
                )
                .await;
            this.update(cx, |this, cx| {
                if this.signed_out() {
                    return;
                }
                this.loading_more = false;
                match page {
                    Ok(mut page) => {
                        let mut threads = std::mem::take(&mut this.threads);
                        let ids: HashSet<&str> = threads.iter().map(|t| t.id.as_str()).collect();
                        page.data.retain(|t| !ids.contains(t.id.as_str()));
                        threads.extend(page.data);
                        this.set_threads(&threads, cx);
                        this.set_next_cursor(page.next_cursor, cx);
                    }
                    Err(_) => this.sidebar.update(cx, |s, cx| s.page_failed(cx)),
                }
            })
        });
        log_err(task, cx);
    }

    /// Puts `thread` at the top of Recents.
    fn add_thread(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        if self.signed_out() {
            return;
        }
        self.threads.retain(|t| t.id != thread.id);
        self.threads.insert(0, thread.clone());
        self.sidebar.update(cx, |s, cx| s.add_thread(thread, cx));
    }

    /// `⌘N`, the `New chat` row and rail Home: Home with an empty, focused composer.
    fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat_section = None;
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

    /// HC-033: a printable key (no ⌘/⌃, not Space) typed outside an editable, with no
    /// dialog, menu or popover open, goes to the route's composer and focuses it.
    /// Content ancestors can hold plain focus handles without being popovers.
    /// The thread transcript is the composer's sibling, so only its exact focus is allowed.
    fn type_anywhere(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let Some(text) = keystroke.key_char.as_deref() else {
            return;
        };
        let printable = text.chars().count() == 1 && !text.chars().all(char::is_whitespace);
        let route_focus = self.route_focus(cx);
        let in_popover = window.focused(cx).is_some_and(|focus| {
            !focus.tab_stop
                && focus != self.focus
                && !focus.contains(&route_focus, window)
                && !self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.read(cx).transcript_focused(window))
        });
        let editing = window
            .context_stack()
            .iter()
            .any(|context| context.contains("Composer"));
        if !printable
            || keystroke.modifiers.platform
            || keystroke.modifiers.control
            || self.dialog.is_some()
            || self.command_menu.is_some()
            || route_focus == self.focus
            || in_popover
            || editing
        {
            return;
        }
        self.composer(cx).update(cx, |composer, cx| {
            composer.focus_handle(cx).focus(window, cx);
            composer.replace_text_in_range(None, text, window, cx);
        });
        cx.stop_propagation();
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
                let (_, command, _) = view.read(cx).terminal()?;
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
        let cwd = std::path::Path::new(view.read(cx).cwd())
            .components()
            .as_path()
            .to_string_lossy()
            .into_owned();
        let path = if action.path.starts_with('/') {
            action.path.clone()
        } else {
            format!("{cwd}/{}", action.path.trim_start_matches("./"))
        };
        let file = cx.new(|_| FileView::new(&cwd, path.clone(), action.line));
        // The original asks its Electron main for `vscode://codex/read-file`; the port's local
        // host is this process, so it reads the file itself.
        // ponytail: FileView loads the whole file; add paging there instead of a command-only size limit.
        let read = cx.background_spawn(async move {
            let contents = std::fs::read_to_string(&path).map_err(|error| error.to_string());
            (path, contents)
        });
        let target = file.clone();
        cx.spawn(async move |this, cx| {
            let (path, contents) = read.await;
            let failed = contents.is_err();
            target.update(cx, |file, cx| file.set_contents(contents, cx));
            if failed {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Could not open file"), cx)
                })
                .ok();
            } else {
                this.update(cx, |this, cx| this.remember_file(&cwd, &path, cx))
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
        if let Some(thread) = &self.thread {
            thread.update(cx, |view, cx| view.close_summary(cx));
        }
        let unread = self.sidebar.read(cx).unread.clone();
        let thread_open = matches!(self.state.route, Route::Thread(_));
        let cloud = self.chatgpt_account && self.host.supports_cloud_search();
        let menu = cx.new(|cx| {
            CommandMenu::with_threads(&self.threads, &unread, thread_open, "", cx)
                .with_cloud_search(cloud)
        });
        cx.subscribe_in(&menu, window, |this, _, event, window, cx| {
            this.run_command(event, window, cx)
        })
        .detach();
        window.focus(&menu.focus_handle(cx), cx);
        self.command_menu = Some(menu);
        self.refresh_git_actions(cx);
        cx.notify();
    }

    pub fn dialog(&self) -> Option<&Entity<Dialog>> {
        self.dialog.as_ref()
    }

    fn model_announcement_history(cx: &App) -> &[Value] {
        cx.try_global::<host_settings::Settings>()
            .and_then(|settings| settings.read_all().get("seen-model-upgrade-list"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn remember_model_announcement(&mut self, model: &str, cx: &mut Context<Self>) {
        let seen = Self::model_announcement_history(cx);
        if seen.iter().any(|seen| seen.as_str() == Some(model)) {
            return;
        }
        let mut seen = seen.to_vec();
        seen.push(json!(model));
        self.write_setting(
            &SettingChanged {
                path: "host.seen-model-upgrade-list",
                value: Value::Array(seen),
            },
            cx,
        );
    }

    fn open_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| Dialog::new(kind, cx));
        self.show_dialog(dialog, window, cx);
    }

    fn create_task(
        &mut self,
        prompt: String,
        rrule: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let host = self.host.clone();
        let name: String = prompt
            .lines()
            .next()
            .unwrap_or(&prompt)
            .chars()
            .take(80)
            .collect();
        let params = json!({"name": name, "prompt": prompt, "rrule": rrule, "cwds": [self.cwd]});
        let write = cx.background_spawn(async move {
            host.fetch(&host::Request::post("create-automation", Some(params)))
                .and_then(|response| host::json_body(&response))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = write.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.dialog = None;
                        window.focus(&this.focus, cx);
                        this.load_automations(cx);
                    }
                    Err(error) => dialog.update(cx, |dialog, cx| {
                        dialog.schedule_result(format!("Could not create task: {error}"), cx);
                    }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show_dialog(&mut self, dialog: Entity<Dialog>, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        self.command_menu = None;
        let git_dialog = matches!(
            dialog.read(cx).kind(),
            DialogKind::Commit | DialogKind::Branches | DialogKind::CreateBranch
        );
        if git_dialog
            && dialog.read(cx).kind() != DialogKind::Branches
            && let Some(thread) = &self.thread
        {
            thread.update(cx, |view, cx| view.close_summary(cx));
        }
        cx.subscribe_in(&dialog, window, move |this, dialog, outcome, window, cx| {
            if let DialogOutcome::ScheduleTask { prompt, rrule } = outcome {
                this.create_task(prompt.clone(), rrule.clone(), window, cx);
                return;
            }
            if let DialogOutcome::CheckoutBranch { name, create } = outcome {
                this.checkout_branch(dialog.clone(), name.clone(), *create, window, cx);
                return;
            }
            if let DialogOutcome::CommitBeforeCheckout = outcome {
                this.commit_before_checkout(dialog.clone(), window, cx);
                return;
            }
            if let DialogOutcome::Commit {
                step,
                message,
                include_unstaged,
                new_branch,
            } = outcome
                && let Some(target) = dialog.read(cx).checkout_target()
            {
                if this.git_busy.is_none()
                    && let Some(cwd) = this.commit_cwd.clone()
                {
                    dialog.update(cx, |dialog, cx| dialog.begin_checkout(target, cx));
                    this.git_workflow(
                        GitRequest {
                            cwd,
                            step: *step,
                            message: message.clone(),
                            include_unstaged: *include_unstaged,
                            new_branch: new_branch.clone(),
                        },
                        window,
                        cx,
                    );
                }
                return;
            }
            if dialog.read(cx).saving() {
                return;
            }
            dialog.read(cx).release_artwork(window);
            this.dialog = None;
            let focus = if std::mem::take(&mut this.dialog_to_composer) {
                this.composer(cx).focus_handle(cx)
            } else {
                this.focus.clone()
            };
            match outcome {
                DialogOutcome::Dismiss => {
                    if git_dialog {
                        this.commit_cwd = None;
                        window.focus(&this.composer(cx).focus_handle(cx), cx);
                    } else {
                        window.focus(&focus, cx);
                    }
                }
                DialogOutcome::LogOut => this.logout(window, cx),
                DialogOutcome::RevertChanges { .. } => window.focus(&this.focus, cx),
                DialogOutcome::DismissAnnouncement(model)
                | DialogOutcome::UseAnnouncedModel { model, .. } => {
                    if let DialogOutcome::UseAnnouncedModel {
                        reasoning_effort, ..
                    } = outcome
                    {
                        this.write_model(model, reasoning_effort, cx);
                    }
                    this.remember_model_announcement(model, cx);
                    window.focus(&this.focus, cx);
                }
                DialogOutcome::FullAccess => {
                    let composer = this.composer(cx);
                    composer.update(cx, |composer, cx| composer.confirm_full_access(cx));
                    window.focus(&composer.focus_handle(cx), cx);
                }
                DialogOutcome::CloseWindow { do_not_ask_again } => {
                    if *do_not_ask_again {
                        this.state.confirm_before_close = ConfirmBeforeClose::Never;
                        this.config["host"]["confirm-before-close"] = "never".into();
                        this.state.save();
                    }
                    this.flush_settings(cx);
                    windows::close(window, cx);
                }
                DialogOutcome::Rename { thread, name } => {
                    window.focus(&focus, cx);
                    this.rename(thread.clone(), name.clone(), window, cx);
                }
                DialogOutcome::SectionName { section, name } => {
                    window.focus(&focus, cx);
                    this.save_section(section.clone(), name.clone(), cx);
                }
                DialogOutcome::ArchiveSection(id) => {
                    window.focus(&focus, cx);
                    this.archive_section(id.clone(), window, cx);
                }
                DialogOutcome::Commit {
                    step,
                    message,
                    include_unstaged,
                    new_branch,
                } => {
                    window.focus(&this.composer(cx).focus_handle(cx), cx);
                    this.commit_include_unstaged = *include_unstaged;
                    if let Some(cwd) = this.commit_cwd.take() {
                        let request = GitRequest {
                            cwd,
                            step: *step,
                            message: message.clone(),
                            include_unstaged: *include_unstaged,
                            new_branch: new_branch.clone(),
                        };
                        this.git_workflow(request, window, cx);
                    }
                }
                DialogOutcome::ScheduleTask { .. }
                | DialogOutcome::CheckoutBranch { .. }
                | DialogOutcome::CommitBeforeCheckout => unreachable!(),
            }
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.dialog = Some(dialog);
        cx.notify();
    }

    fn logout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.logout_pending || self.signed_out() {
            return;
        }
        self.logout_pending = true;
        self.qa_work_started = true;
        window.focus(&self.route_focus(cx), cx);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = bridge
                .request::<_, Value>("account/logout", json!({}))
                .await;
            this.update_in(cx, |this, window, cx| {
                this.logout_pending = false;
                match result {
                    Ok(_) => {
                        this.login = Some(this.new_login(window, cx));
                        this.onboarding_load = None;
                        this.onboarding_context = None;
                        this.onboarding_page = None;
                        this.account_email = None;
                        this.chatgpt_account = false;
                        let host = this.host.clone();
                        cx.background_spawn(async move { host.invalidate_identity() })
                            .detach();
                        this.command_menu = None;
                        this.dialog = None;
                        this.thread = None;
                        this.diff = None;
                        this.file = None;
                        this.split = None;
                        this.tabs.clear();
                        this.side = None;
                        this.loaded.clear();
                        this.approvals.clear();
                        this.new_chat_section = None;
                        this.sidebar.update(cx, |s, cx| s.clear_sections(cx));
                        this.show_approval(cx);
                        this.set_threads(&[], cx);
                        this.set_next_cursor(None, cx);
                        this.automation_load = None;
                        this.project_load = None;
                        this.suggestion_load = None;
                        this.automations = None;
                        this.scheduled = None;
                        this.suggestions = None;
                        this.back.clear();
                        this.forward.clear();
                        this.navigation_epoch += 1;
                        this.sidebar_automation = None;
                        this.thread_run_settings.clear();
                        this.state.route = Route::Home;
                        this.toaster = cx.new(|_| Toaster::new());
                        for composer in [
                            this.home.read(cx).composer.clone(),
                            this.thread_composer.clone(),
                        ] {
                            composer.update(cx, |composer, cx| composer.clear_session(cx));
                        }
                        this.sync(window, cx);
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: logout: {error:#}");
                        this.toast(Toast::danger("Could not log out. Try again."), cx);
                        window.focus(&this.route_focus(cx), cx);
                    }
                }
                cx.notify();
            })
        });
        log_err(task, cx);
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
            self.flush_settings(cx);
            windows::close(window, cx);
        } else {
            self.open_dialog(DialogKind::CloseWindow, window, cx);
        }
    }

    /// Runs implemented commands through the same actions as the keyboard and menus.
    /// Unsupported destinations remain unfinished outside this dispatch slice.
    fn run_command(
        &mut self,
        event: &CommandMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommandMenuEvent::Search(request) => {
                return self.search_threads(request.clone(), window, cx);
            }
            CommandMenuEvent::RetryCloudSearch(request) => {
                return self.search_cloud_threads(request.clone(), window, cx);
            }
            _ => {}
        }
        if let CommandMenuEvent::SearchFiles(query) = event {
            return self.search_command_files(query.clone(), cx);
        }
        self.command_menu = None;
        window.focus(&self.route_focus(cx), cx);
        match event {
            CommandMenuEvent::Dismiss
            | CommandMenuEvent::Search(_)
            | CommandMenuEvent::RetryCloudSearch(_)
            | CommandMenuEvent::SearchFiles(_) => {}
            CommandMenuEvent::OpenUrl(url) => cx.open_url(url),
            CommandMenuEvent::OpenFile(path) => {
                self.open_file(
                    &OpenFile {
                        path: path.clone(),
                        line: None,
                    },
                    window,
                    cx,
                );
            }
            CommandMenuEvent::OpenThread(id) => {
                self.navigate(Route::Thread(id.clone()), window, cx)
            }
            CommandMenuEvent::Command(label) => match label.as_ref() {
                "New chat" | "New standalone chat" => window.dispatch_action(Box::new(NewChat), cx),
                "Search files" => self.open_file_search(window, cx),
                "Open in new window" => {
                    window.dispatch_action(Box::new(features::OpenThreadInNewWindow), cx)
                }
                "Open folder" => window.dispatch_action(Box::new(OpenFolder), cx),
                "Archive chat" => window.dispatch_action(Box::new(ArchiveChat), cx),
                "Find" => window.dispatch_action(Box::new(FindInThread), cx),
                "Back" => window.dispatch_action(Box::new(GoBack), cx),
                "Forward" => window.dispatch_action(Box::new(GoForward), cx),
                "Previous chat" => window.dispatch_action(Box::new(PreviousChat), cx),
                "Next chat" => window.dispatch_action(Box::new(NextChat), cx),
                "Go to recent chat 1" => window.dispatch_action(Box::new(GoToRecentChat(0)), cx),
                "Go to recent chat 2" => window.dispatch_action(Box::new(GoToRecentChat(1)), cx),
                "Go to recent chat 3" => window.dispatch_action(Box::new(GoToRecentChat(2)), cx),
                "Go to recent chat 4" => window.dispatch_action(Box::new(GoToRecentChat(3)), cx),
                "Go to recent chat 5" => window.dispatch_action(Box::new(GoToRecentChat(4)), cx),
                "Go to recent chat 6" => window.dispatch_action(Box::new(GoToRecentChat(5)), cx),
                "Toggle sidebar" => window.dispatch_action(Box::new(ToggleSidebar), cx),
                "Open Changes tab" => window.dispatch_action(Box::new(OpenChangesPanel), cx),
                "New tab" => window.dispatch_action(Box::new(ToggleChangesPanel), cx),
                "Open terminal" => window.dispatch_action(Box::new(OpenBackgroundTerminal), cx),
                "Commit or push" => self.open_git_form(DialogKind::Commit, window, cx),
                "Manage scheduled tasks" => window.dispatch_action(Box::new(OpenScheduled), cx),
                "Log out" => window.dispatch_action(Box::new(LogOut), cx),
                label => {
                    if let Some(section) = settings::section(label) {
                        self.open_settings_section(section, window, cx);
                    }
                }
            },
        }
        cx.notify();
    }

    /// The command menu's content search: `thread/search` for `term` (the original's
    /// `searchThreads`), answered into the open menu.
    fn search_threads(&self, request: SearchRequest, window: &Window, cx: &mut Context<Self>) {
        let Some(menu) = self.command_menu.clone() else {
            return;
        };
        if menu.read(cx).cloud_search_enabled() {
            self.search_cloud_threads(request.clone(), window, cx);
        }
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |_, cx| -> Result<()> {
            let result = async {
                let mut response: Value = bridge
                    .request(
                        "thread/search",
                        json!({
                            "cursor": null,
                            "limit": SEARCH_LIMIT,
                            "sortKey": "recency_at",
                            "archived": false,
                            "sourceKinds": [],
                            "searchTerm": request.term.as_ref(),
                        }),
                    )
                    .await?;
                Ok::<Vec<SearchHit>, anyhow::Error>(serde_json::from_value(
                    response["data"].take(),
                )?)
            }
            .await;
            let (hits, error) = match result {
                Ok(hits) => (hits, None),
                Err(error) => (Vec::new(), Some(error)),
            };
            cx.update(|_, cx| {
                menu.update(cx, |menu, cx| menu.set_search_results(&request, hits, cx))
            })?;
            error.map_or(Ok(()), Err)
        });
        log_err(task, cx);
    }

    fn search_cloud_threads(
        &self,
        request: SearchRequest,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.command_menu.clone() else {
            return;
        };
        let host = self.host.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let lookup = host.clone();
            let identity = cx.background_spawn(async move { lookup.identity() }).await;
            let result = if let Some(identity) = identity {
                let http = Arc::new(search::request(
                    &request.term,
                    &identity,
                    &host::device_id(),
                ));
                let mut result = Err(anyhow::anyhow!("ChatGPT search did not complete"));
                for attempt in 0..4 {
                    if attempt > 0 {
                        cx.background_executor()
                            .timer(Duration::from_secs(1 << (attempt - 1)))
                            .await;
                    }
                    if !this.update_in(cx, |root, _, cx| {
                        root.command_menu.as_ref() == Some(&menu)
                            && menu.read(cx).current_search(&request)
                    })? {
                        return Ok(());
                    }
                    let host = host.clone();
                    let http = http.clone();
                    // ponytail: Host reads are synchronous; stale answers and later retries are discarded.
                    // Add cancellable reads if overlapping searches exhaust the background pool.
                    result = cx
                        .background_spawn(async move {
                            host.fetch(&http)
                                .and_then(|response| host::json_body(&response))
                                .and_then(search::results)
                        })
                        .await;
                    if result.is_ok() {
                        break;
                    }
                }
                result
            } else {
                Err(anyhow::anyhow!("No authenticated workspace is available"))
            };
            this.update_in(cx, |root, _, cx| {
                if root.command_menu.as_ref() == Some(&menu) {
                    menu.update(cx, |menu, cx| {
                        menu.set_cloud_search_results(&request, result.map_err(|_| ()), cx)
                    });
                }
            })?;
            Ok(())
        });
        log_err(task, cx);
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
        if self.signed_out() {
            return;
        }
        let running = thread
            .turns
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress);
        // The composer is shared; load the selected thread's own queue after navigation.
        self.thread_composer.update(cx, |c, cx| {
            c.set_running(running, cx);
            c.set_queued(Vec::new(), cx);
            c.set_queue_pending(self.queue_pending, cx);
            c.set_search_roots(vec![thread.cwd.clone()], cx);
        });
        if self
            .thread
            .as_ref()
            .is_none_or(|view| view.read(cx).id() != thread.id.as_str())
        {
            if let Some(state) = self.thread_run_settings.get(&thread.id) {
                self.thread_composer.update(cx, |c, cx| {
                    c.restore_run_settings(&state.collaboration_mode, state.permission, cx)
                });
            } else if let Some(model) = thread.model.as_deref() {
                let effort = thread
                    .reasoning_effort
                    .as_deref()
                    .or_else(|| self.config["model_reasoning_effort"].as_str())
                    .unwrap_or("medium");
                self.thread_composer
                    .update(cx, |c, cx| c.set_model_and_effort(model, effort, cx));
            }
        }
        let route = Route::Thread(thread.id.clone());
        let clock = self.clock.unwrap_or_else(|| Clock {
            utc_offset: utc_offset(),
            pinned_now: None,
            elapsed_since: None,
        });
        let composer = self.thread_composer.clone().into();
        let diff = cx.new(|_| DiffPanel::last_turn(&self.cwd, &thread.cwd, &thread.turns));
        let cwd = thread.cwd.clone();
        cx.subscribe_in(
            &diff,
            window,
            move |this, panel, request: &DiffRequest, window, cx| match request {
                DiffRequest::Revert(_) => {
                    this.confirm_revert(panel.clone(), &cwd, request, window, cx)
                }
                DiffRequest::Changes {
                    action: "revert", ..
                } => this.confirm_revert(panel.clone(), &cwd, request, window, cx),
                DiffRequest::Open(path) => this.open_in_editor(&cwd, path, cx),
                _ => this.repository_request(panel.clone(), &cwd, request, cx),
            },
        )
        .detach();
        self.diff = Some(diff);
        self.file = None;
        let view = cx.new(|_| ThreadView::new(thread, clock, composer));
        view.update(cx, |view, cx| view.set_git_phase(self.git_busy, cx));
        cx.subscribe_in(
            &view,
            window,
            |this, _, action: &GitAction, window, cx| match action {
                GitAction::Refresh => this.refresh_git_actions(cx),
                GitAction::OpenForm => this.open_git_form(DialogKind::Commit, window, cx),
                GitAction::CreateBranch => this.open_git_form(DialogKind::CreateBranch, window, cx),
                GitAction::SwitchBranch => this.open_git_form(DialogKind::Branches, window, cx),
            },
        )
        .detach();
        self.thread = Some(view);
        self.show_approval(cx);
        self.refresh_git_actions(cx);
        if self.state.route == route {
            self.sync(window, cx);
        } else {
            self.navigate(route, window, cx);
        }
    }

    /// Home composer: `thread/start` with the chosen model and permissions, open it, then
    /// `turn/start`; the turn streams in through [`Self::on_event`].
    fn start_thread(&mut self, s: Submission, window: &mut Window, cx: &mut Context<Self>) {
        self.qa_work_started = true;
        let bridge = self.bridge.clone();
        let cwd = self
            .state
            .project_root
            .as_ref()
            .unwrap_or(&self.cwd)
            .clone();
        let section = self.new_chat_section.clone();
        let computer_use = computer_use_overrides(&self.config);
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let mut params = json!({ "cwd": cwd, "model": model(&s) });
            apply_permissions(&mut params, s.permission);
            if let Some(config) = computer_use {
                params["config"] = config;
            }
            let mut thread = match bridge
                .request::<_, ThreadStartResponse>("thread/start", params)
                .await
            {
                Ok(started) => started.thread,
                Err(error) => {
                    // The draft, its attachments and the chosen project stay as they were.
                    this.update(cx, |this, cx| {
                        let composer = this.home.read(cx).composer.clone();
                        composer.update(cx, |c, cx| c.restore_submission(&s.text, cx));
                        this.toast(Toast::danger("Error starting chat"), cx);
                    })?;
                    return Err(error);
                }
            };
            // Title the new chat (Recents row, header) with its prompt until it has a preview.
            if thread.preview.is_empty() {
                thread.preview = s.text.clone();
            }
            let id = thread.id.clone();
            let section = if let Some(section) = section {
                match bridge
                    .request::<_, Value>(
                        "thread/section/move",
                        json!({
                            "threadId": id, "sectionId": section, "beforeThreadId": null
                        }),
                    )
                    .await
                {
                    Ok(_) => {
                        thread.section = Some(ThreadSection {
                            id: section.clone(),
                        });
                        Some(section)
                    }
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.toast(Toast::danger("Couldn't move chat to section"), cx)
                        })?;
                        None
                    }
                }
            } else {
                None
            };
            this.update_in(cx, |this, window, cx| {
                this.new_chat_section = None;
                this.loaded.insert(id.clone());
                this.add_thread(&thread, cx);
                if let Some(section) = &section {
                    this.sidebar
                        .update(cx, |s, cx| s.assign_section(&id, section, cx));
                }
                this.show_thread(thread, window, cx);
            })?;
            turn_start(&bridge, &id, &s, user_input(&s)).await
        });
        log_err(task, cx);
    }

    /// Thread composer: `turn/start` on the open thread, resuming it first if needed. While a
    /// turn runs (CP-108), `Follow-up behavior` decides: `Queue` adds the message to the
    /// app-server's queue (`thread/queue/add`, drained by the server when the turn ends),
    /// `Steer` sends it into the turn (`turn/steer`). A queued draft taken back by `Edit
    /// message` returns to its place in the queue. A refusal shows `Error submitting
    /// message` and puts the draft back.
    fn send(&mut self, s: Submission, window: &mut Window, cx: &mut Context<Self>) {
        self.qa_work_started = true;
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
        let edit = self.queue_edit.take().filter(|edit| edit.thread_id == id);
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            resume_if(resume, &bridge, &this, &id, cx).await?;
            let mut input = user_input(&s);
            input.extend(edit.iter().flat_map(|edit| edit.extra.clone()));
            let Some(turn_id) = running else {
                return turn_start(&bridge, &id, &s, input).await;
            };
            let (method, params) = if queue {
                let params = json!({
                    "threadId": id, "input": input, "clientUserMessageId": uuid_v4(),
                });
                ("thread/queue/add", params)
            } else {
                let params = json!({ "threadId": id, "input": input, "expectedTurnId": turn_id });
                ("turn/steer", params)
            };
            let result = bridge.request::<_, Value>(method, params).await;
            if queue
                && let (Ok(added), Some(edit)) = (&result, &edit)
                && let Some(added) = added["queuedSubmission"]["id"].as_str()
                && let Err(error) = restore_place(&bridge, &id, added, edit).await
            {
                eprintln!("codex-gpui: thread/queue/reorder: {error:#}");
            }
            this.update(cx, |this, cx| match &result {
                Ok(_) if queue => this.list_queue(id, cx),
                Ok(_) => {}
                Err(_) => {
                    this.queue_edit = edit;
                    this.thread_composer.update(cx, |c, cx| {
                        c.restore_text(&s.text, cx);
                        c.drop_files(&s.attachments, cx);
                    });
                    this.toast(Toast::danger("Error submitting message"), cx);
                }
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }

    fn list_queue(&mut self, id: String, cx: &mut Context<Self>) {
        if self.queue_pending
            || !matches!(&self.state.route, Route::Thread(current) if current == &id)
        {
            return;
        }
        let bridge = self.bridge.clone();
        self.queue_load = Some(cx.spawn(async move |this, cx| {
            let result = read_queue(&bridge, &id).await;
            this.update(cx, |this, cx| match result {
                Ok(rows) if matches!(&this.state.route, Route::Thread(current) if current == &id) =>
                {
                    this.thread_composer
                        .update(cx, |composer, cx| composer.set_queued(rows, cx));
                }
                Ok(_) => {}
                Err(error) => eprintln!("codex-gpui: failed to read queued messages: {error:#}"),
            })
            .ok();
        }));
    }

    fn queue_action(&mut self, action: &QueueAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            QueueAction::Edit(id) => return self.edit_queued(id.clone(), window, cx),
            QueueAction::Queueing(queue) => return self.set_queueing(*queue, cx),
            _ => {}
        }
        if self.queue_pending {
            return;
        }
        let Route::Thread(thread_id) = self.state.route.clone() else {
            return;
        };
        let (QueueAction::Steer(id) | QueueAction::Delete(id)) = action else {
            return;
        };
        let rows = self.thread_composer.read(cx).queued();
        let Some(at) = rows.iter().position(|row| row.submission.id == *id) else {
            return;
        };
        let deleted = DeletedQueuedMessage {
            thread_id: thread_id.clone(),
            submission: rows[at].submission.clone(),
            previous: at
                .checked_sub(1)
                .map(|i| rows[i].submission.client_user_message_id.clone()),
            next: rows
                .get(at + 1)
                .map(|row| row.submission.client_user_message_id.clone()),
        };
        let steer = matches!(action, QueueAction::Steer(_));
        let turn_id = self.thread.as_ref().and_then(|view| {
            view.read(cx)
                .turns()
                .iter()
                .rev()
                .find(|turn| turn.status == TurnStatus::InProgress)
                .map(|turn| turn.id.clone())
        });
        let key = (
            thread_id.clone(),
            deleted.submission.client_user_message_id.clone(),
        );
        let already_steered = self.steered_queued.contains(&key);
        let resume = !self.loaded.contains(&thread_id);
        self.begin_queue_action(cx);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = async {
                resume_if(resume, &bridge, &this, &thread_id, cx).await?;
                if steer && !already_steered {
                    if let Some(turn_id) = turn_id {
                        bridge.request::<_, Value>("turn/steer", json!({
                            "threadId": thread_id,
                            "input": deleted.submission.input,
                            "clientUserMessageId": deleted.submission.client_user_message_id,
                            "expectedTurnId": turn_id,
                        })).await?;
                        this.update(cx, |this, _| {
                            this.steered_queued.insert(key.clone());
                        })?;
                    } else {
                        bridge.request::<_, Value>("thread/queue/start", json!({
                            "threadId": thread_id, "queuedSubmissionId": deleted.submission.id,
                        })).await?;
                        return Ok(());
                    }
                }
                let response: Value = bridge
                    .request(
                        "thread/queue/delete",
                        json!({
                            "threadId": thread_id, "queuedSubmissionId": deleted.submission.id,
                        }),
                    )
                    .await?;
                ensure!(
                    response["deleted"] == true,
                    "The queued message was not deleted"
                );
                this.update(cx, |this, _| {
                    let accepted = this.steered_queued.remove(&key);
                    if !steer && !accepted {
                        this.deleted_queued.push(deleted.clone());
                    }
                })?;
                Ok(())
            }
            .await;
            this.update(cx, |this, cx| {
                if result.is_ok()
                    && matches!(&this.state.route, Route::Thread(current) if current == &thread_id)
                {
                    this.thread_composer.update(cx, |composer, cx| {
                        composer.remove_queued(&deleted.submission.id, cx);
                    });
                }
                if result.is_err() {
                    this.toast(
                        Toast::danger(if steer {
                            "Error submitting message"
                        } else {
                            "Failed to delete queued message"
                        }),
                        cx,
                    );
                }
                this.finish_queue_action(cx);
            })?;
            result
        });
        log_err(task, cx);
    }

    fn begin_queue_action(&mut self, cx: &mut Context<Self>) {
        self.queue_pending = true;
        self.queue_load = None;
        self.thread_composer
            .update(cx, |composer, cx| composer.set_queue_pending(true, cx));
    }

    fn finish_queue_action(&mut self, cx: &mut Context<Self>) {
        self.queue_pending = false;
        self.thread_composer
            .update(cx, |composer, cx| composer.set_queue_pending(false, cx));
        if let Route::Thread(id) = self.state.route.clone() {
            self.list_queue(id, cx);
        }
    }

    fn undo_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(deleted) = self.deleted_queued.last().cloned() else {
            cx.propagate();
            return;
        };
        cx.stop_propagation();
        if self.queue_pending {
            return;
        }
        self.begin_queue_action(cx);
        self.navigate(Route::Thread(deleted.thread_id.clone()), window, cx);
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = restore_queued(&bridge, &deleted).await;
            this.update(cx, |this, cx| {
                match &result {
                    Ok(()) => {
                        this.deleted_queued.pop();
                        this.toast(Toast::info("Queued message restored"), cx);
                    }
                    Err(_) => this.toast(Toast::danger("Failed to restore queued message"), cx),
                }
                this.finish_queue_action(cx);
            })?;
            result
        });
        log_err(task, cx);
    }

    /// `Edit message` on a queued row: `thread/queue/delete` takes the submission out of the
    /// queue, then its text and images go back to the composer. Sending the draft queues it
    /// again where it was ([`restore_place`]). A submission already gone only refreshes the
    /// tray; one the composer cannot show stays queued.
    fn edit_queued(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_pending {
            return;
        }
        let Some(view) = &self.thread else { return };
        let thread_id = view.read(cx).id().to_owned();
        let composer = self.thread_composer.read(cx);
        let rows = composer.queued();
        let Some(index) = rows.iter().position(|row| row.submission.id == id) else {
            return;
        };
        let neighbour = |at: Option<usize>| {
            at.and_then(|i| rows.get(i))
                .map(|r| r.submission.id.clone())
        };
        let (previous, next) = (neighbour(index.checked_sub(1)), neighbour(Some(index + 1)));
        let (text, images, extra) = split_input(&rows[index].submission.input);
        if text.is_empty() && images.is_empty() {
            self.toast(Toast::danger("Couldn’t edit this queued message"), cx);
            return;
        }
        self.begin_queue_action(cx);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let params = json!({ "threadId": thread_id, "queuedSubmissionId": id });
            let result = bridge
                .request::<_, Value>("thread/queue/delete", params)
                .await;
            this.update_in(cx, |this, window, cx| {
                match &result {
                    Ok(taken) if taken["deleted"] == true => {
                        this.queue_edit = Some(QueueEdit {
                            thread_id: thread_id.clone(),
                            previous,
                            next,
                            extra,
                        });
                        this.thread_composer
                            .update(cx, |c, cx| c.load_queued(&text, &images, window, cx));
                    }
                    Ok(_) => {}
                    Err(_) => this.toast(Toast::danger("Couldn’t edit this queued message"), cx),
                }
                this.finish_queue_action(cx);
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }

    /// `Turn off queueing` / `Turn on queueing`: `Follow-up behavior` as the Settings row
    /// writes it.
    fn set_queueing(&mut self, queue: bool, cx: &mut Context<Self>) {
        let value = if queue { "queue" } else { "steer" };
        self.write_setting(
            &SettingChanged {
                path: "desktop.followUpQueueMode",
                value: value.into(),
            },
            cx,
        );
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
        if !self
            .interrupting
            .insert((thread_id.clone(), turn_id.clone()))
        {
            return;
        }
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = bridge.turn_interrupt(&thread_id, &turn_id).await;
            if result.is_err() {
                this.update(cx, |this, cx| {
                    this.interrupting.remove(&(thread_id, turn_id));
                    this.toast(Toast::danger("Failed to stop chat"), cx)
                })?;
            }
            result
        });
        log_err(task, cx);
    }

    fn stop_background_terminals(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.thread.clone() else {
            return;
        };
        let view = thread.read(cx);
        if view.terminals_stopping || view.background_terminal_items().next().is_none() {
            return;
        }
        let id = view.id().to_owned();
        thread.update(cx, |thread, cx| {
            thread.terminals_stopping = true;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result: Result<Value> = bridge
                .request(
                    "thread/backgroundTerminals/clean",
                    json!({ "threadId": id }),
                )
                .await;
            this.update(cx, |this, cx| {
                thread.update(cx, |thread, cx| {
                    thread.terminals_stopping = false;
                    cx.notify();
                });
                if result.is_err() {
                    this.toast(Toast::danger("Unable to stop background terminals"), cx);
                }
            })?;
            result.map(drop)
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
        let Some(later) = view
            .read(cx)
            .turns()
            .iter()
            .rev()
            .position(|t| t.id == turn_id)
        else {
            return;
        };
        self.fork_chat(later, window, cx);
    }

    /// `thread/fork` of the open chat in its workspace, `thread/rollback` of its last `later`
    /// turns, then opens the new chat; a refusal shows `Failed to fork chat`.
    fn fork_chat(&mut self, later: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let id = view.read(cx).id().to_owned();
        let bridge = self.bridge.clone();
        let computer_use = computer_use_overrides(&self.config);
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let forked = async {
                let mut params = json!({ "threadId": id });
                if let Some(config) = computer_use {
                    params["config"] = config;
                }
                // Both answer `{ thread }`, like `thread/read`.
                let mut read: ThreadReadResponse = bridge.request("thread/fork", params).await?;
                if later > 0 {
                    let params = json!({ "threadId": read.thread.id, "numTurns": later });
                    read = bridge.request("thread/rollback", params).await?;
                }
                Ok::<_, anyhow::Error>(read)
            }
            .await;
            match forked {
                Ok(read) => this.update_in(cx, |this, window, cx| {
                    this.loaded.insert(read.thread.id.clone());
                    this.add_thread(&read.thread, cx);
                    this.show_thread(read.thread, window, cx);
                }),
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.toast(Toast::danger("Failed to fork chat"), cx)
                    })?;
                    Err(error)
                }
            }
        });
        log_err(task, cx);
    }

    /// `/compact`: `thread/compact/start` on the open chat, resumed first when this app-server
    /// has not loaded it. A running turn refuses it; a failed request shows `Failed to compact chat`.
    fn compact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.thread else { return };
        let view = view.read(cx);
        let id = view.id().to_owned();
        let running = view
            .turns()
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress);
        if running {
            self.toast(
                Toast::danger("Compact is disabled while a chat is in progress"),
                cx,
            );
            return;
        }
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let started = async {
                resume_if(resume, &bridge, &this, &id, cx).await?;
                bridge
                    .request::<_, Value>("thread/compact/start", json!({ "threadId": id }))
                    .await
                    .map(drop)
            }
            .await;
            if started.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Failed to compact chat"), cx)
                })?;
            }
            started
        });
        log_err(task, cx);
    }

    /// Opens `Rename chat` for `id`. From the composer, closing it returns focus there.
    fn open_rename(
        &mut self,
        id: String,
        from_composer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog.is_some() {
            return;
        }
        let title = self.threads.iter().find(|t| t.id == id).map(thread_title);
        let title = title.unwrap_or_default().to_string();
        let dialog = cx.new(|cx| Dialog::rename(id, title, cx));
        self.show_dialog(dialog, window, cx);
        self.dialog_to_composer = from_composer;
    }

    /// `thread/section/move` into or out of the Pinned section: the row moves at once (no toast);
    /// a refused write puts it back, silently, as the original does.
    fn set_pinned(
        &mut self,
        id: String,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == id) else {
            return;
        };
        let section = pinned.then(|| ThreadSection {
            id: PINNED_SECTION_ID.to_owned(),
        });
        let old_section = std::mem::replace(&mut thread.section, section);
        self.sidebar
            .update(cx, |s, cx| s.set_threads(&self.threads, cx));
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let done = bridge
                .request::<_, Value>(
                    "thread/section/move",
                    json!({ "threadId": id, "sectionId": pinned.then_some(PINNED_SECTION_ID), "beforeThreadId": null }),
                )
                .await;
            this.update(cx, |this, cx| {
                if done.is_err() {
                    if let Some(thread) = this.threads.iter_mut().find(|thread| thread.id == id) {
                        thread.section = old_section;
                    }
                    this.sidebar
                        .update(cx, |s, cx| s.set_threads(&this.threads, cx));
                }
            })?;
            done.map(drop)
        });
        log_err(task, cx);
    }

    /// `Rename chat`'s Save: `thread/name/set`; the row (and the open chat's header) take the
    /// name once the write succeeds, else `Couldn't save your changes. Try again`.
    fn rename(&mut self, id: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let done = bridge
                .request::<_, Value>("thread/name/set", json!({ "threadId": id, "name": name }))
                .await;
            this.update(cx, |this, cx| match &done {
                Ok(_) => {
                    if let Some(thread) = this.threads.iter_mut().find(|thread| thread.id == id) {
                        thread.name = Some(name);
                    }
                    this.sidebar
                        .update(cx, |s, cx| s.set_threads(&this.threads, cx));
                }
                Err(_) => this.toast(Toast::danger("Couldn't save your changes. Try again"), cx),
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
                            view.update(cx, |this, cx| this.navigate(Route::Archive, window, cx))
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
                        if let Some(page) = &this.archive_page {
                            page.update(cx, |page, cx| page.unarchived(&thread.id, cx));
                        }
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

    fn load_computer_use(&self, cx: &mut Context<Self>) {
        if cx.global::<ComputerUseState>().pending {
            return;
        }
        cx.update_global::<ComputerUseState, _>(|state, _| {
            state.pending = true;
            state.error = None;
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let service = host
                .computer_use_settings()
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((
                service.readiness().map_err(|error| error.to_string()),
                service
                    .get_app_approvals()
                    .map_err(|error| error.to_string()),
                service.get_sound_mode().map_err(|error| error.to_string()),
            ))
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            cx.update_global::<ComputerUseState, _>(|state, _| {
                state.pending = false;
                match result {
                    Ok((readiness, approvals, sound)) => {
                        state.plugin = match readiness {
                            Ok(readiness) => {
                                Load::Ready(readiness.supported.then_some(ComputerUsePlugin {
                                    installed: readiness.relay_installed,
                                    enabled: readiness.provider_registered,
                                }))
                            }
                            Err(error) => Load::Failed(error),
                        };
                        state.approvals = match approvals {
                            Ok(approvals) => Load::Ready(
                                approvals
                                    .into_iter()
                                    .flat_map(|approvals| approvals.approved_apps)
                                    .map(|app| ComputerUseAppApproval {
                                        bundle_identifier: app.bundle_identifier,
                                        display_name: app.display_name,
                                        icon_data_url: app.icon_data_url,
                                    })
                                    .collect(),
                            ),
                            Err(error) => Load::Failed(error),
                        };
                        match sound {
                            Ok(sound) => {
                                state.sound = sound.map(|mode| {
                                    match mode {
                                host::computer_use::SoundMode::ForegroundClicks => {
                                    ComputerUseSoundMode::ForegroundClicks
                                }
                                host::computer_use::SoundMode::ForegroundAndBackgroundClicks => {
                                    ComputerUseSoundMode::ForegroundAndBackgroundClicks
                                }
                                host::computer_use::SoundMode::Off => ComputerUseSoundMode::Off,
                            }
                                })
                            }
                            Err(error) => state.error = Some(error),
                        }
                    }
                    Err(error) => {
                        state.plugin = Load::Failed(error.clone());
                        state.approvals = Load::Failed(error);
                    }
                }
            });
        })
        .detach();
    }

    fn computer_use_action(&mut self, action: ComputerUseAction, cx: &mut Context<Self>) {
        if matches!(action, ComputerUseAction::Refresh) {
            self.load_computer_use(cx);
            return;
        }
        if matches!(action, ComputerUseAction::OpenPermissions) {
            cx.open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            );
            return;
        }
        let state = cx.global::<ComputerUseState>();
        if state.pending || !state.available {
            return;
        }
        if matches!(action, ComputerUseAction::SetEnabled(true))
            && !matches!(&state.plugin, Load::Ready(Some(plugin)) if plugin.installed)
        {
            cx.update_global::<ComputerUseState, _>(|state, _| {
                state.error = Some("Install Computer Use before enabling it.".into());
            });
            return;
        }
        let reload = matches!(
            action,
            ComputerUseAction::Install | ComputerUseAction::SetEnabled(_)
        );
        if reload
            && (self
                .threads
                .iter()
                .any(|thread| thread.status == ThreadStatus::Active)
                || !self.approvals.is_empty())
        {
            cx.update_global::<ComputerUseState, _>(|state, _| {
                state.error = Some("Stop active chats before changing Computer Use.".into());
            });
            return;
        }
        cx.update_global::<ComputerUseState, _>(|state, _| {
            state.pending = true;
            state.error = None;
        });
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let operation = cx.background_spawn(async move {
            match action {
                ComputerUseAction::Install => bridge::setup_computer_use()?,
                ComputerUseAction::SetEnabled(enabled) => {
                    let listed: Value = bridge.request("hooks/list", json!({})).await?;
                    let hooks: Vec<&Value> = listed["data"].as_array().into_iter().flatten()
                        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
                        .filter(|hook| hook["server"] == "lcu" && hook["tool"] == "turn_ended")
                        .collect();
                    if enabled {
                        for event in ["stop", "interrupt", "subagentStop"] {
                            ensure!(hooks.iter().filter(|hook| hook["eventName"] == event
                                && hook["trustStatus"] == "trusted").count() == 1,
                                "Run Computer Use setup to install trusted lifecycle hooks");
                        }
                    }
                    let mut edits = vec![json!({
                        "keyPath": "mcp_servers.lcu.enabled", "value": enabled,
                        "mergeStrategy": "upsert",
                    })];
                    for hook in hooks {
                        let key = hook["key"].as_str().context("Computer Use hook has no key")?;
                        edits.push(json!({
                            "keyPath": format!("hooks.state.{}.enabled", serde_json::to_string(key)?),
                            "value": enabled, "mergeStrategy": "upsert",
                        }));
                    }
                    bridge.request::<_, Value>("config/batchWrite", json!({
                        "edits": edits, "reloadUserConfig": true,
                    })).await?;
                }
                ComputerUseAction::SetSoundMode(mode) => {
                    let mode = match mode {
                        ComputerUseSoundMode::ForegroundClicks => host::computer_use::SoundMode::ForegroundClicks,
                        ComputerUseSoundMode::ForegroundAndBackgroundClicks => host::computer_use::SoundMode::ForegroundAndBackgroundClicks,
                        ComputerUseSoundMode::Off => host::computer_use::SoundMode::Off,
                    };
                    host.computer_use_settings()?.set_sound_mode(mode)?;
                }
                ComputerUseAction::RemoveApproval(bundle) => {
                    host.computer_use_settings()?.remove_app_approval(&bundle)?;
                }
                ComputerUseAction::Refresh | ComputerUseAction::OpenPermissions => unreachable!(),
            }
            let config = if reload {
                bridge.request::<_, Value>("config/mcpServer/reload", json!({})).await?;
                Some(bridge.config_read().await?.config)
            } else {
                None
            };
            Ok::<_, anyhow::Error>(config)
        });
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            let success = result.is_ok();
            this.update(cx, |this, cx| {
                cx.update_global::<ComputerUseState, _>(|state, _| {
                    state.pending = false;
                    state.error = result.as_ref().err().map(|error| error.to_string());
                });
                if let Ok(Some(config)) = result {
                    this.config = config;
                }
                if success {
                    this.load_computer_use(cx);
                }
            })
            .ok();
        })
        .detach();
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

    /// `recentThread1..6`: the `n`th chat by last update, newest first.
    fn go_to_recent_chat(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut recent: Vec<&Thread> = self.threads.iter().collect();
        recent.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        if let Some(id) = recent.get(n).map(|t| t.id.clone()) {
            self.navigate(Route::Thread(id), window, cx);
        }
    }

    /// A model choice: `config/batchWrite` of `model` and `model_reasoning_effort`
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
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let done = bridge
                .request::<_, Value>("config/batchWrite", params)
                .await;
            this.update(cx, |this, cx| {
                if done.is_ok() {
                    let composers = [
                        this.home.read(cx).composer.clone(),
                        this.thread_composer.clone(),
                    ];
                    for composer in composers {
                        composer.update(cx, |composer, cx| {
                            composer.set_model_and_effort(&model, &effort, cx);
                        });
                    }
                    this.config["model"] = model.into();
                    this.config["model_reasoning_effort"] = effort.into();
                } else {
                    this.toast(Toast::danger("Couldn’t update model settings"), cx);
                }
            })?;
            done.map(drop)
        });
        log_err(task, cx);
    }

    fn flush_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.settings_page.clone() {
            let pending = page.update(cx, |page, _| page.take_pending_instructions());
            for change in pending {
                self.write_setting(&change, cx);
            }
        }
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
                let message = match key {
                    "git-branch-prefix" => "Failed to save branch prefix",
                    "git-pull-request-merge-method" => "Failed to save pull request merge method",
                    "git-always-force-push" => "Failed to save force push setting",
                    "git-create-pull-request-as-draft" => {
                        "Failed to save draft pull request setting"
                    }
                    "git-review-mode" => "Could not save Git diff setting",
                    "git-pr-watch-auto-merge" => "Failed to save automatic PR merging",
                    "git-commit-instructions"
                    | "git-pr-instructions"
                    | "git-pr-watch-instructions" => "Failed to save instructions",
                    _ => "Couldn't save your changes. Try again",
                };
                self.toast(Toast::danger(message), cx);
            } else {
                let message = match key {
                    "git-branch-prefix" => Some("Saved branch prefix".to_owned()),
                    "git-pull-request-merge-method" => {
                        Some("Saved pull request merge method".to_owned())
                    }
                    "git-always-force-push" => Some(format!(
                        "Always force push {}",
                        if e.value == true {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )),
                    "git-create-pull-request-as-draft" => Some(format!(
                        "Create draft pull requests {}",
                        if e.value == true {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )),
                    "git-pr-watch-auto-merge" => Some(format!(
                        "Automatic PR merging {}",
                        if e.value == true {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )),
                    _ => None,
                };
                if let Some(message) = message {
                    self.toast(Toast::info(message), cx);
                }
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
        &mut self,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Value) + 'static,
    ) {
        self.qa_work_started = true;
        let bridge = self.bridge.clone();
        let review_delivery = params["keyPath"] == "desktop.reviewDelivery";
        let queue_mode = params["keyPath"] == "desktop.followUpQueueMode";
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            if let Err(error) = bridge.request::<_, Value>(method, params).await {
                if review_delivery {
                    this.update(cx, |this, cx| {
                        this.toast(
                            Toast::danger("Could not save review delivery preference"),
                            cx,
                        );
                    })?;
                }
                if queue_mode {
                    // The composer shows the behavior that still applies.
                    this.update(cx, |this, cx| {
                        let queue = this.config["desktop"]["followUpQueueMode"] == "queue";
                        this.thread_composer
                            .update(cx, |c, cx| c.set_queue_mode(queue, cx));
                        this.toast(Toast::danger("Couldn’t save your changes. Try again"), cx);
                    })?;
                }
                return Err(error);
            }
            this.update(cx, |this, _| apply(&mut this.config))
        });
        log_err(task, cx);
    }

    /// Settles only the request whose card produced the answer.
    fn answer(&mut self, id: Value, thread: &str, response: Value, cx: &mut Context<Self>) {
        let Some(i) = self
            .approvals
            .iter()
            .position(|p| p.id == id && p.thread() == thread)
        else {
            return;
        };
        if self.approvals[i].interrupting {
            return;
        }
        if response.is_null() && self.approvals[i].method == "item/tool/requestUserInput" {
            self.approvals[i].interrupting = true;
            let params =
                json!({ "threadId": thread, "turnId": self.approvals[i].params["turnId"] });
            let thread = thread.to_owned();
            let bridge = self.bridge.clone();
            cx.spawn(async move |this, cx| {
                let result = bridge.request::<_, Value>("turn/interrupt", params).await;
                this.update(cx, |this, cx| {
                    if let Some(i) = this
                        .approvals
                        .iter()
                        .position(|p| p.id == id && p.thread() == thread)
                    {
                        if let Err(error) = result {
                            this.approvals[i].interrupting = false;
                            this.toast(
                                Toast::danger(format!("Could not stop task: {error:#}")),
                                cx,
                            );
                        } else {
                            this.approvals.remove(i);
                        }
                        this.show_approval(cx);
                    }
                })
                .ok();
            })
            .detach();
            self.show_approval(cx);
            return;
        }
        let pending = self.approvals.remove(i);
        let native_accept =
            computer_use_requests::accepted(&pending.method, &pending.params, &response);
        if let Err(err) = self.bridge.respond(pending.id, response) {
            eprintln!("codex-gpui: approval: {err:#}");
        } else if native_accept {
            computer_use_requests::remember_acceptance(cx);
        }
        self.show_approval(cx);
    }

    /// The card of `pending`; `None` for another request kind or params that do not parse.
    fn approval_card(&self, pending: &PendingApproval, cx: &App) -> Option<Approval> {
        let params = pending.params.clone();
        match pending.method.as_str() {
            "item/commandExecution/requestApproval" => {
                let request = serde_json::from_value::<CommandApprovalRequest>(params).ok()?;
                return Some(Approval::command(pending.id.clone(), &request));
            }
            "item/permissions/requestApproval" => {
                let request = serde_json::from_value::<PermissionsApprovalRequest>(params).ok()?;
                return Some(Approval::permissions(pending.id.clone(), &request));
            }
            "mcpServer/elicitation/request" => {
                return self
                    .computer_use_approval(pending, cx)
                    .or_else(|| Approval::form(pending.id.clone(), &pending.method, &params));
            }
            "item/fileChange/requestApproval" => {}
            _ => return Approval::form(pending.id.clone(), &pending.method, &params),
        }
        let request = serde_json::from_value::<FileChangeApprovalRequest>(params).ok()?;
        // The edited paths come from the request's thread, when it is the open one.
        let paths: Vec<String> = self
            .thread
            .as_ref()
            .map(|v| v.read(cx))
            .filter(|view| view.id() == request.thread_id)
            .and_then(|view| {
                view.turns()
                    .iter()
                    .flat_map(|t| &t.items)
                    .find_map(|i| match i {
                        ThreadItem::FileChange { id, changes, .. } if *id == request.item_id => {
                            Some(changes.iter().map(|c| c.path.clone()).collect())
                        }
                        _ => None,
                    })
            })
            .unwrap_or_default();
        Some(Approval::file_change(pending.id.clone(), &request, &paths))
    }

    /// Puts the open thread's oldest approval request on the thread composer, or no card.
    fn show_approval(&self, cx: &mut Context<Self>) {
        let open = self.thread.as_ref().map(|v| v.read(cx).id().to_owned());
        let pending = self
            .approvals
            .iter()
            .find(|p| Some(p.thread()) == open.as_deref())
            .filter(|p| !p.interrupting);
        let question = pending
            .filter(|p| {
                p.method == "item/tool/requestUserInput"
                    || (p.method == "item/tool/call"
                        && p.params["tool"] == "request_onboarding_input")
            })
            .and_then(|p| p.params["turnId"].as_str())
            .map(str::to_owned);
        if let Some(thread) = &self.thread {
            thread.update(cx, |thread, cx| {
                thread.set_pending_question(question, cx);
                cx.notify();
            });
        }
        let card = pending.and_then(|p| self.approval_card(p, cx));
        self.thread_composer.update(cx, |composer, cx| {
            composer.set_approval(card, cx);
            composer.retain_request_drafts(|id, thread| {
                self.approvals
                    .iter()
                    .any(|pending| pending.id == *id && pending.thread() == thread)
            });
        });
    }

    /// Every server message: streamed into the open thread and the sign-in view, plus
    /// Recents updates and approval prompts.
    pub fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if let Some(login) = &self.login {
            login.update(cx, |login, cx| login.apply(&event, cx));
            return;
        }
        if let Event::Notification { method, params } = &event
            && let Some(generation) = &mut self.generation
            && params["threadId"].as_str() == Some(&generation.thread)
        {
            match method.as_str() {
                "item/agentMessage/delta" => generation
                    .text
                    .get_or_insert_default()
                    .push_str(params["delta"].as_str().unwrap_or_default()),
                "item/completed" if params["item"]["type"] == "agentMessage" => {
                    generation.text = params["item"]["text"].as_str().map(str::to_owned)
                }
                "turn/completed" => {
                    let status = params["turn"]["status"].as_str().unwrap_or_default();
                    let result = if status == "completed" {
                        Ok(generation.text.take())
                    } else {
                        let error = params["turn"]["error"]["message"].as_str();
                        Err(match error {
                            Some(error) => format!("Structured turn failed: {error}"),
                            None => format!("Structured turn ended with status {status}."),
                        })
                    };
                    generation.done.try_send(result).ok();
                }
                _ => {}
            }
            return;
        }
        if matches!(&event, Event::Notification { method, .. } if method == "turn/started")
            || matches!(&event, Event::Request { .. })
        {
            self.qa_work_started = true;
        }
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
        match event {
            Event::Notification { method, params } => match method.as_str() {
                "thread/started" if params["thread"]["ephemeral"] != true => {
                    if let Ok(n) = serde_json::from_value::<ThreadStartedNotification>(params) {
                        self.add_thread(&n.thread, cx);
                    }
                }
                // The row's spinner follows the thread status.
                "thread/status/changed" => {
                    if let Some(id) = params["threadId"].as_str()
                        && let Ok(status) = ThreadStatus::deserialize(&params["status"])
                    {
                        let active = status == ThreadStatus::Active;
                        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                            thread.status = status;
                        }
                        self.sidebar
                            .update(cx, |s, cx| s.set_running(id, active, cx));
                    }
                }
                // Completion can leave the selected thread unread, including a background terminal.
                // It also finishes the thread's open automation run, like the original host.
                "turn/completed" => {
                    if let Some(id) = params["threadId"].as_str() {
                        self.sidebar.update(cx, |s, cx| s.set_unread(id, cx));
                        let host = self.host.clone();
                        let (id, turn) = (id.to_owned(), params["turn"].clone());
                        cx.background_spawn(async move {
                            if let Err(error) = host.finish_automation_run(&id, &turn) {
                                eprintln!("codex-gpui: failed to finish automation run: {error:#}");
                            }
                        })
                        .detach();
                    }
                    // Standalone elicitations and nonblocking questions wait for explicit resolution.
                    let turn = &params["turn"]["id"];
                    self.interrupting.retain(|(thread, pending_turn)| {
                        params["threadId"].as_str() != Some(thread.as_str())
                            || turn.as_str() != Some(pending_turn.as_str())
                    });
                    self.approvals.retain(|p| {
                        (p.method == "mcpServer/elicitation/request"
                            && p.params["serverName"] != "lcu")
                            || (p.method == "item/tool/requestUserInput"
                                && p.params["isBlocking"] == false)
                            || p.params["threadId"] != params["threadId"]
                            || p.params["turnId"] != *turn
                    });
                    self.show_approval(cx);
                    self.refresh_threads(cx)
                }
                // Answered or withdrawn elsewhere: the request leaves without an answer.
                "serverRequest/resolved" => {
                    self.approvals.retain(|p| p.id != params["requestId"]);
                    self.show_approval(cx);
                }
                "thread/queue/changed" => {
                    if let Some(id) = params["threadId"].as_str()
                        && self.thread.as_ref().is_some_and(|v| v.read(cx).id() == id)
                    {
                        self.list_queue(id.to_owned(), cx);
                    }
                }
                "fuzzyFileSearch/sessionUpdated" | "fuzzyFileSearch/sessionCompleted" => {
                    let completed = method == "fuzzyFileSearch/sessionCompleted";
                    if !completed && let Some(menu) = &self.command_menu {
                        menu.update(cx, |menu, cx| menu.apply_file_results(&params, cx));
                    }
                    let home = self.home.read(cx).composer.clone();
                    for composer in [home, self.thread_composer.clone()] {
                        composer.update(cx, |c, cx| c.apply_file_search(completed, &params, cx));
                    }
                }
                _ => {}
            },
            Event::Request { id, method, params } => {
                let pending = PendingApproval {
                    id,
                    method,
                    params,
                    interrupting: false,
                };
                if self.approvals.iter().any(|p| p.id == pending.id) {
                    return;
                }
                if self.approval_card(&pending, cx).is_some() {
                    self.approvals.push(pending);
                    self.show_approval(cx);
                } else {
                    let response = match pending.method.as_str() {
                        "item/tool/call" => json!({
                            "success": false,
                            "contentItems": [{ "type": "inputText", "text": format!("Unsupported dynamic tool: {}", pending.params["tool"].as_str().unwrap_or_default()) }]
                        }),
                        "mcpServer/elicitation/request" => json!({
                            "action": "decline", "content": null, "_meta": null
                        }),
                        "item/tool/requestUserInput" => json!({ "answers": {} }),
                        "item/permissions/requestApproval" => json!({
                            "permissions": {}, "scope": "turn"
                        }),
                        _ => return,
                    };
                    if let Err(error) = self.bridge.respond(pending.id, response) {
                        eprintln!("codex-gpui: unsupported {}: {error:#}", pending.method);
                    }
                }
            }
        }
    }

    /// Reads the first `thread/list` page again; threads of later loaded pages stay after it.
    fn refresh_threads(&self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let mut list = bridge.thread_list().await?;
            this.update(cx, |this, cx| {
                if this.signed_out() {
                    return;
                }
                if this.threads.is_empty() {
                    this.set_next_cursor(list.next_cursor, cx);
                }
                let fresh: HashSet<&str> = list.data.iter().map(|t| t.id.as_str()).collect();
                let rest: Vec<Thread> = this
                    .threads
                    .iter()
                    .filter(|t| !fresh.contains(t.id.as_str()))
                    .cloned()
                    .collect();
                list.data.extend(rest);
                this.set_threads(&list.data, cx)
            })
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
        let mut params = json!({ "threadId": id });
        if let Some(config) = this.update(cx, |root, _| computer_use_overrides(&root.config))? {
            params["config"] = config;
        }
        bridge.request::<_, Value>("thread/resume", params).await?;
        this.update(cx, |this, _| this.loaded.insert(id.to_owned()))?;
    }
    Ok(())
}

/// The Home chip label for a project root: its folder name.
fn project_name(root: &str) -> SharedString {
    std::path::Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(root)
        .to_owned()
        .into()
}

fn computer_use_overrides(config: &Value) -> Option<Value> {
    if config["mcp_servers"]["lcu"]["enabled"] != true {
        return None;
    }
    let mut overrides = serde_json::Map::new();
    if config["mcp_servers"]
        .get("cua_repl")
        .is_some_and(Value::is_object)
    {
        overrides.insert("mcp_servers.cua_repl.enabled".into(), Value::Bool(false));
    }
    if let Some(plugins) = config["plugins"].as_object() {
        for name in plugins
            .keys()
            .filter(|name| name.split('@').next() == Some("unified-computer-use"))
        {
            overrides.insert(
                format!("plugins.{}.enabled", serde_json::to_string(name).ok()?),
                Value::Bool(false),
            );
        }
    }
    (!overrides.is_empty()).then_some(Value::Object(overrides))
}

/// `None` (the server's default) when `model/list` gave the picker nothing.
fn model(s: &Submission) -> Option<&str> {
    (!s.model.is_empty()).then_some(s.model.as_str())
}
fn apply_permissions(params: &mut Value, permission: Permission) {
    let (policy, profile, reviewer) = match permission {
        Permission::Ask => ("on-request", ":workspace", "user"),
        Permission::ApproveForMe => ("on-request", ":workspace", "guardian_subagent"),
        Permission::FullAccess => ("never", ":danger-full-access", "user"),
        Permission::Custom => return,
    };
    params["approvalPolicy"] = policy.into();
    params["permissions"] = profile.into();
    params["approvalsReviewer"] = reviewer.into();
}

/// What `Edit message` returns to the composer from a queued submission's input: its text, its
/// local images, and the rest (remote images, audio, skills, mentions), which the composer
/// cannot show and the next send passes through unchanged.
fn split_input(input: &[Value]) -> (String, Vec<PathBuf>, Vec<Value>) {
    let (mut text, mut images, mut extra) = (String::new(), Vec::new(), Vec::new());
    for part in input {
        match (part["type"].as_str(), part["path"].as_str()) {
            (Some("text"), _) => text.push_str(part["text"].as_str().unwrap_or_default()),
            (Some("localImage"), Some(path)) if part["detail"].is_null() => {
                images.push(PathBuf::from(path));
            }
            _ => extra.push(part.clone()),
        }
    }
    (text, images, extra)
}

/// A queued draft taken back by `Edit message`: where it stood in the queue, and the input
/// the composer cannot show.
struct QueueEdit {
    thread_id: String,
    previous: Option<String>,
    next: Option<String>,
    extra: Vec<Value>,
}

/// After an edited draft is queued again (`thread/queue/add` appends it), moves it before the
/// submission that followed it, else after the one that preceded it, as the original restores
/// an edit (`thread/queue/reorder`). Other positions need no reorder.
async fn restore_place(
    bridge: &Bridge,
    thread_id: &str,
    added: &str,
    edit: &QueueEdit,
) -> Result<()> {
    let list: ThreadQueueListResponse = bridge
        .request(
            "thread/queue/list",
            json!({ "threadId": thread_id, "cursor": null }),
        )
        .await?;
    // ponytail: a reorder of a partial page would drop the rest; queues of one page only.
    if list.next_cursor.is_some() {
        return Ok(());
    }
    let mut ids: Vec<_> = list
        .data
        .into_iter()
        .map(|row| row.id)
        .filter(|id| id != added)
        .collect();
    let find = |id: &Option<String>| ids.iter().position(|row| Some(row) == id.as_ref());
    let at = match (find(&edit.next), find(&edit.previous)) {
        (Some(next), _) => next,
        (None, Some(previous)) => previous + 1,
        (None, None) => return Ok(()),
    };
    ids.insert(at, added.to_owned());
    bridge
        .request::<_, Value>(
            "thread/queue/reorder",
            json!({ "threadId": thread_id, "queuedSubmissionIds": ids }),
        )
        .await?;
    Ok(())
}

/// The prompt as `turn/start` input; non-image files go in as paths the agent can open.
fn user_input(s: &Submission) -> Vec<Value> {
    const IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
    let (images, files): (Vec<_>, Vec<_>) = s.attachments.iter().partition(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGES.contains(&e.to_lowercase().as_str()))
    });
    let text = files
        .iter()
        .fold(s.text.clone(), |text, p| format!("{text}\n{}", p.display()));
    let mut input = vec![json!({ "type": "text", "text": text, "text_elements": [] })];
    input.extend(
        images
            .iter()
            .map(|p| json!({ "type": "localImage", "path": p })),
    );
    input
}

/// `turn/start` with the prompt, attachments, model, effort and Fast mode.
async fn turn_start(
    bridge: &Bridge,
    thread_id: &str,
    s: &Submission,
    input: Vec<Value>,
) -> Result<()> {
    let mut params = json!({
        "threadId": thread_id,
        "input": input,
        "model": model(s),
        "effort": s.effort,
    });
    apply_permissions(&mut params, s.permission);
    if s.fast {
        params["serviceTier"] = "priority".into();
    }
    bridge.request::<_, Value>("turn/start", params).await?;
    Ok(())
}

async fn read_queue(bridge: &Bridge, thread_id: &str) -> Result<Vec<QueuedSubmission>> {
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page: ThreadQueueListResponse = bridge
            .request(
                "thread/queue/list",
                json!({ "threadId": thread_id, "cursor": cursor }),
            )
            .await?;
        rows.extend(page.data);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(rows);
        }
    }
}

/// The ephemeral app-server thread that writes a commit message, and what it has said.
struct Generation {
    thread: String,
    text: Option<String>,
    done: async_channel::Sender<Result<Option<String>, String>>,
}

/// A server request that waits for the user's answer or a turn interruption.
#[derive(Clone)]
struct PendingApproval {
    /// The server request's id: the answer and `serverRequest/resolved` carry it.
    id: Value,
    method: String,
    params: Value,
    interrupting: bool,
}

impl PendingApproval {
    fn thread(&self) -> &str {
        self.params["threadId"].as_str().unwrap_or_default()
    }
}

/// What a submitted commit form, or the push-only command, asks of one repository.
struct GitRequest {
    cwd: String,
    step: CommitStep,
    message: String,
    include_unstaged: bool,
    new_branch: Option<String>,
}

/// What one commit workflow needs off the UI thread.
struct GitRun {
    host: Arc<dyn host::Host>,
    bridge: Arc<Bridge>,
    model: String,
    cwd: String,
    instructions: String,
}

enum GitOutcome {
    Done(String),
    Failed {
        title: String,
        detail: Option<String>,
    },
}

impl GitOutcome {
    fn failed(title: impl Into<String>, detail: Option<String>) -> Self {
        Self::Failed {
            title: title.into(),
            detail,
        }
    }
}

async fn restore_queued(bridge: &Bridge, deleted: &DeletedQueuedMessage) -> Result<()> {
    let mut rows = read_queue(bridge, &deleted.thread_id).await?;
    let client_id = &deleted.submission.client_user_message_id;
    if !rows
        .iter()
        .any(|row| &row.client_user_message_id == client_id)
    {
        bridge
            .request::<_, Value>(
                "thread/queue/add",
                json!({
                    "threadId": deleted.thread_id, "input": deleted.submission.input,
                    "clientUserMessageId": client_id,
                }),
            )
            .await?;
        rows = read_queue(bridge, &deleted.thread_id).await?;
    }
    let original_position = rows
        .iter()
        .position(|row| &row.client_user_message_id == client_id)
        .context("The restored queued message is no longer queued")?;
    let restored_id = rows.remove(original_position).id;
    let position = deleted
        .next
        .as_ref()
        .and_then(|next| {
            rows.iter()
                .position(|row| &row.client_user_message_id == next)
        })
        .or_else(|| {
            deleted.previous.as_ref().and_then(|previous| {
                rows.iter()
                    .position(|row| &row.client_user_message_id == previous)
                    .map(|at| at + 1)
            })
        })
        .unwrap_or(rows.len());
    if position != original_position {
        let mut order: Vec<_> = rows.into_iter().map(|row| row.id).collect();
        order.insert(position, restored_id);
        bridge
            .request::<_, Value>(
                "thread/queue/reorder",
                json!({
                    "threadId": deleted.thread_id, "queuedSubmissionIds": order,
                }),
            )
            .await?;
    }
    Ok(())
}

/// A host setting of the original's Git page (`git-branch-prefix`, `git-commit-instructions`).
fn git_setting(cx: &App, key: &str) -> Option<String> {
    cx.try_global::<host_settings::Settings>()?
        .read_all()
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

fn git_call(host: &dyn host::Host, endpoint: &str, params: Value) -> Result<Value> {
    host::json_body(&host.fetch(&host::Request::post(endpoint, Some(params)))?)
}

impl GitRun {
    async fn call(
        &self,
        cx: &AsyncWindowContext,
        endpoint: &'static str,
        params: Value,
    ) -> Result<Value, String> {
        let host = self.host.clone();
        cx.background_executor()
            .spawn(async move { git_call(&*host, endpoint, params) })
            .await
            .map_err(|error| format!("{error:#}"))
    }
}

fn commit_repository(
    status: &Value,
    include_unstaged: bool,
    new_branch_name: String,
) -> Result<CommitRepository> {
    anyhow::ensure!(status["staged"].is_object(), "missing Git status");
    let selection = |key: &str| {
        let count = |field: &str| status[key][field].as_u64().unwrap_or_default() as usize;
        Selection {
            files: count("files"),
            additions: count("additions"),
            deletions: count("deletions"),
        }
    };
    Ok(CommitRepository {
        branch: status["branch"].as_str().map(str::to_owned),
        branches: status["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|branch| branch.as_str().map(str::to_owned))
            .collect(),
        has_commits: status["hasCommits"].as_bool().unwrap_or(false),
        ahead: status["commitsAhead"].as_u64().unwrap_or_default() as usize,
        staged: selection("staged"),
        all: selection("all"),
        include_unstaged,
        new_branch_name,
    })
}

/// The original's default new branch: the prefix and the first five words of the chat title.
fn branch_name(prefix: &str, title: &str) -> String {
    let slug = title
        .trim()
        .to_lowercase()
        .split_whitespace()
        .take(5)
        .map(|word| {
            word.chars()
                .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{}{slug}", prefix.trim())
}

/// `Committed to {branch}` and `Pushed {branch}` come from the step that finished last.
fn git_result(done: &Value) -> Result<Option<String>, GitOutcome> {
    if done["status"] == "success" {
        return Ok(done["branch"].as_str().map(str::to_owned));
    }
    let title = match done["stage"].as_str() {
        Some("branch") => "Failed to create branch",
        Some("push") => "Failed to push changes",
        _ => "Failed to commit changes",
    };
    Err(GitOutcome::failed(
        title,
        done["error"].as_str().map(str::to_owned),
    ))
}

async fn commit_workflow(
    this: &WeakEntity<Root>,
    cx: &mut AsyncWindowContext,
    run: &GitRun,
    request: GitRequest,
) -> GitOutcome {
    let GitRequest {
        step,
        message,
        include_unstaged,
        new_branch,
        ..
    } = request;
    let mut branch = None;
    if step != CommitStep::Push {
        let message = if message.is_empty() {
            match generate_message(this, cx, run, include_unstaged).await {
                Ok(Some(message)) => message,
                Ok(None) => return GitOutcome::failed("Couldn't generate a commit message", None),
                Err(error) => {
                    return GitOutcome::failed(
                        format!("Failed to generate commit message: {error}"),
                        Some(error),
                    );
                }
            }
        } else {
            message
        };
        this.update(cx, |this, cx| this.set_git_phase(Some("Committing…"), cx))
            .ok();
        let params = json!({
            "cwd": run.cwd,
            "message": message,
            "includeUnstaged": include_unstaged,
            "newBranch": new_branch,
        });
        match run.call(cx, "git-commit", params).await {
            Ok(done) => match git_result(&done) {
                Ok(committed) => branch = committed,
                Err(failed) => return failed,
            },
            Err(error) => return GitOutcome::failed("Failed to commit changes", Some(error)),
        }
    }
    if step != CommitStep::Commit {
        this.update(cx, |this, cx| {
            this.set_git_phase(Some("Pushing changes…"), cx)
        })
        .ok();
        let new_branch = new_branch.filter(|_| step == CommitStep::Push);
        let params = json!({"cwd": run.cwd, "newBranch": new_branch});
        match run.call(cx, "git-push", params).await {
            Ok(done) => match git_result(&done) {
                Ok(pushed) => branch = pushed,
                Err(failed) => return failed,
            },
            Err(error) => return GitOutcome::failed("Failed to push changes", Some(error)),
        }
    }
    let target = branch.filter(|branch| !branch.is_empty());
    GitOutcome::Done(match (step, target) {
        (CommitStep::Commit, Some(branch)) => format!("Committed to {branch}"),
        (CommitStep::Commit, None) => "Committed to your branch".into(),
        (_, Some(branch)) => format!("Pushed {branch}"),
        (_, None) => "Pushed your branch".into(),
    })
}

const COMMIT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// The model the original generates commit messages with (`ephemeralGenerationModel`), else
/// the default model that supports low reasoning effort.
fn generation_model(models: &[Model]) -> String {
    const DEFAULT: &str = "gpt-6-luna";
    if models.is_empty() || models.iter().any(|model| model.model == DEFAULT) {
        return DEFAULT.into();
    }
    let low = |model: &&Model| {
        !model.hidden
            && model
                .supported_reasoning_efforts
                .iter()
                .any(|effort| effort["reasoningEffort"] == "low")
    };
    models
        .iter()
        .filter(low)
        .find(|model| model.is_default)
        .or_else(|| models.iter().find(low))
        .map_or_else(|| DEFAULT.into(), |model| model.model.clone())
}

/// Writes a commit message the way the original's `generate-commit-message` does: an
/// ephemeral read-only app-server thread answers the diff prompt with structured output.
async fn generate_message(
    this: &WeakEntity<Root>,
    cx: &mut AsyncWindowContext,
    run: &GitRun,
    include_unstaged: bool,
) -> Result<Option<String>, String> {
    let diff = run
        .call(
            cx,
            "git-commit-diff",
            json!({"cwd": run.cwd, "includeUnstaged": include_unstaged}),
        )
        .await
        .ok()
        .and_then(|diff| diff["unifiedDiff"].as_str().map(str::to_owned));
    let prompt = generation_prompt(&run.instructions, diff.as_deref());
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Ok(None);
    }
    let prompt: String = prompt.chars().take(20_000).collect();
    let started: Value = run
        .bridge
        .request(
            "thread/start",
            json!({
                "model": run.model,
                "modelProvider": null,
                "allowProviderModelFallback": true,
                "cwd": run.cwd,
                "approvalPolicy": "never",
                "permissions": ":read-only",
                "runtimeWorkspaceRoots": [],
                "config": {
                    "features.enable_fanout": false,
                    "features.multi_agent": false,
                    "features.multi_agent_v2": false,
                    "web_search": "disabled",
                    "mcp_servers.codex_app": {"enabled": false, "command": ""},
                    "model_reasoning_effort": "low",
                },
                "personality": null,
                "ephemeral": true,
                "threadSource": "commit_message",
                "experimentalRawEvents": false,
                "dynamicTools": null,
                "serviceTier": null,
            }),
        )
        .await
        .map_err(|error| format!("{error:#}"))?;
    let thread = started["thread"]["id"]
        .as_str()
        .ok_or("the app-server started no thread")?
        .to_owned();
    let (done, answer) = async_channel::bounded(1);
    this.update(cx, |this, _| {
        this.generation = Some(Generation {
            thread: thread.clone(),
            text: None,
            done: done.clone(),
        });
    })
    .map_err(|error| error.to_string())?;
    let executor = cx.background_executor().clone();
    let timeout = executor.spawn({
        let (executor, done) = (executor.clone(), done.clone());
        async move {
            executor.timer(COMMIT_MESSAGE_TIMEOUT).await;
            done.try_send(Err("Timed out waiting for structured result.".into()))
                .ok();
        }
    });
    let turn = run
        .bridge
        .request::<_, Value>(
            "turn/start",
            json!({
                "threadId": thread,
                "input": [{"type": "text", "text": generation_instructions(&prompt), "text_elements": []}],
                "outputSchema": COMMIT_MESSAGE_SCHEMA.clone(),
            }),
        )
        .await;
    let result = match turn {
        Ok(_) => answer
            .recv()
            .await
            .unwrap_or_else(|_| Err("the app-server closed".into())),
        Err(error) => Err(format!("{error:#}")),
    };
    drop(timeout);
    this.update(cx, |this, _| this.generation = None).ok();
    run.bridge
        .request::<_, Value>("thread/unsubscribe", json!({"threadId": thread}))
        .await
        .ok();
    Ok(parse_commit_message(result?.as_deref()))
}

static COMMIT_MESSAGE_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    json!({
        "type": "object",
        "properties": {"message": {
            "type": "string",
            "minLength": 8,
            "maxLength": 4000,
            "pattern": "^[^\\r\\n]{1,72}(?:\\r?\\n[\\s\\S]*)?$",
        }},
        "required": ["message"],
        "additionalProperties": false,
    })
});

/// The structured answer's `message` when it meets the original's bounds.
fn parse_commit_message(text: Option<&str>) -> Option<String> {
    let answer: Value = serde_json::from_str(text?.trim()).ok()?;
    let message = answer["message"].as_str()?;
    let subject = message.split('\n').next()?;
    let subject = subject.strip_suffix('\r').unwrap_or(subject);
    let length = message.chars().count();
    let valid = (8..=4000).contains(&length)
        && (1..=72).contains(&subject.chars().count())
        && !subject.contains('\r');
    valid.then(|| message.trim().to_owned())
}

/// The original's context for generation: custom instructions, then the changed tracked
/// files' diff, or its size when it is over 1000 changed lines.
fn generation_prompt(instructions: &str, diff: Option<&str>) -> String {
    const UNTRACKED: &str = "Untracked changes are not included.";
    let mut parts = Vec::new();
    let instructions = instructions.trim();
    if !instructions.is_empty() {
        parts.push(format!(
            "Custom commit instructions (follow these for commit message content and formatting):\n{instructions}\n"
        ));
    }
    let diff = diff.filter(|diff| !diff.trim().is_empty());
    if let Some(diff) = diff {
        let (mut files, mut added, mut removed, mut in_hunk) = (0, 0, 0, false);
        for line in diff.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.starts_with("diff --git ") {
                (files, in_hunk) = (files + 1, false);
            } else if line.starts_with("@@") {
                in_hunk = true;
            } else if in_hunk && line.starts_with('+') {
                added += 1;
            } else if in_hunk && line.starts_with('-') {
                removed += 1;
            }
        }
        parts.push(if added + removed > 1000 {
            format!(
                "Changes:\nDiff too large to include inline.\nSummary: {files} changed files, +{added}/-{removed} lines.\n\n{UNTRACKED}"
            )
        } else {
            format!("Changes:\n{diff}\n{UNTRACKED}")
        });
    }
    parts.join("\n\n")
}

/// The original's commit-message prompt (`Using the supplied git context below ...`).
fn generation_instructions(context: &str) -> String {
    [
        "Using the supplied git context below, generate a git commit message.",
        "Write the result into the structured response field message.",
        "message must contain plain commit-message text only, not JSON, field labels, or code fences.",
        "Custom commit instructions for message content and formatting override the fallback rules below.",
        "Make 0 tool calls.",
        "Bounds:",
        "- Keep the complete message under 4000 characters.",
        "- Keep the subject under 72 characters.",
        "Fallback rules:",
        "- Generate a concise single-line subject.",
        "- Use an imperative verb first.",
        "- Do not add a scope prefix unless the context already clearly uses one.",
        "- Do not include markdown, quotes, or trailing punctuation.",
        "",
        "Diff context:",
        context,
    ]
    .join("\n")
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let exclusive = self
            .login
            .as_ref()
            .map(|view| view.clone().into_any_element())
            .or_else(|| {
                self.onboarding_page
                    .as_ref()
                    .filter(|view| view.read(cx).page() != ui::screens::onboarding::Page::FirstRun)
                    .map(|view| view.clone().into_any_element())
            });
        if let Some(exclusive) = exclusive {
            return div()
                .size_full()
                .relative()
                .font_family(ui::SANS)
                .track_focus(&self.focus)
                .on_key_down(ui::focus::tab)
                .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                    this.close_window(window, cx);
                }))
                .on_action(|_: &Minimize, window, _| window.minimize_window())
                .on_action(|_: &Zoom, window, _| window.zoom_window())
                .child(exclusive)
                .children(self.dialog.clone())
                .into_any_element();
        }
        let content: AnyElement = match (&self.page, &self.split) {
            (_, Some(split)) if self.sidebar_automation.is_some() => {
                split.clone().into_any_element()
            }
            (Some(page), _) => page.clone().into_any_element(),
            (None, Some(split)) => split.clone().into_any_element(),
            (None, None) => self.shell.clone().into_any_element(),
        };
        let root = div()
            .size_full()
            .relative()
            .font_family(ui::SANS)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::type_anywhere))
            .capture_action(cx.listener(|this, _: &Undo, window, cx| {
                this.undo_queued(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.step(false, window, cx)))
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
        let root = routes::actions(root, cx);
        section_actions::actions(features::actions(root, cx), cx).into_any_element()
    }
}

fn automation_update_body(item: &Value, status: &str) -> Value {
    let heartbeat = item["kind"] == "heartbeat";
    let mut body = json!({
        "id": item["id"], "kind": item["kind"], "name": item["name"],
        "prompt": item["prompt"], "status": status, "rrule": item["rrule"],
        "notificationPolicy": item["notificationPolicy"],
        "model": if heartbeat { Value::Null } else { item["model"].clone() },
        "reasoningEffort": if heartbeat { Value::Null } else { item["reasoningEffort"].clone() },
    });
    if heartbeat {
        body["targetThreadId"] = item["targetThreadId"].clone();
    } else {
        body["autoArchive"] = json!(item["autoArchive"].as_bool().unwrap_or(false));
        body["projectId"] = match item["target"]["type"].as_str() {
            Some("project") => item["target"]["projectId"].clone(),
            Some("projectless") => Value::Null,
            _ => item["cwds"]
                .get(0)
                .filter(|cwd| **cwd != "~")
                .cloned()
                .unwrap_or(Value::Null),
        };
        body["executionEnvironment"] = item["executionEnvironment"].clone();
        body["localEnvironmentConfigPath"] = item["localEnvironmentConfigPath"].clone();
    }
    body
}

#[cfg(test)]
mod computer_use_tests {
    use super::*;

    #[test]
    fn lcu_cutover_disables_only_the_original_computer_use_surface() {
        let mut config = json!({
            "mcp_servers": {
                "lcu": {"enabled": true},
                "cua_repl": {"enabled": true, "command": "original"}
            },
            "plugins": {
                "unified-computer-use@openai-bundled": {"enabled": true},
                "unrelated@openai-bundled": {"enabled": true}
            }
        });
        assert_eq!(
            computer_use_overrides(&config),
            Some(json!({
                "mcp_servers.cua_repl.enabled": false,
                "plugins.\"unified-computer-use@openai-bundled\".enabled": false
            }))
        );
        config["mcp_servers"]["lcu"]["enabled"] = false.into();
        assert_eq!(computer_use_overrides(&config), None);
    }

    #[test]
    fn lcu_without_an_original_provider_does_not_create_an_invalid_transport() {
        assert_eq!(
            computer_use_overrides(&json!({"mcp_servers": {"lcu": {"enabled": true}}})),
            None
        );
    }
}
