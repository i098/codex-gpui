use anyhow::{Context as _, Result, ensure};
use async_channel::Sender;
#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    ScreenshotCurrent,
    ScreenshotNew,
}

pub type Binding = (Shortcut, u32, u32);

pub fn binding(shortcut: Shortcut, accelerator: &str) -> Result<Binding> {
    let mut modifiers = 0;
    let mut key = None;
    for part in accelerator.split('+') {
        if let Some(modifier) = modifier_code(part) {
            modifiers |= modifier;
        } else {
            ensure!(key.is_none(), "A shortcut must contain only one key");
            key =
                Some(key_code(part).with_context(|| format!("Unsupported shortcut key: {part}"))?);
        }
    }
    ensure!(modifiers != 0, "A global shortcut needs a modifier");
    Ok((shortcut, key.context("A shortcut needs a key")?, modifiers))
}

fn modifier_code(part: &str) -> Option<u32> {
    [
        ("alt", 0x0800),
        ("option", 0x0800),
        ("ctrl", 0x1000),
        ("control", 0x1000),
        ("cmd", 0x0100),
        ("command", 0x0100),
        ("cmdorctrl", 0x0100),
        ("shift", 0x0200),
    ]
    .into_iter()
    .find_map(|(name, code)| part.eq_ignore_ascii_case(name).then_some(code))
}

fn key_code(key: &str) -> Option<u32> {
    const KEYS: [&str; 50] = [
        "a", "s", "d", "f", "h", "g", "z", "x", "c", "v", "", "b", "q", "w", "e", "r", "y", "t",
        "1", "2", "3", "4", "6", "5", "", "9", "7", "", "8", "0", "", "o", "u", "", "i", "p",
        "return", "l", "j", "", "k", "", "", "", "", "n", "m", "", "tab", "space",
    ];
    let key = if key.eq_ignore_ascii_case("enter") {
        "return"
    } else {
        key
    };
    KEYS.iter()
        .position(|candidate| !candidate.is_empty() && candidate.eq_ignore_ascii_case(key))
        .map(|code| code as u32)
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct HotkeyId {
    signature: u32,
    id: u32,
}
#[cfg(target_os = "macos")]
#[repr(C)]
struct EventType {
    class: u32,
    kind: u32,
}

#[cfg(target_os = "macos")]
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
        count: u32,
        types: *const EventType,
        data: *mut c_void,
        output: *mut *mut c_void,
    ) -> i32;
    fn RemoveEventHandler(handler: *mut c_void) -> i32;
    fn RegisterEventHotKey(
        key: u32,
        modifiers: u32,
        id: HotkeyId,
        target: *mut c_void,
        options: u32,
        output: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        kind: u32,
        actual_kind: *mut u32,
        size: u32,
        actual_size: *mut u32,
        value: *mut c_void,
    ) -> i32;
}

pub struct Native {
    #[cfg(target_os = "macos")]
    registrations: Vec<*mut c_void>,
    #[cfg(target_os = "macos")]
    handler: *mut c_void,
    #[cfg(target_os = "macos")]
    callback: Box<(Sender<Shortcut>, Vec<Shortcut>)>,
}

impl Native {
    pub fn new(bindings: &[Binding], sender: Sender<Shortcut>) -> Result<Self> {
        #[cfg(target_os = "macos")]
        unsafe {
            let mut state = Self {
                registrations: Vec::new(),
                handler: std::ptr::null_mut(),
                callback: Box::new((sender, bindings.iter().map(|binding| binding.0).collect())),
            };
            let event_type = EventType {
                class: u32::from_be_bytes(*b"keyb"),
                kind: 6,
            };
            let result = InstallEventHandler(
                GetApplicationEventTarget(),
                receive,
                1,
                &event_type,
                (&mut *state.callback as *mut (Sender<Shortcut>, Vec<Shortcut>)).cast(),
                &mut state.handler,
            );
            ensure!(
                result == 0,
                "Cannot install global shortcut handler ({result})"
            );
            for (id, binding) in bindings.iter().enumerate() {
                let mut registration = std::ptr::null_mut();
                let result = RegisterEventHotKey(
                    binding.1,
                    binding.2,
                    HotkeyId {
                        signature: u32::from_be_bytes(*b"CGXS"),
                        id: id as u32,
                    },
                    GetApplicationEventTarget(),
                    0,
                    &mut registration,
                );
                ensure!(
                    result == 0,
                    "Global shortcut {:?} is unavailable ({result})",
                    binding.0
                );
                state.registrations.push(registration);
            }
            Ok(state)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (bindings, sender);
            anyhow::bail!("Global shortcuts require macOS")
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn receive(_: *mut c_void, event: *mut c_void, data: *mut c_void) -> i32 {
    let mut id = HotkeyId {
        signature: 0,
        id: 0,
    };
    let result = unsafe {
        GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<HotkeyId>() as u32,
            std::ptr::null_mut(),
            (&mut id as *mut HotkeyId).cast(),
        )
    };
    if result != 0 || id.signature != u32::from_be_bytes(*b"CGXS") {
        return -9874;
    }
    let (sender, shortcuts) = unsafe { &*data.cast::<(Sender<Shortcut>, Vec<Shortcut>)>() };
    if let Some(shortcut) = shortcuts.get(id.id as usize) {
        let _ = sender.try_send(*shortcut);
    }
    0
}

impl Drop for Native {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            for registration in &self.registrations {
                UnregisterEventHotKey(*registration);
            }
            if !self.handler.is_null() {
                RemoveEventHandler(self.handler);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_or_unmodified_global_shortcuts() {
        assert!(binding(Shortcut::ScreenshotCurrent, "Space").is_err());
        assert!(binding(Shortcut::ScreenshotCurrent, "Alt+K+C").is_err());
        assert!(binding(Shortcut::ScreenshotCurrent, "Alt+").is_err());
        let screenshot = binding(Shortcut::ScreenshotNew, "Control+Alt+Shift+4").unwrap();
        assert_eq!(screenshot.1, 21);
        assert_eq!(screenshot.2, 0x1a00);
    }
}
