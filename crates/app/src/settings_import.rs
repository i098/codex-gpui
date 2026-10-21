use crate::*;
use bridge::imports::{
    self, DetectedItem, Detection, Histories, History, ItemType, MigrationItem, Policy,
    Preferences, Provider,
};
use gpui::BorrowAppContext as _;
use std::{collections::BTreeSet, time::Instant};
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
            Err(error) => failure = Some(error),
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
            if let Ok(detection) = bridge
                .request::<_, Detection>("externalAgentConfig/detect", params)
                .await
            {
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
        }
    }
    if items.is_empty() {
        if let Some(error) = failure {
            return Err(error);
        }
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

pub async fn import_items(bridge: &Bridge, selected: &[DetectedItem]) -> Result<Vec<Value>> {
    ensure!(!selected.is_empty(), "no import items selected");
    let mut completions = Vec::new();
    for provider in [Provider::ClaudeCode, Provider::Cursor] {
        let items: Vec<&MigrationItem> = selected
            .iter()
            .filter(|item| item.provider_id == provider)
            .map(|item| &item.item)
            .collect();
        if items.is_empty() {
            continue;
        }
        let events = bridge.subscribe();
        let mut params = json!({"migrationItems":items,"providerId":provider.id(),"source":"app"});
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
                if method == "externalAgentConfig/import/completed" && params["importId"] == id {
                    completions.push(params);
                    break;
                }
            }
        }
    }
    ensure!(
        !completions.is_empty(),
        "the selected import provider is unavailable"
    );
    Ok(completions)
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
        .and_then(|settings| settings.read_all().get(key))
        .map(|value| {
            serde_json::from_value(value.clone()).with_context(|| format!("invalid {key}"))
        })
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
            state.policy = Policy::default();
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
                        state.policy_error = None;
                    }
                    Err(error) => {
                        state.policy = Policy::default();
                        state.policy_error = Some(error.to_string());
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
        if method == "mcpServer/oauth/login/completed" {
            if let Some(name) = params["name"].as_str() {
                cx.update_global::<ImportState, _>(|state, _| {
                    state.auth_urls.remove(name);
                });
                self.refresh_imports(cx);
            }
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
                .await;
            cx.update(|cx| match result {
                Ok(response) => {
                    if let Some(url) = response["authorizationUrl"].as_str() {
                        cx.open_url(url);
                        cx.update_global::<ImportState, _>(|state, _| {
                            state.auth_urls.insert(name, Some(url.to_owned()));
                        });
                    } else {
                        cx.update_global::<ImportState, _>(|state, _| {
                            state.auth_urls.remove(&name);
                        });
                    }
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
            ImportAction::Import(items) => self.start_import(items, false, cx),
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
                    Err(_) => {
                        state.error = Some("Couldn’t save sync preferences. Try again".into())
                    }
                });
            }
            ImportAction::OpenItem(category, target) => match category {
                ItemType::Sessions => {
                    if let Some(id) = imported_thread_id(&target) {
                        self.navigate(Route::Thread(id), window, cx);
                    }
                }
                ItemType::Hooks => self.open_settings_section("Hooks", window, cx),
                ItemType::McpServerConfig => self.open_settings_section("Plugins", window, cx),
                ItemType::Skills | ItemType::Commands | ItemType::Plugins => {
                    self.open_settings_section("Plugins", window, cx)
                }
                _ => {
                    if let Ok(url) = url::Url::from_file_path(&target) {
                        cx.open_url(url.as_str());
                    }
                }
            },
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
                            root.start_import(selected, true, cx);
                        }
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: import sync: {error:#}");
                        root.finish_import_sync(cx);
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

    fn start_import(&self, items: Vec<DetectedItem>, automatic: bool, cx: &mut Context<Self>) {
        let state = cx.global::<ImportState>();
        if !state.policy.enabled
            || state.pending
            || items.is_empty()
            || (automatic && (!state.policy.sync || !state.sync_enabled))
        {
            return;
        }
        let Load::Ready(detected) = &state.detected else {
            return;
        };
        if items.iter().any(|item| {
            !detected.contains(item)
                || item.provider_id == Provider::ClaudeCowork
                || (item.provider_id == Provider::Cursor && !state.policy.cursor)
        }) {
            return;
        }
        let mut sync = state.sync_state.clone();
        let preferences = if !sync.connected()
            && detected
                .iter()
                .filter(|item| {
                    items
                        .iter()
                        .any(|selected| selected.provider_id == item.provider_id)
                })
                .all(|item| items.contains(item))
        {
            Preferences::All
        } else {
            state.preferences.clone()
        };
        if !automatic {
            sync.merge_import(&items, chrono::Utc::now().timestamp_millis());
            for item in detected.iter().filter(|item| {
                !items.contains(item)
                    && items
                        .iter()
                        .any(|selected| selected.provider_id == item.provider_id)
            }) {
                let key = if item.item.item_type == ItemType::Sessions {
                    "chats".into()
                } else if item.project().is_some() {
                    format!("projects:{}", item.key())
                } else {
                    item.key()
                };
                sync.selection.insert(key, false);
            }
        }
        if let Err(error) = persist(cx, imports::SYNC_STATE, json!(sync)) {
            cx.update_global::<ImportState, _>(|state, _| {
                state.error = Some(format!("Couldn’t save import preferences: {error}"))
            });
            return;
        }
        if let Err(error) = persist(cx, imports::SYNC_ITEM_TYPES, json!(preferences)) {
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
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let run = cx.background_spawn(async move {
            let result = pollster::block_on(import_items(&bridge, &items));
            let projects: Result<Vec<String>> = if result.is_ok() {
                items
                    .iter()
                    .filter_map(|item| item.project())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|root| host.add_project(std::path::Path::new(root)))
                    .collect()
            } else {
                Ok(Vec::new())
            };
            let history = pollster::block_on(histories(&bridge));
            (result, history, projects)
        });
        cx.spawn(async move |root, cx| {
            let (result, history, projects) = run.await;
            root.update(cx, |root, cx| {
                let failed = result.as_ref().map(|completions| completions.iter().any(|completion|
                    completion["itemTypeResults"].as_array().into_iter().flatten().any(|category|
                        category["failures"].as_array().is_some_and(|failures| !failures.is_empty()))));
                let succeeded = matches!(failed, Ok(false));
                cx.update_global::<ImportState, _>(|state, _| {
                    state.pending = false;
                    state.running = None;
                    state.histories = match history { Ok(history) => Load::Ready(Arc::new(history)), Err(error) => Load::Failed(error.to_string()) };
                    state.error = if succeeded { None } else { Some("Some config could not be imported. Check the selected items and try again".into()) };
                    if succeeded { state.import_completed += 1; }
                });
                if let Err(error) = result { eprintln!("codex-gpui: external import: {error:#}"); }
                if let Err(error) = projects {
                    eprintln!("codex-gpui: imported project registration: {error:#}");
                    root.toast(Toast::danger("Couldn’t add imported projects"), cx);
                }
                root.refresh_threads(cx);
                root.load_projects(cx);
                if automatic {
                    root.finish_import_sync(cx);
                    root.toast(if succeeded { Toast::success("Imports synced") } else { Toast::danger("Unable to sync imports") }, cx);
                }
                root.refresh_imports(cx);
            }).ok();
        }).detach();
    }
}

fn imported_thread_id(target: &str) -> Option<String> {
    let id = target
        .strip_prefix("codex://thread/")
        .or_else(|| target.strip_prefix("thread:"))
        .unwrap_or(target);
    (!id.is_empty() && !id.contains(['/', '?', '#'])).then(|| id.to_owned())
}
