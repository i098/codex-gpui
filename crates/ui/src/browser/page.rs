//! The web page behind a [`super::BrowserTab`]: a native WKWebView when the window has an
//! AppKit view, else [`Detached`] navigation state.

use gpui::{Bounds, Pixels, Window};
use url::Url;

use super::{LoadError, PageState};

#[cfg(target_os = "macos")]
mod native;

/// A message from the page; the owner then reads [`Page::state`] again.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PageChange {
    /// The committed URL, title or history position changed.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS page reports it
    Changed,
    /// A document finished loading.
    Finished,
    /// The main-frame load failed.
    Failed(LoadError),
    /// The page asked for a new window (`target=_blank`, `window.open`); the owner opens
    /// the address in a new browser tab.
    #[cfg(target_os = "macos")]
    NewWindow(Url),
    /// A right-click outside editable text and selections, in page (viewport) pixels; the
    /// owner shows the page context menu there.
    #[cfg(target_os = "macos")]
    ContextMenu {
        x: f32,
        y: f32,
        /// The `href` of the link under the pointer.
        link: Option<Url>,
    },
}

pub(super) enum Page {
    #[cfg(target_os = "macos")]
    Native(native::WebPage),
    Detached(Detached),
}

impl Page {
    pub fn new(window: &Window, changed: async_channel::Sender<PageChange>) -> Self {
        #[cfg(target_os = "macos")]
        if let Some(page) = native::WebPage::new(window, changed.clone()) {
            return Self::Native(page);
        }
        let _ = window;
        Self::Detached(Detached {
            changed,
            history: Vec::new(),
            position: 0,
        })
    }

    pub fn load(&mut self, url: &Url) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.load(url),
            Self::Detached(page) => page.load(url),
        }
    }

    pub fn back(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.back(),
            Self::Detached(page) => page.step(-1),
        }
    }

    pub fn forward(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.forward(),
            Self::Detached(page) => page.step(1),
        }
    }

    /// `hard` reloads from the origin, past the cache.
    pub fn reload(&mut self, hard: bool) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.reload(hard),
            Self::Detached(page) => {
                let _ = hard;
                page.finish();
            }
        }
    }

    pub fn state(&self) -> PageState {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(page) => page.state(),
            Self::Detached(page) => page.state(),
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
    changed: async_channel::Sender<PageChange>,
    /// Back-forward list; the current entry is `history[position - 1]`.
    history: Vec<Url>,
    position: usize,
}

impl Detached {
    fn load(&mut self, url: &Url) {
        // `.invalid` never resolves (RFC 2606): the load fails as a real page's would.
        if url
            .host_str()
            .is_some_and(|host| host == "invalid" || host.ends_with(".invalid"))
        {
            let _ = self.changed.try_send(PageChange::Failed(LoadError {
                url: url.clone(),
                code: "ERR_NAME_NOT_RESOLVED",
            }));
            return;
        }
        self.history.truncate(self.position);
        self.history.push(url.clone());
        self.position = self.history.len();
        self.finish();
    }

    fn step(&mut self, delta: isize) {
        if let Some(position) = self.position.checked_add_signed(delta)
            && (1..=self.history.len()).contains(&position)
        {
            self.position = position;
            self.finish();
        }
    }

    fn finish(&self) {
        let _ = self.changed.try_send(PageChange::Finished);
    }

    fn state(&self) -> PageState {
        PageState {
            url: self
                .position
                .checked_sub(1)
                .and_then(|i| self.history.get(i))
                .cloned(),
            title: String::new(),
            can_go_back: self.position > 1,
            can_go_forward: self.position < self.history.len(),
            loading: false,
        }
    }
}
