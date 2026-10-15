//! `automations` parity screen: rail `Scheduled` (`/automations`). Sidebar `ScheduledSidebar`
//! and the page's empty state with suggested tasks, from `refs/automations/*.spec.json`.
//! Host data (`list-automations`, `/suggested_automations`) comes from the recorded answers
//! in `tools/route1/oracle.json`.

use crate::{Shell, icon::icon, icon_button, text};
use gpui::{
    Action, AnyView, App, AppContext as _, Context, Corners, Entity, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    actions, canvas, div, img, px,
};
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::Path};
use theme::{ActiveTheme as _, Theme};

actions!(
    automations,
    [
        /// Sidebar `New task`: opens the create flow.
        NewTask,
        /// Sidebar `Search scheduled tasks`.
        SearchTasks,
        /// Sidebar `Filter tasks` (ownership `Personal` / `Team`).
        FilterTasks,
    ]
);

/// A suggested-task card was clicked: start a task from its prompt.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct StartSuggestedTask {
    pub prompt: SharedString,
}

/// One `/suggested_automations` entry (or a built-in landing example).
#[derive(Clone, Deserialize)]
struct Suggestion {
    title: SharedString,
    description: SharedString,
    user_message: SharedString,
    emoji: SharedString,
    icon_url: Option<String>,
}

/// The recorded answer of the first `tools/route1/oracle.json` entry whose key contains
/// `key`: a host fetch body (`bodyJsonString`) or a ChatGPT backend body (`__response.body`).
fn oracle(key: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/route1/oracle.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let all: serde_json::Map<String, Value> = serde_json::from_str(&text).expect("oracle JSON");
    let entry = all
        .iter()
        .find(|(k, _)| k.contains(key))
        .unwrap_or_else(|| panic!("oracle.json: no entry for {key}"))
        .1;
    let body = entry["bodyJsonString"]
        .as_str()
        .or_else(|| entry["value"]["response"]["__response"]["body"].as_str())
        .unwrap_or_else(|| panic!("oracle.json: {key} has no body"));
    serde_json::from_str(body).expect("oracle body JSON")
}

/// `scheduled.landing.example.*` in bundle order: the page fills up to 6 cards with these
/// after the server's suggestions. ponytail: the original shuffles them per mount
/// (`Math.random`); the port keeps bundle order.
fn landing_examples() -> Vec<Suggestion> {
    [
        (
            "🥗",
            "Weekly meal plan",
            "Plan meals for the week and make a grocery list.",
            "Help me set up a weekly meal plan with a grocery list. Ask about my dietary preferences, household size, and when I’d like to receive it before scheduling.",
        ),
        (
            "💰",
            "Weekly finances update",
            "Review my recent spending, and anything that needs attention.",
            "Help me set up a weekly review of my recent spending, recurring charges, and anything that needs attention. Ask which financial information I want to use and what day and time works for me before scheduling.",
        ),
        (
            "😴",
            "Sleep coaching",
            "Track my sleep quality and help me improve my habits.",
            "Help me set up a regular sleep check-in. Ask about my sleep goals, the information I’m comfortable sharing, and when I’d like to check in before scheduling.",
        ),
        (
            "🛩",
            "Find flights for a trip",
            "Find the best deal on flights for an upcoming trip.",
            "Help me set up a flight price watch for an upcoming trip. Ask where I’m flying from and to, my dates, budget, and how often to check. Notify me when there’s a good match.",
        ),
        (
            "✉️",
            "Email monitor",
            "Scan my emails and let me know anything that needs my attention.",
            "Help me set up an email monitor that flags messages needing my attention. Ask which inbox or senders to watch, what counts as important, and how often to check before scheduling. Help me connect email if needed.",
        ),
    ]
    .into_iter()
    .map(|(emoji, title, description, prompt)| Suggestion {
        title: title.into(),
        description: description.into(),
        user_message: prompt.into(),
        emoji: emoji.into(),
        icon_url: None,
    })
    .collect()
}

