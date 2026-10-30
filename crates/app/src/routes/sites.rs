use crate::host::Request;
use crate::*;
use gpui::Div;
use ui::OpenSites;
use ui::screens::sites::{self, Load, Page, Refresh};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenSites, window, cx| this.navigate(Route::Sites, window, cx)),
    )
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let clock = root.clock.unwrap_or(crate::Clock {
        utc_offset: crate::utc_offset(),
        pinned_now: None,
        elapsed_since: None,
    });
    let page = cx.new(|_| Page::new(clock));
    cx.subscribe(&page, |this, page, _: &Refresh, cx| {
        this.load_sites(page, cx)
    })
    .detach();
    root.load_sites(page.clone(), cx);
    root.sites_page = Some(page.clone());
    sites::shell(page, cx)
}

/// The original's owned-sites query (`appgen-sites-query` `E("owned")`, first page).
fn request() -> Request {
    Request::Http(json!({
        "url": "/websites?limit=20",
        "method": "GET",
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop",
        },
        "returnErrorResponse": true,
    }))
}

impl Root {
    pub(crate) fn load_sites(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.load = Load::Loading;
            cx.notify();
            page.generation
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            host.fetch(&request())
                .and_then(|response| host::json_body(&response))
                .map_err(|error| error.to_string())
                .and_then(|body| sites::sites(&body))
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.load = result.map_or_else(Load::Failed, Load::Ready);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
