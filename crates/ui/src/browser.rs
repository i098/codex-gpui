//! The thread side panel's in-app browser: one [`BrowserTab`] per browser tab of the strip,
//! each an address bar over a native web page.
//!
//! The page is a WKWebView laid over the window's view on macOS ([`page`]). A window without
//! a native view (gpui's headless renderer) keeps the tab's navigation state and draws no page.

use gpui::{
    Action, App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, SharedString, Styled, Task,
    Window, canvas, div, prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;
use url::Url;

use crate::composer::{Composer, SettingsFieldSubmit};
use crate::{icon, text};

mod page;

/// `thread.browser.addressPlaceholder`.
const ADDRESS_PLACEHOLDER: &str = "Search or enter a URL";
/// The label of a tab with no page.
pub const NEW_TAB_TITLE: &str = "New tab";
/// Address-bar text that is not an address searches the web here.
const SEARCH_URL: &str = "https://www.google.com/search";

/// `openBrowserTab` (`⌘T`) with no address, and a link's `Open in` ▸ `New tab` with one.
#[derive(Clone, Default, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct OpenBrowserTab(pub Option<String>);

/// Where the page is in loading its document.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LoadState {
    /// No page was requested yet.
    #[default]
    Idle,
    Loading,
    Loaded,
}

/// A change of the page that its owner shows outside the tab (the strip's label).
#[derive(Clone, Debug, PartialEq)]
pub enum BrowserEvent {
    LoadStarted { url: Url },
    /// The committed URL changed, including a same-document navigation.
    Navigated { url: Url },
    TitleChanged,
    LoadFinished,
}

/// What the page reports: its committed URL, title and history position.
#[derive(Clone, Debug, Default, PartialEq)]
struct PageState {
    url: Option<Url>,
    title: String,
    can_go_back: bool,
    can_go_forward: bool,
}

pub struct BrowserTab {
    page: page::Page,
    address: Entity<Composer>,
    state: PageState,
    load_state: LoadState,
    page_bounds: Bounds<Pixels>,
    /// Set by the owner while the tab is not on screen (another tab is selected).
    hidden: bool,
    focus: FocusHandle,
    _changes: Task<()>,
}

impl EventEmitter<BrowserEvent> for BrowserTab {}

