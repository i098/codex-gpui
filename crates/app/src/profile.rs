//! Settings ▸ Profile: loads `GET /wham/profiles/me` into the view's global.

use crate::{Root, host::Host};
use anyhow::anyhow;
use gpui::{App, Global, WeakEntity};
use std::sync::Arc;
use ui::screens::{
    computer_use::Load,
    settings::profile::{self as view, State},
};

/// Bumped per load and per account change so a stale response never lands.
#[derive(Default)]
struct Generation(u64);
impl Global for Generation {}

pub fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(Generation::default());
    cx.set_global(State {
        dispatch: Some(Arc::new(move |cx| {
            if let Some(root) = root.upgrade() {
                let host = root.read(cx).host.clone();
                load(host, cx);
            }
        })),
        ..Default::default()
    });
}

fn load(host: Arc<dyn Host>, cx: &mut App) {
    let generation = {
        let g = cx.global_mut::<Generation>();
        g.0 += 1;
        g.0
    };
    cx.update_global::<State, _>(|state, _| state.profile = Load::Loading);
    let task = cx.background_executor().spawn(async move {
        let identity = host
            .identity()
            .ok_or_else(|| anyhow!("Profile requires a ChatGPT account"))?;
        crate::usage::http(host.as_ref(), &identity, "GET", "/wham/profiles/me", None)
    });
    cx.spawn(async move |cx| {
        let result = task.await;
        let _ = cx.update(|cx| {
            if cx.global::<Generation>().0 != generation {
                return;
            }
            let profile = match result {
                Ok(body) => Load::Ready(Arc::new(view::parse(&body))),
                Err(error) => Load::Failed(error.to_string()),
            };
            cx.update_global::<State, _>(|state, _| state.profile = profile);
        });
    })
    .detach();
}

/// The account or host changed: drop the old profile and reload it if the page is open.
pub fn invalidate(cx: &mut App) {
    cx.global_mut::<Generation>().0 += 1;
    cx.update_global::<State, _>(|state, _| state.profile = Load::Loading);
    let state = cx.global::<State>();
    if state.page_open
        && let Some(dispatch) = state.dispatch.clone()
    {
        cx.defer(move |cx| dispatch(cx));
    }
}

pub fn on_event(event: &bridge::Event, cx: &mut App) {
    if let bridge::Event::Notification { method, .. } = event
        && method == "account/updated"
    {
        invalidate(cx);
    }
}
