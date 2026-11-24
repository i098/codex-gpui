//! Rail `Space` (`/space`): the original's `LegacySpacesPage` list of the account's Spaces,
//! read from `GET /spaces/v2`, with its search filter, empty, no-match and load-error states.
//! `Files` opens the [`library`] view in the same page.

pub mod library;

use crate::{
    EmptySidebar, Shell, composer::Composer, confirmation::Confirmation, focus, icon::icon, text,
};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use theme::ActiveTheme as _;

/// One `/spaces/v2` item: the card shows `title || name`, as the original's list does.
#[derive(Clone, Debug, PartialEq)]
pub struct Space {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Spaces {
    #[default]
    Loading,
    Ready(Vec<Space>),
    /// `space.list.loadError`: the list could not load.
    Failed,
}

/// One private Page from `GET /pages?surface=private`.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Pages {
    #[default]
    Loading,
    Ready(Vec<Page>),
    Failed,
}

/// `space.list.retry`: load the list again.
#[derive(Clone, Debug)]
pub struct Retry;

/// `space.list.retryPersonalPages`: load the page list again.
#[derive(Clone, Debug)]
pub struct PageRetry;

pub struct SpacePage {
    pub spaces: Spaces,
    pub pages: Pages,
    pub query: Entity<Composer>,
    /// `Files`: the Library replaces the Spaces list while it is `Some`.
    pub library: Option<library::Library>,
    confirmation: Option<Entity<Confirmation>>,
    focus: FocusHandle,
}

impl EventEmitter<Retry> for SpacePage {}
impl EventEmitter<PageRetry> for SpacePage {}
impl EventEmitter<library::Action> for SpacePage {}
impl Focusable for SpacePage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SpacePage {
    pub fn set_spaces(&mut self, spaces: Spaces, cx: &mut Context<Self>) {
        self.spaces = spaces;
        cx.notify();
    }

    pub fn set_pages(&mut self, pages: Pages, cx: &mut Context<Self>) {
        self.pages = pages;
        cx.notify();
    }

    /// The spaces whose name contains the trimmed, case-folded search text.
    pub fn visible(&self, cx: &App) -> Vec<Space> {
        let query = self.query.read(cx).field_text().trim().to_lowercase();
        match &self.spaces {
            Spaces::Ready(spaces) => spaces
                .iter()
                .filter(|space| space.name.to_lowercase().contains(&query))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The open `Delete forever?` dialog.
    pub fn confirmation(&self) -> Option<&Entity<Confirmation>> {
        self.confirmation.as_ref()
    }
}

/// A rounded text button with a keyboard focus ring; Enter and Space press it too.
fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<SpacePage>,
    press: impl Fn(&mut SpacePage, &mut Window, &mut Context<SpacePage>) + 'static,
) -> Stateful<Div> {
    let id = id.into();
    let label = label.into();
    let t = cx.theme().clone();
    let focus = focus::handle(id.clone(), 0, window, cx);
    let press = std::rc::Rc::new(press);
    let key_press = press.clone();
    focus::ring(
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .h(px(28.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .rounded_full()
            .hover(|d| d.bg(t.ghost_hover))
            .child(text(label, 13., 18., 500., t.text)),
        &focus,
        px(14.),
        px(2.),
        cx,
    )
    .on_click(cx.listener(move |page, _, window, cx| press(page, window, cx)))
    .on_key_down(cx.listener(move |page, event: &KeyDownEvent, window, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            key_press(page, window, cx);
            cx.stop_propagation();
        }
    }))
}

/// `/space`: sidebar closed, rail `Space` current.
pub fn shell(cx: &mut App) -> (Entity<Shell>, Entity<SpacePage>) {
    let sidebar = cx.new(|_| EmptySidebar).into();
    let page = cx.new(|cx| {
        let query = cx.new(|cx| {
            Composer::embedded_field("Search spaces and content", "", px(13.), px(18.), cx)
        });
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        SpacePage {
            spaces: Spaces::Loading,
            pages: Pages::Loading,
            query,
            library: None,
            confirmation: None,
            focus: cx.focus_handle(),
        }
    });
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.clone().into());
        shell.sidebar_open = false;
        shell.selected_rail = Some("space");
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell
    });
    (shell, page)
}

