use crate::{Root, Route, host, windows};
use anyhow::{Context as _, Result, bail};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Div, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle, actions, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use theme::ActiveTheme as _;
use ui::screens::hotkey::{Change, Controls, Mode, Options};
use ui::screens::settings::keyboard_shortcuts::KeyboardState;
use ui::screens::settings::voice::Policy as VoicePolicy;

#[cfg(target_os = "macos")]
mod audio;
#[cfg(target_os = "macos")]
mod desktop;
#[cfg(target_os = "macos")]
mod dictation;
mod platform;
pub(crate) mod policy;
use platform::{Native, Shortcut, ShortcutEvent};

const GLOBAL_COMMANDS: [(&str, Shortcut, Option<&str>); 6] = [
    ("spotlightSearch", Shortcut::Spotlight, Some("Alt+K")),
    (
        "screenshotToCurrentChat",
        Shortcut::ScreenshotCurrent,
        Some("Control+Shift+4"),
    ),
    (
        "screenshotToNewChat",
        Shortcut::ScreenshotNew,
        Some("Control+Alt+Shift+4"),
    ),
    ("hotkeyWindow", Shortcut::Popout, None),
    ("globalDictationHold", Shortcut::DictationHold, None),
    ("globalDictationSingleTap", Shortcut::DictationToggle, None),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Home,
    NewThread,
    Thread(String),
    Remote(String),
    Spotlight,
}

impl Target {
    pub fn parse(path: &str) -> Result<Self> {
        let path = path
            .split(['?', '#'])
            .next()
            .unwrap_or(path)
            .trim_end_matches('/');
        if let Some(Route::Remote {
            task_id,
            hotkey: true,
        }) = Route::remote_path(path)
        {
            return Ok(Self::Remote(task_id));
        }
        match path {
            "/hotkey-window" => Ok(Self::Home),
            "/hotkey-window/new-thread" => Ok(Self::NewThread),
            "/spotlight" => Ok(Self::Spotlight),
            _ => path
                .strip_prefix("/hotkey-window/thread/")
                .filter(|id| !id.is_empty() && !id.contains('/'))
                .map(|id| Self::Thread(id.to_owned()))
                .context("Unsupported popout route"),
        }
    }
}

pub(crate) struct Popout {
    target: Target,
    owner: WindowHandle<Root>,
    controls: Entity<Controls>,
    project_read: Option<gpui::Task<()>>,
}

pub(crate) fn thread_shown(root: &Root, window: &mut Window, cx: &mut Context<Root>) {
    if root
        .popout
        .as_ref()
        .is_some_and(|popout| popout.target != Target::Spotlight)
    {
        windows::set_kind(window, windows::Kind::HotkeyThread, cx);
        let bounds = window.bounds();
        let _ = windows::set_bounds(
            window.window_handle(),
            gpui::Bounds::new(
                gpui::point(bounds.origin.x, (bounds.bottom() - px(640.)).max(px(0.))),
                gpui::size(bounds.size.width, px(640.)),
            ),
            cx,
        );
    }
}

actions!(
    hotkey,
    [
        OpenPopout,
        OpenSpotlight,
        Collapse,
        Dismiss,
        OpenInMainWindow,
        ScreenshotCurrent,
        ScreenshotNew
    ]
);

#[derive(Default)]
struct Runtime {
    primary: Option<WindowHandle<Root>>,
    active: Option<WindowHandle<Root>>,
    popout: Option<WindowHandle<Root>>,
    spotlight: Option<WindowHandle<Root>>,
    native: Option<Native>,
    listener: Option<gpui::Task<()>>,
    bindings: Vec<platform::Binding>,
    sender: Option<async_channel::Sender<ShortcutEvent>>,
    initialized: bool,
    requested: Option<(Vec<platform::Binding>, bool)>,
    policy_generation: u64,
    policy_task: Option<gpui::Task<()>>,
    #[cfg(target_os = "macos")]
    dictation: Option<Entity<dictation::Controller>>,
}
impl gpui::Global for Runtime {}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(Runtime::default());
    cx.observe_global::<KeyboardState>(sync_keyboard).detach();
    cx.observe_global::<VoicePolicy>(sync_keyboard).detach();
    cx.bind_keys([gpui::KeyBinding::new("escape", Dismiss, Some("Popout"))]);
    ui::screens::hotkey::init(cx);
    cx.on_action(|_: &OpenPopout, cx| launch(Target::Home, cx));
    cx.on_action(|_: &OpenSpotlight, cx| launch(Target::Spotlight, cx));
    cx.on_action(|_: &ScreenshotCurrent, cx| screenshot(false, cx));
    cx.on_action(|_: &ScreenshotNew, cx| screenshot(true, cx));
}

pub(crate) fn primary(handle: WindowHandle<Root>, cx: &mut App) {
    cx.global_mut::<Runtime>().primary = Some(handle);
    if !cx.global::<Runtime>().initialized {
        cx.global_mut::<Runtime>().initialized = true;
        sync_keyboard(cx);
    }
}

