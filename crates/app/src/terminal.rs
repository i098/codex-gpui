use crate::features::shortcuts::ToggleBottomPanel;
use crate::{Root, Route, SidePanel, host_settings};
use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui::{
    App, AppContext as _, BackgroundExecutor, Context, Entity, EventEmitter, Focusable as _,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Window, canvas, div, prelude::FluentBuilder as _, px,
};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use theme::ActiveTheme as _;
use ui::{
    Shell,
    screens::terminal::{TerminalEvent, TerminalStyle, TerminalSurface},
    text,
};

static NEXT_PROCESS: AtomicU64 = AtomicU64::new(1);
const HOME_TERMINAL: &str = "home";
const READ_THREAD_TERMINAL: &str = "read_thread_terminal";

/// The `dynamicTools` entry a new chat starts with, as the original declares it.
pub(crate) fn read_thread_terminal_tool() -> Value {
    json!({
        "name": READ_THREAD_TERMINAL,
        "description": "Read the current app terminal output for this desktop thread. Use it when you need shell output or the current prompt before deciding the next step. This tool takes no arguments.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
    })
}

fn tool_failure(text: &str) -> Value {
    json!({ "success": false, "contentItems": [{ "type": "inputText", "text": text }] })
}

/// The `desktop.integratedTerminalShell` setting, kept in step with the config by `Root`.
pub(crate) struct TerminalShell(pub Option<String>);
impl gpui::Global for TerminalShell {}

impl Root {
    pub(super) fn send_shell(
        &mut self,
        side: Option<usize>,
        submission: ui::composer::Submission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((view, composer)) = self.conversation(side) else {
            return;
        };
        let id = view.read(cx).id().to_owned();
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = async {
                crate::resume_if(resume, &bridge, &this, &id, cx).await?;
                shell_command(&bridge, &id, &submission).await
            }
            .await;
            if result.is_err() {
                this.update(cx, |this, cx| {
                    composer.update(cx, |composer, cx| {
                        composer.restore_text(&submission.text, cx);
                        composer.drop_files(&submission.attachments, cx);
                    });
                    this.toast(ui::toast::Toast::danger("Error submitting message"), cx);
                })?;
            }
            result
        });
        crate::log_err(task, cx);
    }

    /// `environmentAction1`…`environmentAction9` (`⌘⇧D` runs the first): types the command of
    /// the selected local environment's action in that slot into a new terminal tab.
    pub(crate) fn run_environment_action(
        &mut self,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, cwd)) = self.terminal_target(cx) else {
            return;
        };
        let host = self.host.clone();
        let read = cx.background_spawn(async move { environment_actions(&*host, &cwd) });
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let command = read
                .await?
                .get(slot)
                .and_then(|action| action["command"].as_str().map(str::to_owned));
            if let Some(command) = command {
                this.update_in(cx, |this, window, cx| {
                    this.run_in_terminal(&command, window, cx)
                })?;
            }
            Ok(())
        });
        crate::log_err(task, cx);
    }

    /// Gives the open thread's header the names of its workspace's environment actions.
    pub(crate) fn load_environment_actions(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.thread.clone() else {
            return;
        };
        let cwd = view.read(cx).cwd().to_owned();
        let host = self.host.clone();
        let read = cx.background_spawn(async move { environment_actions(&*host, &cwd) });
        let task = cx.spawn(async move |_, cx| -> Result<()> {
            let names = read
                .await?
                .iter()
                .filter_map(|action| action["name"].as_str())
                .map(|name| gpui::SharedString::from(name.to_owned()))
                .collect();
            view.update(cx, |view, cx| view.set_environment_actions(names, cx));
            Ok(())
        });
        crate::log_err(task, cx);
    }

    /// Runs `command` in a new tab of the route's terminal, shown where terminals open.
    fn run_in_terminal(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        let before = self
            .terminal_panel()
            .map_or(0, |panel| panel.read(cx).sessions.len());
        self.show_terminal(false, window, cx);
        let Some(panel) = self.terminal_panel().cloned() else {
            return;
        };
        panel.update(cx, |panel, cx| {
            // `show_terminal` opened a first shell only for a panel that had none.
            if panel.sessions.len() == before {
                panel.open(cx);
            }
            panel.write(panel.active, format!("{command}\n").into_bytes());
        });
    }
}

/// The run actions of `cwd`'s selected local environment (`local-environment-actions`).
fn environment_actions(host: &dyn crate::host::Host, cwd: &str) -> Result<Vec<Value>> {
    let request = crate::host::Request::post(
        "local-environment-actions",
        Some(serde_json::json!({ "workspaceRoot": cwd })),
    );
    let mut response = crate::host::json_body(&host.fetch(&request)?)?;
    Ok(match response["actions"].take() {
        Value::Array(actions) => actions,
        _ => Vec::new(),
    })
}

async fn shell_command(
    bridge: &bridge::Bridge,
    thread: &str,
    submission: &ui::composer::Submission,
) -> Result<()> {
    anyhow::ensure!(
        submission.permission == ui::composer::Permission::FullAccess,
        "Shell commands require Full access"
    );
    anyhow::ensure!(
        submission.attachments.is_empty() && submission.context_files.is_empty(),
        "Shell commands cannot include attachments"
    );
    let command = submission.text.strip_prefix('!').unwrap_or_default();
    anyhow::ensure!(!command.trim().is_empty(), "Shell command is empty");
    bridge
        .request::<_, Value>(
            "thread/shellCommand",
            serde_json::json!({ "threadId": thread, "command": command }),
        )
        .await?;
    Ok(())
}

#[derive(Default)]
pub struct Terminals {
    panels: HashMap<String, Entity<TerminalPanel>>,
    /// The bottom panel is shown (the terminal may live in the side panel instead).
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

enum Output {
    Bytes(Vec<u8>),
    Error(String),
    /// A cloud shell's input status: why typed input may not have reached it, or `None`.
    Input(Option<String>),
    Finished,
}

/// A tab's running shell.
enum Process {
    Local(Pty),
    Cloud(CloudShell),
}

impl Process {
    fn send(&self, bytes: Vec<u8>) -> Result<()> {
        match self {
            Process::Local(pty) => pty.input.send(bytes)?,
            Process::Cloud(shell) => shell.input.try_send(bytes)?,
        }
        Ok(())
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        match self {
            Process::Local(pty) => pty.resize(rows, cols),
            // The original sizes a cloud shell once, when it starts.
            Process::Cloud(_) => Ok(()),
        }
    }
}

/// A shell on a local pseudo-terminal; dropping it kills the shell.
struct Pty {
    master: Box<dyn MasterPty + Send>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    input: mpsc::Sender<Vec<u8>>,
}

impl Pty {
    fn spawn(command: CommandBuilder) -> Result<(Self, async_channel::Receiver<Output>)> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let (output, receive) = async_channel::bounded(32);
        let errors = output.clone();
        let (input, pending) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in pending {
                if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                    let _ = errors.send_blocking(Output::Error(error.to_string()));
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        if output
                            .send_blocking(Output::Bytes(bytes[..count].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if error.raw_os_error() != Some(5) {
                            let _ = output.send_blocking(Output::Error(error.to_string()));
                        }
                        break;
                    }
                }
            }
            let _ = output.send_blocking(Output::Finished);
        });
        Ok((
            Self {
                master: pair.master,
                child: Some(child),
                input,
            },
            receive,
        ))
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            });
        }
    }
}