impl Render for SpacePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let (header, body) = match self.library.clone() {
            Some(library) => (
                self.library_header(&library, window, cx),
                self.library_body(&library, window, cx),
            ),
            None => (self.spaces_header(window, cx), self.landing(window, cx)),
        };
        div()
            .id("space-page")
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .pt(px(30.))
            .px(px(30.))
            .bg(t.surface)
            .child(header)
            .child(
                div()
                    .id("space-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(30.))
                    .child(body),
            )
            .children(self.confirmation.clone())
    }
}

impl SpacePage {
    fn spaces_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let search = div()
            .id("space-search")
            .w(px(240.))
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .rounded_full()
            .border_1()
            .border_color(t.border_strong)
            .child(icon("search", 16., t.text_secondary))
            .child(div().flex_1().min_w_0().child(self.query.clone()));
        let files = button("space-files", "Files", window, cx, |page, _, cx| {
            page.open_library(cx)
        });
        div()
            .h(px(32.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .mb(px(24.))
            .child(text("Space", 24., 28.8, 500., t.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(files)
                    .child(search),
            )
    }

    /// The Spaces, then the account's Pages. The original's `space.gettingStarted.*` heading
    /// sits above template tiles; it stays hidden until the tiles land (#1595).
    fn landing(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .child(self.spaces_body(window, cx))
            // Legacy page `mt-8` section around recent-pages `pt-4`.
            .child(div().pt(px(48.)).child(self.pages_body(window, cx)))
    }

    fn pages_body(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let message = |s: &'static str| text(s, 14., 20., 400., t.text_secondary);
        match &self.pages {
            Pages::Loading => div(),
            Pages::Failed => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(message("Could not load pages"))
                .child(button(
                    "pages-retry",
                    "Try again",
                    window,
                    cx,
                    |_, _, cx| cx.emit(PageRetry),
                )),
            Pages::Ready(pages) if pages.is_empty() => message("No pages yet"),
            // `ContentRow` in a `sidebar-navigation` list: `py-row-y` 5px, `px-row-x` 8px,
            // `text-base` 14/21 `text-default`, 16px page icon, `gap-2`, `rounded-xl`,
            // `hover:bg-primary-ghost-hover`, and a `border-subtle` divider inset 12px.
            Pages::Ready(pages) => div().flex().flex_col().children(pages.iter().map(|page| {
                let group = SharedString::from(format!("page-{}", page.id));
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(theme::Theme::RADIUS_XL)
                    .hover(|s| s.bg(t.ghost_hover))
                    .child(icon("diff-file-default", 16., t.text_secondary))
                    .child(div().min_w_0().truncate().child(text(
                        page.title.clone(),
                        14.,
                        21.,
                        theme::Theme::FONT_WEIGHT_UI.0,
                        t.text,
                    )))
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left(px(12.))
                            .right(px(12.))
                            .h(px(1.))
                            .bg(t.border_subtle)
                            .group_hover(group, |s| s.invisible()),
                    )
            })),
        }
    }

    fn spaces_body(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let message = |s: &'static str| text(s, 14., 20., 400., t.text_secondary);
        match &self.spaces {
            Spaces::Loading => div(),
            Spaces::Failed => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(message("Could not load spaces"))
                .child(button(
                    "space-retry",
                    "Try again",
                    window,
                    cx,
                    |_, _, cx| cx.emit(Retry),
                )),
            Spaces::Ready(spaces) => {
                let visible = self.visible(cx);
                if visible.is_empty() {
                    message(if spaces.is_empty() {
                        "No spaces yet"
                    } else {
                        "No matching spaces"
                    })
                } else {
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(12.))
                        .children(visible.into_iter().map(|space| {
                            div()
                                .id(gpui::SharedString::from(format!("space-{}", space.id)))
                                .w(px(240.))
                                .h(px(48.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .px(px(12.))
                                .rounded(px(12.))
                                .border_1()
                                .border_color(t.border_subtle)
                                .bg(t.surface_elevated)
                                .child(icon("space", 16., t.text_secondary))
                                .child(div().min_w_0().truncate().child(text(
                                    space.name,
                                    14.,
                                    20.,
                                    400.,
                                    t.text_secondary,
                                )))
                        }))
                }
            }
        }
    }
}
