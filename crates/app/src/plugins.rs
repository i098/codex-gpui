use crate::*;
use ui::screens::plugins::{self, Action, Data, Modal, Page, Plugin, Resource};

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
        let mut params = HashMap::new();
        if let Ok(url) = url::Url::parse(&format!("https://codex.invalid{route}")) {
            for (key, value) in url.query_pairs() {
                params
                    .entry(key.into_owned())
                    .or_insert_with(|| value.into_owned());
            }
        }
        let host_id = params.remove("hostId").unwrap_or_else(|| "local".into());
        let local_host = host_id == "local";
        let available =
            !self.bridge.is_acp() && self.config["features"]["plugins"] != false && local_host;
        if !self.bridge.is_acp()
            && local_host
            && cx
                .try_global::<ui::screens::settings::integrations::State>()
                .is_none_or(|state| !state.gates_loaded)
        {
            self.load_settings_integrations(cx);
        }
        let page = cx.new(|cx| Page::new(&route, available, cx));
        page.update(cx, |page, cx| {
            page.host_id = host_id;
            page.directory = match params
                .remove("directoryTab")
                .or_else(|| params.remove("directory"))
                .as_deref()
            {
                Some("personal") => plugins::Directory::Personal,
                _ => plugins::Directory::Public,
            };
            page.category = params.remove("category");
            page.marketplace_path = params.remove("marketplacePath");
            page.share_on_load = params.remove("mode").as_deref() == Some("share");
            if let Some(query) = params.remove("search") {
                page.query
                    .update(cx, |editor, cx| editor.set_text(query.trim(), cx));
            }
        });
        cx.subscribe_in(&page, window, |this, page, action, window, cx| {
            this.plugin_action(page.clone(), action, window, cx);
        })
        .detach();
        self.load_plugins(page.clone(), route, cx);
        self.plugins_page = Some(page.clone());
        let sidebar = cx.new(|cx| plugins::Sidebar::new(page.clone(), cx)).into();
        cx.new(|_| {
            let mut shell = Shell::new(sidebar, page.into());
            shell.selected_rail = Some("plugins");
            shell.sidebar_open = true;
            shell.chat_chrome = true;
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
        if page.read(cx).host_id != "local" {
            page.update(cx, |page, cx| {
                page.data = Data {
                    plugins: Resource::Failed("Plugins are not available for this host".into()),
                    skills: Resource::Failed("Skills are not available for this host".into()),
                    apps: Resource::Failed("Apps are not available for this host".into()),
                    personal_marketplace: None,
                };
                cx.notify();
            });
            return;
        }
        let bridge = self.bridge.clone();
        let cwd = self.cwd.clone();
        let personal_marketplace = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".agents/plugins/marketplace.json"));
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
                personal_marketplace,
            }
        });
        cx.spawn(async move |this, cx| {
            let data = task.await;
            let _ =
                this.update(cx, |this, cx| {
                    if page.read(cx).generation != generation {
                        return;
                    }
                    let marketplace = page.read(cx).marketplace_path.as_deref();
                    let selected = route_plugin_id(&route).and_then(|id| {
                        data.plugins
                            .value()
                            .into_iter()
                            .flatten()
                            .flat_map(|market| &market.plugins)
                            .find(|plugin| {
                                plugin.id() == id
                                    && marketplace.is_none_or(|path| {
                                        plugin.marketplace_path.as_deref() == Some(path)
                                    })
                            })
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
                        if let Some(Modal::Skill(skill)) = &mut page.modal
                            && let Some(updated) = data.skills.value().and_then(|skills| {
                                skills
                                    .iter()
                                    .find(|updated| updated["path"] == skill["path"])
                            })
                        {
                            *skill = updated.clone();
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
            let _ = page.update(cx, |page, cx| {
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
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            (|| -> Result<String> {
                let response = host.fetch(&host::Request::post(
                    "read-file",
                    Some(json!({"hostId":"local","path":path})),
                ))?;
                let value = host::json_body(&response)?;
                Ok(value["contents"]
                    .as_str()
                    .context("Skill file response has no contents")?
                    .to_owned())
            })()
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            let _ = view.update(cx, |view, cx| view.set_contents(result, cx));
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

    fn load_recommended_skills(&self, page: Entity<Page>, refresh: bool, cx: &mut Context<Self>) {
        if page.read(cx).pending {
            return;
        }
        page.update(cx, |page, cx| {
            page.recommended = Resource::Loading;
            page.recommended_error = None;
            page.pending = true;
            cx.notify();
        });
        let host = self.host.clone();
        let load = cx.background_spawn(async move {
            let response = host.fetch(&host::Request::post(
                "recommended-skills",
                Some(json!({"hostId":"local","refresh":refresh})),
            ))?;
            let mut value = host::json_body(&response)?;
            let root = value["repoRoot"].as_str().map(PathBuf::from);
            let warning = value["error"].as_str().map(str::to_owned);
            let Value::Array(mut skills) = value["skills"].take() else {
                anyhow::bail!("Recommended skill response has no skills");
            };
            if let Some(root) = root {
                for skill in &mut skills {
                    if let Some(relative) = skill["repoPath"].as_str() {
                        let mut path = root.join(relative);
                        if !relative.to_lowercase().ends_with(".md") {
                            path.push("SKILL.md");
                        }
                        skill["path"] = json!(path);
                    }
                }
            }
            Ok::<_, anyhow::Error>((skills, warning))
        });
        cx.spawn(async move |_, cx| {
            let result = load.await;
            let _ = page.update(cx, |page, cx| {
                page.pending = false;
                page.recommended = match result {
                    Ok((skills, warning)) => {
                        page.recommended_error = warning;
                        Resource::Ready(skills)
                    }
                    Err(error) => Resource::Failed(error.to_string()),
                };
                cx.notify();
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
        let local_host = page.read(cx).host_id == "local";
        if !local_host
            && !matches!(
                action,
                Action::Navigate(_) | Action::SelectDirectory(_) | Action::Refresh
            )
        {
            page.update(cx, |page, cx| {
                page.error = Some("Plugins are not available for this host".into());
                cx.notify();
            });
            return;
        }
        let personal = match action {
            Action::Share(plugin)
            | Action::CreateShare(plugin)
            | Action::UpdateSharedVersion(plugin)
            | Action::EditPlugin(plugin) => Some(plugin),
            _ => None,
        };
        if personal.is_some_and(|plugin| page.read(cx).data.personal_path(plugin).is_none()) {
            page.update(cx, |page, cx| {
                page.error = Some(
                    "Only plugins from your personal marketplace can be edited or shared".into(),
                );
                cx.notify();
            });
            return;
        }
        match action {
            Action::Share(plugin) => {
                page.update(cx, |page, cx| {
                    page.share_url = page
                        .detail_data
                        .value()
                        .and_then(|detail| detail["shareUrl"].as_str())
                        .map(str::to_owned);
                    page.show_modal(Modal::Share(plugin.clone()), window, cx);
                });
            }
            Action::Manage => self.open_settings_section("Plugins", window, cx),
            Action::Navigate(route) => {
                if local_host {
                    self.open_plugins(route, window, cx);
                } else {
                    match route_query(route, "hostId", &page.read(cx).host_id) {
                        Ok(route) => {
                            self.open_plugins(&route, window, cx);
                        }
                        Err(error) => self.toast(Toast::danger(error.to_string()), cx),
                    }
                }
            }
            Action::SelectDirectory(directory) => {
                if let Route::Plugins(route) = &self.state.route {
                    match route_query(route, "directoryTab", directory.query_value()) {
                        Ok(route) => self.navigate(Route::Plugins(route), window, cx),
                        Err(error) => self.toast(Toast::danger(error.to_string()), cx),
                    }
                }
            }
            Action::CreatePlugin => self.open_plugin_creator(window, cx),
            Action::ReadRecommended => self.load_recommended_skills(page, false, cx),
            Action::Read(plugin) => self.navigate(
                Route::Plugins(format!("/plugins/{}", plugin.id())),
                window,
                cx,
            ),
            Action::Refresh => {
                if local_host {
                    self.load_settings_integrations(cx);
                    if page.read(cx).skill_section == plugins::SkillSection::Recommended {
                        self.load_recommended_skills(page.clone(), true, cx);
                    }
                }
                let route = match &self.state.route {
                    Route::Plugins(route) => route.clone(),
                    _ => "/plugins".into(),
                };
                self.load_plugins(page, route, cx);
            }
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
                    let state = page.read(cx);
                    state.data.can_use(plugin, state.detail_data.value())
                }) else {
                    page.update(cx, |page, cx| {
                        page.error = Some(
                            "Enable this plugin and connect its required apps before trying it"
                                .into(),
                        );
                        cx.notify();
                    });
                    return;
                };
                let prompts = &plugin.summary["interface"]["defaultPrompt"];
                let prompt = prompts
                    .as_str()
                    .or_else(|| prompts.as_array()?.first()?.as_str())
                    .unwrap_or("");
                self.plugin_prefill(
                    ui::composer::DraftMention {
                        label: plugin.title(),
                        link: format!("[@{}](plugin://{})", plugin.title(), plugin.id()),
                        icon: "sidebar-plugins",
                    },
                    prompt,
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
                    .map(|skill| {
                        (
                            plugins::string(skill, "path").to_owned(),
                            plugins::skill_title(skill),
                        )
                    });
                if let Some((path, label)) = creator {
                    if cx.try_global::<host_settings::Settings>().is_none() {
                        cx.set_global(host_settings::Settings::default());
                    }
                    let seen = cx
                        .try_global::<host_settings::Settings>()
                        .and_then(|settings| {
                            settings
                                .read_all()
                                .get("has-opened-skill-creator-prefill-v1")
                        })
                        .and_then(Value::as_bool)
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
                    self.plugin_prefill(
                        ui::composer::DraftMention {
                            label: &label,
                            link: format!("[$skill-creator]({path})"),
                            icon: "skill-mention",
                        },
                        starter,
                        window,
                        cx,
                    );
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

    pub(crate) fn plugin_prefill(
        &mut self,
        mention: ui::composer::DraftMention<'_>,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_chat(window, cx);
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, cx| {
            composer.set_mention_draft(mention, prompt, cx)
        });
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
                let copy_link = matches!(action, Action::CreateShare(_));
                let preserve_skill = matches!(action, Action::EnableSkill { .. });
                match action {
                    Action::CreateShare(plugin) | Action::UpdateSharedVersion(plugin) => {
                        page.update(cx, |page, cx| {
                            page.share_url = value["shareUrl"].as_str().map(str::to_owned);
                            if copy_link && let Some(url) = &page.share_url {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                            }
                            let mut published = (*plugin).clone();
                            if !published.summary["shareContext"].is_object() {
                                published.summary["shareContext"] = json!({});
                            }
                            published.summary["shareContext"]["remotePluginId"] =
                                value["remotePluginId"].clone();
                            if let Resource::Ready(detail) = &mut page.detail_data {
                                detail["shareUrl"] = value["shareUrl"].clone();
                                detail["summary"] = published.summary.clone();
                            }
                            let published = Arc::new(published);
                            page.detail = Some(published.clone());
                            page.modal = Some(Modal::Share(published));
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
                            if !preserve_skill {
                                page.modal = None;
                            }
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
        Ok(value) => Resource::Ready(plugins::standalone_skills(
            plugins::array(&value, "data")
                .iter()
                .flat_map(|entry| plugins::array(entry, "skills").iter().cloned()),
        )),
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
        Action::InstallRecommended(skill) => {
            let id = skill["id"]
                .as_str()
                .context("Recommended skill has no ID")?;
            let repo = skill["repoPath"]
                .as_str()
                .context("Recommended skill has no repository path")?;
            let response = host.fetch(&host::Request::post(
                "install-recommended-skill",
                Some(json!({"hostId":"local","skillId":id,"repoPath":repo,"installRoot":null})),
            ))?;
            let value = host::json_body(&response)?;
            ensure!(
                value["success"] == true,
                "{}",
                value["error"]
                    .as_str()
                    .unwrap_or("Recommended skill installation failed")
            );
            Ok(value)
        }
        Action::CreateShare(plugin) | Action::UpdateSharedVersion(plugin) => {
            let path = plugin
                .local_path()
                .context("Only a local plugin can publish a shared version")?;
            let remote_id = &plugin.summary["shareContext"]["remotePluginId"];
            if matches!(action, Action::UpdateSharedVersion(_)) {
                ensure!(
                    remote_id.is_string(),
                    "Create a share link before updating its version"
                );
            }
            let result: Value = bridge
                .request(
                    "plugin/share/save",
                    json!({"pluginPath":path,"remotePluginId":remote_id}),
                )
                .await?;
            ensure!(
                result["remotePluginId"].is_string()
                    && result["shareUrl"]
                        .as_str()
                        .is_some_and(|url| !url.is_empty()),
                "The server returned an incomplete plugin share"
            );
            Ok(result)
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
                "open-file",
                Some(json!({
                    "cwd":root.path(), "path":path,
                })),
            )
        };
        let opened = host::Host::fetch(&host, &request(&plugin))?;
        let body: Value =
            serde_json::from_str(opened["bodyJsonString"].as_str().context("open response")?)?;
        assert_eq!(body["path"], json!(plugin.canonicalize()?));
        let escape = root.path().join("escape");
        symlink(outside.path(), &escape)?;
        assert!(
            host::Host::fetch(&host, &request(&escape))
                .unwrap_err()
                .to_string()
                .contains("outside the project")
        );
        Ok(())
    }
}

fn route_query(route: &str, key: &str, value: &str) -> Result<String> {
    let mut url = url::Url::parse(&format!("https://codex.invalid{route}"))?;
    let mut pairs: Vec<_> = url
        .query_pairs()
        .filter(|(name, _)| name != key)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    pairs.push((key.to_owned(), value.to_owned()));
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(pairs);
    Ok(url[url::Position::BeforePath..].to_owned())
}
