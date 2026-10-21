use crate::{
    Root, SettingsLeave,
    host::{self, Host, Identity, Request},
};
use anyhow::{Context as _, Result, bail, ensure};
use bridge::Bridge;
use gpui::{AppContext as _, Context, Focusable as _};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use ui::dialog::{Dialog, WritingApp, WritingAuthorization, WritingCategory, WritingConnection};
use ui::screens::settings::personalization::{
    MemoryChange, MemorySettings, WritingPlugin, WritingPreference,
};
use ui::toast::Toast;

pub struct MemoryRead {
    pub config: Value,
    pub settings: MemorySettings,
}

pub struct MemoryWrite {
    pub read: Result<MemoryRead>,
    pub error: Option<String>,
}

fn request(bridge: &Bridge, method: &'static str, params: Value) -> Result<Value> {
    pollster::block_on(bridge.request(method, params))
}

fn read_unified_instructions(host: &dyn Host) -> Result<bool> {
    let Some(identity) = host.identity() else {
        return Ok(false);
    };
    let response = host.fetch(&Request::Http(json!({
        "url": "/wham/statsig/bootstrap",
        "method": "POST",
        "body": json!({
            "app_session_id": crate::uuid_v4(),
            "app_version": env!("CARGO_PKG_VERSION"),
            "brand_name": "chatgpt",
            "build_flavor": "prod",
            "locale": "en-US",
            "stable_id": identity.device_id,
            "system_name": if cfg!(target_os = "macos") { "macOS" } else { std::env::consts::OS },
            "window_type": "electron"
        }).to_string(),
        "headers": {
            "OAI-Language": "en",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop"
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "retry": false,
        "returnErrorResponse": true
    })))?;
    let response = host::json_body(&response)?;
    let payload: Value = serde_json::from_str(
        response["statsigPayload"]
            .as_str()
            .context("missing personalization feature gates")?,
    )?;
    Ok(payload["feature_gates"]["2826489636"]["value"]
        .as_bool()
        .unwrap_or(false))
}

pub fn read_memory(bridge: &Bridge) -> Result<MemoryRead> {
    let mut response = request(bridge, "config/read", json!({"includeLayers": true}))?;
    let file_path = response["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|layer| layer["name"]["type"] == "user")
        .and_then(|layer| layer["name"]["file"].as_str())
        .map(str::to_owned);
    let config = response
        .get_mut("config")
        .filter(|value| value.is_object())
        .map(Value::take)
        .context("missing memory configuration")?;
    let mut cursor = Value::Null;
    let mut feature = None;
    loop {
        let page = request(
            bridge,
            "experimentalFeature/list",
            json!({"cursor": cursor, "limit": 100}),
        )?;
        let features = page["data"]
            .as_array()
            .context("missing experimental features")?;
        if let Some(memory) = features
            .iter()
            .find(|feature| feature["name"] == "memories")
        {
            feature = Some(
                memory["enabled"]
                    .as_bool()
                    .context("invalid memory feature state")?,
            );
        }
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    let memory = &config["memories"];
    let settings = MemorySettings {
        available: feature.is_some(),
        feature_enabled: feature.unwrap_or(false),
        generate: memory["generate_memories"].as_bool().unwrap_or(true),
        use_memories: memory["use_memories"].as_bool().unwrap_or(true),
        disable_on_external_context: memory["disable_on_external_context"]
            .as_bool()
            .or_else(|| memory["no_memories_if_mcp_or_web_search"].as_bool())
            .unwrap_or(false),
        file_path,
    };
    Ok(MemoryRead { config, settings })
}

pub fn write_memory(
    bridge: &Bridge,
    previous: &MemorySettings,
    change: MemoryChange,
) -> MemoryWrite {
    let results = match change {
        MemoryChange::Enabled(enabled) => [
            Some(request(
                bridge,
                "config/batchWrite",
                json!({
                    "edits": [{"keyPath": "features.memories", "value": enabled, "mergeStrategy": "upsert"}],
                    "filePath": null, "expectedVersion": null
                }),
            )),
            Some(write_memory_config(
                bridge,
                previous,
                json!([
                    {"keyPath": "memories.generate_memories", "value": enabled, "mergeStrategy": "upsert"},
                    {"keyPath": "memories.use_memories", "value": enabled, "mergeStrategy": "upsert"}
                ]),
            )),
            (!enabled).then(|| {
                write_memory_config(
                    bridge,
                    previous,
                    json!([
                        {"keyPath": "features.chronicle", "value": false, "mergeStrategy": "upsert"}
                    ]),
                )
            }),
        ],
        MemoryChange::ToolAssisted(enabled) => [
            Some(write_memory_config(
                bridge,
                previous,
                json!([
                    {"keyPath": "memories.disable_on_external_context", "value": !enabled, "mergeStrategy": "upsert"},
                    {"keyPath": "memories.no_memories_if_mcp_or_web_search", "value": null, "mergeStrategy": "upsert"}
                ]),
            )),
            None,
            None,
        ],
    };
    let error = results
        .into_iter()
        .flatten()
        .find_map(Result::err)
        .map(|error| error.to_string());
    MemoryWrite {
        read: read_memory(bridge),
        error,
    }
}

fn write_memory_config(bridge: &Bridge, previous: &MemorySettings, edits: Value) -> Result<Value> {
    request(
        bridge,
        "config/batchWrite",
        json!({
            "edits": edits,
            "filePath": previous.file_path,
            "expectedVersion": null,
            "reloadUserConfig": true
        }),
    )
}

pub fn reset_memory(bridge: &Bridge) -> Result<()> {
    request(bridge, "memory/reset", Value::Null).map(|_| ())
}

const WRITING_APP: &str = "connector_openai_write_like_me";

pub struct WritingWrite {
    pub preference: Result<WritingPreference>,
    pub error: Option<String>,
    pub skills: Option<Value>,
    pub authorization: Option<Arc<Vec<WritingAuthorization>>>,
}

fn authenticated_response(
    host: &dyn Host,
    identity: &Identity,
    mut request: Value,
) -> Result<Value> {
    request["headers"] = json!({
        "OAI-Product-Sku": "CODEX",
        "X-Codex-Surface": "settings",
        "OAI-Language": "en-US",
        "oai-did": identity.device_id,
        "ChatGPT-Account-ID": identity.account_id,
        "X-OpenAI-Expected-Account-Id": identity.account_id,
        "X-OpenAI-Attach-Auth": "1",
        "X-OpenAI-Attach-Desktop-Surface": "1",
        "X-OpenAI-Attach-Integrity-State": "1"
    });
    request["expectedIdentity"] = json!({
        "accountId": identity.account_id,
        "userId": identity.user_id
    });
    host.fetch(&Request::Http(request))
}

fn authenticated(host: &dyn Host, identity: &Identity, request: Value) -> Result<Value> {
    authenticated_response(host, identity, request).and_then(|response| host::json_body(&response))
}

fn response_status(response: &Value) -> Result<u64> {
    let response = response.get("value").unwrap_or(response);
    response
        .pointer("/response/__response/status")
        .or_else(|| response.get("status"))
        .and_then(Value::as_u64)
        .context("missing HTTP status")
}

fn authenticated_empty(host: &dyn Host, identity: &Identity, request: Value) -> Result<()> {
    let response = authenticated_response(host, identity, request)?;
    let status = response_status(&response)?;
    ensure!(
        (200..300).contains(&status),
        "plugin request failed with status {status}"
    );
    Ok(())
}

fn plugin_path(id: &str, action: Option<&str>) -> Result<String> {
    let mut url = url::Url::parse("https://chatgpt.com/ps/plugins/")?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid plugin URL"))?;
    segments.pop_if_empty().push(id);
    if let Some(action) = action {
        segments.push(action);
    }
    drop(segments);
    Ok(url.path().to_owned())
}

fn installed_plugins(host: &dyn Host, identity: &Identity) -> Result<Vec<Value>> {
    let mut next: Option<String> = None;
    let mut installed = Vec::new();
    loop {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("limit", "1000");
        if let Some(token) = &next {
            query.append_pair("pageToken", token);
        }
        let mut page = authenticated(
            host,
            identity,
            json!({
                "url": format!("/ps/plugins/installed?{}", query.finish()), "method": "GET"
            }),
        )?;
        next = page["pagination"]["next_page_token"]
            .as_str()
            .map(str::to_owned);
        let plugins = page
            .get_mut("plugins")
            .context("missing installed plugins")?
            .take();
        let Value::Array(mut plugins) = plugins else {
            bail!("invalid installed plugins");
        };
        installed.append(&mut plugins);
        if next.is_none() {
            return Ok(installed);
        }
    }
}

fn installed_writing(
    host: &dyn Host,
    identity: &Identity,
    id: Option<&str>,
) -> Result<Option<Value>> {
    let mut plugins = installed_plugins(host, identity)?;
    let found = plugins
        .iter()
        .position(|plugin| id.is_some_and(|id| plugin["id"] == id))
        .or_else(|| {
            plugins
                .iter()
                .position(|plugin| plugin["canonical_app_id"] == WRITING_APP)
        });
    Ok(found.map(|index| plugins.swap_remove(index)))
}

fn read_writing_for(host: &dyn Host, identity: Identity) -> Result<WritingPreference> {
    let canonical = authenticated(
        host,
        &identity,
        json!({
            "url": format!("/ps/plugins/canonical-app/{WRITING_APP}"), "method": "GET"
        }),
    )?;
    let id = canonical["plugin_id"].as_str();
    let installed = installed_writing(host, &identity, id)?;
    let enabled = installed
        .as_ref()
        .is_some_and(|plugin| plugin["enabled"] == true);
    let is_installed = installed.is_some();
    let plugin = match (installed, id) {
        (Some(plugin), _) => Some(plugin),
        (None, Some(id)) => Some(authenticated(
            host,
            &identity,
            json!({
                "url": plugin_path(id, None)?, "method": "GET"
            }),
        )?),
        (None, None) => None,
    };
    let can_install = plugin.as_ref().is_some_and(|plugin| {
        id.is_some_and(|id| plugin["id"] == id)
            && plugin["canonical_app_id"] == WRITING_APP
            && plugin["status"] == "ENABLED"
            && plugin["installation_policy"] != "NOT_AVAILABLE"
    });
    let plugin = plugin
        .map(|plugin| -> Result<WritingPlugin> {
            Ok(WritingPlugin {
                id: plugin["id"]
                    .as_str()
                    .context("missing Writing plugin ID")?
                    .to_owned(),
            })
        })
        .transpose()?;
    Ok(WritingPreference {
        account_id: identity.account_id,
        user_id: identity.user_id,
        enabled,
        installed: is_installed,
        can_install,
        plugin,
    })
}

pub fn read_writing(host: &dyn Host) -> Result<WritingPreference> {
    let identity = host
        .identity()
        .context("Writing style identity is unavailable")?;
    read_writing_for(host, identity)
}

fn current_writing_account(host: &dyn Host, previous: &WritingPreference) -> Result<Identity> {
    let identity = host
        .identity()
        .context("Writing style requires a ChatGPT account")?;
    ensure!(
        identity.account_id == previous.account_id && identity.user_id == previous.user_id,
        "Writing style account changed"
    );
    Ok(identity)
}

pub fn write_writing(
    host: &dyn Host,
    bridge: &Bridge,
    previous: &WritingPreference,
    enabled: bool,
    cwd: &str,
) -> WritingWrite {
    let mut authorization = None;
    let operation = (|| -> Result<Option<Value>> {
        let identity = current_writing_account(host, previous)?;
        ensure!(
            !enabled || previous.can_install,
            "Writing style is unavailable"
        );
        let plugin = previous
            .plugin
            .as_ref()
            .context("Writing plugin is unavailable")?;
        if enabled && !previous.installed {
            let result = request(
                bridge,
                "plugin/install",
                json!({
                    "marketplacePath": null,
                    "remoteMarketplaceName": "chatgpt-workspace",
                    "pluginName": plugin.id,
                }),
            )?;
            let apps = result["appsNeedingAuth"]
                .as_array()
                .context("missing plugin authorization result")?;
            if !apps.is_empty() {
                let apps = apps
                    .iter()
                    .map(|app| -> Result<WritingAuthorization> {
                        let url = app["installUrl"]
                            .as_str()
                            .context("missing app connection URL")?;
                        validate_connection_url(url)?;
                        Ok(WritingAuthorization {
                            app_id: app["id"].as_str().context("missing app ID")?.to_owned(),
                            name: app["name"].as_str().context("missing app name")?.to_owned(),
                            url: url.to_owned(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                authorization = Some(Arc::new(apps));
                return Ok(None);
            }
        } else if !enabled && previous.installed {
            request(bridge, "plugin/uninstall", json!({"pluginId": plugin.id}))?;
        } else if enabled && !previous.enabled {
            authenticated_empty(
                host,
                &identity,
                json!({
                    "url": plugin_path(&plugin.id, Some("enable"))?, "method": "POST"
                }),
            )?;
        }
        request(
            bridge,
            "skills/list",
            json!({"cwds": [cwd], "forceReload": true}),
        )
        .map(Some)
    })();
    let (skills, mut error) = match operation {
        Ok(skills) => (skills, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let preference = current_writing_account(host, previous)
        .and_then(|identity| read_writing_for(host, identity));
    if error.is_none()
        && authorization.is_none()
        && preference
            .as_ref()
            .is_ok_and(|preference| preference.enabled != enabled)
    {
        error = Some("Writing style preference was not saved".into());
    }
    WritingWrite {
        preference,
        error,
        skills,
        authorization,
    }
}

const WRITING_SOURCES: [(&str, &str, WritingCategory); 7] = [
    (
        "plugin_connector_1p_95d39881713c8191931482a62d6edff9",
        "Gmail",
        WritingCategory::Email,
    ),
    (
        "plugin_connector_1p_6bcb5879c73c819196abc70016166099",
        "Outlook Email",
        WritingCategory::Email,
    ),
    (
        "plugin_asdk_app_69a1d78e929881919bba0dbda1f6436d",
        "Slack",
        WritingCategory::Messaging,
    ),
    (
        "plugin_connector_1p_eba8b52fe53881918408d4b46b957644",
        "Teams",
        WritingCategory::Messaging,
    ),
    (
        "plugin_connector_1p_ab21a553bfbc81919ea8fd1858e3ffa7",
        "Google Drive",
        WritingCategory::Documents,
    ),
    (
        "plugin_connector_1p_dca009ae2c848191ae14df3a47c5e7fd",
        "SharePoint",
        WritingCategory::Documents,
    ),
    (
        "plugin_asdk_app_69c18c28f1188191bf5b8445c4ab0a2e",
        "Notion",
        WritingCategory::Documents,
    ),
];

fn app_key(id: &str) -> impl Iterator<Item = u8> + '_ {
    id.trim()
        .trim_start_matches("connector_")
        .trim_start_matches("mcp_")
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && !matches!(*byte, b'_' | b'-'))
        .map(|byte| byte.to_ascii_lowercase())
}

fn same_app(left: &str, right: &str) -> bool {
    app_key(left).eq(app_key(right))
}

fn available_apps(bridge: &Bridge) -> Result<Vec<Value>> {
    let mut cursor = Value::Null;
    let mut apps = Vec::new();
    loop {
        let mut page = request(
            bridge,
            "app/list",
            json!({
                "cursor": cursor, "limit": 100, "forceRefetch": true
            }),
        )?;
        cursor = page["nextCursor"].clone();
        let data = page
            .get_mut("data")
            .context("missing available apps")?
            .take();
        let Value::Array(mut data) = data else {
            bail!("invalid available apps");
        };
        apps.append(&mut data);
        if cursor.is_null() {
            return Ok(apps);
        }
    }
}

fn source_plugin(host: &dyn Host, identity: &Identity, id: &str) -> Result<Option<Value>> {
    let response = authenticated_response(
        host,
        identity,
        json!({
            "url": plugin_path(id, None)?, "method": "GET", "returnErrorResponse": true
        }),
    )?;
    if response_status(&response)? == 404 {
        return Ok(None);
    }
    host::json_body(&response).map(Some)
}

fn plugin_app_ids(plugin: &Value) -> Vec<String> {
    let mut ids = Vec::<String>::new();
    for id in [
        plugin["canonical_app_id"].as_str(),
        plugin["connector_id"].as_str(),
    ]
    .into_iter()
    .flatten()
    .chain(
        plugin["release"]["app_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str),
    ) {
        if !ids.iter().any(|old| same_app(old, id)) {
            ids.push(id.to_owned());
        }
    }
    ids
}

pub fn read_writing_apps(
    host: &dyn Host,
    bridge: &Bridge,
    preference: &WritingPreference,
    cwd: &str,
) -> Result<Vec<WritingApp>> {
    let identity = current_writing_account(host, preference)?;
    let apps = available_apps(bridge)?;
    let installed = request(bridge, "plugin/installed", json!({"cwds": [cwd]}))?;
    let plugins = installed["marketplaces"]
        .as_array()
        .context("missing installed marketplaces")?
        .iter()
        .flat_map(|market| market["plugins"].as_array().into_iter().flatten());
    let plugins: Vec<&Value> = plugins.collect();
    let links = authenticated(
        host,
        &identity,
        json!({
            "url": "/aip/connectors/links/list_accessible",
            "method": "POST",
            "body": json!({"principals": [], "link_refresh_strategy": "BLOCKING"}).to_string()
        }),
    )?;
    let links = links["links"]
        .as_array()
        .context("missing connected app links")?;
    let mut sources = Vec::with_capacity(WRITING_SOURCES.len());
    for (id, name, category) in WRITING_SOURCES {
        let Some(plugin) = source_plugin(host, &identity, id)? else {
            sources.push(WritingApp {
                plugin_id: id,
                name: name.into(),
                category,
                connection: WritingConnection::Unavailable,
                install_url: None,
                app_ids: Vec::new(),
            });
            continue;
        };
        let app_ids = plugin_app_ids(&plugin);
        let matching = |app: &&Value| {
            app["id"]
                .as_str()
                .is_some_and(|id| app_ids.iter().any(|key| same_app(key, id)))
        };
        let install_url = apps
            .iter()
            .filter(matching)
            .find_map(|app| app["installUrl"].as_str())
            .map(str::to_owned);
        let installed = plugins
            .iter()
            .copied()
            .find(|entry| entry["remotePluginId"] == id || entry["id"] == id);
        let accessible = app_ids.iter().all(|id| {
            apps.iter().any(|app| {
                app["id"].as_str().is_some_and(|key| same_app(key, id))
                    && app["isAccessible"] == true
                    && app["isEnabled"] == true
            })
        });
        let active_links = app_ids.iter().all(|id| {
            links.iter().any(|link| {
                link["visibility"] != "HIDDEN"
                    && !link["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("implicit_link::"))
                    && link["connector_id"]
                        .as_str()
                        .is_some_and(|key| same_app(key, id))
                    && link["auth_status"].as_str().unwrap_or("ACTIVE") == "ACTIVE"
            })
        });
        let reauth = links.iter().any(|link| {
            link["auth_status"] == "REAUTH_REQUIRED"
                && link["connector_id"]
                    .as_str()
                    .is_some_and(|key| app_ids.iter().any(|id| same_app(id, key)))
        });
        let disabled_app = apps
            .iter()
            .filter(matching)
            .any(|app| app["isEnabled"] == false);
        let connection = if app_ids.is_empty()
            || plugin["status"] != "ENABLED"
            || plugin["installation_policy"] == "NOT_AVAILABLE"
        {
            WritingConnection::Unavailable
        } else if installed.is_some_and(|plugin| plugin["enabled"] == false) || disabled_app {
            WritingConnection::Disabled
        } else if installed.is_some() && active_links && accessible {
            WritingConnection::Connected
        } else if reauth {
            WritingConnection::Reconnect
        } else if installed.is_some() && active_links {
            WritingConnection::Refresh
        } else if installed.is_some() {
            WritingConnection::Connect
        } else {
            WritingConnection::Install
        };
        sources.push(WritingApp {
            plugin_id: id,
            name: plugin["release"]["display_name"]
                .as_str()
                .unwrap_or(name)
                .to_owned()
                .into(),
            category,
            connection,
            install_url,
            app_ids,
        });
    }
    Ok(sources)
}

pub fn connect_writing_app(
    host: &dyn Host,
    bridge: &Bridge,
    preference: &WritingPreference,
    app: &WritingApp,
    cwd: &str,
) -> Result<Option<String>> {
    let identity = current_writing_account(host, preference)?;
    let mut install_url = app.install_url.clone();
    match app.connection {
        WritingConnection::Unavailable => bail!("This Writing source is unavailable"),
        WritingConnection::Install => {
            let response = request(
                bridge,
                "plugin/install",
                json!({
                    "marketplacePath": null, "remoteMarketplaceName": "chatgpt-workspace", "pluginName": app.plugin_id
                }),
            )?;
            if let Some(url) = response["appsNeedingAuth"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|app| app["installUrl"].as_str())
            {
                install_url = Some(url.to_owned());
            }
        }
        WritingConnection::Disabled => {
            authenticated_empty(
                host,
                &identity,
                json!({
                    "url": plugin_path(app.plugin_id, Some("enable"))?, "method": "POST"
                }),
            )?;
            let edits: Vec<Value> = app
                .app_ids
                .iter()
                .map(|id| {
                    json!({
                        "keyPath": format!("apps.{}.enabled", json!(id)),
                        "value": true, "mergeStrategy": "upsert"
                    })
                })
                .collect();
            request(
                bridge,
                "config/batchWrite",
                json!({
                    "edits": edits, "filePath": null, "expectedVersion": null, "reloadUserConfig": true
                }),
            )?;
        }
        WritingConnection::Connect | WritingConnection::Reconnect => {}
        WritingConnection::Connected | WritingConnection::Refresh => return Ok(None),
    }
    request(
        bridge,
        "skills/list",
        json!({"cwds": [cwd], "forceReload": true}),
    )?;
    if let Some(url) = &install_url {
        validate_connection_url(url)?;
    }
    ensure!(
        install_url.is_some() || matches!(app.connection, WritingConnection::Disabled),
        "This Writing source does not provide a connection URL"
    );
    Ok(install_url)
}

fn validate_connection_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url)?;
    ensure!(
        parsed.scheme() == "https"
            || parsed.scheme() == "http"
                && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1")),
        "Invalid Writing connection URL"
    );
    Ok(())
}

const WRITING_SAMPLE_PROMPT: &str = "Look through my chats, Library files, and any connected apps for a new and real writing task you could help with. Briefly explain what you found, why you chose it, and which examples of my own writing you’ll use. Then produce a useful draft closely matching my style. Point out two specific choices you made to match my style. If you can’t find a clear task or enough examples, ask me for a draft, template, or writing sample instead of inventing one. Show the draft here; do not send, post, or modify anything.";

impl Root {
    fn current_writing_preference(&self, cx: &gpui::App) -> Option<Arc<WritingPreference>> {
        self.settings_page
            .as_ref()
            .and_then(|page| page.read(cx).personalization.read(cx).writing_preference())
            .or_else(|| self.writing_dialog_preference.clone())
    }

    pub(super) fn authorize_writing(
        &mut self,
        dialog: gpui::Entity<Dialog>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.settings_page.as_ref() else {
            return;
        };
        let view = page.read(cx).personalization.clone();
        let Some(preference) = view.read(cx).writing_preference() else {
            return;
        };
        let Some(authorization) = dialog.read(cx).writing_authorizations() else {
            return;
        };
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let cwd: Arc<str> = self.cwd.as_str().into();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let mut opened = None;
            loop {
                if !this
                    .update_in(cx, |this, _, _| this.dialog.as_ref() == Some(&dialog))
                    .unwrap_or(false)
                {
                    return;
                }
                let pending = {
                    let host = host.clone();
                    let bridge = bridge.clone();
                    let preference = preference.clone();
                    let authorization = authorization.clone();
                    executor.spawn(async move {
                        current_writing_account(host.as_ref(), &preference)?;
                        let apps = available_apps(&bridge)?;
                        Ok::<_, anyhow::Error>(authorization.iter().position(|required| {
                            !apps.iter().any(|app| {
                                app["id"]
                                    .as_str()
                                    .is_some_and(|id| same_app(id, &required.app_id))
                                    && app["isAccessible"] == true
                            })
                        }))
                    })
                }
                .await;
                match pending {
                    Err(error) => {
                        this.update_in(cx, |this, _, cx| {
                            if this.dialog.as_ref() == Some(&dialog) {
                                dialog.update(cx, |dialog, cx| {
                                    dialog.writing_failed(error.to_string(), cx)
                                });
                            }
                        })
                        .ok();
                        return;
                    }
                    Ok(Some(index)) => {
                        let active = this
                            .update_in(cx, |this, _, cx| {
                                if this.dialog.as_ref() != Some(&dialog) {
                                    return false;
                                }
                                dialog.update(cx, |dialog, cx| {
                                    dialog.writing_authorization_progress(index, cx)
                                });
                                if opened != Some(index) {
                                    cx.open_url(&authorization[index].url);
                                }
                                true
                            })
                            .unwrap_or(false);
                        if !active {
                            return;
                        }
                        opened = Some(index);
                    }
                    Ok(None) => break,
                }
                executor.timer(Duration::from_secs(2)).await;
            }
            let result = executor
                .spawn(async move {
                    let identity = current_writing_account(host.as_ref(), &preference)?;
                    let fresh = read_writing_for(host.as_ref(), identity)?;
                    Ok::<_, anyhow::Error>(write_writing(
                        host.as_ref(),
                        &bridge,
                        &fresh,
                        true,
                        &cwd,
                    ))
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.dialog.as_ref() != Some(&dialog) {
                    return;
                }
                match result {
                    Ok(result)
                        if result.error.is_none()
                            && result.authorization.is_none()
                            && result
                                .preference
                                .as_ref()
                                .is_ok_and(|preference| preference.enabled) =>
                    {
                        if let Some(skills) = result.skills {
                            this.skills = skills;
                        }
                        view.update(cx, |view, cx| {
                            view.writing_changed(
                                result
                                    .preference
                                    .map(Some)
                                    .map_err(|error| error.to_string()),
                                None,
                                cx,
                            )
                        });
                        this.dialog = None;
                        this.open_writing_setup(window, cx);
                    }
                    _ => dialog.update(cx, |dialog, cx| {
                        dialog.writing_failed(
                            "Couldn't finish Writing style setup. Try again.".into(),
                            cx,
                        )
                    }),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_writing_setup(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(preference) = self.current_writing_preference(cx) else {
            return;
        };
        if self.instructions_changed(cx) {
            self.confirm_instructions_leave(SettingsLeave::WritingSetup, window, cx);
            return;
        }
        if self.dialog.is_some() {
            return;
        }
        let dialog = cx.new(|cx| Dialog::writing_style(preference.enabled, cx));
        self.writing_dialog_preference = Some(preference);
        let (route, automation) = self
            .back
            .iter()
            .rev()
            .find(|(route, _)| *route != crate::Route::Settings)
            .cloned()
            .unwrap_or((crate::Route::Home, None));
        self.remember_thread_settings(cx);
        self.state.route = route;
        self.sidebar_automation = automation;
        self.navigation_epoch += 1;
        self.sync(window, cx);
        self.select_route(cx);
        self.show_dialog(dialog.clone(), window, cx);
        self.load_writing_apps(dialog, cx);
    }

    pub(super) fn load_writing_apps(
        &mut self,
        dialog: gpui::Entity<Dialog>,
        cx: &mut Context<Self>,
    ) {
        let Some(preference) = self.current_writing_preference(cx) else {
            return;
        };
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let cwd = self.cwd.clone();
        let read = cx.background_spawn(async move {
            read_writing_apps(host.as_ref(), &bridge, &preference, &cwd)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if this.dialog.as_ref() == Some(&dialog) {
                    dialog.update(cx, |dialog, cx| dialog.writing_apps(result, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn connect_writing_source(
        &mut self,
        dialog: gpui::Entity<Dialog>,
        id: &'static str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(preference) = self.current_writing_preference(cx) else {
            return;
        };
        let Some(app) = dialog.read(cx).writing_app(id) else {
            return;
        };
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let cwd: Arc<str> = self.cwd.as_str().into();
        let operation = {
            let host = host.clone();
            let bridge = bridge.clone();
            let preference = preference.clone();
            let cwd = cwd.clone();
            cx.background_spawn(async move {
                connect_writing_app(host.as_ref(), &bridge, &preference, &app, &cwd)
            })
        };
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let operation = operation.await;
            let poll = this
                .update_in(cx, |this, _, cx| {
                    if this.dialog.as_ref() != Some(&dialog) {
                        return None;
                    }
                    match operation {
                        Ok(url) => {
                            if let Some(url) = &url {
                                cx.open_url(url);
                            }
                            Some(url.is_some())
                        }
                        Err(error) => {
                            dialog.update(cx, |dialog, cx| {
                                dialog.writing_failed(error.to_string(), cx)
                            });
                            None
                        }
                    }
                })
                .ok()
                .flatten();
            let Some(poll) = poll else {
                return;
            };
            loop {
                let active = this
                    .update_in(cx, |this, _, cx| {
                        this.dialog.as_ref() == Some(&dialog)
                            && dialog.read(cx).writing_is_pending(id)
                    })
                    .unwrap_or(false);
                if !active {
                    return;
                }
                if poll {
                    executor.timer(Duration::from_secs(2)).await;
                }
                let read = {
                    let host = host.clone();
                    let bridge = bridge.clone();
                    let preference = preference.clone();
                    let cwd = cwd.clone();
                    executor.spawn(async move {
                        read_writing_apps(host.as_ref(), &bridge, &preference, &cwd)
                            .map_err(|error| error.to_string())
                    })
                }
                .await;
                let keep_polling = this
                    .update_in(cx, |this, _, cx| {
                        if this.dialog.as_ref() != Some(&dialog) {
                            return false;
                        }
                        dialog.update(cx, |dialog, cx| {
                            if !poll {
                                dialog.writing_connection_finished(cx);
                            }
                            dialog.writing_apps(read, cx);
                            dialog.writing_is_pending(id)
                        })
                    })
                    .unwrap_or(false);
                if !poll || !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn start_writing_sample(
        &mut self,
        dialog: gpui::Entity<Dialog>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let view = self
            .settings_page
            .as_ref()
            .map(|page| page.read(cx).personalization.clone());
        let Some(preference) = self.current_writing_preference(cx) else {
            return;
        };
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let cwd = self.cwd.clone();
        let write = cx.background_spawn(async move {
            write_writing(host.as_ref(), &bridge, &preference, true, &cwd)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = write.await;
            this.update_in(cx, |this, window, cx| {
                if this.dialog.as_ref() != Some(&dialog) {
                    return;
                }
                let saved = result.error.is_none()
                    && result.authorization.is_none()
                    && result
                        .preference
                        .as_ref()
                        .is_ok_and(|preference| preference.enabled);
                if let Some(skills) = result.skills {
                    this.skills = skills;
                }
                if let Some(view) = &view {
                    view.update(cx, |view, cx| {
                        view.writing_changed(
                            result
                                .preference
                                .map(Some)
                                .map_err(|error| error.to_string()),
                            result.error,
                            cx,
                        )
                    });
                }
                if saved {
                    dialog.read(cx).release_artwork(window);
                    this.dialog = None;
                    this.writing_dialog_preference = None;
                    this.navigate(crate::Route::Home, window, cx);
                    let composer = this.composer(cx);
                    composer.update(cx, |composer, cx| {
                        composer.set_text(WRITING_SAMPLE_PROMPT, cx)
                    });
                    window.focus(&composer.read(cx).focus_handle(cx), cx);
                } else {
                    dialog.update(cx, |dialog, cx| {
                        dialog.writing_failed(
                            "Couldn't finish Writing style setup. Try again.".into(),
                            cx,
                        )
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn refresh_writing(&mut self, cx: &mut Context<Self>) {
        let Some(page) = &self.settings_page else {
            return;
        };
        let view = page.read(cx).personalization.clone();
        if !self.chatgpt_account {
            view.update(cx, |view, cx| view.writing_loaded(Ok(None), cx));
            return;
        }
        let host = self.host.clone();
        let read = cx.background_spawn(async move { read_writing(host.as_ref()) });
        cx.spawn(async move |this, cx| {
            let result = read.await.map(Some).map_err(|error| error.to_string());
            this.update(cx, |this, cx| {
                if this
                    .settings_page
                    .as_ref()
                    .is_some_and(|page| page.read(cx).personalization == view)
                {
                    view.update(cx, |view, cx| view.writing_loaded(result, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn change_writing(
        &mut self,
        enabled: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = &self.settings_page else {
            return;
        };
        let view = page.read(cx).personalization.clone();
        let Some(previous) = view.read(cx).writing_preference() else {
            return;
        };
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let cwd = self.cwd.clone();
        let write = cx.background_spawn(async move {
            write_writing(host.as_ref(), &bridge, &previous, enabled, &cwd)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = write.await;
            this.update_in(cx, |this, window, cx| {
                if !this
                    .settings_page
                    .as_ref()
                    .is_some_and(|page| page.read(cx).personalization == view)
                {
                    return;
                }
                let show_setup = enabled
                    && result.error.is_none()
                    && result.authorization.is_none()
                    && result
                        .preference
                        .as_ref()
                        .is_ok_and(|preference| preference.enabled);
                let authorization = if result.error.is_none() && result.preference.is_ok() {
                    result.authorization
                } else {
                    None
                };
                let mut error = result.error;
                if authorization.is_some() && this.dialog.is_some() {
                    error = Some("Close the open dialog and retry Writing setup".into());
                }
                if let Some(error) = &error {
                    eprintln!("codex-gpui: update Writing style: {error}");
                }
                if let Some(skills) = result.skills {
                    this.skills = skills;
                }
                view.update(cx, |view, cx| {
                    view.writing_changed(
                        result
                            .preference
                            .map(Some)
                            .map_err(|error| error.to_string()),
                        error,
                        cx,
                    )
                });
                if let Some(authorization) = authorization {
                    if this.dialog.is_some() {
                        return;
                    }
                    let dialog = cx.new(|cx| Dialog::writing_authorization(authorization, cx));
                    this.show_dialog(dialog.clone(), window, cx);
                    if this.dialog.as_ref() == Some(&dialog) {
                        view.update(cx, |view, cx| view.writing_awaiting_authorization(cx));
                        this.authorize_writing(dialog, window, cx);
                    }
                    return;
                }
                if show_setup {
                    this.open_writing_setup(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn refresh_memory(&mut self, cx: &mut Context<Self>) {
        let Some(page) = &self.settings_page else {
            return;
        };
        let view = page.read(cx).personalization.clone();
        let host = self.host.clone();
        let mode_view = view.clone();
        let mode_read =
            cx.background_spawn(async move { read_unified_instructions(host.as_ref()) });
        cx.spawn(async move |this, cx| {
            let result = mode_read.await;
            this.update(cx, |this, cx| {
                if this
                    .settings_page
                    .as_ref()
                    .is_some_and(|page| page.read(cx).personalization == mode_view)
                {
                    match result {
                        Ok(enabled) => mode_view
                            .update(cx, |view, cx| view.set_unified_instructions(enabled, cx)),
                        Err(error) => {
                            eprintln!("codex-gpui: personalization feature gates: {error:#}")
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
        let bridge = self.bridge.clone();
        let read = cx.background_spawn(async move { read_memory(&bridge) });
        cx.spawn(async move |this, cx| {
            let result = read
                .await
                .map(|read| read.settings)
                .map_err(|error| error.to_string());
            this.update(cx, |this, cx| {
                if this
                    .settings_page
                    .as_ref()
                    .is_some_and(|page| page.read(cx).personalization == view)
                {
                    view.update(cx, |view, cx| view.memory_loaded(result, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn change_memory(&mut self, change: MemoryChange, cx: &mut Context<Self>) {
        let Some(page) = &self.settings_page else {
            return;
        };
        let view = page.read(cx).personalization.clone();
        let Some(previous) = view.read(cx).memory().cloned() else {
            return;
        };
        let bridge = self.bridge.clone();
        let write = cx.background_spawn(async move { write_memory(&bridge, &previous, change) });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            this.update(cx, |this, cx| {
                if !this
                    .settings_page
                    .as_ref()
                    .is_some_and(|page| page.read(cx).personalization == view)
                {
                    return;
                }
                if let Some(error) = result.error {
                    eprintln!("codex-gpui: update memory settings: {error}");
                    if matches!(change, MemoryChange::ToolAssisted(_)) {
                        this.toast(
                            Toast::danger("Could not save tool-assisted memory preference"),
                            cx,
                        );
                    }
                }
                let settings = result
                    .read
                    .map(|mut read| {
                        if let Some(memory) = read.config.get_mut("memories") {
                            this.config["memories"] = memory.take();
                        }
                        this.config["features"]["memories"] = read.settings.feature_enabled.into();
                        if let Some(chronicle) = read.config["features"]["chronicle"].as_bool() {
                            this.config["features"]["chronicle"] = chronicle.into();
                        }
                        read.settings
                    })
                    .map_err(|error| error.to_string());
                view.update(cx, |view, cx| view.memory_loaded(settings, cx));
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn delete_memories(
        &mut self,
        dialog: gpui::Entity<ui::dialog::Dialog>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let delete = cx.background_spawn(async move { reset_memory(&bridge) });
        cx.spawn_in(window, async move |this, cx| {
            let result = delete.await;
            this.update_in(cx, |this, _, cx| {
                if this.dialog.as_ref() != Some(&dialog) {
                    return;
                }
                dialog.update(cx, |dialog, cx| dialog.memory_delete_finished(cx));
                match result {
                    Ok(()) => {
                        this.toast(Toast::success("Codex memories deleted"), cx);
                        dialog.update(cx, |_, cx| cx.emit(ui::dialog::Outcome::Dismiss));
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: delete memories: {error:#}");
                        this.toast(Toast::danger("Unable to delete Codex memories"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    fn server(exchanges: Vec<(&str, std::result::Result<Value, &str>)>) -> Bridge {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for (id, (method, result)) in exchanges.into_iter().enumerate() {
            writeln!(
                file,
                "{}",
                json!({"dir": "send", "msg": {
                    "id": id, "method": method, "params": {}
                }})
            )
            .unwrap();
            let reply = match result {
                Ok(result) => json!({"id": id, "result": result}),
                Err(message) => json!({"id": id, "error": {"code": -32000, "message": message}}),
            };
            writeln!(file, "{}", json!({"dir": "recv", "msg": reply})).unwrap();
        }
        Bridge::replay(file.path()).unwrap().0
    }

    #[test]
    fn unified_instructions_follow_the_original_gate_and_fail_closed() {
        struct GateHost(Value);
        impl Host for GateHost {
            fn identity(&self) -> Option<Identity> {
                Some(Identity {
                    account_id: "account".into(),
                    user_id: "user".into(),
                    device_id: "device".into(),
                })
            }

            fn fetch(&self, _: &Request) -> Result<Value> {
                Ok(json!({"status": 200, "body": self.0.to_string()}))
            }
        }
        for (gate, enabled) in [
            (json!(true), true),
            (json!(false), false),
            (Value::Null, false),
        ] {
            let host = GateHost(json!({"statsigPayload": json!({
                "feature_gates": {"2826489636": {"value": gate}}
            }).to_string()}));
            assert_eq!(read_unified_instructions(&host).unwrap(), enabled);
        }
        assert!(read_unified_instructions(&GateHost(json!({}))).is_err());
    }

    #[test]
    fn memory_prefers_the_current_tool_setting_over_the_legacy_setting() {
        let bridge = server(vec![
            (
                "config/read",
                Ok(json!({"config": {"memories": {
                    "disable_on_external_context": false, "no_memories_if_mcp_or_web_search": true
                }}})),
            ),
            (
                "experimentalFeature/list",
                Ok(json!({"data": [{"name": "memories", "enabled": true}]})),
            ),
        ]);
        let read = read_memory(&bridge).unwrap();
        assert!(!read.settings.disable_on_external_context);
        assert!(read.settings.enabled());
    }

    #[test]
    fn memory_reads_feature_availability_from_later_pages() {
        let bridge = server(vec![
            ("config/read", Ok(json!({"config": {}}))),
            (
                "experimentalFeature/list",
                Ok(json!({"data": [], "nextCursor": "next"})),
            ),
            (
                "experimentalFeature/list",
                Ok(json!({"data": [{"name": "memories", "enabled": true}]})),
            ),
        ]);
        assert!(read_memory(&bridge).unwrap().settings.enabled());
    }

    #[test]
    fn a_partial_memory_write_keeps_the_successful_feature_change() {
        let bridge = server(vec![
            ("config/batchWrite", Ok(Value::Null)),
            ("config/batchWrite", Err("managed memory configuration")),
            (
                "config/read",
                Ok(json!({"config": {"memories": {
                    "generate_memories": false, "use_memories": true
                }}})),
            ),
            (
                "experimentalFeature/list",
                Ok(json!({"data": [{"name": "memories", "enabled": true}]})),
            ),
        ]);
        let previous = MemorySettings {
            available: true,
            feature_enabled: false,
            generate: false,
            use_memories: true,
            disable_on_external_context: false,
            file_path: None,
        };
        let result = write_memory(&bridge, &previous, MemoryChange::Enabled(true));
        assert!(result.error.is_some());
        let saved = result.read.unwrap().settings;
        assert!(saved.feature_enabled);
        assert!(!saved.enabled());
    }

    struct WritingHost {
        account: &'static str,
        installed: bool,
        enabled: AtomicBool,
        requests: Mutex<Vec<Value>>,
    }

    impl Host for WritingHost {
        fn identity(&self) -> Option<Identity> {
            Some(Identity {
                account_id: self.account.into(),
                user_id: "user".into(),
                device_id: "device".into(),
            })
        }

        fn fetch(&self, request: &Request) -> Result<Value> {
            let Request::Http(request) = request else {
                bail!("unexpected local request");
            };
            self.requests.lock().unwrap().push(request.clone());
            let url = request["url"].as_str().unwrap();
            let plugin = json!({
                "id": "writing-plugin", "name": "writing", "canonical_app_id": WRITING_APP,
                "status": "ENABLED", "installation_policy": "AVAILABLE",
                "release": {"display_name": "Writing style"},
                "enabled": self.enabled.load(Ordering::SeqCst)
            });
            let (status, body) = if url.ends_with("/enable") {
                self.enabled.store(true, Ordering::SeqCst);
                (204, String::new())
            } else if url.starts_with("/ps/plugins/canonical-app/") {
                (200, json!({"plugin_id": "writing-plugin"}).to_string())
            } else if url.starts_with("/ps/plugins/installed?") {
                (
                    200,
                    json!({"plugins": if self.installed { vec![plugin] } else { vec![] },
                    "pagination": {"next_page_token": null}})
                    .to_string(),
                )
            } else if url == "/ps/plugins/writing-plugin" {
                (200, plugin.to_string())
            } else {
                bail!("unexpected plugin request");
            };
            Ok(json!({"response": {"__response": {"status": status, "body": body}}}))
        }
    }

    #[test]
    fn enabling_an_installed_writing_plugin_accepts_an_empty_success_body() {
        let host = WritingHost {
            account: "account",
            installed: true,
            enabled: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
        };
        let previous = read_writing(&host).unwrap();
        let bridge = server(vec![("skills/list", Ok(json!({"data": []})))]);
        let result = write_writing(&host, &bridge, &previous, true, "/project");
        assert!(result.error.is_none());
        assert!(result.preference.unwrap().enabled);
    }

    #[test]
    fn writing_changes_cannot_cross_an_account_switch() {
        let original = WritingHost {
            account: "original",
            installed: true,
            enabled: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
        };
        let previous = read_writing(&original).unwrap();
        let current = WritingHost {
            account: "different",
            installed: true,
            enabled: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
        };
        let result = write_writing(&current, &server(Vec::new()), &previous, true, "/project");
        assert!(result.error.is_some());
        assert!(!current.enabled.load(Ordering::SeqCst));
        assert!(current.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn writing_installation_waits_for_required_browser_authorization() {
        let host = WritingHost {
            account: "account",
            installed: false,
            enabled: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
        };
        let previous = read_writing(&host).unwrap();
        let bridge = server(vec![(
            "plugin/install",
            Ok(json!({
                "authPolicy": "ON_INSTALL",
                "appsNeedingAuth": [{"id": "mail", "name": "Mail", "installUrl": "https://example.invalid/connect"}]
            })),
        )]);
        let result = write_writing(&host, &bridge, &previous, true, "/project");
        assert!(result.error.is_none());
        assert!(!result.preference.unwrap().enabled);
        assert_eq!(result.authorization.unwrap()[0].app_id, "mail");
    }
}