pub(crate) fn activation_changed(root: &Root, window: &mut Window, cx: &mut App) {
    if !window.is_window_active() {
        if root.popout.is_some() {
            windows::close(window, cx);
        }
        return;
    }
    if let Some(handle) = window.window_handle().downcast::<Root>() {
        if root.popout.is_none() {
            primary(handle, cx);
        }
        cx.global_mut::<Runtime>().active = Some(handle);
    }
}

pub fn suspend_global_bindings(cx: &mut App) {
    let runtime = cx.global_mut::<Runtime>();
    runtime.initialized = true;
    runtime.native = None;
    runtime.listener = None;
    #[cfg(target_os = "macos")]
    if let Err(error) = dictation::registration_changed(cx) {
        eprintln!("codex-gpui: cannot suspend dictation: {error:#}");
    }
}

fn sync_keyboard(cx: &mut App) {
    if crate::qa::ENABLED || !cfg!(target_os = "macos") || cx.global::<Runtime>().primary.is_none()
    {
        return;
    }
    let Some(state) = cx
        .try_global::<KeyboardState>()
        .filter(|state| state.loaded)
    else {
        return;
    };
    let suspended = state.capture_owner.is_some();
    let bindings = if suspended {
        Ok(Vec::new())
    } else {
        gated_bindings(&state.preferences.bindings, cx)
    };
    let bindings = match bindings {
        Ok(bindings) => bindings,
        Err(error) => {
            let message = error.to_string();
            if cx.global::<KeyboardState>().error.as_deref() != Some(message.as_str()) {
                cx.update_global::<KeyboardState, _>(|state, _| state.error = Some(message));
            }
            return;
        }
    };
    if cx
        .global::<Runtime>()
        .requested
        .as_ref()
        .is_some_and(|(current, capture)| current == &bindings && *capture == suspended)
    {
        publish_active_commands(cx);
        return;
    }
    cx.global_mut::<Runtime>().requested = Some((bindings.clone(), suspended));
    if suspended {
        suspend_global_bindings(cx);
    } else if let Err(error) = install_bindings(bindings, cx) {
        cx.update_global::<KeyboardState, _>(|state, _| state.error = Some(error.to_string()));
    }
    publish_active_commands(cx);
}

fn publish_active_commands(cx: &mut App) {
    let runtime = cx.global::<Runtime>();
    let active = GLOBAL_COMMANDS.map(|(id, shortcut, _)| {
        (
            id,
            runtime.native.is_some()
                && runtime.bindings.iter().any(|binding| binding.0 == shortcut),
        )
    });
    if cx.try_global::<KeyboardState>().is_some_and(|state| {
        active
            .iter()
            .any(|(id, enabled)| state.active_commands.contains(*id) != *enabled)
    }) {
        cx.update_global::<KeyboardState, _>(|state, _| {
            for (id, enabled) in active {
                if enabled {
                    state.active_commands.insert(id);
                } else {
                    state.active_commands.remove(id);
                }
            }
        });
    }
}

