//! Explore ▸ `GPTs` (`/gpts`): the original's `GptDiscoveryPage`, which lists the user's GPTs
//! (`GET /gizmos/bootstrap?limit=20`) and the catalog (`GET /public-api/gizmos/discovery`), and
//! a GPT's page (`/g/:gizmoId`), which reads `GET /gizmos/{id}`.

use crate::screens::settings::button;
use crate::{EmptySidebar, Shell, focus, text};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use serde_json::Value;
use theme::ActiveTheme as _;

/// One gizmo's `display` and author.
#[derive(Clone, Debug, PartialEq)]
pub struct Gpt {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: Option<String>,
    pub prompt_starters: Vec<String>,
}

/// A `gizmo` object; `None` without an id. A missing name reads `Untitled`, like `My GPTs`.
fn gpt(gizmo: &Value) -> Option<Gpt> {
    let display = &gizmo["display"];
    let string = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    Some(Gpt {
        id: gizmo["id"].as_str()?.to_owned(),
        name: display["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .unwrap_or("Untitled")
            .to_owned(),
        description: string(&display["description"]),
        author: gizmo["author"]["display_name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
        prompt_starters: display["prompt_starters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    })
}

/// `resource.gizmo` of each item, without projects (`gizmo_type` `snorlax`), like the original's
/// discovery filter.
fn gpts(items: &Value) -> Result<Vec<Gpt>, String> {
    Ok(items
        .as_array()
        .ok_or("missing GPT list")?
        .iter()
        .map(|item| &item["resource"]["gizmo"])
        .filter(|gizmo| gizmo["gizmo_type"] != "snorlax")
        .filter_map(gpt)
        .collect())
}

/// The `/gizmos/bootstrap` body (`{gizmos, cursor}`).
pub fn mine(body: &Value) -> Result<Vec<Gpt>, String> {
    gpts(&body["gizmos"])
}

/// The `/public-api/gizmos/discovery` body (`{list: {items}, cursor}`).
pub fn catalog(body: &Value) -> Result<Vec<Gpt>, String> {
    gpts(&body["list"]["items"])
}

/// The `/gizmos/{id}` body (`{gizmo, tools, files, …}`).
pub fn detail(body: &Value) -> Result<Gpt, String> {
    gpt(&body["gizmo"]).ok_or_else(|| "missing gizmo".to_owned())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lists {
    pub mine: Vec<Gpt>,
    pub catalog: Vec<Gpt>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(String),
}

/// `Try again` on a failed load.
pub struct Refresh;
/// A GPT row: open its page.
pub struct Open(pub String);

/// What the page shows: the list, or one GPT.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    List(Load<Lists>),
    Gpt(String, Load<Gpt>),
}

pub struct Page {
    pub view: View,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
}

impl EventEmitter<Refresh> for Page {}
impl EventEmitter<Open> for Page {}

/// The content column: the original's page body width at 1280 px.
const WIDTH: f32 = 728.;

impl Page {
    /// The list, else the GPT `id`.
    pub fn new(id: Option<String>) -> Self {
        Self {
            view: match id {
                Some(id) => View::Gpt(id, Load::Loading),
                None => View::List(Load::Loading),
            },
            generation: 0,
        }
    }

    /// The names the page shows: both lists, else the open GPT.
    pub fn names(&self) -> Vec<&str> {
        match &self.view {
            View::List(Load::Ready(lists)) => lists
                .mine
                .iter()
                .chain(&lists.catalog)
                .map(|gpt| gpt.name.as_str())
                .collect(),
            View::Gpt(_, Load::Ready(gpt)) => vec![gpt.name.as_str()],
            _ => Vec::new(),
        }
    }

    fn retry(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let handle = focus::handle("gpts-retry", 0, window, cx);
        focus::ring(
            button("Try again", t.text_alpha_5, None, &t)
                .id("gpts-retry")
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

    fn row(&self, gpt: &Gpt, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme().clone();
        let hover = t.menu_item_hover;
        let id = gpt.id.clone();
        let mut meta = gpt.description.clone();
        if let Some(author) = &gpt.author {
            if !meta.is_empty() {
                meta.push_str(" · ");
            }
            meta.push_str(&format!("By {author}"));
        }
        div()
            .id(SharedString::from(format!("gpt-{}", gpt.id)))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(gpt.name.clone()))
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(12.))
            .hover(move |s| s.bg(hover))
            .child(text(gpt.name.clone(), 14., 20., 500., t.text))
            .child(text(meta, 13., 18., 430., t.text_secondary))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(Open(id.clone()))))
    }

    fn section(
        &self,
        title: &'static str,
        empty: &'static str,
        gpts: &[Gpt],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().pb(px(4.)).child(text(title, 16., 24., 500., t.text)))
            .children(gpts.is_empty().then(|| {
                div()
                    .px(px(12.))
                    .child(text(empty, 13., 18., 430., t.text_secondary))
            }))
            .children(gpts.iter().map(|gpt| self.row(gpt, cx)))
    }

    fn message(&self, title: &'static str, cx: &App) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .child(text(title, 16., 24., 430., cx.theme().text))
    }

    fn list(&self, load: &Load<Lists>, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme().clone();
        let heading = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pb(px(32.))
            .child(text("GPTs", 24., 28.8, 500., t.text))
            .child(text(
                "Discover custom versions of ChatGPT that combine instructions, extra knowledge, and any combination of skills",
                14., 20., 430., t.text_secondary,
            ));
        let body = match load {
            Load::Loading => self.message("Loading GPTs", cx).into_any_element(),
            Load::Failed(_) => self
                .message("Couldn't load GPTs", cx)
                .child(self.retry(window, cx))
                .into_any_element(),
            Load::Ready(lists) => div()
                .flex()
                .flex_col()
                .gap(px(32.))
                .child(self.section("My GPTs", "No GPTs here yet", &lists.mine, cx))
                .child(self.section("GPT catalog", "No GPTs available", &lists.catalog, cx))
                .into_any_element(),
        };
        div().flex().flex_col().child(heading).child(body)
    }

    fn gpt(&self, load: &Load<Gpt>, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme().clone();
        match load {
            Load::Loading => self.message("Loading GPT", cx),
            Load::Failed(_) => self
                .message("Couldn't load this GPT", cx)
                .child(self.retry(window, cx)),
            Load::Ready(gpt) => {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(40.))
                    .child(text(gpt.name.clone(), 24., 28.8, 500., t.text))
                    .children(gpt.author.as_ref().map(|author| {
                        text(format!("By {author}"), 13., 18., 430., t.text_secondary)
                    }))
                    .children(
                        (!gpt.description.is_empty())
                            .then(|| text(gpt.description.clone(), 14., 20., 430., t.text)),
                    )
                    .children((!gpt.prompt_starters.is_empty()).then(|| {
                        div()
                            .pt(px(24.))
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(text("Conversation starters", 14., 20., 500., t.text))
                            .children(gpt.prompt_starters.iter().map(|starter| {
                                div()
                                    .px(px(12.))
                                    .py(px(10.))
                                    .rounded(px(12.))
                                    .border_1()
                                    .border_color(t.border)
                                    .child(text(starter.clone(), 14., 20., 430., t.text))
                            }))
                    }))
            }
        }
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body = match self.view.clone() {
            View::List(load) => self.list(&load, window, cx),
            View::Gpt(_, load) => self.gpt(&load, window, cx),
        };
        div()
            .id("gpts-page")
            .size_full()
            .overflow_y_scroll()
            .bg(t.surface)
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(WIDTH))
                    .flex_none()
                    .pt(px(24.))
                    .pb(px(44.))
                    .child(body),
            )
    }
}

/// `/gpts` and `/g/:gizmoId`: sidebar closed, the rail's Explore item current.
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
    fn lists_skip_projects_and_name_untitled_gpts() {
        let item = |id: &str, kind: &str, name: &str| {
            json!({"resource": {"gizmo": {
                "id": id, "gizmo_type": kind,
                "author": {"display_name": "Ana"},
                "display": {"name": name, "description": "d", "prompt_starters": ["Hi"]},
            }}})
        };
        let body = json!({"list": {"items": [
            item("g-1", "gpt", "One"),
            item("g-p-1", "snorlax", "Project"),
            item("g-2", "gpt", ""),
        ]}});
        let gpts = catalog(&body).unwrap();
        let names: Vec<_> = gpts.iter().map(|gpt| gpt.name.as_str()).collect();
        assert_eq!(names, ["One", "Untitled"]);
        assert_eq!(gpts[0].author.as_deref(), Some("Ana"));
        assert_eq!(gpts[0].prompt_starters, ["Hi"]);
        assert!(mine(&json!({})).is_err());
        assert!(detail(&json!({"gizmo": {}})).is_err());
    }
}
