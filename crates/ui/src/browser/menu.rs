//! The page context menu (`browserSidebar.contextMenu.*`): a native menu at a right-click
//! in the page, as the original's Electron `Menu.popup`. Editable text and selections keep
//! WebKit's own menu.

use gpui::{ClipboardItem, Context, Point, Window, px};
use theme::{ActiveTheme as _, Appearance};
use url::Url;

use super::{BrowserTab, OpenBrowserTab};
use crate::screens::sidebar::menu::{Item, native};

const fn item(id: &'static str, label: &'static str) -> Item {
    Item {
        id,
        label,
        accelerator: None,
        icon: None,
        submenu: &[],
    }
}

/// The original's order; a click off a link shows the items after [`LINK_ITEMS`].
static ITEMS: [Item; 8] = [
    item("open-link-in-new-tab", "Open in new tab"),
    item("open-link-in-external-browser", "Open in external browser"),
    item("link-separator", ""),
    item("copy-link", "Copy link address"),
    item("copy-separator", ""),
    item("back", "Back"),
    item("forward", "Forward"),
    item("reload", "Reload"),
];
const LINK_ITEMS: usize = 5;

impl BrowserTab {
    /// Shows the menu for a right-click at `(x, y)` in the page, over `link` when set.
    pub(super) fn context_menu(
        &mut self,
        x: f32,
        y: f32,
        link: Option<Url>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = if link.is_some() {
            &ITEMS[..]
        } else {
            &ITEMS[LINK_ITEMS..]
        };
        let enabled: Vec<bool> = items
            .iter()
            .map(|item| match item.id {
                "back" => self.can_go_back(),
                "forward" => self.can_go_forward(),
                "reload" => self.url().is_some(),
                _ => true,
            })
            .collect();
        let origin = self.page_bounds.origin;
        let at = Point::new(origin.x + px(x), origin.y + px(y));
        let dark = cx.theme().appearance == Appearance::Dark;
        let tab = cx.weak_entity();
        // AppKit tracks the menu in a nested event loop; release the tab borrow first.
        window.defer(cx, move |window, cx| {
            let selected = match native::popup(items, &enabled, at, dark, window) {
                Ok(outcome) => outcome.selected,
                Err(error) => {
                    eprintln!("codex-gpui: browser context menu: {error:#}");
                    None
                }
            };
            let Some(id) = selected.map(|index| items[index].id) else {
                return;
            };
            let link = link.map(String::from);
            match (id, link) {
                ("open-link-in-new-tab", link @ Some(_)) => {
                    window.dispatch_action(Box::new(OpenBrowserTab(link)), cx)
                }
                ("open-link-in-external-browser", Some(link)) => cx.open_url(&link),
                ("copy-link", Some(link)) => cx.write_to_clipboard(ClipboardItem::new_string(link)),
                (id, _) => {
                    let _ = tab.update(cx, |tab, cx| match id {
                        "back" => tab.go_back(cx),
                        "forward" => tab.go_forward(cx),
                        "reload" => tab.reload(false, cx),
                        _ => {}
                    });
                }
            }
        });
    }
}
