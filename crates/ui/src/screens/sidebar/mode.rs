//! `Codex ⌄` mode switch menu: the original's two items, `ChatGPT` and `Codex`. The ChatGPT
//! surface has two homes, Chat (`⌃1`) and Work (`⌃2`); Codex is `⌃3`.

use super::HomeSidebar;
use crate::{ModeSwitch, RAIL_WIDTH, icon::icon, text};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, point, prelude::FluentBuilder as _, px, size,
};
use theme::{ActiveTheme as _, Theme};

/// Mode names by index: the ChatGPT surface's Chat home, Codex, and its Work home.
pub const MODES: [&str; 3] = ["ChatGPT", "Codex", "Work"];

/// Index into [`MODES`] at launch: the original opens in Codex.
pub const DEFAULT_MODE: usize = 1;

/// The menu items in order: label, caption and the [`MODES`] index a pick selects.
const ITEMS: [(&str, &str, usize); 2] = [
    ("ChatGPT", "Create, learn, and explore", 0),
    ("Codex", "Build, debug, and ship", DEFAULT_MODE),
];

/// The 84x32 `Codex ⌄` trigger, 10 px right of the rail and 10 px under the titlebar.
fn trigger_bounds() -> Bounds<Pixels> {
    Bounds::new(
        point(RAIL_WIDTH + px(10.), Theme::TITLEBAR_HEIGHT + px(10.)),
        size(px(84.22), px(32.)),
    )
}

pub(super) struct ModeMenu {
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    /// The highlighted [`ITEMS`] row; none after a pointer open, like the original's Radix menu.
    selected: Option<usize>,
}

impl HomeSidebar {
    pub fn mode(&self) -> usize {
        self.mode
    }

    pub fn mode_menu_open(&self) -> bool {
        self.mode_menu.is_some()
    }

    /// `⌃1` `⌃2` `⌃3` and menu picks: select a mode and close the menu.
    pub fn set_mode(&mut self, mode: usize, window: &mut Window, cx: &mut Context<Self>) {
        if mode >= MODES.len() {
            return;
        }
        if self.mode != mode {
            self.mode = mode;
            cx.emit(super::ModeChanged(mode));
        }
        self.close_mode_menu(window, cx);
    }

    fn open_mode_menu(&mut self, keyboard: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode_menu.is_some() {
            return self.close_mode_menu(window, cx);
        }
        let focus = cx.focus_handle();
        self.mode_menu = Some(ModeMenu {
            previous: window.focused(cx),
            focus: focus.clone(),
            selected: keyboard.then_some(0),
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// The [`ITEMS`] row of the current mode: ChatGPT for both of its homes.
    fn current_item(&self) -> usize {
        usize::from(!self.chatgpt_surface())
    }

    /// Picking the current item keeps the current home, like the original's no-op reselect.
    fn pick(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if item == self.current_item() {
            self.mode
        } else {
            ITEMS[item].2
        };
        self.set_mode(mode, window, cx);
    }

    fn close_mode_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.mode_menu.take()
            && let Some(previous) = menu.previous
        {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    pub(super) fn mode_switch(&self, window: &Window, cx: &mut Context<Self>) -> ModeSwitch {
        ModeSwitch {
            current: ITEMS[self.current_item()].0,
            // The original's `Dns`: the Codex wordmark in Codex, else the ChatGPT one.
            wordmark: if self.chatgpt_surface() {
                ("chatgpt-wordmark", 77.13)
            } else {
                ("codex-wordmark", 53.22)
            },
            on_click: Box::new(cx.listener(|s, e: &ClickEvent, window, cx| {
                s.open_mode_menu(e.is_keyboard(), window, cx)
            })),
            menu: self
                .mode_menu
                .as_ref()
                .map(|menu| self.mode_popup(menu, window, cx)),
        }
    }

    fn mode_popup(&self, menu: &ModeMenu, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let current = self.current_item();
        // `Pm.Item` with `py-2.5 text-base`: 232x54, the check at 75% until hovered.
        let rows = ITEMS
            .into_iter()
            .enumerate()
            .map(|(i, (label, caption, _))| {
                let hot = menu.selected == Some(i);
                div()
                    .id(("mode-option", i))
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(label)
                    .when(i == current, |d| d.aria_toggled(gpui::Toggled::True))
                    .when(hot, |d| d.aria_active_descendant())
                    .px(px(8.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(15.))
                    .when(hot, |d| d.bg(t.menu_item_hover))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(text(label, 14., 20., 430., t.text).font_family("OpenAI Sans"))
                            .child(text(caption, 12., 14., 430., t.text_tertiary)),
                    )
                    .when(i == current, |d| {
                        let alpha = if hot { 1. } else { 0.75 };
                        d.child(icon("menu-check", 16., t.text.opacity(alpha)))
                    })
                    .on_hover(cx.listener(move |s, hovered: &bool, _, cx| {
                        if *hovered && let Some(menu) = &mut s.mode_menu {
                            menu.selected = Some(i);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |s, _, window, cx| s.pick(i, window, cx)))
            });
        let popup = crate::composer::popover(&t, 240., |d| d.children(rows))
            .id("mode-menu")
            .role(gpui::Role::Menu)
            .aria_label("Switch mode")
            .track_focus(&menu.focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                let Some(menu) = &mut s.mode_menu else {
                    return;
                };
                let n = ITEMS.len();
                // With no highlight, Radix starts at the first row (down) or the last (up).
                match (e.keystroke.key.as_str(), menu.selected) {
                    ("escape", _) => s.close_mode_menu(window, cx),
                    ("up", Some(i)) => menu.selected = Some((i + n - 1) % n),
                    ("up", None) => menu.selected = Some(n - 1),
                    ("down", Some(i)) => menu.selected = Some((i + 1) % n),
                    ("down", None) => menu.selected = Some(0),
                    ("enter" | "space", Some(i)) => s.pick(i, window, cx),
                    _ => return,
                }
                cx.notify();
                cx.stop_propagation();
            }));
        // `sideOffset: 4` plus the panel's `m-px`.
        let below = trigger_bounds().bottom_left() + point(px(1.), px(5.));
        // A full-window backdrop under the popup: a click anywhere else (the trigger too) closes.
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|s, _, window, cx| s.close_mode_menu(window, cx)),
                    )
                    .child(
                        anchored()
                            .position(below)
                            .anchor(gpui::Anchor::TopLeft)
                            .snap_to_window_with_margin(px(8.))
                            .child(popup),
                    ),
            ),
        )
        .with_priority(4)
        .into_any_element()
    }
}
