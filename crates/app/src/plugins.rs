use crate::*;
use ui::screens::plugins::{self, Action, Data, Modal, Page, Plugin, Resource};

pub(crate) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|root, _: &ui::OpenPlugins, window, cx| {
        root.open_plugins("/plugins", window, cx);
    }))
}

impl Root {
    pub fn plugins_page(&self) -> Option<&Entity<Page>> {
        self.plugins_page.as_ref()
    }

    /// Opens a plugin-family path and returns whether this family owns it.
    pub fn open_plugins(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let pathname = path.split('?').next().unwrap_or(path);
        if pathname == "/skills/manage" {
            self.open_settings_section("Plugins", window, cx);
            return true;
        }
        if !matches!(pathname, "/plugins" | "/skills")
            && !pathname.starts_with("/plugins/")
            && !pathname.starts_with("/skills/")
        {
            return false;
        }
        let path = path
            .replacen("/skills/manage/plugins/", "/plugins/", 1)
            .replacen("/skills/plugins/", "/plugins/", 1);
        self.navigate(Route::Plugins(path), window, cx);
        true
    }

    pub(crate) fn plugins_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        if matches!(event, Event::Notification {method, ..}
            if matches!(method.as_str(), "app/list/updated" | "skills/changed"))
            && let Some(page) = self.plugins_page.clone()
            && let Route::Plugins(route) = &self.state.route
        {
            self.load_plugins(page, route.clone(), cx);
        }
    }

    pub(crate) fn plugins_shell(
        &mut self,
        route: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Shell> {
        let available = !self.bridge.is_acp() && self.config["features"]["plugins"] != false;
        let page = cx.new(|cx| Page::new(&route, available, cx));
        cx.subscribe_in(&page, window, |this, page, action, window, cx| {
            this.plugin_action(page.clone(), action, window, cx);
        })
        .detach();
        self.load_plugins(page.clone(), route, cx);
        self.plugins_page = Some(page.clone());
        let sidebar = self.sidebar.clone().into();
        cx.new(|_| {
            let mut shell = Shell::new(sidebar, page.into());
            shell.selected_rail = Some("plugins");
            shell.sidebar_open = false;
            shell.chat_chrome = false;
            shell.new_tab = false;
            shell
        })
    }

    fn load_plugins(&self, page: Entity<Page>, route: String, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.data = Data::default();
            page.error = None;
            cx.notify();
            page.generation
        });
        let bridge = self.bridge.clone();
        let cwd = self.cwd.clone();
        let available = page.read(cx).available;
        let task = cx.background_spawn(async move {
            let plugins = if available {
                match bridge
                    .request::<_, Value>("plugin/list", json!({"cwds":[cwd],"forceRefetch":true}))
                    .await
                {
                    Ok(value) => Resource::Ready(plugins::marketplaces(&value)),
                    Err(error) => Resource::Failed(error.to_string()),
                }
            } else {
                Resource::Ready(Vec::new())
            };
            let skills = read_skills(&bridge, &cwd).await;
            let apps = read_apps(&bridge).await;
            Data {
                plugins,
                skills,
                apps,
            }
        });
        cx.spawn(async move |this, cx| {
            let data = task.await;
            let _ =
                this.update(cx, |this, cx| {
                    if page.read(cx).generation != generation {
                        return;
                    }
                    let selected = route_plugin_id(&route).and_then(|id| {
                        data.plugins
                            .value()
                            .into_iter()
                            .flatten()
                            .flat_map(|market| &market.plugins)
                            .find(|plugin| plugin.id() == id)
                            .cloned()
                    });
                    page.update(cx, |page, cx| {
                        if let Some(Modal::Connecting(app)) = &page.modal
                            && data.apps.value().into_iter().flatten().any(|entry| {
                                entry["id"] == app["id"] && entry["isAccessible"] == true
                            })
                        {
                            page.modal = None;
                        }
                        page.data = data;
                        cx.notify();
                    });
                    if let Some(plugin) = selected {
                        this.read_plugin(page.clone(), plugin, cx);
                    } else if route_plugin_id(&route).is_some() {
                        page.update(cx, |page, cx| {
                            page.error = Some("Plugin not found".into());
                            cx.notify();
                        });
                    }
                });
        })
        .detach();
    }

    fn read_plugin(&self, page: Entity<Page>, plugin: Arc<Plugin>, cx: &mut Context<Self>) {
        page.update(cx, |page, cx| {
            page.detail = Some(plugin.clone());
            page.detail_data = Resource::Loading;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let params = plugin_params(&plugin);
        cx.spawn(async move |_, cx| {
            let result = bridge.request::<_, Value>("plugin/read", params).await;
            page.update(cx, |page, cx| {
                if page
                    .detail
                    .as_ref()
                    .is_none_or(|current| current.id() != plugin.id())
                {
                    return;
                }
                page.detail_data = match result {
                    Ok(value) => Resource::Ready(value["plugin"].clone()),
                    Err(error) => Resource::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn read_skill(&self, page: Entity<Page>, path: String, cx: &mut Context<Self>) {
        let view = cx.new(|_| FileView::new(&self.cwd, path.clone(), None));
        page.update(cx, |page, cx| {
            page.skill_file = Some(view.clone());
            cx.notify();
        });
        let read = cx.background_spawn(async move {
            std::fs::read_to_string(&path).map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            view.update(cx, |view, cx| view.set_contents(result, cx));
        })
        .detach();
    }

    fn connect_plugin_app(&self, page: Entity<Page>, app: Value, cx: &mut Context<Self>) {
        page.update(cx, |page, cx| {
            page.pending = true;
            page.error = None;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let operation = cx.background_spawn(async move {
            let app = if app["installUrl"].is_string() {
                app
            } else {
                let id = app["id"].as_str().context("The app has no connector ID")?;
                let response: Value = bridge.request("app/read", json!({"appIds":[id]})).await?;
                plugins::array(&response, "apps")
                    .iter()
                    .find(|app| app["id"] == id)
                    .context("The requested app is unavailable")?
                    .clone()
            };
            Ok::<_, anyhow::Error>((app_connect_url(&app)?, app))
        });
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.plugins_page.as_ref() != Some(&page) {
                    return;
                }
                page.update(cx, |page, cx| {
                    page.pending = false;
                    match result {
                        Ok((url, app)) => {
                            cx.open_url(&url);
                            page.modal = Some(Modal::Connecting(app));
                        }
                        Err(error) => page.error = Some(error.to_string()),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn plugin_action(
        &mut self,
        page: Entity<Page>,
        action: &Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Action::Manage => self.open_settings_section("Plugins", window, cx),
            Action::Navigate(route) => {
                self.open_plugins(route, window, cx);
            }
            Action::Read(plugin) => self.navigate(
                Route::Plugins(format!("/plugins/{}", plugin.id())),
                window,
                cx,
            ),
            Action::Refresh => self.load_plugins(
                page,
                match &self.state.route {
                    Route::Plugins(route) => route.clone(),
                    _ => "/plugins".into(),
                },
                cx,
            ),
            Action::TryPlugin(plugin) => {
                let current = page
                    .read(cx)
                    .data
                    .plugins
                    .value()
                    .into_iter()
                    .flatten()
                    .flat_map(|market| &market.plugins)
                    .find(|current| current.id() == plugin.id())
                    .cloned();
                let Some(plugin) = current.filter(|plugin| {
                    plugin.installed() && plugin.enabled() && plugin.disabled_reason().is_none()
                }) else {
                    page.update(cx, |page, cx| {
                        page.error = Some("Enable this plugin before trying it".into());
                        cx.notify();
                    });
                    return;
                };
                let prompt = plugin.summary["interface"]["defaultPrompt"]
                    .as_str()
                    .unwrap_or("");
                self.plugin_prefill(
                    &format!("[@{}](plugin://{}) {}", plugin.title(), plugin.id(), prompt),
                    window,
                    cx,
                );
            }
            Action::NewSkill => {
                let creator = page
                    .read(cx)
                    .data
                    .skills
                    .value()
                    .into_iter()
                    .flatten()
                    .find(|skill| skill["name"] == "skill-creator")
                    .map(|skill| plugins::string(skill, "path").to_owned());
                if let Some(path) = creator {
                    if cx.try_global::<host_settings::Settings>().is_none() {
                        cx.set_global(host_settings::Settings::default());
                    }
                    let seen = cx
                        .try_global::<host_settings::Settings>()
                        .and_then(|settings| {
                            settings
                                .read_all()
                                .get("has-opened-skill-creator-prefill-v1")
                                .and_then(Value::as_bool)
                        })
                        .unwrap_or(false);
                    if !seen {
                        let result =
                            cx.update_global::<host_settings::Settings, _>(|settings, _| {
                                settings.write("has-opened-skill-creator-prefill-v1", json!(true))
                            });
                        if let Err(error) = result {
                            page.update(cx, |page, cx| {
                                page.error = Some(error.to_string());
                                cx.notify();
                            });
                            return;
                        }
                    }
                    let starter = if seen { "" } else { "help me create a skill" };
                    self.plugin_prefill(&format!("[$skill-creator]({path}) {starter}"), window, cx);
                } else {
                    page.update(cx, |page, cx| {
                        page.error = Some("The skill-creator skill is not available".into());
                        cx.notify();
                    });
                }
            }
            Action::ReadSkill(path) => self.read_skill(page, path.clone(), cx),
            Action::OpenSkill(path) => {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    page.update(cx, |page, cx| {
                        page.modal = None;
                        cx.notify();
                    });
                    self.open_in_editor(&parent.to_string_lossy(), path, cx);
                }
            }
            Action::EditPlugin(plugin) => {
                if let Some(path) = plugin.local_path() {
                    self.open_in_editor(path, path, cx);
                }
            }
            Action::ConnectApp(app) => self.connect_plugin_app(page, app.clone(), cx),
            _ => self.mutate_plugin(page, action.clone(), cx),
        }
    }

    fn plugin_prefill(&mut self, prompt: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, cx| composer.set_text(prompt, cx));
        window.focus(&composer.focus_handle(cx), cx);
    }

    fn mutate_plugin(&self, page: Entity<Page>, action: Action, cx: &mut Context<Self>) {
        if page.read(cx).pending {
            return;
        }
        page.update(cx, |page, cx| {
            page.pending = true;
            page.error = None;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let operation = cx.background_spawn(async move {
            perform(&bridge, host.as_ref(), &action)
                .await
                .map(|value| (action, value))
        });
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                page.update(cx, |page, cx| {
                    page.pending = false;
                    if let Err(error) = &result {
                        page.error = Some(error.to_string());
                    }
                    cx.notify();
                });
                let Ok((action, value)) = result else {
                    return;
                };
                match action {
                    Action::Share(plugin) | Action::UpdateSharedVersion(plugin) => {
                        page.update(cx, |page, cx| {
                            page.share_url = value["shareUrl"].as_str().map(str::to_owned);
                            let mut published = (*plugin).clone();
                            published.summary["shareContext"] =
                                json!({"remotePluginId":value["remotePluginId"]});
                            page.modal = Some(Modal::Share(Arc::new(published)));
                            cx.notify();
                        });
                    }
                    Action::Install(plugin) => {
                        let apps = plugins::array(&value, "appsNeedingAuth").to_vec();
                        let route = format!("/plugins/{}", plugin.id());
                        this.load_plugins(page.clone(), route, cx);
                        if !apps.is_empty() {
                            page.update(cx, |page, cx| {
                                page.modal = Some(Modal::Connections { plugin, apps });
                                cx.notify();
                            });
                        } else {
                            page.update(cx, |page, cx| {
                                page.modal = None;
                                cx.notify();
                            });
                        }
                    }
                    _ => {
                        page.update(cx, |page, cx| {
                            page.modal = None;
                            cx.notify();
                        });
                        let route = match &this.state.route {
                            Route::Plugins(route) => route.clone(),
                            _ => "/plugins".into(),
                        };
                        this.load_plugins(page, route, cx);
                    }
                }
            });
        })
        .detach();
    }
}

fn route_plugin_id(route: &str) -> Option<&str> {
    let path = route.split('?').next().unwrap_or(route);
    let path = path
        .strip_prefix("/skills/manage/plugins/")
        .or_else(|| path.strip_prefix("/skills/plugins/"))
        .or_else(|| path.strip_prefix("/plugins/"))?;
    path.split('/').next().filter(|id| !id.is_empty())
}

fn plugin_params(plugin: &Plugin) -> Value {
    json!({"pluginName":plugin.name(), "marketplacePath":plugin.marketplace_path,
        "remoteMarketplaceName":plugin.remote_marketplace_name})
}

async fn read_skills(bridge: &Bridge, cwd: &str) -> Resource<Vec<Value>> {
    match bridge
        .request::<_, Value>("skills/list", json!({"cwds":[cwd],"forceReload":true}))
        .await
    {
        Ok(value) => Resource::Ready(
            plugins::array(&value, "data")
                .iter()
                .flat_map(|entry| plugins::array(entry, "skills").iter().cloned())
                .collect(),
        ),
        Err(error) => Resource::Failed(error.to_string()),
    }
}

async fn read_apps(bridge: &Bridge) -> Resource<Vec<Value>> {
    let result = async {
        let mut apps = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let value: Value = bridge
                .request(
                    "app/list",
                    json!({"cursor":cursor,"limit":1000,"forceRefetch":true}),
                )
                .await?;
            let data = value["data"].as_array().context("App list has no data")?;
            apps.extend(data.iter().cloned());
            let next = value["nextCursor"].clone();
            if next.is_null() {
                return Ok::<_, anyhow::Error>(apps);
            }
            ensure!(
                next.is_string() && next != cursor,
                "App list returned an invalid cursor"
            );
            cursor = next;
        }
    }
    .await;
    match result {
        Ok(apps) => Resource::Ready(apps),
        Err(error) => Resource::Failed(error.to_string()),
    }
}

async fn write_config(bridge: &Bridge, key: String, value: Value) -> Result<Value> {
    let response = bridge
        .request(
            "config/value/write",
            json!({
                "keyPath":key, "value":value,"mergeStrategy":"upsert","reloadUserConfig":true,
            }),
        )
        .await?;
    bridge
        .request::<_, Value>("config/mcpServer/reload", json!({}))
        .await?;
    Ok(response)
}

async fn perform(bridge: &Bridge, host: &dyn host::Host, action: &Action) -> Result<Value> {
    match action {
        Action::Install(plugin) => {
            ensure!(
                plugin.disabled_reason().is_none(),
                "This plugin is unavailable"
            );
            bridge
                .request("plugin/install", plugin_params(plugin))
                .await
        }
        Action::EnablePlugin(plugin, enabled) => {
            ensure!(
                plugin.disabled_reason().is_none(),
                "This plugin is unavailable"
            );
            write_config(
                bridge,
                format!("plugins.{}.enabled", plugin.id()),
                json!(enabled),
            )
            .await
        }
        Action::Uninstall(plugin) => {
            ensure!(
                plugin.can_uninstall(),
                "This plugin is managed by Codex and cannot be uninstalled"
            );
            bridge
                .request("plugin/uninstall", json!({"pluginId":plugin.id()}))
                .await
        }
        Action::EnableSkill {
            name,
            path,
            enabled,
        } => {
            let params = if name.contains(':') {
                json!({"name":name,"enabled":enabled})
            } else {
                json!({"path":path,"enabled":enabled})
            };
            bridge.request("skills/config/write", params).await
        }
        Action::EnableApp { id, enabled } => {
            write_config(bridge, format!("apps.{id}.enabled"), json!(enabled)).await
        }
        Action::Share(plugin) | Action::UpdateSharedVersion(plugin) => {
            let path = plugin
                .local_path()
                .context("Only a local plugin can publish a shared version")?;
            bridge.request("plugin/share/save", json!({"pluginPath":path,"remotePluginId":plugin.summary["shareContext"]["remotePluginId"]})).await
        }
        Action::DisconnectApp(id) => connector_request(
            host,
            "DELETE",
            "/aip/connectors/links",
            Some(json!({"connector_id":id})),
        ),
        _ => anyhow::bail!("The action does not change plugin state"),
    }
}

fn connector_request(
    host: &dyn host::Host,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let response = host.fetch(&host::Request::Http(json!({
        "url":path,"method":method,"body":body.map(|value| value.to_string()),
        "headers":{"X-OpenAI-Attach-Auth":"true","X-OpenAI-Attach-Desktop-Surface":"true",
            "OAI-Product-Sku":"CODEX"},
    })))?;
    let value = &response["value"];
    ensure!(
        value["responseType"] != "error",
        "{}",
        value["error"].as_str().unwrap_or("App request failed")
    );
    let wire = &value["response"]["__response"];
    let status = wire["status"]
        .as_u64()
        .context("App response has no status")?;
    ensure!(
        (200..300).contains(&status),
        "App request failed with status {status}"
    );
    let body = wire["body"].as_str().unwrap_or("");
    if body.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_str(body)?)
    }
}

fn app_connect_url(app: &Value) -> Result<String> {
    let mut url = url::Url::parse(
        app["installUrl"]
            .as_str()
            .context("This app does not provide a browser setup URL right now")?,
    )?;
    ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "The app setup URL is not secure"
    );
    let id = app["id"].as_str().context("The app has no connector ID")?;
    let fragment = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("connector", id)
        .append_pair("add-connector-link", "true")
        .append_pair("product-sku", "CODEX")
        .append_pair("referrer", "codex")
        .finish();
    url.set_fragment(Some(&format!("settings/Connectors?{fragment}")));
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_urls_reject_unsafe_schemes_and_embedded_credentials() {
        for url in [
            "file:///tmp/secret",
            "javascript:alert(1)",
            "https://user:password@example.com",
        ] {
            assert!(app_connect_url(&json!({"id":"connector","installUrl":url})).is_err());
        }
        let url =
            app_connect_url(&json!({"id":"a&b","installUrl":"https://chatgpt.com/apps"})).unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let fragment = parsed
            .fragment()
            .unwrap()
            .strip_prefix("settings/Connectors?")
            .unwrap();
        let params: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(fragment.as_bytes()).collect();
        assert_eq!(params["connector"], "a&b");
        assert_eq!(params["product-sku"], "CODEX");
    }

    #[test]
    fn server_refusals_do_not_report_a_successful_disconnect() {
        struct RefusingHost;
        impl host::Host for RefusingHost {
            fn fetch(&self, _: &host::Request) -> Result<Value> {
                Ok(json!({"value":{"responseType":"error","status":403,"error":"Forbidden"}}))
            }
        }
        let error = connector_request(
            &RefusingHost,
            "DELETE",
            "/aip/connectors/links",
            Some(json!({"connector_id":"denied"})),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "Forbidden");
    }

    #[cfg(unix)]
    #[test]
    fn plugin_directories_open_without_allowing_symlink_escape() -> Result<()> {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let plugin = root.path().join("plugin");
        std::fs::create_dir(&plugin)?;
        let host = host::LocalHost::new(root.path()).with_opener("/usr/bin/true");
        let request = |path: &std::path::Path| {
            host::Request::post(
                "open-in",
                Some(json!({
                    "hostId":"local", "target":"systemDefault",
                    "cwd":root.path(), "path":path,
                })),
            )
        };
        assert_eq!(
            host::json_body(&host::Host::fetch(&host, &request(&plugin))?)?["success"],
            true
        );
        let escape = root.path().join("escape");
        symlink(outside.path(), &escape)?;
        assert_eq!(
            host::json_body(&host::Host::fetch(&host, &request(&escape))?)?["success"],
            false
        );
        Ok(())
    }
}
