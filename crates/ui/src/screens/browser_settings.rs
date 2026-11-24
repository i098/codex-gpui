use super::*;
use crate::confirmation::{Confirmation, Outcome};
use crate::settings_kit::{Choice, Select, Selected};
use account_page::{Kind, action, checkbox, frame, paragraph};
use chrono::TimeZone as _;
use gpui::BorrowAppContext as _;
use std::collections::HashSet;

/// `browser_use.default_origin_policy.full_cdp_access`: `allow` or `deny`.
pub(super) const FULL_CDP: &str = "browser_use.default_origin_policy.full_cdp_access";

/// The original's `Allow`/`Block` choices for a default site permission.
const POLICY_OPTIONS: &[(&str, &str)] = &[("allow", "Allow"), ("deny", "Block")];

/// Host settings key for persistent browser-tool approvals.
pub const BROWSER_TOOL_GRANTS: &str = "in-app-browser-allowed-origins";

/// The user asked to revoke a browser-tool approval for this origin.
pub struct RevokeBrowserToolGrant(pub String);

impl EventEmitter<RevokeBrowserToolGrant> for SettingsPage {}

impl SettingsPage {
    pub fn refresh_browser_tool_grants(&mut self, grants: Value, cx: &mut Context<Self>) {
        self.values["host"][BROWSER_TOOL_GRANTS] = grants;
        cx.notify();
    }
}

/// One page the built-in browser committed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// Unix milliseconds.
    pub time: i64,
    /// The agent drove the browser there (the original's `visitSource` `agent`).
    #[serde(default)]
    pub agent: bool,
}

/// The built-in browser's visits, oldest first. The app records and persists them.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct BrowsingHistory(pub Vec<Visit>);

impl Global for BrowsingHistory {}

impl BrowsingHistory {
    // shortcut: one capped JSON list; move to SQLite if users need more history.
    pub const CAP: usize = 5000;

    /// Records a page the user opened, dropping the oldest visits past [`Self::CAP`].
    pub fn record(&mut self, url: &str, title: &str, time: i64) {
        self.0.push(Visit {
            url: url.to_owned(),
            title: title.to_owned(),
            time,
            agent: false,
        });
        let excess = self.0.len().saturating_sub(Self::CAP);
        self.0.drain(..excess);
    }

    /// A title that arrives after the navigation renames the newest visit of `url`.
    pub fn set_title(&mut self, url: &str, title: &str) {
        if let Some(visit) = self.0.last_mut().filter(|visit| visit.url == url) {
            visit.title = title.to_owned();
        }
    }
}

/// The history filter (`settings.browserUse.browsingHistory.source.*`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    All,
    Agent,
    Other,
}

impl Source {
    fn parse(value: &str) -> Self {
        match value {
            "agent" => Self::Agent,
            "other" => Self::Other,
            _ => Self::All,
        }
    }
}

/// The visits from `source` whose title or URL contains `query` (lowercase), newest first.
fn search<'a>(visits: &'a [Visit], query: &str, source: Source) -> Vec<&'a Visit> {
    visits
        .iter()
        .rev()
        .filter(|visit| match source {
            Source::All => true,
            Source::Agent => visit.agent,
            Source::Other => !visit.agent,
        })
        .filter(|visit| {
            visit.title.to_lowercase().contains(query) || visit.url.to_lowercase().contains(query)
        })
        .collect()
}

/// A visit's identity in the selection: its time and URL.
type Key = (i64, String);

fn key(visit: &Visit) -> Key {
    (visit.time, visit.url.clone())
}

/// The breadcrumb's `Browser`: back to the Browser page.
pub struct Back;

