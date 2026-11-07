use crate::host::Host;
use crate::*;
use anyhow::Context as _;
use gpui::{Div, EventEmitter};
use ui::OpenDots;
use ui::screens::dots::{self, Capable, Load, Page, Refresh};

/// The dot itself (`/dots/home` with an available dot) is not native yet: it opens on ChatGPT.
const DOT_HOME: &str = "https://chatgpt.com/dots/home";

/// Emitted by the route once the dot opened elsewhere.
struct Leave;

impl EventEmitter<Leave> for Page {}

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenDots, window, cx| this.navigate(Route::Dots, window, cx)),
    )
}

pub(super) fn page(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    let page = cx.new(|_| Page::new());
    cx.subscribe(&page, |this, page, _: &Refresh, cx| {
        this.load_dots(page, cx)
    })
    .detach();
    cx.subscribe_in(&page, window, |this, _, _: &Leave, window, cx| {
        this.navigate(Route::Home, window, cx)
    })
    .detach();
    root.load_dots(page.clone(), cx);
    dots::shell(page, root.sidebar.clone().into(), cx)
}

/// `GET /accounts/check`: whether the account's dot is available.
fn capable(host: &dyn Host) -> Result<bool> {
    let identity = host
        .identity()
        .context("Your dot requires a ChatGPT account")?;
    let accounts = usage::http(
        host,
        &identity,
        "GET",
        "/accounts/check/v4-2023-04-27",
        None,
    )?;
    Ok(dots::capable(&accounts, &identity.account_id))
}

/// Records a successful read for the sidebar's `Your dot` row.
// shortcut: a failed read keeps the row (the original hides it while not known capable), so
// signed-out or offline launches keep the recorded sidebar; hide it on errors once that is wanted.
fn remember(capable: bool, cx: &mut App) {
    if cx
        .try_global::<Capable>()
        .is_none_or(|known| known.0 != capable)
    {
        cx.set_global(Capable(capable));
        cx.refresh_windows();
    }
}

impl Root {
    /// Reads the dot's availability at startup, for the sidebar row.
    pub(crate) fn load_dots_capability(&self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let read = cx.background_spawn(async move { capable(host.as_ref()) });
        cx.spawn(async move |_, cx| {
            if let Ok(capable) = read.await {
                cx.update(|cx| remember(capable, cx));
            }
        })
        .detach();
    }

    fn load_dots(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.load = Load::Loading;
            cx.notify();
            page.generation
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move { capable(host.as_ref()) });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            if let Ok(capable) = result {
                cx.update(|cx| remember(capable, cx));
            }
            page.update(cx, |page, cx| {
                if page.generation != generation {
                    return;
                }
                page.load = match result {
                    Ok(true) => {
                        cx.open_url(DOT_HOME);
                        cx.emit(Leave);
                        Load::Opened
                    }
                    Ok(false) => Load::Failed("Your dot is unavailable for this account".into()),
                    Err(error) => Load::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }
}