/// Server suggestions, then landing examples whose title is not already shown, at most 6.
fn cards(mut all: Vec<Suggestion>) -> Vec<Suggestion> {
    let key = |s: &Suggestion| s.title.trim().to_lowercase();
    let taken: Vec<String> = all.iter().map(key).collect();
    all.extend(
        landing_examples()
            .into_iter()
            .filter(|s| !taken.contains(&key(s))),
    );
    all.truncate(6);
    all
}

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let server = serde_json::from_value(oracle("\"url\":\"/suggested_automations\""))
        .expect("suggested_automations shape");
    let shell = shell_with(server, cx);
    shell.update(cx, |shell, _| shell.can_go_back = true);
    shell.into()
}

/// The app's `/scheduled` route: no automations yet. The port has no ChatGPT backend for
/// `/suggested_automations`, so the cards are the landing examples, as the original shows
/// them without server suggestions.
pub fn shell(cx: &mut App) -> Entity<Shell> {
    shell_with(Vec::new(), cx)
}

fn shell_with(server: Vec<Suggestion>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| ScheduledSidebar).into();
    let main = cx
        .new(|_| AutomationsPage {
            cards: cards(server),
        })
        .into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, main);
        shell.selected_rail = Some("scheduled");
        shell.new_tab = false;
        shell
    })
}

/// `ScheduledSidebar`: header `Scheduled` + search, `New task`, section `Upcoming`.
pub struct ScheduledSidebar;

impl Render for ScheduledSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let hover_bg = t.list_hover;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .child(
                        div()
                            .h(px(52.))
                            .ml(px(8.))
                            .pr(px(4.))
                            .flex()
                            .items_center()
                            .child(div().flex_1().font_family("OpenAI Sans").child(text(
                                "Scheduled",
                                18.,
                                24.,
                                600.,
                                t.text_default_nav,
                            )))
                            .child(
                                icon_button(
                                    "search-tasks",
                                    icon("search", 16., t.text_tertiary),
                                    t,
                                )
                                .on_click(crate::dispatch::<SearchTasks>),
                            ),
                    )
                    .child(
                        div()
                            .id("new-task")
                            .mb(px(8.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .rounded(px(12.5))
                            .hover(move |s| s.bg(hover_bg))
                            .on_click(crate::dispatch::<NewTask>)
                            .child(icon("automations-plus", 16., t.text_default_nav))
                            .child(text("New task", 14., 21., 430., t.text_default_nav)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .px(px(8.))
                    .pb(px(8.))
                    .child(
                        div()
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(8.))
                            .pr(px(6.))
                            .child(text("Upcoming", 14., 21., 500., t.text_tertiary))
                            .child(
                                icon_button(
                                    "filter-tasks",
                                    icon("automations-filter", 16., t.text_tertiary),
                                    t,
                                )
                                .on_click(crate::dispatch::<FilterTasks>),
                            ),
                    )
                    .child(div().mt(px(1.)).px(px(8.)).py(px(4.)).child(text(
                        "No scheduled tasks",
                        13.,
                        18.571_428,
                        430.,
                        t.text_tertiary,
                    ))),
            )
    }
}

/// The ref captures' partial rasters of the card tiles (`shapes::Decoration::playback`), by
/// viewport: after the svg `<img>` reload, cc re-rastered these root tiles through these rects
/// (device px), found by replaying Chromium's command log per tile. Both themes alike.
fn card_playback(window: &Window) -> &'static [([i32; 2], [i32; 4])] {
    if window.viewport_size().width >= px(1440.) {
        &[
            ([1020, 510], [1085, 748, 1532, 880]),
            ([1020, 1020], [1075, 1140, 1532, 1368]),
            ([1530, 510], [1532, 900, 2042, 922]),
        ]
    } else {
        &[
            ([510, 510], [924, 770, 1022, 1022]),
            ([510, 1020], [915, 1070, 1022, 1300]),
        ]
    }
}

/// Main column of the empty page: hero and the suggested-task grid.
pub struct AutomationsPage {
    cards: Vec<Suggestion>,
}

