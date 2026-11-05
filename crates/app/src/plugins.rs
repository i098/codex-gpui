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
            if matches!(method.as_str(), "skills/changed" | "codex/event/skills_update_available"))
        {
            self.refresh_skills(cx);
        }
        if matches!(event, Event::Notification {method, ..}
            if matches!(method.as_str(), "app/list/updated" | "skills/changed"))
            && let Some(page) = self.plugins_page.clone()
            && let Route::Plugins(route) = &self.state.route
            && should_reload_plugins(&page.read(cx).data.plugins)
        {
            self.load_plugins(page, route.clone(), false, cx);
        }
    }

    pub(crate) fn set_skills(&mut self, skills: Value, cx: &mut Context<Self>) {
        let mut composers = self.thread_composers();
        composers.push(self.home.read(cx).composer.clone());
        for composer in composers {
            composer.update(cx, |composer, cx| composer.set_slash_skills(&skills, cx));
        }
        self.skills = skills;
    }

    pub(crate) fn refresh_skills(&self, cx: &mut Context<Self>) {
        if self.bridge.is_acp() {
            return;
        }
        let bridge = self.bridge.clone();
        let cwd = self.cwd.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let skills = bridge
                .request("skills/list", json!({"cwds": [&cwd], "forceReload": true}))
                .await?;
            this.update(cx, |this, cx| {
                if this.cwd != cwd {
                    return;
                }
                this.set_skills(skills, cx);
            })?;
            Ok(())
        });
        log_err(task, cx);
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
        self.load_plugins(page.clone(), route, false, cx);
        self.plugins_page = Some(page.clone());
        let nav = cx.new(|cx| plugins::Nav::new(page.clone(), cx)).into();
        cx.new(|_| {
            let mut shell = Shell::new(nav, page.into());
            shell.selected_rail = Some("plugins");
            shell.new_tab = false;
            shell
        })
    }

    /// Starts the page's three reads at once and shows each as it arrives, so Skills does not
    /// wait for the slow remote plugin catalog or app list. `force` refetches the remote
    /// catalogs (~40 s each when signed in); opening a page reads the app-server's cache, as
    /// the original does.
    fn load_plugins(&self, page: Entity<Page>, route: String, force: bool, cx: &mut Context<Self>) {
        let personal_marketplace = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".agents/plugins/marketplace.json"));
        let (generation, available) = page.update(cx, |page, cx| {
            page.generation += 1;
            page.data = Data {
                personal_marketplace,
                ..Data::default()
            };
            page.error = None;
            cx.notify();
            (page.generation, page.available)
        });
        let (bridge, cwd) = (self.bridge.clone(), self.cwd.clone());
        let skills = cx.background_spawn(async move { read_skills(&bridge, &cwd).await });
        let bridge = self.bridge.clone();
        let connecting_app_id = page.read(cx).modal.as_ref().and_then(|modal| match modal {
            Modal::Connecting(app) => app["id"].as_str().map(str::to_owned),
            _ => None,
        });
        let apps = cx.background_spawn(async move {
            let apps = read_apps(&bridge, force).await;
            let connected_app_id = connecting_app_id.filter(|id| {
                apps.value()
                    .into_iter()
                    .flatten()
                    .any(|app| app["id"].as_str() == Some(id) && app["isAccessible"] == true)
            });
            let reconciliation_error = if connected_app_id.is_some() {
                reconcile_plugin_suggestions(&bridge)
                    .await
                    .err()
                    .map(|error| error.to_string())
            } else {
                None
            };
            (apps, connected_app_id, reconciliation_error)
        });
        let (bridge, cwd) = (self.bridge.clone(), self.cwd.clone());
        let plugins = cx.background_spawn(async move {
            if !available {
                return Resource::Ready(Vec::new());
            }
            match bridge
                .request::<_, Value>("plugin/list", json!({"cwds":[cwd],"forceRefetch":force}))
                .await
            {
                Ok(value) => Resource::Ready(plugins::marketplaces(&value)),
                Err(error) => Resource::Failed(error.to_string()),
            }
        });
        let skills_page = page.clone();
        cx.spawn(async move |_, cx| {
            let skills = skills.await;
            skills_page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.data.skills = skills;
                    cx.notify();
                }
            })
        })
        .detach();
        let apps_page = page.clone();
        cx.spawn(async move |_, cx| {
            let (apps, connected_app_id, reconciliation_error) = apps.await;
            apps_page.update(cx, |page, cx| {
                if page.generation != generation {
                    return;
                }
                if connected_app_id.as_deref().is_some_and(|id| {
                    matches!(
                        &page.modal,
                        Some(Modal::Connecting(app)) if app["id"].as_str() == Some(id)
                    )
                }) {
                    if let Some(error) = reconciliation_error {
                        page.error =
                            Some(format!("Could not reconcile plugin suggestions: {error}"));
                    } else {
                        page.modal = None;
                    }
                }
                page.data.apps = apps;
                cx.notify();
            })
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let plugins = plugins.await;
            let _ = this.update(cx, |this, cx| {
                if page.read(cx).generation != generation {
                    return;
                }
                let selected = route_plugin_id(&route).and_then(|id| {
                    plugins
                        .value()
                        .into_iter()
                        .flatten()
                        .flat_map(|market| &market.plugins)
                        .find(|plugin| plugin.id() == id)
                        .cloned()
                });
                page.update(cx, |page, cx| {
                    page.data.plugins = plugins;
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
        let skill = page.read(cx).modal.as_ref().and_then(|modal| match modal {
            Modal::Skill(skill) => Some(skill.clone()),
            _ => None,
        });
        let remote_params = skill.as_ref().map(remote_skill_request_params).transpose();
        let remote = matches!(&remote_params, Ok(Some(_)));
        let display_path = if remote {
            PathBuf::from(&self.cwd)
                .join(".remote-plugin-skill/SKILL.md")
                .to_string_lossy()
                .into_owned()
        } else {
            path.clone()
        };
        let view = cx.new(|_| {
            let mut view = FileView::new(&self.cwd, display_path, None);
            view.set_local_controls(!remote);
            view
        });
        page.update(cx, |page, cx| {
            page.skill_file = Some(view.clone());
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let read = cx.background_spawn(async move {
            match remote_params {
                Ok(Some(params)) => {
                    let result: Value = bridge.request("plugin/skill/read", params).await?;
                    result["skill_md_contents"]
                        .as_str()
                        .map(str::to_owned)
                        .context("plugin/skill/read returned no skill markdown")
                }
                Ok(None) => std::fs::read_to_string(&path).context("Could not read skill markdown"),
                Err(error) => Err(error),
            }
        });
        cx.spawn(async move |_, cx| {
            let result = read.await.map_err(|error| error.to_string());
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
        let personal = match action {
            Action::Share(plugin)
            | Action::CreateShare(plugin)
            | Action::UpdateSharedVersion(plugin)
            | Action::UpdateShareTargets(plugin)
            | Action::DeleteShare(plugin)
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
            _ => self.manage_plugin_action(page, action, window, cx),
        }
    }

    fn manage_plugin_action(
        &mut self,
        page: Entity<Page>,
        action: &Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Action::Share(plugin) => {
                let has_share = share_remote_plugin_id(plugin).is_ok();
                page.update(cx, |page, cx| {
                    page.share_url = page
                        .detail_data
                        .value()
                        .and_then(|detail| detail["shareUrl"].as_str())
                        .map(str::to_owned);
                    page.show_modal(Modal::Share(plugin.clone()), window, cx);
                });
                if has_share {
                    self.load_share_targets(page, plugin.clone(), cx);
                }
            }
            Action::UpdateShareTargets(plugin) => {
                self.update_share_targets(page, plugin.clone(), cx);
            }
            Action::DeleteShare(plugin) => self.delete_share(page, plugin.clone(), window, cx),
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
                true,
                cx,
            ),
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
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                if matches!(action, Action::EnableSkill { .. }) {
                    this.refresh_skills(cx);
                }
                let copy_link = matches!(action, Action::CreateShare(_));
                match action {
                    Action::CreateShare(plugin) | Action::UpdateSharedVersion(plugin) => {
                        let mut published = (*plugin).clone();
                        if !published.summary["shareContext"].is_object() {
                            published.summary["shareContext"] = json!({});
                        }
                        published.summary["shareContext"]["remotePluginId"] =
                            value["remotePluginId"].clone();
                        let published = Arc::new(published);
                        page.update(cx, |page, cx| {
                            page.share_url = value["shareUrl"].as_str().map(str::to_owned);
                            if copy_link && let Some(url) = &page.share_url {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                            }
                            if let Resource::Ready(detail) = &mut page.detail_data {
                                detail["shareUrl"] = value["shareUrl"].clone();
                                detail["summary"] = published.summary.clone();
                            }
                            page.detail = Some(published.clone());
                            page.modal = Some(Modal::Share(published.clone()));
                            cx.notify();
                        });
                        if copy_link {
                            this.load_share_targets(page.clone(), published, cx);
                        }
                    }
                    Action::Install(plugin) => {
                        let apps = plugins::array(&value, "appsNeedingAuth").to_vec();
                        let route = format!("/plugins/{}", plugin.id());
                        this.load_plugins(page.clone(), route, true, cx);
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
                        this.load_plugins(page, route, true, cx);
                    }
                }
            });
        })
        .detach();
    }
    fn load_share_targets(&self, page: Entity<Page>, plugin: Arc<Plugin>, cx: &mut Context<Self>) {
        let remote_plugin_id = match share_remote_plugin_id(&plugin) {
            Ok(id) => id,
            Err(error) => {
                page.update(cx, |page, cx| {
                    page.error = Some(error.to_string());
                    cx.notify();
                });
                return;
            }
        };
        page.update(cx, |page, cx| {
            page.pending = true;
            page.error = None;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let task = cx.background_spawn(async move {
            let response: Value = bridge.request("plugin/share/list", json!({})).await?;
            let share = plugins::array(&response, "data")
                .iter()
                .find(|share| share["remotePluginId"] == remote_plugin_id);
            Ok::<_, anyhow::Error>((
                share
                    .and_then(|share| share["discoverability"].as_str())
                    .unwrap_or("PRIVATE")
                    .to_owned(),
                share
                    .and_then(|share| share["sharePrincipals"].as_array())
                    .map(|targets| visible_share_targets(targets))
                    .unwrap_or_default(),
                share
                    .and_then(|share| share["shareUrl"].as_str())
                    .map(str::to_owned),
            ))
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            page.update(cx, |page, cx| {
                page.pending = false;
                match result {
                    Ok((discoverability, targets, share_url)) => {
                        page.share_discoverability = discoverability;
                        page.share_targets = targets;
                        if let Some(share_url) = share_url {
                            page.share_url = Some(share_url);
                        }
                    }
                    Err(error) => page.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_share_targets(
        &self,
        page: Entity<Page>,
        plugin: Arc<Plugin>,
        cx: &mut Context<Self>,
    ) {
        if page.read(cx).pending || page.read(cx).error.is_some() {
            return;
        }
        let remote_plugin_id = match share_remote_plugin_id(&plugin) {
            Ok(id) => id,
            Err(error) => {
                page.update(cx, |page, cx| {
                    page.error = Some(error.to_string());
                    cx.notify();
                });
                return;
            }
        };
        let (discoverability, share_targets) = {
            let state = page.read(cx);
            (
                state.share_discoverability.clone(),
                state.share_targets.clone(),
            )
        };
        let share_targets = match share_target_params(&share_targets) {
            Ok(targets) => targets,
            Err(error) => {
                page.update(cx, |page, cx| {
                    page.error = Some(error.to_string());
                    cx.notify();
                });
                return;
            }
        };
        page.update(cx, |page, cx| {
            page.pending = true;
            page.error = None;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let task = cx.background_spawn(async move {
            bridge
                .request::<_, Value>(
                    "plugin/share/updateTargets",
                    json!({
                        "remotePluginId": remote_plugin_id,
                        "discoverability": discoverability,
                        "shareTargets": share_targets,
                    }),
                )
                .await
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            page.update(cx, |page, cx| {
                page.pending = false;
                match result {
                    Ok(response) => {
                        if let Some(principals) = response["principals"].as_array() {
                            page.share_targets = visible_share_targets(principals);
                        }
                    }
                    Err(error) => page.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_share(
        &self,
        page: Entity<Page>,
        plugin: Arc<Plugin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if page.read(cx).pending {
            return;
        }
        let remote_plugin_id = match share_remote_plugin_id(&plugin) {
            Ok(id) => id,
            Err(error) => {
                page.update(cx, |page, cx| {
                    page.error = Some(error.to_string());
                    cx.notify();
                });
                return;
            }
        };
        let plugin_id = plugin.id().to_owned();
        page.update(cx, |page, cx| {
            page.pending = true;
            page.error = None;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let window = window.window_handle();
        let task = cx.background_spawn(async move {
            bridge
                .request::<_, Value>(
                    "plugin/share/delete",
                    json!({"remotePluginId": remote_plugin_id}),
                )
                .await
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = cx.update_window(window, |_, window, cx| {
                page.update(cx, |page, cx| {
                    page.pending = false;
                    match result {
                        Ok(_) => {
                            page.share_url = None;
                            page.share_targets.clear();
                            page.share_discoverability = "PRIVATE".into();
                            if let Some(detail) = page
                                .detail
                                .as_ref()
                                .filter(|detail| detail.id() == plugin_id)
                            {
                                let mut detail = (**detail).clone();
                                detail.summary["shareContext"] = Value::Null;
                                page.detail = Some(Arc::new(detail));
                            }
                            if let Resource::Ready(detail) = &mut page.detail_data {
                                detail["shareUrl"] = Value::Null;
                                detail["summary"]["shareContext"] = Value::Null;
                            }
                            page.close_modal(window, cx);
                        }
                        Err(error) => {
                            page.error = Some(error.to_string());
                            cx.notify();
                        }
                    }
                });
            });
        })
        .detach();
    }
}

fn share_remote_plugin_id(plugin: &Plugin) -> anyhow::Result<String> {
    plugin.summary["shareContext"]["remotePluginId"]
        .as_str()
        .map(str::to_owned)
        .context("Create a share link before managing plugin access")
}
fn visible_share_targets(targets: &[Value]) -> Vec<Value> {
    targets
        .iter()
        .filter(|target| {
            target["principal_type"]
                .as_str()
                .or_else(|| target["principalType"].as_str())
                != Some("workspace")
        })
        .map(|target| {
            json!({
                "principal_id": target.get("principal_id")
                    .or_else(|| target.get("principalId"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "principal_type": target.get("principal_type")
                    .or_else(|| target.get("principalType"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "role": target["role"],
                "name": target["name"],
            })
        })
        .collect()
}

fn share_target_params(targets: &[Value]) -> anyhow::Result<Vec<Value>> {
    targets
        .iter()
        .map(|target| {
            let principal_id = target
                .get("principal_id")
                .or_else(|| target.get("principalId"))
                .and_then(Value::as_str)
                .context("Plugin share target is missing its principal ID")?;
            let principal_type = target
                .get("principal_type")
                .or_else(|| target.get("principalType"))
                .and_then(Value::as_str)
                .context("Plugin share target is missing its principal type")?;
            Ok(json!({
                "principalId": principal_id,
                "principalType": principal_type,
                "role": target["role"].as_str().unwrap_or("reader"),
            }))
        })
        .collect()
}
fn route_plugin_id(route: &str) -> Option<&str> {
    let path = route.split('?').next().unwrap_or(route);
    let path = path
        .strip_prefix("/skills/manage/plugins/")
        .or_else(|| path.strip_prefix("/skills/plugins/"))
        .or_else(|| path.strip_prefix("/plugins/"))?;
    path.split('/').next().filter(|id| !id.is_empty())
}

/// The original's `plugin/read` and `plugin/install` params: a remote catalog names the plugin
/// by its `remotePluginId`, which the ChatGPT backend's `/ps/plugins/{id}` routes expect.
fn plugin_params(plugin: &Plugin) -> Value {
    match &plugin.marketplace_path {
        Some(path) => json!({"marketplacePath": path, "pluginName": plugin.name()}),
        None => json!({
            "pluginName": plugin.summary["remotePluginId"].as_str().unwrap_or(plugin.name()),
            "remoteMarketplaceName": plugin.remote_marketplace_name,
        }),
    }
}

/// The original's `requestPluginId`: a remote catalog entry uninstalls by its `remotePluginId`;
/// its catalog `id` returns success without removing the plugin.
fn request_plugin_id(plugin: &Plugin) -> &str {
    match &plugin.marketplace_path {
        Some(_) => plugin.id(),
        None => plugin.summary["remotePluginId"]
            .as_str()
            .unwrap_or(plugin.id()),
    }
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
fn remote_skill_request_params(skill: &Value) -> Result<Option<Value>> {
    let nested = skill
        .get("remotePluginSkill")
        .filter(|remote| !remote.is_null());
    let remote = if let Some(remote) = nested {
        remote
            .as_object()
            .context("Remote plugin skill metadata is invalid")?;
        remote
    } else {
        skill
    };
    let is_remote = nested.is_some()
        || !remote["remotePluginId"].is_null()
        || !remote["remoteMarketplaceName"].is_null();
    if !is_remote {
        return Ok(None);
    }
    let remote_plugin_id = remote["remotePluginId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .context("Remote plugin skill has no plugin ID")?;
    let skill_name = remote["skillName"]
        .as_str()
        .filter(|value| !value.is_empty())
        .context("Remote plugin skill has no skill name")?;
    if nested.is_some() {
        return Ok(Some(remote.clone()));
    }
    let mut params = json!({
        "remotePluginId": remote_plugin_id,
        "skillName": skill_name,
    });
    if let Some(marketplace_name) = remote.get("remoteMarketplaceName") {
        params["remoteMarketplaceName"] = marketplace_name.clone();
    }
    Ok(Some(params))
}

async fn reconcile_plugin_suggestions(bridge: &Bridge) -> Result<()> {
    let result: Value = bridge
        .request(
            "plugin/reconcile",
            json!({"reason":"plugin_suggestion_connected"}),
        )
        .await?;
    reconcile_plugin_suggestions_result(&result)
}

fn reconcile_plugin_suggestions_result(result: &Value) -> Result<()> {
    let failed_plugins = result["failedRemotePluginIds"]
        .as_array()
        .context("plugin/reconcile returned no failedRemotePluginIds")?;
    let failed_materializations = result["failedMaterializationRemotePluginIds"]
        .as_array()
        .context("plugin/reconcile returned no failedMaterializationRemotePluginIds")?;
    ensure!(
        failed_plugins.is_empty() && failed_materializations.is_empty(),
        "plugin/reconcile could not complete all plugin suggestions"
    );
    Ok(())
}

async fn read_apps(bridge: &Bridge, force: bool) -> Resource<Vec<Value>> {
    let result = async {
        let mut apps = Vec::new();
        let mut cursor = Value::Null;
        loop {
            // Each forced page refetches the whole catalog (~30 s); later pages read that cache.
            let value: Value = bridge
                .request(
                    "app/list",
                    json!({"cursor":cursor,"limit":1000,"forceRefetch":force && cursor.is_null()}),
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
                .request(
                    "plugin/uninstall",
                    json!({"pluginId":request_plugin_id(plugin)}),
                )
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

fn should_reload_plugins(plugins: &Resource<Vec<plugins::Marketplace>>) -> bool {
    !matches!(plugins, Resource::Loading)
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
    fn app_list_updated_during_loading_does_not_reload_plugins() {
        assert!(!should_reload_plugins(&Resource::Loading));
        assert!(should_reload_plugins(&Resource::Ready(Vec::new())));
    }
    #[test]
    fn remote_skill_request_uses_remote_plugin_metadata() {
        let skill = json!({
            "remotePluginSkill": {
                "remoteMarketplaceName": "openai-curated",
                "remotePluginId": "remote-plugin",
                "skillName": "linear",
            }
        });
        assert_eq!(
            remote_skill_request_params(&skill).unwrap(),
            Some(json!({
                "remoteMarketplaceName": "openai-curated",
                "remotePluginId": "remote-plugin",
                "skillName": "linear",
            }))
        );
        assert_eq!(
            remote_skill_request_params(&json!({"path":"/tmp/SKILL.md"})).unwrap(),
            None
        );
        assert!(remote_skill_request_params(&json!({"remotePluginId":"remote-plugin"})).is_err());
    }

    #[test]
    fn reconciliation_rejects_partial_plugin_suggestion_failures() {
        assert!(
            reconcile_plugin_suggestions_result(&json!({
                "failedRemotePluginIds": [],
                "failedMaterializationRemotePluginIds": [],
            }))
            .is_ok()
        );
        assert!(
            reconcile_plugin_suggestions_result(&json!({
                "failedRemotePluginIds": ["remote-plugin"],
                "failedMaterializationRemotePluginIds": [],
            }))
            .is_err()
        );
        assert!(reconcile_plugin_suggestions_result(&json!({})).is_err());
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
}
