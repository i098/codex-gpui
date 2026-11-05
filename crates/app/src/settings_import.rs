use crate::*;
use bridge::imports::{
    self, DetectedItem, Detection, Histories, History, ItemType, MigrationItem, Policy,
    Preferences, Provider,
};
use gpui::BorrowAppContext as _;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
use ui::screens::settings::imports::{ImportAction, ImportState};

pub fn bootstrap_request(session_id: &str, stable_id: &str, system_version: &str) -> host::Request {
    host::Request::Http(json!({
        "url": "/wham/statsig/bootstrap", "method": "POST",
        "body": json!({
            "app_session_id": session_id, "app_version": "26.1007.30912", "brand_name": "chatgpt",
            "build_flavor": "prod", "locale": "en-US", "stable_id": stable_id,
            "system_name": if cfg!(target_os = "macos") { "macOS" } else { "Linux" },
            "system_version": system_version, "window_type": "electron"
        }).to_string(),
        "headers": {"OAI-Language":"en", "X-OpenAI-Attach-Auth":"1", "X-OpenAI-Attach-Integrity-State":"1", "originator":"Codex Desktop"},
        "retry": false, "returnErrorResponse": true
    }))
}

pub async fn detect(bridge: &Bridge, policy: Policy) -> Result<Vec<DetectedItem>> {
    let mut items = Vec::new();
    let mut failure = None;
    let providers = if policy.cursor {
        &[Provider::ClaudeCode, Provider::Cursor][..]
    } else {
        &[Provider::ClaudeCode][..]
    };
    for &provider in providers {
        let mut params = json!({"includeHome":true});
        if provider == Provider::Cursor {
            params["migrationSource"] = json!("cursor");
        }
        match bridge
            .request::<_, Detection>("externalAgentConfig/detect", params)
            .await
        {
            Ok(detection) => items.extend(detection.items.into_iter().map(|item| DetectedItem {
                item,
                provider_id: provider,
            })),
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    let roots: BTreeSet<_> = items
        .iter()
        .filter(|item| item.item.item_type == ItemType::Sessions)
        .flat_map(|item| {
            item.item
                .details
                .as_ref()
                .and_then(|details| details["sessions"].as_array())
                .into_iter()
                .flatten()
        })
        .filter_map(|session| {
            session["cwd"]
                .as_str()
                .filter(|cwd| !cwd.is_empty())
                .map(str::to_owned)
        })
        .collect();
    if !roots.is_empty() {
        for &provider in providers {
            let mut params = json!({"includeHome":false,"cwds":roots});
            if provider == Provider::Cursor {
                params["migrationSource"] = json!("cursor");
            }
            match bridge
                .request::<_, Detection>("externalAgentConfig/detect", params)
                .await
            {
                Ok(detection) => {
                    for item in detection.items {
                        let item = DetectedItem {
                            item,
                            provider_id: provider,
                        };
                        if !items.contains(&item) {
                            items.push(item);
                        }
                    }
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    items.sort_by_key(|item| (item.provider_id, item.item.item_type));
    Ok(items)
}

pub async fn histories(bridge: &Bridge) -> Result<Vec<History>> {
    let mut histories: Histories = bridge
        .request("externalAgentConfig/import/readHistories", json!({}))
        .await?;
    histories
        .data
        .sort_by_key(|history| std::cmp::Reverse(history.completed_at_ms));
    Ok(histories.data)
}

#[derive(Default)]
pub struct ImportOutcome {
    pub completions: Vec<Value>,
    pub project_roots: BTreeSet<String>,
    pub failure: Option<anyhow::Error>,
}

pub async fn import_items(
    bridge: &Bridge,
    host: &dyn host::Host,
    selected: &[DetectedItem],
    progress: async_channel::Sender<(usize, usize)>,
) -> Result<ImportOutcome> {
    ensure!(!selected.is_empty(), "no import items selected");
    ensure!(
        selected
            .iter()
            .all(|item| item.provider_id != Provider::ClaudeCowork),
        "the selected import provider is unavailable"
    );
    let mut outcome = ImportOutcome::default();
    for provider in [Provider::ClaudeCode, Provider::Cursor] {
        let mut items: Vec<Cow<'_, MigrationItem>> = selected
            .iter()
            .filter(|item| item.provider_id == provider)
            .map(|item| Cow::Borrowed(&item.item))
            .collect();
        if items.is_empty() {
            continue;
        }
        let imported = async {
            let workspaces = if provider == Provider::Cursor {
                prepare_projectless_imports(&mut items, host)?
            } else {
                BTreeMap::new()
            };
            let events = bridge.subscribe();
            let mut params = import_params(&items, provider);
            if provider == Provider::Cursor {
                params["migrationSource"] = json!("cursor");
            }
            let result: Value = bridge.request("externalAgentConfig/import", params).await?;
            let id = result["importId"]
                .as_str()
                .context("import response has no importId")?;
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                let event = events
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .context("external agent import did not complete")?;
                if let Event::Notification { method, params } = event {
                    if method == "externalAgentConfig/import/progress" && params["importId"] == id {
                        progress
                            .try_send(import_progress_counts(&params["itemTypeResults"]))
                            .ok();
                    } else if method == "externalAgentConfig/import/completed"
                        && params["importId"] == id
                    {
                        let roots = imported_project_roots(&params, &items, &workspaces, host)?;
                        return Ok::<_, anyhow::Error>((params, roots));
                    }
                }
            }
        }
        .await;
        match imported {
            Ok((completion, roots)) => {
                outcome.project_roots.extend(roots);
                outcome.completions.push(completion);
            }
            Err(error) => {
                outcome.failure = Some(error);
                break;
            }
        }
    }
    Ok(outcome)
}
fn import_progress_counts(results: &Value) -> (usize, usize) {
    results
        .as_array()
        .into_iter()
        .flatten()
        .fold((0, 0), |(successes, failures), result| {
            (
                successes + result["successes"].as_array().map_or(0, Vec::len),
                failures + result["failures"].as_array().map_or(0, Vec::len),
            )
        })
}

fn register_imported_projects(host: &dyn host::Host, roots: &BTreeSet<String>) -> Result<()> {
    let mut first_error = None;
    for root in roots {
        if let Err(error) = host.add_project(std::path::Path::new(root))
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn import_params(items: &[Cow<'_, MigrationItem>], provider: Provider) -> Value {
    let mut params = json!({"migrationItems": items, "providerId": provider.id(), "source": "app"});
    for item in params["migrationItems"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        for session in item
            .get_mut("details")
            .and_then(|details| details.get_mut("sessions"))
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(session) = session.as_object_mut() {
                session.retain(|field, _| matches!(field.as_str(), "cwd" | "path" | "title"));
            }
        }
    }
    params
}

fn projectless_source(source: &str) -> Option<Cow<'_, str>> {
    let path = if source.contains('\\') {
        Cow::Owned(source.replace('\\', "/"))
    } else {
        Cow::Borrowed(source)
    };
    let prefix = "projects/empty-window/agent-transcripts/";
    let start = if path.starts_with(prefix) {
        prefix.len()
    } else {
        path.find("/projects/empty-window/agent-transcripts/")? + 1 + prefix.len()
    };
    if start == path.len() {
        return None;
    }
    Some(match path {
        Cow::Borrowed(path) => Cow::Borrowed(&path[start..]),
        Cow::Owned(mut path) => {
            path.drain(..start);
            Cow::Owned(path)
        }
    })
}

fn prepare_projectless_imports(
    items: &mut [Cow<'_, MigrationItem>],
    host: &dyn host::Host,
) -> Result<BTreeMap<String, host::Workspace>> {
    let mut workspaces = BTreeMap::new();
    for item in items {
        let has_projectless = item
            .details
            .as_ref()
            .and_then(|details| details["sessions"].as_array())
            .is_some_and(|sessions| {
                sessions
                    .iter()
                    .any(|session| session["workspaceKind"] == "projectless")
            });
        if !has_projectless {
            continue;
        }
        let sessions = item
            .to_mut()
            .details
            .as_mut()
            .and_then(|details| details["sessions"].as_array_mut())
            .context("projectless import has no sessions")?;
        for session in sessions
            .iter_mut()
            .filter(|session| session["workspaceKind"] == "projectless")
        {
            let source = session["path"]
                .as_str()
                .and_then(projectless_source)
                .context("invalid Cursor projectless session path")?
                .into_owned();
            let workspace = host.prepare_projectless(session["title"].as_str().unwrap_or(""))?;
            session["cwd"] = json!(workspace.cwd);
            workspaces.insert(source, workspace);
        }
    }
    Ok(workspaces)
}

fn imported_project_roots(
    completion: &Value,
    items: &[Cow<'_, MigrationItem>],
    workspaces: &BTreeMap<String, host::Workspace>,
    host: &dyn host::Host,
) -> Result<BTreeSet<String>> {
    let sessions: Vec<_> = items
        .iter()
        .flat_map(|item| {
            item.details
                .as_ref()
                .and_then(|details| details["sessions"].as_array())
                .into_iter()
                .flatten()
        })
        .collect();
    let roots: BTreeMap<_, _> = sessions
        .iter()
        .filter(|session| session["workspaceKind"] != "projectless")
        .filter_map(|session| Some((session["path"].as_str()?, session["cwd"].as_str()?)))
        .collect();
    let mut projects = BTreeSet::new();
    let mut imported_sessions = 0;
    for category in completion["itemTypeResults"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for success in category["successes"].as_array().into_iter().flatten() {
            let workspace = success["source"]
                .as_str()
                .and_then(projectless_source)
                .and_then(|source| workspaces.get(source.as_ref()))
                .or_else(|| {
                    workspaces
                        .values()
                        .find(|workspace| success["cwd"] == workspace.cwd)
                });
            if let Some(workspace) = workspace {
                if success["itemType"] == "SESSIONS"
                    && let Some(thread) = success["target"]
                        .as_str()
                        .filter(|thread| !thread.is_empty())
                {
                    host.record_projectless(thread, workspace)?;
                }
            } else if let Some(root) = success["cwd"]
                .as_str()
                .or_else(|| {
                    (category["itemType"] == "SESSIONS")
                        .then(|| {
                            success["source"]
                                .as_str()
                                .and_then(|source| roots.get(source).copied())
                        })
                        .flatten()
                })
                .filter(|root| !root.is_empty())
            {
                projects.insert(root.to_owned());
            }
            if category["itemType"] == "SESSIONS"
                && success["itemType"] == "SESSIONS"
                && success["target"]
                    .as_str()
                    .is_some_and(|thread| !thread.is_empty())
            {
                imported_sessions += 1;
            }
        }
    }
    if imported_sessions == sessions.len() {
        projects.extend(roots.values().map(|root| (*root).to_owned()));
    }
    Ok(projects)
}

async fn imported_mcp_auth(bridge: &Bridge, histories: &[History]) -> Result<Vec<String>> {
    let imported: BTreeSet<_> = histories
        .iter()
        .flat_map(|history| &history.successes)
        .filter(|item| item.item_type == ItemType::McpServerConfig)
        .filter_map(|item| item.target.as_deref())
        .collect();
    if imported.is_empty() {
        return Ok(Vec::new());
    }
    let response: Value = bridge.request("mcpServerStatus/list", json!({})).await?;
    let servers = response["data"]
        .as_array()
        .context("MCP status response has no data")?;
    Ok(servers
        .iter()
        .filter(|server| server["authStatus"] == "notLoggedIn")
        .filter_map(|server| server["name"].as_str())
        .filter(|name| imported.contains(name))
        .map(str::to_owned)
        .collect())
}

fn system_version() -> String {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|version| version.trim().to_owned())
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    {
        String::new()
    }
}

fn saved<T: serde::de::DeserializeOwned + Default>(cx: &App, key: &str) -> Result<T> {
    cx.try_global::<host_settings::Settings>()
        .and_then(|settings| settings.read_all().get(key).cloned())
        .map(|value| serde_json::from_value(value).with_context(|| format!("invalid {key}")))
        .unwrap_or_else(|| Ok(T::default()))
}

fn persist(cx: &mut App, key: &str, value: Value) -> Result<()> {
    ensure!(
        cx.has_global::<host_settings::Settings>(),
        "host settings are unavailable"
    );
    cx.global_mut::<host_settings::Settings>().write(key, value)
}

impl Root {
    pub(crate) fn install_import(&self, cx: &mut Context<Self>) {
        let root = cx.weak_entity();
        let state = (|| -> Result<ImportState> {
            Ok(ImportState {
                sync_enabled: saved(cx, imports::SYNC_ENABLED)?,
                preferences: saved(cx, imports::SYNC_ITEM_TYPES)?,
                sync_state: saved(cx, imports::SYNC_STATE)?,
                ..Default::default()
            })
        })();
        let mut state = state.unwrap_or_else(|error| ImportState {
            error: Some(error.to_string()),
            ..Default::default()
        });
        state.dispatch = Some(Arc::new(move |action, window, cx| {
            root.update(cx, |root, cx| root.import_action(action, window, cx))
                .ok();
        }));
        cx.set_global(state);
        cx.spawn(async move |root, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(imports::SYNC_INTERVAL_MS as u64))
                    .await;
                if root.update(cx, |root, cx| root.sync_imports(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn load_import_policy(&self, cx: &mut Context<Self>) {
        if self.bridge.is_acp() {
            return;
        }
        let request = cx.update_global::<ImportState, _>(|state, _| {
            state.policy_request += 1;
            state.policy_request
        });
        let host = self.host.clone();
        let session = host::device_id();
        let read = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("Import requires an authenticated feature policy")?;
            let response = host::json_body(&host.fetch(&bootstrap_request(
                &session,
                &identity.device_id,
                &system_version(),
            ))?)?;
            let payload = match &response["statsigPayload"] {
                Value::String(payload) => serde_json::from_str(payload)?,
                Value::Object(_) => response["statsigPayload"].clone(),
                _ => anyhow::bail!("feature bootstrap has no statsigPayload"),
            };
            Ok::<_, anyhow::Error>(Policy::from_evaluations(&payload))
        });
        cx.spawn(async move |root, cx| {
            let result = read.await;
            root.update(cx, |root, cx| {
                if cx.global::<ImportState>().policy_request != request {
                    return;
                }
                cx.update_global::<ImportState, _>(|state, _| match result {
                    Ok(policy) => {
                        state.policy = policy;
                    }
                    Err(error) => {
                        state.policy = Policy::default();
                        eprintln!("codex-gpui: import feature policy: {error:#}");
                    }
                });
                root.sync_imports(cx);
            })
            .ok();
        })
        .detach();
    }

    fn refresh_imports(&self, cx: &mut Context<Self>) {
        let state = cx.global::<ImportState>();
        if !state.policy.enabled || state.pending {
            return;
        }
        let policy = state.policy;
        cx.update_global::<ImportState, _>(|state, _| {
            state.pending = true;
            state.error = None;
            state.detected = Load::Loading;
            state.histories = Load::Loading;
        });
        let bridge = self.bridge.clone();
        let read = cx.background_spawn(async move {
            let detected = pollster::block_on(detect(&bridge, policy));
            let history = pollster::block_on(histories(&bridge));
            let mcp_auth = pollster::block_on(imported_mcp_auth(
                &bridge,
                history.as_deref().unwrap_or(&[]),
            ));
            (detected, history, mcp_auth)
        });
        cx.spawn(async move |_, cx| {
            let (detected, history, mcp_auth) = read.await;
            cx.update_global::<ImportState, _>(|state, _| {
                state.pending = false;
                state.detected = match detected {
                    Ok(items) => Load::Ready(Arc::new(items)),
                    Err(error) => Load::Failed(error.to_string()),
                };
                state.histories = match history {
                    Ok(items) => Load::Ready(Arc::new(items)),
                    Err(error) => Load::Failed(error.to_string()),
                };
                state.mcp_auth = match mcp_auth {
                    Ok(names) => Load::Ready(Arc::new(names)),
                    Err(error) => Load::Failed(error.to_string()),
                };
            });
        })
        .detach();
    }

    pub(crate) fn on_import_event(&self, event: &Event, cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        if !cx.has_global::<ImportState>() {
            return;
        }
        if method == "account/updated"
            || (method == "account/login/completed" && params["success"] == true)
        {
            self.load_import_policy(cx);
        }
        if method == "mcpServer/oauth/login/completed"
            && let Some(name) = params["name"].as_str()
        {
            cx.update_global::<ImportState, _>(|state, _| {
                state.auth_urls.remove(name);
            });
            self.refresh_imports(cx);
        }
    }

    fn authenticate_imported_mcp(&self, name: String, cx: &mut Context<Self>) {
        let state = cx.global::<ImportState>();
        if !matches!(&state.mcp_auth, Load::Ready(names) if names.contains(&name)) {
            return;
        }
        if let Some(url) = state.auth_urls.get(&name) {
            if let Some(url) = url {
                cx.open_url(url);
            }
            return;
        }
        cx.update_global::<ImportState, _>(|state, _| {
            state.auth_urls.insert(name.clone(), None);
        });
        let bridge = self.bridge.clone();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .request::<_, Value>("mcpServer/oauth/login", json!({"name":name}))
                .await
                .and_then(|response| {
                    if response["authorizationUrl"].as_str().is_none() {
                        return Ok(None);
                    }
                    Ok(Some(
                        crate::settings_integrations::authorization_url(&response)?.to_owned(),
                    ))
                });
            cx.update(|cx| match result {
                Ok(Some(url)) => {
                    cx.open_url(&url);
                    cx.update_global::<ImportState, _>(|state, _| {
                        state.auth_urls.insert(name, Some(url));
                    });
                }
                Ok(None) => {
                    cx.update_global::<ImportState, _>(|state, _| {
                        state.auth_urls.remove(&name);
                    });
                }
                Err(error) => {
                    eprintln!("codex-gpui: imported MCP login: {error:#}");
                    cx.update_global::<ImportState, _>(|state, _| {
                        state.auth_urls.remove(&name);
                        state.error = Some("Failed to start login for imported MCP server".into());
                    });
                }
            });
        })
        .detach();
    }

    fn import_action(&mut self, action: ImportAction, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.global::<ImportState>().policy.enabled {
            return;
        }
        match action {
            ImportAction::Refresh => self.refresh_imports(cx),
            ImportAction::Import { items, selection } => {
                self.start_import(items, selection, false, cx)
            }
            ImportAction::AuthenticateMcp(name) => self.authenticate_imported_mcp(name, cx),
            ImportAction::SetSync(enabled) => {
                if !cx.global::<ImportState>().policy.sync {
                    return;
                }
                let result = persist(cx, imports::SYNC_ENABLED, json!(enabled));
                cx.update_global::<ImportState, _>(|state, _| match result {
                    Ok(()) => {
                        state.sync_enabled = enabled;
                        state.error = None;
                    }
                    Err(_) => {
                        state.error = Some("Couldn’t change automatic sync. Try again".into())
                    }
                });
                if enabled {
                    self.sync_imports(cx);
                }
            }
            ImportAction::SavePreferences(preferences) => {
                self.save_import_preferences(preferences, cx)
            }
            ImportAction::OpenItem(category, target) => {
                self.open_imported_item(category, &target, window, cx)
            }
        }
    }

    fn save_import_preferences(&self, preferences: Preferences, cx: &mut Context<Self>) {
        let state = cx.global::<ImportState>();
        if !state.policy.sync
            || !state.sync_state.connected()
            || !ItemType::ALL
                .iter()
                .any(|category| preferences.category_selected(*category, &state.sync_state))
        {
            return;
        }
        let result = persist(cx, imports::SYNC_ITEM_TYPES, json!(preferences));
        cx.update_global::<ImportState, _>(|state, _| match result {
            Ok(()) => {
                state.preferences = preferences;
                state.preferences_saved += 1;
                state.error = None;
            }
            Err(_) => state.error = Some("Couldn’t save sync preferences. Try again".into()),
        });
    }

    fn open_imported_item(
        &mut self,
        category: ItemType,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match category {
            ItemType::Sessions => {
                if let Some(id) = imported_thread_id(target) {
                    self.navigate(Route::Thread(id), window, cx);
                }
            }
            ItemType::Hooks => self.open_settings_section("Hooks", window, cx),
            ItemType::McpServerConfig => self.open_settings_section("MCP servers", window, cx),
            ItemType::Skills | ItemType::Commands | ItemType::Plugins => {
                self.open_settings_section("Plugins", window, cx)
            }
            ItemType::AgentsMd | ItemType::Config | ItemType::Subagents | ItemType::Memory => {}
        }
    }

    fn sync_imports(&self, cx: &mut Context<Self>) {
        let state = cx.global::<ImportState>();
        if !state.policy.enabled
            || !state.policy.sync
            || !state.sync_enabled
            || !state.sync_state.connected()
            || state.pending
            || state.reviewing
        {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        if state.sync_state.last_sync_started_at_ms.is_none() {
            let mut sync = state.sync_state.clone();
            sync.last_sync_started_at_ms = Some(now);
            if persist(cx, imports::SYNC_STATE, json!(sync)).is_ok() {
                cx.update_global::<ImportState, _>(|state, _| state.sync_state = sync);
            }
            return;
        }
        if !state.sync_state.due(now) {
            return;
        }
        let policy = state.policy;
        let sync = state.sync_state.clone();
        let preferences = state.preferences.clone();
        cx.update_global::<ImportState, _>(|state, _| state.pending = true);
        let bridge = self.bridge.clone();
        let read = cx.background_spawn(async move { pollster::block_on(detect(&bridge, policy)) });
        cx.spawn(async move |root, cx| {
            let result = read.await;
            root.update(cx, |root, cx| {
                cx.update_global::<ImportState, _>(|state, _| state.pending = false);
                match result {
                    Ok(items) => {
                        let selected = items
                            .iter()
                            .filter(|item| preferences.allows(item, &sync))
                            .cloned()
                            .collect::<Vec<_>>();
                        cx.update_global::<ImportState, _>(|state, _| {
                            state.detected = Load::Ready(Arc::new(items))
                        });
                        if selected.is_empty() {
                            root.finish_import_sync(cx);
                        } else {
                            root.start_import(
                                selected,
                                std::collections::BTreeMap::new(),
                                true,
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: import sync: {error:#}");
                        root.toast(Toast::danger("Unable to sync imports"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_import_sync(&self, cx: &mut Context<Self>) {
        let mut sync = cx.global::<ImportState>().sync_state.clone();
        sync.last_sync_started_at_ms = Some(chrono::Utc::now().timestamp_millis());
        if persist(cx, imports::SYNC_STATE, json!(sync)).is_ok() {
            cx.update_global::<ImportState, _>(|state, _| state.sync_state = sync);
        }
    }

    fn can_import(state: &ImportState, items: &[DetectedItem], automatic: bool) -> bool {
        if !state.policy.enabled
            || state.pending
            || items.is_empty()
            || (automatic && (!state.policy.sync || !state.sync_enabled))
        {
            return false;
        }
        let Load::Ready(detected) = &state.detected else {
            return false;
        };
        items.iter().all(|item| {
            detected.contains(item)
                && item.provider_id != Provider::ClaudeCowork
                && (item.provider_id != Provider::Cursor || state.policy.cursor)
        })
    }

    fn start_import(
        &self,
        items: Vec<DetectedItem>,
        selection: std::collections::BTreeMap<String, bool>,
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        let state = cx.global::<ImportState>();
        if !Self::can_import(state, &items, automatic) {
            return;
        }
        let mut sync = state.sync_state.clone();
        let preferences = if !sync.connected()
            && !selection.is_empty()
            && selection.values().all(|checked| *checked)
        {
            Preferences::All
        } else {
            state.preferences.clone()
        };
        if !automatic {
            sync.merge_import(&items, selection, chrono::Utc::now().timestamp_millis());
        }
        let persisted = if cx.has_global::<host_settings::Settings>() {
            cx.global_mut::<host_settings::Settings>().write_batch([
                (imports::SYNC_STATE, json!(sync)),
                (imports::SYNC_ITEM_TYPES, json!(preferences)),
            ])
        } else {
            Err(anyhow::anyhow!("host settings are unavailable"))
        };
        if let Err(error) = persisted {
            cx.update_global::<ImportState, _>(|state, _| {
                state.error = Some(format!("Couldn’t save import preferences: {error}"))
            });
            return;
        }
        cx.update_global::<ImportState, _>(|state, _| {
            state.sync_state = sync;
            state.preferences = preferences;
            state.pending = true;
            state.error = None;
            state.running = Some(Arc::new(items.clone()));
            state.progress = None;
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let (progress_sender, progress_receiver) = async_channel::unbounded();
        let run = cx.background_spawn(async move {
            let result = pollster::block_on(import_items(
                &bridge,
                host.as_ref(),
                &items,
                progress_sender,
            ));
            let projects = result.as_ref().map_or_else(
                |_| Ok(()),
                |outcome| register_imported_projects(host.as_ref(), &outcome.project_roots),
            );
            (result, projects)
        });
        let progress_task = cx.spawn(async move |root, cx| {
            while let Ok(progress) = progress_receiver.recv().await {
                root.update(cx, |_, cx| {
                    cx.update_global::<ImportState, _>(|state, _| state.progress = Some(progress));
                    cx.notify();
                })
                .ok();
            }
        });
        cx.spawn(async move |root, cx| {
            let outcome = run.await;
            progress_task.await;
            root.update(cx, |root, cx| root.finish_import(outcome, automatic, cx))
                .ok();
        })
        .detach();
    }

    fn finish_import(
        &mut self,
        (result, projects): (Result<ImportOutcome>, Result<()>),
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        let failed = result.as_ref().map(|outcome| {
            outcome.failure.is_some()
                || outcome.completions.iter().any(|completion| {
                    completion["itemTypeResults"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|category| {
                            category["failures"]
                                .as_array()
                                .is_some_and(|failures| !failures.is_empty())
                        })
                })
        });
        let succeeded = matches!(failed, Ok(false));
        cx.update_global::<ImportState, _>(|state, _| {
            state.pending = false;
            state.running = None;
            state.progress = None;
            state.error = if succeeded {
                None
            } else {
                Some(
                    "Some config could not be imported. Check the selected items and try again"
                        .into(),
                )
            };
            if succeeded {
                state.import_completed += 1;
            }
        });
        if let Some(error) = result
            .as_ref()
            .ok()
            .and_then(|outcome| outcome.failure.as_ref())
        {
            eprintln!("codex-gpui: external import: {error:#}");
        }
        if let Err(error) = result {
            eprintln!("codex-gpui: external import: {error:#}");
        }
        if let Err(error) = projects {
            eprintln!("codex-gpui: imported project registration: {error:#}");
            self.toast(Toast::danger("Couldn’t add imported projects"), cx);
        }
        self.refresh_threads(cx);
        self.load_projects(cx);
        if automatic {
            self.finish_import_sync(cx);
            self.toast(
                if succeeded {
                    Toast::success("Imports synced")
                } else {
                    Toast::danger("Unable to sync imports")
                },
                cx,
            );
        }
        self.refresh_imports(cx);
    }
}

fn imported_thread_id(target: &str) -> Option<String> {
    let id = target
        .strip_prefix("codex://thread/")
        .or_else(|| target.strip_prefix("thread:"))
        .unwrap_or(target);
    (!id.is_empty() && !id.contains(['/', '?', '#'])).then(|| id.to_owned())
}

#[cfg(test)]
mod imported_project_tests {
    use super::*;
    #[test]
    fn import_progress_counts_all_provider_results() {
        use serde_json::json;

        assert_eq!(
            import_progress_counts(&json!([
                {"successes": [{}, {}], "failures": [{}]},
                {"successes": [{}], "failures": []}
            ])),
            (3, 1)
        );
        assert_eq!(import_progress_counts(&json!([])), (0, 0));
    }

    #[test]
    fn import_items_reports_progress_before_completion() -> Result<()> {
        use serde_json::json;
        use std::io::{BufRead as _, BufReader, BufWriter, Write as _};

        let (client_input, server_output) = std::io::pipe()?;
        let (server_input, client_output) = std::io::pipe()?;
        let (bridge, _) = Bridge::from_io(client_input, client_output, None);
        let server = std::thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(server_input)
                .read_line(&mut request)
                .expect("read import request");
            let request: Value = serde_json::from_str(&request).expect("parse import request");
            let mut output = BufWriter::new(server_output);
            let results = json!([{
                "itemType": "CONFIG",
                "successes": [{
                    "itemType": "CONFIG",
                    "cwd": null,
                    "source": "settings.json",
                    "target": "config/settings.json",
                    "title": null
                }],
                "failures": [{
                    "itemType": "CONFIG",
                    "errorType": null,
                    "subErrorType": null,
                    "failureStage": "import",
                    "message": "fixture failure",
                    "cwd": null,
                    "source": "hooks.json"
                }]
            }]);
            writeln!(
                output,
                "{}",
                json!({"id": request["id"], "result": {"importId": "fixture-import"}})
            )
            .expect("write import response");
            for method in [
                "externalAgentConfig/import/progress",
                "externalAgentConfig/import/completed",
            ] {
                writeln!(
                    output,
                    "{}",
                    json!({"method": method, "params": {
                        "importId": "fixture-import",
                        "itemTypeResults": results
                    }})
                )
                .expect("write import notification");
            }
            output.flush().expect("flush import notifications");
        });
        let directory = tempfile::tempdir()?;
        let host = host::LocalHost::new(directory.path().join("codex"));
        let item = DetectedItem {
            item: MigrationItem {
                item_type: ItemType::Config,
                description: "Settings".into(),
                cwd: None,
                details: None,
            },
            provider_id: Provider::ClaudeCode,
        };
        let (progress_sender, progress_receiver) = async_channel::unbounded();

        let outcome = pollster::block_on(import_items(
            &bridge,
            &host,
            std::slice::from_ref(&item),
            progress_sender,
        ))?;

        assert_eq!(progress_receiver.try_recv()?, (1, 1));
        assert_eq!(outcome.completions.len(), 1);
        assert!(outcome.failure.is_none());
        server.join().expect("import server");
        Ok(())
    }
    #[test]
    fn registration_continues_after_an_invalid_root() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let invalid = directory.path().join("a-missing");
        let valid = directory.path().join("b-valid");
        std::fs::create_dir_all(&valid)?;
        let roots = BTreeSet::from([
            invalid.to_string_lossy().into_owned(),
            valid.to_string_lossy().into_owned(),
        ]);
        let host = host::LocalHost::new(directory.path().join("codex"));

        let error = register_imported_projects(&host, &roots).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("resolve selected project folder")
        );
        let state: Value = serde_json::from_slice(&std::fs::read(
            directory.path().join("codex/.codex-global-state.json"),
        )?)?;
        let valid = valid.canonicalize()?.to_string_lossy().into_owned();
        assert!(
            state["local-projects"]
                .as_object()
                .is_some_and(|projects| projects.values().any(|project| {
                    project["rootPaths"].as_array().is_some_and(|roots| {
                        roots
                            .iter()
                            .any(|root| root.as_str() == Some(valid.as_str()))
                    })
                }))
        );
        Ok(())
    }
}
