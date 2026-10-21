use super::Kind;
use gpui::{App, Bounds, Pixels, Window};

pub(super) fn work_area(owner: Option<Bounds<Pixels>>, cx: &App) -> Bounds<Pixels> {
    let displays = cx.displays();
    let display = owner
        .and_then(|owner| {
            displays
                .iter()
                .find(|display| display.bounds().intersects(&owner))
        })
        .or_else(|| displays.first());
    let bounds = display.map(|display| display.bounds()).unwrap_or_else(|| {
        Bounds::new(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(1280.), gpui::px(832.)),
        )
    });
    #[cfg(target_os = "macos")]
    {
        native::work_area(bounds).unwrap_or(bounds)
    }
    #[cfg(not(target_os = "macos"))]
    {
        bounds
    }
}

pub(super) fn configure(window: &Window, kind: Kind) {
    #[cfg(target_os = "macos")]
    native::configure(window, kind);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, kind);
}

pub(super) fn hide(window: &Window) -> bool {
    #[cfg(target_os = "macos")]
    return native::with_window(window, |window| unsafe {
        let _: () =
            objc2::msg_send![window, orderOut: std::ptr::null::<objc2::runtime::AnyObject>()];
    })
    .is_some();
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        false
    }
}

pub(super) fn show(window: &Window, focus: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if native::with_window(window, |window| unsafe {
        let minimized: bool = objc2::msg_send![window, isMiniaturized];
        if minimized {
            let _: () = objc2::msg_send![window, deminiaturize: std::ptr::null::<objc2::runtime::AnyObject>()];
        }
        if focus {
            let _: () = objc2::msg_send![window, makeKeyAndOrderFront: std::ptr::null::<objc2::runtime::AnyObject>()];
        } else {
            let _: () = objc2::msg_send![window, orderFront: std::ptr::null::<objc2::runtime::AnyObject>()];
        }
    }).is_some() { return Ok(()); }
    #[cfg(not(target_os = "macos"))]
    if focus {
        window.activate_window();
        return Ok(());
    }
    anyhow::bail!("non-activating native presentation requires AppKit")
}

pub(super) fn set_bounds(window: &Window, bounds: Bounds<Pixels>) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return native::set_bounds(window, bounds);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, bounds);
        anyhow::bail!("native window placement requires AppKit")
    }
}

pub(super) fn set_pointer_interactive(window: &Window, interactive: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if native::with_window(window, |window| unsafe {
        let _: () = objc2::msg_send![window, setIgnoresMouseEvents: !interactive];
    })
    .is_some()
    {
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, interactive);
    anyhow::bail!("native pointer pass-through requires AppKit")
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::{class, msg_send, runtime::AnyObject, sel};
    use objc2_foundation::NSRect;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub(super) fn with_window<T>(
        window: &Window,
        action: impl FnOnce(&AnyObject) -> T,
    ) -> Option<T> {
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        // GPUI owns the view and its window for the duration of this main-thread call.
        unsafe {
            let view = handle.ns_view.cast::<AnyObject>().as_ref();
            let native: *mut AnyObject = msg_send![view, window];
            native.as_ref().map(action)
        }
    }

    pub(super) fn configure(window: &Window, kind: Kind) {
        with_window(window, |native| unsafe {
            if kind.panel() {
                let mask: usize = msg_send![native, styleMask];
                if kind != Kind::Hud {
                    let mask = match kind {
                        Kind::HotkeyThread => (mask & !1usize) | 8,
                        Kind::HotkeyHome => mask & !9usize,
                        _ => mask & !1usize,
                    };
                    let _: () = msg_send![native, setStyleMask: mask];
                    if let Some(minimum) = kind.minimum() {
                        let size = objc2_foundation::NSSize::new(
                            f32::from(minimum.width) as f64,
                            f32::from(minimum.height) as f64,
                        );
                        let _: () = msg_send![native, setContentMinSize: size];
                    }
                }
                let _: () = msg_send![native, setHasShadow: matches!(kind, Kind::Hud | Kind::HotkeyHome | Kind::HotkeyThread)];
                let level: isize = if matches!(
                    kind,
                    Kind::HotkeyHome | Kind::HotkeyThread | Kind::BrowserCommentPopup
                ) {
                    0
                } else {
                    3
                };
                let _: () = msg_send![native, setLevel: level];
            }
            if matches!(
                kind,
                Kind::Spotlight | Kind::AvatarOverlay | Kind::ScreenOverlay | Kind::GlobalDictation
            ) {
                let _: () = msg_send![native, setCollectionBehavior: (1usize << 0) | (1usize << 8)];
            }
            if kind.passive() {
                let supported: bool =
                    msg_send![native, respondsToSelector: sel!(_setPreventsActivation:)];
                if supported {
                    let _: () = msg_send![native, _setPreventsActivation: true];
                }
                let _: () = msg_send![native, setIgnoresMouseEvents: true];
            }
        });
    }

    pub(super) fn work_area(bounds: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        // AppKit has a bottom-left origin; GPUI uses the primary display's top-left origin.
        unsafe {
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            let screens = screens.as_ref()?;
            let count: usize = msg_send![screens, count];
            if count == 0 {
                return None;
            }
            let first: &AnyObject = msg_send![screens, objectAtIndex: 0usize];
            let first_frame: NSRect = msg_send![first, frame];
            let top = first_frame.origin.y + first_frame.size.height;
            for index in 0..count {
                let screen: &AnyObject = msg_send![screens, objectAtIndex: index];
                let frame: NSRect = msg_send![screen, frame];
                let x = frame.origin.x as f32;
                let y = (top - frame.origin.y - frame.size.height) as f32;
                if (f32::from(bounds.origin.x) - x).abs() > 1.
                    || (f32::from(bounds.origin.y) - y).abs() > 1.
                {
                    continue;
                }
                let visible: NSRect = msg_send![screen, visibleFrame];
                return Some(Bounds::new(
                    gpui::point(
                        gpui::px(visible.origin.x as f32),
                        gpui::px((top - visible.origin.y - visible.size.height) as f32),
                    ),
                    gpui::size(
                        gpui::px(visible.size.width as f32),
                        gpui::px(visible.size.height as f32),
                    ),
                ));
            }
            None
        }
    }

    pub(super) fn set_bounds(window: &Window, bounds: Bounds<Pixels>) -> anyhow::Result<()> {
        with_window(window, |window| unsafe {
            let screens: &AnyObject = msg_send![class!(NSScreen), screens];
            let first: &AnyObject = msg_send![screens, objectAtIndex: 0usize];
            let screen: NSRect = msg_send![first, frame];
            let frame = NSRect::new(
                objc2_foundation::NSPoint::new(
                    f32::from(bounds.origin.x) as f64,
                    screen.origin.y + screen.size.height - f32::from(bounds.bottom()) as f64,
                ),
                objc2_foundation::NSSize::new(
                    f32::from(bounds.size.width) as f64,
                    f32::from(bounds.size.height) as f64,
                ),
            );
            let _: () = msg_send![window, setFrame: frame, display: false];
        })
        .ok_or_else(|| anyhow::anyhow!("the native window is unavailable"))
    }
}
