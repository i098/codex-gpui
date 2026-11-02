use crate::host::Request;
use crate::*;
use gpui::Div;
use ui::OpenSites;
use ui::screens::sites::{self, Event, Page};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenSites, window, cx| this.navigate(Route::Sites, window, cx)),
    )
}

/// `/sites`, or a row's `/sites/:projectId` when `site` is set.
pub(super) fn page(
    root: &mut Root,
    site: Option<String>,
    window: &mut Window,
    cx: &mut Context<Root>,
) -> Entity<Shell> {
    let clock = root.clock.unwrap_or(crate::Clock {
        utc_offset: crate::utc_offset(),
        pinned_now: None,
        elapsed_since: None,
    });
    let page = cx.new(|_| match site {
        Some(id) => Page::detail(clock, id),
        None => Page::new(clock),
    });
    cx.subscribe_in(&page, window, |this, page, event: &Event, window, cx| {
        match event {
            Event::Refresh => this.load_sites(page.clone(), cx),
            Event::Open(id) => this.navigate(Route::Site(id.clone()), window, cx),
            // The original's `Edit` continues the site in a chat that mentions it.
            Event::Edit(site) => this.plugin_prefill(
                &format!("[@{}](sites-project://{}) ", site.title, site.id),
                window,
                cx,
            ),
        }
    })
    .detach();
    root.load_sites(page.clone(), cx);
    root.sites_page = Some(page.clone());
    sites::shell(page, cx)
}

/// The original's sites queries (`appgen-sites-query`): the owned list's first page, or one site.
fn request(url: String) -> Request {
    Request::Http(json!({
        "url": url,
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
        let (generation, url) = page.update(cx, |page, cx| {
            page.generation += 1;
            page.set_loading();
            cx.notify();
            (page.generation, page.url())
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            host.fetch(&request(url))
                .and_then(|response| host::json_body(&response))
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let body = read.await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.set_body(body);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
