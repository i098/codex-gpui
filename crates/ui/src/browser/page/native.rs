//! The page as a WKWebView child of the gpui window's AppKit view.

use std::{cell::RefCell, collections::HashMap, path::PathBuf};

use anyhow::{Context as _, Result, ensure};
use block2::{DynBlock, RcBlock};
use gpui::{Bounds, Pixels, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_foundation::{
    MainThreadMarker, NSData, NSDictionary, NSError, NSFileManager, NSHTTPURLResponse, NSPoint,
    NSRect, NSSearchPathDirectory, NSSearchPathDomainMask, NSSize, NSString, NSURL, NSURLResponse,
    ns_string,
};
use objc2_web_kit::{
    WKContentWorld, WKDownload, WKDownloadDelegate, WKNavigation, WKNavigationAction,
    WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKScriptMessage, WKScriptMessageHandler, WKSnapshotConfiguration,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
};
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder, WebViewExtMacOS as _};

use super::download::Destination;
use super::{LoadError, PageChange, PageState};
use crate::screens::settings::browser_settings::Download;

pub(in crate::browser) struct WidgetPage {
    view: WebView,
    clip: Retained<objc2_app_kit::NSView>,
}

impl WidgetPage {
    pub(in crate::browser) fn new(
        window: &Window,
        html: String,
        key: String,
        messages: async_channel::Sender<serde_json::Value>,
    ) -> Option<Self> {
        use objc2_app_kit::NSView;
        let mtm = MainThreadMarker::new()?;
        let view = WebViewBuilder::new()
            .with_incognito(true)
            .with_visible(false)
            .with_navigation_handler(|url| matches!(url.as_str(), "about:blank" | "about:srcdoc"))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_ipc_handler(move |request| {
                let body = request.into_body();
                if let Some(payload) = body.strip_prefix(&key)
                    && payload.len() <= 64 * 1024
                    && let Ok(message) = serde_json::from_str(payload)
                {
                    let _ = messages.try_send(message);
                }
            })
            .with_html(html)
            .build_as_child(window)
            .ok()?;
        let webview = view.webview();
        // WebKit cancels file selection on macOS when no UI delegate implements it.
        unsafe { webview.setUIDelegate(None) };
        // SAFETY: This AppKit view was created on the main thread above.
        let parent = unsafe { webview.superview() }?;
        let clip = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        clip.setClipsToBounds(true);
        parent.addSubview(&clip);
        webview.removeFromSuperview();
        clip.addSubview(&webview);
        Some(Self { view, clip })
    }

    pub(in crate::browser) fn post(&self, message: &serde_json::Value) {
        let _ = self
            .view
            .evaluate_script(&format!("sendWidgetMessage({message})"));
    }

    pub(in crate::browser) fn place(&self, bounds: Bounds<Pixels>, visible: Bounds<Pixels>) {
        // SAFETY: The clip is only accessed on the AppKit thread.
        let Some(parent) = (unsafe { self.clip.superview() }) else {
            return;
        };
        let number = |pixel: Pixels| f64::from(f32::from(pixel));
        let y = if parent.isFlipped() {
            number(visible.top())
        } else {
            parent.bounds().size.height - number(visible.bottom())
        };
        self.clip.setFrame(NSRect::new(
            NSPoint::new(number(visible.left()), y),
            NSSize::new(number(visible.size.width), number(visible.size.height)),
        ));
        let _ = self.view.set_bounds(Rect {
            position: LogicalPosition::new(
                number(bounds.left() - visible.left()),
                number(bounds.top() - visible.top()),
            )
            .into(),
            size: LogicalSize::new(number(bounds.size.width), number(bounds.size.height)).into(),
        });
        let _ = self.view.set_visible(true);
    }

    pub(in crate::browser) fn hide(&self) {
        let _ = self.view.set_visible(false);
    }
}

impl Drop for WidgetPage {
    fn drop(&mut self) {
        self.clip.removeFromSuperview();
    }
}

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
    window.ipc.postMessage(JSON.stringify({ type: "context-menu", x: event.clientX, y: event.clientY, link }));
  });
}
"#;

