//! Context menus drawn in place of Electron's native ones, like the chat actions menu
//! (`header.rs`): [`ContextMenu`] holds the items, the anchor and keyboard focus for any view;
//! the transcript's link menu (`electronBridge.showContextMenu`, refs/interactions/s012.json)
//! is built on it here, the composer's editing menu in `composer.rs`. A right-click on plain
//! text opens nothing: Electron's default template is empty without a selection.
use super::*;
use gpui::{Anchor, FocusHandle, KeyDownEvent, MouseButton, Point, Role, anchored, deferred};
use std::path::PathBuf;

/// One row: Electron's `{id, label, enabled, icon?, accelerator?}` or a separator.
#[derive(Default)]
pub(crate) struct Item {
    pub id: &'static str,
    pub label: SharedString,
    pub enabled: bool,
    pub icon: Option<&'static [u8]>,
    /// Shown at the right, as the native menu shows `⌘X`.
    pub accelerator: Option<&'static str>,
    pub separator: bool,
}

impl Item {
    pub fn new(id: &'static str, label: impl Into<SharedString>, enabled: bool) -> Self {
        Self {
            id,
            label: label.into(),
            enabled,
            ..Self::default()
        }
    }

    pub fn separator(id: &'static str) -> Self {
        Self {
            id,
            separator: true,
            ..Self::default()
        }
    }
}

pub(crate) struct ContextMenu {
    pub items: Vec<Item>,
    /// What the menu is about: the link's URL, the editor's selected text.
    pub subject: String,
    /// The pointer: the menu's top-left corner.
    pub position: Point<Pixels>,
    pub active: Option<usize>,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

/// What a key did to the open menu.
pub(crate) enum Outcome {
    Moved,
    Close,
    Activate(usize),
}

impl ContextMenu {
    /// Opens at `position` and takes keyboard focus; `close` returns it.
    pub fn open(
        items: Vec<Item>,
        subject: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let return_focus = window.focused(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            items,
            subject,
            position,
            active: None,
            return_focus,
            focus,
        }
    }

    /// Returns keyboard focus to where it was.
    pub fn close(self, window: &mut Window, cx: &mut App) {
        if let Some(focus) = self.return_focus {
            window.focus(&focus, cx);
        }
    }

    /// The items as `(id, label, enabled)`.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = (&str, &str, bool)> {
        self.items
            .iter()
            .map(|item| (item.id, item.label.as_ref(), item.enabled))
    }

