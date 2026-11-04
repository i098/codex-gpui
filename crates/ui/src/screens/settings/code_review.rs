//! Settings ▸ Code Review, the `Review agent` (cloud) surface of the original `code-review` page
//! (`settings-*.js`): personal review preferences, the repository list with search and paging,
//! and one repository's review settings, over the ChatGPT backend's `/wham/settings/user` and
//! `/wham/settings/code_review*`.

use super::{
    Layer, SettingsPage,
    account_page::{Kind, action, frame, paragraph, status},
    backend::{self, Call, Response},
    settings_row_content, switch,
    worktrees::card,
};
use crate::{
    composer::Composer,
    icon::icon,
    settings_kit::{Choice, Select, Selected},
    text,
    toast::Toast,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, time::Duration};
use theme::{ActiveTheme as _, Theme};

/// `Lt`: rows per page.
const PAGE_SIZE: usize = 8;
const SAVE_ERROR: &str = "Unable to save code review settings";
const PROVIDERS: &[(&str, &str)] = &[("github", "GitHub"), ("gitlab", "GitLab")];
const TRIGGERS: &[(&str, &str)] = &[("pr_open", "On PR open"), ("every_push", "On every push")];
const SMART: (&str, &str) = ("smart_detect", "Smart detect (Experimental)");
const WITH_CODE_REVIEW: (&str, &str) = ("code_review", "Whenever code review runs");
const PERSONAL: (&str, &str) = ("personal", "Follow personal preferences");
const SCOPES: &[(&str, &str)] = &[("everyone", "All PRs"), ("team", "Team PRs"), PERSONAL];
const SEVERITIES: &[(&str, &str)] = &[
    ("critical", "Critical"),
    ("high", "High"),
    ("medium", "Medium"),
    ("low", "Low"),
];

pub fn render(page: Entity<SettingsPage>, window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("code-review-settings", cx, |window, cx| {
            CodeReview::new(page, window, cx)
        })
        .into_any_element()
}

enum Load<T> {
    Loading,
    Failed,
    Ready(T),
}

struct Repos {
    rows: Vec<Value>,
    next: Option<String>,
}

struct Detail {
    id: String,
    load: Load<Value>,
    /// The failed save's HTTP status, or `save` for any other failure (`h`).
    error: Option<String>,
    saving: bool,
    threat_path: Entity<Composer>,
    rule: Option<Entity<Composer>>,
}

pub struct CodeReview {
    page: Entity<SettingsPage>,
    prefs: Load<Value>,
    saving: bool,
    provider: &'static str,
    search: Entity<Composer>,
    query: String,
    generation: u64,
    /// The cursor of each page reached so far; `tokens[0]` is the first page.
    tokens: Vec<Option<String>>,
    repos: Load<Repos>,
    detail: Option<Detail>,
    menus: HashMap<String, Entity<Select>>,
    layer: Layer,
}

