use crate::{Root, Route, chatgpt, host, host_settings};
use anyhow::{Context as _, Result, ensure};
use gpui::{AppContext as _, Context, Entity, PathPromptOptions, Window};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::screens::onboarding::{Completion, Onboarding, OnboardingEvent, Page, Project};

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
        cx.subscribe_in(
            &view,
            window,
            |this, view, event: &OnboardingEvent, window, cx| {
                if this.onboarding_page.as_ref() != Some(view) {
                    return;
                }
                match event {
                    OnboardingEvent::Reload => this.read_onboarding(view.clone(), window, cx),
                    OnboardingEvent::Complete(completion) => this.complete_onboarding(
                        view.clone(),
                        completion.clone(),
                        Route::Home,
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
        let saved = cx
            .global::<host_settings::Settings>()
            .read_all()
            .get("electron:onboarding-welcome-v2-role-state")
            .cloned();
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
                        view.set_context(roles, suggestions, import_available, projects, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let chatgpt = self.chatgpt_account && view.read(cx).page() != Page::SelectWorkspace;
        let payload = json!({"conversational_onboarding_skipped":true,"onboarding_credit_reward_warning_shown":false,"role":completion.representative_role()});
        cx.spawn_in(window,async move |this,cx| {
            let result=cx.background_spawn(async move {
                if chatgpt {
                    let request=host::Request::Http(chatgpt::desktop_request("POST","/wham/onboarding/desktop/complete",Some(payload)));
                    successful_response(&host.fetch(&request)?)?;
                }
                for root in &completion.roots {host.add_project(Path::new(root))?;}
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
                    }
                    Err(error)=>view.update(cx,|view,cx|view.set_error(format!("Couldn't finish setup — try again: {error}"),cx)),
                }
            }).ok();
        }).detach();
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
}
