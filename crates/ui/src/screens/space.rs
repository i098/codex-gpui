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
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
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
    /// The item's `document_type`, which picks the row icon's `text-file-*` tint.
    pub document_type: Option<String>,
}

/// One landing tab: the bundle's `space.list.*Tab` keys, in its order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum View {
    #[default]
    Suggested,
    All,
    Pages,
    Pinned,
    Files,
    Sites,
}

impl View {
    const TABS: [View; 6] = [
        View::Suggested,
        View::All,
        View::Pages,
        View::Pinned,
        View::Files,
        View::Sites,
    ];

    fn label(self) -> &'static str {
        match self {
            View::Suggested => "Suggested",
            View::All => "All",
            View::Pages => "Pages",
            View::Pinned => "Pinned",
            View::Files => "Files",
            View::Sites => "Sites",
        }
    }

    /// Only a tab whose source the app loads is live. `All` (pages, files and Sites), `Pinned`,
    /// `Files` and `Sites` await their own requests, so they stay disabled rather than show the
    /// page list under a name that promises other items.
    fn ready(self) -> bool {
        matches!(self, View::Suggested | View::Pages)
    }
}

/// `space.list.suggestedTab` shows the recent window: `slice(0, 12)` in the bundle.
const RECENT_PAGES: usize = 12;

/// Icon tint by `document_type`, the bundle's `yKr` map onto the `--color-file-*` tokens.
fn page_icon_color(document_type: Option<&str>) -> gpui::Hsla {
    let rgb = match document_type {
        // `--color-file-canvas: var(--yellow-500)`
        Some("granola_whiteboard") => 0xe0ac00,
        // `--color-file-spreadsheet: var(--green-400)`
        Some("granola_workbook") => 0x04b84c,
        // `--color-file-presentation: var(--red-400)`
        Some("granola_presentation") => 0xfa423e,
        // `--color-file-document: var(--blue-400)` for a Page and an unknown type.
        _ => 0x0285ff,
    };
    gpui::rgb(rgb).into()
}