/// Settings ▸ Browser ▸ Browsing history: search, the All/Agent/Other filter, selection
/// removal and `Clear browsing data` over [`BrowsingHistory`].
pub struct HistoryPage {
    search: Entity<Composer>,
    source_menu: Entity<Select>,
    source: Source,
    selected: HashSet<Key>,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl EventEmitter<Back> for HistoryPage {}

impl HistoryPage {
    fn new(cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| Composer::settings_compact_field("Search browsing history", "", cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        let choices = [("all", "All"), ("agent", "Agent"), ("other", "Other")]
            .into_iter()
            .map(|(value, label)| Choice {
                value: value.into(),
                label: label.into(),
                checked: false,
                disabled: false,
            })
            .collect();
        let source_menu = cx.new(|cx| Select::new("all", choices, None, cx).with_width(160.));
        cx.subscribe(&source_menu, |this, _, selected: &Selected, cx| {
            this.source = Source::parse(&selected.0);
            this.selected.clear();
            cx.notify();
        })
        .detach();
        cx.observe_global::<BrowsingHistory>(|_, cx| cx.notify())
            .detach();
        Self {
            search,
            source_menu,
            source: Source::All,
            selected: HashSet::new(),
            confirmation: None,
            layer: Layer::default(),
        }
    }

    fn remove(&mut self, keys: HashSet<Key>, cx: &mut Context<Self>) {
        if cx.has_global::<BrowsingHistory>() {
            cx.update_global::<BrowsingHistory, _>(|history, _| {
                history.0.retain(|visit| !keys.contains(&key(visit)))
            });
        }
        self.selected.retain(|selected| !keys.contains(selected));
        cx.notify();
    }

    fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Clear browsing data",
                "Browsing history",
                "Delete browsing history",
                window,
                cx,
            )
            .destructive()
            .settings_style()
        });
        cx.subscribe(&dialog, |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm && cx.has_global::<BrowsingHistory>() {
                cx.update_global::<BrowsingHistory, _>(|history, _| history.0.clear());
                this.selected.clear();
            }
            this.confirmation = None;
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    fn row(&self, index: usize, visit: &Visit, t: &Theme, cx: &mut Context<Self>) -> Div {
        let key = key(visit);
        let selected = self.selected.contains(&key);
        let time = chrono::Local
            .timestamp_millis_opt(visit.time)
            .single()
            .map(|time| time.format("%b %-d, %Y, %H:%M").to_string())
            .unwrap_or_default();
        let detail = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .when(visit.agent, |d| {
                d.child(text("Agent", 13., 18., 430., t.text_secondary).flex_none())
            })
            .child(
                text(
                    format!("{time} · {}", visit.url),
                    13.,
                    18.,
                    430.,
                    t.text_secondary,
                )
                .min_w_0()
                .truncate(),
            );
        let select = checkbox("browsing-history-select", selected, "", t).on_click({
            let key = key.clone();
            cx.listener(move |this, _, _, cx| {
                if !this.selected.remove(&key) {
                    this.selected.insert(key.clone());
                }
                cx.notify();
            })
        });
        let remove = action(
            "browsing-history-remove",
            "Remove from history",
            Kind::Danger,
            false,
            t,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.remove(HashSet::from([key.clone()]), cx)));
        let url = visit.url.clone();
        let name = if visit.title.trim().is_empty() {
            &visit.url
        } else {
            &visit.title
        };
        // A long unbroken title or URL truncates instead of covering Remove.
        let title = div()
            .id("browsing-history-open")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(16.))
            .py(px(12.))
            .cursor_pointer()
            .child(text(name.clone(), 13., 18.571_428, 500., t.text).truncate())
            .child(detail)
            .on_click(move |_, _, cx| cx.open_url(&url));
        div().child(
            div()
                .id(("browsing-history-row", index))
                .flex()
                .items_center()
                .pl(px(16.))
                .pr(px(16.))
                .child(select)
                .child(title)
                .child(remove),
        )
    }
}

