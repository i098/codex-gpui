use anyhow::{Result, ensure};
use async_channel::Sender;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use std::ffi::c_void;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Spotlight,
    Mini,
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
    // Held for its `Drop`, which stops the worker thread.
    #[allow(dead_code)]
    #[cfg(target_os = "linux")]
    linux: Option<LinuxNative>,
    #[cfg(target_os = "windows")]
    windows: Option<WindowsNative>,
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
        #[cfg(target_os = "linux")]
        {
            let linux = if bindings.is_empty() {
                None
            } else {
                Some(LinuxNative::new(bindings, sender)?)
            };
            Ok(Self { linux })
        }
        #[cfg(target_os = "windows")]
        {
            let windows = if bindings.is_empty() {
                None
            } else {
                Some(WindowsNative::new(bindings, sender)?)
            };
            Ok(Self { windows })
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            let _ = (bindings, sender);
            anyhow::bail!("Global shortcuts are not supported on this platform")
        }
    }
}
#[cfg(target_os = "linux")]
struct LinuxNative {
    stop: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
impl LinuxNative {
    fn new(bindings: &[Binding], sender: Sender<ShortcutEvent>) -> Result<Self> {
        let (stop, stop_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let bindings = bindings.to_vec();
        let worker = std::thread::spawn(move || run_linux(bindings, sender, stop_rx, ready_tx));
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(anyhow::anyhow!(error))
            }
            Err(error) => {
                let _ = worker.join();
                Err(anyhow::anyhow!(
                    "Linux hotkey worker stopped during startup: {error}"
                ))
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxNative {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(target_os = "linux")]
type XErrorHandler = Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> std::os::raw::c_int>;

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy)]
struct XKeyEvent {
    _event_type: std::os::raw::c_int,
    _serial: std::os::raw::c_ulong,
    _send_event: std::os::raw::c_int,
    _display: *mut c_void,
    _window: std::os::raw::c_ulong,
    _root: std::os::raw::c_ulong,
    _subwindow: std::os::raw::c_ulong,
    _time: std::os::raw::c_ulong,
    _x: std::os::raw::c_int,
    _y: std::os::raw::c_int,
    _x_root: std::os::raw::c_int,
    _y_root: std::os::raw::c_int,
    state: std::os::raw::c_uint,
    keycode: std::os::raw::c_uint,
    _same_screen: std::os::raw::c_int,
}

#[cfg(target_os = "linux")]
#[repr(C)]
union XEvent {
    event_type: std::os::raw::c_int,
    key: XKeyEvent,
    _pad: [std::os::raw::c_long; 24],
}

#[cfg(target_os = "linux")]
#[link(name = "X11")]
unsafe extern "C" {
    #[cfg(test)]
    fn XInitThreads() -> std::os::raw::c_int;
    fn XOpenDisplay(name: *const std::os::raw::c_char) -> *mut c_void;
    fn XCloseDisplay(display: *mut c_void) -> std::os::raw::c_int;
    fn XDefaultRootWindow(display: *mut c_void) -> std::os::raw::c_ulong;
    fn XKeysymToKeycode(
        display: *mut c_void,
        keysym: std::os::raw::c_ulong,
    ) -> std::os::raw::c_uint;
    fn XGrabKey(
        display: *mut c_void,
        keycode: std::os::raw::c_int,
        modifiers: std::os::raw::c_uint,
        window: std::os::raw::c_ulong,
        owner_events: std::os::raw::c_int,
        pointer_mode: std::os::raw::c_int,
        keyboard_mode: std::os::raw::c_int,
    );
    fn XUngrabKey(
        display: *mut c_void,
        keycode: std::os::raw::c_int,
        modifiers: std::os::raw::c_uint,
        window: std::os::raw::c_ulong,
    );
    fn XSetErrorHandler(handler: XErrorHandler) -> XErrorHandler;
    fn XSync(display: *mut c_void, discard: std::os::raw::c_int) -> std::os::raw::c_int;
    fn XPending(display: *mut c_void) -> std::os::raw::c_int;
    fn XNextEvent(display: *mut c_void, event: *mut XEvent) -> std::os::raw::c_int;
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct X11Binding {
    keycode: std::os::raw::c_uint,
    modifiers: std::os::raw::c_uint,
    shortcut: Shortcut,
}

#[cfg(target_os = "linux")]
static X11_ERROR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
unsafe extern "C" fn capture_x11_error(_: *mut c_void, _: *mut c_void) -> std::os::raw::c_int {
    X11_ERROR.store(true, std::sync::atomic::Ordering::Relaxed);
    0
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn character_key(key: u32) -> Option<u8> {
    Some(match key {
        0 => b'a',
        1 => b's',
        2 => b'd',
        3 => b'f',
        4 => b'h',
        5 => b'g',
        6 => b'z',
        7 => b'x',
        8 => b'c',
        9 => b'v',
        11 => b'b',
        12 => b'q',
        13 => b'w',
        14 => b'e',
        15 => b'r',
        16 => b'y',
        17 => b't',
        18 => b'1',
        19 => b'2',
        20 => b'3',
        21 => b'4',
        22 => b'6',
        23 => b'5',
        25 => b'9',
        26 => b'7',
        28 => b'8',
        29 => b'0',
        31 => b'o',
        32 => b'u',
        34 => b'i',
        35 => b'p',
        37 => b'l',
        38 => b'j',
        40 => b'k',
        45 => b'n',
        46 => b'm',
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
fn x11_keysym(key: u32) -> Option<std::os::raw::c_ulong> {
    match key {
        36 => Some(0xff0d),
        48 => Some(0xff09),
        49 => Some(b' ' as _),
        _ => character_key(key).map(|key| key as _),
    }
}

#[cfg(target_os = "linux")]
fn x11_modifiers(modifiers: u32) -> Option<std::os::raw::c_uint> {
    let supported = 0x1b00;
    if modifiers & !supported != 0 {
        return None;
    }
    let mut result = 0;
    if modifiers & 0x0800 != 0 {
        result |= 1 << 3;
    }
    if modifiers & (0x1000 | 0x0100) != 0 {
        result |= 1 << 2;
    }
    if modifiers & 0x0200 != 0 {
        result |= 1;
    }
    Some(result)
}

#[cfg(target_os = "linux")]
// SAFETY: This worker owns the Xlib display and initializes event storage before reading it.
fn run_linux(
    bindings: Vec<Binding>,
    sender: Sender<ShortcutEvent>,
    stop: std::sync::mpsc::Receiver<()>,
    ready: std::sync::mpsc::SyncSender<std::result::Result<(), String>>,
) {
    const KEY_PRESS: i32 = 2;
    const KEY_RELEASE: i32 = 3;
    const LOCK_MASK: u32 = 1 << 1;
    const NUM_LOCK_MASK: u32 = 1 << 4;
    let display = unsafe { XOpenDisplay(std::ptr::null()) };
    if display.is_null() {
        let _ = ready.send(Err("Cannot connect to the X11 display".into()));
        return;
    }
    let root = unsafe { XDefaultRootWindow(display) };
    let mut registered = Vec::new();
    for (shortcut, key, modifiers) in bindings {
        let Some(key) = key else {
            eprintln!("codex-gpui: skipping modifier-only X11 shortcut {shortcut:?}");
            continue;
        };
        let Some(keysym) = x11_keysym(key) else {
            eprintln!("codex-gpui: skipping unsupported X11 key {key}");
            continue;
        };
        let keycode = unsafe { XKeysymToKeycode(display, keysym) };
        let Some(modifiers) = x11_modifiers(modifiers) else {
            eprintln!("codex-gpui: skipping unsupported X11 modifiers for {shortcut:?}");
            continue;
        };
        if keycode == 0 {
            eprintln!("codex-gpui: skipping X11 shortcut without a keycode: {shortcut:?}");
            continue;
        }
        registered.push(X11Binding {
            keycode,
            modifiers,
            shortcut,
        });
    }

    X11_ERROR.store(false, std::sync::atomic::Ordering::Relaxed);
    let previous_handler = unsafe { XSetErrorHandler(Some(capture_x11_error)) };
    for binding in &registered {
        for lock in [0, LOCK_MASK, NUM_LOCK_MASK, LOCK_MASK | NUM_LOCK_MASK] {
            unsafe {
                XGrabKey(
                    display,
                    binding.keycode as i32,
                    binding.modifiers | lock,
                    root,
                    0,
                    1,
                    1,
                );
            }
        }
    }
    unsafe { XSync(display, 0) };
    if X11_ERROR.load(std::sync::atomic::Ordering::Relaxed) {
        for binding in &registered {
            for lock in [0, LOCK_MASK, NUM_LOCK_MASK, LOCK_MASK | NUM_LOCK_MASK] {
                unsafe {
                    XUngrabKey(
                        display,
                        binding.keycode as i32,
                        binding.modifiers | lock,
                        root,
                    );
                }
            }
        }
        unsafe {
            XSync(display, 0);
            XSetErrorHandler(previous_handler);
            XCloseDisplay(display);
        }
        let _ = ready.send(Err("An X11 global shortcut is unavailable".into()));
        return;
    }
    unsafe { XSetErrorHandler(previous_handler) };
    if ready.send(Ok(())).is_err() {
        unsafe { XCloseDisplay(display) };
        return;
    }

    loop {
        match stop.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        while unsafe { XPending(display) } > 0 {
            let mut event = XEvent { _pad: [0; 24] };
            unsafe { XNextEvent(display, &mut event) };
            let event_type = unsafe { event.event_type };
            if event_type != KEY_PRESS && event_type != KEY_RELEASE {
                continue;
            }
            let key = unsafe { event.key };
            let modifiers = key.state & !(LOCK_MASK | NUM_LOCK_MASK);
            for binding in &registered {
                if key.keycode == binding.keycode && modifiers == binding.modifiers {
                    let _ = sender.try_send(ShortcutEvent {
                        shortcut: binding.shortcut,
                        pressed: event_type == KEY_PRESS,
                    });
                }
            }
        }
    }

    for binding in &registered {
        for lock in [0, LOCK_MASK, NUM_LOCK_MASK, LOCK_MASK | NUM_LOCK_MASK] {
            unsafe {
                XUngrabKey(
                    display,
                    binding.keycode as i32,
                    binding.modifiers | lock,
                    root,
                );
            }
        }
    }
    unsafe {
        XSync(display, 0);
        XCloseDisplay(display);
    }
}

#[cfg(target_os = "windows")]
struct WindowsNative {
    thread_id: u32,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "windows")]
impl WindowsNative {
    fn new(bindings: &[Binding], sender: Sender<ShortcutEvent>) -> Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let bindings = bindings.to_vec();
        let worker = std::thread::spawn(move || run_windows(bindings, sender, ready_tx));
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(anyhow::anyhow!(error))
            }
            Err(error) => {
                let _ = worker.join();
                Err(anyhow::anyhow!(
                    "Windows hotkey worker stopped during startup: {error}"
                ))
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsNative {
    fn drop(&mut self) {
        // SAFETY: The worker publishes its thread ID only after creating its message queue.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(target_os = "windows")]
const WM_QUIT: u32 = 0x0012;
#[cfg(target_os = "windows")]
const WM_HOTKEY: u32 = 0x0312;
#[cfg(target_os = "windows")]
const WM_KEYUP: u32 = 0x0101;
#[cfg(target_os = "windows")]
const WM_SYSKEYUP: u32 = 0x0105;
#[cfg(target_os = "windows")]
const WH_KEYBOARD_LL: i32 = 13;
#[cfg(target_os = "windows")]
const MOD_ALT: u32 = 0x0001;
#[cfg(target_os = "windows")]
const MOD_CONTROL: u32 = 0x0002;
#[cfg(target_os = "windows")]
const MOD_SHIFT: u32 = 0x0004;
#[cfg(target_os = "windows")]
const MOD_NOREPEAT: u32 = 0x4000;

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinPoint {
    _x: i32,
    _y: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinMessage {
    _window: *mut c_void,
    message: u32,
    w_param: usize,
    _l_param: isize,
    _time: u32,
    _point: WinPoint,
    _private: u32,
}

#[cfg(target_os = "windows")]
type LowLevelKeyboardProc = Option<unsafe extern "system" fn(i32, usize, isize) -> isize>;

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinKeyboardHook {
    key: u32,
    _scan_code: u32,
    _flags: u32,
    _time: u32,
    _extra_info: usize,
}

#[cfg(target_os = "windows")]
struct WindowsHold {
    id: usize,
    key: u32,
    active: bool,
}

#[cfg(target_os = "windows")]
std::thread_local! {
    static WINDOWS_HOLDS: std::cell::RefCell<Vec<WindowsHold>> = std::cell::RefCell::new(Vec::new());
    static WINDOWS_SENDER: std::cell::RefCell<Option<Sender<ShortcutEvent>>> = std::cell::RefCell::new(None);
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterHotKey(window: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
    fn UnregisterHotKey(window: *mut c_void, id: i32) -> i32;
    fn GetMessageW(message: *mut WinMessage, window: *mut c_void, min: u32, max: u32) -> i32;
    fn PeekMessageW(
        message: *mut WinMessage,
        window: *mut c_void,
        min: u32,
        max: u32,
        remove: u32,
    ) -> i32;
    fn PostThreadMessageW(thread_id: u32, message: u32, w_param: usize, l_param: isize) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn SetWindowsHookExW(
        id: i32,
        callback: LowLevelKeyboardProc,
        instance: *mut c_void,
        thread_id: u32,
    ) -> *mut c_void;
    fn UnhookWindowsHookEx(hook: *mut c_void) -> i32;
    fn CallNextHookEx(hook: *mut c_void, code: i32, w_param: usize, l_param: isize) -> isize;
}

#[cfg(target_os = "windows")]
// SAFETY: Windows supplies a valid keyboard event pointer for nonnegative hook codes.
unsafe extern "system" fn windows_keyboard_hook(
    code: i32,
    w_param: usize,
    l_param: isize,
) -> isize {
    if code >= 0 && matches!(w_param as u32, WM_KEYUP | WM_SYSKEYUP) && l_param != 0 {
        let event = unsafe { &*(l_param as *const WinKeyboardHook) };
        WINDOWS_HOLDS.with(|holds| {
            if let Ok(mut holds) = holds.try_borrow_mut() {
                WINDOWS_SENDER.with(|sender| {
                    if let Ok(sender) = sender.try_borrow() {
                        if let Some(sender) = sender.as_ref() {
                            for hold in holds.iter_mut() {
                                if hold.active && hold.key == event.key {
                                    hold.active = false;
                                    let _ = sender.try_send(ShortcutEvent {
                                        shortcut: Shortcut::DictationHold,
                                        pressed: false,
                                    });
                                }
                            }
                        }
                    }
                });
            }
        });
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

#[cfg(target_os = "windows")]
fn windows_key(key: u32) -> Option<u32> {
    match key {
        36 => Some(0x0d),
        48 => Some(0x09),
        49 => Some(0x20),
        _ => character_key(key).map(|key| key.to_ascii_uppercase() as u32),
    }
}

#[cfg(target_os = "windows")]
fn windows_modifiers(modifiers: u32) -> Option<u32> {
    if modifiers & !0x1b00 != 0 {
        return None;
    }
    let mut result = MOD_NOREPEAT;
    if modifiers & 0x0800 != 0 {
        result |= MOD_ALT;
    }
    if modifiers & (0x1000 | 0x0100) != 0 {
        result |= MOD_CONTROL;
    }
    if modifiers & 0x0200 != 0 {
        result |= MOD_SHIFT;
    }
    Some(result)
}

#[cfg(target_os = "windows")]
// SAFETY: This worker owns its message queue, hotkey registrations, and hook handle.
fn run_windows(
    bindings: Vec<Binding>,
    sender: Sender<ShortcutEvent>,
    ready: std::sync::mpsc::SyncSender<std::result::Result<u32, String>>,
) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let mut message = WinMessage {
        _window: std::ptr::null_mut(),
        message: 0,
        w_param: 0,
        _l_param: 0,
        _time: 0,
        _point: WinPoint { _x: 0, _y: 0 },
        _private: 0,
    };
    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 0) };
    let mut registered = Vec::new();
    let mut holds = Vec::new();
    for (id, binding) in bindings.iter().enumerate() {
        let Some(key) = binding.1 else {
            eprintln!(
                "codex-gpui: skipping modifier-only Windows shortcut {:?}",
                binding.0
            );
            continue;
        };
        let Some(key) = windows_key(key) else {
            eprintln!("codex-gpui: skipping unsupported Windows key {key}");
            continue;
        };
        let Some(modifiers) = windows_modifiers(binding.2) else {
            eprintln!(
                "codex-gpui: skipping unsupported Windows modifiers for {:?}",
                binding.0
            );
            continue;
        };
        if unsafe { RegisterHotKey(std::ptr::null_mut(), id as i32, modifiers, key) } == 0 {
            let error = unsafe { GetLastError() };
            let _ = ready.send(Err(format!(
                "Global shortcut {:?} is unavailable ({error})",
                binding.0
            )));
            for id in registered {
                unsafe { UnregisterHotKey(std::ptr::null_mut(), id) };
            }
            return;
        }
        registered.push(id as i32);
        if binding.0 == Shortcut::DictationHold {
            holds.push(WindowsHold {
                id,
                key,
                active: false,
            });
        }
    }
    let hook = if holds.is_empty() {
        std::ptr::null_mut()
    } else {
        WINDOWS_HOLDS.with(|state| *state.borrow_mut() = holds);
        WINDOWS_SENDER.with(|state| *state.borrow_mut() = Some(sender.clone()));
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(windows_keyboard_hook),
                std::ptr::null_mut(),
                0,
            )
        };
        if hook.is_null() {
            let error = unsafe { GetLastError() };
            WINDOWS_HOLDS.with(|state| state.borrow_mut().clear());
            WINDOWS_SENDER.with(|state| *state.borrow_mut() = None);
            let _ = ready.send(Err(format!("Cannot monitor global key release ({error})")));
            for id in registered {
                unsafe { UnregisterHotKey(std::ptr::null_mut(), id) };
            }
            return;
        }
        hook
    };
    if ready.send(Ok(thread_id)).is_err() {
        if !hook.is_null() {
            unsafe { UnhookWindowsHookEx(hook) };
        }
        WINDOWS_HOLDS.with(|state| state.borrow_mut().clear());
        WINDOWS_SENDER.with(|state| *state.borrow_mut() = None);
        for id in registered {
            unsafe { UnregisterHotKey(std::ptr::null_mut(), id) };
        }
        return;
    }
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        if message.message == WM_HOTKEY {
            let id = message.w_param;
            if let Some(binding) = bindings.get(id) {
                let should_send = if binding.0 == Shortcut::DictationHold {
                    let mut should_send = false;
                    WINDOWS_HOLDS.with(|holds| {
                        if let Ok(mut holds) = holds.try_borrow_mut() {
                            if let Some(hold) = holds.iter_mut().find(|hold| hold.id == id) {
                                should_send = !hold.active;
                                hold.active = true;
                            }
                        }
                    });
                    should_send
                } else {
                    true
                };
                if should_send {
                    let _ = sender.try_send(ShortcutEvent {
                        shortcut: binding.0,
                        pressed: true,
                    });
                }
            }
        }
    }
    if !hook.is_null() {
        unsafe { UnhookWindowsHookEx(hook) };
    }
    WINDOWS_HOLDS.with(|state| state.borrow_mut().clear());
    WINDOWS_SENDER.with(|state| *state.borrow_mut() = None);
    for id in registered {
        unsafe { UnregisterHotKey(std::ptr::null_mut(), id) };
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
    #[cfg(target_os = "linux")]
    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(filename: *const std::os::raw::c_char, flags: std::os::raw::c_int)
        -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const std::os::raw::c_char) -> *mut c_void;
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn maps_x11_keys_and_modifiers() {
        assert_eq!(x11_keysym(40), Some(b'k' as _));
        assert_eq!(x11_keysym(36), Some(0xff0d));
        assert_eq!(x11_keysym(48), Some(0xff09));
        assert_eq!(x11_modifiers(0x1a00), Some(0x0d));
        assert!(x11_modifiers(1 << 23).is_none());
    }

    // SAFETY: The smoke test initializes Xlib threading before using two display connections.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires Xvfb and the XTEST extension"]
    fn x11_hotkey_emits_press_and_release() {
        use std::os::raw::c_ulong;
        use std::time::{Duration, Instant};

        type FakeKeyEvent = unsafe extern "C" fn(*mut c_void, u32, i32, c_ulong) -> i32;
        unsafe {
            assert_ne!(XInitThreads(), 0);
        }
        let display = unsafe { XOpenDisplay(std::ptr::null()) };
        assert!(!display.is_null());
        let xtst = unsafe { dlopen(c"libXtst.so.6".as_ptr(), 2) };
        assert!(!xtst.is_null());
        let fake = unsafe { dlsym(xtst, c"XTestFakeKeyEvent".as_ptr()) };
        assert!(!fake.is_null());
        // SAFETY: `dlsym` found the XTest function with this ABI and signature.
        let fake_key_event: FakeKeyEvent = unsafe { std::mem::transmute(fake) };
        let alt = unsafe { XKeysymToKeycode(display, 0xffe9) };
        let key = unsafe { XKeysymToKeycode(display, b'k' as _) };
        assert_ne!(alt, 0);
        assert_ne!(key, 0);
        let bindings = [
            binding(Shortcut::Spotlight, "Alt+K").unwrap(),
            binding(Shortcut::DictationHold, "Fn").unwrap(),
        ];
        let (sender, receiver) = async_channel::unbounded();
        let native = Native::new(&bindings, sender).unwrap();
        for (key, pressed) in [(alt, 1), (key, 1), (key, 0), (alt, 0)] {
            assert_ne!(unsafe { fake_key_event(display, key, pressed, 0) }, 0);
        }
        unsafe { XSync(display, 0) };
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut events = Vec::new();
        while events.len() < 2 && Instant::now() < deadline {
            if let Ok(event) = receiver.try_recv() {
                events.push(event.pressed);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(events, [true, false]);
        drop(native);
        unsafe {
            XCloseDisplay(display);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn maps_windows_keys_and_modifiers() {
        assert_eq!(windows_key(40), Some(b'K' as u32));
        assert_eq!(windows_key(18), Some(b'1' as u32));
        assert_eq!(windows_key(36), Some(0x0d));
        assert_eq!(windows_key(49), Some(0x20));
        assert_eq!(
            windows_modifiers(0x1a00),
            Some(MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT)
        );
        assert!(windows_modifiers(1 << 23).is_none());
    }

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
