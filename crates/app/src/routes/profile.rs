use crate::*;
use gpui::Div;
use ui::screens::profile::{self, Event, Load, OpenProfile, Page};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenProfile, window, cx| this.navigate(Route::Profile, window, cx)),
    )
}

/// `/profile`: the signed-in user's profile page.
pub(super) fn page(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    let clock = root.clock.unwrap_or(crate::Clock {
        utc_offset: crate::utc_offset(),
        pinned_now: None,
        elapsed_since: None,
    });
    // The original's `todayIso` is the local calendar date.
    let today = chrono::DateTime::from_timestamp(clock.now() + clock.utc_offset, 0)
        .unwrap_or_default()
        .date_naive();
    let page = cx.new(|_| Page::new(today));
    cx.subscribe_in(
        &page,
        window,
        |this, page, event: &Event, window, cx| match event {
            Event::Refresh => this.load_profile(page.clone(), cx),
            Event::Browse => {
                this.open_plugins("/skills", window, cx);
            }
        },
    )
    .detach();
    root.load_profile(page.clone(), cx);
    profile::shell(page, cx)
}

impl Root {
    /// The original's `profile/page` query: `GET /profiles/me/page` for the signed-in
    /// account, then the profile picture and plugin logos.
    fn load_profile(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.load = Load::Loading;
            cx.notify();
            page.generation
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let identity = host.identity().ok_or_else(|| "not signed in".to_owned())?;
            crate::usage::http(host.as_ref(), &identity, "GET", "/profiles/me/page", None)
                .map_err(|error| error.to_string())
                .and_then(|body| profile::parse(&body))
        });
        let host = self.host.clone();
        cx.spawn(async move |_, cx| {
            let result = read.await;
            if let Err(error) = &result {
                eprintln!("codex-gpui: profile: {error}");
            }
            let urls = result.as_ref().ok().map(|profile| {
                let logos: Vec<_> = profile
                    .plugins
                    .iter()
                    .flatten()
                    .filter_map(|p| Some((p.id.clone(), p.logo_url.clone()?)))
                    .collect();
                (profile.image_url.clone(), logos)
            });
            let current = page.update(cx, |page, cx| {
                let current = page.generation == generation;
                if current {
                    page.load = result.map_or_else(Load::Failed, |p| Load::Ready(Box::new(p)));
                    cx.notify();
                }
                current
            });
            let Some((avatar, logos)) = urls.filter(|_| current) else {
                return;
            };
            let images = cx
                .background_spawn(async move {
                    let avatar =
                        avatar.and_then(|url| super::maps::image(host.as_ref(), &url).ok());
                    let logos = logos
                        .into_iter()
                        .filter_map(|(id, url)| {
                            Some((id, super::maps::image(host.as_ref(), &url).ok()?))
                        })
                        .collect();
                    (avatar, logos)
                })
                .await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    (page.avatar, page.logos) = images;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