impl Render for HistoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let query = self.search.read(cx).field_text().trim().to_lowercase();
        let visits = cx
            .try_global::<BrowsingHistory>()
            .map(|history| search(&history.0, &query, self.source))
            .unwrap_or_default()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let rows: Vec<Div> = visits
            .iter()
            .enumerate()
            .map(|(index, visit)| self.row(index, visit, &t, cx))
            .collect();
        let (empty_title, empty_detail) = match (query.is_empty(), self.source) {
            (false, _) => (
                "No matching pages",
                "Try searching for a different page or address",
            ),
            (true, Source::All) => (
                "No browsing history yet",
                "Pages visited in the built-in browser will appear here",
            ),
            (true, Source::Agent) => (
                "No agent visits",
                "Choose a different visit source to see more history",
            ),
            (true, Source::Other) => (
                "No other visits",
                "Choose a different visit source to see more history",
            ),
        };
        let back = div()
            .id("browsing-history-back")
            .debug_selector(|| "browsing-history-back".into())
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .child(icon("arrow-left", 16., t.text_tertiary).runtime())
            .child(text("Browser", 13., 18., 430., t.text_tertiary))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(Back)));
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(text("All-time history", 16., 22., 500., t.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(!self.selected.is_empty(), |d| {
                        d.child(
                            action(
                                "browsing-history-remove-selected",
                                "Remove selected",
                                Kind::Danger,
                                false,
                                &t,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let keys = std::mem::take(&mut this.selected);
                                this.remove(keys, cx)
                            })),
                        )
                    })
                    .child(
                        action(
                            "browsing-history-clear",
                            "Clear browsing data",
                            Kind::Secondary,
                            false,
                            &t,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.confirm_clear(window, cx)),
                        ),
                    ),
            );
        let content = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(header)
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .debug_selector(|| "browsing-history-search".into())
                            .child(self.search.clone()),
                    )
                    .child(self.source_menu.clone()),
            )
            .when(rows.is_empty(), |d| {
                d.child(text(empty_title, 14., 21., 500., t.text))
                    .child(paragraph(empty_detail, &t))
            })
            .when(!rows.is_empty(), |d| {
                d.child(super::worktrees::card(rows, Some(&self.layer), &t))
            });
        frame(
            "browsing-history-page",
            "Browsing history",
            vec![back.into_any_element()],
            content,
            self.confirmation.as_ref(),
            &self.layer,
            window,
            &t,
        )
    }
}

impl SettingsPane {
    /// The in-app browser's `browser_use` permissions in `config.toml`, as the original's
    /// Browser settings write them (`config/batchWrite` on `browser_use.*`).
    pub(super) fn browser(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let row = |title, description, control| settings_row(title, description, control, t);
        let permissions = vec![
            row(
                "Browse",
                Some("Default for websites without custom permissions"),
                self.select_menu(
                    "browser_use.default_origin_policy.access",
                    POLICY_OPTIONS,
                    None,
                    |select| select,
                    cx,
                ),
            ),
            row(
                "Download",
                None,
                self.select_menu(
                    "browser_use.default_origin_policy.downloads",
                    POLICY_OPTIONS,
                    None,
                    |select| select,
                    cx,
                ),
            ),
            row(
                "Upload",
                None,
                self.select_menu(
                    "browser_use.default_origin_policy.uploads",
                    POLICY_OPTIONS,
                    None,
                    |select| select,
                    cx,
                ),
            ),
            row(
                "History",
                Some("Access history without asking"),
                self.toggle("browser_use.allow_history_access", false, t, window, cx),
            ),
            row(
                "Enable full CDP access",
                Some(
                    "Allow ChatGPT to use full Chrome DevTools Protocol (CDP) access in connected Browser Use sessions. Full CDP access lets ChatGPT inspect and control sensitive browser internals that may put your data at risk.",
                ),
                self.toggle(FULL_CDP, false, t, window, cx),
            ),
        ];
        let page = self.page.read(cx);
        let mut sites: std::collections::BTreeMap<_, _> =
            site_rows(&page.values["browser_use"]["origins"])
                .into_iter()
                .collect();
        let grants = page.values["host"][BROWSER_TOOL_GRANTS].as_object();
        for (origin, _) in grants
            .into_iter()
            .flatten()
            .filter(|(_, value)| matches!(value, Value::Bool(true)))
        {
            sites.entry(origin.clone()).or_default();
        }
        let sites = if sites.is_empty() {
            vec![row(
                "No custom site permissions",
                Some("Only sites with custom permissions appear here"),
                div().into_any_element(),
            )]
        } else {
            sites
                .into_iter()
                .map(|(origin, mut summary)| {
                    let revoke = grants
                        .is_some_and(|grants| grants.get(&origin) == Some(&Value::Bool(true)));
                    let control = if revoke {
                        if !summary.is_empty() {
                            summary.push_str(" · ");
                        }
                        summary.push_str("Codex access: Always allow");
                        let site = origin.clone();
                        action(
                            format!("revoke-browser-grant-{origin}"),
                            "Revoke",
                            Kind::Danger,
                            false,
                            t,
                        )
                        .tab_index(0)
                        .focus_visible(crate::focus::trigger_focus)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page.update(cx, |_, cx| {
                                cx.emit(RevokeBrowserToolGrant(site.clone()));
                            });
                        }))
                        .into_any_element()
                    } else {
                        div().into_any_element()
                    };
                    settings_row_content(
                        origin,
                        Some(text(summary, 12., 16., 430., t.text_secondary).into_any_element()),
                        control,
                        t,
                    )
                })
                .collect()
        };
        let manage = action(
            "browsing-history-manage",
            "Manage",
            Kind::Secondary,
            false,
            t,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            let page = cx.new(HistoryPage::new);
            cx.subscribe(&page, |this, _, _: &Back, cx| {
                this.browsing_history = None;
                cx.notify();
            })
            .detach();
            this.browsing_history = Some(page);
            cx.notify();
        }));
        let general = vec![row(
            "Browsing history",
            Some("View and manage pages visited in the built-in browser"),
            manage.into_any_element(),
        )];
        vec![
            ("General", general),
            ("Permissions", permissions),
            ("Site permissions", sites),
        ]
    }
}