impl CodeReview {
    fn new(page: Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| Composer::settings_search("Search repositories", cx));
        cx.observe(&search, |this, search, cx| {
            let query = search
                .read(cx)
                .field_text()
                .chars()
                .take(256)
                .collect::<String>();
            if query != this.query {
                this.query = query;
                this.debounce(cx);
            }
        })
        .detach();
        let entity = cx.entity();
        window.defer(cx, move |_, cx| {
            entity.update(cx, |this, cx| {
                this.load_prefs(cx);
                this.load_repos(cx);
            })
        });
        Self {
            page,
            prefs: Load::Loading,
            saving: false,
            provider: "github",
            search,
            query: String::new(),
            generation: 0,
            tokens: vec![None],
            repos: Load::Loading,
            detail: None,
            menus: HashMap::new(),
            layer: Layer::default(),
        }
    }

    fn toast(&self, title: &'static str, cx: &mut App) {
        if let Some(toaster) = self.page.read(cx).toaster.clone() {
            toaster.update(cx, |toaster, cx| toaster.push(Toast::danger(title), cx));
        }
    }

    fn load_prefs(&mut self, cx: &mut Context<Self>) {
        self.prefs = Load::Loading;
        let call = Call::new("GET", "/wham/settings/user", None);
        backend::request(call, cx, |this, result, _| {
            this.prefs = match result {
                Ok(response) if response.ok() && response.body.is_object() => {
                    Load::Ready(response.body)
                }
                _ => Load::Failed,
            };
        });
    }

    /// `PATCH /wham/settings/user` with the changed keys; the reply is the saved settings.
    fn save_prefs(&mut self, delta: Value, cx: &mut Context<Self>) {
        let Load::Ready(prefs) = &mut self.prefs else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        merge(prefs, &delta);
        let call = Call::new("PATCH", "/wham/settings/user", Some(delta));
        backend::request(call, cx, |this, result, cx| {
            this.saving = false;
            match result {
                Ok(response) if response.ok() && response.body.is_object() => {
                    this.prefs = Load::Ready(response.body);
                }
                _ => {
                    this.toast(SAVE_ERROR, cx);
                    this.load_prefs(cx);
                }
            }
        });
    }

    fn debounce(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.tokens = vec![None];
                    this.load_repos(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn load_repos(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.repos = Load::Loading;
        let query = self.query.trim().to_owned();
        let page = self.tokens.len();
        let cursor = self.tokens.last().cloned().flatten();
        let provider = self.provider;
        if page == 1
            && let Some(repo) = repo_from_query(&query, provider)
        {
            let call = Call::new(
                "GET",
                format!(
                    "/wham/settings/code_review/{}",
                    encode(&format!("{provider}-{repo}"))
                ),
                None,
            );
            backend::request(call, cx, move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(response) if response.ok() => {
                        let archived = response.body["repository"]["archived"] == true;
                        let rows = if archived {
                            vec![]
                        } else {
                            vec![response.body]
                        };
                        this.repos = Load::Ready(Repos { rows, next: None });
                    }
                    Ok(response) if matches!(response.status, 400 | 403 | 404) => {
                        this.search_repos(generation, &query, None, cx)
                    }
                    _ => this.repos = Load::Failed,
                }
            });
            return;
        }
        self.search_repos(generation, &query, cursor, cx);
    }

    /// GitLab without a query lists repositories by `next_token`; otherwise
    /// `code_review_search` pages by number, up to page 125 on GitHub.
    fn search_repos(
        &mut self,
        generation: u64,
        query: &str,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let provider = self.provider;
        let listing = provider == "gitlab" && query.is_empty();
        let page: usize = cursor.as_deref().and_then(|c| c.parse().ok()).unwrap_or(1);
        let path = if listing {
            let mut path =
                format!("/wham/settings/code_review?provider=gitlab&per_page={PAGE_SIZE}");
            if let Some(cursor) = &cursor {
                path.push_str(&format!("&next_token={}", encode(cursor)));
            }
            path
        } else {
            let query = if provider == "github" {
                format!("{query} archived:false").trim().to_owned()
            } else {
                query.to_owned()
            };
            format!(
                "/wham/settings/code_review_search?query={}&limit={PAGE_SIZE}&page={page}&provider={provider}",
                encode(&query)
            )
        };
        backend::request(Call::new("GET", path, None), cx, move |this, result, _| {
            if this.generation != generation {
                return;
            }
            this.repos = match result {
                Ok(response) if response.ok() => {
                    let rows = response.body["repo_review_settings"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let next = if listing {
                        response.body["next_token"].as_str().map(str::to_owned)
                    } else {
                        search_next(rows.len(), page, provider)
                    };
                    Load::Ready(Repos { rows, next })
                }
                _ => Load::Failed,
            };
        });
    }

    fn set_provider(&mut self, provider: &str, cx: &mut Context<Self>) {
        let Some(&(provider, _)) = PROVIDERS.iter().find(|(id, _)| *id == provider) else {
            return;
        };
        if provider != self.provider {
            self.provider = provider;
            self.tokens = vec![None];
            self.load_repos(cx);
        }
    }

    fn open_repo(&mut self, id: String, cx: &mut Context<Self>) {
        let threat_path =
            cx.new(|cx| Composer::settings_compact_field("docs/threat-model.md", "", cx));
        self.detail = Some(Detail {
            id,
            load: Load::Loading,
            error: None,
            saving: false,
            threat_path,
            rule: None,
        });
        self.load_detail(cx);
    }

    fn load_detail(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        // A refetch keeps the shown settings and only disables the controls, as the original does.
        if matches!(detail.load, Load::Ready(_)) {
            detail.saving = true;
        } else {
            detail.load = Load::Loading;
        }
        let id = detail.id.clone();
        let path = format!("/wham/settings/code_review/{}", encode(&id));
        backend::request(Call::new("GET", path, None), cx, move |this, result, cx| {
            let Some(detail) = this.detail.as_mut().filter(|d| d.id == id) else {
                return;
            };
            detail.saving = false;
            let path = |repo: &Value| {
                repo["security_review"]["threat_model_file_path"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            };
            let shown = match &detail.load {
                Load::Ready(repo) => Some(path(repo)),
                _ => None,
            };
            detail.load = match result {
                Ok(response) if response.ok() && response.body.is_object() => {
                    let saved = path(&response.body);
                    // A draft survives other saves: the field resets only when the saved path changes.
                    if shown.as_ref() != Some(&saved) {
                        let field = detail.threat_path.clone();
                        field.update(cx, |field, cx| field.set_text(&saved, cx));
                    }
                    Load::Ready(response.body)
                }
                _ => Load::Failed,
            };
        });
    }

    /// `PATCH /wham/settings/code_review` with the repository, its scope and funding generation,
    /// and the changed keys. A 409 reloads the repository.
    fn save_repo(&mut self, delta: Value, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        let Load::Ready(repo) = &detail.load else {
            return;
        };
        if detail.saving {
            return;
        }
        let mut item = json!({
            "repository": detail.id,
            "review_preference": repo["review_preference"],
            "funding_generation": repo["funding_generation"],
        });
        merge(&mut item, &delta);
        detail.saving = true;
        detail.error = None;
        let id = detail.id.clone();
        let body = json!({ "repo_review_settings": [item] });
        backend::request(
            Call::new("PATCH", "/wham/settings/code_review", Some(body)),
            cx,
            move |this, result, cx| {
                let Some(detail) = this.detail.as_mut().filter(|d| d.id == id) else {
                    return;
                };
                detail.saving = false;
                let status = match &result {
                    Ok(response) if response.ok() => None,
                    Ok(Response { status, .. }) => Some(*status),
                    Err(_) => Some(0),
                };
                if let Some(status) = status {
                    detail.error = Some(match status {
                        403 | 409 => status.to_string(),
                        _ => "save".into(),
                    });
                    if status != 409 {
                        return;
                    }
                }
                if status.is_none() {
                    // The original invalidates every `repository-review-settings` query.
                    this.load_repos(cx);
                }
                this.load_detail(cx);
            },
        );
    }

    /// The `{key}` menu: a `Select` kept across frames, its value refreshed each render.
    fn menu(
        &mut self,
        key: String,
        value: &str,
        options: &[(&str, &str)],
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choices = || -> Vec<Choice> {
            options
                .iter()
                .map(|(value, label)| Choice {
                    value: SharedString::new(*value),
                    label: SharedString::new(*label),
                    checked: false,
                    disabled: false,
                })
                .collect()
        };
        let select = self
            .menus
            .entry(key.clone())
            .or_insert_with(|| {
                let select = cx.new(|cx| Select::new(value.to_owned(), choices(), None, cx));
                cx.subscribe(&select, move |this, _, selected: &Selected, cx| {
                    this.choose(&key, selected.0.to_string(), cx);
                })
                .detach();
                select
            })
            .clone();
        select.update(cx, |select, cx| {
            let known = select.choices().iter().map(|c| c.value.as_ref());
            if !known.eq(options.iter().map(|(value, _)| *value)) {
                select.set_choices(choices(), cx);
            }
            select.set_value(value.to_owned(), cx);
            select.set_disabled(disabled, cx);
        });
        div().flex_none().child(select).into_any_element()
    }

    fn choose(&mut self, key: &str, value: String, cx: &mut Context<Self>) {
        let delta = match key {
            "provider" => return self.set_provider(&value, cx),
            "trigger" => {
                return self.save_prefs(json!({ "code_review_trigger_policy": value }), cx);
            }
            "security-trigger" | "security-severity" | "security-manual" => {
                let field = match key {
                    "security-trigger" => "trigger_policy",
                    "security-severity" => "severity_threshold",
                    _ => "manual_severity_threshold",
                };
                return self.save_prefs(json!({ "security_review": { field: value } }), cx);
            }
            "repo-scope" => scope_delta(&value),
            "repo-trigger" => {
                json!({ "code_review_trigger_policy": (value != "personal").then_some(&value) })
            }
            "repo-exhaustive" => {
                json!({ "exhaustive_code_review": (value != "personal").then_some(value == "on") })
            }
            "repo-security-scope" => json!({ "security_review": scope_delta(&value) }),
            "repo-security-trigger" => json!({ "security_review": { "trigger_policy": value } }),
            "repo-security-severity" => {
                json!({ "security_review": { "severity_threshold": value } })
            }
            "repo-security-manual" => {
                json!({ "security_review": { "manual_severity_threshold": value } })
            }
            _ => match key.strip_prefix("rule-") {
                Some(rest) => match self.rule_delta(rest, &value) {
                    Some(delta) => delta,
                    None => return,
                },
                None => return,
            },
        };
        self.save_repo(delta, cx);
    }

    /// `rule-{auto|manual}-{index}`: one path rule's severity; `inherit` clears it.
    fn rule_delta(&self, rest: &str, value: &str) -> Option<Value> {
        let (which, index) = rest.split_once('-')?;
        let index: usize = index.parse().ok()?;
        let Load::Ready(repo) = &self.detail.as_ref()?.load else {
            return None;
        };
        let mut rules = repo["security_review"]["path_rules"].as_array()?.clone();
        let field = match which {
            "auto" => "auto_severity_threshold",
            _ => "manual_severity_threshold",
        };
        rules.get_mut(index)?[field] = if value == "inherit" {
            Value::Null
        } else {
            value.into()
        };
        Some(json!({ "security_review": { "path_rules": rules } }))
    }

    fn rules_change(&mut self, rules: Vec<Value>, cx: &mut Context<Self>) {
        self.save_repo(json!({ "security_review": { "path_rules": rules } }), cx);
    }

    // ── Rendering ────────────────────────────────────────────────────────────────────────────

    fn overview(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let repositories = self.repositories(t, cx);
        let prefs = self.preferences(t, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(40.))
            .child(repositories)
            .child(prefs)
            .into_any_element()
    }

    fn repositories(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let provider = self.menu("provider".into(), self.provider, PROVIDERS, false, cx);
        let mut rows = vec![div().p(px(12.)).child(self.search.clone())];
        match &self.repos {
            Load::Loading => rows.push(message_row("Loading repositories…", t)),
            Load::Failed => rows.push(retry_row(
                "code-review-repos-retry",
                "Unable to load repositories",
                t,
                cx.listener(|this, _, _, cx| this.load_repos(cx)),
            )),
            Load::Ready(repos) if repos.rows.is_empty() => {
                rows.push(message_row("No repositories found", t))
            }
            Load::Ready(repos) => {
                for (index, repo) in repos.rows.iter().enumerate() {
                    rows.push(repo_row(index, repo, t, cx));
                }
            }
        }
        let page = self.tokens.len();
        let (has_next, fetching) = match &self.repos {
            Load::Ready(repos) => (repos.next.is_some(), false),
            Load::Loading => (false, true),
            Load::Failed => (false, false),
        };
        if page > 1 || has_next {
            let hint = (self.provider == "github" && page >= 125).then(|| {
                text(
                    "Search by repository name to narrow these results",
                    13.,
                    18.,
                    430.,
                    t.text_secondary,
                )
            });
            rows.push(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(div().children(hint))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                action(
                                    "code-review-previous",
                                    "Previous",
                                    Kind::Secondary,
                                    page == 1 || fetching,
                                    t,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.tokens.len() > 1 && !fetching {
                                            this.tokens.pop();
                                            this.load_repos(cx);
                                        }
                                    },
                                )),
                            )
                            .child(
                                action(
                                    "code-review-next",
                                    "Next",
                                    Kind::Secondary,
                                    !has_next || fetching,
                                    t,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Load::Ready(Repos {
                                            next: Some(next), ..
                                        }) = &this.repos
                                        {
                                            this.tokens.push(Some(next.clone()));
                                            this.load_repos(cx);
                                        }
                                    },
                                )),
                            ),
                    ),
            );
        }
        section(
            "Repositories",
            Some("Choose a repository to configure its automatic reviews"),
            Some(provider),
            card(rows, Some(&self.layer), t),
            t,
        )
    }

    fn preferences(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let prefs = match &self.prefs {
            Load::Loading => {
                return message_row("Loading code review settings…", t).into_any_element();
            }
            Load::Failed => {
                return retry_row(
                    "code-review-prefs-retry",
                    "Unable to load code review settings",
                    t,
                    cx.listener(|this, _, _, cx| this.load_prefs(cx)),
                )
                .into_any_element();
            }
            Load::Ready(prefs) => prefs.clone(),
        };
        let disabled = self.saving;
        let auto = prefs["code_review_preference"] == "always";
        let trigger = prefs["code_review_trigger_policy"]
            .as_str()
            .unwrap_or("pr_open");
        let mut triggers = TRIGGERS.to_vec();
        if trigger == SMART.0 {
            triggers.push(SMART);
        }
        let trigger_menu = self.menu("trigger".into(), trigger, &triggers, disabled, cx);
        let mut rows = vec![
            settings_row_content(
                "Automatic review",
                Some(description(PRIORITY, t)),
                self.pref_switch("code-review-auto", auto, disabled, t, cx, |on| {
                    json!({ "code_review_preference": if on { "always" } else { "repo_default" } })
                }),
                t,
            ),
            settings_row_content(
                "Review trigger",
                Some(description("Choose when ChatGPT should review your pull requests", t)),
                trigger_menu,
                t,
            ),
            settings_row_content(
                "Exhaustive code review",
                Some(description(
                    "Keep looking for findings until ChatGPT stops finding new issues",
                    t,
                )),
                self.pref_switch(
                    "code-review-exhaustive",
                    prefs["exhaustive_code_review"] == true,
                    disabled,
                    t,
                    cx,
                    |on| json!({ "exhaustive_code_review": on }),
                ),
                t,
            ),
        ];
        if prefs
            .get("allow_credits_for_code_reviews")
            .is_some_and(|v| !v.is_null())
        {
            rows.push(settings_row_content(
                "Use credits for reviews",
                Some(description(
                    "Allow credits to be consumed for reviews after rate limits",
                    t,
                )),
                self.pref_switch(
                    "code-review-credits",
                    prefs["allow_credits_for_code_reviews"] == true,
                    disabled,
                    t,
                    cx,
                    |on| json!({ "allow_credits_for_code_reviews": on }),
                ),
                t,
            ));
        }
        let mut sections = vec![section(
            "Personal preferences",
            None,
            None,
            card(rows, Some(&self.layer), t),
            t,
        )];
        let security = &prefs["security_review"];
        if !security.is_null() {
            let trigger = security["trigger_policy"]
                .as_str()
                .unwrap_or(WITH_CODE_REVIEW.0);
            let mut triggers = TRIGGERS.to_vec();
            if trigger == SMART.0 {
                triggers.push(SMART);
            }
            triggers.push(WITH_CODE_REVIEW);
            let trigger_menu =
                self.menu("security-trigger".into(), trigger, &triggers, disabled, cx);
            let severity = self.menu(
                "security-severity".into(),
                security["severity_threshold"].as_str().unwrap_or("high"),
                SEVERITIES,
                disabled,
                cx,
            );
            let manual = self.menu(
                "security-manual".into(),
                security["manual_severity_threshold"]
                    .as_str()
                    .unwrap_or("medium"),
                SEVERITIES,
                disabled,
                cx,
            );
            let mut rows = vec![
                settings_row_content(
                    "Automatic security review",
                    Some(description(PRIORITY, t)),
                    self.pref_switch(
                        "code-review-security-auto",
                        security["preference"] == "always",
                        disabled,
                        t,
                        cx,
                        |on| json!({ "security_review": { "preference": if on { "always" } else { "repo_default" } } }),
                    ),
                    t,
                ),
                settings_row_content(
                    "Security review trigger",
                    Some(description("Choose when Codex should review your pull requests for security", t)),
                    trigger_menu,
                    t,
                ),
                settings_row_content(
                    "Minimum automatic review reporting severity",
                    Some(description("Choose the lowest severity reported for automated reviews", t)),
                    severity,
                    t,
                ),
                settings_row_content(
                    "Minimum manual review reporting severity",
                    Some(description("Choose the lowest severity reported for reviews requested with @codex", t)),
                    manual,
                    t,
                ),
            ];
            if prefs["allow_credits_for_security_reviews"].is_boolean() {
                rows.push(settings_row_content(
                    "Use credits for security reviews",
                    Some(description(
                        "Security reviews use this account's shared Codex allowance. Turn this on to use credits after that allowance runs out. This also applies to 'Review Everyone' security reviews you've enabled.",
                        t,
                    )),
                    self.pref_switch(
                        "code-review-security-credits",
                        prefs["allow_credits_for_security_reviews"] == true,
                        disabled,
                        t,
                        cx,
                        |on| json!({ "allow_credits_for_security_reviews": on }),
                    ),
                    t,
                ));
            }
            sections.push(section(
                "Personal security review preferences",
                None,
                None,
                card(rows, Some(&self.layer), t),
                t,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(40.))
            .children(sections)
            .into_any_element()
    }

    fn pref_switch(
        &self,
        id: &'static str,
        on: bool,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
        delta: impl Fn(bool) -> Value + 'static,
    ) -> AnyElement {
        switch(id, on, disabled, t)
            .debug_selector(move || id.into())
            .on_click(cx.listener(move |this, _, _, cx| {
                if !disabled {
                    this.save_prefs(delta(!on), cx);
                }
            }))
            .into_any_element()
    }

    fn repo_switch(
        &self,
        id: &'static str,
        on: bool,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
        delta: impl Fn(bool) -> Value + 'static,
    ) -> AnyElement {
        switch(id, on, disabled, t)
            .debug_selector(move || id.into())
            .on_click(cx.listener(move |this, _, _, cx| {
                if !disabled {
                    this.save_repo(delta(!on), cx);
                }
            }))
            .into_any_element()
    }

    fn repository(&mut self, t: &Theme, cx: &mut Context<Self>) -> (SharedString, AnyElement) {
        let Some(detail) = &self.detail else {
            return (SharedString::default(), div().into_any_element());
        };
        let mut body = Vec::new();
        let repo = match &detail.load {
            Load::Ready(repo) => Some(repo.clone()),
            _ => None,
        };
        let funding = repo
            .as_ref()
            .is_some_and(|repo| repo["funding_authorization_required"] == true);
        let error = match (&detail.load, &detail.error) {
            (Load::Failed, _) => Some("load".to_owned()),
            (_, Some(code)) if code == "409" && funding => Some("funding".into()),
            (_, error) => error.clone(),
        };
        if let Some(kind) = error {
            let message = match kind.as_str() {
                "load" => "Unable to load repository settings",
                "funding" => {
                    "Workspace funding was removed. Reviews for all PRs require it to be reconnected."
                }
                "403" => "You do not have permission to change these repository settings",
                "409" => "Repository settings changed. Review the current settings and try again.",
                _ => "Unable to save repository settings. Try again.",
            };
            let mut notice = div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(status(message, true, t));
            if kind != "funding" {
                let label = if kind == "load" { "Retry" } else { "Refresh" };
                notice = notice.child(
                    action("code-review-repo-retry", label, Kind::Secondary, false, t).on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(detail) = &mut this.detail {
                                detail.error = None;
                            }
                            this.load_detail(cx);
                        }),
                    ),
                );
            }
            body.push(notice.into_any_element());
        }
        let Some(repo) = repo else {
            if matches!(detail.load, Load::Loading) {
                body.push(message_row("Loading repository settings…", t).into_any_element());
            }
            return (SharedString::default(), column(body));
        };
        let name: SharedString = repo["repository"]["repository_full_name"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
            .into();
        let permissions = &repo["repository"]["permissions"];
        let writable = permissions["admin"] == true || permissions["push"] == true;
        let disabled = detail.saving || !writable;
        if !writable {
            body.push(
                status(
                    "You need repository admin or push access to change these settings",
                    false,
                    t,
                )
                .into_any_element(),
            );
        }
        if funding {
            body.push(self.funding_notice(&repo, disabled, t, cx));
        }
        body.push(self.review_code(&repo, disabled, t, cx));
        if !repo["security_review"].is_null() {
            body.push(self.review_security(&repo, disabled, t, cx));
        }
        if let Some(instructions) = instructions(&repo, t) {
            body.push(instructions);
        }
        (name, column(body))
    }

    fn funding_notice(
        &self,
        repo: &Value,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let message = match repo["funding_account_name"].as_str() {
            Some(account) => format!(
                "Allow {account} to fund automatic reviews for this repository? Review preferences will stay the same."
            ),
            None => "Workspace funding was removed from this repository. Reconnect funding to run automatic reviews.".into(),
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(status(message, false, t))
            .child(
                action(
                    "code-review-allow-funding",
                    "Allow funding",
                    Kind::Primary,
                    disabled,
                    t,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !disabled {
                        this.save_repo(json!({ "authorize_funding": true }), cx);
                    }
                })),
            )
            .into_any_element()
    }

    /// `wn`: Automatic review, its scope, trigger and exhaustive review.
    fn review_code(
        &mut self,
        repo: &Value,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows = self.scope_rows(repo, false, disabled, t, cx);
        // The original shows the card's other rows only while automatic review is on.
        if repo["global_review_preference"] == "never_review" {
            return section(
                "Review code",
                None,
                None,
                card(rows, Some(&self.layer), t),
                t,
            );
        }
        let trigger = repo["code_review_trigger_policy"]
            .as_str()
            .unwrap_or("personal");
        let mut triggers = vec![PERSONAL];
        triggers.extend_from_slice(TRIGGERS);
        if trigger == SMART.0 {
            triggers.push(SMART);
        }
        let trigger = self.menu("repo-trigger".into(), trigger, &triggers, disabled, cx);
        rows.push(settings_row_content("Trigger", None, trigger, t));
        let exhaustive = match repo["exhaustive_code_review"].as_bool() {
            None => "personal",
            Some(true) => "on",
            Some(false) => "off",
        };
        let exhaustive = self.menu(
            "repo-exhaustive".into(),
            exhaustive,
            &[PERSONAL, ("on", "On"), ("off", "Off")],
            disabled,
            cx,
        );
        rows.push(settings_row_content(
            "Exhaustive review",
            None,
            exhaustive,
            t,
        ));
        section(
            "Review code",
            None,
            None,
            card(rows, Some(&self.layer), t),
            t,
        )
    }

    /// The toggle and `Review` scope shared by the code and security cards (`Fields`).
    fn scope_rows(
        &mut self,
        policy: &Value,
        security: bool,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Div> {
        let on = policy["global_review_preference"] != "never_review";
        let scope = if policy["global_review_preference"] == "review_everyone" {
            "everyone"
        } else if policy["review_preference"] == "team" {
            "team"
        } else {
            "personal"
        };
        let (id, label) = if security {
            ("code-review-repo-security-auto", "Auto security review")
        } else {
            ("code-review-repo-auto", "Automatic review")
        };
        let toggle = self.repo_switch(id, on, disabled, t, cx, move |on| {
            let delta = json!({ "global_review_preference": if on { "review_if_enabled" } else { "never_review" } });
            if security { json!({ "security_review": delta }) } else { delta }
        });
        let notice = (!on).then(|| description("You can still request a review with @codex", t));
        let mut rows = vec![settings_row_content(label, notice, toggle, t)];
        if on {
            let key = if security {
                "repo-security-scope"
            } else {
                "repo-scope"
            };
            let menu = self.menu(key.into(), scope, SCOPES, disabled, cx);
            rows.push(settings_row_content("Review", None, menu, t));
        }
        rows
    }

    /// `cn`: the security review card.
    fn review_security(
        &mut self,
        repo: &Value,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let security = repo["security_review"].clone();
        let mut rows = self.scope_rows(&security, true, disabled, t, cx);
        if security["global_review_preference"] == "never_review" {
            return section(
                "Review security vulnerabilities",
                None,
                None,
                card(rows, Some(&self.layer), t),
                t,
            );
        }
        let trigger = security["trigger_policy"]
            .as_str()
            .unwrap_or(WITH_CODE_REVIEW.0);
        let mut triggers = TRIGGERS.to_vec();
        if trigger == SMART.0 {
            triggers.push(SMART);
        }
        triggers.push(WITH_CODE_REVIEW);
        let trigger = self.menu(
            "repo-security-trigger".into(),
            trigger,
            &triggers,
            disabled,
            cx,
        );
        rows.push(settings_row_content("Trigger", None, trigger, t));
        // `Lt`: the path row shows only for the repository-file threat model source.
        if security["threat_model_source"]
            .as_str()
            .is_none_or(|source| source == "repository_file")
        {
            rows.push(self.threat_path_row(&security, disabled, t, cx));
        }
        let severity = self.menu(
            "repo-security-severity".into(),
            security["severity_threshold"].as_str().unwrap_or("high"),
            SEVERITIES,
            disabled,
            cx,
        );
        rows.push(settings_row_content(
            "Minimum auto review reporting severity",
            Some(description(
                "Choose the lowest severity Codex should report for automated reviews",
                t,
            )),
            severity,
            t,
        ));
        let manual = self.menu(
            "repo-security-manual".into(),
            security["manual_severity_threshold"]
                .as_str()
                .unwrap_or("medium"),
            SEVERITIES,
            disabled,
            cx,
        );
        rows.push(settings_row_content(
            "Minimum manual review reporting severity",
            Some(description(
                "Choose the lowest severity Codex should report for reviews requested with @codex",
                t,
            )),
            manual,
            t,
        ));
        rows.extend(self.path_rules(&security, disabled, t, cx));
        section(
            "Review security vulnerabilities",
            None,
            None,
            card(rows, Some(&self.layer), t),
            t,
        )
    }

    fn threat_path_row(
        &self,
        security: &Value,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let Some(detail) = &self.detail else {
            return div();
        };
        let saved = security["threat_model_file_path"]
            .as_str()
            .unwrap_or_default();
        let draft = detail.threat_path.read(cx).field_text().trim().to_owned();
        let dirty = draft != saved;
        div()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text("Path to threat model file in this repository", 14., 21., 430., t.text))
            .child(description(
                "File path is relative to the repository root. If not specified, Codex generates a threat model for every review.",
                t,
            ))
            .child(detail.threat_path.clone())
            .when(dirty, |d| {
                let saved = saved.to_owned();
                d.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            action("code-review-threat-save", "Save", Kind::Primary, disabled, t).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if disabled {
                                        return;
                                    }
                                    let Some(detail) = &this.detail else { return };
                                    let path = detail.threat_path.read(cx).field_text().trim().to_owned();
                                    let path = (!path.is_empty()).then_some(path);
                                    this.save_repo(json!({ "security_review": { "threat_model_source": "repository_file", "threat_model_file_path": path } }), cx);
                                }),
                            ),
                        )
                        .child(
                            action("code-review-threat-cancel", "Cancel", Kind::Secondary, disabled, t).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(detail) = &this.detail {
                                        let field = detail.threat_path.clone();
                                        field.update(cx, |field, cx| field.set_text(&saved, cx));
                                    }
                                }),
                            ),
                        ),
                )
            })
    }

    fn path_rules(
        &mut self,
        security: &Value,
        disabled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Div> {
        let rules = security["path_rules"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let adding = self.detail.as_ref().and_then(|d| d.rule.clone());
        let add = action(
            "code-review-add-rule",
            "Add path override",
            Kind::Secondary,
            disabled || adding.is_some(),
            t,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if disabled || this.detail.as_ref().is_some_and(|d| d.rule.is_some()) {
                return;
            }
            let field = cx.new(|cx| Composer::settings_compact_field("src/", "", cx));
            window.focus(&gpui::Focusable::focus_handle(field.read(cx), cx), cx);
            if let Some(detail) = &mut this.detail {
                detail.rule = Some(field);
            }
            cx.notify();
        }));
        let mut rows = vec![settings_row_content(
            "Path-based reporting severity override",
            Some(description(
                "Override reporting severity for specific repository paths",
                t,
            )),
            add.into_any_element(),
            t,
        )];
        if let Some(field) = adding {
            let all = rules.clone();
            rows.push(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text("Repository path prefix", 14., 21., 430., t.text))
                    .child(field.clone())
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                action("code-review-rule-add", "Add", Kind::Primary, disabled, t)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let prefix = field.read(cx).field_text().trim().to_owned();
                                        if disabled || prefix.is_empty() {
                                            return;
                                        }
                                        let mut rules = all.clone();
                                        rules.push(json!({
                                            "path_prefix": prefix,
                                            "auto_severity_threshold": null,
                                            "manual_severity_threshold": null,
                                        }));
                                        if let Some(detail) = &mut this.detail {
                                            detail.rule = None;
                                        }
                                        this.rules_change(rules, cx);
                                    })),
                            )
                            .child(
                                action(
                                    "code-review-rule-cancel",
                                    "Cancel",
                                    Kind::Secondary,
                                    false,
                                    t,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Some(detail) = &mut this.detail {
                                            detail.rule = None;
                                        }
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            );
        }
        let options: Vec<(&str, &str)> = std::iter::once(("inherit", "Use repository setting"))
            .chain(SEVERITIES.iter().copied())
            .collect();
        for (index, rule) in rules.iter().enumerate() {
            let auto = self.menu(
                format!("rule-auto-{index}"),
                rule["auto_severity_threshold"]
                    .as_str()
                    .unwrap_or("inherit"),
                &options,
                disabled,
                cx,
            );
            let manual = self.menu(
                format!("rule-manual-{index}"),
                rule["manual_severity_threshold"]
                    .as_str()
                    .unwrap_or("inherit"),
                &options,
                disabled,
                cx,
            );
            let remaining: Vec<Value> = rules
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, rule)| rule.clone())
                .collect();
            rows.push(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div().flex_1().min_w_0().child(
                            text(
                                rule["path_prefix"].as_str().unwrap_or_default().to_owned(),
                                13.,
                                18.,
                                430.,
                                t.text,
                            )
                            .font_family(crate::MONO),
                        ),
                    )
                    .child(text("Automatic", 13., 18., 430., t.text_secondary))
                    .child(auto)
                    .child(text("Manual", 13., 18., 430., t.text_secondary))
                    .child(manual)
                    .child(
                        div()
                            .id(("code-review-rule-remove", index))
                            .px(px(8.))
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .rounded(px(8.))
                            .hover(|d| d.bg(t.ghost_hover))
                            .child(text("Remove", 13., 18., 430., t.text))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !disabled {
                                    this.rules_change(remaining.clone(), cx);
                                }
                            })),
                    ),
            );
        }
        rows
    }
}