fn configured_bindings(
    overrides: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<platform::Binding>> {
    let mut bindings = Vec::new();
    for (id, shortcut, fallback) in GLOBAL_COMMANDS {
        if let Some(values) = overrides.get(id) {
            anyhow::ensure!(
                values.len() <= 1,
                "OS-global shortcuts support one keybinding"
            );
            for value in values {
                bindings.push(platform::binding(shortcut, value)?);
            }
        } else if let Some(value) = fallback {
            bindings.push(platform::binding(shortcut, value)?);
        }
    }
    Ok(bindings)
}

fn gated_bindings(
    overrides: &BTreeMap<String, Vec<String>>,
    cx: &App,
) -> Result<Vec<platform::Binding>> {
    let mut bindings = configured_bindings(overrides)?;
    if !cx
        .try_global::<VoicePolicy>()
        .is_some_and(|policy| policy.global_dictation)
    {
        bindings.retain(|binding| {
            !matches!(
                binding.0,
                Shortcut::DictationHold | Shortcut::DictationToggle
            )
        });
    }
    Ok(bindings)
}

pub fn replace_global_bindings(
    overrides: &BTreeMap<String, Vec<String>>,
    cx: &mut App,
) -> Result<()> {
    let result = install_bindings(gated_bindings(overrides, cx)?, cx);
    publish_active_commands(cx);
    result
}

fn install_bindings(bindings: Vec<platform::Binding>, cx: &mut App) -> Result<()> {
    let (sender, receiver) = async_channel::unbounded();
    let previous = cx.global_mut::<Runtime>().native.take();
    let was_active = previous.is_some();
    drop(previous);
    let native = match Native::new(&bindings, sender.clone()) {
        Ok(native) => native,
        Err(error) => {
            let runtime = cx.global_mut::<Runtime>();
            if let Some(sender) = runtime.sender.clone().filter(|_| was_active) {
                runtime.native = Some(Native::new(&runtime.bindings, sender)
                    .map_err(|restore| anyhow::anyhow!("Global shortcut update failed: {error:#}; restoring prior shortcuts failed: {restore:#}"))?);
            }
            return Err(error);
        }
    };
    let listener = cx.spawn(async move |cx| {
        while let Ok(event) = receiver.recv().await {
            cx.update(|cx| match event.shortcut {
                Shortcut::Spotlight if event.pressed => launch(Target::Spotlight, cx),
                Shortcut::Popout if event.pressed => launch(Target::Home, cx),
                Shortcut::ScreenshotCurrent if event.pressed => screenshot(false, cx),
                Shortcut::ScreenshotNew if event.pressed => screenshot(true, cx),
                Shortcut::DictationHold | Shortcut::DictationToggle => {
                    #[cfg(target_os = "macos")]
                    dictation::event(event, cx);
                }
                _ => {}
            });
        }
    });
    let runtime = cx.global_mut::<Runtime>();
    runtime.native = Some(native);
    runtime.listener = Some(listener);
    runtime.bindings = bindings;
    runtime.sender = Some(sender);
    runtime.initialized = true;
    #[cfg(target_os = "macos")]
    dictation::registration_changed(cx)?;
    Ok(())
}

fn screenshot(new_chat: bool, cx: &mut App) {
    let runtime = cx.global::<Runtime>();
    let Some(owner) = (if new_chat {
        runtime.primary
    } else {
        runtime.active.or(runtime.primary)
    }) else {
        return;
    };
    let capture = cx.background_spawn(async move {
        #[cfg(target_os = "macos")]
        {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("selection.png");
            let result = std::process::Command::new("/usr/sbin/screencapture")
                .args(["-i", "-x"])
                .arg(&path)
                .status()?;
            if !path.exists() {
                return Ok::<_, anyhow::Error>(None);
            }
            anyhow::ensure!(result.success(), "Screenshot capture failed");
            Ok(Some(std::fs::read(path)?))
        }
        #[cfg(not(target_os = "macos"))]
        {
            bail!("Screenshot selection requires macOS")
        }
    });
    cx.spawn(async move |cx| {
        let result: Result<Option<Vec<u8>>> = capture.await;
        let _ = owner.update(cx, |root, window, cx| match result {
            Ok(Some(bytes)) => {
                if new_chat {
                    root.new_chat(window, cx);
                }
                root.composer(cx)
                    .update(cx, |composer, cx| composer.attach_png(bytes, cx));
                let _ = windows::show(owner.into(), windows::Presentation::Foreground, cx);
            }
            Ok(None) => {}
            Err(error) => root.toast(
                ui::toast::Toast::danger("Could not capture screenshot")
                    .description(error.to_string()),
                cx,
            ),
        });
    })
    .detach();
}

fn launch(target: Target, cx: &mut App) {
    let Some(owner) = cx.global::<Runtime>().primary else {
        return;
    };
    let existing = match target {
        Target::Home => cx.global::<Runtime>().popout,
        Target::Spotlight => cx.global::<Runtime>().spotlight,
        _ => None,
    };
    if let Some(handle) = existing
        && windows::show(handle.into(), windows::Presentation::Foreground, cx).is_ok()
    {
        return;
    }
    if let Err(error) = open(owner, target, cx) {
        eprintln!("codex-gpui: cannot open popout: {error:#}");
    }
}

pub fn open(owner: WindowHandle<Root>, target: Target, cx: &mut App) -> Result<WindowHandle<Root>> {
    let (mut state, cwd, host, bridge) = owner.update(cx, |root, _, _| {
        (
            root.state.clone(),
            root.cwd.clone(),
            root.host.clone(),
            root.bridge.clone(),
        )
    })?;
    state.route = match &target {
        Target::Thread(id) => Route::Thread(id.clone()),
        Target::Remote(task_id) => Route::Remote {
            task_id: task_id.clone(),
            hotkey: true,
        },
        _ => Route::Home,
    };
    state.path = None;
    state.sidebar_open = false;
    state.confirm_before_close = crate::ConfirmBeforeClose::Never;
    let kind = match target {
        Target::Home => windows::Kind::HotkeyHome,
        Target::Spotlight => windows::Kind::Spotlight,
        _ => windows::Kind::HotkeyThread,
    };
    let saved_target = target.clone();
    let mut spec = windows::Spec::new(kind, "Codex");
    spec.owner = Some(owner.into());
    let events = bridge.subscribe_async();
    let handle = windows::open(spec, cx, |window, cx| {
        cx.new(|cx| {
            let mut root = Root::with_connection(state, bridge, events, cwd, window, cx);
            root.host = host;
            attach(&mut root, owner, target, window, cx);
            root
        })
    })?;
    match saved_target {
        Target::Home => cx.global_mut::<Runtime>().popout = Some(handle),
        Target::Spotlight => cx.global_mut::<Runtime>().spotlight = Some(handle),
        _ => {}
    }
    Ok(handle)
}

fn attach(
    root: &mut Root,
    owner: WindowHandle<Root>,
    target: Target,
    window: &mut Window,
    cx: &mut Context<Root>,
) {
    let composer = root.home.read(cx).composer.clone();
    let projectless = cx
        .try_global::<crate::host_settings::Settings>()
        .is_some_and(|settings| {
            settings
                .read_all()
                .get("hotkey-window-projectless-default-enabled")
                == Some(&json!(true))
        });
    let options = Options {
        root: (!projectless)
            .then(|| root.state.project_root.clone())
            .flatten(),
        projectless,
        ..Default::default()
    };
    let controls = cx.new(|cx| Controls::new(composer.clone(), options, cx));
    cx.subscribe_in(&controls, window, |root, _, change, window, cx| {
        changed(root, change, window, cx)
    })
    .detach();
    root.popout = Some(Popout {
        target,
        owner,
        controls,
        project_read: None,
    });
    root.load_projects(cx);
    refresh(root, cx);
    composer.focus_handle(cx).focus(window, cx);
    if root
        .popout
        .as_ref()
        .is_some_and(|popout| popout.target == Target::Spotlight)
    {
        root.open_command_menu(window, cx);
        if let Some(menu) = &root.command_menu {
            menu.update(cx, |menu, cx| menu.set_standalone(cx));
        }
    }
}

pub(crate) fn projects(
    root: &mut Root,
    projects: &[ui::screens::sidebar::Project],
    cx: &mut Context<Root>,
) {
    let Some(popout) = &root.popout else { return };
    let controls = popout.controls.clone();
    let mut options = controls.read(cx).options().clone();
    options.projects = projects
        .iter()
        .filter(|project| !project.missing)
        .filter_map(|project| Some((project.label.clone(), project.root.as_ref()?.to_string())))
        .collect();
    controls.update(cx, |controls, cx| controls.update_options(options, cx));
    configure_composer(root, cx);
}

fn configure_composer(root: &Root, cx: &mut Context<Root>) {
    let Some(popout) = &root.popout else { return };
    if popout.target != Target::Home {
        return;
    }
    let controls = popout.controls.clone();
    let placeholder = controls.read(cx).options().placeholder();
    let composer = root.home.read(cx).composer.clone();
    composer.update(cx, |composer, cx| {
        composer.set_hotkey_home(controls.into(), placeholder, cx)
    });
}

fn refresh(root: &mut Root, cx: &mut Context<Root>) {
    let Some(popout) = &root.popout else { return };
    let controls = popout.controls.clone();
    let mut options = controls.read(cx).options().clone();
    options.loading = options.root.is_some();
    options.git_root = None;
    options.branches.clear();
    options.branch = None;
    options.environments.clear();
    options.environment = None;
    let selected = options.root.clone();
    controls.update(cx, |controls, cx| controls.update_options(options, cx));
    configure_composer(root, cx);
    let Some(selected) = selected else { return };
    let host = root.host.clone();
    let path = selected.clone();
    let read = cx.background_spawn(async move {
        let status = crate::git_call(&*host, "git-commit-status", json!({"cwd": path}));
        let environments = environments(Path::new(&path));
        (status, environments)
    });
    let task = cx.spawn(async move |root, cx| {
        let (status, environments) = read.await;
        root.update(cx, |root, cx| {
            let mut options = controls.read(cx).options().clone();
            if options.root.as_ref() != Some(&selected) {
                return;
            }
            options.loading = false;
            if let Ok(status) = status {
                options.git_root = status["root"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| status["branch"].as_str().map(|_| selected.clone()));
                options.branch = status["branch"].as_str().map(str::to_owned);
                options.branches = status["branches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
            match environments {
                Ok(environments) => {
                    options.environments = environments;
                    options.environment_error = None;
                }
                Err(error) => options.environment_error = Some(error.to_string()),
            }
            controls.update(cx, |controls, cx| controls.update_options(options, cx));
            configure_composer(root, cx);
            cx.notify();
        })
        .ok();
    });
    if let Some(popout) = &mut root.popout {
        popout.project_read = Some(task);
    }
}

fn environments(root: &Path) -> Result<Vec<(String, String)>> {
    let directory = root.join(".codex/environments");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut options = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)?;
        let config: toml::Value = toml::from_str(&contents)?;
        let name = config
            .get("name")
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        options.push((name, path.to_string_lossy().into_owned()));
    }
    options.sort();
    Ok(options)
}

fn changed(root: &mut Root, change: &Change, window: &mut Window, cx: &mut Context<Root>) {
    let Some(popout) = &root.popout else { return };
    let controls = popout.controls.clone();
    let mut options = controls.read(cx).options().clone();
    match change {
        Change::Project(project) => {
            options.root = project.clone();
            options.projectless = project.is_none();
            options.mode = Mode::Local;
            root.state.project_root = project.clone();
            if let Some(path) = project {
                root.select_project(path.clone(), window, cx);
            }
            controls.update(cx, |controls, cx| controls.update_options(options, cx));
            refresh(root, cx);
            return;
        }
        Change::Mode(mode) => options.mode = *mode,
        Change::Branch(branch) if options.mode == Mode::Local => {
            let Some(cwd) = options.root else { return };
            let host = root.host.clone();
            let branch = branch.clone();
            let result = cx.background_spawn(async move {
                let value = crate::git_call(
                    &*host,
                    "git-checkout",
                    json!({"cwd": cwd, "branch": branch}),
                )?;
                anyhow::ensure!(
                    value["status"] == "success",
                    "{}",
                    value["error"].as_str().unwrap_or("Could not switch branch")
                );
                Ok::<_, anyhow::Error>(())
            });
            cx.spawn(async move |root, cx| {
                let result = result.await;
                root.update(cx, |root, cx| {
                    if let Err(error) = result {
                        root.toast(
                            ui::toast::Toast::danger("Could not switch branch")
                                .description(error.to_string()),
                            cx,
                        );
                    }
                    refresh(root, cx);
                })
                .ok();
            })
            .detach();
            return;
        }
        Change::Branch(branch) => options.branch = Some(branch.clone()),
        Change::Environment(environment) => options.environment = environment.clone(),
        Change::Permission(ui::composer::Permission::FullAccess) => {
            window.dispatch_action(Box::new(ui::composer::RequestFullAccess), cx);
            return;
        }
        Change::Permission(permission) => {
            root.home
                .read(cx)
                .composer
                .clone()
                .update(cx, |composer, cx| {
                    composer.restore_run_settings(&Value::Null, *permission, cx)
                });
        }
        Change::EnvironmentSettings => {
            let owner = popout.owner;
            let result = owner.update(cx, |owner, window, cx| {
                owner.settings_section = Some("Local environments");
                owner.navigate(Route::Settings, window, cx);
            });
            if result.is_ok() {
                let _ = windows::show(owner.into(), windows::Presentation::Foreground, cx);
            }
            return;
        }
    }
    controls.update(cx, |controls, cx| controls.update_options(options, cx));
    configure_composer(root, cx);
}

pub(crate) fn start_options(root: &Root, cx: &App) -> Option<Options> {
    root.popout
        .as_ref()
        .map(|popout| popout.controls.read(cx).options().clone())
}

pub(crate) struct Prepared {
    cwd: String,
    host: Option<Arc<dyn host::Host>>,
    workspace: Option<host::Workspace>,
    worktree: bool,
}

impl Prepared {
    pub(crate) fn local(cwd: String) -> Self {
        Self {
            cwd,
            host: None,
            workspace: None,
            worktree: false,
        }
    }

    pub(crate) fn apply(&self, params: &mut Value) {
        params["cwd"] = json!(self.cwd);
        if let Some(workspace) = &self.workspace {
            params["developerInstructions"] = json!(workspace.instructions());
        }
    }

    pub(crate) fn needs_recording(&self) -> bool {
        self.host.is_some()
    }

    pub(crate) fn record(self, id: &str) -> Result<()> {
        if let Some(host) = self.host {
            if let Some(workspace) = self.workspace {
                host.record_projectless(id, &workspace)?;
            }
            if self.worktree
                && let Err(error) = crate::git_call(
                    &*host,
                    "set-worktree-owner-thread",
                    json!({"worktree": self.cwd, "conversationId": id}),
                )
            {
                eprintln!("codex-gpui: cannot record worktree owner: {error:#}");
            }
        }
        Ok(())
    }
}

pub(crate) fn prepare_start(
    host: Arc<dyn host::Host>,
    options: Options,
    prompt: &str,
) -> Result<Prepared> {
    match options.mode {
        Mode::Local => match options.root {
            Some(cwd) => Ok(Prepared::local(cwd)),
            None => {
                let workspace = host.prepare_projectless(prompt)?;
                Ok(Prepared {
                    cwd: workspace.cwd.clone(),
                    host: Some(host),
                    workspace: Some(workspace),
                    worktree: false,
                })
            }
        },
        Mode::Worktree => {
            let root = options
                .root
                .context("Select a project before starting a worktree")?;
            let params = json!({"cwd": root, "startingState": options.branch.map(|branch| json!({"type":"branch", "branchName":branch})), "localEnvironmentConfigPath":options.environment});
            let response = crate::git_call(&*host, "create-worktree", params)?;
            let cwd = response["worktreeWorkspaceRoot"]
                .as_str()
                .context("Worktree creation returned no working directory")?
                .to_owned();
            Ok(Prepared {
                cwd,
                host: Some(host),
                workspace: None,
                worktree: true,
            })
        }
        Mode::Cloud => bail!("Cloud execution requires a cloud environment"),
    }
}

pub(crate) fn command(
    root: &mut Root,
    event: &ui::screens::sidebar::CommandMenuEvent,
    window: &mut Window,
    cx: &mut Context<Root>,
) -> bool {
    use ui::screens::sidebar::CommandMenuEvent;
    let Some(popout) = root
        .popout
        .as_ref()
        .filter(|popout| popout.target == Target::Spotlight)
    else {
        return false;
    };
    if matches!(
        event,
        CommandMenuEvent::Search(_)
            | CommandMenuEvent::RetryCloudSearch(_)
            | CommandMenuEvent::SearchFiles(_)
    ) {
        return false;
    }
    if matches!(event, CommandMenuEvent::Dismiss) {
        windows::close(window, cx);
        return true;
    }
    let owner = popout.owner;
    if let Some(menu) = &root.command_menu {
        menu.update(cx, |menu, cx| menu.set_open_error(false, cx));
    }
    match owner.update(cx, |root, window, cx| root.run_command(event, window, cx)) {
        Ok(true) => {
            let _ = windows::show(owner.into(), windows::Presentation::Foreground, cx);
            windows::close(window, cx);
        }
        Ok(false) | Err(_) => {
            if let Some(menu) = &root.command_menu {
                menu.update(cx, |menu, cx| menu.set_open_error(true, cx));
            }
        }
    }
    true
}

pub(crate) fn content(root: &Root, cx: &App) -> Option<AnyElement> {
    let popout = root.popout.as_ref()?;
    if popout.target == Target::Spotlight {
        return Some(div().size_full().into_any_element());
    }
    let t = cx.theme();
    if popout.target == Target::Home && matches!(root.state.route, Route::Home) {
        return Some(
            ui::screens::hotkey::home(root.home.read(cx).composer.clone(), t).into_any_element(),
        );
    }
    let remote = root
        .remote_page()
        .filter(|_| matches!(root.state.route, Route::Remote { .. }));
    let title: SharedString = remote
        .and_then(|page| {
            page.read(cx)
                .data
                .as_ref()
                .map(|data| data.title.clone().into())
        })
        .or_else(|| {
            root.thread
                .as_ref()
                .filter(|_| matches!(root.state.route, Route::Thread(_)))
                .map(|thread| thread.read(cx).header_title())
        })
        .unwrap_or_else(|| {
            if popout.target == Target::NewThread {
                "New chat"
            } else {
                "Codex"
            }
            .into()
        });
    let header = div()
        .h(px(44.))
        .px(px(12.))
        .flex()
        .items_center()
        .child(button("Dismiss Popout Window", "close", Dismiss, t))
        .when(!root.back.is_empty(), |header| {
            header.child(button("Back", "arrow-left", ui::GoBack, t))
        })
        .child(div().flex_1().overflow_hidden().child(ui::text(
            title,
            14.,
            20.,
            500.,
            t.text_secondary,
        )))
        .when(popout.target != Target::NewThread, |header| {
            header.child(button("Start New Chat", "plus", Collapse, t))
        })
        .child(button(
            "Open in Main Window",
            "arrow-up-right",
            OpenInMainWindow,
            t,
        ));
    let body = if let Some(remote) = remote {
        div()
            .flex_1()
            .min_h_0()
            .child(remote.clone())
            .into_any_element()
    } else if matches!(root.state.route, Route::Thread(_)) {
        div()
            .flex_1()
            .min_h_0()
            .children(root.thread.clone())
            .into_any_element()
    } else {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::text("Let’s build", 28., 34., 400., t.text)),
            )
            .child(root.home.read(cx).composer.clone())
            .into_any_element()
    };
    Some(
        div()
            .size_full()
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface_elevated)
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element(),
    )
}

