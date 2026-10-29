mod model;

pub use model::{CALLBACK_URL, CallbackRoute, ConnectRequest, ConnectedApp};

use crate::{Root, host};
use gpui::{AppContext as _, Context, Entity, Window};
use std::{collections::HashMap, rc::Rc, sync::Arc};
use ui::screens::oauth::{CallbackPage, CallbackState};
use ui::toast::Toast;
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionEvent {
    Opened,
    Connected(ConnectedApp),
    Failed,
    Replaced,
    Cancelled,
}

type Completion = Rc<dyn Fn(&mut Root, &ConnectionEvent, &mut Window, &mut Context<Root>)>;

struct Pending {
    attempt: u64,
    request: Arc<ConnectRequest>,
    identity: host::Identity,
    on_event: Completion,
    claimed: bool,
    window: gpui::AnyWindowHandle,
}

#[derive(Default)]
pub(crate) struct Connections {
    next_attempt: u64,
    latest: HashMap<String, HashMap<String, u64>>,
    pending: HashMap<String, Pending>,
    pub page: Option<Entity<CallbackPage>>,
    page_attempt: Option<u64>,
}

impl Connections {
    pub fn leave_page(&mut self) {
        self.page = None;
        self.page_attempt = None;
    }

    fn current(&self, request: &ConnectRequest, attempt: u64) -> bool {
        self.latest
            .get(&request.host_id)
            .and_then(|attempts| attempts.get(&request.connector_id))
            == Some(&attempt)
    }

    fn clear_page(&mut self, attempt: u64) {
        if self.page_attempt == Some(attempt) {
            self.leave_page();
        }
    }
}

impl Root {
    pub(crate) fn owns_oauth_state(&self, state: &str) -> bool {
        self.oauth.pending.contains_key(state)
    }
    pub(crate) fn invalidate_oauth(&mut self, cx: &mut Context<Self>) {
        self.oauth.latest.clear();
        self.oauth.leave_page();
        let pending = std::mem::take(&mut self.oauth.pending);
        let root = cx.weak_entity();
        cx.defer(move |cx| {
            for pending in pending.into_values() {
                pending
                    .window
                    .update(cx, |_, window, cx| {
                        root.update(cx, |root, cx| {
                            (pending.on_event)(root, &ConnectionEvent::Replaced, window, cx);
                        })
                        .ok();
                    })
                    .ok();
            }
        });
        cx.notify();
    }

