use crate::{Root, Route, chatgpt, host, host_settings};
use anyhow::{Context as _, Result, ensure};
use gpui::{AppContext as _, Context, Entity, PathPromptOptions, Window};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::screens::onboarding::{
    Completion, Onboarding, OnboardingEvent, Page, Project, StarterTask,
};

pub fn route(path: &str) -> Option<Route> {
    let page = match path {
        "/welcome" => Page::Welcome,
        "/onboarding" => Page::Onboarding,
        "/select-workspace" => Page::SelectWorkspace,
        "/first-run" => Page::FirstRun,
        _ => return None,
    };
    Some(Route::Onboarding(page))
}

pub(crate) struct ContextData {
    account: Option<Value>,
    projects: Vec<Project>,
}

fn context_request() -> host::Request {
    host::Request::Http(chatgpt::desktop_request(
        "GET",
        "/wham/onboarding/context",
        None,
    ))
}

fn projects(host: &dyn host::Host) -> Result<Vec<Project>> {
    let mut projects: Vec<_> = host
        .projects()?
        .into_iter()
        .filter(|project| !project.missing)
        .filter_map(|project| {
            Some(Project {
                root: project.root?.to_string(),
                label: project.label.to_string(),
            })
        })
        .collect();
    projects.sort_by(|left, right| left.root.cmp(&right.root));
    projects.dedup_by(|left, right| left.root == right.root);
    Ok(projects)
}

impl Root {
    pub fn onboarding(&self) -> Option<&Entity<Onboarding>> {
        self.onboarding_page.as_ref()
    }

