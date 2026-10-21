use super::{Link, Review, parse};
use crate::{Root, Route, host, windows};
use gpui::{App, AppContext as _, Context, Focusable as _, Global, Window};
use std::path::{Path, PathBuf};
use ui::{dialog::Kind as DialogKind, screens::sidebar::Project, toast::Toast};

#[derive(Default)]
pub(crate) struct Navigation {
    generation: u64,
    review: Option<(String, u64, Review)>,
}

#[derive(Default)]
struct PendingUrl(Option<String>);
impl Global for PendingUrl {}

pub fn listen(receiver: async_channel::Receiver<Vec<String>>, cx: &mut App) {
    if !cx.has_global::<PendingUrl>() { cx.set_global(PendingUrl::default()); }
    cx.spawn(async move |cx| {
        while let Ok(urls) = receiver.recv().await {
            cx.update(|cx| {
                for url in urls {
                    if parse(&url).is_some() {
                        cx.global_mut::<PendingUrl>().0 = Some(url);
                    }
                }
                if cx.global::<PendingUrl>().0.is_some() {
                    if let Err(error) = windows::reopen(cx) {
                        eprintln!("codex-gpui: cannot open the deep-link window: {error:#}");
                    }
                    flush(cx);
                }
            });
        }
    }).detach();
}

pub(crate) fn flush(cx: &mut App) {
    let Some(handle) = windows::navigation_root(cx) else { return };
    if !handle.update(cx, |root, _, _| root.ready()).unwrap_or(false) { return; }
    if !cx.has_global::<PendingUrl>() { return; }
    let Some(url) = cx.global_mut::<PendingUrl>().0.take() else { return };
    let _ = handle.update(cx, |root, window, cx| root.receive_deep_link(&url, window, cx));
}

impl Root {
    pub fn receive_deep_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(link) = parse(url) else { return false };
        if !self.ready {
            if !cx.has_global::<PendingUrl>() { cx.set_global(PendingUrl::default()); }
            cx.global_mut::<PendingUrl>().0 = Some(url.to_owned());
            return true;
        }
        self.deep_link.generation += 1;
        self.deep_link.review = None;
        match link {
            Link::Launch => true,
            Link::Thread { id, host, shared, prompt, review } => {
                if shared || host.as_deref().is_some_and(|host| host != "local") { return false; }
                self.navigate(Route::Thread(id.clone()), window, cx);
                self.deep_link.review = review.map(|review| (id, self.navigation_epoch, review));
                if let Some(prompt) = prompt {
                    self.thread_composer.update(cx, |composer, cx| composer.set_text(&prompt, cx));
                    window.focus(&self.thread_composer.focus_handle(cx), cx);
                }
                self.finish_deep_link(window, cx);
                true
            }
            Link::NewThread { prompt, path, origin, project, mode } => {
                if mode.as_deref().is_some_and(|mode| mode != "codex") { return false; }
                self.open_new_thread_link(prompt, path, origin, project, window, cx);
                true
            }
            Link::Settings(section) => {
                let label = match section.split('#').next().unwrap_or("") {
                    "" => "General",
                    "browser-use" => "Browser",
                    "chronicle" => "Computer history",
                    "connections" => "Connections",
                    _ => return false,
                };
                self.open_settings_section(label, window, cx);
                true
            }
            Link::Automations { id: None, .. } => {
                self.navigate(Route::Scheduled, window, cx);
                self.open_dialog(DialogKind::ScheduleTask, window, cx);
                true
            }
            Link::Automations { id: Some(id), source, edit: false } if source == "local" => {
                self.navigate(Route::Automation(id), window, cx);
                true
            }
            Link::Family(raw) => {
                let Ok(url) = url::Url::parse(&raw) else { return false };
                let host = url.host_str().unwrap_or("");
                if !matches!(host, "plugins" | "skills") { return false; }
                let mut path = if host == "skills" { "/skills".to_owned() } else { format!("/plugins{}", url.path()) };
                if let Some(query) = url.query() {
                    path.push('?');
                    path.push_str(query);
                }
                self.open_plugins(&path, window, cx)
            }
            Link::Automations { .. } => false,
        }
    }

    pub(crate) fn finish_deep_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, epoch, _)) = &self.deep_link.review else { return };
        if *epoch != self.navigation_epoch {
            self.deep_link.review = None;
            return;
        }
        if !self.thread.as_ref().is_some_and(|thread| thread.read(cx).id() == id) { return; }
        let (_, _, review) = self.deep_link.review.take().unwrap();
        if let Some(panel) = &self.diff {
            panel.update(cx, |panel, cx| {
                if let Some(filter) = review.filter {
                    panel.select_source(if filter == "branch" {
                        ui::screens::diff::panel::DiffSource::Branch
                    } else {
                        ui::screens::diff::panel::DiffSource::LastTurn
                    }, cx);
                    if let Some(path) = review.path { panel.reveal_file(path, cx); }
                }
            });
        }
        self.set_side(crate::SidePanel::Changes, window, cx);
    }

    fn open_new_thread_link(&mut self, prompt: Option<String>, path: Option<String>, origin: Option<String>, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_none() && origin.is_none() && project.is_none() {
            self.new_chat(window, cx);
            self.prefill_home(prompt, window, cx);
            return;
        }
        let generation = self.deep_link.generation;
        let epoch = self.navigation_epoch;
        let host = self.host.clone();
        let cwd = self.cwd.clone();
        let selection = cx.background_spawn(async move { resolve_project(&*host, &cwd, path, origin, project) });
        cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root.deep_link.generation != generation || root.navigation_epoch != epoch { return; }
                match result {
                    Ok(project) => {
                        root.new_chat(window, cx);
                        if let Some((path, projects)) = project {
                            root.set_projects(projects, cx);
                            root.select_project(path, window, cx);
                        }
                        root.prefill_home(prompt, window, cx);
                    }
                    Err(error) => root.toast(Toast::danger("Cannot open project folder").description(error.to_string()), cx),
                }
            });
        }).detach();
    }

    fn prefill_home(&mut self, prompt: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.home.read(cx).composer.clone();
        if let Some(prompt) = prompt { composer.update(cx, |composer, cx| composer.set_text(&prompt, cx)); }
        window.focus(&composer.focus_handle(cx), cx);
    }
}

