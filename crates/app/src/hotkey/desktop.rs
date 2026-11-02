use anyhow::{Context as _, Result, ensure};
use gpui::{Bounds, Pixels, Window, point, px};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSPoint, NSRect};
use raw_window_handle::RawWindowHandle;
use std::{cell::RefCell, ffi::c_void, ptr, rc::Rc};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: u8) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(location: u32, event: *mut c_void);
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut AnyObject;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}

pub fn clipboard_stamp() -> isize {
    unsafe {
        let pasteboard: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        msg_send![pasteboard, changeCount]
    }
}

pub fn clear_clipboard() {
    unsafe {
        let pasteboard: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let _: isize = msg_send![pasteboard, clearContents];
    }
}

pub fn frontmost_application() -> Option<i32> {
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let application: *mut AnyObject = msg_send![workspace, frontmostApplication];
        if application.is_null() {
            None
        } else {
            Some(msg_send![application, processIdentifier])
        }
    }
}

/// The window number of the frontmost application's front window, for `screencapture -l`.
pub fn frontmost_window() -> Option<u32> {
    let pid = frontmost_application()?;
    unsafe {
        // On-screen only, desktop excluded; the list runs front to back.
        let windows = CGWindowListCopyWindowInfo(1 | 16, 0);
        if windows.is_null() {
            return None;
        }
        let number = |info: *mut AnyObject, key: &str| -> Option<i64> {
            let key = objc2_foundation::NSString::from_str(key);
            let value: *mut AnyObject = msg_send![info, objectForKey: &*key];
            (!value.is_null()).then(|| msg_send![value, longLongValue])
        };
        let count: usize = msg_send![windows, count];
        let found = (0..count).find_map(|index| {
            let info: *mut AnyObject = msg_send![windows, objectAtIndex: index];
            (number(info, "kCGWindowOwnerPID") == Some(i64::from(pid))
                && number(info, "kCGWindowLayer") == Some(0))
            .then(|| number(info, "kCGWindowNumber"))
            .flatten()
        });
        CFRelease(windows.cast());
        found.and_then(|number| u32::try_from(number).ok())
    }
}

struct Event(*mut c_void);
impl Drop for Event {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}

pub fn paste() -> Result<bool> {
    if unsafe { AXIsProcessTrusted() } == 0 {
        return Ok(false);
    }
    let down = Event(unsafe { CGEventCreateKeyboardEvent(ptr::null(), 9, 1) });
    ensure!(!down.0.is_null(), "Could not create paste key event");
    let up = Event(unsafe { CGEventCreateKeyboardEvent(ptr::null(), 9, 0) });
    ensure!(!up.0.is_null(), "Could not create paste release event");
    unsafe {
        CGEventSetFlags(down.0, 1 << 20);
        CGEventSetFlags(up.0, 1 << 20);
        CGEventPost(0, down.0);
        CGEventPost(0, up.0);
    }
    Ok(true)
}

pub fn pointer_inside(window: &Window, region: Bounds<Pixels>) -> Result<bool> {
    let RawWindowHandle::AppKit(handle) =
        raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw()
    else {
        anyhow::bail!("Dictation requires an AppKit window");
    };
    unsafe {
        let view = handle.ns_view.cast::<AnyObject>().as_ref();
        let native: *mut AnyObject = msg_send![view, window];
        ensure!(!native.is_null(), "Dictation window is unavailable");
        let screen: NSPoint = msg_send![class!(NSEvent), mouseLocation];
        let local: NSPoint = msg_send![native, convertPointFromScreen: screen];
        let frame: NSRect = msg_send![view, frame];
        Ok(region.contains(&point(
            px(local.x as f32),
            px((frame.size.height - local.y) as f32),
        )))
    }
}

pub struct Monitor {
    global: Option<Retained<AnyObject>>,
    local: Option<Retained<AnyObject>>,
}

impl Monitor {
    fn listen(mask: usize, callback: impl Fn(*mut AnyObject) + 'static) -> Result<Self> {
        let callback = Rc::new(callback);
        let global_callback = callback.clone();
        let global = block2::RcBlock::new(move |event: *mut AnyObject| global_callback(event));
        let local = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            callback(event);
            event
        });
        let mut monitor = Self {
            global: None,
            local: None,
        };
        unsafe {
            let token: *mut AnyObject = msg_send![class!(NSEvent), addGlobalMonitorForEventsMatchingMask: mask, handler: &*global];
            monitor.global =
                Some(Retained::retain(token).context("Could not monitor global input")?);
            let token: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: mask, handler: &*local];
            monitor.local = Some(Retained::retain(token).context("Could not monitor local input")?);
        }
        Ok(monitor)
    }

    pub fn pointer() -> Result<(Self, async_channel::Receiver<()>)> {
        let (sender, receiver) = async_channel::bounded(1);
        let monitor = Self::listen((1 << 5) | (1 << 6) | (1 << 7), move |_| {
            let _ = sender.try_send(());
        })?;
        Ok((monitor, receiver))
    }

    pub fn modifiers(
        bindings: Vec<(super::platform::Shortcut, u32)>,
        sender: async_channel::Sender<super::platform::ShortcutEvent>,
    ) -> Result<Self> {
        ensure!(
            unsafe { AXIsProcessTrusted() } != 0,
            "Enable Accessibility to use modifier-only dictation shortcuts"
        );
        let active = RefCell::new(vec![false; bindings.len()]);
        Self::listen(1 << 12, move |event| {
            if event.is_null() {
                return;
            }
            let flags: usize = unsafe { msg_send![event, modifierFlags] };
            let modifiers = ((flags & (1 << 20) != 0) as u32 * 0x0100)
                | ((flags & (1 << 17) != 0) as u32 * 0x0200)
                | ((flags & (1 << 19) != 0) as u32 * 0x0800)
                | ((flags & (1 << 18) != 0) as u32 * 0x1000)
                | (flags as u32 & (1 << 23));
            let mut active = active.borrow_mut();
            for (index, &(shortcut, expected)) in bindings.iter().enumerate() {
                let pressed = modifiers == expected;
                if active[index] != pressed {
                    active[index] = pressed;
                    let _ = sender.try_send(super::platform::ShortcutEvent { shortcut, pressed });
                }
            }
        })
    }

    /// Calls `pressed` each time both keys of one modifier go down together. `device` is the
    /// pair's `NX_DEVICE*KEYMASK` bits, which tell the left key from the right one.
    pub fn both(device: usize, pressed: impl Fn() + 'static) -> Result<Self> {
        ensure!(
            unsafe { AXIsProcessTrusted() } != 0,
            "Enable Accessibility to use the appshot hotkey"
        );
        let down = std::cell::Cell::new(false);
        Self::listen(1 << 12, move |event| {
            if event.is_null() {
                return;
            }
            let flags: usize = unsafe { msg_send![event, modifierFlags] };
            let both = flags & device == device;
            if both && !down.get() {
                pressed();
            }
            down.set(both);
        })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        for monitor in [&self.global, &self.local].into_iter().flatten() {
            unsafe {
                let _: () = msg_send![class!(NSEvent), removeMonitor: &**monitor];
            }
        }
    }
}