/// Runs before the login shell of a cloud terminal: sizes the pseudo-terminal and marks each
/// prompt (OSC 133), word for word as the original sends it.
const CLOUD_PROLOGUE: &str = r#"stty cols "$1" rows "$2"; printf "\033[0m"; shift 2
PROMPT_COMMAND='__codex_prompt_status=$?; __codex_prompt_start() { printf "\033]133;A\007"; return "$__codex_prompt_status"; }; __codex_prompt_start'"${PROMPT_COMMAND:+
$PROMPT_COMMAND}"
export PROMPT_COMMAND; exec "$@""#;

/// The environment user's login shell, else `/bin/bash`.
const CLOUD_LOGIN_SHELL: &str = r#"login_shell="${SHELL:-}"
if [ -z "$login_shell" ] && command -v getent >/dev/null 2>&1; then
  login_shell="$(getent passwd "$(id -un)" | cut -d: -f7 || true)"
fi
if [ -z "$login_shell" ] || [ ! -x "$login_shell" ]; then
  login_shell="/bin/bash"
fi
exec "$login_shell" -il"#;

/// The server ends a cloud shell after this much output.
const CLOUD_OUTPUT_CAP: u64 = 1024 * 1024;

/// A shell in a cloud task's environment: a `command/exec` process with a pseudo-terminal on
/// the durable app-server, as the original's Cloud terminal runs it. Dropping it ends the
/// process.
struct CloudShell {
    bridge: Arc<bridge::Bridge>,
    process: String,
    input: async_channel::Sender<Vec<u8>>,
    executor: BackgroundExecutor,
    /// The server answered `command/exec`: the process has ended.
    done: Arc<AtomicBool>,
    _tasks: [gpui::Task<()>; 2],
}

