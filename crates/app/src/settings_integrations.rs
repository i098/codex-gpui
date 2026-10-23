use crate::{
    Root,
    host::{self, Host, Request},
};
use anyhow::{Context as _, Result, ensure};
use bridge::{Bridge, Event};
use gpui::{AppContext as _, BorrowAppContext as _, Context};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};
use ui::{
    screens::{
        computer_use::Load,
        settings::integrations::{Action, State},
    },
    toast::Toast,
};

impl Root {
    pub(crate) fn load_settings_integrations(&mut self, cx: &mut Context<Self>) {
        if self.settings_integrations.read(cx).dispatch.is_none() {
            let root = cx.weak_entity();
            self.settings_integrations.update(cx, |state, cx| {
                *state = State {
                    configuration: Arc::new(json!({"config":self.config})),
                    plugin_creator: ui::screens::settings::integrations::plugin_creator_path(
                        &self.skills,
                    ),
                    dispatch: Some(Arc::new(move |action, cx| {
                        root.update(cx, |root, cx| root.settings_integration_action(action, cx))
                            .ok();
                    })),
                    ..Default::default()
                };
                cx.notify();
            });
        }
        self.settings_integration_action(Action::Refresh, cx);
    }

    pub(crate) fn invalidate_settings_integrations(&mut self, cx: &mut Context<Self>) {
        self.settings_integration_task = None;
        self.settings_integrations.update(cx, |state, cx| {
            *state = State {
                dispatch: state.dispatch.take(),
                ..Default::default()
            };
            cx.notify();
        });
    }

    pub(crate) fn open_plugin_creator(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.settings_integrations.read(cx).plugin_creator.clone() else {
            self.toast(
                Toast::danger("The plugin-creator skill is not available"),
                cx,
            );
            return;
        };
        let key = "has-opened-skill-creator-prefill-v1";
        let Some(settings) = cx.try_global::<crate::host_settings::Settings>() else {
            self.toast(Toast::danger("Host settings are unavailable"), cx);
            return;
        };
        let seen = settings
            .read_all()
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !seen
            && let Err(error) = cx
                .global_mut::<crate::host_settings::Settings>()
                .write(key, json!(true))
        {
            self.toast(Toast::danger(error.to_string()), cx);
            return;
        }
        let starter = if seen { "" } else { "help me create a plugin" };
        self.plugin_prefill(&format!("[$plugin-creator]({path}) {starter}"), window, cx);
    }

    fn settings_integration_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if self
            .settings_integrations
            .update(cx, |state, _| defer_if_busy(state, &action))
        {
            return;
        }
        let oauth = match &action {
            Action::Authenticate { name, .. } => Some(name.clone()),
            _ => None,
        };
        if let Some(name) = &oauth {
            if let Some(Some(url)) = self.settings_integrations.read(cx).oauth.get(name) {
                cx.open_url(url);
                return;
            }
            if self.settings_integrations.read(cx).oauth.contains_key(name) {
                return;
            }
        }
        self.settings_integrations.update(cx, |state, cx| {
            state.pending = true;
            state.form_error = None;
            state.operation_error = None;
            if let Some(name) = &oauth {
                state.oauth.insert(name.clone(), None);
            }
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let roots = self.settings_integrations.read(cx).roots.clone();
        let configuration = self.settings_integrations.read(cx).configuration.clone();
        let cached_plugins = match &self.settings_integrations.read(cx).plugins {
            Load::Ready(value) => value.clone(),
            _ => Arc::new(Value::Null),
        };
        let cached_gates = self.settings_integrations.read(cx).gates_loaded.then(|| {
            let state = self.settings_integrations.read(cx);
            (state.evaluations.clone(), state.hidden_plugins.clone())
        });
        let operation = action.clone();
        let read = cx.background_spawn(async move {
            let result = execute(&bridge, host.as_ref(), &operation, &roots, &configuration).await;
            let snapshot = if oauth.is_none() {
                Some(load(&bridge, host.as_ref(), &cached_plugins, cached_gates).await)
            } else {
                None
            };
            (result, snapshot)
        });
        self.settings_integration_task = Some(cx.spawn(async move |this, cx| {
            let (result, snapshot) = read.await;
            this.update(cx, |this, cx| {
                this.finish_settings_integration(action, result, snapshot, cx);
            })
            .ok();
        }));
    }

    fn finish_settings_integration(
        &mut self,
        action: Action,
        result: Result<Option<String>>,
        snapshot: Option<Snapshot>,
        cx: &mut Context<Self>,
    ) {
        if let Some(snapshot) = snapshot {
            if let Ok(configuration) = &snapshot.configuration {
                self.config = configuration["config"].clone();
            }
            self.settings_integrations
                .update(cx, |state, _| snapshot.apply(state));
        }
        let (notice, refresh_requested) = self.settings_integrations.update(cx, |state, cx| {
            state.pending = false;
            let refresh_requested = std::mem::take(&mut state.refresh_requested);
            let notice = match result {
                Ok(url) => {
                    integration_succeeded(state, &action, url, cx);
                    None
                }
                Err(error) => integration_failed(state, &action, error),
            };
            cx.notify();
            (notice, refresh_requested)
        });
        if let Some(notice) = notice {
            self.toast(Toast::danger(notice), cx);
        }
        if matches!(action, Action::ReloadHooks)
            && matches!(&self.settings_integrations.read(cx).hooks, Load::Ready(_))
        {
            self.toast(Toast::info("Refreshed hooks"), cx);
        }
        if refresh_requested {
            self.settings_integration_action(Action::Refresh, cx);
        }
    }

    pub(crate) fn settings_integration_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        if method != "mcpServer/oauth/login/completed" {
            return;
        }
        let Some(name) = params["name"].as_str() else {
            return;
        };
        self.settings_integrations.update(cx, |state, cx| {
            state.oauth.remove(name);
            cx.notify();
        });
        if params["success"] == true {
            self.settings_integration_action(Action::Refresh, cx);
        } else {
            self.toast(
                Toast::danger(format!(
                    "Could not authenticate {name}: {}",
                    params["error"].as_str().unwrap_or("Authentication failed")
                )),
                cx,
            );
        }
    }
}

