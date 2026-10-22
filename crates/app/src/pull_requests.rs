use crate::{Root, Route, host};
use anyhow::{Context as _, Result};
use gpui::{
    AppContext as _, Context, Div, Entity, Focusable as _, InteractiveElement as _, Window,
};
use serde_json::{Value, json};
use ui::screens::pull_requests::{
    self as screen, Availability, Event, Page,
    model::{Account, Identity, SearchPage},
};

pub(crate) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &ui::OpenPullRequests, window, cx| {
        this.navigate(Route::PullRequests(None), window, cx)
    }))
}

impl Root {
    pub(crate) fn pull_requests_shell(
        &mut self,
        selected: Option<Identity>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ui::Shell> {
        let page = if let Some(page) = &self.pull_requests_page {
            page.clone()
        } else {
            let clock = self.clock.unwrap_or(crate::Clock {
                utc_offset: crate::utc_offset(),
                pinned_now: None,
                elapsed_since: None,
            });
            let page = cx.new(|cx| Page::new(clock, cx));
            cx.subscribe_in(&page, window, |this, page, event: &Event, window, cx| {
                this.pull_request_event(page.clone(), event, window, cx);
            })
            .detach();
            self.pull_requests_page = Some(page.clone());
            page
        };
        let changed = page.update(cx, |page, cx| page.select(selected, cx));
        let changed_host = page
            .read(cx)
            .selected
            .as_ref()
            .zip(page.read(cx).account())
            .is_some_and(|(selected, account)| {
                !selected.hostname.eq_ignore_ascii_case(&account.hostname)
            });
        if changed_host || matches!(page.read(cx).availability, Availability::Checking) {
            self.check_pull_request_access(page.clone(), cx);
        } else if changed && page.read(cx).account().is_some() {
            self.load_pull_request_detail(page.clone(), cx);
        }
        screen::shell(page, cx)
    }

    fn pull_request_event(
        &mut self,
        page: Entity<Page>,
        event: &Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            Event::Query => self.load_pull_requests(page, false, cx),
            Event::More => self.load_pull_requests(page, true, cx),
            Event::Retry => self.check_pull_request_access(page, cx),
            Event::Select(selected) => {
                self.navigate(Route::PullRequests(selected.clone()), window, cx)
            }
            Event::AskCodex(prompt) => self.prefill_pull_request_chat(prompt, window, cx),
            Event::Media(url) => self.load_pull_request_media(page, url.clone(), cx),
        }
    }

