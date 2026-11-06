//! Menu-bar status item (the original's `new Tray`): Open, New chat, recent chats,
//! Send Feedback and Quit, gated by the `Show in menu bar` setting. The app already
//! keeps its last primary window hidden but alive on macOS, so the item reopens it.

use gpui::App;
#[cfg(target_os = "macos")]
use std::cell::RefCell;

/// Recent chats shown in the menu.
#[cfg(target_os = "macos")]
const RECENT_LIMIT: usize = 8;

/// A recent chat: thread id and display title.
pub(crate) type Recent = (String, String);

#[cfg(target_os = "macos")]
thread_local! {
    static TRAY: RefCell<native::Tray> = RefCell::new(native::Tray::default());
}

/// Stores the recent chats and rebuilds the menu.
pub(crate) fn set_recents(recents: &[Recent], cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        TRAY.with(|tray| {
            tray.borrow_mut().recents = recents.iter().take(RECENT_LIMIT).cloned().collect()
        });
        refresh(cx);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (recents, cx);
}

/// Shows or removes the status item from the `Show in menu bar` setting.
pub(crate) fn refresh(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        let enabled = cx
            .try_global::<crate::host_settings::Settings>()
            .and_then(|settings| settings.read_all().get("mac-menu-bar-enabled").cloned())
            .is_none_or(|value| value != false);
        let app = cx.to_async();
        TRAY.with(|tray| tray.borrow_mut().sync(enabled, app));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

#[cfg(target_os = "macos")]
fn act(tag: isize, id: Option<String>, cx: &mut App) {
    use crate::{Root, windows};
    match (tag, id) {
        (0, _) => {
            let _ = windows::reopen(cx);
        }
        (1, _) => crate::deep_link::open("codex://threads/new", cx),
        (2, _) => {
            let _ = windows::reopen(cx);
            if let Some(handle) = windows::primary(cx).and_then(|window| window.downcast::<Root>())
            {
                let _ = handle.update(cx, |root, window, cx| root.open_feedback(window, cx));
            }
        }
        (3, _) => cx.quit(),
        (_, Some(id)) => crate::deep_link::open(&format!("codex://threads/{id}"), cx),
        _ => {}
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{Recent, act};
    use gpui::AsyncApp;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, DefinedClass, MainThreadOnly, class, define_class, msg_send, sel};
    use objc2_app_kit::NSImage;
    use objc2_foundation::{
        MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSSize, NSString,
    };

    struct Handler(Box<dyn Fn(isize)>);

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = Handler]
        struct TrayTarget;

        unsafe impl NSObjectProtocol for TrayTarget {}

        impl TrayTarget {
            #[unsafe(method(trayAction:))]
            fn tray_action(&self, item: &AnyObject) {
                let tag: isize = unsafe { msg_send![item, tag] };
                (self.ivars().0)(tag);
            }
        }
    );

    #[derive(Default)]
    pub(super) struct Tray {
        pub recents: Vec<Recent>,
        item: Option<Retained<AnyObject>>,
        target: Option<Retained<TrayTarget>>,
    }

    impl Tray {
        pub(super) fn sync(&mut self, enabled: bool, app: AsyncApp) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            if !enabled {
                if let Some(item) = self.item.take() {
                    unsafe {
                        let bar: Retained<AnyObject> =
                            msg_send![class!(NSStatusBar), systemStatusBar];
                        let _: () = msg_send![&*bar, removeStatusItem: &*item];
                    }
                }
                self.target = None;
                return;
            }
            let ids: Vec<String> = self.recents.iter().map(|(id, _)| id.clone()).collect();
            let target = self.target.get_or_insert_with(|| {
                let handler = Handler(Box::new(move |tag| {
                    let id = usize::try_from(tag - 100)
                        .ok()
                        .and_then(|index| TRAY_IDS.with(|ids| ids.borrow().get(index).cloned()));
                    app.update(|cx| act(tag, id, cx));
                }));
                let object = TrayTarget::alloc(mtm).set_ivars(handler);
                unsafe { msg_send![super(object), init] }
            });
            TRAY_IDS.with(|cell| *cell.borrow_mut() = ids);
            let target = target.clone();
            if self.item.is_none() {
                self.item = Some(unsafe { new_item() });
            }
            let item = self.item.as_ref().unwrap();
            unsafe {
                let menu = build_menu(&self.recents, &target);
                let _: () = msg_send![&**item, setMenu: &*menu];
            }
        }
    }

    thread_local! {
        static TRAY_IDS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// An NSStatusItem of variable length with the app icon (title `Codex` if it is missing).
    unsafe fn new_item() -> Retained<AnyObject> {
        unsafe {
            let bar: Retained<AnyObject> = msg_send![class!(NSStatusBar), systemStatusBar];
            let item: Retained<AnyObject> = msg_send![&*bar, statusItemWithLength: -1.0f64];
            let button: Option<Retained<AnyObject>> = msg_send![&*item, button];
            if let Some(button) = button {
                let image = theme::Assets::get("dock/codex-light.png").and_then(|png| {
                    let data = NSData::with_bytes(png.data.as_ref());
                    NSImage::initWithData(NSImage::alloc(), &data)
                });
                match image {
                    Some(image) => {
                        image.setSize(NSSize::new(18., 18.));
                        let _: () = msg_send![&*button, setImage: &*image];
                    }
                    None => {
                        let _: () = msg_send![&*button, setTitle: &*NSString::from_str("Codex")];
                    }
                }
            }
            item
        }
    }

    unsafe fn build_menu(recents: &[Recent], target: &TrayTarget) -> Retained<AnyObject> {
        unsafe {
            let menu: Retained<AnyObject> = msg_send![class!(NSMenu), new];
            let add = |title: &str, tag: isize| {
                let item: Retained<AnyObject> = msg_send![class!(NSMenuItem), new];
                let _: () = msg_send![&*item, setTitle: &*NSString::from_str(title)];
                let _: () = msg_send![&*item, setAction: sel!(trayAction:)];
                let _: () = msg_send![&*item, setTarget: target];
                let _: () = msg_send![&*item, setTag: tag];
                let _: () = msg_send![&*menu, addItem: &*item];
            };
            let separator = || {
                let item: Retained<AnyObject> = msg_send![class!(NSMenuItem), separatorItem];
                let _: () = msg_send![&*menu, addItem: &*item];
            };
            add("Open Codex", 0);
            add("New chat", 1);
            if !recents.is_empty() {
                separator();
                for (index, (_, title)) in recents.iter().enumerate() {
                    add(title, 100 + index as isize);
                }
            }
            separator();
            add("Send Feedback", 2);
            add("Quit Codex", 3);
            menu
        }
    }
}
