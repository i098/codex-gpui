//! Explore ▸ `Images` (`/images`): the generated-images grid, from the original's Library image
//! tab read `GET /files/library/nodes?…&categories=image` (`library-nodes` query).

use crate::screens::settings::button;
use crate::{EmptySidebar, Shell, focus, text};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use theme::ActiveTheme as _;

/// One image file of the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready(Vec<Image>),
    Failed(String),
}

/// The `/files/library/nodes` body (`{items, cursor}`): the `file` nodes whose MIME type is an
/// image. Directories and other files are skipped; a body without `items` fails.
pub fn images(body: &Value) -> Result<Vec<Image>, String> {
    let items = body["items"].as_array().ok_or("missing items")?;
    Ok(items
        .iter()
        .filter(|item| item["type"] == "file")
        .filter_map(|item| {
            let file = &item["file"];
            file["mime_type"]
                .as_str()?
                .starts_with("image/")
                .then_some(())?;
            Some(Image {
                id: file["id"].as_str()?.to_owned(),
                name: file["name"]
                    .as_str()
                    .or_else(|| item["name"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect())
}

/// `Try again` on a failed load.
pub struct Refresh;

pub struct Page {
    pub load: Load,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    /// The loaded thumbnail per file id.
    pub thumbnails: HashMap<String, Arc<gpui::Image>>,
}

impl EventEmitter<Refresh> for Page {}

/// The content column: the original's page body width at 1280 px.
const WIDTH: f32 = 728.;
const COLUMNS: usize = 4;
const GAP: f32 = 8.;

impl Page {
    pub fn new() -> Self {
        Self {
            load: Load::Loading,
            generation: 0,
            thumbnails: HashMap::new(),
        }
    }

    pub fn names(&self) -> Vec<&str> {
        match &self.load {
            Load::Ready(images) => images.iter().map(|image| image.name.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    fn retry(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let handle = focus::handle("images-retry", 0, window, cx);
        focus::ring(
            button("Try again", t.text_alpha_5, None, &t)
                .id("images-retry")
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

    /// Square tiles, `COLUMNS` per row.
    fn grid(&self, tiles: Vec<gpui::AnyElement>) -> impl IntoElement {
        let mut rows = div().flex().flex_col().gap(px(GAP));
        let mut tiles = tiles.into_iter().peekable();
        while tiles.peek().is_some() {
            let row: Vec<_> = tiles.by_ref().take(COLUMNS).collect();
            let missing = COLUMNS - row.len();
            rows = rows.child(
                div()
                    .flex()
                    .gap(px(GAP))
                    .children(row)
                    .children((0..missing).map(|_| div().flex_1())),
            );
        }
        rows
    }

    fn tile(&self, cx: &App) -> gpui::Stateful<gpui::Div> {
        let side = (WIDTH - GAP * (COLUMNS - 1) as f32) / COLUMNS as f32;
        div()
            .id("images-tile")
            .flex_1()
            .h(px(side))
            .relative()
            .overflow_hidden()
            .rounded(px(12.))
            .bg(cx.theme().surface_secondary)
    }

    fn image(&self, image: &Image, cx: &App) -> gpui::AnyElement {
        let t = cx.theme();
        let tile = self
            .tile(cx)
            .id(SharedString::from(format!("images-tile-{}", image.id)))
            .aria_label(SharedString::from(image.name.clone()));
        match self.thumbnails.get(&image.id) {
            Some(thumbnail) => tile.child(gpui::img(thumbnail.clone()).absolute().size_full()),
            // Until the thumbnail loads (or when it cannot), the file name stands in.
            None => tile.p(px(10.)).flex().items_end().child(text(
                image.name.clone(),
                12.,
                16.,
                430.,
                t.text_secondary,
            )),
        }
        .into_any_element()
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = cx.theme().clone();
        let message = |title: &'static str| {
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(text(title, 16., 24., 430., t.text))
        };
        match &self.load {
            Load::Loading => self
                .grid((0..8).map(|_| self.tile(cx).into_any_element()).collect())
                .into_any_element(),
            Load::Failed(_) => message("Couldn't load images")
                .child(self.retry(window, cx))
                .into_any_element(),
            Load::Ready(images) if images.is_empty() => message("No images yet").into_any_element(),
            Load::Ready(images) => self
                .grid(images.iter().map(|image| self.image(image, cx)).collect())
                .into_any_element(),
        }
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
        div()
            .id("images-page")
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
                            .child(text("Images", 24., 28.8, 500., t.text)),
                    )
                    .child(self.body(window, cx)),
            )
    }
}

/// `/images`: sidebar closed, the rail's Explore item current.
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
    fn only_image_files_are_listed() {
        let body = json!({"items": [
            {"type": "file", "name": "a.png", "file": {"id": "f1", "name": "a.png", "mime_type": "image/png"}},
            {"type": "file", "name": "n.md", "file": {"id": "f2", "name": "n.md", "mime_type": "text/markdown"}},
            {"type": "directory", "name": "Folder"},
            {"type": "file", "name": "b.webp", "file": {"id": "f3", "mime_type": "image/webp"}},
        ]});
        let ids: Vec<_> = images(&body)
            .unwrap()
            .into_iter()
            .map(|image| (image.id, image.name))
            .collect();
        assert_eq!(
            ids,
            [
                ("f1".into(), "a.png".into()),
                ("f3".into(), "b.webp".into())
            ]
        );
        assert!(images(&json!({})).is_err());
    }
}