impl Render for CodeReview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let detail = self.detail.is_some();
        let (title, content) = if detail {
            self.repository(&t, cx)
        } else {
            ("Code Review".into(), self.overview(&t, cx))
        };
        let intro = detail
            .then(|| paragraph("Manage how Codex reviews this repository", &t).into_any_element());
        let page = frame(
            "code-review-page",
            title,
            intro.into_iter().collect(),
            content,
            None,
            &self.layer,
            window,
            &t,
        );
        // `backSlot`: a ghost toolbar button in the page's 46 px toolbar row.
        let back = detail.then(|| {
            div()
                .id("code-review-back")
                .debug_selector(|| "code-review-back".into())
                .role(gpui::Role::Button)
                .absolute()
                .top(px(9.))
                .left(px(12.))
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(8.))
                .hover(|d| d.bg(t.ghost_hover))
                .child(icon("chevron-left", 16., t.text_secondary))
                .child(text("Code review", 13., 18., 430., t.text))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.detail = None;
                    cx.notify();
                }))
        });
        div().size_full().relative().child(page).children(back)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────────────────────

const PRIORITY: &str = "If a repository’s review setting differs from your personal preference, the repository setting takes priority";

/// Overwrites `target`'s keys with `delta`'s, recursing into objects.
fn merge(target: &mut Value, delta: &Value) {
    match (target, delta) {
        (Value::Object(target), Value::Object(delta)) => {
            for (key, value) in delta {
                merge(
                    target
                        .entry(key.clone())
                        .or_insert(Value::Object(Map::new())),
                    value,
                );
            }
        }
        (target, delta) => *target = delta.clone(),
    }
}