impl CloudShell {
    /// Resolves the environment's working directory, then starts the shell there.
    async fn spawn(
        bridge: Arc<bridge::Bridge>,
        environment: String,
        (rows, cols): (u16, u16),
        executor: BackgroundExecutor,
    ) -> Result<(Self, async_channel::Receiver<Output>, String)> {
        let cwd = cloud_cwd(&bridge, &environment, &executor).await?;
        let process = format!(
            "gpui-cloud-terminal-{}-{}",
            std::process::id(),
            NEXT_PROCESS.fetch_add(1, Ordering::Relaxed)
        );
        let params = json!({
            "environmentId": environment,
            "processId": process,
            "command": [
                "sh", "-c", CLOUD_PROLOGUE, "sh", cols.to_string(), rows.to_string(),
                "sh", "-c", CLOUD_LOGIN_SHELL,
            ],
            "cwd": cwd,
            "env": {"TERM": "xterm-256color", "TERMINFO": null, "TERMINFO_DIRS": null},
            "tty": true,
            "streamStdin": true,
            "streamStdoutStderr": true,
            "disableTimeout": true,
            "outputBytesCap": CLOUD_OUTPUT_CAP,
        });
        let events = bridge.subscribe_async();
        let (output, receive) = async_channel::unbounded();
        let (input, pending) = async_channel::unbounded::<Vec<u8>>();
        // Closed at the shell's first output: the original holds input until then.
        let (ready, started) = async_channel::bounded::<()>(1);
        let done = Arc::new(AtomicBool::new(false));
        let run = executor.spawn(run_cloud_shell(
            bridge.clone(),
            params,
            events,
            output.clone(),
            ready,
            done.clone(),
        ));
        let write = executor.spawn({
            let (bridge, process, done) = (bridge.clone(), process.clone(), done.clone());
            async move {
                let _ = started.recv().await;
                let mut status = None;
                while let Ok(bytes) = pending.recv().await {
                    if done.load(Ordering::Acquire) {
                        continue;
                    }
                    let sent = bridge
                        .request::<_, Value>(
                            "command/exec/write",
                            json!({"processId": process, "deltaBase64": BASE64.encode(bytes)}),
                        )
                        .await
                        .err()
                        .map(|error| error.to_string());
                    if sent != status {
                        status = sent.clone();
                        if output.send(Output::Input(sent)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let shell = Self {
            bridge,
            process,
            input,
            executor,
            done,
            _tasks: [run, write],
        };
        Ok((shell, receive, cwd))
    }
}

impl Drop for CloudShell {
    fn drop(&mut self) {
        if !self.done.load(Ordering::Acquire) {
            let (bridge, process) = (self.bridge.clone(), self.process.clone());
            self.executor
                .spawn(async move {
                    let _ = bridge
                        .request::<_, Value>(
                            "command/exec/terminate",
                            json!({ "processId": process }),
                        )
                        .await;
                })
                .detach();
        }
    }
}

/// Forwards the process's output until `command/exec` answers; ends with `Finished` for an
/// exited shell, else `Error` with why the connection to it was lost.
async fn run_cloud_shell(
    bridge: Arc<bridge::Bridge>,
    params: Value,
    events: async_channel::Receiver<bridge::Event>,
    output: async_channel::Sender<Output>,
    ready: async_channel::Sender<()>,
    done: Arc<AtomicBool>,
) {
    let process = params["processId"].clone();
    let mut ready = Some(ready);
    let mut exec = std::pin::pin!(bridge.request::<_, Value>("command/exec", params));
    let error = loop {
        // Events first: the server sends a process's last output before its answer.
        let next = futures_lite::future::or(async { Ok(events.recv().await) }, async {
            Err(exec.as_mut().await)
        })
        .await;
        let answer = match next {
            Ok(Ok(bridge::Event::Notification { method, params }))
                if method == "command/exec/outputDelta" && params["processId"] == process =>
            {
                ready = None;
                let Some(bytes) = params["deltaBase64"]
                    .as_str()
                    .and_then(|delta| BASE64.decode(delta).ok())
                else {
                    break Some("Cloud terminal received invalid output".to_owned());
                };
                let _ = output.send(Output::Bytes(bytes)).await;
                if params["capReached"] == true {
                    break Some("Cloud terminal reached the server's 1 MiB output limit".into());
                }
                continue;
            }
            Ok(Ok(bridge::Event::Notification { .. } | bridge::Event::Request { .. })) => continue,
            // A closed connection fails its unanswered requests, so the answer is ready.
            Ok(Ok(bridge::Event::Closed) | Err(_)) => exec.as_mut().await,
            Err(answer) => answer,
        };
        done.store(true, Ordering::Release);
        break match answer {
            Ok(exit) if exit["exitCode"].is_i64() => None,
            Ok(_) => Some("Cloud terminal received an invalid exit".into()),
            Err(error) => Some(error.to_string()),
        };
    };
    drop(ready);
    let _ = output
        .send(error.map_or(Output::Finished, Output::Error))
        .await;
}

/// The environment's working directory, as `environment/info` reports it, once the server
/// confirms it is a directory. An offline environment is asked again for up to five minutes.
async fn cloud_cwd(
    bridge: &bridge::Bridge,
    environment: &str,
    executor: &BackgroundExecutor,
) -> Result<String> {
    const RETRY: Duration = Duration::from_secs(5);
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        match environment_cwd(bridge, environment).await {
            // shortcut: polls every 5 s; the original also wakes on `thread/environment/connected`.
            Err(error) if environment_offline(&error) && Instant::now() + RETRY <= deadline => {
                executor.timer(RETRY).await;
            }
            result => {
                return result?
                    .context("The terminal environment did not report a valid working directory");
            }
        }
    }
}

async fn environment_cwd(bridge: &bridge::Bridge, environment: &str) -> Result<Option<String>> {
    let info: Value = bridge
        .request("environment/info", json!({ "environmentId": environment }))
        .await?;
    let Some(cwd) = info["cwd"].as_str() else {
        return Ok(None);
    };
    let cwd = if cwd.starts_with("file:") {
        match url::Url::parse(cwd)
            .ok()
            .and_then(|url| url.to_file_path().ok())
        {
            Some(path) => path.to_string_lossy().into_owned(),
            None => return Ok(None),
        }
    } else {
        cwd.to_owned()
    };
    if !cwd.starts_with('/') {
        return Ok(None);
    }
    let metadata = bridge
        .request::<_, Value>(
            "fs/getMetadata",
            json!({ "environmentId": environment, "path": cwd }),
        )
        .await;
    let directory = match metadata {
        Ok(metadata) => metadata["isDirectory"] == true,
        Err(error)
            if error
                .downcast_ref::<bridge::RpcError>()
                .is_some_and(|error| {
                    matches!(
                        error.message.as_str(),
                        "No such file or directory (os error 2)" | "Not a directory (os error 20)"
                    )
                }) =>
        {
            false
        }
        Err(error) => return Err(error),
    };
    Ok(directory.then_some(cwd))
}

/// The environment registry's answer for an environment that is not running yet.
fn environment_offline(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<bridge::RpcError>() else {
        return false;
    };
    ["404 Not Found", "409 Conflict"].iter().any(|status| {
        error
            .message
            .strip_prefix("environment registry connect returned ")
            .and_then(|rest| rest.strip_prefix(status))
            .and_then(|rest| rest.strip_prefix(": environment_offline"))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
    })
}

/// The durable app-server a cloud thread runs on and the environments it reported
/// (`thread/environment/connected`), which a cloud terminal panel runs its shells in.
#[derive(Clone, Default)]
pub struct CloudTarget {
    pub bridge: Option<Arc<bridge::Bridge>>,
    /// The thread's environment, kept after it disconnects.
    pub environment: Option<String>,
    /// The environments connected now.
    pub connected: Vec<String>,
}

impl CloudTarget {
    /// Applies a `thread/environment/connected` or `…/disconnected` notification.
    pub fn apply(&mut self, method: &str, environment: &str) {
        // The original ignores the managed placeholder environment.
        if environment == "managed" {
            return;
        }
        match method {
            "thread/environment/connected" => {
                self.environment = Some(environment.to_owned());
                if !self.connected.iter().any(|id| id == environment) {
                    self.connected.push(environment.to_owned());
                }
            }
            "thread/environment/disconnected" => self.connected.retain(|id| id != environment),
            _ => {}
        }
    }
}

/// The quiet status row under a cloud terminal, as the original words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloudStatus {
    Connecting,
    Reconnecting,
    EnvironmentChanged,
    InputInterrupted,
    ShellExited,
    ConnectionFailed,
}

impl CloudStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Reconnecting => "Reconnecting…",
            Self::EnvironmentChanged => "Environment changed",
            Self::InputInterrupted => "Input may be incomplete",
            Self::ShellExited => "Shell exited",
            Self::ConnectionFailed => "Couldn’t reconnect",
        }
    }
}

/// The cloud facts of one terminal tab.
#[derive(Default)]
struct CloudState {
    /// The environment the tab's shell runs in, fixed when it first starts.
    environment: Option<String>,
    has_output: bool,
    reconnecting: bool,
    /// Why the connection to the shell was lost.
    error: Option<String>,
    input_error: Option<String>,
    reconnect: Reconnect,
}

impl CloudState {
    fn attached(&mut self, now: Instant) {
        self.reconnect.connected_at = Some(now);
        self.reconnecting = false;
        self.error = None;
        self.input_error = None;
    }

    /// `None` while the tab waits for an environment; else whether `Opening terminal…` covers
    /// the terminal, and the status row under it.
    fn view(
        &self,
        target: &CloudTarget,
        attached: bool,
        exited: bool,
    ) -> Option<(bool, Option<CloudStatus>)> {
        let environment = self
            .environment
            .as_deref()
            .or(target.environment.as_deref())?;
        let stopped = exited || self.error.is_some();
        let changed = self.environment.is_some()
            && target.environment.is_some()
            && self.environment != target.environment;
        let status = if changed {
            Some(CloudStatus::EnvironmentChanged)
        } else if self.reconnecting {
            Some(CloudStatus::Reconnecting)
        } else if stopped {
            Some(if self.error.is_some() {
                CloudStatus::ConnectionFailed
            } else {
                CloudStatus::ShellExited
            })
        } else if self.input_error.is_some() {
            Some(CloudStatus::InputInterrupted)
        } else if !attached
            && (self.has_output || !target.connected.iter().any(|id| id == environment))
        {
            Some(CloudStatus::Connecting)
        } else {
            None
        };
        let overlay = !stopped && !self.reconnecting && !attached && !self.has_output;
        Some((overlay, status))
    }
}

/// The original's automatic recovery of a cloud shell: three attempts, 1 s, 2 s and 4 s after
/// a loss, counted again once a shell stayed connected for 30 s.
#[derive(Default)]
struct Reconnect {
    attempts: usize,
    connected_at: Option<Instant>,
}

impl Reconnect {
    const DELAYS: [Duration; 3] = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
    ];

