use crate::host::{Host, Request};
use crate::*;
use base64::Engine as _;
use gpui::Div;
use ui::screens::home::{BROWSE_LIMIT, LibraryFile, WorkEvent};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
        .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
        .on_action(cx.listener(|this, a: &SelectProject, window, cx| {
            this.select_project(a.0.clone(), window, cx)
        }))
        .on_action(cx.listener(|this, _: &ClearProject, window, cx| this.clear_project(window, cx)))
        .on_action(
            cx.listener(|this, _: &ui::screens::home::ReloadAccount, window, cx| {
                this.read_account(Some(window), cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &ui::screens::home::SignOutAccount, window, cx| {
                let email = this.account_email.clone();
                let dialog = cx.new(|cx| Dialog::logout(email, cx));
                this.show_dialog(dialog, window, cx);
            }),
        )
        .on_action(
            cx.listener(|this, _: &ui::screens::home::SeeResets, window, cx| {
                if let Some(section) = settings::section("Usage & billing") {
                    this.open_settings_section(section, window, cx);
                }
            }),
        )
        .on_action(
            cx.listener(|this, _: &ui::screens::home::TryMultiAgent, _, cx| {
                this.try_multi_agent(cx)
            }),
        )
        .on_action(
            cx.listener(|this, a: &ui::screens::home::DismissBanner, _, cx| {
                this.dismiss_home_banner(a.0, cx)
            }),
        )
        .on_action(
            cx.listener(|this, a: &ui::screens::home::UseSuggestion, window, cx| {
                this.use_suggestion(&a.0, window, cx)
            }),
        )
        .on_action(
            cx.listener(|this, a: &ui::screens::home::DismissSuggestion, _, cx| {
                this.dismiss_suggestion(&a.0, cx)
            }),
        )
}

/// The Codex Home's `composer/items` query: three suggestions for the `work` entrypoint.
const SUGGESTIONS: &str = "/composer/items?entrypoint=work&limit=3";

/// The suggestion cards of a `composer/items` response.
/// shortcut: only `prompt` destinations without `auto_send_in_background`; conversation,
/// connector and background-start items need the thread-open and connector install flows.
fn suggestions(response: &Value) -> Vec<ui::screens::home::Suggestion> {
    response["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let destination = &item["destination_metadata"];
            if destination["type"] != "prompt" || destination["auto_send_in_background"] == true {
                return None;
            }
            Some(ui::screens::home::Suggestion {
                id: item["id"].as_str()?.to_owned(),
                title: item["title"].as_str()?.to_owned(),
                description: item["description"].as_str().map(str::to_owned),
                prompt: destination["prompt"].as_str().map(str::to_owned),
                auto_send: destination["auto_send"] == true,
                item: item.clone(),
            })
        })
        .take(3)
        .collect()
}

impl Root {
    /// `⌘N`, the `New chat` row and rail Home: Home with an empty, focused composer.
    pub(crate) fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_state.new_chat_mode = features::shortcuts::NewChatMode::Project;
        self.home.update(cx, |home, cx| {
            home.project = self.state.project_root.as_deref().map(project_name);
            cx.notify();
        });
        self.new_chat_section = None;
        if self.state.route == Route::Home {
            self.back
                .push((Route::Home, self.sidebar_automation.take()));
            self.forward.clear();
            self.navigation_epoch += 1;
            self.sync(window, cx);
        }
        self.navigate(Route::Home, window, cx);
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |c, cx| c.clear_session(cx));
        window.focus(&composer.focus_handle(cx), cx);
    }
}

