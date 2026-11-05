//! Runs the account settings pages' ChatGPT backend calls through the root's host.
use crate::{Root, host};
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
    let request = host::backend(call.method, &call.path, call.body.as_ref());
    let response = cx.background_spawn(async move {
        let (status, body) = host::backend_response(&host.fetch(&request)?)?;
        anyhow::Ok(Response { status, body })
    });
    cx.spawn(async move |_, cx| {
        let response = response.await;
        if let Err(error) = &response {
            eprintln!("codex-gpui: {} {}: {error:#}", call.method, call.path);
        }
        cx.update(|cx| reply(response, cx));
    })
    .detach();
}