    /// The delay before the next attempt, or `None` once they ran out.
    fn schedule(&mut self, now: Instant) -> Option<Duration> {
        if self
            .connected_at
            .take()
            .is_some_and(|at| now.duration_since(at) >= Duration::from_secs(30))
        {
            self.attempts = 0;
        }
        let delay = *Self::DELAYS.get(self.attempts)?;
        self.attempts += 1;
        Some(delay)
    }
}

impl Root {
    pub fn terminal_panel(&self) -> Option<&Entity<TerminalPanel>> {
        let id = match &self.state.route {
            Route::Home => HOME_TERMINAL,
            Route::Thread(id) => id,
            Route::CloudSetup { config_id, .. } => config_id,
            _ => return None,
        };
        self.terminal.panels.get(id)
    }

    /// The durable thread of a cloud setup route, whose terminal runs in its environment.
    fn cloud_terminal(&self) -> Option<&CloudTarget> {
        match &self.state.route {
            Route::CloudSetup { .. } => self.cloud.as_ref()?.terminal.as_ref(),
            _ => None,
        }
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
            Route::CloudSetup { config_id, .. } => self
                .cloud_terminal()
                .map(|_| (config_id.clone(), String::new())),
            _ => None,
        }
    }

    /// Gives the open cloud setup route's terminal the durable thread's latest target.
    pub fn set_cloud_terminal(
        &mut self,
        page: &Entity<ui::screens::cloud_task::CloudTaskPage>,
        target: CloudTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cloud.as_mut().filter(|session| session.page == *page) else {
            return;
        };
        session.terminal = Some(target.clone());
        if let Some(panel) = self.terminal_panel().cloned() {
            panel.update(cx, |panel, cx| panel.set_cloud(target, cx));
        }
    }

    /// Opens the route's terminal panel (a shell in its first tab) and shows it in the bottom
    /// panel, or in the side panel when the user's default location is not the bottom.
    pub(crate) fn show_terminal(
        &mut self,
        force_bottom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((id, cwd)) = self.terminal_target(cx) else {
            return;
        };
        let cloud = self.cloud_terminal().cloned();
        if let std::collections::hash_map::Entry::Vacant(slot) = self.terminal.panels.entry(id) {
            let panel = cx.new(|cx| TerminalPanel::new(cwd, cloud, cx));
            cx.subscribe_in(&panel, window, |root, _, _: &PanelChanged, window, cx| {
                if root.side == Some(SidePanel::Console)
                    && root
                        .terminal_panel()
                        .is_some_and(|panel| panel.read(cx).sessions.is_empty())
                {
                    root.close_side(SidePanel::Console, window, cx);
                }
                root.sync_terminal(cx)
            })
            .detach();
            slot.insert(panel);
        }
        if let Some(panel) = self.terminal_panel().cloned()
            && panel.read(cx).sessions.is_empty()
        {
            panel.update(cx, |panel, cx| panel.open(cx));
        }
        let bottom = force_bottom
            || !matches!(self.state.route, Route::Thread(_))
            || self.config["desktop"]["defaultTerminalLocation"]
                .as_str()
                .unwrap_or("bottom")
                == "bottom";
        self.terminal.visible = bottom;
        if bottom {
            if self.side == Some(SidePanel::Console) {
                self.close_side(SidePanel::Console, window, cx);
            }
            self.sync_terminal(cx);
        } else {
            self.sync_terminal(cx);
            self.set_side(SidePanel::Console, window, cx);
        }
        let surface = self
            .terminal_panel()
            .and_then(|panel| panel.read(cx).surface().cloned());
        if let Some(surface) = surface {
            surface.focus_handle(cx).focus(window, cx);
        }
    }

    /// Runs an `environmentAction1`…`9` shortcut from the environment selected in settings.
    pub(crate) fn run_selected_environment_action(
        &mut self,
        number: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ui::screens::computer_use::Load::Ready(config) = &cx
            .global::<ui::screens::settings::environments::State>()
            .selected
        else {
            return;
        };
        let Some(command) = Self::environment_action_command(config, number) else {
            return;
        };
        let command = format!("{command}\r").into_bytes();
        self.show_terminal(false, window, cx);
        if let Some(panel) = self.terminal_panel().cloned() {
            panel.update(cx, |panel, _| {
                let active = panel.active;
                panel.write(active, command);
            });
        }
    }

    fn environment_action_command(config: &serde_json::Value, number: usize) -> Option<&str> {
        if !(1..=9).contains(&number) {
            return None;
        }
        config["environment"]["environment"]["actions"]
            .as_array()?
            .get(number - 1)
            .filter(|action| {
                action["platform"].as_str().is_none_or(|platform| {
                    platform
                        == if cfg!(target_os = "macos") {
                            "darwin"
                        } else if cfg!(windows) {
                            "win32"
                        } else {
                            "linux"
                        }
                })
            })
            .and_then(|action| action["command"].as_str())
    }

    pub(crate) fn toggle_bottom_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.visible
            && self
                .terminal_panel()
                .is_some_and(|panel| !panel.read(cx).sessions.is_empty())
        {
            self.terminal.visible = false;
            self.sync_terminal(cx);
            self.composer(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.show_terminal(true, window, cx);
        }
    }

    pub fn terminal_visible(&self) -> bool {
        self.terminal_panel().is_some()
            && (self.terminal.visible || self.side == Some(SidePanel::Console))
    }

    /// The answer to an `item/tool/call` of `read_thread_terminal`, or `None` for another tool:
    /// the active tab of the calling thread's terminal, as the original words it.
    pub(crate) fn read_thread_terminal(&self, params: &Value, cx: &App) -> Option<Value> {
        if params["tool"] != READ_THREAD_TERMINAL {
            return None;
        }
        if !params["arguments"].is_null()
            && params["arguments"]
                .as_object()
                .is_none_or(|a| !a.is_empty())
        {
            return Some(tool_failure(&format!(
                "{READ_THREAD_TERMINAL} takes no arguments."
            )));
        }
        let session = params["threadId"]
            .as_str()
            .and_then(|id| self.terminal.panels.get(id))
            .map(|panel| panel.read(cx))
            .and_then(|panel| panel.sessions.get(panel.active));
        let Some(session) = session else {
            return Some(tool_failure(
                "No app terminal session is attached to this thread yet.",
            ));
        };
        let buffer = &session.surface.read(cx).buffer;
        let output = buffer
            .selected(
                (0, 0),
                (
                    buffer.rows.len().saturating_sub(1),
                    usize::from(buffer.size.1),
                ),
            )
            .trim_end()
            .to_owned();
        let mut lines = vec![
            "App terminal snapshot for this thread:".to_owned(),
            format!("cwd: {}", session.cwd),
            format!("shell: {}", configured_shell(cx)),
        ];
        if buffer.truncated() {
            lines.push(
                "note: output is truncated to the latest terminal buffer kept by the app.".into(),
            );
        }
        lines.push("```text".into());
        lines.push(if output.is_empty() {
            "[terminal has no output yet]".into()
        } else {
            output
        });
        lines.push("```".into());
        Some(json!({
            "success": true,
            "contentItems": [{ "type": "inputText", "text": lines.join("\n") }],
        }))
    }

    /// The active tab's screen text.
    pub fn terminal_contents(&self, cx: &App) -> Option<String> {
        let surface = self.terminal_panel()?.read(cx).surface()?;
        let buffer = &surface.read(cx).buffer;
        Some(
            buffer
                .rows
                .iter()
                .map(|row| row.text(0, row.cells.len()))
                .collect::<Vec<_>>()
                .join("\n"),
        )
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
            && panel.read(cx).cloud.is_none()
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
        shell.update(cx, |shell: &mut Shell, cx| shell.set_bottom(bottom, cx));
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
    /// `None` until the shell has started, and while a cloud shell reconnects.
    shell: Option<Process>,
    /// Input the surface sent before the shell started.
    pending_input: Vec<Vec<u8>>,
    /// The surface's last size: a shell that starts later, a reconnected one too, takes it.
    size: Option<(u16, u16)>,
    /// Counts the shell's starts, so a replaced shell's output and timers are dropped.
    generation: u64,
    cloud: CloudState,
}

