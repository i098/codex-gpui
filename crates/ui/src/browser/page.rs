//! The web page behind a [`super::BrowserTab`]: a native WKWebView when the window has an
//! AppKit view, else [`Detached`] navigation state.

use gpui::{Bounds, Pixels, Window};
use url::Url;

use super::PageState;

#[cfg(target_os = "macos")]
mod native;

pub(super) enum Page {
    #[cfg(target_os = "macos")]
    Native(native::WebPage),
    Detached(Detached),
}

impl Page {
    /// `changed` receives a message after every change of the page's [`PageState`]: `true`
    /// when a document finished loading.
    pub fn new(window: &Window, changed: async_channel::Sender<bool>) -> Self {
        #[cfg(target_os = "macos")]
        if let Some(page) = native::WebPage::new(window, changed.clone()) {
            return Self::Native(page);
        }
        let _ = window;
        Self::Detached(Detached {
            changed,
            state: PageState::default(),
        })
    }

    pub fn load(&mut self, url: &Url) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.load(url),
            Self::Detached(page) => page.load(url),
        }
    }

    pub fn state(&self) -> PageState {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.state(),
            Self::Detached(page) => page.state.clone(),
        }
    }

    /// Lays the page over `bounds` (window coordinates) and shows it.
    pub fn place(&self, bounds: Bounds<Pixels>) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.place(bounds),
            Self::Detached(_) => {
                let _ = bounds;
            }
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.set_hidden(hidden),
            Self::Detached(_) => {
                let _ = hidden;
            }
        }
    }
}

/// A page with no native view: gpui's headless renderer, and Linux, which has no WebKit
/// view to embed. Each load commits at once and nothing is drawn.
// shortcut: no page renders off macOS; embed WebKitGTK when a Linux build ships.
pub(super) struct Detached {
    changed: async_channel::Sender<bool>,
    state: PageState,
}

impl Detached {
    fn load(&mut self, url: &Url) {
        self.state = PageState {
            url: Some(url.clone()),
            title: String::new(),
            can_go_back: self.state.url.is_some(),
            can_go_forward: false,
        };
        let _ = self.changed.try_send(true);
    }
}