    pub(crate) fn maybe_onboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.chatgpt_account
            || self.state.route != Route::Home
            || self.onboarding_load.is_some()
        {
            return;
        }
        let host = self.host.clone();
        let epoch = self.navigation_epoch;
        self.onboarding_load = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let context = host::json_body(&host.fetch(&context_request())?)?;
                    let workspaces = projects(host.as_ref())?;
                    ensure!(
                        context.get("desktop_onboarding_completed_at").is_some(),
                        "Onboarding context lacks completion state"
                    );
                    Ok::<_, anyhow::Error>(
                        if context["desktop_onboarding_completed_at"].is_null()
                            && workspaces.is_empty()
                        {
                            Some(ContextData {
                                account: Some(context),
                                projects: workspaces,
                            })
                        } else {
                            None
                        },
                    )
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.onboarding_load = None;
                if epoch == this.navigation_epoch
                    && this.state.route == Route::Home
                    && this.chatgpt_account
                    && this.login.is_none()
                    && let Ok(Some(context)) = result
                {
                    this.onboarding_context = Some(context);
                    this.navigate(Route::Onboarding(Page::Welcome), window, cx);
                }
            })
            .ok();
        }));
    }

    pub(crate) fn open_onboarding(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (chatgpt, clock) = (self.chatgpt_account, self.clock);
        let view = cx.new(|cx| Onboarding::new(page, chatgpt, clock, cx));
        let role_plugins = std::cell::RefCell::new(None);
        cx.subscribe_in(
            &view,
            window,
            move |this, view, event: &OnboardingEvent, window, cx| {
                if this.onboarding_page.as_ref() != Some(view) {
                    return;
                }
                match event {
                    OnboardingEvent::Reload => this.read_onboarding(view.clone(), window, cx),
                    OnboardingEvent::Complete(completion) => this.complete_onboarding(
                        view.clone(),
                        completion.clone(),
                        Route::Home,
                        role_plugins.borrow().clone(),
                        window,
                        cx,
                    ),
                    OnboardingEvent::PickFolder => {
                        this.pick_onboarding_folder(view.clone(), window, cx)
                    }
                    OnboardingEvent::CreateProject => {
                        this.create_onboarding_project(view.clone(), window, cx)
                    }
                    OnboardingEvent::OpenImport(completion) => {
                        this.complete_onboarding(
                            view.clone(),
                            completion.clone(),
                            Route::Settings,
                            role_plugins.borrow().clone(),
                            window,
                            cx,
                        );
                    }
                    // The original writes only this key for the shipped `none` NUX variant.
                    OnboardingEvent::FirstRunComplete => {
                        let result = cx
                            .global_mut::<host_settings::Settings>()
                            .write("viewed2025-09-15-nux", json!(true));
                        match result {
                            Ok(()) => this.navigate(Route::Home, window, cx),
                            Err(error) => view.update(cx, |view, cx| {
                                view.set_error(format!("Could not save setup: {error}"), cx)
                            }),
                        }
                    }
                    OnboardingEvent::PreinstallRolePlugins(role)
                        if !this.bridge.is_acp() && this.config["features"]["plugins"] != false =>
                    {
                        let (sender, receiver) = async_channel::bounded(1);
                        role_plugins.replace(Some(receiver));
                        let role = role.clone();
                        let host = this.host.clone();
                        let bridge = this.bridge.clone();
                        let stable_id = cx
                            .global::<host_settings::Settings>()
                            .read_all()
                            .get("statsig-stable-id")
                            .and_then(Value::as_str)
                            .map_or_else(host::device_id, str::to_owned);
                        cx.background_spawn(async move {
                            let result = async {
                                let identity = host.identity();
                                let request =
                                    chatgpt::bootstrap_request(&stable_id, identity.as_ref());
                                let config =
                                    chatgpt::payload(host::json_body(&host.fetch(&request)?)?)?;
                                install_role_plugins(
                                    &bridge,
                                    &config["dynamic_configs"]["3734000477"]["value"],
                                    &role,
                                )
                                .await
                            }
                            .await;
                            match result {
                                Ok(Some(path)) => {
                                    let _ = sender.try_send(path);
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    eprintln!("codex-gpui: onboarding role plugins: {error:#}")
                                }
                            }
                        })
                        .detach();
                    }
                    OnboardingEvent::PreinstallRolePlugins(_) => {}
                    OnboardingEvent::LoadTasks(role) => {
                        let request = recommendations_request(
                            role,
                            mail_provider(this.account_email.as_deref()),
                        );
                        let (view, host) = (view.clone(), this.host.clone());
                        cx.spawn_in(window, async move |_, cx| {
                            let tasks = cx
                                .background_spawn(async move {
                                    let response = host
                                        .fetch(&request)
                                        .and_then(|response| host::json_body(&response));
                                    starter_tasks(response.ok().as_ref())
                                })
                                .await;
                            view.update(cx, |view, cx| view.set_tasks(tasks, cx));
                        })
                        .detach();
                    }
                    OnboardingEvent::RequestLocalFiles(folders) => {
                        let (view, folders) = (view.clone(), folders.clone());
                        let home = crate::env::home();
                        cx.spawn_in(window, async move |_, cx| {
                            let accessible = cx
                                .background_spawn(async move {
                                    home.map_or_else(
                                        || vec![false],
                                        |home| folder_access(&home, &folders),
                                    )
                                })
                                .await;
                            view.update(cx, |view, cx| view.set_local_files(accessible, cx));
                        })
                        .detach();
                    }
                    #[cfg(target_os = "macos")]
                    OnboardingEvent::EnableComputerCapability(capability) => {
                        let (view, bridge, host, capability) = (
                            view.clone(),
                            this.bridge.clone(),
                            this.host.clone(),
                            *capability,
                        );
                        cx.spawn_in(window, async move |_, cx| {
                            let result = cx
                                .background_spawn(async move {
                                    use ui::screens::onboarding::ComputerCapability;
                                    match capability {
                                        ComputerCapability::Use => {
                                            enable_computer_use(&bridge, host.as_ref()).await
                                        }
                                        ComputerCapability::History => {
                                            prepare_computer_use_helper(host.as_ref())?;
                                            crate::chronicle::enable_from_onboarding(
                                                &bridge,
                                                host.as_ref(),
                                            )
                                        }
                                    }
                                })
                                .await;
                            if let Err(error) = &result {
                                eprintln!("codex-gpui: onboarding {capability:?}: {error:#}");
                            }
                            view.update(cx, |view, cx| {
                                view.set_computer_capability(
                                    capability,
                                    result.is_ok_and(|enabled| enabled),
                                    cx,
                                );
                            });
                        })
                        .detach();
                    }
                }
            },
        )
        .detach();
        self.onboarding_page = Some(view.clone());
        if page != Page::FirstRun {
            self.read_onboarding(view, window, cx);
        }
    }

    fn read_onboarding(
        &mut self,
        view: Entity<Onboarding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let workspace = view.read(cx).page() == Page::SelectWorkspace;
        let prepared = self.onboarding_context.take();
        let settings = cx.global::<host_settings::Settings>().read_all();
        let saved = settings
            .get("electron:onboarding-welcome-v2-role-state")
            .cloned();
        let life_sciences = settings
            .get("electron:onboarding-login-entrypoint")
            .and_then(Value::as_str)
            == Some("life_sciences");
        self.onboarding_load = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(prepared) = prepared {
                        return Ok(prepared);
                    }
                    let response = if workspace {
                        None
                    } else {
                        let response = host::json_body(&host.fetch(&context_request())?)?;
                        ensure!(response.is_object(), "Invalid onboarding context");
                        Some(response)
                    };
                    Ok::<_, anyhow::Error>(ContextData {
                        account: response,
                        projects: projects(host.as_ref())?,
                    })
                })
                .await;
            let mut import_available = false;
            if result.is_ok() && !workspace {
                for params in [
                    json!({"includeHome":true}),
                    json!({"includeHome":true,"migrationSource":"cursor"}),
                ] {
                    if let Ok(detected) = bridge
                        .request::<_, Value>("externalAgentConfig/detect", params)
                        .await
                    {
                        import_available |= detected["items"]
                            .as_array()
                            .is_some_and(|items| !items.is_empty());
                    }
                }
            }
            this.update_in(cx, |this, _, cx| {
                this.onboarding_load = None;
                if this.onboarding_page.as_ref() != Some(&view) {
                    return;
                }
                view.update(cx, |view, cx| match result {
                    Ok(ContextData {
                        account: context,
                        projects,
                    }) => {
                        let roles = saved
                            .as_ref()
                            .and_then(|saved| saved["roles"].as_array())
                            .map(|roles| {
                                roles
                                    .iter()
                                    .filter_map(|role| role.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_else(|| {
                                context
                                    .as_ref()
                                    .and_then(|context| context["role"].as_str())
                                    .map(|role| vec![role.to_owned()])
                                    .unwrap_or_default()
                            });
                        let suggestions = saved
                            .as_ref()
                            .and_then(|saved| saved["personalizedSuggestionsEnabled"].as_bool())
                            .unwrap_or(true);
                        view.set_context(
                            roles,
                            life_sciences,
                            suggestions,
                            import_available,
                            projects,
                            cx,
                        );
                    }
                    Err(error) => view.set_error(error.to_string(), cx),
                });
            })
            .ok();
        }));
    }

    fn complete_onboarding(
        &mut self,
        view: Entity<Onboarding>,
        completion: Completion,
        destination: Route,
        role_plugins: Option<async_channel::Receiver<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let chatgpt = self.chatgpt_account && view.read(cx).page() != Page::SelectWorkspace;
        let payload = json!({"conversational_onboarding_skipped":completion.task.is_none(),"onboarding_credit_reward_warning_shown":false,"role":completion.representative_role()});
        let desktop = crate::env::home().map(|home| home.join("Desktop"));
        cx.spawn_in(window,async move |this,cx| {
            let result=cx.background_spawn(async move {
                if chatgpt {
                    let request=host::Request::Http(chatgpt::desktop_request("POST","/wham/onboarding/desktop/complete",Some(payload)));
                    successful_response(&host.fetch(&request)?)?;
                }
                for root in &completion.roots {host.add_project(Path::new(root))?;}
                // The backend already recorded completion, so a missing Desktop must not block setup.
                if completion.task.as_ref().is_some_and(|task| task.prompt.is_none())
                    && let Err(error) = desktop.context("Home directory is unavailable").and_then(|desktop| create_desktop_note(&desktop))
                {
                    eprintln!("codex-gpui: cannot write the Desktop note: {error:#}");
                }
                Ok::<_,anyhow::Error>(completion)
            }).await;
            this.update_in(cx,|this,window,cx| {
                if this.onboarding_page.as_ref()!=Some(&view) {return;}
                let result=result.and_then(|completion| {
                    let settings=cx.global_mut::<host_settings::Settings>();
                    settings.write("electron:onboarding-welcome-v2-role-state",json!({"roles":completion.roles,"personalizedSuggestionsEnabled":completion.personalized_suggestions,"workMode":completion.work_mode()}))?;
                    settings.write("last_completed_onboarding",json!(chrono::Utc::now().timestamp()))?;
                    Ok(completion)
                });
                match result {
                    Ok(completion)=>{
                        if destination == Route::Settings {this.settings_section=Some("Import");}
                        this.load_projects(cx);
                        if destination == Route::Home && let Some(root)=completion.roots.first() {this.select_project(root.clone(),window,cx);}else{this.navigate(destination,window,cx);}
                        // The original runs a prompt task as the first turn.
                        if let Some(prompt)=completion.task.and_then(|task| task.prompt) {
                            this.plugin_prefill(&prompt,window,cx);
                            let composer=this.home.read(cx).composer.clone();
                            composer.update(cx,|composer,cx| composer.send_draft(false,window,cx));
                        }
                        // A failed setup save must leave the result available for a retry.
                        if !completion.skipped && let Some(role_plugins) = role_plugins {
                            this.show_role_plugins_toast(role_plugins, window, cx);
                        }
                    }
                    Err(error)=>view.update(cx,|view,cx|view.set_error(format!("Couldn't finish setup — try again: {error}"),cx)),
                }
            }).ok();
        }).detach();
    }

    fn show_role_plugins_toast(
        &self,
        role_plugins: async_channel::Receiver<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let Ok(path) = role_plugins.recv().await else {
                return;
            };
            this.update_in(cx, |this, _, cx| {
                let root = cx.weak_entity();
                this.toast(
                    ui::toast::Toast::info("Plugins installed").action(
                        "Manage",
                        move |window, cx| {
                            let _ =
                                root.update(cx, |this, cx| this.open_plugins(&path, window, cx));
                        },
                    ),
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn pick_onboarding_folder(
        &mut self,
        view: Entity<Onboarding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.picking_project {
            return;
        }
        self.picking_project = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project".into()),
        });
        let host = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(paths) = paths.await?? else {
                    return Ok(None);
                };
                let Some(path) = paths.first() else {
                    return Ok(None);
                };
                let path = path.clone();
                cx.background_spawn(async move {
                    let root = host.add_project(&path)?;
                    Ok::<_, anyhow::Error>(Some((root, projects(host.as_ref())?)))
                })
                .await
            }
            .await;
            this.update_in(cx, |this, _, cx| {
                this.picking_project = false;
                if this.onboarding_page.as_ref() != Some(&view) {
                    return;
                }
                view.update(cx, |view, cx| match result {
                    Ok(Some((root, projects))) => view.set_projects(projects, Some(root), cx),
                    Ok(None) => view.set_busy(false, cx),
                    Err(error) => {
                        view.set_error(format!("Could not add the selected folders: {error}"), cx)
                    }
                });
            })
            .ok();
        })
        .detach();
    }

    fn create_onboarding_project(
        &mut self,
        view: Entity<Onboarding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let home = crate::env::home();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let root = create_project(
                        &home
                            .context("Home directory is unavailable")?
                            .join("Documents"),
                    )?;
                    let root = host.add_project(&root)?;
                    Ok::<_, anyhow::Error>((root, projects(host.as_ref())?))
                })
                .await;
            this.update_in(cx, |this, _, cx| {
                if this.onboarding_page.as_ref() != Some(&view) {
                    return;
                }
                view.update(cx, |view, cx| match result {
                    Ok((root, projects)) => view.project_created(projects, root, cx),
                    Err(error) => {
                        view.set_error(format!("Couldn't create a new project: {error}"), cx)
                    }
                });
            })
            .ok();
        })
        .detach();
    }
}

