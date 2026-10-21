use anyhow::{Result, ensure};
use async_channel::Sender;
#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Spotlight,
    ScreenshotCurrent,
    ScreenshotNew,
    Popout,
    DictationHold,
    DictationToggle,
}

pub type Binding = (Shortcut, Option<u32>, u32);

#[derive(Clone, Copy)]
pub struct ShortcutEvent {
    pub shortcut: Shortcut,
    pub pressed: bool,
}

pub fn binding(shortcut: Shortcut, accelerator: &str) -> Result<Binding> {
    let mut modifiers = 0;
    let mut key = None;
    for part in accelerator.split('+') {
        match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => modifiers |= 0x0800,
            "ctrl" | "control" => modifiers |= 0x1000,
            "cmd" | "command" | "cmdorctrl" => modifiers |= 0x0100,
            "shift" => modifiers |= 0x0200,
            "fn" | "function" => modifiers |= 1 << 23,
            other => {
                ensure!(key.is_none(), "A shortcut must contain only one key");
                key = Some(match other {
                    "a" => 0,
                    "s" => 1,
                    "d" => 2,
                    "f" => 3,
                    "h" => 4,
                    "g" => 5,
                    "z" => 6,
                    "x" => 7,
                    "c" => 8,
                    "v" => 9,
                    "b" => 11,
                    "q" => 12,
                    "w" => 13,
                    "e" => 14,
                    "r" => 15,
                    "y" => 16,
                    "t" => 17,
                    "1" => 18,
                    "2" => 19,
                    "3" => 20,
                    "4" => 21,
                    "6" => 22,
                    "5" => 23,
                    "9" => 25,
                    "7" => 26,
                    "8" => 28,
                    "0" => 29,
                    "o" => 31,
                    "u" => 32,
                    "i" => 34,
                    "p" => 35,
                    "l" => 37,
                    "j" => 38,
                    "k" => 40,
                    "n" => 45,
                    "m" => 46,
                    "return" | "enter" => 36,
                    "tab" => 48,
                    "space" => 49,
                    _ => anyhow::bail!("Unsupported shortcut key: {part}"),
                });
            }
        }
    }
    ensure!(modifiers != 0, "A global shortcut needs a modifier");
    ensure!(
        key.is_some()
            || matches!(
                shortcut,
                Shortcut::DictationHold | Shortcut::DictationToggle
            ),
        "A shortcut needs a key"
    );
    Ok((shortcut, key, modifiers))
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
    fn GetEventKind(event: *mut c_void) -> u32;
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
    callback: Box<(Sender<ShortcutEvent>, Vec<Shortcut>)>,
    #[cfg(target_os = "macos")]
    modifiers: Option<super::desktop::Monitor>,
}

impl Native {
    pub fn new(bindings: &[Binding], sender: Sender<ShortcutEvent>) -> Result<Self> {
        #[cfg(target_os = "macos")]
        unsafe {
            let mut state = Self {
                registrations: Vec::new(),
                handler: std::ptr::null_mut(),
                callback: Box::new((
                    sender.clone(),
                    bindings.iter().map(|binding| binding.0).collect(),
                )),
                modifiers: None,
            };
            let event_types = [
                EventType {
                    class: u32::from_be_bytes(*b"keyb"),
                    kind: 6,
                },
                EventType {
                    class: u32::from_be_bytes(*b"keyb"),
                    kind: 7,
                },
            ];
            let result = InstallEventHandler(
                GetApplicationEventTarget(),
                receive,
                event_types.len() as u32,
                event_types.as_ptr(),
                (&mut *state.callback as *mut (Sender<ShortcutEvent>, Vec<Shortcut>)).cast(),
                &mut state.handler,
            );
            ensure!(
                result == 0,
                "Cannot install global shortcut handler ({result})"
            );
            for (id, binding) in bindings.iter().enumerate() {
                let Some(key) = binding.1 else { continue };
                let mut registration = std::ptr::null_mut();
                let result = RegisterEventHotKey(
                    key,
                    binding.2,
                    HotkeyId {
                        signature: u32::from_be_bytes(*b"CGXH"),
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
            let modifiers: Vec<_> = bindings
                .iter()
                .filter(|binding| binding.1.is_none())
                .map(|binding| (binding.0, binding.2))
                .collect();
            if !modifiers.is_empty() {
                state.modifiers = Some(super::desktop::Monitor::modifiers(modifiers, sender)?);
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
    if result != 0 || id.signature != u32::from_be_bytes(*b"CGXH") {
        return -9874;
    }
    let (sender, shortcuts) = unsafe { &*data.cast::<(Sender<ShortcutEvent>, Vec<Shortcut>)>() };
    if let Some(shortcut) = shortcuts.get(id.id as usize) {
        let _ = sender.try_send(ShortcutEvent {
            shortcut: *shortcut,
            pressed: unsafe { GetEventKind(event) } == 6,
        });
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
        assert!(binding(Shortcut::Popout, "Space").is_err());
        assert!(binding(Shortcut::Popout, "Alt+K+C").is_err());
        assert!(binding(Shortcut::Popout, "Alt+").is_err());
        let screenshot = binding(Shortcut::ScreenshotNew, "Control+Alt+Shift+4").unwrap();
        assert_eq!(screenshot.1, Some(21));
        assert_eq!(screenshot.2, 0x1a00);
        assert_eq!(binding(Shortcut::DictationHold, "Fn").unwrap().1, None);
    }
}
