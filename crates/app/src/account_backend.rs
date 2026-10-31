//! Runs the account settings pages' ChatGPT backend calls through the root's host.
use crate::{Root, host};
use anyhow::Context as _;
use gpui::{App, AppContext as _, Context, WeakEntity};
use std::sync::Arc;
use ui::screens::settings::backend::{Backend, Call, Reply, Response};

pub(crate) fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(Backend {
        dispatch: Some(Arc::new(move |call, reply, cx| {
            root.update(cx, |root, cx| send(root, call, reply, cx)).ok();
        })),
    });
}

fn send(root: &Root, call: Call, reply: Reply, cx: &mut Context<Root>) {
    let host = root.host.clone();
    let (method, path) = (call.method, call.path.clone());
    let response = cx.background_spawn(async move {
        let mut body = call.body;
        if let Some(field) = call.account_user_field {
            let user = host
                .account_user_id()
                .context("Missing authenticated identity")?;
            body.get_or_insert_with(|| serde_json::json!({}))[field] = user.into();
        }
        let request = host::backend(call.method, &call.path, body.as_ref());
        let (status, body) = host::backend_response(&host.fetch(&request)?)?;
        anyhow::Ok(Response { status, body })
    });
    cx.spawn(async move |_, cx| {
        let response = response.await;
        if let Err(error) = &response {
            eprintln!("codex-gpui: {method} {path}: {error:#}");
        }
        cx.update(|cx| reply(response, cx));
    })
    .detach();
}