fn resolve_project(host: &dyn host::Host, cwd: &str, path: Option<String>, origin: Option<String>, id: Option<String>) -> anyhow::Result<Option<(String, Vec<Project>)>> {
    let projects = host.projects()?;
    let selected = id.as_ref().and_then(|id| projects.iter().find(|project| project.id.as_ref() == id).and_then(|project| project.root.as_ref()).map(|root| PathBuf::from(root.as_ref())));
    let selected = selected.or_else(|| path.map(|path| {
        let path = PathBuf::from(path);
        if path.is_absolute() { path } else { Path::new(cwd).join(path) }
    }).filter(|path| path.is_dir()));
    let selected = selected.or_else(|| {
        let requested = repository_key(origin.as_deref()?)?;
        projects.iter().filter_map(|project| project.root.as_ref()).find_map(|root| {
            let result = std::process::Command::new("git").args(["-C", root.as_ref(), "remote", "get-url", "origin"]).output().ok()?;
            if !result.status.success() { return None; }
            let remote = String::from_utf8(result.stdout).ok()?;
            (repository_key(remote.trim()).as_ref() == Some(&requested)).then(|| PathBuf::from(root.as_ref()))
        })
    });
    let Some(path) = selected else { return Ok(None) };
    let path = host.add_project(&path)?;
    Ok(Some((path, host.projects()?)))
}

fn repository_key(value: &str) -> Option<(String, String)> {
    let url = url::Url::parse(value).ok().or_else(|| {
        let (host, path) = value.split_once(':')?;
        let host = host.rsplit('@').next()?;
        url::Url::parse(&format!("ssh://{host}/{path}")).ok()
    })?;
    let path = url.path().trim_matches('/').trim_end_matches(".git");
    (!path.is_empty()).then(|| (url.host_str().unwrap_or("").to_ascii_lowercase(), path.to_owned()))
}
