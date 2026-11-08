//! The thread side panel's in-app browser: one [`BrowserTab`] per browser tab of the strip,
//! each an address bar over a native web page.
//!
//! The page is a WKWebView laid over the window's view on macOS ([`page`]). A window without
//! a native view (gpui's headless renderer) keeps the tab's navigation state and draws no page.

use gpui::{
    Action, Animation, AnimationExt as _, App, AppContext as _, Bounds, Context, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Task, Transformation, Window,
    canvas, div, prelude::FluentBuilder as _, px, radians,
};
use std::time::Duration;
use theme::ActiveTheme as _;
use url::Url;

use crate::composer::{Composer, SettingsFieldSubmit};
use crate::tooltip::{Side, tooltip};
use crate::{icon, icon_button, text};

#[cfg(target_os = "macos")]
mod menu;
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

// `navigateBrowserBack` (`⌘←`), `navigateBrowserForward` (`⌘→`), `reloadBrowserPage` (`⌘R`),
// `hardReloadBrowserPage` (`⌘⇧R`), `focusBrowserAddressBar` (`⌘L`) and
// `browser.toggleAnnotationMode` (`⌘⇧.`): the selected browser tab's commands.
gpui::actions!(
    ui,
    [
        BrowserBack,
        BrowserForward,
        ReloadBrowserPage,
        HardReloadBrowserPage,
        FocusBrowserAddressBar,
        ToggleAnnotationMode
    ]
);

/// Where the page is in loading its document.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LoadState {
    /// No page was requested yet.
    #[default]
    Idle,
    Loading,
    Loaded,
    /// The main-frame navigation failed: the tab shows its error page.
    Failed,
}

/// A failed main-frame load: the page it was for and its Chromium error name
/// (`ERR_NAME_NOT_RESOLVED`), which picks the error page's summary.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    pub url: Url,
    pub code: &'static str,
}

impl LoadError {
    /// `browserSidebar.loadError.*Summary` for the failed page's host.
    fn summary(&self) -> String {
        let host = self
            .url
            .host_str()
            .filter(|host| !host.is_empty())
            .unwrap_or(self.url.path());
        match self.code {
            "DNS_PROBE_POSSIBLE" | "ERR_NAME_NOT_RESOLVED" => {
                format!("{host}'s server IP address could not be found")
            }
            "ERR_INTERNET_DISCONNECTED" => {
                format!("{host} could not be loaded because the computer is offline")
            }
            "ERR_CONNECTION_REFUSED" => format!("{host} refused to connect"),
            "ERR_CONNECTION_TIMED_OUT" | "ERR_TIMED_OUT" => {
                format!("{host} took too long to respond")
            }
            code if code.starts_with("ERR_CERT_") => {
                format!("{host}'s certificate could not be verified")
            }
            _ => format!("{host} could not be loaded"),
        }
    }
}

/// A change of the page that its owner shows outside the tab (the strip's label).
#[derive(Clone, Debug, PartialEq)]
pub enum BrowserEvent {
    LoadStarted {
        url: Url,
    },
    /// The committed URL changed, including a same-document navigation.
    Navigated {
        url: Url,
    },
    TitleChanged,
    LoadFinished,
    /// The annotation toolbar's Send: the owner sends the pending annotations as a turn.
    SendAnnotations,
    /// The annotation toolbar's Discard: the owner confirms, then drops the pending
    /// annotations.
    DiscardAnnotations,
}

/// What the page reports: its committed URL, title, history position and whether it is
/// loading a document.
#[derive(Clone, Debug, Default, PartialEq)]
struct PageState {
    url: Option<Url>,
    title: String,
    can_go_back: bool,
    can_go_forward: bool,
    loading: bool,
}

pub struct BrowserTab {
    page: page::Page,
    address: Entity<Composer>,
    state: PageState,
    load_state: LoadState,
    /// The failed load the error page shows, over the last committed page.
    error: Option<LoadError>,
    /// The error page's `#details` suggestions are expanded.
    error_details: bool,
    page_bounds: Bounds<Pixels>,
    /// Set by the owner while the tab is not on screen (another tab is selected).
    hidden: bool,
    /// `thread.browser.commentMode`: clicks on the page annotate it.
    annotating: bool,
    /// The chat's browser annotations not sent yet, as the owner reports them.
    pending_annotations: usize,
    focus: FocusHandle,
    _changes: Task<()>,
}