impl Root {
    /// Reads Home's task suggestions; a failure leaves the list empty, as the original's
    /// `retry: false` query shows nothing.
    pub(crate) fn load_task_suggestions(&self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        self.home.update(cx, |home, cx| {
            home.suggestions_loading = true;
            cx.notify();
        });
        let read = cx.background_spawn(async move {
            let identity = host.identity().context("No ChatGPT account")?;
            crate::usage::http(&*host, &identity, "GET", SUGGESTIONS, None)
        });
        cx.spawn(async move |this, cx| {
            let items = read.await.map(|response| suggestions(&response));
            this.update(cx, |this, cx| {
                this.home.update(cx, |home, cx| {
                    home.suggestions_loading = false;
                    home.suggestions = items.unwrap_or_default();
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    /// `POST /composer/items/interactions`; a dismissal also posts its `dismissal_id`.
    fn suggestion_interaction(
        &self,
        item: Value,
        interaction: &'static str,
        cx: &App,
    ) -> Task<Result<()>> {
        let host = self.host.clone();
        cx.background_spawn(async move {
            let identity = host.identity().context("No ChatGPT account")?;
            let body = json!({
                "id": item["id"], "producer": item["producer"], "item_type": item["item_type"],
                "interaction_type": interaction, "entrypoint": "work",
            });
            let path = "/composer/items/interactions";
            crate::usage::http(&*host, &identity, "POST", path, Some(body))?;
            if interaction == "dismissed"
                && let Some(id) = item["action_metadata"]["dismissal_id"].as_str()
            {
                let body = json!({"task_suggestion_id": id, "action": "dismiss"});
                let path = "/task_suggestions/dismiss";
                crate::usage::http(&*host, &identity, "POST", path, Some(body))?;
            }
            Ok(())
        })
    }

    /// A card's click: its prompt goes in the composer, sent at once when `auto_send`.
    fn use_suggestion(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let home = self.home.read(cx);
        let Some(suggestion) = home.suggestions.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let composer = home.composer.clone();
        log_err(
            self.suggestion_interaction(suggestion.item, "clicked", cx),
            cx,
        );
        let Some(prompt) = suggestion.prompt else {
            return;
        };
        composer.update(cx, |composer, cx| composer.set_text(&prompt, cx));
        window.focus(&composer.focus_handle(cx), cx);
        if suggestion.auto_send {
            window.dispatch_action(Box::new(ui::composer::Submit), cx);
        }
    }

    /// `Dismiss suggestion`: the card leaves at once; a failed report says so in a toast.
    fn dismiss_suggestion(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut removed = None;
        self.home.update(cx, |home, cx| {
            if let Some(index) = home.suggestions.iter().position(|s| s.id == id) {
                removed = Some(home.suggestions.remove(index));
            }
            cx.notify();
        });
        let Some(suggestion) = removed else {
            return;
        };
        let report = self.suggestion_interaction(suggestion.item, "dismissed", cx);
        cx.spawn(async move |this, cx| {
            if report.await.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Couldn't dismiss suggestion"), cx)
                })
                .ok();
            }
        })
        .detach();
    }
}

/// `Files` menu reads: `POST /files/library` with the `ftn` body for native files, at most
/// `We` (10) of them; `q` narrows by name.
fn library_files(host: &dyn Host, query: &str, limit: usize) -> Result<Vec<LibraryFile>> {
    let identity = host.identity().context("No ChatGPT account")?;
    let q = (!query.is_empty()).then_some(query);
    let body = json!({"limit": limit, "cursor": null, "state": "ready", "q": q});
    let response = crate::usage::http(host, &identity, "POST", "/files/library", Some(body))?;
    Ok(response["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(LibraryFile {
                id: item["id"].as_str()?.to_owned(),
                file_id: item["file_id"].as_str()?.to_owned(),
                name: item["file_name"].as_str()?.to_owned(),
                size: item["file_size_bytes"].as_u64(),
            })
        })
        .collect())
}

/// A Library file's content (`Rxc` through `vtn`): its inline URL, then the bytes there.
fn library_file_bytes(host: &dyn Host, file: &LibraryFile) -> Result<Vec<u8>> {
    let identity = host.identity().context("No ChatGPT account")?;
    let http = |path: &str| crate::usage::http(host, &identity, "GET", path, None);
    // [INFERENCE] `vg` matches Library ids; other file ids download through `/files/download`.
    let url = if file.file_id.starts_with("libfile") {
        let path = format!("/files/library/files/{}/content_url", file.file_id);
        http(&path)?["content_url"].clone()
    } else {
        let path = format!(
            "/files/download/{}?inline=true",
            file.file_id.replace('#', "*")
        );
        let body = http(&path)?;
        // shortcut: no `retry` polling (`atn`); a file still processing fails the attach.
        ensure!(
            body["status"] == "success",
            "download status {}",
            body["status"]
        );
        body["download_url"].clone()
    };
    let url = url.as_str().context("no download URL")?;
    // ChatGPT's own paths take the account's credentials; other origins get none.
    let response = if url.starts_with('/') || url.starts_with("https://chatgpt.com/") {
        crate::usage::fetch(host, &identity, "GET", url, None)?
    } else {
        host.fetch(&Request::Http(
            json!({"url": url, "method": "GET", "headers": {}, "retry": "never"}),
        ))?
    };
    let wire = host::response_wire(&response);
    let status = wire["status"].as_u64().context("missing download status")?;
    ensure!(
        (200..300).contains(&status),
        "download HTTP status {status}"
    );
    match (wire["base64"].as_str(), wire["body"].as_str()) {
        (Some(data), _) => Ok(base64::engine::general_purpose::STANDARD.decode(data)?),
        (None, Some(body)) => Ok(body.as_bytes().to_vec()),
        (None, None) => anyhow::bail!("empty download"),
    }
}

impl Root {
    /// Answers the Work rail's requests.
    pub(crate) fn work_event(
        &mut self,
        event: &WorkEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkEvent::LibraryFiles(query) => {
                let host = self.host.clone();
                let query = query.clone();
                let read = cx.background_spawn({
                    let query = query.clone();
                    async move { library_files(&*host, &query, 10) }
                });
                let home = self.home.clone();
                cx.spawn(async move |_, cx| {
                    let files = read
                        .await
                        .inspect_err(|e| eprintln!("library files: {e:#}"));
                    home.update(cx, |home, cx| {
                        home.set_library_files(&query, files.ok(), cx)
                    })
                })
                .detach();
            }
            WorkEvent::LibraryBrowse(query) => {
                let host = self.host.clone();
                let query = query.clone();
                let read = cx.background_spawn({
                    let query = query.clone();
                    async move { library_files(&*host, &query, BROWSE_LIMIT) }
                });
                let home = self.home.clone();
                cx.spawn(async move |_, cx| {
                    let files = read
                        .await
                        .inspect_err(|e| eprintln!("library browse: {e:#}"));
                    home.update(cx, |home, cx| home.set_browse_files(&query, files.ok(), cx))
                })
                .detach();
            }
            WorkEvent::AttachLibraryFile(file) => {
                let host = self.host.clone();
                let read = cx.background_spawn({
                    let file = file.clone();
                    async move { library_file_bytes(&*host, &file) }
                });
                let file = file.clone();
                cx.spawn(async move |this, cx| {
                    let bytes = read.await.inspect_err(|e| eprintln!("library file: {e:#}"));
                    this.update(cx, |this, cx| {
                        if bytes.is_err() {
                            this.toast(Toast::danger("Couldn't attach this Library file"), cx);
                        }
                        this.home
                            .update(cx, |home, cx| home.library_file_done(&file, bytes.ok(), cx));
                    })
                })
                .detach();
            }
            WorkEvent::BrowsePlugins => {
                self.open_plugins("/plugins", window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_keep_the_first_three_prompt_items() {
        let prompt = |id: &str| {
            json!({
                "id": id, "title": format!("Title {id}"),
                "destination_metadata": {"type": "prompt", "prompt": "Fix it"},
            })
        };
        let response = json!({"items": [
            {"id": "c", "title": "Open",
             "destination_metadata": {"type": "conversation", "conversation_id": "x"}},
            {"id": "b", "title": "Bg", "destination_metadata":
                {"type": "prompt", "prompt": "p", "auto_send_in_background": true}},
            {"id": "untitled", "destination_metadata": {"type": "prompt", "prompt": "p"}},
            {"id": "s", "title": "Send", "description": "Now", "destination_metadata":
                {"type": "prompt", "prompt": "Go", "auto_send": true}},
            prompt("1"), prompt("2"), prompt("3"),
        ]});
        let cards = suggestions(&response);
        let ids: Vec<_> = cards.iter().map(|card| card.id.as_str()).collect();
        assert_eq!(ids, ["s", "1", "2"]);
        assert!(cards[0].auto_send);
        assert_eq!(cards[0].description.as_deref(), Some("Now"));
        assert_eq!(cards[1].prompt.as_deref(), Some("Fix it"));
        assert!(suggestions(&json!({})).is_empty());
    }
}
