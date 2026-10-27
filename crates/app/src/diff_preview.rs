use crate::host::{Host, Request, json_body};
use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui::{App, Entity};
use serde_json::json;
use std::sync::Arc;
use ui::screens::diff::panel::{DiffPanel, PreviewContent, PreviewRequest, PreviewSource};

fn read(
    host: &dyn Host,
    request: &PreviewRequest,
    svg: gpui::SvgRenderer,
) -> Result<Option<PreviewContent>> {
    if let Some(bytes) = &request.data {
        return PreviewContent::decode(bytes.clone(), None, request, svg).map(Some);
    }
    let (endpoint, params) = match &request.source {
        PreviewSource::Git {
            cwd,
            path,
            reference,
        } => (
            "read-git-file-binary",
            json!({"cwd":cwd,"path":path,"ref":reference}),
        ),
        PreviewSource::Worktree { cwd, path } => {
            ("read-file-binary", json!({"cwd":cwd,"path":path}))
        }
    };
    let response = json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)?;
    let Some(base64) = response["contentsBase64"].as_str() else {
        return Ok(None);
    };
    let bytes = Arc::new(
        STANDARD
            .decode(base64)
            .context("Invalid binary file response")?,
    );
    PreviewContent::decode(bytes, response["mimeType"].as_str(), request, svg).map(Some)
}

pub(crate) fn load(
    panel: Entity<DiffPanel>,
    host: Arc<dyn Host>,
    request: PreviewRequest,
    cx: &mut App,
) {
    let target = panel.downgrade();
    let svg = cx.svg_renderer();
    let task = cx.background_executor().spawn(async move {
        let result = read(host.as_ref(), &request, svg);
        if let Err(error) = &result {
            eprintln!("codex-gpui: cannot load diff preview: {error:#}");
        }
        (request, result)
    });
    cx.spawn(async move |cx| {
        let (request, result) = task.await;
        let _ = target.update(cx, |panel, cx| panel.preview_loaded(&request, result, cx));
    })
    .detach();
}