/// The Settings shell when it still exists, else the platform default.
fn configured_shell(cx: &App) -> String {
    cx.try_global::<TerminalShell>()
        .and_then(|shell| shell.0.clone())
        .filter(|shell| std::path::Path::new(shell).is_file())
        .unwrap_or_else(ui::screens::settings::default_shell)
}

pub struct PanelChanged;
impl EventEmitter<PanelChanged> for TerminalPanel {}

pub struct TerminalPanel {
    sessions: Vec<Session>,
    active: usize,
    hovered: Option<usize>,
    strip_width: f32,
    strip_left: f32,
    cwd: String,
    roots: Vec<String>,
    workspaces_loaded: bool,
    focus_pending: bool,
    /// Set for a cloud thread's panel: its tabs run in the thread's environment.
    cloud: Option<CloudTarget>,
}

impl TerminalPanel {
    fn new(cwd: String, cloud: Option<CloudTarget>, cx: &mut Context<Self>) -> Self {
        let mut panel = Self {
            sessions: Vec::new(),
            active: 0,
            hovered: None,
            strip_width: 0.,
            strip_left: 0.,
            cwd: cwd.clone(),
            roots: vec![cwd],
            workspaces_loaded: false,
            focus_pending: false,
            cloud,
        };
        panel.open(cx);
        panel
    }

    /// The active cloud tab's view: `None` while it waits for an environment, else whether
    /// `Opening terminal…` covers it and its status row.
    fn cloud_view(&self) -> Option<Option<(bool, Option<CloudStatus>)>> {
        let target = self.cloud.as_ref()?;
        let session = self.sessions.get(self.active)?;
        Some(
            session
                .cloud
                .view(target, session.shell.is_some(), session.exited),
        )
    }

    /// What the active cloud tab says about its shell: the waiting or opening title, and the
    /// status row; empty for a local panel or a running cloud shell.
    pub fn cloud_status(&self) -> Vec<&'static str> {
        match self.cloud_view() {
            None => Vec::new(),
            Some(None) => vec!["Waiting for environment…"],
            Some(Some((overlay, status))) => overlay
                .then_some("Opening terminal…")
                .into_iter()
                .chain(status.map(CloudStatus::label))
                .collect(),
        }
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