fn defer_if_busy(state: &mut State, action: &Action) -> bool {
    if !state.pending {
        return false;
    }
    state.refresh_requested |= matches!(action, Action::Refresh);
    true
}

fn integration_succeeded(
    state: &mut State,
    action: &Action,
    url: Option<String>,
    cx: &mut gpui::App,
) {
    if matches!(action, Action::SaveServer { .. } | Action::RemoveServer(_)) {
        state.form_saved += 1;
    }
    match action {
        Action::SetServerEnabled { .. } | Action::SaveServer { .. } | Action::RemoveServer(_) => {
            state.reload_required = true;
        }
        Action::ReloadServers => {
            state.reload_required = false;
            state.suppressed_servers.clear();
        }
        Action::AddMarketplace(_) => state.marketplace_added += 1,
        Action::RemoveMarketplace(name) => state.removal_completed = Some(name.clone()),
        Action::RemovePlugin(plugin) => {
            state.removal_completed = plugin["id"].as_str().map(|id| format!("plugin:{id}"));
        }
        _ => {}
    }
    if let Action::RemoveServer(key) = action {
        state.suppressed_servers.insert(key.clone());
    }
    if let Action::Authenticate { name, .. } = action
        && let Some(url) = url
    {
        state.oauth.insert(name.clone(), Some(url.clone()));
        cx.open_url(&url);
    }
}

fn integration_failed(state: &mut State, action: &Action, error: anyhow::Error) -> Option<String> {
    if let Action::Authenticate { name, .. } = action {
        state.oauth.remove(name);
    }
    if matches!(action, Action::SaveServer { .. } | Action::RemoveServer(_)) {
        state.form_error = Some(error.to_string());
    } else if matches!(
        action,
        Action::TrustHooks(_)
            | Action::SetHookEnabled { .. }
            | Action::RemoveMarketplace(_)
            | Action::AddMarketplace(_)
            | Action::RemovePlugin(_)
    ) {
        state.operation_error = Some(error.to_string());
    } else {
        return Some(error.to_string());
    }
    None
}

struct Snapshot {
    configuration: Result<Value>,
    servers: Result<Value>,
    plugins: Result<Value>,
    marketplaces: Result<Value>,
    hooks: Result<Value>,
    apps: Result<Value>,
    skills: Result<Value>,
    roots: Vec<String>,
    evaluations: Arc<Value>,
    hidden_plugins: Arc<HashSet<String>>,
}

