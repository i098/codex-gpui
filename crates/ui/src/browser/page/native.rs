//! The page as a WKWebView child of the gpui window's AppKit view.

use gpui::{Bounds, Pixels, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{MainThreadMarker, NSError, NSString, ns_string};
use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView};
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{PageLoadEvent, Rect, WebView, WebViewBuilder, WebViewExtMacOS as _};

use super::{LoadError, PageChange, PageState};

/// Posts a right-click to the host instead of WebKit's menu. Editable text and selections
/// keep WebKit's own menu (cut, copy, paste, spelling). Runs in every frame; a frame's
/// coordinates are not the page's, so only the main frame reports.
const CONTEXT_MENU_SCRIPT: &str = r#"
if (window === window.top) {
  addEventListener("contextmenu", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const editable = target?.closest("input, textarea, [contenteditable=''], [contenteditable='true']");
    if (event.defaultPrevented || editable || String(getSelection()).trim()) return;
    event.preventDefault();
    const link = target?.closest("a[href]")?.href ?? null;
    window.ipc.postMessage(JSON.stringify({ x: event.clientX, y: event.clientY, link }));
  });
}
"#;

/// The page's `CONTEXT_MENU_SCRIPT` message; `None` for any other body.
fn context_menu(body: &str) -> Option<PageChange> {
    #[derive(serde::Deserialize)]
    struct Click {
        x: f32,
        y: f32,
        link: Option<String>,
    }
    let click: Click = serde_json::from_str(body).ok()?;
    Some(PageChange::ContextMenu {
        x: click.x,
        y: click.y,
        link: click
            .link
            .and_then(|link| Url::parse(&link).ok())
            .filter(|link| matches!(link.scheme(), "http" | "https")),
    })
}

pub(in crate::browser) struct WebPage {
    view: WebView,
    /// The view's navigation delegate: wry holds the one it wraps.
    _failures: Retained<FailureDelegate>,
}

impl WebPage {
    /// `None` when the window has no AppKit view (gpui's headless renderer).
    pub fn new(window: &Window, changed: async_channel::Sender<PageChange>) -> Option<Self> {
        let on_title = changed.clone();
        let on_load = changed.clone();
        let on_menu = changed.clone();
        let view = WebViewBuilder::new()
            .with_visible(false)
            // `Started` fires once the navigation commits: the URL has changed.
            .with_on_page_load_handler(move |event, _| {
                let _ = on_load.try_send(match event {
                    PageLoadEvent::Started => PageChange::Changed,
                    PageLoadEvent::Finished => PageChange::Finished,
                });
            })
            .with_document_title_changed_handler(move |_| {
                let _ = on_title.try_send(PageChange::Changed);
            })
            .with_initialization_script(CONTEXT_MENU_SCRIPT)
            .with_ipc_handler(move |request| {
                if let Some(change) = context_menu(request.body()) {
                    let _ = on_menu.try_send(change);
                }
            })
            .build_as_child(window)
            .ok()?;
        let webview = view.webview();
        // SAFETY: main-thread reads and writes of the view's delegate, which wry keeps alive.
        let inner = unsafe { webview.navigationDelegate() }?;
        let failures = FailureDelegate::new(MainThreadMarker::new()?, inner, changed);
        unsafe { webview.setNavigationDelegate(Some(ProtocolObject::from_ref(&*failures))) };
        Some(Self {
            view,
            _failures: failures,
        })
    }

    pub fn load(&self, url: &Url) {
        // A URL the view rejects leaves the previous page, as a failed navigation does.
        let _ = self.view.load_url(url.as_str());
    }

    pub fn back(&self) {
        // SAFETY: a responder action on the main thread, which owns the view.
        unsafe { self.view.webview().goBack_(None) };
    }

    pub fn forward(&self) {
        // SAFETY: as in `back`.
        unsafe { self.view.webview().goForward_(None) };
    }

    pub fn reload(&self, hard: bool) {
        let webview = self.view.webview();
        // SAFETY: as in `back`.
        unsafe {
            if hard {
                webview.reloadFromOrigin_(None);
            } else {
                webview.reload_(None);
            }
        }
    }

    pub fn state(&self) -> PageState {
        let webview = self.view.webview();
        // SAFETY: plain property reads on the main thread, which owns the view.
        let (title, can_go_back, can_go_forward, loading) = unsafe {
            (
                webview
                    .title()
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
                webview.canGoBack(),
                webview.canGoForward(),
                webview.isLoading(),
            )
        };
        PageState {
            url: self.view.url().ok().and_then(|url| Url::parse(&url).ok()),
            title,
            can_go_back,
            can_go_forward,
            loading,
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

/// The Chromium error name for an `NSURLErrorDomain` code; `None` for a load a newer
/// navigation cancelled, which shows no error page.
fn chromium_error(code: isize) -> Option<&'static str> {
    Some(match code {
        -999 => return None,
        -1003 | -1006 => "ERR_NAME_NOT_RESOLVED",
        -1009 => "ERR_INTERNET_DISCONNECTED",
        -1004 => "ERR_CONNECTION_REFUSED",
        -1001 => "ERR_TIMED_OUT",
        -1201 | -1204 => "ERR_CERT_DATE_INVALID",
        -1202 | -1203 => "ERR_CERT_AUTHORITY_INVALID",
        -1200 => "ERR_SSL_PROTOCOL_ERROR",
        _ => "ERR_FAILED",
    })
}

struct Failures {
    /// wry's delegate: every method but the two failures forwards to it.
    inner: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    changed: async_channel::Sender<PageChange>,
}

define_class!(
    // wry's navigation delegate has no failure callbacks; this one reports them and
    // forwards the rest.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Failures]
    struct FailureDelegate;

    unsafe impl NSObjectProtocol for FailureDelegate {}

    unsafe impl WKNavigationDelegate for FailureDelegate {
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(
            &self,
            web_view: &WKWebView,
            _: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.failed(web_view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, web_view: &WKWebView, _: Option<&WKNavigation>, error: &NSError) {
            self.failed(web_view, error);
        }
    }

    impl FailureDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.ivars().inner.respondsToSelector(selector)
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target(&self, _: Sel) -> *mut AnyObject {
            Retained::as_ptr(&self.ivars().inner)
                .cast::<AnyObject>()
                .cast_mut()
        }
    }
);

impl FailureDelegate {
    fn new(
        mtm: MainThreadMarker,
        inner: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
        changed: async_channel::Sender<PageChange>,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(Failures { inner, changed });
        unsafe { msg_send![super(object), init] }
    }

    fn failed(&self, web_view: &WKWebView, error: &NSError) {
        if error.domain().to_string() != "NSURLErrorDomain" {
            // WebKit's own errors (a policy change, a download) are not failed pages.
            return;
        }
        let Some(code) = chromium_error(error.code()) else {
            return;
        };
        let failed = error
            .userInfo()
            .objectForKey(ns_string!("NSErrorFailingURLStringKey"))
            .and_then(|url| url.downcast::<NSString>().ok())
            // SAFETY: a property read on the main thread, which owns the view.
            .or_else(|| unsafe { web_view.URL().and_then(|url| url.absoluteString()) })
            .map(|url| url.to_string());
        if let Some(url) = failed.and_then(|url| Url::parse(&url).ok()) {
            let _ = self
                .ivars()
                .changed
                .try_send(PageChange::Failed(LoadError { url, code }));
        }
    }
}