    pub(crate) fn oauth_account_event(&mut self, event: &bridge::Event, cx: &mut Context<Self>) {
        if matches!(event, bridge::Event::Notification { method, .. } if method == "account/updated")
        {
            self.invalidate_oauth(cx);
        }
    }
    /// Starts the original connector OAuth request and opens its provider URL.
    /// The callback runs on this window for start, completion, failure, replacement, or cancellation.
    /// No redirect URL or OAuth state enters persisted window settings.
    pub fn start_app_connection(
        &mut self,
        request: ConnectRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_event: impl Fn(&mut Root, &ConnectionEvent, &mut Window, &mut Context<Root>) + 'static,
    ) {
        self.oauth.next_attempt += 1;
        let attempt = self.oauth.next_attempt;
        self.oauth
            .latest
            .entry(request.host_id.clone())
            .or_default()
            .insert(request.connector_id.clone(), attempt);
        let request = Arc::new(request);
        let on_event: Completion = Rc::new(on_event);
        let host = self.host.clone();
        let read_host = host.clone();
        let read_request = request.clone();
        let start = cx.background_spawn(async move {
            read_host.invalidate_identity();
            let identity = read_host
                .identity()
                .ok_or_else(|| anyhow::anyhow!("no signed-in account"))?;
            let redirect = read_request.start(read_host.as_ref(), &identity)?;
            Ok::<_, anyhow::Error>((identity, redirect))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = start.await;
            this.update_in(cx, |this, window, cx| {
                if !Arc::ptr_eq(&this.host, &host) || !this.oauth.current(&request, attempt) {
                    on_event(this, &ConnectionEvent::Replaced, window, cx);
                    return;
                }
                match result {
                    Ok((identity, redirect)) => {
                        let Some(state) = model::oauth_state(&redirect) else {
                            return;
                        };
                        if this.oauth.pending.contains_key(&state) {
                            this.toast(Toast::danger("Failed to connect app"), cx);
                            on_event(this, &ConnectionEvent::Failed, window, cx);
                            return;
                        }
                        this.oauth.pending.insert(
                            state,
                            Pending {
                                attempt,
                                request,
                                identity,
                                on_event: on_event.clone(),
                                claimed: false,
                                window: window.window_handle(),
                            },
                        );
                        cx.open_url(redirect.as_str());
                        on_event(this, &ConnectionEvent::Opened, window, cx);
                    }
                    Err(_) => {
                        this.toast(Toast::danger("Failed to connect app"), cx);
                        on_event(this, &ConnectionEvent::Failed, window, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Consumes an original callback URL. Excluded ChatGPT and mobile products remain gated.
    /// Unknown states never send a request or report a successful connection.
    pub fn receive_oauth_redirect(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(route) = CallbackRoute::from_url(value) else {
            return false;
        };
        if !route.is_codex() {
            return true;
        }
        let Some(state) = Url::parse(value)
            .ok()
            .and_then(|url| model::oauth_state(&url))
        else {
            self.toast(Toast::info("Missing OAuth callback data"), cx);
            return true;
        };
        let Some(pending) = self.oauth.pending.get(&state) else {
            self.toast(Toast::info("Missing OAuth callback data"), cx);
            return true;
        };
        if pending.claimed {
            return true;
        }
        let attempt = pending.attempt;
        let request = pending.request.clone();
        let identity = pending.identity.clone();
        let on_event = pending.on_event.clone();
        if !self.oauth.current(&request, attempt) {
            self.oauth.pending.remove(&state);
            if self.oauth.page.is_none() {
                let page = cx.new(|cx| CallbackPage::new(cx));
                page.update(cx, |page, _| page.state = CallbackState::Superseded);
                self.oauth.page = Some(page);
                self.oauth.page_attempt = Some(attempt);
            }
            on_event(self, &ConnectionEvent::Replaced, window, cx);
            cx.notify();
            return true;
        }
        self.oauth
            .pending
            .get_mut(&state)
            .expect("pending callback exists")
            .claimed = true;
        self.command_menu = None;
        self.dialog = None;
        self.oauth.page = Some(cx.new(|cx| CallbackPage::new(cx)));
        self.oauth.page_attempt = Some(attempt);
        cx.notify();
        let host = self.host.clone();
        let read_host = host.clone();
        let redirect = value.to_owned();
        let complete = cx.background_spawn(async move {
            model::complete(read_host.as_ref(), &identity, &request, &redirect)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = complete.await;
            this.update_in(cx, |this, window, cx| {
                if !Arc::ptr_eq(&this.host, &host) {
                    return;
                }
                let Some(pending) = this.oauth.pending.get_mut(&state) else {
                    return;
                };
                pending.claimed = false;
                let request = pending.request.clone();
                let current = this.oauth.current(&request, attempt);
                if !current {
                    this.oauth.pending.remove(&state);
                    if this.oauth.page_attempt == Some(attempt)
                        && let Some(page) = &this.oauth.page
                    {
                        page.update(cx, |page, cx| {
                            page.state = CallbackState::Superseded;
                            cx.notify();
                        });
                    }
                    on_event(this, &ConnectionEvent::Replaced, window, cx);
                    return;
                }
                this.oauth.clear_page(attempt);
                match result {
                    Ok(app) => {
                        let pending = this
                            .oauth
                            .pending
                            .remove(&state)
                            .expect("pending callback exists");
                        let name = app
                            .name
                            .as_deref()
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                            .unwrap_or(&pending.request.name);
                        this.toast(Toast::success(format!("{name} is now connected")), cx);
                        on_event(this, &ConnectionEvent::Connected(app), window, cx);
                    }
                    Err(_) => {
                        this.toast(Toast::danger("Failed to finish connecting app"), cx);
                        on_event(this, &ConnectionEvent::Failed, window, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        true
    }

    pub fn cancel_app_connection(
        &mut self,
        host_id: &str,
        connector_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(attempts) = self.oauth.latest.get_mut(host_id) {
            attempts.remove(connector_id);
        }
        let states: Vec<_> = self
            .oauth
            .pending
            .iter()
            .filter(|(_, pending)| pending.request.key() == (host_id, connector_id))
            .map(|(state, _)| state.clone())
            .collect();
        for state in states {
            if let Some(pending) = self.oauth.pending.remove(&state) {
                self.oauth.clear_page(pending.attempt);
                (pending.on_event)(self, &ConnectionEvent::Cancelled, window, cx);
            }
        }
        cx.notify();
    }

    pub fn oauth_page(&self) -> Option<&Entity<CallbackPage>> {
        self.oauth.page.as_ref()
    }
}
