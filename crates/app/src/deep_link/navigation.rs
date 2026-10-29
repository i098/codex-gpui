use super::*;
use anyhow::{Result, anyhow};
use gpui::WindowHandle;
use std::path::PathBuf;
use ui::toast::Toast;

pub(super) fn host_window(
    host_id: &str,
    except: Option<gpui::WindowId>,
    cx: &App,
) -> Option<WindowHandle<Root>> {
    cx.windows()
        .into_iter()
        .filter(|window| Some(window.window_id()) != except)
        .filter_map(|window| window.downcast::<Root>())
        .find(|window| window.read(cx).is_ok_and(|root| root.host_id() == host_id))
}

impl Root {
    pub(crate) fn open_link_host(
        &mut self,
        host_id: &str,
        link: &Link,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<WindowHandle<Root>> {
        let result = if let Some(handle) =
            host_window(host_id, Some(self.context_window.window_id()), cx)
        {
            Ok(handle)
        } else if host_id == "local" {
            let cwd = std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            windows::open_primary(State::load(), cwd, cx)
        } else if let Some(bridge) = crate::connections::bridge(host_id, cx) {
            let mut state = self.state.clone();
            state.path = None;
            state.project_root = None;
            state.route = Route::Home;
            let host_id = host_id.to_owned();
            let cwd = self.cwd.clone();
            let events = bridge.subscribe_async();
            windows::open(
                windows::Spec::new(windows::Kind::Primary, "Codex"),
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Root::with_host_connection(state, bridge, events, cwd, host_id, window, cx)
                    })
                },
            )
        } else {
            Err(anyhow!(
                "Connect to the requested host before opening this link"
            ))
        };
        match result {
            Ok(handle) => {
                let _ = handle.update(cx, |root, window, cx| root.open_deep_link(link, window, cx));
                Some(handle)
            }
            Err(error) => {
                self.toast(Toast::danger(error.to_string()), cx);
                None
            }
        }
    }

    pub(super) fn open_link_project(
        &mut self,
        project_id: Option<String>,
        path: Option<String>,
        origin_url: Option<String>,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if project_id.is_none() && path.is_none() && origin_url.is_none() {
            self.new_chat(window, cx);
            self.prefill_link(prompt.as_deref(), window, cx);
            return;
        }
        let host = self.host.clone();
        let epoch = self.navigation_epoch;
        let read = cx.background_spawn(async move {
            let projects = host.projects()?;
            let selected = linked_project(
                &projects,
                project_id.as_deref(),
                path.as_deref(),
                origin_url.as_deref(),
            );
            if let Some(root) = &selected {
                host.add_project(std::path::Path::new(root))?;
            }
            Ok::<_, anyhow::Error>((selected, host.projects()?))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<_> = read.await;
            this.update_in(cx, |this, window, cx| {
                if this.navigation_epoch != epoch {
                    return;
                }
                match result {
                    Ok((root, projects)) => {
                        this.set_projects(projects, cx);
                        this.new_chat(window, cx);
                        if let Some(root) = root {
                            this.select_project(root, window, cx);
                        }
                        this.prefill_link(prompt.as_deref(), window, cx);
                    }
                    Err(error) => this.toast(Toast::danger(error.to_string()), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn prefill_link(
        &mut self,
        prompt: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = prompt.filter(|prompt| !prompt.is_empty()) {
            let composer = self.composer(cx);
            composer.update(cx, |composer, cx| composer.set_text(prompt, cx));
            window.focus(&composer.focus_handle(cx), cx);
        }
    }

    pub(super) fn open_automation_link(
        &mut self,
        id: String,
        source: &'static str,
        edit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_automation_source(source);
        self.pending_automation_edit = edit.then(|| id.clone());
        let route = Route::Automation(id.clone());
        if self.state.route == route {
            self.sync(window, cx);
        } else {
            self.navigate(route, window, cx);
        }
        if edit && self.automations.as_ref().is_some_and(Result::is_ok) {
            self.pending_automation_edit = None;
            self.edit_automation(Some(id), window, cx);
        }
    }

    pub(crate) fn set_automation_source(&mut self, source: &'static str) -> bool {
        if self.automation_source == source {
            return false;
        }
        self.automation_load = None;
        self.automations = None;
        self.automation_runs.clear();
        self.automation_updates.clear();
        self.running_automations.clear();
        self.automation_editor = None;
        self.pending_automation_edit = None;
        self.automation_source = source;
        true
    }

    pub(crate) fn automation_request(
        &self,
        endpoint: &'static str,
        mut body: Option<serde_json::Value>,
    ) -> crate::host::Request {
        let endpoint = if self.automation_source == "cloud" {
            if let Some(body) = body.as_mut()
                && let Some(item) = self
                    .automations
                    .as_ref()
                    .and_then(|list| list.as_ref().ok())
                    .and_then(|list| list["items"].as_array())
                    .and_then(|items| items.iter().find(|item| item["id"] == body["id"]))
            {
                body["expectedIdentity"] = item["expectedIdentity"].clone();
                body["cloudAutomation"] = item["cloudAutomation"].clone();
            }
            match endpoint {
                "list-automations" => "cloud-list-automations",
                "automation-update" => "cloud-automation-update",
                "automation-delete" => "cloud-automation-delete",
                "automation-run-now" => "cloud-automation-run-now",
                "automation-set-status" => "cloud-automation-set-status",
                _ => endpoint,
            }
        } else {
            endpoint
        };
        crate::host::Request::post(endpoint, body)
    }
}

fn linked_project(
    projects: &[ui::screens::sidebar::Project],
    id: Option<&str>,
    path: Option<&str>,
    origin: Option<&str>,
) -> Option<String> {
    if let Some(project) = id.and_then(|id| {
        projects
            .iter()
            .find(|project| project.id.as_ref() == id && !project.missing)
    }) {
        return project.root.as_ref().map(ToString::to_string);
    }
    if let Some(path) = path
        .and_then(|path| PathBuf::from(path).canonicalize().ok())
        .filter(|path| path.is_dir())
    {
        return Some(path.to_string_lossy().into_owned());
    }
    let origin = origin?;
    projects
        .iter()
        .filter(|project| !project.missing)
        .find_map(|project| {
            let root = project.root.as_ref()?;
            let output = std::process::Command::new("git")
                .args(["-C", root.as_ref(), "remote", "get-url", "origin"])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let candidate = std::str::from_utf8(&output.stdout).ok()?.trim();
            (candidate == origin
                || repository_key(candidate).is_some_and(|key| Some(key) == repository_key(origin)))
            .then(|| root.to_string())
        })
}

fn repository_key(origin: &str) -> Option<(String, String)> {
    let url = Url::parse(origin)
        .ok()
        .filter(|url| url.host_str().is_some())
        .or_else(|| {
            let (host, path) = origin.split_once(':')?;
            Url::parse(&format!("ssh://{host}/{path}")).ok()
        })?;
    let path = url.path().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let host = url.host_str()?.to_owned();
    (!path.is_empty()).then(|| (host, path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Host, LocalHost};

    #[test]
    fn project_selection_uses_id_then_folder_then_matching_origin() -> Result<()> {
        let profile = tempfile::tempdir()?;
        let repository = profile.path().join("repository");
        let folder = profile.path().join("folder");
        std::fs::create_dir(&repository)?;
        std::fs::create_dir(&folder)?;
        assert!(
            std::process::Command::new("git")
                .arg("init")
                .arg(&repository)
                .output()?
                .status
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args([
                    "remote",
                    "add",
                    "origin",
                    "git@github.com:owner/project.git"
                ])
                .output()?
                .status
                .success()
        );
        let host = LocalHost::new(profile.path());
        host.add_project(&repository)?;
        let projects = host.projects()?;
        let id = projects[0].id.as_ref();
        let repository = repository.canonicalize()?;
        let folder = folder.canonicalize()?;
        let repository = repository.to_str().unwrap();
        let folder = folder.to_str().unwrap();
        assert_eq!(
            linked_project(&projects, Some(id), Some(folder), None).as_deref(),
            Some(repository)
        );
        assert_eq!(
            linked_project(&projects, Some("missing"), Some(folder), None).as_deref(),
            Some(folder)
        );
        assert_eq!(
            linked_project(
                &projects,
                None,
                Some("/missing"),
                Some("https://github.com/owner/project")
            )
            .as_deref(),
            Some(repository)
        );
        assert_eq!(
            linked_project(
                &projects,
                None,
                None,
                Some("https://github.com/other/project")
            ),
            None
        );
        assert_eq!(
            linked_project(&[], None, Some("."), None),
            Some(
                std::env::current_dir()?
                    .canonicalize()?
                    .to_string_lossy()
                    .into_owned()
            )
        );
        Ok(())
    }
}
