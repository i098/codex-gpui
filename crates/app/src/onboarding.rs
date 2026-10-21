use crate::{Root, Route, host, host_settings};
use anyhow::{Context as _, Result, ensure};
use gpui::{AppContext as _, Context, Entity, PathPromptOptions, Window};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::screens::onboarding::{Completion, Onboarding, OnboardingEvent, Page, Project};

fn context_request() -> host::Request {
    host::Request::Http(
        json!({"url":"/wham/onboarding/context","method":"GET","headers":{"OAI-Language":"en-US","X-OpenAI-Attach-Auth":"1","X-OpenAI-Attach-Integrity-State":"1","originator":"Codex Desktop"},"returnErrorResponse":true}),
    )
}

fn projects(host: &dyn host::Host) -> Result<Vec<Project>> {
    Ok(host
        .projects()?
        .into_iter()
        .filter(|project| !project.missing)
        .filter_map(|project| {
            Some(Project {
                root: project.root?.to_string(),
                label: project.label.to_string(),
            })
        })
        .collect())
}

impl Root {
    pub fn show_onboarding(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Onboarding(page), window, cx);
    }

    pub fn onboarding(&self) -> Option<&Entity<Onboarding>> {
        self.onboarding_page.as_ref()
    }

    pub(crate) fn maybe_onboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.chatgpt_account
            || self.state.route != Route::Home
            || self.onboarding_load.is_some()
            || cx
                .global::<host_settings::Settings>()
                .read_all()
                .get("electron:onboarding-projectless-completed")
                == Some(&Value::Bool(true))
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
                        context["desktop_onboarding_completed_at"].is_null()
                            && workspaces.is_empty(),
                    )
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.onboarding_load = None;
                if epoch == this.navigation_epoch
                    && result.unwrap_or(false)
                    && this.state.route == Route::Home
                {
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
        let view = cx.new(|_| Onboarding::new(page, self.chatgpt_account));
        cx.subscribe_in(&view,window,|this,view,event:&OnboardingEvent,window,cx| {
            if this.onboarding_page.as_ref()!=Some(view) {return;}
            match event {
                OnboardingEvent::Reload=>this.read_onboarding(view.clone(),window,cx),
                OnboardingEvent::Complete(completion)=>this.complete_onboarding(view.clone(),completion.clone(),window,cx),
                OnboardingEvent::FirstRunComplete=>{
                    let key=if this.chatgpt_account {"viewed2025-09-15-full-chatgpt-auth-nux"}else{"viewed2025-09-15-apikey-auth-nux"};
                    let result=(||{let settings=cx.global_mut::<host_settings::Settings>();settings.write("viewed2025-09-15-nux",json!(true))?;settings.write(key,json!(true))})();
                    match result {Ok(())=>this.navigate(Route::Home,window,cx),Err(error)=>view.update(cx,|view,cx|view.set_error(format!("Could not save setup: {error}"),cx))}
                }
                OnboardingEvent::PickFolder=>this.pick_onboarding_folder(view.clone(),window,cx),
                OnboardingEvent::CreateProject=>this.create_onboarding_project(view.clone(),window,cx),
                OnboardingEvent::OpenImport(completion)=>{
                    let result=cx.global_mut::<host_settings::Settings>().write("electron:onboarding-welcome-v2-role-state",json!({"roles":completion.roles,"personalizedSuggestionsEnabled":completion.personalized_suggestions,"workMode":completion.work_mode()}));
                    if let Err(error)=result {view.update(cx,|view,cx|view.set_error(error.to_string(),cx));return;}
                    this.settings_section=Some("Import");
                    this.navigate(Route::Settings,window,cx);
                }
            }
        }).detach();
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
        let saved = cx
            .global::<host_settings::Settings>()
            .read_all()
            .get("electron:onboarding-welcome-v2-role-state")
            .cloned();
        self.onboarding_load = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let response = if workspace {
                        None
                    } else {
                        let response = host::json_body(&host.fetch(&context_request())?)?;
                        ensure!(response.is_object(), "Invalid onboarding context");
                        Some(response)
                    };
                    Ok::<_, anyhow::Error>((response, projects(host.as_ref())?))
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
                    Ok((context, projects)) => {
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let chatgpt = self.chatgpt_account && view.read(cx).page() != Page::SelectWorkspace;
        let project_root = self.cwd.clone();
        let payload = json!({"conversational_onboarding_skipped":true,"onboarding_credit_reward_warning_shown":false,"role":completion.representative_role()});
        cx.spawn_in(window,async move |this,cx| {
            let result=cx.background_spawn(async move {
                if chatgpt {
                    let request=host::Request::Http(json!({"url":"/wham/onboarding/desktop/complete","method":"POST","body":payload.to_string(),"headers":{"OAI-Language":"en-US","X-OpenAI-Attach-Auth":"1","X-OpenAI-Attach-Integrity-State":"1","originator":"Codex Desktop"},"returnErrorResponse":true}));
                    successful_response(&host.fetch(&request)?)?;
                }
                for root in &completion.roots {host.add_project(Path::new(root))?;}
                if completion.personalized_suggestions {
                    let response=host.fetch(&host::Request::post("ambient-suggestions-refresh",Some(json!({"domain":null,"hostId":"local","projectRoot":project_root}))));
                    if let Err(error)=response.and_then(|response|host::json_body(&response)) {eprintln!("Failed to refresh ambient suggestions after onboarding: {error}");}
                }
                Ok::<_,anyhow::Error>(completion)
            }).await;
            this.update_in(cx,|this,window,cx| {
                if this.onboarding_page.as_ref()!=Some(&view) {return;}
                let result=result.and_then(|completion| {
                    let settings=cx.global_mut::<host_settings::Settings>();
                    settings.write("electron:onboarding-welcome-v2-role-state",json!({"roles":completion.roles,"personalizedSuggestionsEnabled":completion.personalized_suggestions,"workMode":completion.work_mode()}))?;
                    settings.write("last_completed_onboarding",json!(chrono::Utc::now().timestamp()))?;
                    settings.write("electron:onboarding-projectless-completed",json!(true))?;
                    Ok(completion)
                });
                match result {
                    Ok(completion)=>{
                        this.load_projects(cx);
                        if let Some(root)=completion.roots.first() {this.select_project(root.clone(),window,cx);}else{this.navigate(Route::Home,window,cx);}
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
        let home = std::env::var_os("HOME").map(PathBuf::from);
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
}
