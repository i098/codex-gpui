//! Rail `Sites` (`/sites`): the owned sites list the original's `AppgenPage` reads from
//! `GET /websites?limit=20` (`appgen-sites-query`), and a row's `/sites/:projectId` details
//! (`appgen-site-detail-layout`) from `GET /websites/{project_id}`.

use crate::screens::settings::button;
use crate::screens::thread::view::Clock;
use crate::{EmptySidebar, OpenSites, Shell, focus, text};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use serde::Deserialize;
use serde_json::Value;
use theme::ActiveTheme as _;

/// One `/websites` item; the original's zod schema `A` requires these fields.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Site {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub live_url: Option<String>,
    pub updated_at: String,
    /// `suspended` hides `Visit` and disables `Edit`.
    #[serde(default)]
    pub status: Option<String>,
}

impl Site {
    fn suspended(&self) -> bool {
        self.status.as_deref() == Some("suspended")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load<T = Vec<Site>> {
    #[default]
    Loading,
    Ready(T),
    Failed(String),
}

/// The `/websites` body (`{items, cursor}`); any malformed item fails the list, like `A.parse`.
pub fn sites(body: &Value) -> Result<Vec<Site>, String> {
    Vec::<Site>::deserialize(&body["items"]).map_err(|error| error.to_string())
}

/// The `/websites/{project_id}` body: one site.
pub fn site(body: &Value) -> Result<Site, String> {
    Site::deserialize(body).map_err(|error| error.to_string())
}

pub enum Event {
    /// `Retry` on a failed load.
    Refresh,
    /// A row opens `/sites/:projectId`.
    Open(String),
    /// The details page's `Edit`: continue the site in a new chat.
    Edit(Site),
}

/// `/sites/:projectId`: the site the page shows instead of the list.
pub struct Detail {
    pub id: String,
    pub load: Load<Site>,
}

pub struct Page {
    pub load: Load,
    pub detail: Option<Detail>,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    clock: Clock,
}

impl EventEmitter<Event> for Page {}

impl Page {
    pub fn new(clock: Clock) -> Self {
        Self {
            load: Load::Loading,
            detail: None,
            generation: 0,
            clock,
        }
    }

    pub fn detail(clock: Clock, id: String) -> Self {
        Self {
            detail: Some(Detail {
                id,
                load: Load::Loading,
            }),
            ..Self::new(clock)
        }
    }

    /// The path this page reads: the list, or the opened site.
    pub fn url(&self) -> String {
        match &self.detail {
            Some(detail) => format!("/websites/{}", detail.id),
            None => "/websites?limit=20".into(),
        }
    }

    /// Stores a response for [`Self::url`].
    pub fn set_body(&mut self, body: Result<Value, String>) {
        match &mut self.detail {
            Some(detail) => {
                detail.load = body
                    .and_then(|b| site(&b))
                    .map_or_else(Load::Failed, Load::Ready)
            }
            None => {
                self.load = body
                    .and_then(|b| sites(&b))
                    .map_or_else(Load::Failed, Load::Ready)
            }
        }
    }

    pub fn set_loading(&mut self) {
        match &mut self.detail {
            Some(detail) => detail.load = Load::Loading,
            None => self.load = Load::Loading,
        }
    }

    pub fn titles(&self) -> Vec<&str> {
        match &self.load {
            Load::Ready(sites) => sites.iter().map(|site| site.title.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    fn state(&self, title: &str, description: Option<&str>, cx: &App) -> gpui::Div {
        let t = cx.theme();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(7.))
            .pt(px(44.))
            .child(text(title.to_owned(), 16., 24., 500., t.text))
            .children(
                description.map(|value| text(value.to_owned(), 13., 18.57, 430., t.text_secondary)),
            )
    }

    fn retry(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        self.action("sites-retry", "Retry", 0, window, cx, |_, cx| {
            cx.emit(Event::Refresh)
        })
        .bg(t.text_alpha_5)
    }

    /// A toolbar button that runs `run` on click, `Enter` or `Space`.
    fn action(
        &self,
        id: &'static str,
        label: &'static str,
        index: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Window, &mut Context<Self>) + Clone + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme().clone();
        let handle = focus::handle(id, index, window, cx);
        let key = run.clone();
        focus::ring(
            button(label, t.surface, Some(t.border), &t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .cursor_pointer(),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |_, _, window, cx| run(window, cx)))
        .on_key_down(cx.listener(move |_, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                key(window, cx);
                cx.stop_propagation();
            }
        }))
    }

    fn row(
        &self,
        index: usize,
        site: &Site,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut meta = vec![updated(&site.updated_at, self.clock)];
        if site.live_url.is_none() {
            meta.push("Not published".into());
        }
        let id = SharedString::from(format!("site-{}", site.id));
        let handle = focus::handle(id.clone(), 1 + index as isize, window, cx);
        let (click, key) = (site.id.clone(), site.id.clone());
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Link)
                .aria_label(SharedString::from(site.title.clone()))
                .cursor_pointer()
                .flex()
                .flex_col()
                .gap(px(2.))
                .py(px(10.))
                .border_b_1()
                .border_color(t.border)
                .hover(|row| row.bg(t.text_alpha_5))
                .child(text(site.title.clone(), 14., 20., 500., t.text))
                .child(text(meta.join(" · "), 13., 18., 430., t.text_secondary)),
            &handle,
            px(6.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::Open(click.clone()))))
        .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "enter" {
                cx.emit(Event::Open(key.clone()));
                cx.stop_propagation();
            }
        }))
    }

    /// The details header: `Sites` back, then `Visit` (published, not suspended) and `Edit`.
    fn detail_header(
        &self,
        site: Option<&Site>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let back = self.action("site-back", "Sites", 0, window, cx, |window, cx| {
            window.dispatch_action(Box::new(OpenSites), cx)
        });
        let mut actions = div().flex().gap(px(8.));
        if let Some(site) = site {
            if let Some(url) = site.live_url.clone().filter(|_| !site.suspended()) {
                actions = actions.child(self.action(
                    "site-visit",
                    "Visit",
                    1,
                    window,
                    cx,
                    move |_, cx| cx.open_url(&url),
                ));
            }
            let edit = site.clone();
            let button = self.action("site-edit", "Edit", 2, window, cx, move |_, cx| {
                if !edit.suspended() {
                    cx.emit(Event::Edit(edit.clone()))
                }
            });
            actions = actions.child(if site.suspended() {
                button.opacity(0.5)
            } else {
                button
            });
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .pb(px(24.))
            .child(back)
            .child(actions)
    }

    fn site_details(&self, site: &Site, cx: &App) -> gpui::Div {
        let t = cx.theme();
        let address = site.live_url.clone().unwrap_or_else(|| site.slug.clone());
        let mut meta = vec![format!("Edited {}", updated(&site.updated_at, self.clock))];
        if site.suspended() {
            meta.push("Suspended".into());
        } else if site.live_url.is_none() {
            meta.push("Not published".into());
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text(site.title.clone(), 28., 36., 600., t.text))
            .child(text(address, 14., 20., 430., t.text_secondary))
            .child(text(meta.join(" · "), 13., 18., 430., t.text_secondary))
    }

    fn render_detail(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let load = self.detail.as_ref().map(|detail| &detail.load);
        let site = match load {
            Some(Load::Ready(site)) => Some(site),
            _ => None,
        };
        let header = self.detail_header(site, window, cx);
        let body = match load {
            Some(Load::Ready(site)) => self.site_details(site, cx).into_any_element(),
            Some(Load::Failed(error)) => self
                .state("Unable to load site", Some(error), cx)
                .child(self.retry(window, cx))
                .into_any_element(),
            _ => self.state("Loading site", None, cx).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body = match &self.load {
            _ if self.detail.is_some() => self.render_detail(window, cx),
            Load::Loading => self.state("Loading sites", None, cx).into_any_element(),
            Load::Failed(error) => self
                .state("Unable to load sites", Some(error), cx)
                .child(self.retry(window, cx))
                .into_any_element(),
            Load::Ready(sites) if sites.is_empty() => self
                .state(
                    "No sites yet",
                    Some(
                        "Build websites and apps with databases and sign-in—try “Build a team dashboard” or “Create an event signup page”",
                    ),
                    cx,
                )
                .into_any_element(),
            Load::Ready(sites) => {
                let rows: Vec<_> = sites
                    .iter()
                    .enumerate()
                    .map(|(index, site)| self.row(index, site, window, cx).into_any_element())
                    .collect();
                div().flex().flex_col().children(rows).into_any_element()
            }
        };
        let title = self.detail.is_none().then(|| {
            div()
                .pb(px(24.))
                .child(text("Sites", 28., 36., 600., t.text))
        });
        div()
            .id("sites-page")
            .size_full()
            .flex()
            .flex_col()
            .pt(px(30.))
            .px(px(30.))
            .bg(t.surface)
            .children(title)
            .child(
                div()
                    .id("sites-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(30.))
                    .child(body),
            )
    }
}

/// The original's `$n` last-edit label: `{m}m ago`, `{h}h ago`, `Yesterday`, else `Oct 15`.
fn updated(date: &str, clock: Clock) -> String {
    let Ok(date) = chrono::DateTime::parse_from_rfc3339(date) else {
        return String::new();
    };
    let now = clock.now();
    let seconds = (now - date.timestamp()).max(0);
    let day = |at: i64| (at + clock.utc_offset).div_euclid(86400);
    if seconds < 3600 {
        format!("{}m ago", (seconds / 60).max(1))
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else if day(now) - day(date.timestamp()) == 1 {
        "Yesterday".into()
    } else {
        let local = date.timestamp() + clock.utc_offset;
        chrono::DateTime::from_timestamp(local, 0)
            .map(|at| at.format("%b %-d").to_string())
            .unwrap_or_default()
    }
}

/// `/sites`: sidebar closed, the rail's `Sites` current.
pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.selected_rail = Some("sites");
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn updated_labels_follow_the_original_buckets() {
        // 2026-10-16T20:00:00Z, UTC clock.
        let clock = Clock {
            utc_offset: 0,
            pinned_now: Some(1_792_180_800),
            elapsed_since: None,
        };
        assert_eq!(updated("2026-10-16T19:59:30Z", clock), "1m ago");
        assert_eq!(updated("2026-10-16T17:00:00Z", clock), "3h ago");
        assert_eq!(updated("2026-10-15T10:00:00Z", clock), "Yesterday");
        assert_eq!(updated("2026-10-02T10:00:00Z", clock), "Oct 2");
    }

    #[test]
    fn a_malformed_item_fails_the_list() {
        let item = json!({"id":"a","title":"A","slug":"a","live_url":null,"updated_at":"x"});
        assert_eq!(sites(&json!({"items":[item]})).unwrap()[0].title, "A");
        assert!(sites(&json!({"items":[{"id":"a","slug":"a"}]})).is_err());
        assert!(sites(&json!({})).is_err());
    }
}
