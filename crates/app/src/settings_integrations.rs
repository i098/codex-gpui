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
        if cx
            .try_global::<State>()
            .is_none_or(|state| state.dispatch.is_none())
        {
            let root = cx.weak_entity();
            cx.set_global(State {
                configuration: Arc::new(json!({"config":self.config})),
                dispatch: Some(Arc::new(move |action, cx| {
                    root.update(cx, |root, cx| root.settings_integration_action(action, cx))
                        .ok();
                })),
                ..Default::default()
            });
        }
        self.settings_integration_action(Action::Refresh, cx);
    }

    fn settings_integration_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if cx.global::<State>().pending {
            return;
        }
        let saved_form = matches!(action, Action::SaveServer { .. } | Action::RemoveServer(_));
        let oauth = match &action {
            Action::Authenticate { name, .. } => Some(name.clone()),
            _ => None,
        };
        if let Some(name) = &oauth {
            if let Some(Some(url)) = cx.global::<State>().oauth.get(name) {
                cx.open_url(url);
                return;
            }
            if cx.global::<State>().oauth.contains_key(name) {
                return;
            }
        }
        cx.update_global::<State, _>(|state, _| {
            state.pending = true;
            state.form_error = None;
            state.operation_error = None;
            if let Some(name) = &oauth {
                state.oauth.insert(name.clone(), None);
            }
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let roots = cx.global::<State>().roots.clone();
        let configuration = cx.global::<State>().configuration.clone();
        let cached_plugins = match &cx.global::<State>().plugins {
            Load::Ready(value) => value.clone(),
            _ => Arc::new(Value::Null),
        };
        let cached_gates = cx.global::<State>().gates_loaded.then(|| {
            let state = cx.global::<State>();
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
        cx.spawn(async move |this, cx| {
            let (result, snapshot) = read.await;
            this.update(cx, |this, cx| {
                if let Some(snapshot) = snapshot {
                    if let Ok(configuration) = &snapshot.configuration {
                        this.config = configuration["config"].clone();
                    }
                    cx.update_global::<State, _>(|state, _| snapshot.apply(state));
                }
                let mut notice = None;
                cx.update_global::<State, _>(|state, cx| {
                    state.pending = false;
                    match result {
                        Ok(url) => {
                            if saved_form {
                                state.form_saved += 1;
                            }
                            match &action {
                                Action::SetServerEnabled { .. }
                                | Action::SaveServer { .. }
                                | Action::RemoveServer(_) => state.reload_required = true,
                                Action::ReloadServers => {
                                    state.reload_required = false;
                                    state.suppressed_servers.clear();
                                }
                                Action::RemoveMarketplace(name) => {
                                    state.removal_completed = Some(name.clone())
                                }
                                Action::RemovePlugin(plugin) => {
                                    state.removal_completed =
                                        plugin["id"].as_str().map(|id| format!("plugin:{id}"))
                                }
                                _ => {}
                            }
                            if let Action::RemoveServer(key) = &action {
                                state.suppressed_servers.insert(key.clone());
                            }
                            if let Action::Authenticate { name, .. } = &action
                                && let Some(url) = url
                            {
                                state.oauth.insert(name.clone(), Some(url.clone()));
                                cx.open_url(&url);
                            }
                        }
                        Err(error) => {
                            if let Action::Authenticate { name, .. } = &action {
                                state.oauth.remove(name);
                            }
                            if saved_form {
                                state.form_error = Some(error.to_string());
                            } else if matches!(
                                &action,
                                Action::TrustHooks(_)
                                    | Action::SetHookEnabled { .. }
                                    | Action::RemoveMarketplace(_)
                                    | Action::RemovePlugin(_)
                            ) {
                                state.operation_error = Some(error.to_string());
                            } else {
                                notice = Some(error.to_string());
                            }
                        }
                    }
                });
                if let Some(notice) = notice {
                    this.toast(Toast::danger(notice), cx);
                }
                if matches!(action, Action::ReloadHooks)
                    && matches!(&cx.global::<State>().hooks, Load::Ready(_))
                {
                    this.toast(Toast::info("Refreshed hooks"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn settings_integration_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        if method != "mcpServer/oauth/login/completed" || !cx.has_global::<State>() {
            return;
        }
        let Some(name) = params["name"].as_str() else {
            return;
        };
        cx.update_global::<State, _>(|state, _| {
            state.oauth.remove(name);
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
        for marketplace in value["marketplaces"].as_array_mut().into_iter().flatten() {
            for plugin in marketplace["plugins"]
                .as_array_mut()
                .into_iter()
                .flatten()
                .filter(|plugin| {
                    ui::screens::settings::integrations::visible_plugin(plugin, &hidden_plugins)
                })
            {
                let cached = cached_plugins["marketplaces"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|marketplace| marketplace["plugins"].as_array().into_iter().flatten())
                    .find(|old| {
                        old["id"] == plugin["id"] && old["interface"] == plugin["interface"]
                    })
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
    let server_key = match action {
        Action::SetServerEnabled { key, .. }
        | Action::SaveServer { key, .. }
        | Action::RemoveServer(key) => Some(key),
        _ => None,
    };
    if let Some(key) = server_key {
        ensure!(
            !ui::screens::settings::integrations::server_read_only(configuration, key),
            "Project MCP servers are read-only"
        );
    }
    match action {
        Action::Refresh | Action::ReloadHooks => {}
        Action::ReloadServers => {
            bridge
                .request::<_, Value>("config/mcpServer/reload", json!({}))
                .await?;
        }
        Action::SetServerEnabled { key, enabled } => {
            write(
                bridge,
                vec![edit(
                    format!("mcp_servers.{}.enabled", quote(key)?),
                    json!(enabled),
                    "upsert",
                )],
            )
            .await?;
        }
        Action::SaveServer { key, config } => {
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
        Action::SetHookEnabled { key, enabled } => {
            write(
                bridge,
                vec![edit(
                    format!("hooks.state.{}.enabled", quote(key)?),
                    json!(enabled),
                    "upsert",
                )],
            )
            .await?;
        }
        Action::TrustHooks(hooks) => {
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
            write(bridge, edits).await?;
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
        }
        Action::SetPluginEnabled { plugin, enabled } => {
            if plugin["source"]["type"] == "remote" {
                let id = plugin["remotePluginId"]
                    .as_str()
                    .context("Remote plugin has no identifier")?;
                let id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
                let endpoint = if *enabled { "enable" } else { "disable" };
                host::json_body(&host.fetch(&plugin_request(
                    host,
                    format!("/ps/plugins/{id}/{endpoint}"),
                ))?)?;
            } else {
                let id = plugin["id"].as_str().context("Plugin has no identifier")?;
                write(
                    bridge,
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
        }
        Action::RemovePlugin(plugin) => uninstall_plugin(bridge, plugin).await?,
        Action::RemoveMarketplace(name) => remove_marketplace(bridge, name, roots).await?,
        Action::UpgradeMarketplace(name) => {
            bridge
                .request::<_, Value>("marketplace/upgrade", json!({"marketplaceName":name}))
                .await?;
        }
        Action::SetAppEnabled { id, enabled } => {
            write(
                bridge,
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

async fn write(bridge: &Bridge, edits: Vec<Value>) -> Result<()> {
    let response: Value = bridge
        .request(
            "config/batchWrite",
            json!({"edits":edits,"filePath":null,"expectedVersion":null,"reloadUserConfig":true}),
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
}