impl EventEmitter<BrowserEvent> for BrowserTab {}

impl BrowserTab {
    pub fn new(url: Option<Url>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (changed, changes) = async_channel::unbounded();
        let page = page::Page::new(window, changed);
        let address = cx.new(|cx| Composer::address_field("Address", ADDRESS_PLACEHOLDER, cx));
        cx.subscribe(&address, |tab, address, _: &SettingsFieldSubmit, cx| {
            let input = address.read(cx).field_text().to_owned();
            tab.navigate(&input, cx);
        })
        .detach();
        let address_focus = address.focus_handle(cx);
        cx.on_blur(&address_focus, window, |tab, _, cx| tab.show_url(cx))
            .detach();
        let changes = cx.spawn_in(window, async move |tab, cx| {
            while let Ok(change) = changes.recv().await {
                #[cfg(target_os = "macos")]
                if let page::PageChange::ContextMenu { x, y, link } = change {
                    let shown = tab.update_in(cx, |tab, window, cx| {
                        tab.context_menu(x, y, link, window, cx)
                    });
                    if shown.is_err() {
                        break;
                    }
                    continue;
                }
                if tab
                    .update(cx, |tab, cx| tab.page_changed(change, cx))
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
            error: None,
            error_details: false,
            page_bounds: Bounds::default(),
            hidden: false,
            annotating: false,
            pending_annotations: 0,
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
        self.clear_error();
        self.page.load(&url);
        self.load_state = LoadState::Loading;
        cx.emit(BrowserEvent::LoadStarted { url });
        cx.notify();
    }

    /// `Back`: the previous history entry; from an error page, the page under it.
    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.error.is_some() && self.state.url.is_some() {
            self.clear_error();
            self.load_state = LoadState::Loaded;
            self.show_url(cx);
            cx.notify();
        } else if self.state.can_go_back {
            self.clear_error();
            self.page.back();
            self.load_state = LoadState::Loading;
            cx.notify();
        }
    }

    /// `Next`: the next history entry.
    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.state.can_go_forward {
            self.clear_error();
            self.page.forward();
            self.load_state = LoadState::Loading;
            cx.notify();
        }
    }

    /// `Reload page`; `hard` skips the cache (`⌘⇧R`). An error page loads its address again.
    pub fn reload(&mut self, hard: bool, cx: &mut Context<Self>) {
        if let Some(error) = self.error.clone() {
            self.load(error.url, cx);
        } else if self.state.url.is_some() {
            self.page.reload(hard);
            self.load_state = LoadState::Loading;
            cx.notify();
        }
    }

    /// The failed load the tab's error page shows.
    pub fn load_error(&self) -> Option<&LoadError> {
        self.error.as_ref()
    }

    fn clear_error(&mut self) {
        self.error_details = false;
        if self.error.take().is_some() {
            // The next frame's layout places the page again.
            self.page.set_hidden(self.hidden);
        }
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

    /// An error page goes back to the page under it.
    pub fn can_go_back(&self) -> bool {
        self.state.can_go_back || (self.error.is_some() && self.state.url.is_some())
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
            self.page.set_hidden(hidden || self.error.is_some());
            cx.notify();
        }
    }

    /// `⌘T` and `⌘L`: the address bar focused with its text selected.
    pub fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.address.focus_handle(cx).focus(window, cx);
        window.dispatch_action(Box::new(crate::SelectAll), cx);
    }

    /// `⌘⇧.` and the Annotate button: enters annotation mode on a page, or leaves it.
    pub fn toggle_annotation_mode(&mut self, cx: &mut Context<Self>) {
        self.set_annotating(!self.annotating && self.can_annotate(), cx);
    }

    /// `Exit annotation mode` keeps the pending annotations; Send leaves the mode too.
    fn set_annotating(&mut self, annotating: bool, cx: &mut Context<Self>) {
        if self.annotating != annotating {
            self.annotating = annotating;
            cx.notify();
        }
    }

    pub fn annotating(&self) -> bool {
        self.annotating
    }

    /// The original disables the Annotate button without a page to annotate.
    fn can_annotate(&self) -> bool {
        self.state.url.is_some() && self.error.is_none()
    }

    pub fn pending_annotations(&self) -> usize {
        self.pending_annotations
    }

