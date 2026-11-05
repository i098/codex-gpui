use super::Item;
use anyhow::{Context as _, Result};
use gpui::{Pixels, Point, Window};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSEventModifierFlags, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSPoint, NSSize, NSString,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{Value, json};
use std::cell::Cell;

#[derive(Default)]
struct Selection {
    item: Cell<Option<usize>>,
    opened: Cell<bool>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Selection]
    struct SectionMenuTarget;

    unsafe impl NSObjectProtocol for SectionMenuTarget {}

    unsafe impl NSMenuDelegate for SectionMenuTarget {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _: &NSMenu) {
            self.ivars().opened.set(true);
        }
    }

    impl SectionMenuTarget {
        #[unsafe(method(selectSectionItem:))]
        fn select_item(&self, item: &NSMenuItem) {
            self.ivars().item.set(Some(item.tag() as usize));
        }
    }
);

impl SectionMenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(Selection::default());
        unsafe { msg_send![super(object), init] }
    }
}

pub(crate) struct Outcome {
    pub selected: Option<usize>,
    pub snapshot: Value,
}

pub(crate) fn popup(
    items: &[Item],
    enabled: &[bool],
    anchor: Point<Pixels>,
    dark: bool,
    window: &Window,
) -> Result<Outcome> {
    let mtm = MainThreadMarker::new().context("section menu requires the main thread")?;
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)?.as_raw() else {
        anyhow::bail!("section menu requires an AppKit window");
    };
    // The GPUI window retains this view throughout synchronous menu tracking.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let native_window = view.window().context("section menu view has no window")?;
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let appearance = unsafe {
        NSAppearance::appearanceNamed(if dark {
            NSAppearanceNameDarkAqua
        } else {
            NSAppearanceNameAqua
        })
    };
    menu.setAppearance(appearance.as_deref());
    let target = SectionMenuTarget::new(mtm);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    for (index, entry) in items.iter().enumerate() {
        let item = if entry.is_separator() {
            NSMenuItem::separatorItem(mtm)
        } else {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(entry.label),
                    Some(sel!(selectSectionItem:)),
                    &NSString::from_str(""),
                )
            };
            unsafe { item.setTarget(Some(&target)) };
            item.setTag(index as isize);
            item.setEnabled(enabled[index]);
            item.setState(0);
            item.setKeyEquivalentModifierMask(NSEventModifierFlags(0));
            if let Some(bytes) = entry.icon {
                let data = NSData::with_bytes(bytes);
                let image = NSImage::initWithData(NSImage::alloc(), &data)
                    .context("section menu icon cannot be decoded")?;
                image.setSize(NSSize::new(16., 16.));
                image.setTemplate(true);
                item.setImage(Some(&image));
            }
            item
        };
        menu.addItem(&item);
    }
    let logical = NSPoint::new(
        f64::from(f32::from(anchor.x).round()),
        f64::from(f32::from(anchor.y).round()),
    );
    let location = if view.isFlipped() {
        logical
    } else {
        NSPoint::new(logical.x, view.bounds().size.height - logical.y)
    };
    let actual_items: Vec<Value> = menu
        .itemArray()
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "id": items[index].id,
                "title": item.title().to_string(),
                "separator": item.isSeparatorItem(),
                "keyEquivalent": item.keyEquivalent().to_string(),
                "keyEquivalentModifierMask": item.keyEquivalentModifierMask().0,
                "enabled": item.isEnabled(),
                "state": item.state(),
                "icon": item.image().map(|image| json!({
                    "width": image.size().width, "height": image.size().height,
                    "template": image.isTemplate(),
                    "svg": items[index].icon.map(|bytes| String::from_utf8_lossy(bytes))
                }))
            })
        })
        .collect();
    menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(view));
    Ok(Outcome {
        selected: target.ivars().item.get(),
        snapshot: json!({
            "backend": "NSMenu", "opened": target.ivars().opened.get(),
            "items": actual_items, "autoenablesItems": menu.autoenablesItems(),
            "anchor": {"x": logical.x, "y": logical.y, "coordinateSpace": "window-content-top-left"},
            "positioningItem": null,
            "size": {"width": menu.size().width, "height": menu.size().height, "sizing": "native-default"},
            "window": {"id": native_window.windowNumber(), "role": "originating-app-window"}
        }),
    })
}