fn button<A: gpui::Action>(
    label: &'static str,
    icon: &'static str,
    action: A,
    t: &theme::Theme,
) -> Div {
    let button = ui::composer::pill(label, t)
        .role(gpui::Role::Button)
        .aria_label(label)
        .focusable()
        .tab_index(0)
        .w(px(32.))
        .h(px(32.))
        .justify_center()
        .child(ui::icon::icon(icon, 20., t.text_secondary))
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx));
    div().child(ui::tooltip::tooltip(
        button,
        label,
        None,
        ui::tooltip::Side::Bottom,
    ))
}

pub(crate) fn actions(view: Div, active: bool, cx: &mut Context<Root>) -> Div {
    view.when(active, |view| view.key_context("Popout"))
        .on_action(cx.listener(|root, _: &Dismiss, window, cx| {
            if root.popout.is_some() {
                windows::close(window, cx);
            } else {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|root, _: &Collapse, window, cx| {
            if let Some(popout) = &mut root.popout {
                popout.target = Target::Home;
            }
            root.new_chat(window, cx);
            configure_composer(root, cx);
            windows::set_kind(window, windows::Kind::HotkeyHome, cx);
            let bounds = window.bounds();
            let _ = windows::set_bounds(
                window.window_handle(),
                gpui::Bounds::new(
                    gpui::point(bounds.origin.x, bounds.bottom() - px(290.)),
                    gpui::size(bounds.size.width, px(290.)),
                ),
                cx,
            );
        }))
        .on_action(cx.listener(|root, _: &OpenInMainWindow, window, cx| {
            let Some(popout) = &root.popout else { return };
            let owner = popout.owner;
            let route = match root.state.route.clone() {
                Route::Remote { task_id, .. } => Route::Remote {
                    task_id,
                    hotkey: false,
                },
                route => route,
            };
            if owner
                .update(cx, |root, window, cx| root.navigate(route, window, cx))
                .is_ok()
            {
                let _ = windows::show(owner.into(), windows::Presentation::Foreground, cx);
                windows::close(window, cx);
            }
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_outside_the_popout_family() {
        assert_eq!(
            Target::parse("/hotkey-window/thread/chat-1?host=local").unwrap(),
            Target::Thread("chat-1".into())
        );
        for path in [
            "/",
            "/hotkey-window/thread/",
            "/hotkey-window/thread/a/b",
            "/hotkey-window/nope",
        ] {
            assert!(Target::parse(path).is_err());
        }
    }

    #[test]
    fn clearing_a_global_binding_keeps_it_disabled_without_changing_other_bindings() -> Result<()> {
        let mut overrides = BTreeMap::new();
        overrides.insert("spotlightSearch".into(), Vec::new());
        overrides.insert("hotkeyWindow".into(), vec!["Control+Alt+P".into()]);
        let bindings = configured_bindings(&overrides)?;
        assert!(
            !bindings
                .iter()
                .any(|binding| binding.0 == Shortcut::Spotlight)
        );
        assert!(bindings.contains(&(Shortcut::Popout, Some(35), 0x1800)));
        assert!(
            bindings
                .iter()
                .any(|binding| binding.0 == Shortcut::ScreenshotCurrent)
        );
        assert!(
            bindings
                .iter()
                .any(|binding| binding.0 == Shortcut::ScreenshotNew)
        );
        Ok(())
    }
    #[test]
    fn lists_real_environment_files_and_reports_invalid_config() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let directory = temporary.path().join(".codex/environments");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("dev.toml"),
            "name = 'Development'\nversion = 1\n",
        )?;
        assert_eq!(environments(temporary.path())?[0].0, "Development");
        std::fs::write(directory.join("bad.toml"), "name = [")?;
        assert!(environments(temporary.path()).is_err());
        Ok(())
    }

    #[test]
    fn worktree_chat_uses_an_isolated_checkout_of_the_selected_branch() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("project");
        std::fs::create_dir(&source)?;
        let git = |arguments: &[&str]| -> Result<()> {
            let output = std::process::Command::new("git")
                .current_dir(&source)
                .args(arguments)
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(())
        };
        git(&["init", "-b", "main"])?;
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Initial",
        ])?;
        let host = Arc::new(host::LocalHost::new(temporary.path().join("codex")));
        let options = Options {
            root: Some(source.to_string_lossy().into_owned()),
            git_root: Some(source.to_string_lossy().into_owned()),
            branch: Some("main".into()),
            mode: Mode::Worktree,
            ..Default::default()
        };
        let cwd = prepare_start(host, options, "Create a worktree")?.cwd;
        assert_ne!(Path::new(&cwd), source);
        assert!(Path::new(&cwd).join(".git").is_file());
        assert!(source.join(".git").is_dir());
        Ok(())
    }

    #[test]
    fn popout_keeps_the_primary_draft_and_route_separate() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let oracle: Value = serde_json::from_slice(&std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/route1/oracle.json"),
        )?)?;
        let seen = oracle["persisted-atoms"]["state"]["seen-model-upgrade-list"].clone();
        anyhow::ensure!(seen.is_array(), "Recorded announcement history is missing");
        let mut settings = crate::host_settings::Settings::default();
        settings.write("seen-model-upgrade-list", seen)?;
        let mut app = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
            cx.set_global(settings);
            crate::init(cx);
        });
        app.allow_parking();
        let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl");
        let recorded = std::fs::read_to_string(original)?;
        let lines = recorded
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<serde_json::Result<Vec<_>>>()?;
        let ids = lines
            .iter()
            .filter(|line| {
                line["dir"] == "send"
                    && matches!(
                        line["msg"]["method"].as_str(),
                        Some(
                            "account/read"
                                | "thread/list"
                                | "model/list"
                                | "config/read"
                                | "skills/list"
                        )
                    )
            })
            .map(|line| line["msg"]["id"].clone())
            .collect::<Vec<_>>();
        let repeated = lines
            .iter()
            .filter(|line| !line["msg"]["id"].is_null() && ids.contains(&line["msg"]["id"]))
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let recording = tempfile::NamedTempFile::new()?;
        std::fs::write(
            recording.path(),
            format!("{recorded}\n{repeated}\n{repeated}\n"),
        )?;
        let home = tempfile::tempdir()?;
        let source = home.path().to_string_lossy().into_owned();
        let (bridge, events) = bridge::Bridge::replay(recording.path())?;
        let owner = app.open_window(gpui::size(px(900.), px(700.)), |window, cx| {
            cx.new(|cx| {
                Root::new(
                    crate::State {
                        theme: crate::ThemeSetting::Dark,
                        ..Default::default()
                    },
                    bridge,
                    events,
                    source.clone(),
                    window,
                    cx,
                )
            })
        })?;
        let owner_root = app.read_window(&owner, |root, _| root)?;
        let bridge = owner_root.read_with(&app, |root, _| root.bridge.clone());
        let events = bridge.subscribe_async();
        let popup = app.open_window(gpui::size(px(470.), px(290.)), |window, cx| {
            cx.new(|cx| {
                let mut root = Root::with_connection(
                    crate::State {
                        theme: crate::ThemeSetting::Dark,
                        ..Default::default()
                    },
                    bridge,
                    events,
                    source,
                    window,
                    cx,
                );
                root.host = Arc::new(host::LocalHost::new(home.path()));
                attach(&mut root, owner, Target::Home, window, cx);
                root
            })
        })?;
        app.update_window(popup.into(), |_, window, _| {
            window.set_background_appearance(gpui::WindowBackgroundAppearance::Transparent);
        })?;
        app.run_until_parked();
        let popup_root = app.read_window(&popup, |root, _| root)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !popup_root.read_with(&app, |root, _| root.ready()) {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "Popout startup timed out"
            );
            app.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if popup_root.read_with(&app, |root, _| root.dialog.is_some()) {
            app.test_window(popup.into())
                .simulate_input(gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("escape")?,
                    is_held: false,
                    prefer_character_input: false,
                }));
            app.run_until_parked();
            assert!(popup_root.read_with(&app, |root, _| root.dialog.is_none()));
        }
        if let Some(directory) = std::env::var_os("CGX_HOTKEY_EVIDENCE") {
            std::fs::create_dir_all(&directory)?;
            app.update_window(popup.into(), |_, window, cx| {
                ui::composer::Composer::set_caret_transparent(cx, true);
                window.refresh();
            })?;
            app.run_until_parked();
            app.capture_screenshot(popup.into())?
                .save(Path::new(&directory).join("native-hotkey-home-empty.png"))?;
            for event in [
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position: gpui::point(px(372.), px(258.)),
                    ..Default::default()
                }),
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position: gpui::point(px(372.), px(258.)),
                    ..Default::default()
                }),
            ] {
                app.test_window(popup.into()).simulate_input(event);
            }
            app.run_until_parked();
            app.capture_screenshot(popup.into())?
                .save(Path::new(&directory).join("native-hotkey-settings.png"))?;
            app.test_window(popup.into())
                .simulate_input(gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("escape")?,
                    is_held: false,
                    prefer_character_input: false,
                }));
            app.run_until_parked();
        }
        app.update_window(popup.into(), |_, window, cx| {
            popup_root.update(cx, |root, cx| {
                root.composer(cx).update(cx, |composer, cx| {
                    composer.set_text("Keep this popout draft", cx)
                });
                changed(root, &Change::Project(None), window, cx);
                assert!(root.popout.is_some());
                assert_eq!(
                    root.composer(cx).read(cx).field_text(),
                    "Keep this popout draft"
                );
            });
        })?;
        app.update_window(owner.into(), |_, _, cx| {
            owner_root.update(cx, |root, cx| {
                assert!(root.popout.is_none());
                assert_eq!(root.state.route, Route::Home);
                assert!(root.composer(cx).read(cx).field_text().is_empty());
            });
        })?;
        app.run_until_parked();
        if let Some(directory) = std::env::var_os("CGX_HOTKEY_EVIDENCE") {
            std::fs::create_dir_all(&directory)?;
            app.capture_screenshot(popup.into())?
                .save(Path::new(&directory).join("native-hotkey-home.png"))?;
            let spotlight = app.update(|cx| open(owner, Target::Spotlight, cx))?;
            app.run_until_parked();
            let spotlight_root = app.read_window(&spotlight, |root, _| root)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !spotlight_root.read_with(&app, |root, _| root.ready()) {
                anyhow::ensure!(
                    std::time::Instant::now() < deadline,
                    "Spotlight startup timed out"
                );
                app.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            app.capture_screenshot(spotlight.into())?
                .save(Path::new(&directory).join("native-spotlight-home.png"))?;
        }
        Ok(())
    }
}