impl Snapshot {
    fn apply(self, state: &mut State) {
        state.config_error = self.configuration.as_ref().err().map(ToString::to_string);
        if let Ok(configuration) = self.configuration {
            state.configuration = Arc::new(configuration);
        }
        state.servers = loaded(self.servers);
        state.plugins = loaded(self.plugins);
        state.marketplaces = loaded(self.marketplaces);
        state.hooks = loaded(self.hooks);
        state.apps = loaded(self.apps);
        if let Ok(skills) = &self.skills
            && !skills.is_null()
        {
            state.plugin_creator = ui::screens::settings::integrations::plugin_creator_path(skills);
        }
        state.skills = loaded(self.skills);
        state.roots = Arc::new(self.roots);
        state.evaluations = self.evaluations;
        state.hidden_plugins = self.hidden_plugins;
        state.gates_loaded = true;
    }
}

fn loaded(result: Result<Value>) -> Load<Arc<Value>> {
    match result {
        Ok(value) => Load::Ready(Arc::new(value)),
        Err(error) => Load::Failed(error.to_string()),
    }
}

async fn load(
    bridge: &Bridge,
    host: &dyn Host,
    cached_plugins: &Value,
    cached_gates: Option<(Arc<Value>, Arc<HashSet<String>>)>,
) -> Snapshot {
    let (evaluations, hidden_plugins) = cached_gates.unwrap_or_else(|| {
        let evaluations = Arc::new(read_bootstrap(host).unwrap_or_default());
        let hidden_plugins =
            evaluations["dynamic_configs"]["1349514884"]["value"]["hidden_plugin_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<HashSet<_>>();
        (evaluations, Arc::new(hidden_plugins))
    });
    let project_roots = host.projects().map(|projects| {
        projects
            .into_iter()
            .filter_map(|project| project.root.map(|root| root.to_string()))
            .collect::<Vec<_>>()
    });
    let roots = project_roots.as_ref().cloned().unwrap_or_default();
    let configuration = bridge
        .request("config/read", json!({"includeLayers":true}))
        .await;
    let servers = paged(
        bridge,
        "mcpServerStatus/list",
        json!({"detail":"toolsAndAuthOnly"}),
    )
    .await;
    let mut plugins: Result<Value> = bridge
        .request("plugin/installed", json!({"cwds":roots}))
        .await;
    if let Ok(value) = &mut plugins {
        load_plugin_icons(value, cached_plugins, &hidden_plugins, host);
    }
    let marketplaces = bridge
        .request(
            "plugin/list",
            json!({"cwds":roots,"marketplaceKinds":["local"]}),
        )
        .await;
    let hooks = match project_roots {
        Ok(_) if roots.is_empty() => Ok(Value::Null),
        Ok(_) => bridge.request("hooks/list", json!({"cwds":roots})).await,
        Err(error) => Err(error),
    };
    let apps = paged(bridge, "app/list", json!({"forceRefetch":false})).await;
    let skills = if roots.is_empty() {
        Ok(Value::Null)
    } else {
        bridge
            .request("skills/list", json!({"cwds":roots,"forceReload":true}))
            .await
    };
    Snapshot {
        configuration,
        servers,
        plugins,
        marketplaces,
        hooks,
        apps,
        skills,
        roots,
        evaluations,
        hidden_plugins,
    }
}

fn load_plugin_icons(
    value: &mut Value,
    cached_plugins: &Value,
    hidden_plugins: &HashSet<String>,
    host: &dyn Host,
) {
    for marketplace in value["marketplaces"].as_array_mut().into_iter().flatten() {
        for plugin in marketplace["plugins"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .filter(|plugin| {
                ui::screens::settings::integrations::visible_plugin(plugin, hidden_plugins)
            })
        {
            let cached = cached_plugins["marketplaces"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|marketplace| marketplace["plugins"].as_array().into_iter().flatten())
                .find(|old| old["id"] == plugin["id"] && old["interface"] == plugin["interface"])
                .and_then(|old| old["iconDataUrl"].as_str());
            if let Some(data) = cached {
                plugin["iconDataUrl"] = data.into();
                continue;
            }
            let url = plugin["interface"]["logoUrlDark"]
                .as_str()
                .or_else(|| plugin["interface"]["logoUrl"].as_str());
            if let Some(url) = url
                && let Ok(data) = load_icon(host, url)
            {
                plugin["iconDataUrl"] = data.into();
            }
        }
    }
}

async fn paged(bridge: &Bridge, method: &str, mut params: Value) -> Result<Value> {
    let mut data = Vec::new();
    let mut seen = HashSet::new();
    params["limit"] = 100.into();
    params["cursor"] = Value::Null;
    loop {
        let page: Value = bridge.request(method, &params).await?;
        data.extend(
            page["data"]
                .as_array()
                .context("Invalid paginated response")?
                .iter()
                .cloned(),
        );
        let Some(next) = page["nextCursor"].as_str() else {
            break;
        };
        ensure!(
            seen.insert(next.to_owned()),
            "Response pagination did not advance"
        );
        params["cursor"] = next.into();
    }
    Ok(json!({"data":data}))
}

fn load_icon(host: &dyn Host, url: &str) -> Result<String> {
    let response = host.fetch(&Request::Http(
        json!({"url":url,"method":"GET","headers":{},"retry":"never"}),
    ))?;
    let response = response.get("value").unwrap_or(&response);
    let wire = &response["response"]["__response"];
    let status = wire["status"].as_u64().context("Invalid icon response")?;
    ensure!((200..300).contains(&status), "Plugin icon request failed");
    let mime = wire["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|header| {
            (header[0].as_str()?.eq_ignore_ascii_case("content-type"))
                .then(|| header[1].as_str())
                .flatten()
        })
        .unwrap_or("image/png");
    ensure!(mime.starts_with("image/"), "Plugin icon is not an image");
    let bytes = wire["base64"]
        .as_str()
        .context("Plugin icon response is not binary")?;
    Ok(format!("data:{mime};base64,{bytes}"))
}

async fn execute(
    bridge: &Bridge,
    host: &dyn Host,
    action: &Action,
    roots: &[String],
    configuration: &Value,
) -> Result<Option<String>> {
    match action {
        Action::Refresh | Action::ReloadHooks => {}
        Action::ReloadServers => {
            bridge
                .request::<_, Value>("config/mcpServer/reload", json!({}))
                .await?;
        }
        Action::SetServerEnabled { .. }
        | Action::SaveServer { .. }
        | Action::RemoveServer(_)
        | Action::Authenticate { .. } => {
            return execute_server_action(bridge, action, configuration).await;
        }
        Action::SetHookEnabled { key, enabled } => {
            write(
                bridge,
                None,
                vec![edit(
                    format!("hooks.state.{}.enabled", quote(key)?),
                    json!(enabled),
                    "upsert",
                )],
            )
            .await?;
        }
        Action::TrustHooks(hooks) => trust_hooks(bridge, hooks, roots).await?,
        Action::SetPluginEnabled { plugin, enabled } => {
            set_plugin_enabled(bridge, host, plugin, *enabled).await?;
        }
        Action::RemovePlugin(plugin) => uninstall_plugin(bridge, plugin).await?,
        Action::RemoveMarketplace(name) => remove_marketplace(bridge, name, roots).await?,
        Action::AddMarketplace(params) => {
            ensure!(
                params["source"]
                    .as_str()
                    .is_some_and(|source| !source.trim().is_empty()),
                "Enter a marketplace source"
            );
            bridge
                .request::<_, Value>("marketplace/add", params)
                .await?;
        }
        Action::UpgradeMarketplace(name) => {
            bridge
                .request::<_, Value>("marketplace/upgrade", json!({"marketplaceName":name}))
                .await?;
        }
        Action::SetAppEnabled { id, enabled } => {
            write(
                bridge,
                None,
                vec![edit(
                    format!("apps.{}.enabled", quote(id)?),
                    json!(enabled),
                    "upsert",
                )],
            )
            .await?;
        }
        Action::SetSkillEnabled { path, enabled } => {
            bridge
                .request::<_, Value>(
                    "skills/config/write",
                    json!({"path":path,"enabled":enabled}),
                )
                .await?;
        }
    }
    Ok(None)
}

async fn execute_server_action(
    bridge: &Bridge,
    action: &Action,
    configuration: &Value,
) -> Result<Option<String>> {
    let server_key = match action {
        Action::SetServerEnabled { key, .. }
        | Action::SaveServer { key, .. }
        | Action::RemoveServer(key) => Some(key),
        _ => None,
    };
    if let Some(key) = server_key {
        ensure!(
            !ui::screens::settings::integrations::server_hidden(key),
            "Internal MCP servers are managed by Codex"
        );
        ensure!(
            !ui::screens::settings::integrations::server_read_only(configuration, key),
            "Project MCP servers are read-only"
        );
    }
    match action {
        Action::SetServerEnabled { key, enabled } => {
            write(
                bridge,
                None,
                vec![edit(
                    format!("mcp_servers.{}.enabled", quote(key)?),
                    json!(enabled),
                    "upsert",
                )],
            )
            .await?;
        }
        Action::SaveServer {
            key,
            config,
            expected_version,
        } => {
            ensure!(
                !expected_version.is_empty(),
                "Configuration version is unavailable"
            );
            ensure!(config.is_object(), "Invalid MCP configuration");
            let field = if config.get("url").is_some() {
                "url"
            } else {
                "command"
            };
            ensure!(
                config[field]
                    .as_str()
                    .is_some_and(|value| !value.trim().is_empty()),
                "MCP {field} is required"
            );
            write(
                bridge,
                Some(expected_version),
                vec![edit(
                    format!("mcp_servers.{}", quote(key)?),
                    config.clone(),
                    "replace",
                )],
            )
            .await?;
        }
        Action::RemoveServer(key) => {
            write(
                bridge,
                None,
                vec![edit(
                    format!("mcp_servers.{}", quote(key)?),
                    Value::Null,
                    "replace",
                )],
            )
            .await?;
        }
        Action::Authenticate { name, registration } => {
            let mut params = json!({"name":name});
            if *registration != "auto" {
                params["clientRegistration"] = (*registration).into();
            }
            let result: Value = bridge.request("mcpServer/oauth/login", params).await?;
            return Ok(Some(authorization_url(&result)?.into()));
        }
        _ => return Err(anyhow::anyhow!("Unsupported MCP server action")),
    }
    Ok(None)
}

async fn trust_hooks(bridge: &Bridge, hooks: &[(String, String)], roots: &[String]) -> Result<()> {
    ensure!(!hooks.is_empty(), "No hooks selected for trust");
    let edits = hooks
        .iter()
        .map(|(key, hash)| {
            Ok(edit(
                format!("hooks.state.{}.trusted_hash", quote(key)?),
                json!(hash),
                "upsert",
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    write(bridge, None, edits).await?;
    let loaded: Value = bridge.request("hooks/list", json!({"cwds":roots})).await?;
    for (key, hash) in hooks {
        let trusted = loaded["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
            .any(|hook| {
                hook["key"] == *key
                    && hook["currentHash"] == *hash
                    && hook["trustStatus"] == "trusted"
            });
        ensure!(
            trusted,
            "Another config layer overrides these hook settings and must be changed before approval can take effect"
        );
    }
    Ok(())
}

async fn set_plugin_enabled(
    bridge: &Bridge,
    host: &dyn Host,
    plugin: &Value,
    enabled: bool,
) -> Result<()> {
    if plugin["source"]["type"] == "remote" {
        let id = plugin["remotePluginId"]
            .as_str()
            .context("Remote plugin has no identifier")?;
        let id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
        let endpoint = if enabled { "enable" } else { "disable" };
        host::json_body(&host.fetch(&plugin_request(
            host,
            format!("/ps/plugins/{id}/{endpoint}"),
        ))?)?;
    } else {
        let id = plugin["id"].as_str().context("Plugin has no identifier")?;
        write(
            bridge,
            None,
            vec![edit(
                format!("plugins.{}.enabled", quote(id)?),
                json!(enabled),
                "upsert",
            )],
        )
        .await?;
    }
    bridge
        .request::<_, Value>("config/mcpServer/reload", json!({}))
        .await?;
    Ok(())
}

fn plugin_request(host: &dyn Host, url: String) -> Request {
    let mut request = json!({"url":url,"method":"POST","body":"{}","headers":{
        "Content-Type":"application/json","X-OpenAI-Attach-Auth":"1",
        "X-OpenAI-Attach-Desktop-Surface":"1","X-OpenAI-Attach-Integrity-State":"1"
    },"returnErrorResponse":true});
    if let Some(identity) = host.identity() {
        request["expectedIdentity"] =
            json!({"accountId":identity.account_id,"userId":identity.user_id});
    }
    Request::Http(request)
}

pub(crate) fn read_bootstrap(host: &dyn Host) -> Result<Value> {
    let session = host::device_id();
    let stable = host
        .identity()
        .map_or_else(|| session.clone(), |identity| identity.device_id);
    let request = Request::Http(json!({
        "url":"/wham/statsig/bootstrap", "method":"POST",
        "body":json!({
            "app_session_id":session, "stable_id":stable, "app_version":env!("CARGO_PKG_VERSION"),
            "brand_name":"chatgpt", "build_flavor":"prod", "locale":"en-US",
            "system_name":if cfg!(target_os="macos") { "macOS" } else { std::env::consts::OS },
            "system_version":"", "window_type":"electron"
        }).to_string(),
        "headers":{"OAI-Language":"en","X-OpenAI-Attach-Auth":"1","X-OpenAI-Attach-Integrity-State":"1","originator":"Codex Desktop"},
        "retry":false, "returnErrorResponse":true
    }));
    let response = host::json_body(&host.fetch(&request)?)?;
    let payload: Value = serde_json::from_str(
        response["statsigPayload"]
            .as_str()
            .context("Missing Statsig payload")?,
    )?;
    Ok(payload)
}

async fn remove_marketplace(bridge: &Bridge, name: &str, roots: &[String]) -> Result<()> {
    let config: Value = bridge
        .request("config/read", json!({"includeLayers":false}))
        .await?;
    ensure!(
        config["config"]["marketplaces"].get(name).is_some(),
        "Only configured marketplaces can be removed"
    );
    let installed: Value = bridge
        .request("plugin/installed", json!({"cwds":roots}))
        .await?;
    let mut seen = HashSet::new();
    for marketplace in installed["marketplaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|marketplace| marketplace["name"] == name)
    {
        for plugin in marketplace["plugins"].as_array().into_iter().flatten() {
            let id = plugin["id"]
                .as_str()
                .context("Installed plugin has no identifier")?;
            if seen.insert(id) {
                uninstall_plugin(bridge, plugin).await?;
            }
        }
    }
    bridge
        .request::<_, Value>("marketplace/remove", json!({"marketplaceName":name}))
        .await?;
    Ok(())
}

async fn uninstall_plugin(bridge: &Bridge, plugin: &Value) -> Result<()> {
    let id = plugin["id"].as_str().context("Plugin has no identifier")?;
    ensure!(
        id != "code-review@openai-bundled",
        "The Code Review plugin cannot be uninstalled"
    );
    bridge
        .request::<_, Value>("plugin/uninstall", json!({"pluginId":id}))
        .await?;
    Ok(())
}

fn quote(key: &str) -> Result<String> {
    ensure!(!key.is_empty(), "Configuration key is empty");
    Ok(serde_json::to_string(key)?)
}

fn edit(key: String, value: Value, strategy: &str) -> Value {
    json!({"keyPath":key,"value":value,"mergeStrategy":strategy})
}

async fn write(bridge: &Bridge, expected_version: Option<&str>, edits: Vec<Value>) -> Result<()> {
    let response: Value = bridge
        .request(
            "config/batchWrite",
            json!({"edits":edits,"filePath":null,"expectedVersion":expected_version,"reloadUserConfig":true}),
        )
        .await?;
    ensure!(
        response["status"] != "okOverridden",
        "Another config layer overrides these settings"
    );
    Ok(())
}

fn authorization_url(response: &Value) -> Result<&str> {
    let url = response["authorizationUrl"]
        .as_str()
        .context("MCP OAuth login returned an empty authorization URL")?;
    let parsed =
        url::Url::parse(url).context("MCP OAuth login returned an invalid authorization URL")?;
    ensure!(
        matches!(parsed.scheme(), "https" | "http") && parsed.host_str().is_some(),
        "MCP OAuth login returned an invalid authorization URL"
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_cannot_open_an_executable_or_local_file_url() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        for url in ["javascript:alert(1)", "file:///etc/passwd", "", "not a url"] {
            assert!(authorization_url(&json!({"authorizationUrl":url})).is_err());
        }
        assert_eq!(
            authorization_url(&json!({"authorizationUrl":"https://service.example/authorize"}))
                .unwrap(),
            "https://service.example/authorize"
        );
    }

    #[test]
    fn login_completion_queues_one_refresh_while_an_operation_runs() {
        let mut state = State {
            pending: true,
            ..Default::default()
        };
        assert!(defer_if_busy(&mut state, &Action::Refresh));
        assert!(defer_if_busy(&mut state, &Action::Refresh));
        assert!(state.refresh_requested);
        state.pending = false;
        assert!(std::mem::take(&mut state.refresh_requested));
        assert!(!defer_if_busy(&mut state, &Action::Refresh));
        assert!(!state.refresh_requested);
    }

    #[test]
    fn old_server_forms_do_not_overwrite_newer_settings() -> Result<()> {
        const SERVER: &str = r#"
import json, sys
config = {"command":"server", "tool_timeout_sec":90}
for line in sys.stdin:
    request = json.loads(line)
    params = request.get("params", {})
    if request["method"] == "config/batchWrite":
        if params.get("expectedVersion") != "current":
            print(json.dumps({"id":request["id"],"error":{"code":-32001,"message":"Configuration version mismatch"}}), flush=True)
            continue
        config = params["edits"][0]["value"]
        result = {"status":"ok"}
    elif request["method"] == "config/read":
        result = {"config":{"mcp_servers":{"service":config}}}
    else:
        continue
    print(json.dumps({"id":request["id"],"result":result}), flush=True)
"#;
        let mut command = std::process::Command::new("python3");
        command.args(["-u", "-c", SERVER]);
        let (bridge, _) = Bridge::spawn(command)?;
        let stale = Action::SaveServer {
            key: "service".into(),
            config: json!({"command":"updated", "tool_timeout_sec":30}),
            expected_version: "old".into(),
        };
        let result = pollster::block_on(execute_server_action(&bridge, &stale, &json!({})));
        let error = result.expect_err("a stale form must not replace newer settings");
        assert!(error.to_string().contains("Configuration version mismatch"));
        let mut state = State::default();
        assert!(integration_failed(&mut state, &stale, error).is_none());
        assert_eq!(state.form_saved, 0);
        assert!(
            state
                .form_error
                .as_deref()
                .is_some_and(|message| message.contains("Configuration version mismatch"))
        );
        let current: Value = pollster::block_on(bridge.request("config/read", json!({})))?;
        assert_eq!(
            current["config"]["mcp_servers"]["service"]["tool_timeout_sec"],
            90
        );
        let fresh = Action::SaveServer {
            key: "service".into(),
            config: json!({"command":"updated", "tool_timeout_sec":90}),
            expected_version: "current".into(),
        };
        pollster::block_on(execute_server_action(&bridge, &fresh, &current))?;
        let current: Value = pollster::block_on(bridge.request("config/read", json!({})))?;
        assert_eq!(
            current["config"]["mcp_servers"]["service"],
            json!({"command":"updated","tool_timeout_sec":90})
        );
        Ok(())
    }

    #[test]
    fn closing_another_window_keeps_the_first_manager_connected() -> Result<()> {
        let home = tempfile::tempdir()?;
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl");
        let mut cx = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            || Ok(None),
        );
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
            cx.set_global(crate::host_settings::Settings::default());
            crate::init(cx);
        });
        let open = |cx: &mut gpui::HeadlessAppContext| -> Result<_> {
            let (bridge, events) = Bridge::replay(&fixture)?;
            let window =
                cx.open_window(gpui::size(gpui::px(1280.), gpui::px(832.)), |window, cx| {
                    cx.new(|cx| {
                        Root::new(
                            crate::State::default(),
                            bridge,
                            events,
                            home.path().display().to_string(),
                            window,
                            cx,
                        )
                    })
                })?;
            let root = cx.read_window(&window, |root, _| root)?;
            cx.update(|cx| {
                root.update(cx, |root, cx| {
                    root.set_host(Arc::new(host::LocalHost::new(home.path())), cx);
                    root.load_settings_integrations(cx);
                })
            });
            Ok((window, root))
        };
        let (_first_window, first) = open(&mut cx)?;
        let (second_window, second) = open(&mut cx)?;
        let first_state = cx.update(|cx| first.read(cx).settings_integrations.clone());
        let second_state = cx.update(|cx| second.read(cx).settings_integrations.clone());
        cx.update_window(second_window.into(), |_, window, _| window.remove_window())?;
        drop(second);
        cx.update(|cx| {
            first_state.update(cx, |state, _| {
                state.pending = true;
                state.refresh_requested = false;
            });
            second_state.update(cx, |state, _| state.refresh_requested = false);
            let dispatch = first_state
                .read(cx)
                .dispatch
                .clone()
                .expect("first manager dispatch");
            dispatch(Action::Refresh, cx);
            assert!(first_state.read(cx).refresh_requested);
            assert!(!second_state.read(cx).refresh_requested);
        });
        Ok(())
    }
}