impl AutomationsPage {
    fn card(
        &self,
        i: usize,
        s: &Suggestion,
        playback: &'static [([i32; 2], [i32; 4])],
        cx: &App,
    ) -> impl IntoElement {
        let t: &Theme = cx.theme();
        let hover_bg = t.ghost_hover;
        // `icon_url` is shown from its bundled copy under assets/ui/automations, its paths
        // rasterized by Chromium's Skia like Blink draws an svg `<img>`. Elsewhere GPUI's own
        // image raster (the copies declare 40x40, the 20 px slot at 2x: GPUI rasterizes svg
        // images at their intrinsic size).
        let glyph = match &s.icon_url {
            Some(url) => {
                let path = format!("ui/automations/{}", url.rsplit('/').next().unwrap_or(url));
                if cfg!(target_os = "macos") {
                    canvas(
                        |_, _, _| {},
                        move |b, (), window, cx| {
                            // ponytail: the ref capture reloads svg `<img>`s
                            // (RELOAD_SVG_IMAGES) and cc re-rasters each one's tile; the tiles
                            // rastered before the next draw reuse their resource (partial
                            // raster of the img box), the others raster in full. That is timing,
                            // fitted to the refs: card 0, and card 2 at 1440. Drop it with a
                            // deterministic capture.
                            let partial =
                                i == 0 || i == 2 && window.viewport_size().width >= px(1440.);
                            if let Ok(Some(src)) = cx.asset_source().load(&path) {
                                let white = gpui::white();
                                shapes::paint_icon(
                                    &src, &path, b, false, white, None, partial, window, cx,
                                );
                            }
                        },
                    )
                    .size(px(20.))
                    .into_any_element()
                } else {
                    img(path).size(px(20.)).into_any_element()
                }
            }
            None => text(s.emoji.clone(), 28., 28., 430., t.text).into_any_element(),
        };
        let prompt = s.user_message.clone();
        div()
            .id(("suggestion", i))
            .flex_1()
            .min_w_0()
            .min_h(px(108.))
            .flex()
            .items_center()
            .gap(px(16.))
            .relative()
            // 20 px padding inside a 1 px dashed border.
            .p(px(21.))
            .rounded(px(24.))
            // `border: 1px dashed` (Blink's curved dashed side, `shapes`).
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(24.)),
                border: Some((px(1.), t.border)),
                dashed: true,
                playback,
                ..Default::default()
            }))
            .hover(move |st| st.bg(hover_bg))
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(StartSuggestedTask {
                        prompt: prompt.clone(),
                    }),
                    cx,
                )
            })
            .child(
                div()
                    .size(px(24.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        text(s.title.clone(), 16., 26., 400., t.text)
                            .font_features(textsys::letter_spacing(-0.32))
                            .truncate(),
                    )
                    .child(
                        text(s.description.clone(), 14., 20., 400., t.text_tertiary)
                            .whitespace_normal()
                            .line_clamp(2)
                            .text_ellipsis(),
                    ),
            )
    }
}

impl Render for AutomationsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playback = card_playback(window);
        let t = cx.theme();
        let rows = self.cards.chunks(2).enumerate().map(|(r, pair)| {
            div().flex().gap(px(20.)).children(
                pair.iter()
                    .enumerate()
                    .map(|(c, s)| self.card(r * 2 + c, s, playback, cx)),
            )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            // Page header row (`--height-toolbar`), empty in this state.
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("automations-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .min_h_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .pt(px(24.))
                            .px(px(12.))
                            .pb(px(44.))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(738.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(44.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap(px(12.))
                                            .child(
                                                text("🕗", 48., 48., 400., t.text)
                                                    .font_features(textsys::letter_spacing(-1.2)),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .items_center()
                                                    .gap(px(2.))
                                                    .child(
                                                        text("Schedule a task", 18., 26., 400., t.text)
                                                            .font_features(textsys::letter_spacing(-0.45)),
                                                    )
                                                    .child(
                                                        text(
                                                            "ChatGPT can take care of ongoing tasks so you don’t have to.",
                                                            14.,
                                                            20.,
                                                            400.,
                                                            t.text_tertiary,
                                                        )
                                                        .font_features(textsys::letter_spacing(-0.14)),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .gap(px(20.))
                                            .children(rows),
                                    ),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cards_fill_six_from_oracle() {
        let server = serde_json::from_value(super::oracle("\"url\":\"/suggested_automations\""))
            .expect("shape");
        let titles: Vec<_> = super::cards(server).into_iter().map(|s| s.title).collect();
        assert_eq!(titles.len(), 6);
        assert_eq!(titles[0], "AP Lang practice");
        assert_eq!(titles[3], "Weekly meal plan");
    }
}
