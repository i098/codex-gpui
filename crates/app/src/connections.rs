use crate::{Root, Route, host, host_settings};
use anyhow::{Context as _, Result, ensure};
use bridge::{Bridge, Event};
use gpui::{App, AppContext as _, BorrowAppContext as _, Context};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    process::Child,
    sync::{Arc, Mutex},
    time::Duration,
};
use ui::screens::{
    connections::{
        self as view, Action, Client, Connection, ConnectionsState, Pairing, RemoteStatus,
        SshDraft, Status,
    },
    mobile,
};

struct Peer {
    bridge: Arc<Bridge>,
    version: String,
}

#[derive(Default)]
struct Runtime {
    peers: Mutex<HashMap<String, Peer>>,
    awake: Mutex<Option<Child>>,
    wsl: Mutex<Vec<Connection>>,
}

struct Registry(Arc<Runtime>);
impl gpui::Global for Registry {}

pub fn bridge(host_id: &str, cx: &App) -> Option<Arc<Bridge>> {
    cx.try_global::<Registry>()?
        .0
        .peers
        .lock()
        .ok()?
        .get(host_id)
        .filter(|peer| !peer.bridge.is_closed())
        .map(|peer| peer.bridge.clone())
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Ok(awake) = self.awake.get_mut()
            && let Some(mut child) = awake.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Every window calls this; the runtime and page state are app-wide, so later windows only
/// re-point `dispatch` (the last window wins, like `ComputerUseState`).
pub(super) fn install(cx: &mut Context<Root>) {
    let runtime = match cx.try_global::<Registry>() {
        Some(registry) => registry.0.clone(),
        None => {
            let runtime = Arc::new(Runtime::default());
            cx.set_global(Registry(runtime.clone()));
            runtime
        }
    };
    let weak = cx.weak_entity();
    let dispatch: view::Dispatch = Arc::new(move |action, cx| {
        let runtime = runtime.clone();
        weak.update(cx, |root, cx| handle(root, runtime, action, cx))
            .ok();
    });
    if cx.has_global::<ConnectionsState>() {
        cx.update_global::<ConnectionsState, _>(|state, _| state.dispatch = Some(dispatch));
        return;
    }
    let keep_awake = cx
        .try_global::<host_settings::Settings>()
        .and_then(|s| {
            s.read_all()
                .get("keepRemoteControlAwakeWhilePluggedIn")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    cx.set_global(ConnectionsState {
        keep_awake,
        dispatch: Some(dispatch),
        ..Default::default()
    });
}

pub(super) fn sync(cx: &mut Context<Root>) {
    let native = cx
        .try_global::<ui::screens::computer_use_native::NativeSettingsState>()
        .map(|s| s.policy.clone())
        .unwrap_or_default();
    cx.update_global::<ConnectionsState, _>(|state, _| {
        state.generation = state.generation.wrapping_add(1);
        state.pending = false;
        state.active_setup = None;
        state.gates.slingshot = native.slingshot_enabled;
        state.gates.available = native.remote_control_available;
        state.gates.access_required = native.remote_control_access_required;
    });
}

pub(super) fn on_event(event: &Event, cx: &mut Context<Root>) {
    if let Event::Notification { method, params } = event
        && method == "remoteControl/status/changed"
    {
        match serde_json::from_value::<RemoteStatus>(params.clone()) {
            Ok(status) => {
                cx.update_global::<ConnectionsState, _>(|state, _| state.status = Some(status));
                if let Some(runtime) = cx
                    .try_global::<Registry>()
                    .map(|registry| registry.0.clone())
                {
                    reconcile_awake(&runtime, cx);
                }
            }
            Err(error) => eprintln!("codex-gpui: invalid remote-control status: {error}"),
        }
    }
}

impl Runtime {
    fn peers(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Peer>>> {
        self.peers
            .lock()
            .map_err(|_| anyhow::anyhow!("Remote connection lock is poisoned"))
    }

    /// Saved, WSL and connected rows; auto-connect runs afterwards as a `ConnectBatch`
    /// so the list shows without waiting on SSH.
    fn refresh_rows(
        &self,
        host: &dyn host::Host,
    ) -> Result<(host::connections::SavedConnections, Option<String>)> {
        let wsl_error = match host::connections::discover_wsl() {
            Ok(rows) => {
                *self
                    .wsl
                    .lock()
                    .map_err(|_| anyhow::anyhow!("WSL state lock is poisoned"))? = rows;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        Ok((self.rows(host)?, wsl_error))
    }

    fn rows(&self, host: &dyn host::Host) -> Result<host::connections::SavedConnections> {
        let mut rows = host.remote_connections()?;
        rows.rows.extend(
            self.wsl
                .lock()
                .map_err(|_| anyhow::anyhow!("WSL state lock is poisoned"))?
                .iter()
                .cloned(),
        );
        let mut peers = self.peers()?;
        peers.retain(|_, peer| !peer.bridge.is_closed());
        for row in &mut rows.rows {
            row.state = if peers.contains_key(&row.host_id) {
                "connected"
            } else {
                "disconnected"
            }
            .into();
            row.app_server_version = peers.get(&row.host_id).map(|peer| peer.version.clone());
        }
        Ok(rows)
    }

    async fn connect(
        &self,
        host: &dyn host::Host,
        id: &str,
        enabled: bool,
        executor: &gpui::BackgroundExecutor,
    ) -> Result<()> {
        if !enabled {
            host.set_remote_auto_connect(id, false)?;
            self.peers()?.remove(id);
            return Ok(());
        }
        let connection = self
            .rows(host)?
            .rows
            .into_iter()
            .find(|connection| connection.host_id == id)
            .context("Remote connection not found")?;
        let mut command = host::connections::ssh_command(&connection)?;
        let log = tempfile::tempfile()?;
        command.stderr(std::process::Stdio::from(log.try_clone()?));
        let (bridge, _) = Bridge::spawn(command)?;
        let result = within(
            async {
                let info = bridge
                    .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                    .await?;
                let account = bridge.account_read().await?;
                ensure!(
                    account.account.is_some(),
                    "Sign in to ChatGPT on the remote device"
                );
                Ok(info)
            },
            executor,
        )
        .await;
        let info = result.map_err(|error| ssh_error(error, &log))?;
        host.set_remote_auto_connect(id, true)?;
        self.peers()?.insert(
            id.into(),
            Peer {
                bridge: Arc::new(bridge),
                version: info.user_agent,
            },
        );
        Ok(())
    }

    async fn logout(
        &self,
        host: &dyn host::Host,
        id: &str,
        executor: &gpui::BackgroundExecutor,
    ) -> Result<()> {
        let bridge = self
            .peers()?
            .get(id)
            .map(|peer| peer.bridge.clone())
            .context("Connect to the remote device first")?;
        within(
            bridge.request::<_, Value>("account/logout", json!({})),
            executor,
        )
        .await?;
        self.peers()?.remove(id);
        host.set_remote_auto_connect(id, false)
    }

    fn set_awake(&self, enabled: bool) -> Result<()> {
        let mut awake = self
            .awake
            .lock()
            .map_err(|_| anyhow::anyhow!("Keep-awake lock is poisoned"))?;
        if let Some(child) = awake.as_mut()
            && child.try_wait()?.is_some()
        {
            *awake = None;
        }
        if !enabled {
            if let Some(mut child) = awake.take() {
                child.kill()?;
                child.wait()?;
            }
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if awake.is_none() {
            *awake = Some(
                std::process::Command::new("/usr/bin/caffeinate")
                    .args(["-s", "-w", &std::process::id().to_string()])
                    .spawn()?,
            );
        }
        #[cfg(not(target_os = "macos"))]
        if enabled {
            anyhow::bail!("Keep-awake is not available on this platform");
        }
        Ok(())
    }
}

async fn within<T>(
    future: impl Future<Output = Result<T>>,
    executor: &gpui::BackgroundExecutor,
) -> Result<T> {
    let mut future = std::pin::pin!(future);
    let mut timer = std::pin::pin!(executor.timer(Duration::from_secs(30)));
    std::future::poll_fn(|cx| {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(cx) {
            return std::task::Poll::Ready(value);
        }
        if timer.as_mut().poll(cx).is_ready() {
            return std::task::Poll::Ready(Err(anyhow::anyhow!("Connection timed out")));
        }
        std::task::Poll::Pending
    })
    .await
}

fn ssh_error(error: anyhow::Error, file: &std::fs::File) -> anyhow::Error {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let detail = (|| {
        let mut file = file.try_clone().ok()?;
        let length = file.metadata().ok()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(4096)))
            .ok()?;
        let mut bytes = [0; 4096];
        let count = file.read(&mut bytes).ok()?;
        String::from_utf8_lossy(&bytes[..count])
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::to_owned)
    })();
    match detail {
        Some(detail) => error.context(detail),
        None => error,
    }
}

fn reconcile_awake(runtime: &Runtime, cx: &mut App) {
    let state = cx.global::<ConnectionsState>();
    let enabled = state.keep_awake
        && state
            .status
            .as_ref()
            .is_some_and(|status| matches!(status.status, Status::Connected | Status::Connecting));
    if let Err(error) = runtime.set_awake(enabled) {
        cx.update_global::<ConnectionsState, _>(|state, _| state.error = Some(error.to_string()));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientsPage {
    data: Vec<Client>,
    next_cursor: Option<String>,
}

async fn clients(bridge: &Bridge, environment: Option<&str>) -> Result<Vec<Client>> {
    let Some(environment) = environment else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    loop {
        let page: ClientsPage = bridge
            .request(
                "remoteControl/client/list",
                json!({"environmentId":environment,"cursor":cursor,"limit":100,"order":"desc"}),
            )
            .await?;
        rows.extend(page.data);
        match page.next_cursor {
            Some(next) => {
                ensure!(
                    seen.insert(next.clone()),
                    "Remote device list repeated its cursor"
                );
                cursor = Some(next);
            }
            None => return Ok(rows),
        }
    }
}

fn environment_path(id: &str) -> Result<String> {
    ensure!(!id.is_empty(), "Remote environment ID is missing");
    let mut url = url::Url::parse("https://chatgpt.com/codex/remote/control/environments/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid remote endpoint"))?
        .pop_if_empty()
        .push(id);
    Ok(url.path().to_owned())
}

fn remote_request(
    host: &dyn host::Host,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = json!({
        "url": path,
        "method": method,
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop"
        },
        "returnErrorResponse": true
    });
    if let Some(body) = body {
        request["body"] = Value::String(body.to_string());
        request["headers"]["Content-Type"] = json!("application/json");
    }
    host.fetch(&host::Request::Http(request))
}

fn remote_environments(host: &dyn host::Host) -> Result<Vec<Connection>> {
    let mut cursor = None::<String>;
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    loop {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("limit", "100");
        if let Some(cursor) = &cursor {
            query.append_pair("cursor", cursor);
        }
        let path = format!("/codex/remote/control/environments?{}", query.finish());
        let response = remote_request(host, "GET", &path, None)?;
        let response = host::json_body(&response)?;
        let items = response["items"]
            .as_array()
            .context("Remote environment response has no items")?;
        for item in items {
            let id = item["env_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .context("Remote environment ID is missing")?;
            let display = item["display_name"]
                .as_str()
                .or(item["name"].as_str())
                .or(item["host_name"].as_str())
                .unwrap_or(id)
                .trim();
            rows.push(Connection {
                host_id: format!(
                    "remote-control:{}",
                    environment_path(id)?.rsplit('/').next().unwrap_or_default()
                ),
                display_name: if display.is_empty() { id } else { display }.to_owned(),
                source: "remote-control".into(),
                environment_id: Some(id.to_owned()),
                online: item["online"]
                    .as_bool()
                    .context("Remote environment online status is missing")?,
                state: "disconnected".into(),
                ..Default::default()
            });
        }
        match response["cursor"].as_str() {
            Some(next) => {
                ensure!(
                    seen.insert(next.to_owned()),
                    "Remote environment list repeated its cursor"
                );
                cursor = Some(next.to_owned());
            }
            None => return Ok(rows),
        }
    }
}
fn handle(root: &mut Root, runtime: Arc<Runtime>, action: Action, cx: &mut Context<Root>) {
    if cx.global::<ConnectionsState>().pending {
        return;
    }
    if let Action::KeepAwake(enabled) = action {
        let previous = cx.global::<ConnectionsState>().keep_awake;
        let active = cx
            .global::<ConnectionsState>()
            .status
            .as_ref()
            .is_some_and(|s| matches!(s.status, Status::Connected | Status::Connecting));
        let result = runtime.set_awake(enabled && active).and_then(|_| {
            cx.global_mut::<host_settings::Settings>()
                .write("keepRemoteControlAwakeWhilePluggedIn", json!(enabled))
        });
        if result.is_err() {
            runtime.set_awake(previous && active).ok();
        }
        cx.update_global::<ConnectionsState, _>(|state, _| match result {
            Ok(()) => {
                state.keep_awake = enabled;
                state.error = None;
            }
            Err(error) => state.error = Some(format!("Could not save keep-awake setting: {error}")),
        });
        return;
    }
    cx.update_global::<ConnectionsState, _>(|state, _| {
        state.pending = true;
        state.error = None;
        if let Action::Connect(id, true) | Action::Restart(id) = &action
            && let Some(row) = state
                .connections
                .as_mut()
                .and_then(|rows| rows.iter_mut().find(|row| row.host_id == *id))
        {
            row.state = "connecting".into();
            row.error = None;
        }
        if let Action::ConnectBatch(ids) = &action
            && let Some(rows) = &mut state.connections
        {
            for row in rows.iter_mut().filter(|row| ids.contains(&row.host_id)) {
                row.state = "connecting".into();
                row.error = None;
            }
        }
    });
    let bridge = root.bridge.clone();
    let generation = cx.global::<ConnectionsState>().generation;
    let mobile = mobile_page(root, cx);
    if matches!(action, Action::SetEnabled(true))
        && let Some(page) = &mobile
    {
        page.update(cx, |page, cx| {
            page.set_step(mobile::SetupStep::Enabling, cx)
        });
    }
    let worker_runtime = runtime.clone();
    let worker_bridge = bridge.clone();
    let worker_action = action.clone();
    let host = root.host.clone();
    let mobile_only = root.mobile_page.is_some();
    let read_control = mobile_only || cx.global::<ConnectionsState>().gates.remote_control();
    let read_devices = !mobile_only
        && read_control
        && cx.global::<ConnectionsState>().requested_tab == view::Tab::Devices;
    let executor = cx.background_executor().clone();
    let work = cx.background_spawn(async move {
        let runtime = worker_runtime;
        let bridge = worker_bridge;
        let action = worker_action;
        let result: Result<Update> = async {
            match &action {
                Action::Refresh => {
                    let (rows, wsl_error) = if mobile_only {
                        (None, None)
                    } else {
                        let (rows, error) = runtime.refresh_rows(host.as_ref())?;
                        (Some(rows), error)
                    };
                    let remote = if read_control {
                        let status = bridge
                            .request::<_, RemoteStatus>("remoteControl/status/read", json!({}))
                            .await;
                        Some(match status {
                            Ok(status) => clients(&bridge, status.environment_id.as_deref())
                                .await
                                .map(|clients| (status, clients)),
                            Err(error) => Err(error),
                        })
                    } else {
                        None
                    };
                    let devices = read_devices.then(|| remote_environments(host.as_ref()));
                    Ok(Update::Refresh(Snapshot {
                        rows,
                        remote,
                        devices,
                        wsl_error,
                    }))
                }
                Action::SetEnabled(enabled) => {
                    let status = within(
                        async {
                            let events = bridge.subscribe_async();
                            let method = if *enabled {
                                "remoteControl/enable"
                            } else {
                                "remoteControl/disable"
                            };
                            let mut status: RemoteStatus =
                                bridge.request(method, json!({})).await?;
                            while *enabled && status.status == Status::Connecting {
                                if let Event::Notification { method, params } = events
                                    .recv()
                                    .await
                                    .context("Remote-control status stream closed")?
                                    && method == "remoteControl/status/changed"
                                {
                                    status = serde_json::from_value(params)?;
                                }
                            }
                            ensure!(
                                !*enabled || status.status == Status::Connected,
                                "Could not enable remote control"
                            );
                            Ok(status)
                        },
                        &executor,
                    )
                    .await?;
                    Ok(Update::Enabled(status))
                }
                Action::StartPairing | Action::RefreshPairing => {
                    let status: RemoteStatus = bridge
                        .request("remoteControl/status/read", json!({}))
                        .await?;
                    ensure!(
                        status.status == Status::Connected,
                        "Enable remote control before pairing a device"
                    );
                    let before = clients(&bridge, status.environment_id.as_deref()).await?;
                    let pairing: Pairing = bridge
                        .request("remoteControl/pairing/start", json!({"manualCode":true}))
                        .await?;
                    Ok(Update::Pairing(pairing, before))
                }
                Action::Revoke(client_id) => {
                    let status: RemoteStatus = bridge
                        .request("remoteControl/status/read", json!({}))
                        .await?;
                    let environment = status
                        .environment_id
                        .as_deref()
                        .context("Remote control has no environment")?;
                    bridge
                        .request::<_, Value>(
                            "remoteControl/client/revoke",
                            json!({"environmentId":environment,"clientId":client_id}),
                        )
                        .await?;
                    Ok(Update::Revoked(client_id.clone()))
                }
                Action::DiscoverSsh => Ok(Update::Aliases(host.discover_remote_ssh()?)),
                Action::Save(draft) => {
                    let connect = runtime
                        .rows(host.as_ref())?
                        .rows
                        .iter()
                        .find(|row| Some(&row.host_id) == draft.host_id.as_ref())
                        .is_none_or(|row| row.auto_connect);
                    let connection = host.save_remote_connection(draft)?;
                    Ok(Update::Saved(
                        runtime.rows(host.as_ref())?.rows,
                        if connect {
                            vec![connection.host_id]
                        } else {
                            Vec::new()
                        },
                    ))
                }
                Action::SaveAliases(aliases) => {
                    let mut ids = Vec::with_capacity(aliases.len());
                    for alias in aliases {
                        let draft = SshDraft {
                            display_name: alias.clone(),
                            hostname: alias.clone(),
                            alias: true,
                            ..Default::default()
                        };
                        ids.push(host.save_remote_connection(&draft)?.host_id);
                    }
                    Ok(Update::Saved(runtime.rows(host.as_ref())?.rows, ids))
                }
                Action::ConnectBatch(ids) => {
                    let mut errors = HashMap::new();
                    // ponytail: Connect batches in order; use bounded concurrency for large host lists.
                    for id in ids {
                        if let Err(error) =
                            runtime.connect(host.as_ref(), id, true, &executor).await
                        {
                            errors.insert(id.as_str(), error.to_string());
                        }
                    }
                    let mut rows = runtime.rows(host.as_ref())?.rows;
                    for row in &mut rows {
                        if let Some(error) = errors.remove(row.host_id.as_str()) {
                            row.state = "error".into();
                            row.error = Some(error);
                        }
                    }
                    Ok(Update::Rows(rows))
                }
                Action::RenameRemote {
                    environment_id,
                    name,
                } => {
                    ensure!(
                        !name.trim().is_empty(),
                        "Remote control display name cannot be empty."
                    );
                    let response = remote_request(
                        host.as_ref(),
                        "PATCH",
                        &environment_path(environment_id)?,
                        Some(json!({"name": name.trim()})),
                    )?;
                    host::json_body(&response)?;
                    Ok(Update::Devices(remote_environments(host.as_ref())?))
                }
                Action::DeleteRemote(connection) => {
                    ensure!(
                        !connection.online,
                        "Only offline remote control environments can be deleted."
                    );
                    let id = connection
                        .environment_id
                        .as_deref()
                        .context("Remote environment ID is missing")?;
                    let response =
                        remote_request(host.as_ref(), "DELETE", &environment_path(id)?, None)?;
                    let response = response.get("value").unwrap_or(&response);
                    let response = response.pointer("/response/__response").unwrap_or(response);
                    ensure!(
                        response["status"]
                            .as_u64()
                            .is_some_and(|status| (200..300).contains(&status)),
                        "Could not delete remote control environment"
                    );
                    Ok(Update::Devices(remote_environments(host.as_ref())?))
                }
                Action::Delete(id) => {
                    host.delete_remote_connection(id)?;
                    runtime.peers()?.remove(id);
                    Ok(Update::Rows(runtime.rows(host.as_ref())?.rows))
                }
                Action::Connect(id, enabled) => {
                    runtime
                        .connect(host.as_ref(), id, *enabled, &executor)
                        .await?;
                    Ok(Update::Rows(runtime.rows(host.as_ref())?.rows))
                }
                Action::Restart(id) => {
                    runtime.peers()?.remove(id);
                    runtime.connect(host.as_ref(), id, true, &executor).await?;
                    Ok(Update::Rows(runtime.rows(host.as_ref())?.rows))
                }
                Action::Logout(id) => {
                    runtime.logout(host.as_ref(), id, &executor).await?;
                    Ok(Update::Rows(runtime.rows(host.as_ref())?.rows))
                }
                Action::KeepAwake(_) => unreachable!(),
            }
        }
        .await;
        result
    });
    let task = cx.spawn(async move |weak, cx| {
        let result = work.await;
        weak.update(cx, |root, cx| {
            if cx.global::<ConnectionsState>().generation != generation {
                return;
            }
            cx.update_global::<ConnectionsState, _>(|state, _| state.pending = false);
            match result {
                Ok(update) => {
                    let mut start_poll = None;
                    let mut start_pairing = false;
                    let mut mobile_step = None;
                    let mut connect_saved = Vec::new();
                    let current_mobile = mobile_page(root, cx);
                    let entering_mobile = current_mobile
                        .as_ref()
                        .is_some_and(|page| page.read(cx).step() == mobile::SetupStep::Initial);
                    cx.update_global::<ConnectionsState, _>(|state, _| match update {
                        Update::Refresh(snapshot) => {
                            if let Some(rows) = snapshot.rows {
                                connect_saved = rows
                                    .rows
                                    .iter()
                                    .filter(|row| row.auto_connect && row.state != "connected")
                                    .map(|row| row.host_id.clone())
                                    .collect();
                                state.setup_complete = rows.setup_complete;
                                state.connections = Some(rows.rows);
                                state.wsl_error = snapshot.wsl_error;
                            }
                            if let Some(devices) = snapshot.devices {
                                match devices {
                                    Ok(devices) => {
                                        state.remote_devices = Some(devices);
                                        state.remote_error = None;
                                    }
                                    Err(error) => state.remote_error = Some(error.to_string()),
                                }
                            }
                            if let Some(remote) = snapshot.remote {
                                match remote {
                                    Ok((status, clients)) => {
                                        if entering_mobile && status.status == Status::Connected {
                                            mobile_step = Some(if clients.is_empty() {
                                                mobile::SetupStep::Waiting
                                            } else {
                                                mobile::SetupStep::Ready
                                            });
                                            start_pairing = true;
                                        }
                                        state.status = Some(status);
                                        state.clients = Some(clients);
                                    }
                                    Err(error) => state.error = Some(error.to_string()),
                                }
                            }
                        }
                        Update::Devices(devices) => {
                            state.remote_devices = Some(devices);
                            state.remote_error = None;
                            state.saved_revision = state.saved_revision.wrapping_add(1);
                        }
                        Update::Enabled(status) => {
                            let enabled = status.status == Status::Connected;
                            state.status = Some(status);
                            state.pairing = None;
                            start_pairing = enabled && current_mobile.is_some();
                        }
                        Update::Pairing(pairing, before) => {
                            state.pairing = Some(pairing.clone());
                            state.pairing_error = None;
                            state.pairing_connected = false;
                            start_poll = Some((
                                pairing,
                                before.into_iter().map(|client| client.client_id).collect(),
                            ));
                        }
                        Update::Revoked(id) => {
                            if let Some(clients) = &mut state.clients {
                                clients.retain(|c| c.client_id != id);
                            }
                        }
                        Update::Aliases(mut aliases) => {
                            let used: HashSet<_> = state
                                .connections
                                .iter()
                                .flatten()
                                .filter_map(|connection| connection.alias.as_deref())
                                .collect();
                            aliases.retain(|alias| !used.contains(alias.as_str()));
                            state.aliases = aliases;
                            state.aliases_loaded = true;
                        }
                        Update::Rows(rows) => state.connections = Some(rows),
                        Update::Saved(rows, host_id) => {
                            state.connections = Some(rows);
                            state.saved_revision = state.saved_revision.wrapping_add(1);
                            connect_saved = host_id;
                        }
                    });
                    reconcile_awake(&runtime, cx);
                    if !connect_saved.is_empty() {
                        handle(
                            root,
                            runtime.clone(),
                            Action::ConnectBatch(connect_saved),
                            cx,
                        );
                    }
                    if let Some(step) = mobile_step
                        && let Some(page) = &current_mobile
                    {
                        page.update(cx, |page, cx| page.set_step(step, cx));
                    }
                    if start_pairing {
                        if mobile_step.is_none()
                            && let Some(page) = &current_mobile
                        {
                            page.update(cx, |page, cx| {
                                page.set_step(mobile::SetupStep::Waiting, cx)
                            });
                        }
                        handle(root, runtime.clone(), Action::StartPairing, cx);
                    }
                    if let Some((pairing, before)) = start_poll {
                        poll_pairing(
                            bridge.clone(),
                            root.host.clone(),
                            pairing,
                            before,
                            generation,
                            cx,
                        );
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    if matches!(action, Action::StartPairing | Action::RefreshPairing) {
                        cx.update_global::<ConnectionsState, _>(|state, _| {
                            state.pairing_error = Some(message)
                        });
                    } else {
                        cx.update_global::<ConnectionsState, _>(|state, _| {
                            if let Action::Connect(id, _) | Action::Restart(id) = &action
                                && let Some(row) = state
                                    .connections
                                    .as_mut()
                                    .and_then(|rows| rows.iter_mut().find(|row| row.host_id == *id))
                            {
                                row.state = "error".into();
                                row.error = Some(message.clone());
                            }
                            state.error = Some(message)
                        });
                    }
                    if matches!(action, Action::SetEnabled(true))
                        && let Some(page) = mobile_page(root, cx)
                    {
                        page.update(cx, |page, cx| {
                            page.set_step(mobile::SetupStep::AllowError, cx)
                        });
                    }
                }
            }
        })
        .ok();
    });
    task.detach();
}

enum Update {
    Refresh(Snapshot),
    Enabled(RemoteStatus),
    Pairing(Pairing, Vec<Client>),
    Revoked(String),
    Aliases(Vec<String>),
    Rows(Vec<Connection>),
    Saved(Vec<Connection>, Vec<String>),
    Devices(Vec<Connection>),
}

struct Snapshot {
    rows: Option<host::connections::SavedConnections>,
    remote: Option<Result<(RemoteStatus, Vec<Client>)>>,
    devices: Option<Result<Vec<Connection>>>,
    wsl_error: Option<String>,
}

fn poll_pairing(
    bridge: Arc<Bridge>,
    host: Arc<dyn host::Host>,
    pairing: Pairing,
    before: HashSet<String>,
    generation: u64,
    cx: &mut Context<Root>,
) {
    let executor = cx.background_executor().clone();
    cx.spawn(async move |weak, cx| {
        loop {
            executor.timer(Duration::from_secs(1)).await;
            let active = weak.update(cx, |root, cx| {
                let state = cx.global::<ConnectionsState>();
                state.generation == generation
                    && (root.mobile_page.is_some() || root.state.route == Route::Settings)
                    && state.pairing.as_ref().is_some_and(|p| p.pairing_code == pairing.pairing_code)
            }).unwrap_or(false);
            if !active { return; }
            if chrono::Utc::now().timestamp() >= pairing.expires_at {
                weak.update(cx, |_, cx| cx.update_global::<ConnectionsState, _>(|state, _| state.pairing_error = Some("Pairing code expired".into()))).ok();
                return;
            }
            let result: Result<Option<Vec<Client>>> = async {
                let response: Value = bridge.request("remoteControl/pairing/status", json!({"pairingCode":pairing.pairing_code,"manualPairingCode":pairing.manual_pairing_code})).await?;
                if response["claimed"] != true { return Ok(None); }
                let clients = clients(&bridge, Some(&pairing.environment_id)).await?;
                if clients.iter().any(|client| !before.contains(&client.client_id)) {
                    host.record_mobile_setup()?;
                    Ok(Some(clients))
                } else {
                    Ok(None)
                }
            }.await;
            let stop = weak.update(cx, |root, cx| match result {
                Ok(None) => false,
                Ok(Some(clients)) => {
                    cx.update_global::<ConnectionsState, _>(|state, _| { state.clients = Some(clients); state.pairing_connected = true; state.setup_complete = true; });
                    if let Some(page) = mobile_page(root, cx) { page.update(cx, |page, cx| page.set_step(mobile::SetupStep::Connected, cx)); }
                    true
                }
                Err(error) => { cx.update_global::<ConnectionsState, _>(|state, _| state.pairing_error = Some(error.to_string())); true }
            }).unwrap_or(true);
            if stop { return; }
        }
    }).detach();
}

pub(super) fn open_mobile(root: &mut Root, cx: &mut Context<Root>) {
    root.load_computer_use(cx);
    cx.defer(|cx| view::dispatch(Action::Refresh, cx));
}

pub(super) fn mobile_page(root: &Root, cx: &App) -> Option<gpui::Entity<mobile::MobileSetupPage>> {
    root.mobile_page
        .clone()
        .or_else(|| cx.global::<ConnectionsState>().active_setup.clone())
}

pub(super) fn startup(root: &Root, cx: &mut Context<Root>) {
    if root.state.path.is_some() && !root.bridge.is_acp() {
        cx.defer(|cx| view::dispatch(Action::Refresh, cx));
    }
}

impl Root {
    /// Opens Settings > Connections at `tab` (`codex://settings/connections/<tab>`).
    pub fn open_connections(
        &mut self,
        tab: view::Tab,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        cx.update_global::<ConnectionsState, _>(|state, _| state.requested_tab = tab);
        self.open_settings_section("Connections", window, cx);
    }

    pub(crate) fn open_ssh_link(
        &mut self,
        name: String,
        project: Option<String>,
        enabled: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connections(view::Tab::Ssh, window, cx);
        if project.as_ref().is_some_and(|path| !path.starts_with('/')) {
            self.toast(
                crate::Toast::danger("Remote project folder must be an absolute path"),
                cx,
            );
            return;
        }
        let Some(runtime) = cx
            .try_global::<Registry>()
            .map(|registry| registry.0.clone())
        else {
            self.toast(crate::Toast::danger("Connections are unavailable"), cx);
            return;
        };
        let host = self.host.clone();
        let executor = cx.background_executor().clone();
        let epoch = self.navigation_epoch;
        let work = cx.background_spawn(async move {
            let existing = host
                .remote_connections()?
                .rows
                .into_iter()
                .find(|row| row.alias.as_deref() == Some(name.as_str()));
            let connection = match existing {
                Some(connection) => connection,
                None => host.save_remote_connection(&SshDraft {
                    display_name: name.clone(),
                    hostname: name,
                    alias: true,
                    ..Default::default()
                })?,
            };
            if enabled && runtime.peers()?.get(&connection.host_id).is_none() {
                runtime
                    .connect(host.as_ref(), &connection.host_id, true, &executor)
                    .await?;
            }
            Ok::<_, anyhow::Error>((connection.host_id, runtime.rows(host.as_ref())?.rows))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok((id, rows)) => {
                    cx.update_global::<ConnectionsState, _>(|state, _| {
                        state.connections = Some(rows)
                    });
                    if this.navigation_epoch == epoch && (project.is_some() || enabled) {
                        this.wait_for_link_host(id, project, window, cx);
                    }
                }
                Err(error) => this.toast(crate::Toast::danger(error.to_string()), cx),
            })
            .ok();
        })
        .detach();
    }

    fn wait_for_link_host(
        &mut self,
        id: String,
        project: Option<String>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if bridge(&id, cx).is_some() {
            self.finish_link_host(id, project, window, cx);
            return;
        }
        self.toast(
            crate::Toast::info("Connect to the SSH host to finish adding this project"),
            cx,
        );
        let mut pending = Some((id, project));
        let epoch = self.navigation_epoch;
        self.pending_link_connection =
            Some(cx.observe_global::<ConnectionsState>(move |this, cx| {
                if this.navigation_epoch != epoch {
                    this.pending_link_connection = None;
                    return;
                }
                if pending
                    .as_ref()
                    .is_none_or(|(id, _)| bridge(id, cx).is_none())
                {
                    return;
                }
                let (id, project) = pending.take().expect("pending connection");
                this.pending_link_connection = None;
                let window = this.context_window;
                cx.spawn(async move |this, cx| {
                    cx.update_window(window, |_, window, cx| {
                        this.update(cx, |this, cx| {
                            this.finish_link_host(id, project, window, cx)
                        })
                    })
                    .ok();
                })
                .detach();
            }));
    }

    fn finish_link_host(
        &mut self,
        id: String,
        project: Option<String>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bridge) = bridge(&id, cx) else {
            return;
        };
        let epoch = self.navigation_epoch;
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                if let Some(path) = &project {
                    let metadata = bridge
                        .request::<_, Value>("fs/getMetadata", json!({"path": path}))
                        .await?;
                    anyhow::ensure!(
                        metadata["isDirectory"] == true,
                        "Remote project path is not a directory"
                    );
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                if this.navigation_epoch != epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        if let Some(handle) =
                            this.open_link_host(&id, &crate::deep_link::Link::Launch, window, cx)
                        {
                            let _ = handle.update(cx, |root, window, cx| {
                                if let Some(project) = project {
                                    root.select_project(project, window, cx);
                                } else {
                                    root.new_chat(window, cx);
                                }
                            });
                        }
                    }
                    Err(error) => this.toast(crate::Toast::danger(error.to_string()), cx),
                }
            })
            .ok();
        })
        .detach();
    }
}