/// The repository scope menu's keys (`everyone`, `team`, `personal`).
fn scope_delta(scope: &str) -> Value {
    json!({
        "global_review_preference": if scope == "everyone" { "review_everyone" } else { "review_if_enabled" },
        "review_preference": if scope == "personal" { "never" } else { "team" },
    })
}

/// `code_review_search` has no token: the next page exists while a page is full, up to page 125
/// on GitHub.
fn search_next(rows: usize, page: usize, provider: &str) -> Option<String> {
    (rows >= PAGE_SIZE && !(provider == "github" && page >= 125)).then(|| (page + 1).to_string())
}

/// `It`: `owner/name` (GitLab: the full group path) from a query that is a repository URL or
/// path; `None` for a plain search.
fn repo_from_query(query: &str, provider: &str) -> Option<String> {
    let host = if provider == "gitlab" {
        "gitlab.com/"
    } else {
        "github.com/"
    };
    let bare = query.strip_prefix("www.").unwrap_or(query);
    let scheme = query.starts_with("http://") || query.starts_with("https://");
    let mut path = query.to_owned();
    if scheme || bare.to_lowercase().starts_with(host) {
        let url = if scheme {
            query.to_owned()
        } else {
            format!("https://{bare}")
        };
        path = url::Url::parse(&url).ok()?.path().to_owned();
    }
    if provider == "github" {
        path = path.to_lowercase();
    }
    let path = path.strip_suffix(".git").unwrap_or(&path);
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if provider == "gitlab"
        && let Some(end) = parts
            .iter()
            .enumerate()
            .position(|(i, p)| i >= 2 && *p == "-")
    {
        parts.truncate(end);
    }
    if parts.len() < 2 {
        return None;
    }
    Some(if provider == "gitlab" {
        parts.join("/")
    } else {
        parts[..2].join("/")
    })
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// `Jt`: the repository's automatic review in the list.
fn policy_label(repo: &Value) -> &'static str {
    if repo["global_review_preference"] == "never_review" {
        "Off"
    } else if repo["global_review_preference"] == "review_everyone" {
        "All PRs"
    } else if repo["review_preference"] == "team" {
        "Team PRs"
    } else {
        "Follow personal preferences"
    }
}