/// The page's `CONTEXT_MENU_SCRIPT` message; `None` for any other body.
fn context_menu(body: &str) -> Option<PageChange> {
    #[derive(serde::Deserialize)]
    #[serde(tag = "type", rename_all = "kebab-case")]
    enum Message {
        ContextMenu {
            x: f32,
            y: f32,
            link: Option<String>,
        },
    }
    let Message::ContextMenu { x, y, link } = serde_json::from_str(body).ok()?;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(PageChange::ContextMenu {
        x,
        y,
        link: link
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
        let on_window = changed.clone();
        let view = WebViewBuilder::new()
            .with_visible(false)
            // The context menu's `Inspect` opens WebKit's Web Inspector.
            .with_devtools(true)
            // `Started` fires once the navigation commits: the URL has changed.
            .with_on_page_load_handler(move |event, _| {
                let _ = on_load.try_send(match event {
                    PageLoadEvent::Started => PageChange::DocumentStarted,
                    PageLoadEvent::Finished => PageChange::Finished,
                });
            })
            .with_document_title_changed_handler(move |_| {
                let _ = on_title.try_send(PageChange::Changed);
            })
            // The page gets no window of its own: its address opens in a new tab.
            .with_new_window_req_handler(move |url, _| {
                if let Some(url) = Url::parse(&url)
                    .ok()
                    .filter(|url| matches!(url.scheme(), "http" | "https"))
                {
                    let _ = on_window.try_send(PageChange::NewWindow(url));
                }
                NewWindowResponse::Deny
            })
            .with_initialization_script(CONTEXT_MENU_SCRIPT)
            .with_ipc_handler(move |request| {
                let body = request.into_body();
                // Page writers can neither grow the queue without bound nor block navigation.
                if body.len() <= 64 * 1024 && on_menu.len() < 32 {
                    let change = context_menu(&body).unwrap_or(PageChange::ScriptMessage(body));
                    let _ = on_menu.try_send(change);
                }
            })
            .build_as_child(window)
            .ok()?;
        let webview = view.webview();
        // SAFETY: main-thread reads and writes of the view's delegate, which wry keeps alive.
        let inner = unsafe { webview.navigationDelegate() }?;
        let mtm = MainThreadMarker::new()?;
        let handler = AnnotationHandler::alloc(mtm).set_ivars(changed.clone());
        let handler: Retained<AnnotationHandler> = unsafe { msg_send![super(handler), init] };
        // SAFETY: The controller retains the script and handler in an isolated world on this thread.
        unsafe {
            let world = WKContentWorld::defaultClientWorld(mtm);
            let controller = webview.configuration().userContentController();
            controller.addScriptMessageHandler_contentWorld_name(
                ProtocolObject::from_ref(&*handler),
                &world,
                ns_string!("codexAnnotation"),
            );
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                WKUserScript::alloc(mtm),
                &NSString::from_str(include_str!("../annotation.js")),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
                &world,
            );
            controller.addUserScript(&script);
        }
        let page_script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(include_str!("../site_annotation.js")),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            )
        };
        // SAFETY: The controller retains the main-frame script on the main thread.
        unsafe {
            webview
                .configuration()
                .userContentController()
                .addUserScript(&page_script)
        };
        let failures = FailureDelegate::new(mtm, inner, changed);
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

    pub fn evaluate(&self, script: &str) -> Result<()> {
        self.view
            .evaluate_script(script)
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    pub fn evaluate_in_client_world(
        &self,
        script: &str,
    ) -> Result<async_channel::Receiver<Result<String>>> {
        let mtm = MainThreadMarker::new().context("The browser page needs the main thread")?;
        let (send, receive) = async_channel::bounded(1);
        let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            // SAFETY: WebKit keeps both nullable arguments alive for the completion call.
            let result = unsafe {
                if let Some(error) = error.as_ref() {
                    Err(anyhow::anyhow!("{}", error.localizedDescription()))
                } else {
                    value
                        .as_ref()
                        .and_then(|value| value.downcast_ref::<NSString>())
                        .map(ToString::to_string)
                        .context("No browser tool result")
                }
            };
            let _ = send.try_send(result);
        });
        // SAFETY: The view and isolated world belong to this thread. WebKit copies the block.
        unsafe {
            self.view
                .webview()
                .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(script),
                    None,
                    &WKContentWorld::defaultClientWorld(mtm),
                    Some(&callback),
                );
        }
        Ok(receive)
    }

    pub fn matches_document(&self, expected: &Url) -> bool {
        let view = self.view.webview();
        // SAFETY: The view belongs to the main thread.
        unsafe {
            !view.isLoading()
                && view
                    .URL()
                    .and_then(|url| url.absoluteString())
                    .is_some_and(|url| *url == *NSString::from_str(expected.as_str()))
        }
    }

    pub fn add_user_script(&self, script: &str) -> Result<()> {
        let mtm = MainThreadMarker::new().context("The browser page needs the main thread")?;
        // SAFETY: WebKit owns the controller on the main thread and retains the user script.
        unsafe {
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(script),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            );
            self.view
                .webview()
                .configuration()
                .userContentController()
                .addUserScript(&script);
        }
        Ok(())
    }

    pub fn site_annotation_state(&self, permission_required: Option<bool>, active: bool) {
        let enabled = permission_required.is_some();
        let permission_required = permission_required.unwrap_or(true);
        let _ = self.evaluate(&format!(
            "dispatchEvent(new CustomEvent('browser-sidebar-site-annotation-api-enabled-changed',\
             {{ detail: {{ enabled: {enabled}, permissionRequired: {permission_required}, active: {active} }} }}))"
        ));
    }

    pub fn annotation_mode(&self, generation: u32, mode: u8) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // SAFETY: The view and isolated content world belong to the main thread.
        unsafe {
            self.view
                .webview()
                .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(&format!(
                        "globalThis.codexAnnotation?.({generation},{mode})"
                    )),
                    None,
                    &WKContentWorld::defaultClientWorld(mtm),
                    None,
                );
        }
    }

    pub fn snapshot(
        &self,
        region: Bounds<Pixels>,
    ) -> Result<async_channel::Receiver<Result<Vec<u8>>>> {
        let view = self.view.webview();
        let x = f64::from(f32::from(region.origin.x));
        let y = f64::from(f32::from(region.origin.y));
        let width = f64::from(f32::from(region.size.width));
        let height = f64::from(f32::from(region.size.height));
        let viewport = view.bounds().size;
        ensure!(
            x >= 0.
                && y >= 0.
                && width > 0.
                && height > 0.
                && x + width <= viewport.width
                && y + height <= viewport.height,
            "The snapshot rectangle must be inside the browser viewport"
        );
        let mtm = MainThreadMarker::new().context("The browser page needs the main thread")?;
        let (send, receive) = async_channel::bounded(1);
        let callback = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            // SAFETY: WebKit keeps both nullable arguments alive for the completion call.
            let png = unsafe {
                if let Some(error) = error.as_ref() {
                    Err(anyhow::anyhow!("{}", error.localizedDescription()))
                } else {
                    image
                        .as_ref()
                        .and_then(snapshot_png)
                        .context("No browser snapshot PNG")
                }
            };
            let _ = send.try_send(png);
        });
        // SAFETY: This view and the configuration are on the main thread. WebKit copies the block.
        unsafe {
            let configuration = WKSnapshotConfiguration::new(mtm);
            configuration.setRect(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
            view.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &callback);
        }
        Ok(receive)
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

    /// Opens WebKit's Web Inspector for the page.
    pub fn inspect(&self) {
        self.view.open_devtools();
    }

    pub fn state(&self) -> PageState {
        let webview = self.view.webview();
        // SAFETY: plain property reads on the main thread, which owns the view.
        let (url, title, can_go_back, can_go_forward, loading) = unsafe {
            (
                webview.URL().and_then(|url| url.absoluteString()),
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
            url: url.and_then(|url| Url::parse(&url.to_string()).ok()),
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

fn snapshot_png(image: &NSImage) -> Option<Vec<u8>> {
    // SAFETY: No destination rectangle, graphics context or hints are passed.
    let image =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    // SAFETY: The empty dictionary cannot contain incompatible property values.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    // SAFETY: The retained data stays immutable while its bytes are copied.
    Some(unsafe { png.as_bytes_unchecked() }.to_vec())
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = async_channel::Sender<PageChange>]
    struct AnnotationHandler;

    unsafe impl NSObjectProtocol for AnnotationHandler {}

    unsafe impl WKScriptMessageHandler for AnnotationHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn receive(&self, _: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: WebKit passes a valid message on the main thread.
            let Ok(body) = (unsafe { message.body() }).downcast::<NSString>() else {
                return;
            };
            let body = body.to_string();
            if body.len() > 32 * 1024 || self.ivars().len() >= 32 {
                return;
            }
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct SiteMessage {
                site_request: super::super::SiteAnnotationRequest,
            }
            if let Ok(message) = serde_json::from_str::<SiteMessage>(&body) {
                let _ = self.ivars().try_send(PageChange::SiteAnnotation(message.site_request));
            } else if let Ok(pick) = serde_json::from_str(&body) {
                let _ = self.ivars().try_send(PageChange::Annotation(pick));
            }
        }
    }
);

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
    /// wry's delegate handles all navigation methods not overridden here.
    inner: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    changed: async_channel::Sender<PageChange>,
    downloads: RefCell<HashMap<usize, (Destination, String)>>,
}