fn create_project(documents: &Path) -> Result<PathBuf> {
    let parent = documents.join("ChatGPT");
    std::fs::create_dir_all(&parent)?;
    for index in 1.. {
        let root = parent.join(if index == 1 {
            "New project".to_owned()
        } else {
            format!("New project {index}")
        });
        match std::fs::create_dir(&root) {
            Ok(()) => {
                let git = std::process::Command::new("git")
                    .args(["init", "--quiet"])
                    .current_dir(&root)
                    .status();
                if let Err(error) = git {
                    eprintln!("Could not initialize the new project repository: {error}");
                }
                return Ok(root);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!()
}

async fn install_role_plugins(
    bridge: &bridge::Bridge,
    config: &Value,
    role: &str,
) -> Result<Option<String>> {
    let Some(ids) = config[role]
        .as_array()
        .filter(|ids| !ids.is_empty() && ids.len() <= 16)
    else {
        return Ok(None);
    };
    if !ids
        .iter()
        .all(|id| id.as_str().is_some_and(|id| !id.is_empty()))
    {
        return Ok(None);
    }
    let catalog: Value = bridge
        .request("plugin/list", json!({"cwds": [], "forceRefetch": false}))
        .await?;
    let marketplaces = ui::screens::plugins::array(&catalog, "marketplaces");
    let mut all_installed = true;
    let mut any_new = false;
    let mut seen = std::collections::HashSet::new();
    for id in ids
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| seen.insert(*id))
    {
        let candidate = marketplaces
            .iter()
            .filter(|market| market["path"].is_null())
            .find_map(|market| {
                let plugin = ui::screens::plugins::array(market, "plugins")
                    .iter()
                    .find(|plugin| plugin["name"] == id || plugin["remotePluginId"] == id)?;
                let marketplace = market["remoteMarketplaceName"]
                    .as_str()
                    .or_else(|| market["name"].as_str())?;
                Some((plugin, marketplace, plugin["remotePluginId"].as_str()?))
            });
        let Some((plugin, marketplace, remote_id)) = candidate else {
            all_installed = false;
            continue;
        };
        if marketplaces.iter().any(|market| {
            ui::screens::plugins::array(market, "plugins")
                .iter()
                .any(|entry| {
                    entry["installed"] == true
                        && (entry["id"] == plugin["id"] || entry["remotePluginId"] == remote_id)
                })
        }) {
            continue;
        }
        let params = json!({"pluginName":remote_id,"remoteMarketplaceName":marketplace});
        let result = async {
            let detail: Value = bridge.request("plugin/read", &params).await?;
            if detail["plugin"]["summary"]["mustShowInstallationInterstitial"] != false {
                return Ok(false);
            }
            bridge
                .request::<_, Value>("plugin/install", &params)
                .await?;
            Ok::<_, anyhow::Error>(true)
        }
        .await;
        match result {
            Ok(true) => any_new = true,
            Ok(false) => all_installed = false,
            Err(error) => {
                eprintln!("codex-gpui: onboarding plugin {id}: {error:#}");
                all_installed = false;
            }
        }
    }
    Ok((all_installed && any_new).then(|| {
        config["manage_category_by_role"][role]
            .as_str()
            .map(str::trim)
            .filter(|category| !category.is_empty())
            .map_or_else(
                || "/plugins".to_owned(),
                |category| {
                    let query = url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("category", category)
                        .finish();
                    format!("/skills?{query}")
                },
            )
    }))
}

/// The bundled catalog the original falls back to when the backend has no tasks (`Bl`).
const FALLBACK_TASKS: &str = include_str!("onboarding_tasks.json");

fn recommendations_request(role: &str, mail_provider: &str) -> host::Request {
    let url = format!(
        "/onboarding/recommendations?provider_id=codex-starter-tasks&role={role}&mail_provider={mail_provider}"
    );
    let mut request = chatgpt::desktop_request("GET", &url, None);
    request["headers"]["x-openai-use-sheep"] = "1".into();
    host::Request::Http(request)
}

/// `email-domain-mail-provider` in the original, which reads the domain's MX records.
// shortcut: names the public Google and Microsoft mail domains only, so a custom domain is `other`;
// add an MX lookup when the app gains a DNS resolver.
fn mail_provider(email: Option<&str>) -> &'static str {
    let domain = email
        .and_then(|email| email.rsplit_once('@'))
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => "google",
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => "microsoft",
        _ => "other",
    }
}

