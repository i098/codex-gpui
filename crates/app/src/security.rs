use crate::*;
use ui::screens::security::{self, Action, Project, SecurityPage};

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &ui::OpenSecurity, window, cx| {
        this.navigate(Route::Security, window, cx);
    }))
}

pub(super) fn bind(cx: &mut Context<Root>) {
    cx.observe_global::<settings::integrations::State>(|root, cx| {
        let state = cx.global::<settings::integrations::State>();
        let mut access = if state.gates_loaded && !root.bridge.is_acp() {
            security::Access::from_evaluations(&state.evaluations, false, true)
        } else {
            security::Access::default()
        };
        if let Load::Ready(plugins) = &state.plugins {
            access.plugin_available = plugins["marketplaces"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|marketplace| marketplace["plugins"].as_array().into_iter().flatten())
                .any(bridge::security::Gate::plugin_ready);
        }
        if cx.try_global::<security::Access>() != Some(&access) {
            cx.set_global(access);
            root.shell.update(cx, |_, cx| cx.notify());
        }
    })
    .detach();
}

pub(super) fn load(root: &mut Root, cx: &mut Context<Root>) {
    if !root.bridge.is_acp() {
        root.load_settings_integrations(cx);
    }
}

pub(super) fn shell(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    let model = root.config["model"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            root.models
                .iter()
                .find(|model| model.is_default)
                .map(|model| model.model.clone())
        })
        .unwrap_or_default();
    let effort = root.config["model_reasoning_effort"]
        .as_str()
        .or_else(|| {
            root.models
                .iter()
                .find(|entry| entry.model == model)
                .map(|entry| entry.default_reasoning_effort.as_str())
        })
        .unwrap_or("medium")
        .to_owned();
    let (shell, page) = security::shell(
        root.bridge.clone(),
        Vec::new(),
        root.models.clone(),
        model,
        effort,
        cx,
    );
    cx.subscribe_in(
        &page,
        window,
        |this, page, action, window, cx| match action {
            Action::ViewThread(id) => this.navigate(Route::Thread(id.clone()), window, cx),
            Action::PickFolder => pick_folder(page.clone(), cx),
            Action::OpenPath(path) => open_source(page, path, cx),
            _ => {}
        },
    )
    .detach();
    let host = root.host.clone();
    let page = page.downgrade();
    cx.spawn(async move |_, cx| {
        let projects = cx.background_spawn(async move { host.projects() }).await;
        if let Ok(projects) = projects {
            page.update(cx, |page, cx| {
                page.set_projects(
                    projects
                        .into_iter()
                        .filter(|project| !project.missing)
                        .filter_map(|project| {
                            Some(Project {
                                id: project.id.to_string(),
                                path: project.root?.to_string(),
                                name: project.label.to_string(),
                            })
                        })
                        .collect(),
                    cx,
                )
            })
            .ok();
        }
    })
    .detach();
    shell
}

fn open_source(page: &Entity<SecurityPage>, path: &str, cx: &mut Context<Root>) {
    let workspace = page.read(cx).target_path();
    let path = workspace.join(path);
    let file = cx.new(|_| {
        FileView::new(
            &workspace.to_string_lossy(),
            path.to_string_lossy().into_owned(),
            None,
        )
    });
    page.update(cx, |page, cx| page.show_file(file.clone(), cx));
    cx.spawn(async move |_, cx| {
        let contents = cx
            .background_spawn(async move {
                std::fs::read_to_string(path).map_err(|error| error.to_string())
            })
            .await;
        file.update(cx, |file, cx| file.set_contents(contents, cx));
    })
    .detach();
}

fn pick_folder(page: Entity<SecurityPage>, cx: &mut Context<Root>) {
    let selected = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Select a repository".into()),
    });
    cx.spawn(async move |_, cx| match selected.await {
        Ok(Ok(Some(paths))) => {
            if let Some(path) = paths.first() {
                page.update(cx, |page, cx| {
                    page.set_target(path.to_string_lossy().into_owned(), cx)
                });
            }
        }
        Ok(Err(error)) => {
            page.update(cx, |page, cx| page.show_error(&error.to_string(), cx));
        }
        _ => {}
    })
    .detach();
}
