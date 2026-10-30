use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
        .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
        // `Don't work in a project`: the next chat starts standalone.
        .on_action(
            cx.listener(|_, _: &ui::screens::home::ClearProject, window, cx| {
                window.dispatch_action(Box::new(features::shortcuts::NewStandaloneChat), cx)
            }),
        )
        .on_action(cx.listener(|this, a: &SelectProject, window, cx| {
            this.select_project(a.0.clone(), window, cx)
        }))
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