    /// Escape closes; Enter and Space activate; arrows, Home and End move over enabled items
    /// (wrapping); Tab is swallowed; anything else is not the menu's.
    pub fn key(&mut self, event: &KeyDownEvent) -> Option<Outcome> {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => Some(Outcome::Close),
            "enter" | "space" => self.active.map(Outcome::Activate),
            "down" | "up" | "home" | "end" => {
                let forward = matches!(key, "down" | "home");
                let n = self.items.len();
                let mut index = if matches!(key, "home" | "end") {
                    None
                } else {
                    self.active
                };
                for _ in 0..n {
                    let next = match (index, forward) {
                        (None, true) => 0,
                        (None, false) => n - 1,
                        (Some(i), true) => (i + 1) % n,
                        (Some(i), false) => (i + n - 1) % n,
                    };
                    index = Some(next);
                    if self.items[next].enabled {
                        self.active = index;
                        break;
                    }
                }
                Some(Outcome::Moved)
            }
            "tab" => Some(Outcome::Moved),
            _ => None,
        }
    }

    /// The popover at the pointer over a full-window backdrop that closes it. `menu` reads the
    /// view's open menu; `activate` runs the picked item (its id) with the subject after the
    /// menu closed.
    pub fn overlay<V: 'static>(
        &self,
        id: &'static str,
        aria_label: &'static str,
        menu: fn(&mut V) -> &mut Option<ContextMenu>,
        activate: fn(&mut V, &'static str, String, &mut Window, &mut Context<V>),
        window: &Window,
        cx: &Context<V>,
    ) -> impl IntoElement {
        let t = cx.theme();
        let hover = t.menu_item_hover;
        let close = move |this: &mut V, window: &mut Window, cx: &mut Context<V>| {
            if let Some(open) = menu(this).take() {
                open.close(window, cx);
            }
            cx.stop_propagation();
            cx.notify();
        };
        let pick = move |this: &mut V, index: usize, window: &mut Window, cx: &mut Context<V>| {
            let Some(mut open) = menu(this).take_if(|m| m.items[index].enabled) else {
                return;
            };
            let id = open.items[index].id;
            let subject = std::mem::take(&mut open.subject);
            open.close(window, cx);
            activate(this, id, subject, window, cx);
            cx.stop_propagation();
            cx.notify();
        };
        let rows = self.items.iter().enumerate().map(|(index, item)| {
            if item.separator {
                return div()
                    .h(px(9.))
                    .flex()
                    .items_center()
                    .child(div().h(px(1.)).w_full().bg(t.border))
                    .into_any_element();
            }
            let enabled = item.enabled;
            div()
                .id((id, index))
                .role(Role::MenuItem)
                .aria_label(item.label.clone())
                .aria_disabled(!enabled)
                .when(self.active == Some(index), |d| {
                    d.aria_active_descendant().bg(hover)
                })
                .h(px(28.5625))
                .px(px(8.))
                .rounded(px(12.))
                .flex()
                .items_center()
                .gap(px(6.))
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .on_hover(cx.listener(move |this, hovered, _, cx| {
                            if *hovered && let Some(open) = menu(this) {
                                open.active = Some(index);
                                cx.notify();
                            }
                        }))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| pick(this, index, window, cx)))
                .when_some(item.icon, |d, icon| {
                    let color = t.text;
                    let id = item.id;
                    d.child(
                        div().size(px(16.)).flex_none().child(
                            gpui::canvas(
                                |_, _, _| {},
                                move |bounds, (), window, cx| {
                                    if !shapes::paint_icon(
                                        icon, id, bounds, false, color, None, window, cx,
                                    ) {
                                        window
                                            .paint_svg(
                                                bounds,
                                                id.into(),
                                                Some(icon),
                                                gpui::TransformationMatrix::default(),
                                                color,
                                                cx,
                                            )
                                            .ok();
                                    }
                                },
                            )
                            .size(px(16.)),
                        ),
                    )
                })
                .child(
                    text(
                        item.label.clone(),
                        13.,
                        18.5714,
                        430.,
                        if enabled { t.text } else { t.text_tertiary },
                    )
                    .flex_1(),
                )
                .when_some(item.accelerator, |d, keys| {
                    d.child(text(keys, 13., 18.5714, 430., t.text_tertiary))
                })
                .into_any_element()
        });
        let popup = crate::composer::popover(t, 220., |d| d.children(rows))
            .ml_0()
            .pl_0()
            .id(id)
            .role(Role::Menu)
            .aria_label(aria_label)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let Some(outcome) = menu(this).as_mut().and_then(|m| m.key(event)) else {
                    return;
                };
                match outcome {
                    Outcome::Moved => {}
                    Outcome::Close => close(this, window, cx),
                    Outcome::Activate(index) => pick(this, index, window, cx),
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        deferred(
            anchored().position(gpui::point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| close(this, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, window, cx| close(this, window, cx)),
                    )
                    .child(
                        anchored()
                            .position(self.position)
                            .anchor(Anchor::TopLeft)
                            .snap_to_window_with_margin(px(8.))
                            .child(popup),
                    ),
            ),
        )
        .with_priority(4)
    }
}