    pub fn set_pending_annotations(&mut self, count: usize, cx: &mut Context<Self>) {
        if self.pending_annotations != count {
            self.pending_annotations = count;
            cx.notify();
        }
    }

    /// Reads the page again after it reported a change, emitting what differs.
    fn page_changed(&mut self, change: page::PageChange, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.state, self.page.state());
        if let page::PageChange::Failed(error) = change {
            // The error page draws in place of the native page.
            self.page.set_hidden(true);
            self.error = Some(error);
            self.error_details = false;
            self.load_state = LoadState::Failed;
            self.annotating = false;
            self.show_url(cx);
            cx.emit(BrowserEvent::LoadFinished);
            cx.notify();
            return;
        }
        if previous.url != self.state.url {
            if let Some(url) = self.state.url.clone() {
                cx.emit(BrowserEvent::Navigated { url });
            }
            self.show_url(cx);
        }
        if previous.title != self.state.title {
            cx.emit(BrowserEvent::TitleChanged);
        }
        // A same-document navigation commits without a load to finish.
        let finished = change == page::PageChange::Finished || !self.state.loading;
        if finished && self.load_state == LoadState::Loading {
            self.load_state = LoadState::Loaded;
            cx.emit(BrowserEvent::LoadFinished);
        }
        cx.notify();
    }

    /// Puts the committed URL in the address bar, or the failed one under an error page.
    fn show_url(&mut self, cx: &mut Context<Self>) {
        let url = self
            .error
            .as_ref()
            .map(|error| &error.url)
            .or(self.state.url.as_ref())
            .map(Url::to_string)
            .unwrap_or_default();
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
        let loading = self.load_state == LoadState::Loading;
        // `thread.browser.back`, `.next` and `.reload`; a disabled control draws its glyph at
        // 40 % and takes no clicks, as the titlebar's history controls do.
        let nav = |id: &'static str,
                   glyph: gpui::Svg,
                   label: &'static str,
                   enabled: bool,
                   run: fn(&mut Self, &mut Context<Self>)| {
            if enabled {
                tooltip(
                    icon_button(id, glyph, t)
                        .on_click(cx.listener(move |tab, _, _, cx| run(tab, cx))),
                    label,
                    None,
                    Side::Bottom,
                )
                .into_any_element()
            } else {
                div()
                    .id(id)
                    .flex_none()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph.opacity(0.4))
                    .into_any_element()
            }
        };
        let back = icon("arrow-left", 16., t.text_tertiary);
        // Next is the Back glyph turned half a turn.
        let forward = icon("arrow-left", 16., t.text_tertiary)
            .with_transformation(Transformation::rotate(radians(std::f32::consts::PI)));
        // While a document loads the reload control shows a spinner (`aria-busy`).
        let reload = if loading {
            icon("sidebar-cmd-spinner", 16., t.text_tertiary)
        } else {
            icon("diff-refresh", 16., t.text_tertiary)
        };
        let reload = if loading {
            reload
                .with_animation(
                    "browser-reload-spinner",
                    Animation::new(Duration::from_secs(1)).repeat(),
                    |icon, progress| {
                        icon.with_transformation(Transformation::rotate(radians(
                            progress * std::f32::consts::TAU,
                        )))
                    },
                )
                .into_any_element()
        } else {
            reload.into_any_element()
        };
        let toolbar = div()
            .relative()
            .flex_none()
            .h(px(40.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(1.))
                    .child(nav(
                        "browser-back",
                        back,
                        "Back",
                        self.can_go_back(),
                        Self::go_back,
                    ))
                    .child(nav(
                        "browser-forward",
                        forward,
                        "Next",
                        self.can_go_forward(),
                        Self::go_forward,
                    ))
                    .child(tooltip(
                        icon_button("browser-reload", div().flex().child(reload), t)
                            .on_click(cx.listener(|tab, _, _, cx| tab.reload(false, cx))),
                        "Reload page",
                        None,
                        Side::Bottom,
                    )),
            )
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
            )
            .child(self.annotate_button(cx))
            .when(loading, |d| {
                // The 2 px load indicator along the toolbar's bottom edge (`animate-pulse`).
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(2.))
                        .bg(t.info_solid.opacity(0.35))
                        .with_animation(
                            "browser-load-indicator",
                            Animation::new(Duration::from_secs(2)).repeat(),
                            |bar, progress| {
                                let wave = (progress * std::f32::consts::TAU).cos();
                                bar.opacity(0.75 + 0.25 * wave)
                            },
                        ),
                )
            });
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
                            if !tab.hidden && tab.error.is_none() {
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
                        .child(text(
                            "Enter a URL to open a page",
                            14.,
                            20.,
                            400.,
                            t.text_secondary,
                        )),
                )
            })
            .when_some(self.error.as_ref(), |d, error| {
                d.child(self.error_page(error, cx))
            });
        div()
            .key_context("BrowserTab")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(page)
            .when(self.annotating, |d| d.child(self.annotation_toolbar(cx)))
    }
}

