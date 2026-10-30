//! The page as a WKWebView child of the gpui window's AppKit view.

use gpui::{Bounds, Pixels, Window};
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{PageLoadEvent, Rect, WebView, WebViewBuilder, WebViewExtMacOS as _};

use super::PageState;

pub(in crate::browser) struct WebPage {
    view: WebView,
}

impl WebPage {
    /// `None` when the window has no AppKit view (gpui's headless renderer).
    pub fn new(window: &Window, changed: async_channel::Sender<bool>) -> Option<Self> {
        let on_title = changed.clone();
        let view = WebViewBuilder::new()
            .with_visible(false)
            // `Started` fires once the navigation commits: the URL has changed.
            .with_on_page_load_handler(move |event, _| {
                let _ = changed.try_send(matches!(event, PageLoadEvent::Finished));
            })
            .with_document_title_changed_handler(move |_| {
                let _ = on_title.try_send(false);
            })
            .build_as_child(window)
            .ok()?;
        Some(Self { view })
    }

    pub fn load(&self, url: &Url) {
        // A URL the view rejects leaves the previous page, as a failed navigation does.
        let _ = self.view.load_url(url.as_str());
    }

    pub fn state(&self) -> PageState {
        let webview = self.view.webview();
        // SAFETY: plain property reads on the main thread, which owns the view.
        let (title, can_go_back, can_go_forward) = unsafe {
            (
                webview.title().map(|title| title.to_string()).unwrap_or_default(),
                webview.canGoBack(),
                webview.canGoForward(),
            )
        };
        PageState {
            url: self.view.url().ok().and_then(|url| Url::parse(&url).ok()),
            title,
            can_go_back,
            can_go_forward,
        }
    }

    pub fn place(&self, bounds: Bounds<Pixels>) {
        let rect = Rect {
            position: LogicalPosition::new(
                f32::from(bounds.origin.x) as f64,
                f32::from(bounds.origin.y) as f64,
            )
            .into(),
            size: LogicalSize::new(
                f32::from(bounds.size.width) as f64,
                f32::from(bounds.size.height) as f64,
            )
            .into(),
        };
        let _ = self.view.set_bounds(rect);
        let _ = self.view.set_visible(true);
    }

    pub fn set_hidden(&self, hidden: bool) {
        let _ = self.view.set_visible(!hidden);
    }
}