    fn prefill_pull_request_chat(
        &mut self,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Route::Home, window, cx);
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, cx| composer.set_text(prompt, cx));
        composer.focus_handle(cx).focus(window, cx);
    }

    fn check_pull_request_access(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        page.update(cx, |page, cx| {
            page.generation += 1;
            page.set_availability(Availability::Checking, cx);
        });
        let generation = page.read(cx).generation;
        let hostname = page
            .read(cx)
            .selected
            .as_ref()
            .map_or("github.com", |selected| selected.hostname.as_str())
            .to_owned();
        let service = self.host.clone();
        let load = cx.background_spawn(async move {
            let status = service
                .fetch(&host::Request::post(
                    "gh-cli-status",
                    Some(json!({"hostId":"local", "hostname":hostname})),
                ))
                .and_then(|response| host::json_body(&response));
            let user = service
                .fetch(&host::Request::post(
                    "gh-current-user",
                    Some(json!({"hostId":"local", "hostname":hostname})),
                ))
                .and_then(|response| host::json_body(&response));
            anyhow::Ok((status, user))
        });
        cx.spawn(async move |this, cx| {
            let result = load.await;
            let availability = access(result);
            let ready = matches!(availability, Availability::Ready(_));
            let current = page.update(cx, |page, cx| {
                if page.generation != generation {
                    return false;
                }
                page.set_availability(availability, cx);
                true
            });
            if ready && current {
                this.update(cx, |this, cx| {
                    if this.pull_requests_page.as_ref() != Some(&page) {
                        return;
                    }
                    this.load_pull_requests(page.clone(), false, cx);
                    this.load_pull_request_detail(page, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn load_pull_requests(&self, page: Entity<Page>, more: bool, cx: &mut Context<Self>) {
        let Some(account) = page.read(cx).account().cloned() else {
            return;
        };
        let requests = page.update(cx, |page, cx| page.begin_query(more, cx));
        let generation = page.read(cx).generation;
        for (index, filters, cursor) in requests {
            let service = self.host.clone();
            let request = host::Request::post(
                "gh-pr-search",
                Some(
                    json!({"account": account, "filters": filters, "cursor": cursor, "pageSize": 50}),
                ),
            );
            let load = cx.background_spawn(async move {
                let body = host::json_body(&service.fetch(&request)?)?;
                serde_json::from_value::<SearchPage>(body)
                    .context("Unexpected pull request search response")
            });
            let page = page.clone();
            cx.spawn(async move |_, cx| {
                let result = load.await.map_err(|error| format!("{error:#}"));
                page.update(cx, |page, cx| {
                    page.apply_search(generation, index, more, result, cx)
                });
            })
            .detach();
        }
    }

    fn load_pull_request_detail(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let (Some(account), Some(identity)) = (
            page.read(cx).account().cloned(),
            page.read(cx).selected.clone(),
        ) else {
            return;
        };
        page.update(cx, |page, cx| {
            page.detail_generation += 1;
            page.detail = None;
            cx.notify();
        });
        let generation = page.read(cx).detail_generation;
        let login = account.login.clone();
        let service = self.host.clone();
        let load = cx.background_spawn(async move {
            host::json_body(&service.fetch(&host::Request::post(
                "gh-pr-detail",
                Some(json!({"account": account, "pullRequest": identity})),
            ))?)
        });
        cx.spawn(async move |_, cx| {
            let result = load
                .await
                .map_err(|error| repository_error(&login, format!("{error:#}")));
            page.update(cx, |page, cx| page.apply_detail(generation, result, cx));
        })
        .detach();
    }

    fn load_pull_request_media(&self, page: Entity<Page>, url: String, cx: &mut Context<Self>) {
        let (Some(account), Some(identity)) = (
            page.read(cx).account().cloned(),
            page.read(cx).selected.clone(),
        ) else {
            return;
        };
        let generation = page.read(cx).detail_generation;
        let service = self.host.clone();
        let request = host::Request::post(
            "gh-pr-media",
            Some(json!({"account": account, "pullRequest": identity, "url": url})),
        );
        let load = cx.background_spawn(async move {
            service
                .fetch(&request)
                .and_then(|response| host::json_body(&response))
                .map_err(|error| format!("{error:#}"))
                .and_then(screen::decode_image_preview)
        });
        cx.spawn(async move |_, cx| {
            let result = load.await;
            page.update(cx, |page, cx| {
                page.apply_media(generation, &url, result, cx)
            });
        })
        .detach();
    }

    pub fn pull_requests_page(&self) -> Option<&Entity<Page>> {
        self.pull_requests_page.as_ref()
    }

    pub(crate) fn reset_pull_requests(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state.route, Route::PullRequests(_)) {
            if let Some(page) = self.pull_requests_page.clone() {
                page.update(cx, |page, _| {
                    page.streams.clear();
                    page.detail = None;
                });
                self.check_pull_request_access(page, cx);
            }
        } else {
            self.pull_requests_page = None;
        }
    }
}

fn access(result: Result<(Result<Value>, Result<Value>)>) -> Availability {
    let (status, user) = match result {
        Ok(result) => result,
        Err(error) => return Availability::Unavailable(format!("{error:#}")),
    };
    if let Ok(user) = &user
        && user["status"] == "success"
        && let Ok(account) = serde_json::from_value::<Account>(user["account"].clone())
    {
        return Availability::Ready(account);
    }
    if cfg!(target_os = "macos")
        && status
            .as_ref()
            .is_ok_and(|status| status["isInstalled"] == false)
    {
        return Availability::Missing;
    }
    let error = match user {
        Ok(user) => user["error"]
            .as_str()
            .unwrap_or("Install and sign in with GitHub CLI on Local, then check again.")
            .to_owned(),
        Err(error) => format!("{error:#}"),
    };
    Availability::Unavailable(error)
}

impl Route {
    pub fn pull_request_path(path: &str) -> Option<Self> {
        let path = path.split(['?', '#']).next()?;
        if path == "/pull-requests" || path == "/pull-requests/" {
            return Some(Self::PullRequests(None));
        }
        let mut parts = path.strip_prefix("/pull-requests/")?.split('/');
        let (hostname, owner, repository, number) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let identity = number
            .parse()
            .ok()
            .map(|number| Identity {
                hostname: hostname.to_lowercase(),
                owner: owner.to_owned(),
                repository: repository.to_owned(),
                number,
            })
            .filter(Identity::valid);
        Some(Self::PullRequests(identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_request_identity_returns_to_the_inbox() {
        let route =
            Route::pull_request_path("/pull-requests/github.com/i098/codex-gpui/12?view=all");
        assert!(matches!(
            route,
            Some(Route::PullRequests(Some(Identity { number: 12, .. })))
        ));
        for path in ["/settings", "/pull-requests/github.com/i098/repo/1/extra"] {
            assert_eq!(Route::pull_request_path(path), None, "{path}");
        }
        for path in [
            "/pull-requests/github.com/i098/repo/0",
            "/pull-requests/github.com/i098/repo/not-a-number",
            "/pull-requests/github.com/../repo/2",
        ] {
            assert_eq!(
                Route::pull_request_path(path),
                Some(Route::PullRequests(None)),
                "{path}"
            );
        }
    }
}

fn repository_error(login: &str, error: String) -> String {
    let Some((_, rest)) = error.split_once("Could not resolve to a Repository with the name '")
    else {
        return error;
    };
    let Some((repository, _)) = rest.split_once('\'') else {
        return error;
    };
    let mut parts = repository.split('/');
    if parts.next().is_some_and(screen::model::valid_component)
        && parts.next().is_some_and(screen::model::valid_component)
        && parts.next().is_none()
    {
        format!(
            "GitHub account {login} can’t access {repository}. Switch accounts or request access, then try again."
        )
    } else {
        error
    }
}
