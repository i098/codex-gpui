//! Rail `Sites` (`/sites`): the owned sites list the original's `AppgenPage` reads from
//! `GET /websites?limit=20` (`appgen-sites-query`).

use crate::screens::settings::button;
use crate::screens::thread::view::Clock;
use crate::{EmptySidebar, Shell, focus, text};
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
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready(Vec<Site>),
    Failed(String),
}

/// The `/websites` body (`{items, cursor}`); any malformed item fails the list, like `A.parse`.
pub fn sites(body: &Value) -> Result<Vec<Site>, String> {
    Vec::<Site>::deserialize(&body["items"]).map_err(|error| error.to_string())
}

/// `Retry` on a failed load.
pub struct Refresh;

pub struct Page {
    pub load: Load,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    clock: Clock,
}

impl EventEmitter<Refresh> for Page {}

impl Page {
    pub fn new(clock: Clock) -> Self {
        Self {
            load: Load::Loading,
            generation: 0,
            clock,
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
        let handle = focus::handle("sites-retry", 0, window, cx);
        focus::ring(
            button("Retry", t.text_alpha_5, None, &t)
                .id("sites-retry")
                .role(gpui::Role::Button)
                .aria_label("Retry"),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(|_, _, _, cx| cx.emit(Refresh)))
        .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.emit(Refresh);
                cx.stop_propagation();
            }
        }))
    }

    fn row(&self, site: &Site, cx: &App) -> impl IntoElement {
        let t = cx.theme();
        let mut meta = vec![updated(&site.updated_at, self.clock)];
        if site.live_url.is_none() {
            meta.push("Not published".into());
        }
        div()
            .id(SharedString::from(format!("site-{}", site.id)))
            .flex()
            .flex_col()
            .gap(px(2.))
            .py(px(10.))
            .border_b_1()
            .border_color(t.border)
            .child(text(site.title.clone(), 14., 20., 500., t.text))
            .child(text(meta.join(" · "), 13., 18., 430., t.text_secondary))
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body = match &self.load {
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
            Load::Ready(sites) => div()
                .flex()
                .flex_col()
                .children(sites.iter().map(|site| self.row(site, cx)))
                .into_any_element(),
        };
        div()
            .id("sites-page")
            .size_full()
            .flex()
            .flex_col()
            .pt(px(30.))
            .px(px(30.))
            .bg(t.surface)
            .child(
                div()
                    .pb(px(24.))
                    .child(text("Sites", 28., 36., 600., t.text)),
            )
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
