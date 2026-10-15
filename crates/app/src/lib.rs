//! The app's root view: routes, persisted window state, and the live `codex app-server`
//! connection that feeds the screens (Recents, model picker, threads, approvals, sign-in).
//! `main.rs` only opens the window; `examples/live_turn.rs` drives the same view headless.

use anyhow::Result;
use async_channel::Receiver;
use bridge::types::{
    ApprovalDecision, CommandApprovalRequest, ConfigReadResponse, FileChangeApprovalRequest,
    Thread, ThreadItem, ThreadStartResponse, ThreadStartedNotification,
};
use bridge::{Bridge, Event};
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Task, Window,
    WindowAppearance, actions, div,
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
use ui::{GoBack, GoForward, NewChat, OpenSettings, Shell, ToggleSidebar};

actions!(app, [CloseWindow, Minimize, Zoom]);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Route {
    #[default]
    Home,
    Thread(String),
    Settings,
}

/// Window size, sidebar and route, persisted to `path` (none: not persisted).
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub width: f32,
    pub height: f32,
    pub sidebar_open: bool,
    pub route: Route,
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
        let sidebar_open = state.sidebar_open;
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
        cx.observe_window_bounds(window, |this, window, cx| this.save(window, cx))
            .detach();
        cx.observe_window_appearance(window, |_, window, cx| {
            cx.set_global(theme_for(window.appearance()));
            cx.notify();
        })
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
                this.sidebar
                    .update(cx, |s, cx| s.set_threads(&threads.data, cx));
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
        let main: AnyView = match (&thread_id, &self.thread) {
            (Some(id), Some(view)) if view.read(cx).id() == id => {
                window.focus(&self.thread_composer.focus_handle(cx), cx);
                view.clone().into()
            }
            (Some(id), _) => {
                self.load_thread(id.clone(), window, cx);
                cx.new(|_| ui::EmptyMain).into()
            }
            (None, _) => {
                let focus = self.home.read(cx).composer.focus_handle(cx);
                window.focus(&focus, cx);
                self.home.clone().into()
            }
        };
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
                this.sidebar.update(cx, |sb, cx| sb.add_thread(&thread, cx));
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
                        self.sidebar.update(cx, |s, cx| s.add_thread(&n.thread, cx));
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
        let (bridge, sidebar) = (self.bridge.clone(), self.sidebar.clone());
        let task = cx.spawn(async move |_, cx| -> Result<()> {
            let list = bridge.thread_list().await?;
            sidebar.update(cx, |s, cx| s.set_threads(&list.data, cx));
            Ok(())
        });
        log_err(task, cx);
    }

    fn save(&mut self, window: &Window, cx: &App) {
        let size = window.window_bounds().get_bounds().size;
        self.state.width = size.width.into();
        self.state.height = size.height.into();
        self.state.sidebar_open = self.shell.read(cx).sidebar_open;
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
            .on_action(
                cx.listener(|this, _: &NewChat, window, cx| this.navigate(Route::Home, window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.navigate(Route::Settings, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.step(false, window, cx)))
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