impl BrowserTab {
    /// `thread.browser.commentMode`: the Annotate control right of the address bar. Active,
    /// it reads `Annotating`; without a page it is disabled.
    fn annotate_button(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let t = cx.theme();
        let glyph = |color| icon("diff-comment-add", 16., color);
        if self.annotating {
            return div()
                .id("browser-annotate")
                .flex_none()
                .h(px(28.))
                .px(px(8.))
                .gap(px(4.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label("Annotate this page")
                .bg(t.background_accent)
                .text_color(t.text_accent)
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(glyph(t.text_accent))
                .child("Annotating")
                .on_click(cx.listener(|tab, _, _, cx| tab.toggle_annotation_mode(cx)))
                .into_any_element();
        }
        if !self.can_annotate() {
            return div()
                .id("browser-annotate")
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .child(glyph(t.text_tertiary).opacity(0.4))
                .into_any_element();
        }
        tooltip(
            icon_button("browser-annotate", glyph(t.text_tertiary), t)
                .aria_label("Annotate this page")
                .on_click(cx.listener(|tab, _, _, cx| tab.toggle_annotation_mode(cx))),
            "Point at items to ask about or change",
            Some("⌘⇧."),
            Side::Bottom,
        )
        .into_any_element()
    }

    /// `thread.browser.tweaks.*`: the annotation toolbar under the page while annotating.
    /// Exit keeps the pending annotations; Discard asks first; Send sends them as a turn.
    // shortcut: a bar under the page, not the original's floating one over it, because the
    // native page draws over every gpui element; float it once the page layer (#836) lands.
    fn annotation_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let count = self.pending_annotations;
        let action = |id: &'static str,
                      glyph: &'static str,
                      label: &'static str,
                      run: fn(&mut Self, &mut Context<Self>)| {
            tooltip(
                icon_button(id, icon(glyph, 16., t.text_secondary), t)
                    .aria_label(label)
                    .on_click(cx.listener(move |tab, _, _, cx| run(tab, cx))),
                label,
                None,
                Side::Top,
            )
        };
        let discard = if count > 0 {
            action(
                "browser-annotations-discard",
                "shortcut-trash",
                "Discard annotations",
                |_, cx| cx.emit(BrowserEvent::DiscardAnnotations),
            )
            .into_any_element()
        } else {
            div()
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("shortcut-trash", 16., t.text_secondary).opacity(0.4))
                .into_any_element()
        };
        let status = if count > 0 {
            format!("Annotating · {count}")
        } else {
            "Annotating".to_owned()
        };
        let send_label = if count == 1 {
            "Send 1 pending annotation".to_owned()
        } else {
            format!("Send {count} pending annotations")
        };
        let send = div()
            .id("browser-annotations-send")
            .flex_none()
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(send_label))
            .bg(t.button_primary_solid)
            .child(icon("composer-arrow-up", 16., t.text_inverse))
            .map(|send| {
                if count > 0 {
                    send.cursor_pointer()
                        .hover(|s| s.bg(t.button_primary_solid_hover))
                        .on_click(cx.listener(|tab, _, _, cx| {
                            tab.set_annotating(false, cx);
                            cx.emit(BrowserEvent::SendAnnotations);
                        }))
                } else {
                    send.opacity(0.4)
                }
            });
        div()
            .id("browser-annotation-toolbar")
            .role(gpui::Role::Toolbar)
            .aria_label("Browser annotations")
            .flex_none()
            .h(px(44.))
            .px(px(8.))
            .gap(px(12.))
            .flex()
            .items_center()
            .border_t_1()
            .border_color(t.border_subtle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(action(
                        "browser-annotations-exit",
                        "close",
                        "Exit annotation mode",
                        |tab, cx| tab.set_annotating(false, cx),
                    ))
                    .child(discard),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_center()
                    .child(text(status, 14., 20., 400., t.text).truncate()),
            )
            .child(send)
    }

    /// `browserSidebar.loadError`: the network error page the original loads in place of a
    /// main-frame load that failed, at its narrow (side panel) layout.
    fn error_page(&self, error: &LoadError, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let paragraph = |s: SharedString, color| {
            div()
                .text_size(px(14.))
                .line_height(px(22.))
                .text_color(color)
                .child(s)
        };
        let item = |s: SharedString| {
            div()
                .flex()
                .gap(px(8.))
                .child(paragraph("•".into(), t.text_secondary))
                .child(s)
        };
        let suggestion = |header: &'static str, body: &'static str| {
            div()
                .mt(px(18.))
                .child(
                    paragraph(header.into(), t.text_secondary)
                        .font_weight(gpui::FontWeight::BOLD)
                        .mb(px(4.)),
                )
                .child(paragraph(body.into(), t.text_tertiary))
        };
        div()
            .id("browser-error-page")
            .absolute()
            .size_full()
            .overflow_y_scroll()
            .px(px(24.))
            .pt(px(56.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .child(icon("sidebar-cmd-browser", 26., t.text_secondary).mb(px(24.)))
            .child(
                div()
                    .text_size(px(21.))
                    .line_height(px(25.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .mb(px(8.))
                    .child("This site can't be reached"),
            )
            .child(paragraph(error.summary().into(), t.text_secondary))
            .child(
                div()
                    .mt(px(18.))
                    .text_size(px(14.))
                    .line_height(px(22.))
                    .text_color(t.text_secondary)
                    .child("Try:")
                    .child(item("Checking the connection".into()))
                    .child(
                        item("".into()).child(
                            div()
                                .id("browser-error-details-link")
                                .cursor_pointer()
                                .text_color(t.text_link)
                                .child("Checking the proxy, firewall, and DNS configuration")
                                .on_click(cx.listener(|tab, _, _, cx| {
                                    tab.error_details = !tab.error_details;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(
                div()
                    .mt(px(24.))
                    .text_size(px(11.))
                    .line_height(px(18.))
                    .text_color(t.text_tertiary)
                    .child(error.code),
            )
            .child(
                div().mt(px(30.)).flex().child(
                    div()
                        .id("browser-error-reload")
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .bg(t.text_alpha_5)
                        .hover(|s| s.bg(t.ghost_hover))
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child("Reload")
                        .on_click(cx.listener(|tab, _, _, cx| tab.reload(false, cx))),
                ),
            )
            .when(self.error_details, |d| {
                d.child(
                    div()
                        .mt(px(20.))
                        .child(suggestion(
                            "Check your Internet connection",
                            "Check any cables and restart any routers, modems, or other network \
                             devices you may be using",
                        ))
                        .child(suggestion(
                            "Check your DNS settings",
                            "Contact your network administrator if you are not sure what this \
                             means",
                        ))
                        .child(suggestion(
                            "Allow ChatGPT to access the network in your firewall or security \
                             settings",
                            "If ChatGPT is already listed as an allowed app, try removing it \
                             from the list and adding it again",
                        ))
                        .child(suggestion(
                            "If you use a proxy server",
                            "Open your system network settings and check whether a proxy has \
                             been configured for the active network",
                        )),
                )
            })
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
        assert_eq!(
            resolved("https://openai.com/a?b"),
            Some("https://openai.com/a?b".into())
        );
        assert_eq!(
            resolved("openai.com/codex"),
            Some("https://openai.com/codex".into())
        );
        assert_eq!(
            resolved("localhost:3000/x"),
            Some("http://localhost:3000/x".into())
        );
        assert_eq!(
            resolved("127.0.0.1:8080"),
            Some("http://127.0.0.1:8080/".into())
        );
        assert_eq!(
            resolved("rust borrow checker"),
            Some("https://www.google.com/search?q=rust+borrow+checker".into())
        );
        assert_eq!(
            resolved("codex"),
            Some("https://www.google.com/search?q=codex".into())
        );
        assert_eq!(resolved("   "), None);
    }
}