impl BrowserTab {
    pub fn new(url: Option<Url>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Each message is one page change; `true` when the document finished loading.
        let (changed, changes) = async_channel::unbounded();
        let page = page::Page::new(window, changed);
        let address =
            cx.new(|cx| Composer::address_field("Address", ADDRESS_PLACEHOLDER, cx));
        cx.subscribe(&address, |tab, address, _: &SettingsFieldSubmit, cx| {
            let input = address.read(cx).field_text().to_owned();
            tab.navigate(&input, cx);
        })
        .detach();
        let address_focus = address.focus_handle(cx);
        cx.on_blur(&address_focus, window, |tab, _, cx| tab.show_url(cx))
            .detach();
        let changes = cx.spawn(async move |tab, cx| {
            while let Ok(finished) = changes.recv().await {
                if tab
                    .update(cx, |tab, cx| tab.page_changed(finished, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut tab = Self {
            page,
            address,
            state: PageState::default(),
            load_state: LoadState::Idle,
            page_bounds: Bounds::default(),
            hidden: false,
            focus: cx.focus_handle(),
            _changes: changes,
        };
        if let Some(url) = url {
            tab.load(url, cx);
        }
        tab
    }

    /// Loads the address-bar text `input`: an address, else a web search for it.
    pub fn navigate(&mut self, input: &str, cx: &mut Context<Self>) {
        if let Some(url) = address_url(input) {
            self.load(url, cx);
        }
    }

    fn load(&mut self, url: Url, cx: &mut Context<Self>) {
        self.page.load(&url);
        self.load_state = LoadState::Loading;
        cx.emit(BrowserEvent::LoadStarted { url });
    }

    /// The committed URL.
    pub fn url(&self) -> Option<&Url> {
        self.state.url.as_ref()
    }

    /// The strip label: the page title, else its host, else `New tab`.
    pub fn title(&self) -> SharedString {
        if !self.state.title.trim().is_empty() {
            return self.state.title.trim().to_owned().into();
        }
        self.url()
            .and_then(|url| url.host_str().map(str::to_owned))
            .map_or(NEW_TAB_TITLE.into(), Into::into)
    }

    pub fn load_state(&self) -> LoadState {
        self.load_state
    }

    pub fn can_go_back(&self) -> bool {
        self.state.can_go_back
    }

    pub fn can_go_forward(&self) -> bool {
        self.state.can_go_forward
    }

    /// The page area in window coordinates, as last laid out.
    pub fn page_bounds(&self) -> Bounds<Pixels> {
        self.page_bounds
    }

    /// Hides or shows the native page; it draws over every gpui element.
    pub fn set_page_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if self.hidden != hidden {
            self.hidden = hidden;
            self.page.set_hidden(hidden);
            cx.notify();
        }
    }

    pub fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.address.focus_handle(cx).focus(window, cx);
    }

    /// Reads the page again after it reported a change, emitting what differs.
    fn page_changed(&mut self, finished: bool, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.state, self.page.state());
        if previous.url != self.state.url {
            if let Some(url) = self.state.url.clone() {
                cx.emit(BrowserEvent::Navigated { url });
            }
            self.show_url(cx);
        }
        if previous.title != self.state.title {
            cx.emit(BrowserEvent::TitleChanged);
        }
        if finished && self.load_state == LoadState::Loading {
            self.load_state = LoadState::Loaded;
            cx.emit(BrowserEvent::LoadFinished);
        }
        cx.notify();
    }

    /// Puts the committed URL in the address bar.
    fn show_url(&mut self, cx: &mut Context<Self>) {
        let url = self.state.url.as_ref().map(Url::to_string).unwrap_or_default();
        self.address.update(cx, |field, cx| {
            if field.field_text() != url {
                field.set_text(&url, cx);
            }
        });
    }
}

/// The page for address-bar text: a URL with a scheme, a host (`example.com`,
/// `localhost:3000`), else a web search for the text.
fn address_url(input: &str) -> Option<Url> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(input)
        && matches!(url.scheme(), "http" | "https" | "file" | "about" | "data")
    {
        return Some(url);
    }
    if !input.contains(char::is_whitespace) {
        let host = input.split(['/', '?', '#']).next().unwrap_or(input);
        let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
        let local = name == "localhost" || name.parse::<std::net::Ipv4Addr>().is_ok();
        if local || (name.contains('.') && !name.starts_with('.') && !name.ends_with('.')) {
            let scheme = if local { "http" } else { "https" };
            if let Ok(url) = Url::parse(&format!("{scheme}://{input}")) {
                return Some(url);
            }
        }
    }
    Url::parse_with_params(SEARCH_URL, [("q", input)]).ok()
}

impl Render for BrowserTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let tab = cx.entity();
        let toolbar = div()
            .flex_none()
            .h(px(40.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .bg(t.text_alpha_5)
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .text_size(px(13.))
                    .child(self.address.clone()),
            );
        let empty = self.state.url.is_none() && self.load_state == LoadState::Idle;
        let page = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        tab.update(cx, |tab, _| {
                            tab.page_bounds = bounds;
                            if !tab.hidden {
                                tab.page.place(bounds);
                            }
                        })
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when(empty, |d| {
                // `thread.browser.emptyState`.
                d.child(
                    div()
                        .absolute()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(4.))
                        .child(icon("sidebar-cmd-browser", 24., t.text_secondary).mb(px(4.)))
                        .child(text("Start browsing", 14., 20., 500., t.text))
                        .child(text("Enter a URL to open a page", 14., 20., 400., t.text_secondary)),
                )
            });
        div()
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(page)
    }
}

impl Focusable for BrowserTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::address_url;

    #[test]
    fn address_bar_text_resolves_to_a_page_or_a_search() {
        let resolved = |input: &str| address_url(input).map(String::from);
        assert_eq!(resolved("https://openai.com/a?b"), Some("https://openai.com/a?b".into()));
        assert_eq!(resolved("openai.com/codex"), Some("https://openai.com/codex".into()));
        assert_eq!(resolved("localhost:3000/x"), Some("http://localhost:3000/x".into()));
        assert_eq!(resolved("127.0.0.1:8080"), Some("http://127.0.0.1:8080/".into()));
        assert_eq!(
            resolved("rust borrow checker"),
            Some("https://www.google.com/search?q=rust+borrow+checker".into())
        );
        assert_eq!(resolved("codex"), Some("https://www.google.com/search?q=codex".into()));
        assert_eq!(resolved("   "), None);
    }
}
