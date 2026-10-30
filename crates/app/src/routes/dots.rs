use crate::*;
use gpui::Div;
use ui::OpenDots;
use ui::screens::dots::{self, Page, Primary, Request, State};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenDots, window, cx| this.navigate(Route::Dots, window, cx)),
    )
}

pub(super) fn page(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    let page = cx.new(|_| Page::new());
    cx.subscribe_in(&page, window, |this, page, _: &Primary, window, cx| {
        this.dots_primary(page.clone(), window, cx)
    })
    .detach();
    root.load_dots(page.clone(), cx);
    root.dots_page = Some(page.clone());
    dots::shell(page, cx)
}

/// `GET /accounts/check` plus statsig gate `1207782907`, then the original's `ve()` decision.
fn check(host: &dyn host::Host) -> (State, String) {
    let Some(identity) = host.identity() else {
        return (State::LoadError, String::new());
    };
    let path = "/accounts/check/v4-2023-04-27";
    let Ok(accounts) = usage::http(host, &identity, "GET", path, None) else {
        return (State::LoadError, identity.account_id);
    };
    let gates = usage::http(host, &identity, "POST", "/wham/statsig/bootstrap", Some(json!({
        "app_session_id": host::device_id(), "app_version": "26.1007.30912", "brand_name": "chatgpt",
        "build_flavor": "prod", "locale": "en-US", "stable_id": identity.device_id,
        "system_name": std::env::consts::OS, "window_type": "electron",
    }))).and_then(usage::payload_gates).unwrap_or(Value::Null);
    let gate = gates["feature_gates"]["1207782907"]["value"] == true;
    let state = dots::eligibility(&accounts["accounts"][&identity.account_id], gate);
    (state, identity.account_id)
}

impl Root {
    pub(crate) fn load_dots(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.state = State::Loading;
            cx.notify();
            page.generation
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move { check(host.as_ref()) });
        cx.spawn(async move |_, cx| {
            let (state, account_id) = read.await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.state = state;
                    page.account_id = account_id;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The modal's submit (`onAction`) per state; dismissing actions return to `/`.
    fn dots_primary(&mut self, page: Entity<Page>, window: &mut Window, cx: &mut Context<Self>) {
        let (state, id) = (page.read(cx).state, page.read(cx).account_id.clone());
        match state {
            State::Loading | State::BusinessMember(Some(Request::Pending)) => {}
            State::LoadError => self.load_dots(page, cx),
            State::Available => cx.open_url("https://chatgpt.com/dots/home"),
            State::Upgrade => cx.open_url("https://chatgpt.com/#pricing"),
            State::BusinessOwner => cx.open_url(&format!(
                "https://chatgpt.com/admin/members?account_id={id}&manage_seats={id}"
            )),
            State::BusinessMember(Some(Request::Ready)) => self.request_seat(page, id, cx),
            _ => self.navigate(Route::Home, window, cx),
        }
    }

    /// `POST /accounts/{id}/invites/request` (the seat-request mutation), then `Request sent`.
    fn request_seat(&self, page: Entity<Page>, id: String, cx: &mut Context<Self>) {
        page.update(cx, |page, cx| {
            page.state = State::BusinessMember(Some(Request::Pending));
            cx.notify();
        });
        let host = self.host.clone();
        let send = cx.background_spawn(async move {
            let identity = host.identity().ok_or(())?;
            let url = format!("/accounts/{id}/invites/request");
            usage::http(host.as_ref(), &identity, "POST", &url, None).map_err(|_| ())
        });
        cx.spawn(async move |_, cx| {
            let request = if send.await.is_ok() {
                Request::Sent
            } else {
                Request::Ready
            };
            page.update(cx, |page, cx| {
                page.state = State::BusinessMember(Some(request));
                cx.notify();
            });
        })
        .detach();
    }
}