/// The cards of the starter-task step, in the backend's order; the bundled catalog fills in when
/// the backend has nothing ready. Tasks this app cannot run are left out, as the original drops
/// Computer use here.
fn starter_tasks(response: Option<&Value>) -> Vec<StarterTask> {
    let fallback: Value = serde_json::from_str(FALLBACK_TASKS).unwrap_or_default();
    let ready = response.filter(|response| {
        response["status"] == "ready" && response["provider_id"] == "codex-starter-tasks"
    });
    let ids = ready
        .and_then(|response| response["task_ids"].as_array())
        .filter(|ids| !ids.is_empty())
        .unwrap_or_else(|| fallback["task_ids"].as_array().expect("bundled task ids"));
    ids.iter()
        .filter_map(Value::as_str)
        .filter_map(|id| {
            let task = ready
                .and_then(|response| response["tasks"].get(id))
                .or_else(|| fallback["tasks"].get(id))?;
            let plugin = task["pluginName"].as_str().unwrap_or_default();
            if id == "computer_use" || plugin == "computer-use" {
                return None;
            }
            let prompt = if id == "desktop_note" {
                None
            } else {
                Some(task["prompt"].as_str()?.to_owned())
            };
            Some(StarterTask {
                id: id.to_owned(),
                label: task["messages"]["label"]["defaultMessage"]
                    .as_str()?
                    .to_owned(),
                icon: match plugin {
                    "" => "folder",
                    "visualize" => "project-marker-bar-chart",
                    _ => "plugins",
                },
                prompt,
            })
        })
        .collect()
}

