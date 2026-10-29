use crate::host::{Host, Request, json_body};
use anyhow::{Context as _, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui::{App, AppContext as _, Entity};
use serde_json::json;
use std::sync::Arc;
use ui::screens::diff::panel::{
    BlobError, BlobReply, DiffPanel, DiffRequest, FullError, FullFileLoad, FullFileRequest,
    FullText,
};

pub fn read_full_file(host: &dyn Host, request: &FullFileRequest) -> Result<FullText, FullError> {
    let result = host
        .fetch(&Request::post(
            "git-cat-file",
            Some(json!({
                "cwd": request.cwd,
                "requests": request.requests,
                "maxObjectBytes": 5_242_880,
            })),
        ))
        .and_then(|response| json_body(&response))
        .and_then(|body| serde_json::from_value::<Vec<BlobReply>>(body).map_err(Into::into))
        .and_then(|mut replies| {
            if let Some(index) = request.before
                && let Some(before) = request.requests.get(index)
                && before.oid.is_none()
            {
                let reply = replies
                    .get_mut(index)
                    .context("missing before-file reply")?;
                *reply = read_head_lines(host, &request.cwd, &before.path)?;
            }
            Ok(replies)
        });
    match result {
        Ok(replies) => request.decode(replies),
        Err(error) => {
            eprintln!("codex-gpui: cannot load diff file: {error:#}");
            Err(FullError::Failed)
        }
    }
}

fn read_head_lines(host: &dyn Host, cwd: &str, path: &str) -> anyhow::Result<BlobReply> {
    let body = json_body(&host.fetch(&Request::post(
        "read-git-file-binary",
        Some(json!({"cwd":cwd,"path":path,"ref":"head","maxBytes":5_242_880})),
    ))?)?;
    let bytes = match body.get("contentsBase64") {
        Some(serde_json::Value::String(encoded)) => STANDARD.decode(encoded)?,
        Some(serde_json::Value::Null) => {
            return Ok(BlobReply::Error {
                error: BlobError {
                    kind: "not-found".into(),
                },
            });
        }
        _ => bail!("invalid before-file reply"),
    };
    if bytes.len() > 5_242_880 {
        return Ok(BlobReply::Error {
            error: BlobError {
                kind: "too-large".into(),
            },
        });
    }
    Ok(BlobReply::Success {
        lines: String::from_utf8_lossy(&bytes)
            .split_inclusive('\n')
            .map(str::to_owned)
            .collect(),
    })
}

fn configure(panel: &mut DiffPanel, cx: &App) {
    if let Some(settings) = cx.try_global::<crate::host_settings::Settings>() {
        panel.set_preferences(&settings.read_all());
    } else {
        panel.set_preferences(&serde_json::Map::new());
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

pub(crate) fn load_requested(
    panel: Entity<DiffPanel>,
    host: Arc<dyn Host>,
    request: FullFileLoad,
    cx: &mut App,
) {
    let target = panel.downgrade();
    let read = cx.background_executor().spawn(async move {
        let result = read_full_file(host.as_ref(), &request.request);
        (request, result)
    });
    cx.spawn(async move |cx| {
        let (request, result) = read.await;
        let _ = target.update(cx, |panel, cx| panel.full_file_loaded(&request, result, cx));
    })
    .detach();
}

pub(crate) fn save_viewed_file(
    panel: &Entity<DiffPanel>,
    key: String,
    revision: Option<String>,
    cx: &mut App,
) -> anyhow::Result<()> {
    let settings = cx.global_mut::<crate::host_settings::Settings>();
    settings.update("review-viewed-file-v1", |value| {
        let mut viewed = value
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(revision) = &revision {
            viewed.insert(key.clone(), revision.clone().into());
        } else {
            viewed.remove(&key);
        }
        Ok(viewed.into())
    })?;
    panel.update(cx, |panel, cx| panel.viewed_file_saved(key, revision, cx));
    Ok(())
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
        DiffRequest::ContextMenu(request) => {
            crate::diff_context::load(panel, host.clone(), request.clone(), cx);
        }
        DiffRequest::OpenTarget(request) => {
            let opened = crate::diff_context::open(host.clone(), request, cx);
            cx.spawn(async move |_| {
                if let Err(error) = opened.await {
                    eprintln!("codex-gpui: could not open file: {error}");
                }
            })
            .detach();
        }
        DiffRequest::ViewedFile { key, revision } => {
            if let Err(error) = save_viewed_file(&panel, key.clone(), revision.clone(), cx) {
                eprintln!("codex-gpui: cannot save viewed file: {error:#}");
            }
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