/// Items as the original passes them to `showContextMenu`: id, label and 16 px icon (its
/// data url). The `Open in` heading is disabled; the separator is drawn as a rule.
const ITEMS: [(&str, &str, Option<&[u8]>); 7] = [
    ("open-in-label", "Open in", None),
    (
        "open-in-new-tab",
        "New tab",
        Some(br##"<svg xmlns="http://www.w3.org/2000/svg" height="16" viewBox="0 0 16 16" width="16"><rect fill="none" x="3" y="3.5" width="10" height="10" rx="1" stroke="currentColor" stroke-width="1.05"/></svg>"##),
    ),
    (
        "open-in-new-window",
        "New window",
        Some(br##"<svg xmlns="http://www.w3.org/2000/svg" height="16" viewBox="0 0 16 16" width="16"><path fill-rule="evenodd" clip-rule="evenodd" d="M12.001 1.43408C13.3951 1.43452 14.5254 2.56522 14.5254 3.95947V8.62646C14.5252 10.0206 13.395 11.1504 12.001 11.1509H11.1914V11.9604C11.191 13.3545 10.0611 14.4847 8.66699 14.4849H4C2.60575 14.4849 1.47505 13.3546 1.47461 11.9604V7.29346C1.47461 5.89894 2.60548 4.76807 4 4.76807H4.80859V3.95947C4.80859 2.56495 5.93947 1.43408 7.33398 1.43408H12.001ZM4 5.81885C3.18538 5.81885 2.52539 6.47884 2.52539 7.29346V11.9604C2.52583 12.7747 3.18565 13.4351 4 13.4351H8.66699C9.48119 13.4349 10.1412 12.7746 10.1416 11.9604V7.29346C10.1416 6.47895 9.48146 5.81902 8.66699 5.81885H4ZM7.33398 2.48486C6.51936 2.48486 5.85938 3.14485 5.85938 3.95947V4.76807H8.66699C10.0612 4.76807 11.1916 5.89894 11.1916 7.29346V10.1001H12.001C12.8152 10.0997 13.4752 9.43963 13.4756 8.62646V3.95947C13.4756 3.1453 12.8153 2.48523 12.001 2.48486H7.33398Z" fill="currentColor"/></svg>"##),
    ),
    (
        "open-in-external-browser",
        "Default browser",
        Some(br##"<svg xmlns="http://www.w3.org/2000/svg" height="16" viewBox="0 0 16 16" width="16"><path fill-rule="evenodd" clip-rule="evenodd" d="M8 1.47461C11.6037 1.47461 14.5254 4.39634 14.5254 8C14.5254 11.6037 11.6037 14.5254 8 14.5254C4.39634 14.5254 1.47461 11.6037 1.47461 8C1.47461 4.39634 4.39634 1.47461 8 1.47461ZM5.87012 8.52539C5.92564 9.91134 6.19693 11.1363 6.59375 12.0293C6.81716 12.532 7.06946 12.9068 7.32324 13.1484C7.57428 13.3873 7.80269 13.4745 8 13.4746C8.19723 13.4745 8.42487 13.3871 8.67578 13.1484C8.92962 12.9069 9.18182 12.532 9.40527 12.0293C9.80215 11.1363 10.0742 9.91142 10.1299 8.52539H5.87012ZM2.55078 8.52539C2.74797 10.5966 4.09925 12.3291 5.9541 13.0771C5.83897 12.8833 5.73221 12.6753 5.63477 12.4561C5.16951 11.4092 4.87546 10.0401 4.81934 8.52539H2.55078ZM11.1807 8.52539C11.1245 10.0401 10.8305 11.4092 10.3652 12.4561C10.2678 12.6753 10.161 12.8833 10.0459 13.0771C11.9008 12.3291 13.252 10.5966 13.4492 8.52539H11.1807ZM5.9541 2.92285C4.09925 3.67089 2.74797 5.40338 2.55078 7.47461H4.81934C4.87546 5.95989 5.16951 4.59083 5.63477 3.54395C5.73221 3.32468 5.83897 3.11666 5.9541 2.92285ZM8 2.52539C7.80269 2.52546 7.57428 2.61274 7.32324 2.85156C7.06946 3.09319 6.81716 3.46801 6.59375 3.9707C6.19693 4.8637 5.92564 6.08866 5.87012 7.47461H10.1299C10.0742 6.08858 9.80215 4.8637 9.40527 3.9707C9.18182 3.46801 8.92962 3.09305 8.67578 2.85156C8.42487 2.61286 8.19723 2.52552 8 2.52539ZM10.0459 2.92285C10.161 3.11666 10.2678 3.32468 10.3652 3.54395C10.8305 4.59083 11.1245 5.95989 11.1807 7.47461H13.4492C13.252 5.40338 11.9008 3.67089 10.0459 2.92285Z" fill="currentColor"/></svg>"##),
    ),
    ("external-link-separator", "", None),
    (
        "copy-link",
        "Copy link",
        Some(br##"<svg xmlns="http://www.w3.org/2000/svg" height="16" viewBox="0 0 16 16" width="16"><path d="M3.69541 6.25121C3.89761 6.04427 4.22909 6.03985 4.43662 6.24145C4.64426 6.44372 4.64953 6.77691 4.44736 6.98461L3.61338 7.84008L3.6085 7.84496C2.42952 9.02432 2.40771 10.9534 3.72666 12.2727C5.04612 13.5922 6.97593 13.5702 8.15537 12.3909L8.16025 12.386L9.01572 11.553C9.22335 11.3509 9.55562 11.3553 9.75791 11.5627C9.96017 11.7704 9.9566 12.1036 9.74912 12.3059L8.89365 13.1389C7.28179 14.7455 4.68893 14.7213 2.9835 13.0159C1.27845 11.3104 1.25498 8.71842 2.86143 7.10668L3.69541 6.25121Z" fill="currentColor"/> <path d="M9.629 5.62914C9.83403 5.42415 10.1662 5.42413 10.3712 5.62914C10.5761 5.83417 10.5761 6.16634 10.3712 6.37133L6.37119 10.3713C6.16621 10.5763 5.83403 10.5763 5.629 10.3713C5.42401 10.1663 5.42401 9.83413 5.629 9.62914L9.629 5.62914Z" fill="currentColor"/> <path d="M12.3047 2.86146C14.0098 4.56689 14.0336 7.15917 12.4277 8.77122L11.5938 9.62709C11.3917 9.83382 11.0592 9.83768 10.8516 9.63587C10.6442 9.43359 10.6399 9.10045 10.8418 8.89271L11.6758 8.03724C12.8557 6.85802 12.8773 4.92861 11.5586 3.60951C10.2392 2.29004 8.31021 2.31254 7.13086 3.49134L7.12598 3.49622L6.28418 4.31693C6.07655 4.51902 5.74428 4.51471 5.54199 4.30736C5.33969 4.09967 5.3433 3.76646 5.55078 3.5642L6.39258 2.7435C8.00433 1.13718 10.6 1.1565 12.3047 2.86146Z" fill="currentColor"/></svg>"##),
    ),
    (
        "save-link-as",
        "Save link as…",
        Some(br##"<svg xmlns="http://www.w3.org/2000/svg" height="16" viewBox="0 0 16 16" width="16"><path d="M13.3336 8.14209C13.6234 8.14227 13.858 8.37764 13.858 8.66748V11.2007C13.8578 12.6686 12.6678 13.8589 11.1998 13.8589H4.80042C3.33242 13.8589 2.1415 12.6686 2.14124 11.2007V8.66748C2.14124 8.37753 2.37668 8.14209 2.66663 8.14209C2.95658 8.14209 3.19202 8.37753 3.19202 8.66748V11.2007C3.19228 12.0887 3.91232 12.8091 4.80042 12.8091H11.1998C12.0879 12.8091 12.808 12.0887 12.8082 11.2007V8.66748C12.8082 8.37753 13.0437 8.14209 13.3336 8.14209Z" fill="currentColor"/> <path d="M7.99963 2.14111C8.28958 2.14111 8.52502 2.37655 8.52502 2.6665V8.23291L10.4625 6.29541C10.6675 6.09069 10.9997 6.09069 11.2047 6.29541C11.4093 6.50038 11.4094 6.83266 11.2047 7.0376L8.6295 9.61272C8.43145 9.81074 8.11128 9.81073 7.91323 9.61272L5.33811 7.0376L5.27444 6.96387C5.11054 6.75832 5.12338 6.4538 5.31458 6.26263C5.50573 6.07149 5.81031 6.05861 6.01586 6.22241L6.08959 6.28613L7.47424 7.67078V2.6665C7.47424 2.37655 7.70968 2.14111 7.99963 2.14111Z" fill="currentColor"/></svg>"##),
    ),
];

const HEADING: usize = 0;
const SEPARATOR: usize = 4;

thread_local! {
    /// The external link whose element saw the right button go down: its listener runs
    /// before the transcript's (bubble order), which opens the menu for it.
    static RIGHT_CLICKED_LINK: Cell<Option<String>> = const { Cell::new(None) };
}

pub(super) fn note_right_clicked_link(url: String) {
    RIGHT_CLICKED_LINK.set(Some(url));
}

impl ThreadView {
    /// The open link menu's items as `(id, label, enabled)`; empty when none is open.
    pub fn context_menu_items(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.menu.iter().flat_map(ContextMenu::rows)
    }

    /// Where the open link menu's top-left corner is anchored (the pointer).
    pub fn context_menu_at(&self) -> Option<Point<Pixels>> {
        self.menu.as_ref().map(|menu| menu.position)
    }

    pub(super) fn right_click(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(url) = RIGHT_CLICKED_LINK.take() else {
            return;
        };
        let items = ITEMS
            .iter()
            .enumerate()
            .map(|(index, &(id, label, icon))| Item {
                icon,
                ..if index == SEPARATOR {
                    Item::separator(id)
                } else {
                    Item::new(id, label, index != HEADING)
                }
            })
            .collect();
        self.menu = Some(ContextMenu::open(items, url, event.position, window, cx));
        cx.stop_propagation();
        cx.notify();
    }

    fn activate_link(&mut self, id: &str, url: String, _: &mut Window, cx: &mut Context<Self>) {
        match id {
            // ponytail: the in-app browser panel (New tab / New window) is not ported; its
            // destinations open the system browser until it is.
            "open-in-new-tab" | "open-in-new-window" | "open-in-external-browser" => {
                cx.open_url(&url)
            }
            "copy-link" => cx.write_to_clipboard(ClipboardItem::new_string(url)),
            "save-link-as" => {
                let downloads = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join("Downloads");
                let chosen = cx.prompt_for_new_path(&downloads, None);
                cx.spawn(async move |_, _| {
                    if let Ok(Ok(Some(path))) = chosen.await {
                        // ponytail: the original's download service streams with progress;
                        // curl(1) ships with macOS and fetches the link to the chosen path.
                        let _ = std::process::Command::new("curl")
                            .args(["-fsSL", "-o"])
                            .arg(path)
                            .arg(url)
                            .spawn();
                    }
                })
                .detach();
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn menu_overlay(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        self.menu.as_ref().expect("open link menu").overlay(
            "thread-link-menu",
            "Link actions",
            |this| &mut this.menu,
            Self::activate_link,
            window,
            cx,
        )
    }
}
