use crate::{Root, Route, host};
use anyhow::{Context as _, Result};
use bridge::Bridge;
use gpui::{
    AppContext as _, Context, Div, Entity, Focusable as _, InteractiveElement as _, Window,
};
use serde_json::{Value, json};
use ui::screens::pull_requests::{
    self as screen, Availability, Event, Page,
    model::{Account, Identity, SearchPage},
};
use ui::toast::Toast;

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
        let inline_inset = self.shell().read(cx).main_content_inline_inset();
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
        page.update(cx, |page, _| {
            page.link_threads = self
                .threads
                .iter()
                .map(|thread| (thread.id.clone(), crate::thread_title(thread).to_string()))
                .collect();
        });
        let changed = page.update(cx, |page, cx| {
            let changed = page.select(selected, cx);
            page.mount_pane_width(window.viewport_size(), inline_inset);
            changed
        });
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
            Event::Chat { title, prompt } => {
                self.open_pull_request_chat(page, title.clone(), prompt.clone(), window, cx)
            }
            Event::Code => self.load_pull_request_code(page, cx),
            Event::Links => self.load_pull_request_links(page.clone(), cx),
            Event::Link { thread, remove } => {
                let Some(identity) = page.read(cx).selected.clone() else {
                    return;
                };
                let bridge = self.bridge.clone();
                let thread = thread.clone();
                let remove = *remove;
                page.update(cx, |page, cx| {
                    page.link_generation += 1;
                    page.linked_threads = None;
                    cx.notify();
                });
                let page = page.clone();
                cx.spawn(async move |this, cx| {
                    let result = link_pull_request(&bridge, &thread, &identity, remove).await;
                    this.update(cx, |this, cx| {
                        let toast = match result {
                            Ok(()) => ui::toast::Toast::info(if remove {
                                "Chat unlinked"
                            } else {
                                "Chat linked"
                            }),
                            Err(error) => ui::toast::Toast::danger(error.to_string()),
                        };
                        this.toast(toast, cx);
                        this.load_pull_request_links(page, cx);
                    })
                    .ok();
                })
                .detach();
            }
            Event::Media { generation, url } => {
                self.load_pull_request_media(page, *generation, url.clone(), cx)
            }
        }
    }

    fn load_pull_request_links(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let Some(identity) = page.read(cx).selected.clone() else {
            return;
        };
        let bridge = self.bridge.clone();
        let generation = page.update(cx, |page, cx| {
            page.link_generation += 1;
            page.linked_threads = None;
            cx.notify();
            page.link_generation
        });
        cx.spawn(async move |_, cx| {
            let result = async {
                let key = pull_request_key(&identity)?;
                crate::settings_git::attachment_owners(&bridge, "pull_request", &key).await
            }
            .await
            .map_err(|error| error.to_string());
            page.update(cx, |page, cx| {
                if page.selected.as_ref() == Some(&identity) && page.link_generation == generation {
                    page.linked_threads = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
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

    /// The detail's `Chat`: opens the chat linked to the pull request, or starts a projectless
    /// chat titled after it with no first turn, links it, opens it and prefills `prompt`, as the
    /// original's `pullRequestDetail.threadAction`.
    fn open_pull_request_chat(
        &mut self,
        page: Entity<Page>,
        title: String,
        prompt: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = page.read(cx).chat.clone() {
            self.navigate(Route::Thread(id), window, cx);
            return;
        }
        let (Some(account), Some(identity)) = (
            page.read(cx).account().cloned(),
            page.read(cx).selected.clone(),
        ) else {
            return;
        };
        page.update(cx, |page, cx| {
            page.chat_pending = true;
            cx.notify();
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let selected = identity.clone();
        cx.spawn_in(window, async move |this, cx| {
            let started = async {
                let workspace = {
                    let host = host.clone();
                    let title = title.clone();
                    cx.background_executor()
                        .spawn(async move { host.prepare_projectless(&title) })
                        .await?
                };
                let mut thread = bridge
                    .request::<_, bridge::types::ThreadStartResponse>(
                        "thread/start",
                        json!({
                            "cwd": workspace.cwd,
                            "developerInstructions": workspace.instructions(),
                        }),
                    )
                    .await?
                    .thread;
                let id = thread.id.clone();
                bridge
                    .request::<_, Value>(
                        "thread/name/set",
                        json!({"threadId": id, "name": title}),
                    )
                    .await?;
                thread.name = Some(title);
                let host = host.clone();
                let thread_id = id.clone();
                cx.background_executor()
                    .spawn(async move {
                        host.record_projectless(&thread_id, &workspace)?;
                        host::json_body(&host.fetch(&host::Request::post(
                            "pull-request-chat-link",
                            Some(json!({"account": account, "pullRequest": identity, "threadId": thread_id})),
                        ))?)
                    })
                    .await?;
                Ok::<_, anyhow::Error>(thread)
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                let current = page.read(cx).selected.as_ref() == Some(&selected);
                page.update(cx, |page, cx| {
                    page.chat_pending = false;
                    cx.notify();
                });
                match started {
                    Ok(thread) => {
                        if current {
                            page.update(cx, |page, _| page.chat = Some(thread.id.clone()));
                        }
                        this.loaded.insert(thread.id.clone());
                        this.add_thread(&thread, cx);
                        this.show_thread(thread, window, cx);
                        let composer = this.composer(cx);
                        composer.update(cx, |composer, cx| composer.set_text(&prompt, cx));
                        let focus = composer.read(cx).focus_handle(cx);
                        focus.focus(window, cx);
                    }
                    Err(_) => {
                        this.toast(
                            Toast::danger("Could not start a Codex chat for this pull request"),
                            cx,
                        );
                    }
                }
            })
        })
        .detach();
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
            let status = cfg!(target_os = "macos").then(|| {
                service
                    .fetch(&host::Request::post(
                        "gh-cli-status",
                        Some(json!({"hostId":"local", "hostname":hostname})),
                    ))
                    .and_then(|response| host::json_body(&response))
            });
            let user = service
                .fetch(&host::Request::post(
                    "gh-current-user",
                    Some(json!({"hostId":"local", "hostname":hostname})),
                ))
                .and_then(|response| host::json_body(&response));
            (status, user)
        });
        cx.spawn(async move |this, cx| {
            let (status, user) = load.await;
            let availability = access(status, user);
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
            page.chat = None;
            cx.notify();
        });
        let generation = page.read(cx).detail_generation;
        let login = account.login.clone();
        let service = self.host.clone();
        let load = cx.background_spawn(async move {
            let params = json!({"account": account, "pullRequest": identity});
            // A missing or unreadable link only leaves `Chat` starting a new chat.
            let chat = service
                .fetch(&host::Request::post(
                    "pull-request-chat",
                    Some(params.clone()),
                ))
                .and_then(|response| host::json_body(&response))
                .ok()
                .and_then(|body| body["threadId"].as_str().map(str::to_owned));
            let detail = host::json_body(
                &service.fetch(&host::Request::post("gh-pr-detail", Some(params)))?,
            );
            detail.map(|detail| (detail, chat))
        });
        cx.spawn(async move |_, cx| {
            let result = load
                .await
                .map_err(|error| repository_error(&login, format!("{error:#}")));
            page.update(cx, |page, cx| {
                let result = result.map(|(detail, chat)| {
                    if page.detail_generation == generation {
                        page.chat = chat;
                    }
                    detail
                });
                page.apply_detail(generation, result, cx)
            });
        })
        .detach();
    }

    fn load_pull_request_code(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.read(cx).detail_generation;
        let (Some(account), Some(identity)) = (
            page.read(cx).account().cloned(),
            page.read(cx).selected.clone(),
        ) else {
            page.update(cx, |page, cx| {
                page.apply_code(generation, Err("GitHub access is unavailable".into()), cx)
            });
            return;
        };
        let service = self.host.clone();
        let load = cx.background_spawn(async move {
            let response = host::json_body(&service.fetch(&host::Request::post(
                "gh-pr-patch",
                Some(json!({"account": account, "pullRequest": identity})),
            ))?)?;
            response["diff"]
                .as_str()
                .map(str::to_owned)
                .context("GitHub returned no patch")
        });
        cx.spawn(async move |_, cx| {
            let result = load.await.map_err(|error| format!("{error:#}"));
            page.update(cx, |page, cx| page.apply_code(generation, result, cx));
        })
        .detach();
    }

    fn load_pull_request_media(
        &self,
        page: Entity<Page>,
        generation: u64,
        url: String,
        cx: &mut Context<Self>,
    ) {
        if generation != page.read(cx).detail_generation {
            page.update(cx, |page, cx| {
                page.apply_media(generation, &url, Err("Preview expired".into()), cx)
            });
            return;
        }
        let (Some(account), Some(identity)) = (
            page.read(cx).account().cloned(),
            page.read(cx).selected.clone(),
        ) else {
            page.update(cx, |page, cx| {
                page.apply_media(
                    generation,
                    &url,
                    Err("GitHub account unavailable".into()),
                    cx,
                )
            });
            return;
        };
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

fn access(status: Option<Result<Value>>, user: Result<Value>) -> Availability {
    if let Ok(user) = &user
        && user["status"] == "success"
        && let Ok(account) = serde_json::from_value::<Account>(user["account"].clone())
    {
        return Availability::Ready(account);
    }
    if status.as_ref().is_some_and(|status| {
        status
            .as_ref()
            .is_ok_and(|value| value["isInstalled"] == false)
    }) {
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

pub(crate) async fn link_pull_request(
    bridge: &Bridge,
    thread: &str,
    identity: &Identity,
    remove: bool,
) -> Result<()> {
    let key = pull_request_key(identity)?;
    let mut params = json!({
        "threadId": thread, "attachmentType": "pull_request", "identityKey": key,
    });
    if !remove {
        params["payload"] = json!({"url": identity.url(), "root": null, "headBranch": null});
    }
    let _: Value = bridge
        .request(
            if remove {
                "thread/attachment/remove"
            } else {
                "thread/attachment/add"
            },
            params,
        )
        .await?;
    Ok(())
}

fn pull_request_key(identity: &Identity) -> Result<String> {
    anyhow::ensure!(identity.valid(), "Invalid pull request identity");
    Ok(serde_json::to_string(&(
        identity.hostname.to_lowercase(),
        identity.owner.to_lowercase(),
        identity.repository.to_lowercase(),
        identity.number,
    ))?)
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
