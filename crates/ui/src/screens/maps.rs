//! Explore ▸ `Maps` (`/maps`): the original's `MapsPage`, which reads
//! `GET /local/home` (`maps-home` query) for the prompt-suggestion cards, the nearby places and
//! the map center.

use crate::screens::settings::button;
use crate::{EmptySidebar, Shell, focus, text};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use theme::ActiveTheme as _;

/// One `personalized_prompts` card.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Card {
    pub id: String,
    pub primary_title: String,
    #[serde(default)]
    pub secondary_title: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub image_urls: Vec<String>,
}

impl Card {
    /// The card caption: `secondary_title || primary_title`.
    pub fn title(&self) -> &str {
        self.secondary_title
            .as_deref()
            .filter(|title| !title.is_empty())
            .unwrap_or(&self.primary_title)
    }
}

/// A map pin: a `nearby_entities` item whose `entity_data` has coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Home {
    pub cards: Vec<Card>,
    pub pins: Vec<Pin>,
    pub center: Option<Pin>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready(Home),
    Failed(String),
}

/// The `/local/home` body. A malformed card list fails the load; a malformed entity is skipped,
/// like the original's per-entity `safeParse`.
pub fn home(body: &Value) -> Result<Home, String> {
    let cards = Vec::<Card>::deserialize(&body["personalized_prompts"])
        .map_err(|error| error.to_string())?;
    let pin = |value: &Value| {
        Some(Pin {
            latitude: value["latitude"].as_f64()?,
            longitude: value["longitude"].as_f64()?,
        })
    };
    let pins = body["nearby_entities"]
        .as_array()
        .ok_or("missing nearby_entities")?
        .iter()
        .filter_map(|entity| pin(&entity["entity_data"]))
        .collect();
    Ok(Home {
        cards,
        pins,
        center: pin(&body["map_center"]),
    })
}

/// `Try again` on a failed load.
pub struct Refresh;

pub struct Page {
    pub load: Load,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    pub map: Option<Arc<gpui::Image>>,
    /// The first loaded image per card id.
    pub images: HashMap<String, Arc<gpui::Image>>,
}

impl EventEmitter<Refresh> for Page {}

/// The content column: the original's page body width at 1280 px.
const WIDTH: f32 = 728.;
pub const MAP_HEIGHT: f32 = 400.;

impl Page {
    pub fn new() -> Self {
        Self {
            load: Load::Loading,
            generation: 0,
            map: None,
            images: HashMap::new(),
        }
    }

    pub fn titles(&self) -> Vec<&str> {
        match &self.load {
            Load::Ready(home) => home.cards.iter().map(Card::title).collect(),
            _ => Vec::new(),
        }
    }

    fn retry(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let handle = focus::handle("maps-retry", 0, window, cx);
        focus::ring(
            button("Try again", t.text_alpha_5, None, &t)
                .id("maps-retry")
                .role(gpui::Role::Button)
                .aria_label("Try again"),
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

    /// The `grid-cols-3 gap-4` card grid; `aspect-3/4` `rounded-2xl` tiles.
    fn grid(&self, tiles: Vec<gpui::AnyElement>) -> impl IntoElement {
        let mut rows = div().flex().flex_col().gap(px(16.));
        let mut tiles = tiles.into_iter().peekable();
        while tiles.peek().is_some() {
            let row: Vec<_> = tiles.by_ref().take(3).collect();
            let missing = 3 - row.len();
            rows = rows.child(
                div()
                    .flex()
                    .gap(px(16.))
                    .children(row)
                    .children((0..missing).map(|_| div().flex_1())),
            );
        }
        rows
    }

    fn tile(&self, cx: &App) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme();
        let width = (WIDTH - 32.) / 3.;
        div()
            .id("maps-card")
            .flex_1()
            .h(px(width * 4. / 3.))
            .relative()
            .overflow_hidden()
            .rounded(px(16.))
            .bg(t.surface_secondary)
    }

    fn card(&self, card: &Card, cx: &App) -> gpui::AnyElement {
        self.tile(cx)
            .id(SharedString::from(format!("maps-card-{}", card.id)))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(card.title().to_owned()))
            .when_some(self.images.get(&card.id), |tile, image| {
                tile.child(gpui::img(image.clone()).absolute().size_full())
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .pt(px(32.))
                    .px(px(12.))
                    .pb(px(12.))
                    .child(text(card.title().to_owned(), 14., 20., 500., gpui::white())),
            )
            .into_any_element()
    }

    fn cards(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let t = cx.theme().clone();
        Some(match &self.load {
            Load::Loading => self
                .grid((0..6).map(|_| self.tile(cx).into_any_element()).collect())
                .into_any_element(),
            Load::Failed(_) => div()
                .flex()
                .flex_col()
                .items_start()
                .child(text(
                    "Couldn't load recommendations",
                    16.,
                    24.,
                    430.,
                    t.text,
                ))
                .child(self.retry(window, cx))
                .into_any_element(),
            Load::Ready(home) if home.cards.is_empty() => return None,
            Load::Ready(home) => self
                .grid(home.cards.iter().map(|card| self.card(card, cx)).collect())
                .into_any_element(),
        })
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let map = div()
            .id("maps-map")
            .w_full()
            .h(px(MAP_HEIGHT))
            .overflow_hidden()
            .rounded(px(25.))
            .bg(t.surface_secondary)
            .aria_label("Map")
            .when_some(self.map.clone(), |map, image| {
                map.child(gpui::img(image).size_full())
            });
        div()
            .id("maps-page")
            .size_full()
            .overflow_y_scroll()
            .bg(t.surface)
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(WIDTH))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .pt(px(24.))
                    .pb(px(44.))
                    .child(
                        div()
                            .pb(px(38.))
                            .child(text("Maps", 24., 28.8, 500., t.text)),
                    )
                    // The composer above the map is not built: the port cannot start a
                    // ChatGPT conversation.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(44.))
                            .child(map)
                            .children(self.cards(window, cx)),
                    ),
            )
    }
}

/// `/maps`: sidebar closed, the rail's Explore item current.
pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.selected_rail = Some("explore");
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
    fn home_reads_cards_pins_and_center() {
        let body = json!({
            "personalized_prompts": [
                {"id": "a", "primary_title": "Coffee", "secondary_title": "", "prompt": "Find coffee", "image_urls": ["https://x/a.jpg"]},
                {"id": "b", "primary_title": "Parks", "secondary_title": "Quiet parks", "prompt": "Find parks"},
            ],
            "nearby_entities": [
                {"entity_data": {"name": "Blue Bottle", "latitude": 37.78, "longitude": -122.4}},
                {"entity_data": {"name": "No coordinates"}},
            ],
            "map_center": {"latitude": 37.77, "longitude": -122.42},
        });
        let home = home(&body).unwrap();
        let titles: Vec<_> = home.cards.iter().map(Card::title).collect();
        assert_eq!(titles, ["Coffee", "Quiet parks"]);
        assert_eq!(
            home.pins,
            [Pin {
                latitude: 37.78,
                longitude: -122.4
            }]
        );
        assert_eq!(
            home.center,
            Some(Pin {
                latitude: 37.77,
                longitude: -122.42
            })
        );
    }

    #[test]
    fn home_rejects_a_malformed_card_list() {
        assert!(
            home(&json!({"personalized_prompts": [{"id": 1}], "nearby_entities": []})).is_err()
        );
        assert!(home(&json!({"personalized_prompts": []})).is_err());
    }
}