    /// Shows `message` as the active tab's terminal error.
    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.get_mut(self.active) {
            session.error = Some(message);
            cx.notify();
        }
    }

    pub fn workspace_warning(&self) -> bool {
        if self.cloud.is_some() {
            return false;
        }
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
                TerminalEvent::Input(bytes) => panel.write(index, bytes.clone()),
                TerminalEvent::Resize { rows, cols } => panel.resize(index, *rows, *cols),
                TerminalEvent::New => panel.open(cx),
                TerminalEvent::Directory(number) => panel.change_directory(*number, cx),
            }
        })
        .detach();
        self.active = self.sessions.len();
        self.sessions.push(Session {
            id: id.clone(),
            cwd: if self.cloud.is_some() {
                String::new()
            } else {
                self.cwd.clone()
            },
            surface,
            error: None,
            exited: false,
            warning_dismissed: false,
            shell: None,
            pending_input: Vec::new(),
            size: None,
            generation: 0,
            cloud: CloudState::default(),
        });
        self.start(&id, cx);
        self.focus_pending = true;
        cx.emit(PanelChanged);
        cx.notify();
    }

    /// Starts the shell of tab `id`: a local login shell, or for a cloud panel a shell in the
    /// environment the tab is bound to (the target's current one for a new tab). A cloud tab
    /// without an environment waits; `set_cloud` starts it once the thread reports one.
    fn start(&mut self, id: &str, cx: &mut Context<Self>) {
        let (cloud, cwd) = (self.cloud.clone(), self.cwd.clone());
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let started = match cloud {
            None => {
                // Read per tab so a changed setting applies to the next new terminal.
                let mut command = CommandBuilder::new(configured_shell(cx));
                command.arg("-l");
                command.cwd(cwd);
                command.env("TERM", "xterm-256color");
                command.env("COLORTERM", "truecolor");
                cx.background_executor().spawn(async move {
                    Pty::spawn(command).map(|(pty, receive)| (Process::Local(pty), receive, None))
                })
            }
            Some(target) => {
                let (Some(bridge), Some(environment)) = (
                    target.bridge,
                    session.cloud.environment.clone().or(target.environment),
                ) else {
                    return;
                };
                session.cloud.environment = Some(environment.clone());
                let size = session.size.unwrap_or((24, 80));
                let executor = cx.background_executor().clone();
                cx.background_executor().spawn(async move {
                    let (shell, receive, cwd) =
                        CloudShell::spawn(bridge, environment, size, executor).await?;
                    Ok::<_, anyhow::Error>((Process::Cloud(shell), receive, Some(cwd)))
                })
            }
        };
        session.generation += 1;
        let generation = session.generation;
        let id = id.to_owned();
        cx.spawn(async move |panel, cx| {
            let receive = match started.await {
                Ok((shell, receive, cwd)) => {
                    let ready = panel.update(cx, |panel, cx| {
                        let Some(session) = panel.session(&id, generation) else {
                            return false;
                        };
                        if let Some((rows, cols)) = session.size
                            && let Err(error) = shell.resize(rows, cols)
                        {
                            session.error = Some(error.to_string());
                        }
                        for bytes in session.pending_input.drain(..) {
                            if let Err(error) = shell.send(bytes) {
                                session.error = Some(error.to_string());
                                break;
                            }
                        }
                        if let Some(cwd) = cwd {
                            session.cwd = cwd;
                            session.cloud.attached(std::time::Instant::now());
                            session.exited = false;
                            session
                                .surface
                                .update(cx, |surface, _| surface.writable = true);
                        }
                        session.shell = Some(shell);
                        cx.notify();
                        true
                    });
                    if !ready.unwrap_or(false) {
                        return;
                    }
                    receive
                }
                Err(error) => {
                    panel
                        .update(cx, |panel, cx| {
                            panel.finish(&id, generation, Some(error.to_string()), cx)
                        })
                        .ok();
                    return;
                }
            };
            while let Ok(output) = receive.recv().await {
                let alive = panel.update(cx, |panel, cx| {
                    let cloud = panel.cloud.is_some();
                    let Some(session) = panel.session(&id, generation) else {
                        return false;
                    };
                    match output {
                        Output::Bytes(bytes) => {
                            session.cloud.has_output = true;
                            session
                                .surface
                                .update(cx, |surface, cx| surface.append(&bytes, cx));
                        }
                        Output::Error(error) if cloud => {
                            panel.finish(&id, generation, Some(error), cx);
                            return false;
                        }
                        Output::Error(error) => session.error = Some(error),
                        Output::Input(error) => session.cloud.input_error = error,
                        Output::Finished => {
                            panel.finish(&id, generation, None, cx);
                            return false;
                        }
                    }
                    cx.notify();
                    true
                });
                if !alive.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Tab `id`, unless its shell was restarted since `generation` started.
    fn session(&mut self, id: &str, generation: u64) -> Option<&mut Session> {
        self.sessions
            .iter_mut()
            .find(|s| s.id == id && s.generation == generation)
    }

    /// Marks the shell of tab `id` as ended: its surface stops taking input. A cloud shell that
    /// lost its connection (`error`) opens again after the reconnect delay while attempts remain.
    fn finish(&mut self, id: &str, generation: u64, error: Option<String>, cx: &mut Context<Self>) {
        let cloud = self.cloud.is_some();
        let Some(session) = self.session(id, generation) else {
            return;
        };
        session.shell = None;
        cx.notify();
        if cloud {
            session.cloud.input_error = None;
            session.cloud.reconnecting = false;
            match &error {
                None => session.cloud.reconnect = Reconnect::default(),
                Some(_) => {
                    if let Some(delay) = session.cloud.reconnect.schedule(std::time::Instant::now())
                    {
                        session.cloud.reconnecting = true;
                        session.cloud.error = error;
                        let id = id.to_owned();
                        cx.spawn(async move |panel, cx| {
                            cx.background_executor().timer(delay).await;
                            panel
                                .update(cx, |panel, cx| {
                                    if panel.session(&id, generation).is_some() {
                                        panel.start(&id, cx);
                                    }
                                })
                                .ok();
                        })
                        .detach();
                        return;
                    }
                }
            }
            session.cloud.error = error;
        } else if error.is_some() {
            session.error = error;
        }
        session.exited = true;
        session.surface.update(cx, |surface, cx| {
            surface.writable = false;
            cx.notify();
        });
    }

    /// `Retry`: a fresh shell in the same cloud tab, in the environment it is bound to.
    fn restart(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get_mut(index).filter(|s| s.exited) else {
            return;
        };
        session.exited = false;
        session.cloud.error = None;
        session.cloud.reconnecting = true;
        let id = session.id.clone();
        self.start(&id, cx);
    }

    /// The durable thread's current target; tabs that waited for an environment start now.
    pub fn set_cloud(&mut self, target: CloudTarget, cx: &mut Context<Self>) {
        self.cloud = Some(target);
        let waiting: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.shell.is_none() && !s.exited && s.cloud.environment.is_none())
            .map(|s| s.id.clone())
            .collect();
        for id in waiting {
            self.start(&id, cx);
            // The waiting notice replaced the terminal, so it lost focus.
            self.focus_pending = true;
        }
        cx.notify();
    }

    fn write(&mut self, index: usize, bytes: Vec<u8>) {
        let Some(session) = self.sessions.get_mut(index).filter(|s| !s.exited) else {
            return;
        };
        match &session.shell {
            Some(shell) => {
                if let Err(error) = shell.send(bytes) {
                    session.error = Some(error.to_string());
                }
            }
            None => session.pending_input.push(bytes),
        }
    }

    fn resize(&mut self, index: usize, rows: u16, cols: u16) {
        let Some(session) = self.sessions.get_mut(index).filter(|s| !s.exited) else {
            return;
        };
        session.size = Some((rows, cols));
        if let Some(shell) = &session.shell
            && let Err(error) = shell.resize(rows, cols)
        {
            session.error = Some(error.to_string());
        }
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index);
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
        self.write(self.active, command.into_bytes());
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
    /// A fresh shell in the same cloud tab.
    Retry,
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
            PanelAction::Retry => self.restart(self.active, cx),
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
            .border_color(theme.border)
            // The original rasters the strip into one composited layer (tab glyphs, `+`).
            .layer();
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
                .map_or("Terminal".into(), |name| name.to_string_lossy());
            let label = if self.sessions.len() == 1 {
                name.into_owned()
            } else {
                format!("{name} {}", index + 1)
            };
            // Chromium snaps each tab slot's origin to the device grid and lays the tab out from
            // there.
            let slot_origin = self.strip_left + 8. + slot_width * index as f32;
            let slot_snap = (slot_origin * 2. + 0.5).floor() / 2. - slot_origin;
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
            .left(px(slot_snap))
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
                    .px_0(),
                );
            }
            if index + 1 < self.sessions.len()
                && !selected
                && !hovered
                && self.active != index + 1
                && self.hovered != Some(index + 1)
            {
                tab = tab.child(
                    div()
                        .absolute()
                        .right(px(-2.))
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
                            .child(
                                ui::icon::icon("diff-new-tab", 16., theme.text_tertiary).runtime(),
                            ),
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
                    // The original composites this glyph on its own.
                    .child(
                        ui::icon::icon("chevron-down-medium", 16., theme.text_tertiary)
                            .own_layer(None),
                    ),
            );
        let mut panel = div()
            .id("interactive-terminal-panel")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
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
                        .relative()
                        .px(px(7.))
                        .child(ui::screens::terminal::pill(
                            theme.button_primary_solid,
                            Some(theme.border),
                        ))
                        .child(text("Open new terminal", 13., 18., 430., foreground)),
                );
            panel = panel.child(ui::screens::terminal::workspace_warning(
                actions,
                self.strip_width < 608.,
                cx,
            ));
        }
        let cloud = self.cloud_view();
        if let Some(None) = cloud {
            return panel.child(
                notice(
                    "Waiting for environment…",
                    Some("The terminal will start when the task has an environment"),
                    &theme,
                )
                .flex_1()
                .justify_center(),
            );
        }
        if cloud.is_none() && session.error.is_some() {
            return panel.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .child(notice(
                        "The terminal encountered an error",
                        Some("Try reloading the terminal to continue"),
                        &theme,
                    ))
                    .child(
                        control("reload-terminal", "", PanelAction::Reload, window, cx)
                            .aria_label("Reload")
                            .debug_selector(|| "terminal-reload".into())
                            .relative()
                            .h(px(24.))
                            .px(px(9.))
                            .child(ui::screens::terminal::pill(theme.text_alpha_5, None))
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
        let status = match cloud {
            Some(Some((overlay, status))) => {
                if overlay {
                    body = body.child(
                        notice("Opening terminal…", None, &theme)
                            .absolute()
                            .inset_0()
                            .justify_center()
                            .bg(theme.surface),
                    );
                }
                status
            }
            _ => session.exited.then_some(CloudStatus::ShellExited),
        };
        panel = panel.child(body);
        if let Some(status) = status {
            let action = match status {
                CloudStatus::EnvironmentChanged => Some(("New terminal", PanelAction::New)),
                CloudStatus::ShellExited | CloudStatus::ConnectionFailed if cloud.is_some() => {
                    Some(("Retry", PanelAction::Retry))
                }
                _ => None,
            };
            let mut row = div()
                .id("terminal-status")
                .role(gpui::Role::Status)
                .h(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .child(text(status.label(), 12., 24., 400., theme.text_secondary));
            if let Some((label, action)) = action {
                row = row.child(
                    control("terminal-status-action", label, action, window, cx)
                        .aria_label(label)
                        .debug_selector(|| "terminal-status-action".into())
                        .h(px(20.))
                        .px(px(6.)),
                );
            }
            panel = panel.child(row);
        }
        panel
    }
}

/// A centered title over an optional description, as the terminal's empty states show them.
fn notice(
    title: &'static str,
    description: Option<&'static str>,
    theme: &theme::Theme,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .child(text(title, 16., 24., 500., theme.text))
        .when_some(description, |d, description| {
            d.child(text(description, 13., 18.5714, 430., theme.text_secondary))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, BufReader, LineWriter};

    /// A durable app-server on a pipe: `serve` gets each request the bridge writes and the
    /// writer for the server's lines.
    fn durable(
        serve: impl FnOnce(&mut dyn FnMut() -> Value, &mut dyn FnMut(Value)) + Send + 'static,
    ) -> Arc<bridge::Bridge> {
        let (client_input, server_output) = std::io::pipe().unwrap();
        let (server_input, client_output) = std::io::pipe().unwrap();
        let (bridge, _) = bridge::Bridge::from_io(client_input, client_output, None);
        std::thread::spawn(move || {
            let mut lines = BufReader::new(server_input).lines();
            let mut output = LineWriter::new(server_output);
            serve(
                &mut || serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap(),
                &mut |line| writeln!(output, "{line}").unwrap(),
            );
        });
        Arc::new(bridge)
    }

    /// Runs a cloud shell's `command/exec` until it ends; returns what the panel received.
    fn run(bridge: Arc<bridge::Bridge>) -> (Vec<Output>, bool) {
        let events = bridge.subscribe_async();
        let (output, receive) = async_channel::unbounded();
        let (ready, started) = async_channel::bounded(1);
        let done = Arc::new(AtomicBool::new(false));
        futures_lite::future::block_on(run_cloud_shell(
            bridge,
            json!({"environmentId": "env-1", "processId": "process-1"}),
            events,
            output,
            ready,
            done.clone(),
        ));
        assert!(
            started.is_closed(),
            "input stays held after the shell ended"
        );
        (
            std::iter::from_fn(|| receive.try_recv().ok()).collect(),
            done.load(Ordering::Acquire),
        )
    }

    fn delta(bytes: &[u8], cap: bool) -> Value {
        json!({"method": "command/exec/outputDelta", "params": {
            "processId": "process-1", "stream": "stdout",
            "deltaBase64": BASE64.encode(bytes), "capReached": cap,
        }})
    }

    #[test]
    fn cloud_cwd_is_the_environment_directory() {
        let bridge = durable(|read, write| {
            let info = read();
            assert_eq!(info["method"], "environment/info");
            assert_eq!(info["params"], json!({"environmentId": "env-1"}));
            write(json!({"id": info["id"], "result": {"cwd": "file:///workspace/my%20repo"}}));
            let metadata = read();
            assert_eq!(metadata["method"], "fs/getMetadata");
            assert_eq!(
                metadata["params"],
                json!({"environmentId": "env-1", "path": "/workspace/my repo"})
            );
            write(json!({"id": metadata["id"], "result": {"isDirectory": true}}));
            let info = read();
            write(json!({"id": info["id"], "result": {"cwd": "/gone"}}));
            let metadata = read();
            write(json!({"id": metadata["id"], "error": {
                "code": -32000, "message": "No such file or directory (os error 2)",
            }}));
        });
        let cwd = |bridge| futures_lite::future::block_on(environment_cwd(bridge, "env-1"));
        assert_eq!(cwd(&bridge).unwrap().as_deref(), Some("/workspace/my repo"));
        assert_eq!(cwd(&bridge).unwrap(), None);
    }

    #[test]
    fn cloud_shell_streams_output_until_it_exits() {
        let bridge = durable(|read, write| {
            let exec = read();
            assert_eq!(exec["method"], "command/exec");
            // Another process's output on the same connection is not this shell's.
            let mut other = delta(b"other", false);
            other["params"]["processId"] = "process-2".into();
            write(other);
            write(delta(b"$ ", false));
            write(json!({"id": exec["id"], "result": {"exitCode": 0, "stdout": "", "stderr": ""}}));
        });
        let (output, done) = run(bridge);
        assert!(done);
        assert!(matches!(&output[..], [Output::Bytes(b), Output::Finished] if b == b"$ "));
    }

    #[test]
    fn cloud_shell_reports_a_lost_connection() {
        let capped = durable(|read, write| {
            assert_eq!(read()["method"], "command/exec");
            // The process keeps running until the panel drops the shell, which terminates it.
            write(delta(b"x", true));
        });
        let (output, done) = run(capped);
        assert!(!done);
        assert!(
            matches!(&output[..], [Output::Bytes(_), Output::Error(error)]
            if error == "Cloud terminal reached the server's 1 MiB output limit")
        );

        let refused = durable(|read, write| {
            let exec = read();
            write(json!({"id": exec["id"], "error": {"code": -32000, "message": "executor gone"}}));
        });
        let (output, done) = run(refused);
        assert!(done);
        assert!(matches!(&output[..], [Output::Error(error)] if error.contains("executor gone")));
    }

    #[test]
    fn cloud_view_follows_the_original_states() {
        let target = CloudTarget {
            bridge: None,
            environment: Some("env-1".into()),
            connected: vec!["env-1".into()],
        };
        let view = |state: &CloudState, target: &CloudTarget, attached, exited| {
            state.view(target, attached, exited)
        };
        let mut state = CloudState::default();
        assert_eq!(view(&state, &CloudTarget::default(), false, false), None);
        assert_eq!(view(&state, &target, false, false), Some((true, None)));
        let offline = CloudTarget {
            connected: Vec::new(),
            ..target.clone()
        };
        assert_eq!(
            view(&state, &offline, false, false),
            Some((true, Some(CloudStatus::Connecting)))
        );
        state.environment = Some("env-1".into());
        assert_eq!(view(&state, &target, true, false), Some((false, None)));
        state.input_error = Some("write failed".into());
        assert_eq!(
            view(&state, &target, true, false),
            Some((false, Some(CloudStatus::InputInterrupted)))
        );
        state.input_error = None;
        state.has_output = true;
        state.error = Some("lost".into());
        state.reconnecting = true;
        assert_eq!(
            view(&state, &target, false, false),
            Some((false, Some(CloudStatus::Reconnecting)))
        );
        state.reconnecting = false;
        assert_eq!(
            view(&state, &target, false, true),
            Some((false, Some(CloudStatus::ConnectionFailed)))
        );
        state.error = None;
        assert_eq!(
            view(&state, &target, false, true),
            Some((false, Some(CloudStatus::ShellExited)))
        );
        let moved = CloudTarget {
            environment: Some("env-2".into()),
            ..target
        };
        assert_eq!(
            view(&state, &moved, true, false),
            Some((false, Some(CloudStatus::EnvironmentChanged)))
        );
    }

    #[test]
    fn environment_notifications_update_the_target() {
        let mut target = CloudTarget::default();
        target.apply("thread/environment/connected", "managed");
        assert_eq!(target.environment, None);
        target.apply("thread/environment/connected", "env-1");
        target.apply("thread/environment/connected", "env-1");
        assert_eq!(target.connected, ["env-1"]);
        target.apply("thread/environment/disconnected", "env-1");
        assert!(target.connected.is_empty());
        assert_eq!(target.environment.as_deref(), Some("env-1"));
    }

    #[test]
    fn reconnect_backs_off_then_gives_up() {
        let start = Instant::now();
        let mut reconnect = Reconnect::default();
        let delays: Vec<_> = std::iter::from_fn(|| reconnect.schedule(start)).collect();
        assert_eq!(delays, Reconnect::DELAYS);
        // A shell that stayed up 30 s earns the attempts back; a shorter one does not.
        reconnect.connected_at = Some(start);
        assert_eq!(reconnect.schedule(start + Duration::from_secs(29)), None);
        reconnect.connected_at = Some(start);
        assert_eq!(
            reconnect.schedule(start + Duration::from_secs(30)),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn only_an_offline_environment_is_asked_again() {
        let rpc = |message: &str| {
            anyhow::Error::new(bridge::RpcError {
                code: -32000,
                message: message.into(),
                data: Value::Null,
            })
        };
        for message in [
            "environment registry connect returned 404 Not Found: environment_offline",
            "environment registry connect returned 409 Conflict: environment_offline: starting",
        ] {
            assert!(environment_offline(&rpc(message)), "{message}");
        }
        for message in [
            "environment registry connect returned 500 Internal: environment_offline",
            "environment registry connect returned 404 Not Found: environment_offline_forever",
        ] {
            assert!(!environment_offline(&rpc(message)), "{message}");
        }
        assert!(!environment_offline(&anyhow::anyhow!(
            "environment registry connect returned 404 Not Found: environment_offline"
        )));
    }

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

    #[test]
    fn environment_actions_use_one_based_index_and_current_platform() {
        let current_platform = if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(windows) {
            "win32"
        } else {
            "linux"
        };
        let other_platform = if current_platform == "darwin" {
            "linux"
        } else {
            "darwin"
        };
        let config = json!({
            "environment": {
                "environment": {
                    "actions": [
                        {"command": "run-current", "platform": current_platform},
                        {"command": "run-other", "platform": other_platform}
                    ]
                }
            }
        });
        assert_eq!(
            super::Root::environment_action_command(&config, 1),
            Some("run-current")
        );
        assert_eq!(super::Root::environment_action_command(&config, 2), None);
        assert_eq!(super::Root::environment_action_command(&config, 0), None);
        assert_eq!(super::Root::environment_action_command(&config, 10), None);
    }
}
