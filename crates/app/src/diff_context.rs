use crate::host::{Host, Request, json_body};
use gpui::{App, Entity, Task};
use std::sync::Arc;
use ui::screens::diff::panel::{ContextRequest, DiffPanel, OpenTargetRequest, OpenTargets};

pub(crate) fn load(
    panel: Entity<DiffPanel>,
    host: Arc<dyn Host>,
    request: ContextRequest,
    cx: &mut App,
) {
    let target = panel.downgrade();
    let window = request.window;
    let read = cx.background_executor().spawn(async move {
        let params = serde_json::json!({"cwd":request.cwd,"path":request.path,"line":request.line.map(|(_, line)| line)});
        let catalog = host.fetch(&Request::post("open-in-targets", Some(params.clone())))
            .and_then(|response| json_body(&response))
            .map_err(|error| error.to_string());
        let github = if request.new_tab {
            host.fetch(&Request::post("git-file-url", Some(params))).ok()
                .and_then(|response| json_body(&response).ok())
                .and_then(|value| value["url"].as_str().map(str::to_owned))
        } else { None };
        (request, catalog, github)
    });
    cx.spawn(async move |cx| {
        let (request, catalog, github) = read.await;
        let _ = window.update(cx, |_, window, cx| {
            let catalog = catalog.and_then(|value| {
                serde_json::from_value::<OpenTargets>(value).map_err(|error| error.to_string())
            });
            let _ = target.update(cx, |panel, cx| {
                panel.context_menu_loaded(request, catalog, github, window, cx)
            });
        });
    })
    .detach();
}

pub(crate) fn open(
    host: Arc<dyn Host>,
    request: &OpenTargetRequest,
    cx: &mut App,
) -> Task<anyhow::Result<serde_json::Value>> {
    let params = serde_json::json!({
        "cwd":request.cwd, "path":request.path, "line":request.line,
        "target":request.target, "appPath":request.app_path,
        "persistPreferredTargetPath":request.persist_preferred_target_path,
    });
    cx.background_executor()
        .spawn(async move { host.fetch(&Request::post("open-file", Some(params))) })
}

impl crate::Root {
    pub(super) fn launch_diff_target(
        &mut self,
        request: &OpenTargetRequest,
        cx: &mut gpui::Context<Self>,
    ) {
        let opened = open(self.host.clone(), request, cx);
        cx.spawn(async move |this, cx| {
            if let Err(error) = opened.await {
                let _ = this.update(cx, |this, cx| {
                    this.toast(
                        ui::toast::Toast::danger(format!("Could not open file: {error}")),
                        cx,
                    );
                });
            }
        })
        .detach();
    }
}