/// `browser_use.origins` as `(origin, "Browse: Allow · Download: Block")` rows, sorted.
fn site_rows(origins: &Value) -> Vec<(String, String)> {
    let mut rows: Vec<_> = origins
        .as_object()
        .into_iter()
        .flatten()
        .map(|(origin, policy)| {
            let summary = [
                ("access", "Browse"),
                ("downloads", "Download"),
                ("uploads", "Upload"),
                ("full_cdp_access", "Debug / CDP"),
            ]
            .iter()
            .filter_map(|&(key, label)| {
                let value = match policy[key].as_str()? {
                    "allow" => "Allow",
                    "deny" => "Block",
                    _ => return None,
                };
                Some(format!("{label}: {value}"))
            })
            .collect::<Vec<_>>()
            .join(" · ");
            let summary = if summary.is_empty() {
                "Uses the default permission".to_owned()
            } else {
                summary
            };
            (origin.clone(), summary)
        })
        .collect();
    rows.sort();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_search_filters_newest_first_and_caps() {
        let mut history = BrowsingHistory::default();
        history.record("https://a.example/", "Alpha", 1);
        history.record("https://b.example/docs", "Beta", 2);
        history.0[0].agent = true;
        let urls = |query, source| {
            search(&history.0, query, source)
                .into_iter()
                .map(|visit| visit.url.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            urls("", Source::All),
            ["https://b.example/docs", "https://a.example/"]
        );
        assert_eq!(urls("alpha", Source::All), ["https://a.example/"]);
        assert_eq!(urls("docs", Source::All), ["https://b.example/docs"]);
        assert_eq!(urls("", Source::Agent), ["https://a.example/"]);
        assert_eq!(urls("", Source::Other), ["https://b.example/docs"]);
        for time in 0..BrowsingHistory::CAP as i64 {
            history.record("https://c.example/", "", time);
        }
        assert_eq!(history.0.len(), BrowsingHistory::CAP);
        assert!(
            history
                .0
                .iter()
                .all(|visit| visit.url == "https://c.example/")
        );
    }

    #[test]
    fn site_rows_summarize_custom_permissions() {
        let rows = site_rows(&json!({
            "https://b.example": {},
            "https://a.example": {"access": "allow", "downloads": "deny", "webmcp": "x"},
        }));
        assert_eq!(
            rows,
            [
                (
                    "https://a.example".to_owned(),
                    "Browse: Allow · Download: Block".to_owned()
                ),
                (
                    "https://b.example".to_owned(),
                    "Uses the default permission".to_owned()
                ),
            ]
        );
    }
}
