use crate::host::{Host, Request, json_body};
use gpui::{App, AppContext as _, Entity};
use serde_json::json;
use std::sync::Arc;
use ui::screens::diff::panel::{
    BlobReply, DiffPanel, DiffRequest, FullError, FullFileRequest, FullText,
};

fn configure(panel: &mut DiffPanel, cx: &App) {
    if let Some(settings) = cx.try_global::<crate::host_settings::Settings>() {
        panel.set_preferences(settings.read_all());
    }
}

pub(crate) fn new_panel(
    workspace: &str,
    cwd: &str,
    turns: &[bridge::types::Turn],
    cx: &mut App,
) -> Entity<DiffPanel> {
    cx.new(|cx| {
        let mut panel = DiffPanel::last_turn(workspace, cwd, turns);
        configure(&mut panel, cx);
        panel
    })
}

pub(crate) fn read_file(host: &dyn Host, cwd: &str, path: &str) -> anyhow::Result<String> {
    let response = json_body(&host.fetch(&Request::post(
        "read-file",
        Some(json!({"cwd":cwd,"path":path})),
    ))?)?;
    response["contents"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("file response has no text contents"))
}

fn fetch(host: &dyn Host, request: &FullFileRequest) -> Result<FullText, FullError> {
    let result = host
        .fetch(&Request::post(
            "git-cat-file",
            Some(json!({
                "cwd": request.cwd, "requests": request.requests, "maxObjectBytes": 5_242_880,
            })),
        ))
        .and_then(|value| json_body(&value))
        .and_then(|value| serde_json::from_value::<Vec<BlobReply>>(value).map_err(Into::into));
    match result {
        Ok(replies) => request.decode(replies),
        Err(error) => {
            eprintln!("codex-gpui: cannot load diff file: {error:#}");
            Err(FullError::Failed)
        }
    }
}

pub(crate) fn load_requested(
    panel: Entity<DiffPanel>,
    host: Arc<dyn Host>,
    request: FullFileRequest,
    cx: &mut App,
) {
    let target = panel.downgrade();
    let read = cx.background_executor().spawn(async move {
        let result = fetch(host.as_ref(), &request);
        (request, result)
    });
    cx.spawn(async move |cx| {
        let (request, result) = read.await;
        let _ = target.update(cx, |panel, cx| panel.full_file_loaded(&request, result, cx));
    })
    .detach();
}

pub fn bind_standalone_diff(panel: &Entity<DiffPanel>, host: Arc<dyn Host>, cx: &mut App) {
    panel.update(cx, |panel, cx| configure(panel, cx));
    cx.subscribe(panel, move |panel, event: &DiffRequest, cx| match event {
        DiffRequest::LoadFullFile(request) => {
            load_requested(panel, host.clone(), request.clone(), cx)
        }
        DiffRequest::LoadPreview(request) => {
            crate::diff_preview::load(panel, host.clone(), request.clone(), cx)
        }
        DiffRequest::Preference(path, value) => {
            if let Some(key) = path.strip_prefix("host.")
                && cx.has_global::<crate::host_settings::Settings>()
                && let Err(error) = cx
                    .global_mut::<crate::host_settings::Settings>()
                    .write(key, value.clone())
            {
                eprintln!("codex-gpui: cannot save diff preference: {error:#}");
            }
        }
        _ => {}
    })
    .detach();
}