define_class!(
    // Report load failures and handle downloads that wry would otherwise cancel.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Failures]
    struct FailureDelegate;

    unsafe impl NSObjectProtocol for FailureDelegate {}

    unsafe impl WKNavigationDelegate for FailureDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn navigation_action(
            &self,
            view: &WKWebView,
            action: &WKNavigationAction,
            decide: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: WebKit calls on the main thread and keeps the action and block alive.
            unsafe {
                if action.respondsToSelector(objc2::sel!(shouldPerformDownload))
                    && action.shouldPerformDownload()
                {
                    decide.call((WKNavigationActionPolicy::Download,));
                } else {
                    self.ivars().inner.webView_decidePolicyForNavigationAction_decisionHandler(
                        view, action, decide,
                    );
                }
            }
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn navigation_response(
            &self,
            _: &WKWebView,
            navigation: &WKNavigationResponse,
            decide: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            // SAFETY: WebKit calls on the main thread and keeps the response and block alive.
            let attachment = unsafe { navigation.response() }
                .downcast::<NSHTTPURLResponse>()
                .ok()
                .and_then(|response| response.valueForHTTPHeaderField(ns_string!("Content-Disposition")))
                .is_some_and(|header| {
                    header.to_string().split(';').next().is_some_and(|value| {
                        value.trim().eq_ignore_ascii_case("attachment")
                    })
                });
            if attachment || !unsafe { navigation.canShowMIMEType() } {
                decide.call((WKNavigationResponsePolicy::Download,));
            } else {
                decide.call((WKNavigationResponsePolicy::Allow,));
            }
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_became_download(&self, _: &WKWebView, _: &WKNavigationAction, download: &WKDownload) {
            self.begin_download(download);
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_became_download(&self, _: &WKWebView, _: &WKNavigationResponse, download: &WKDownload) {
            self.begin_download(download);
        }

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

    unsafe impl WKDownloadDelegate for FailureDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn download_destination(
            &self,
            download: &WKDownload,
            response: &NSURLResponse,
            filename: &NSString,
            decide: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let destination = NSFileManager::defaultManager()
                .URLForDirectory_inDomain_appropriateForURL_create_error(
                    NSSearchPathDirectory::DownloadsDirectory,
                    NSSearchPathDomainMask::UserDomainMask,
                    None,
                    true,
                )
                .ok()
                .and_then(|url| url.path())
                .context("No Downloads folder")
                .and_then(|directory| {
                    Destination::new(&PathBuf::from(directory.to_string()), &filename.to_string())
                        .context("Cannot prepare the Downloads folder")
                });
            let destination = match destination {
                Ok(destination) => destination,
                Err(error) => {
                    eprintln!("codex-gpui: {error:#}");
                    decide.call((std::ptr::null_mut(),));
                    return;
                }
            };
            let source = response.URL()
                .filter(|url| url.scheme().is_some_and(|scheme| matches!(&*scheme.to_string(), "http" | "https")))
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string())
                .unwrap_or_default();
            let path = NSString::from_str(&destination.staging_path().to_string_lossy());
            let url = NSURL::fileURLWithPath_isDirectory(&path, false);
            self.ivars().downloads.borrow_mut().insert(download_id(download), (destination, source));
            decide.call((Retained::as_ptr(&url).cast_mut(),));
        }

        #[unsafe(method(downloadDidFinish:))]
        fn download_finished(&self, download: &WKDownload) {
            let Some((destination, source)) = self.ivars().downloads.borrow_mut().remove(&download_id(download)) else {
                return;
            };
            match destination.finish() {
                Ok(path) => {
                    let _ = self.ivars().changed.try_send(PageChange::Downloaded(Download {
                        path,
                        source,
                        time: chrono::Utc::now().timestamp_millis(),
                    }));
                }
                Err(error) => eprintln!("codex-gpui: cannot save download: {error}"),
            }
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn download_failed(&self, download: &WKDownload, error: &NSError, _: Option<&NSData>) {
            self.ivars().downloads.borrow_mut().remove(&download_id(download));
            eprintln!("codex-gpui: download failed: {}", error.localizedDescription());
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
        let object = Self::alloc(mtm).set_ivars(Failures {
            inner,
            changed,
            downloads: RefCell::default(),
        });
        unsafe { msg_send![super(object), init] }
    }

    fn begin_download(&self, download: &WKDownload) {
        // SAFETY: The tab retains this delegate for the lifetime of its WebKit view.
        unsafe { download.setDelegate(Some(ProtocolObject::from_ref(self))) };
        let _ = self.ivars().changed.try_send(PageChange::Finished);
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

fn download_id(download: &WKDownload) -> usize {
    std::ptr::from_ref(download) as usize
}