/// The divider's `bottom` when the row paints it: `._divider:after` sits at `0`, and
/// `._divider:has(+ ._row):after` moves it 1px into the next row. A hovered row, and the row
/// above it, paint none (`_sm7xw` stylesheet).
fn divider_bottom(index: usize, len: usize, hovered: Option<usize>) -> Option<f32> {
    (hovered != Some(index) && hovered != Some(index + 1))
        .then(|| if index + 1 < len { -1. } else { 0. })
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
    /// The landing's selected tab.
    pub view: View,
    /// The hovered page row, which also hides the divider of the row above it.
    pub hovered: Option<usize>,
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

    /// Change the landing tab, when that tab has a source.
    fn select(&mut self, view: View, cx: &mut Context<Self>) {
        if view.ready() && self.view != view {
            self.view = view;
            self.hovered = None;
            cx.notify();
        }
    }

    pub fn set_pages(&mut self, pages: Pages, cx: &mut Context<Self>) {
        self.pages = pages;
        self.hovered = None;
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
            view: View::Suggested,
            hovered: None,
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

    /// The Spaces, then the account's Pages under the landing's tab bar. The original's
    /// `space.gettingStarted.*` heading sits above template tiles; it stays hidden until the
    /// tiles land (#1595).
    fn landing(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .child(self.spaces_body(window, cx))
            // Legacy page `mt-8` section: `pt-4` above the tab bar, `gap-2` before the list.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pt(px(48.))
                    .child(self.page_tabs(window, cx))
                    .child(self.pages_body(window, cx)),
            )
    }

    /// The tab bar between the Space cards and the page list: `variant: browse`
    /// (`gap-1`, `pb-2`), 36px `rounded-full` pills at `px-4`, 13px/18.571 `font-medium`,
    /// selected `bg-primary-ghost-hover text-default`, unselected `text-secondary` with a
    /// half-strength ghost hover.
    fn page_tabs(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        div()
            .id("space-tabs")
            .role(gpui::Role::TabList)
            .flex()
            .items_center()
            .gap(px(4.))
            .pb(px(8.))
            .children(View::TABS.into_iter().map(|view| {
                let id = SharedString::from(format!("space-tab-{}", view.label().to_lowercase()));
                let selected = self.view == view;
                let ready = view.ready();
                let color = if selected { t.text } else { t.text_secondary };
                // Only the selected pill is a tab stop, as the bundle's `tabIndex` has it.
                let focus = focus::handle(id.clone(), if selected { 0 } else { -1 }, window, cx);
                focus::ring(
                    div()
                        .id(id)
                        .role(gpui::Role::Tab)
                        .aria_label(view.label())
                        .aria_selected(selected)
                        .aria_disabled(!ready)
                        .h(px(36.))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .when(selected, |d| d.bg(t.ghost_hover))
                        .when(!selected && ready, |d| {
                            d.hover(|s| s.bg(t.ghost_hover.opacity(0.5)))
                        })
                        .when(!ready, |d| d.opacity(0.5))
                        .child(text(view.label(), 13., 18.571_428, 500., color)),
                    &focus,
                    px(18.),
                    px(2.),
                    cx,
                )
                .when(ready, |d| {
                    d.on_click(cx.listener(move |page, _, _, cx| page.select(view, cx)))
                        .on_key_down(cx.listener(move |page, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                page.select(view, cx);
                                cx.stop_propagation();
                            }
                        }))
                })
            }))
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
            // `text-base` 14/21 `text-default`, 16px `text-page-light-16` icon tinted by
            // `document_type`, `gap-2`, `rounded-xl`, `hover:bg-primary-ghost-hover`, and a
            // `border-subtle` divider inset 12px that a hovered row hides above and below it.
            Pages::Ready(pages) => {
                let shown = match self.view {
                    View::Suggested => &pages[..pages.len().min(RECENT_PAGES)],
                    _ => &pages[..],
                };
                let hovered = self.hovered;
                div()
                    .flex()
                    .flex_col()
                    .children(shown.iter().enumerate().map(|(index, page)| {
                        let group = SharedString::from(format!("page-{}", page.id));
                        div()
                            .id(group.clone())
                            .relative()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(theme::Theme::RADIUS_XL)
                            .hover(|s| s.bg(t.ghost_hover))
                            .child(icon(
                                "text-page-light-16",
                                16.,
                                page_icon_color(page.document_type.as_deref()),
                            ))
                            .child(div().min_w_0().truncate().child(text(
                                page.title.clone(),
                                14.,
                                21.,
                                theme::Theme::FONT_WEIGHT_UI.0,
                                t.text,
                            )))
                            .when_some(divider_bottom(index, shown.len(), hovered), |d, bottom| {
                                d.child(
                                    div()
                                        .absolute()
                                        .bottom(px(bottom))
                                        .left(px(12.))
                                        .right(px(12.))
                                        .h(px(1.))
                                        .bg(t.border_subtle),
                                )
                            })
                            .on_hover(cx.listener(move |page, &is_hovered, _, cx| {
                                let next = if is_hovered {
                                    Some(index)
                                } else if page.hovered == Some(index) {
                                    None
                                } else {
                                    page.hovered
                                };
                                if page.hovered != next {
                                    page.hovered = next;
                                    cx.notify();
                                }
                            }))
                    }))
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_keep_the_bundle_order_and_only_loaded_sources_are_live() {
        assert_eq!(
            View::TABS.map(View::label),
            ["Suggested", "All", "Pages", "Pinned", "Files", "Sites"]
        );
        assert_eq!(View::default(), View::Suggested);
        assert!(View::Suggested.ready() && View::Pages.ready());
        for view in [View::All, View::Pinned, View::Files, View::Sites] {
            assert!(!view.ready(), "{view:?}");
        }
    }

    #[test]
    fn row_icon_keeps_the_document_tint_unless_a_file_type_says_otherwise() {
        // `Hsla` has no `PartialEq`; both sides run the same `rgb` conversion.
        let same = |a: gpui::Hsla, b: gpui::Rgba| {
            let b: gpui::Hsla = b.into();
            (a.h, a.s, a.l, a.a) == (b.h, b.s, b.l, b.a)
        };
        for (document_type, hex) in [
            (None, 0x0285ff),
            (Some("page"), 0x0285ff),
            (Some("granola_whiteboard"), 0xe0ac00),
            (Some("granola_workbook"), 0x04b84c),
            (Some("granola_presentation"), 0xfa423e),
        ] {
            assert!(
                same(page_icon_color(document_type), gpui::rgb(hex)),
                "{document_type:?}"
            );
        }
    }

    #[test]
    fn hover_clears_the_row_divider_and_the_divider_above_it() {
        // The last row's divider sits at its bottom; the others 1px into the next row.
        assert_eq!(divider_bottom(0, 3, None), Some(-1.));
        assert_eq!(divider_bottom(2, 3, None), Some(0.));
        // Hovering row 1 clears its own divider and row 0's.
        assert_eq!(divider_bottom(1, 3, Some(1)), None);
        assert_eq!(divider_bottom(0, 3, Some(1)), None);
        assert_eq!(divider_bottom(2, 3, Some(0)), Some(0.));
    }
}