fn repo_row(index: usize, repo: &Value, t: &Theme, cx: &mut Context<CodeReview>) -> gpui::Div {
    let id = repo["repository"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let name = repo["repository"]["repository_full_name"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let detail = description(format!("Automatic code review: {}", policy_label(repo)), t);
    div().child(
        div()
            .id(("code-review-repo", index))
            .debug_selector(move || format!("code-review-repo-{index}"))
            .role(gpui::Role::Button)
            .hover(|d| d.bg(t.ghost_hover))
            .child(settings_row_content(
                name,
                Some(detail),
                icon("chevron-right", 16., t.text_tertiary).into_any_element(),
                t,
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.open_repo(id.clone(), cx))),
    )
}

fn instructions(repo: &Value, t: &Theme) -> Option<AnyElement> {
    let url = agents_url(repo)?;
    let link = div()
        .id("code-review-open-agents")
        .debug_selector(|| "code-review-open-agents".into())
        .role(gpui::Role::Link)
        .cursor_pointer()
        .child(text("Open file", 13., 18., 430., t.text_link))
        .on_click(move |_, _, cx| cx.open_url(&url));
    let row = settings_row_content(
        "AGENTS.md",
        Some(description(
            "Add review guidelines to AGENTS.md in this repository",
            t,
        )),
        link.into_any_element(),
        t,
    );
    Some(section(
        "Review instructions",
        None,
        None,
        card(vec![row], None, t),
        t,
    ))
}

/// `{origin}{clone path without .git}/blob/{default_branch}/AGENTS.md` (`-/blob` on GitLab).
fn agents_url(repo: &Value) -> Option<String> {
    let repository = &repo["repository"];
    let clone = url::Url::parse(repository["clone_url"].as_str()?).ok()?;
    if !matches!(clone.scheme(), "http" | "https") {
        return None;
    }
    let branch = repository["default_branch"]
        .as_str()
        .filter(|b| !b.is_empty())?;
    let path = clone.path().strip_suffix(".git").unwrap_or(clone.path());
    let blob = if repository["id"].as_str()?.starts_with("gitlab-") {
        "-/blob"
    } else {
        "blob"
    };
    Some(format!(
        "{}{path}/{blob}/{}/AGENTS.md",
        clone.origin().ascii_serialization(),
        encode(branch)
    ))
}

fn description(s: impl Into<SharedString>, t: &Theme) -> AnyElement {
    text(s, 13., 18., 430., t.text_secondary)
        .whitespace_normal()
        .into_any_element()
}

fn message_row(message: &'static str, t: &Theme) -> gpui::Div {
    div()
        .px(px(16.))
        .py(px(12.))
        .child(text(message, 13., 18., 430., t.text_secondary))
}

fn retry_row(
    id: &'static str,
    message: &'static str,
    t: &Theme,
    on_retry: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .px(px(16.))
        .py(px(12.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(text(message, 13., 18., 430., t.text))
        .child(action(id, "Retry", Kind::Secondary, false, t).on_click(on_retry))
}

/// A titled group: title and subtitle on the left, an optional control on the right.
fn section(
    title: &'static str,
    subtitle: Option<&'static str>,
    control: Option<AnyElement>,
    body: impl IntoElement,
    t: &Theme,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(text(title, 14., 21., 500., t.text))
                        .children(subtitle.map(|s| text(s, 13., 18., 430., t.text_secondary))),
                )
                .children(control),
        )
        .child(body)
        .into_any_element()
}

fn column(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(24.))
        .children(children)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_queries_follow_the_original_parser() {
        let github = |q| repo_from_query(q, "github");
        assert_eq!(github("OpenAI/Codex"), Some("openai/codex".into()));
        assert_eq!(
            github("https://github.com/openai/codex/pull/1"),
            Some("openai/codex".into())
        );
        assert_eq!(
            github("www.github.com/openai/codex.git"),
            Some("openai/codex".into())
        );
        assert_eq!(github("codex"), None);
        assert_eq!(
            repo_from_query("gitlab.com/group/sub/proj/-/tree/main", "gitlab"),
            Some("group/sub/proj".into())
        );
    }

    #[test]
    fn search_pages_stop_when_short_or_at_github_limit() {
        assert_eq!(search_next(8, 1, "github"), Some("2".into()));
        assert_eq!(search_next(7, 1, "github"), None);
        assert_eq!(search_next(8, 125, "github"), None);
        assert_eq!(search_next(8, 125, "gitlab"), Some("126".into()));
    }

    #[test]
    fn scope_and_agents_link_match_the_original() {
        assert_eq!(
            scope_delta("personal"),
            json!({"global_review_preference": "review_if_enabled", "review_preference": "never"})
        );
        let repo = json!({"repository": {
            "id": "gitlab-g/p", "clone_url": "https://gitlab.com/g/p.git", "default_branch": "main"
        }});
        assert_eq!(
            agents_url(&repo).as_deref(),
            Some("https://gitlab.com/g/p/-/blob/main/AGENTS.md")
        );
        let mut prefs =
            json!({"security_review": {"preference": "always", "trigger_policy": "pr_open"}});
        merge(
            &mut prefs,
            &json!({"security_review": {"trigger_policy": "every_push"}}),
        );
        assert_eq!(prefs["security_review"]["preference"], "always");
        assert_eq!(prefs["security_review"]["trigger_policy"], "every_push");
    }
}
