//! ChatGPT backend calls for the account settings pages. The app installs [`Backend`]; its host
//! attaches the ChatGPT sign-in, like the original's `safeGet`/`safePost` client.

use gpui::{App, Context, Global};
use serde_json::Value;
use std::sync::Arc;

/// One request to the ChatGPT backend: `path` is relative to `/backend-api`.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub method: &'static str,
    pub path: String,
    pub body: Option<Value>,
}

impl Call {
    pub fn new(method: &'static str, path: impl Into<String>, body: Option<Value>) -> Self {
        Self {
            method,
            path: path.into(),
            body,
        }
    }
}

/// A finished call: the HTTP status and the JSON body (`Null` when the body is empty or not
/// JSON).
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Value,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The backend's error code: `detail.error_code`, `detail.code`, `error_code` or `code`.
    pub fn error_code(&self) -> Option<&str> {
        let detail = &self.body["detail"];
        [
            &detail["error_code"],
            &detail["code"],
            &self.body["error_code"],
            &self.body["code"],
        ]
        .into_iter()
        .find_map(Value::as_str)
    }
}

pub type Reply = Box<dyn FnOnce(anyhow::Result<Response>, &mut App)>;
pub type Dispatch = Arc<dyn Fn(Call, Reply, &mut App)>;

#[derive(Clone, Default)]
pub struct Backend {
    pub dispatch: Option<Dispatch>,
}

impl Global for Backend {}

/// Sends `call` and answers `reply` on the main thread. Without an installed backend the reply
/// is an error at once.
pub fn send(call: Call, reply: Reply, cx: &mut App) {
    match cx.try_global::<Backend>().and_then(|b| b.dispatch.clone()) {
        Some(dispatch) => dispatch(call, reply, cx),
        None => reply(
            Err(anyhow::anyhow!("The ChatGPT backend is unavailable")),
            cx,
        ),
    }
}

/// [`send`] for a page entity: `done` runs on the page (if it still exists), then the page
/// redraws.
pub fn request<T: 'static>(
    call: Call,
    cx: &mut Context<T>,
    done: impl FnOnce(&mut T, anyhow::Result<Response>, &mut Context<T>) + 'static,
) {
    let page = cx.weak_entity();
    let reply: Reply = Box::new(move |result, cx| {
        page.update(cx, |page, cx| {
            done(page, result, cx);
            cx.notify();
        })
        .ok();
    });
    send(call, reply, cx);
}