/// The original's `createDesktopNote`: a text file on the Desktop, numbered when the name is taken.
fn create_desktop_note(desktop: &Path) -> Result<PathBuf> {
    for index in 1.. {
        let path = desktop.join(if index == 1 {
            "Note from ChatGPT.txt".to_owned()
        } else {
            format!("Note from ChatGPT {index}.txt")
        });
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                std::io::Write::write_all(
                    &mut file,
                    "Hello from ChatGPT.\n\nYou ask. I can do – now directly on your computer, too."
                        .as_bytes(),
                )?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!()
}

/// Opens each folder in order; on macOS the first read raises the system's folder-access prompt.
/// Stops at the first folder it cannot open, as the original's `requestLocalFilesAccess` loop does.
fn folder_access(home: &Path, folders: &[&str]) -> Vec<bool> {
    let mut accessible = Vec::new();
    for folder in folders {
        let open = std::fs::read_dir(home.join(folder)).is_ok();
        accessible.push(open);
        if !open {
            break;
        }
    }
    accessible
}

#[cfg(any(target_os = "macos", test))]
async fn enable_computer_use(bridge: &bridge::Bridge, host: &dyn host::Host) -> Result<bool> {
    const ID: &str = "computer-use@openai-bundled";
    struct BundledPlugin {
        marketplace_path: Value,
        installed: bool,
        enabled: bool,
    }

    async fn plugin(bridge: &bridge::Bridge) -> Result<BundledPlugin> {
        let mut list: Value = bridge
            .request(
                "plugin/list",
                json!({"cwds": [], "marketplaceKinds": ["local"]}),
            )
            .await?;
        list["marketplaces"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .find_map(|market| {
                market["path"].as_str()?;
                let plugin = market["plugins"]
                    .as_array()?
                    .iter()
                    .find(|p| p["id"] == ID)?;
                let installed = plugin["installed"] == true;
                let enabled = plugin["enabled"] == true;
                Some(BundledPlugin {
                    marketplace_path: market["path"].take(),
                    installed,
                    enabled,
                })
            })
            .context("The bundled Computer use plugin is unavailable")
    }

    let mut summary = plugin(bridge).await?;
    if !summary.installed {
        bridge
            .request::<_, Value>(
                "plugin/install",
                json!({"marketplacePath": summary.marketplace_path, "pluginName": "computer-use"}),
            )
            .await?;
        summary = plugin(bridge).await?;
    }
    ensure!(summary.installed, "Computer use was not installed");
    if !summary.enabled {
        bridge
            .config_write(&[bridge::ConfigEdit::upsert(
                "plugins.\"computer-use@openai-bundled\".enabled",
                true,
            )])
            .await?;
        let updated = plugin(bridge).await?;
        ensure!(
            updated.installed && updated.enabled,
            "Computer use was not enabled"
        );
    }
    bridge
        .request::<_, Value>("config/mcpServer/reload", json!({}))
        .await?;
    request_computer_use_permissions(host)
}

#[cfg(any(target_os = "macos", test))]
fn prepare_computer_use_helper(host: &dyn host::Host) -> Result<()> {
    if !host.skysight_available() {
        let app = std::env::var_os("CODEX_GPUI_LCU_APP")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/Applications/ChatGPT.app"));
        host.computer_use_native()
            .context("The signed Computer Use helper is unavailable")?
            .install_helper(&app.join(
                "Contents/Resources/cua_node/lib/node_modules/@oai/sky/Codex Computer Use.app",
            ))?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn request_computer_use_permissions(host: &dyn host::Host) -> Result<bool> {
    use host::computer_use_native::NativeReplyError;

    prepare_computer_use_helper(host)?;
    match host.skysight(
        "ComputerUseIPCAppGetSkyshotRequest",
        &json!({"app": "com.apple.systempreferences"}),
        115,
    ) {
        Ok(_) => Ok(true),
        Err(error)
            if error
                .downcast_ref::<NativeReplyError>()
                .is_some_and(|e| e.code == -10014) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn successful_response(response: &Value) -> Result<()> {
    let response = response.get("value").unwrap_or(response);
    ensure!(
        response["responseType"] != "error",
        "Onboarding request failed: {}",
        response["error"]
    );
    let response = response.pointer("/response/__response").unwrap_or(response);
    let status = response["status"]
        .as_u64()
        .context("Missing onboarding response status")?;
    ensure!(
        (200..300).contains(&status),
        "Onboarding request failed with HTTP {status}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_preinstall_requires_a_new_install_and_suppresses_partial_success() -> Result<()> {
        use std::io::Write as _;

        let config = json!({
            "life_sciences": ["eligible", "installed", "eligible"],
            "manage_category_by_role": {"life_sciences": "scientific-research"}
        });
        let catalog = json!({"marketplaces": [{"name": "openai-curated", "plugins": [
            {"id":"eligible@openai-curated","name":"eligible","remotePluginId":"plugin_eligible"},
            {"id":"installed@openai-curated","name":"installed","remotePluginId":"plugin_installed","installed":true}
        ]}]});
        let mut replay = tempfile::NamedTempFile::new()?;
        let params =
            json!({"pluginName":"plugin_eligible","remoteMarketplaceName":"openai-curated"});
        for (id, method, params, result) in [
            (
                1,
                "plugin/list",
                json!({"cwds":[],"forceRefetch":false}),
                catalog.clone(),
            ),
            (
                2,
                "plugin/read",
                params.clone(),
                json!({"plugin":{"summary":{"mustShowInstallationInterstitial":false}}}),
            ),
            (
                3,
                "plugin/install",
                params.clone(),
                json!({"authPolicy":"ON_USE","appsNeedingAuth":[]}),
            ),
            (
                4,
                "plugin/list",
                json!({"cwds":[],"forceRefetch":false}),
                catalog,
            ),
            (
                5,
                "plugin/read",
                params,
                json!({"plugin":{"summary":{"mustShowInstallationInterstitial":true}}}),
            ),
        ] {
            writeln!(
                replay,
                "{}\n{}",
                json!({"dir":"send","msg":{"id":id,"method":method,"params":params}}),
                json!({"dir":"recv","msg":{"id":id,"result":result}})
            )?;
        }
        let (bridge, _) = bridge::Bridge::replay(replay.path())?;
        let run = |role| pollster::block_on(install_role_plugins(&bridge, &config, role));
        assert_eq!(run("engineering")?, None);
        assert_eq!(
            run("life_sciences")?.as_deref(),
            Some("/skills?category=scientific-research")
        );
        assert_eq!(run("life_sciences")?, None);
        Ok(())
    }

    #[test]
    fn computer_use_requests_permissions_only_after_installing_the_bundled_plugin() -> Result<()> {
        use std::io::Write as _;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct PermissionService(AtomicUsize);
        impl host::Host for PermissionService {
            fn fetch(&self, _: &host::Request) -> Result<Value> {
                anyhow::bail!("No network in the permission test")
            }
            fn skysight_available(&self) -> bool {
                true
            }
            fn skysight(&self, method: &str, params: &Value, timeout: u32) -> Result<Value> {
                assert_eq!(method, "ComputerUseIPCAppGetSkyshotRequest");
                assert_eq!(*params, json!({"app": "com.apple.systempreferences"}));
                assert_eq!(timeout, 115);
                if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                    return Err(host::computer_use_native::NativeReplyError {
                        code: -10014,
                        message: "Permission pending".into(),
                    }
                    .into());
                }
                Ok(json!({}))
            }
        }

        let plugin = json!({"id": "computer-use@openai-bundled", "installed": false});
        let available = json!({"marketplaces": [{
            "path": "/bundled/marketplace.json", "plugins": [plugin]
        }]});
        let installed = json!({"marketplaces": [{
            "path": "/bundled/marketplace.json",
            "plugins": [{"id": "computer-use@openai-bundled", "installed": true, "enabled": true}]
        }]});
        let mut replay = tempfile::NamedTempFile::new()?;
        let list = json!({"cwds": [], "marketplaceKinds": ["local"]});
        let install =
            json!({"marketplacePath": "/bundled/marketplace.json", "pluginName": "computer-use"});
        for (id, method, params, result) in [
            (1, "plugin/list", list.clone(), available),
            (2, "plugin/install", install, json!({})),
            (3, "plugin/list", list.clone(), installed.clone()),
            (4, "config/mcpServer/reload", json!({}), json!({})),
            (5, "plugin/list", list, installed),
            (6, "config/mcpServer/reload", json!({}), json!({})),
        ] {
            let send =
                json!({"dir": "send", "msg": {"id": id, "method": method, "params": params}});
            let recv = json!({"dir": "recv", "msg": {"id": id, "result": result}});
            writeln!(replay, "{send}\n{recv}")?;
        }
        let (bridge, _) = bridge::Bridge::replay(replay.path())?;
        let host = PermissionService(AtomicUsize::new(0));
        assert!(!pollster::block_on(enable_computer_use(&bridge, &host))?);
        assert!(pollster::block_on(enable_computer_use(&bridge, &host))?);
        assert_eq!(host.0.load(Ordering::Relaxed), 2);
        Ok(())
    }
    #[test]
    fn workspace_creation_preserves_existing_folders() -> Result<()> {
        let home = tempfile::tempdir()?;
        let first = create_project(home.path())?;
        std::fs::write(first.join("keep"), "existing")?;
        let second = create_project(home.path())?;
        assert_eq!(first.file_name().unwrap(), "New project");
        assert_eq!(second.file_name().unwrap(), "New project 2");
        assert_eq!(std::fs::read_to_string(first.join("keep"))?, "existing");
        Ok(())
    }

    #[test]
    fn folder_access_stops_at_the_first_folder_it_cannot_open() -> Result<()> {
        let home = tempfile::tempdir()?;
        std::fs::create_dir(home.path().join("Desktop"))?;
        std::fs::create_dir(home.path().join("Downloads"))?;
        let folders = ui::screens::onboarding::FOLDERS;
        assert_eq!(folder_access(home.path(), &folders), [true, false]);
        std::fs::create_dir(home.path().join("Documents"))?;
        assert_eq!(folder_access(home.path(), &folders), [true, true, true]);
        Ok(())
    }

    #[test]
    fn starter_tasks_follow_the_backend_and_fall_back_to_the_bundled_catalog() {
        // Recorded from the live backend for a `marketing` role, prompts shortened.
        let live = json!({
            "status": "ready",
            "provider_id": "codex-starter-tasks",
            "task_ids": ["desktop_note", "computer_use", "data_science_csv_chart", "hold_next_free_hour"],
            "tasks": {
                "desktop_note": {"kind": "native_action", "icon": "folder",
                    "messages": {"label": {"defaultMessage": "Leave a note on my Desktop"}}},
                "data_science_csv_chart": {"kind": "prompt", "pluginName": "visualize",
                    "prompt": "Use [@visualize] to chart this CSV",
                    "messages": {"label": {"defaultMessage": "Turn data into a chart"}}},
                "hold_next_free_hour": {"kind": "prompt", "pluginName": "outlook-calendar",
                    "prompt": "Use Outlook Calendar",
                    "messages": {"label": {"defaultMessage": "Schedule a focus block this week"}}},
            },
        });
        let tasks = starter_tasks(Some(&live));
        let labels: Vec<_> = tasks.iter().map(|task| task.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Leave a note on my Desktop",
                "Turn data into a chart",
                "Schedule a focus block this week"
            ]
        );
        assert_eq!(tasks[0].prompt, None);
        assert_eq!(tasks[1].icon, "project-marker-bar-chart");
        assert_eq!(tasks[2].prompt.as_deref(), Some("Use Outlook Calendar"));

        // A pending or failed request shows the bundled tasks this app can run.
        let pending =
            json!({"status": "pending", "provider_id": "codex-starter-tasks", "tasks": {}});
        for response in [Some(&pending), None] {
            let ids: Vec<_> = starter_tasks(response)
                .into_iter()
                .map(|task| task.id)
                .collect();
            assert_eq!(
                ids,
                [
                    "desktop_note",
                    "hold_next_free_hour",
                    "send_message_to_self"
                ]
            );
        }
    }

    #[test]
    fn mail_provider_names_known_domains() {
        assert_eq!(mail_provider(Some("a@Gmail.com")), "google");
        assert_eq!(mail_provider(Some("a@outlook.com")), "microsoft");
        assert_eq!(mail_provider(Some("a@example.org")), "other");
        assert_eq!(mail_provider(None), "other");
    }

    #[test]
    fn desktop_note_never_overwrites_a_file() -> Result<()> {
        let desktop = tempfile::tempdir()?;
        std::fs::write(desktop.path().join("Note from ChatGPT.txt"), "mine")?;
        let note = create_desktop_note(desktop.path())?;
        assert_eq!(note.file_name().unwrap(), "Note from ChatGPT 2.txt");
        assert_eq!(
            std::fs::read_to_string(desktop.path().join("Note from ChatGPT.txt"))?,
            "mine"
        );
        Ok(())
    }
}
