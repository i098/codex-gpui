use crate::*;
use bridge::security_cloud::{Client, OPEN_STATUSES};
use gpui::Div;
use ui::OpenSecurityCloud;
use ui::screens::security_cloud::{self, Page, Refresh};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &OpenSecurityCloud, window, cx| {
        this.navigate(Route::SecurityCloud, window, cx)
    }))
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let clock = root.clock.unwrap_or(crate::Clock {
        utc_offset: crate::utc_offset(),
        pinned_now: None,
        elapsed_since: None,
    });
    let page = cx.new(|cx| Page::new(clock, cx));
    cx.subscribe(&page, |this, page, refresh: &Refresh, cx| {
        this.load_security_cloud(page, refresh.all, cx)
    })
    .detach();
    root.load_security_cloud(page.clone(), false, cx);
    root.security_cloud_page = Some(page.clone());
    security_cloud::shell(page, cx)
}

impl Root {
    pub(crate) fn load_security_cloud(
        &self,
        page: Entity<Page>,
        all: bool,
        cx: &mut Context<Self>,
    ) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.all = all;
            page.snapshot = None;
            cx.notify();
            page.generation
        });
        let bridge = self.bridge.clone();
        cx.spawn(async move |_, cx| {
            let snapshot = match Client::connect(&bridge).await {
                Ok(client) => {
                    let snapshot = client.snapshot(if all { "" } else { OPEN_STATUSES }).await;
                    client.close().await;
                    snapshot
                }
                Err(error) => bridge::security_cloud::Snapshot::failed(error.to_string()),
            };
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.snapshot = Some(snapshot);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
