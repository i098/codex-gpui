//! `Codex ⌄` mode switch menu: `ChatGPT`, `Codex`, `Work` and their `⌃1` (ChatGPT), `⌃2` (Work), `⌃3` (Codex) shortcuts.

use super::HomeSidebar;
use crate::{ModeSwitch, RAIL_WIDTH, icon::icon, text};
use gpui::{
    AnyElement, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Pixels, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, point, prelude::FluentBuilder as _, px, size,
};
use theme::{ActiveTheme as _, Theme};

/// Label, caption and shortcut of each mode, in menu order.
pub const MODES: [(&str, &str, &str); 3] = [
    ("ChatGPT", "", "⌃1"),
    ("Codex", "Build, debug, and ship", "⌃3"),
    ("Work", "Create, learn, and explore", "⌃2"),
];

/// Index into [`MODES`] at launch: the original opens in Codex.
pub const DEFAULT_MODE: usize = 1;

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
    selected: usize,
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

    fn open_mode_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode_menu.is_some() {
            return self.close_mode_menu(window, cx);
        }
        let focus = cx.focus_handle();
        self.mode_menu = Some(ModeMenu {
            previous: window.focused(cx),
            focus: focus.clone(),
            selected: self.mode,
        });
        window.focus(&focus, cx);
        cx.notify();
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
            current: MODES[self.mode].0,
            // The original's `Dns`: the Codex wordmark in Codex, else the ChatGPT one.
            wordmark: if self.chatgpt_surface() {
                ("chatgpt-wordmark", 77.129955)
            } else {
                ("codex-wordmark", 53.225983)
            },
            on_click: Box::new(cx.listener(|s, _, window, cx| s.open_mode_menu(window, cx))),
            menu: self
                .mode_menu
                .as_ref()
                .map(|menu| self.mode_popup(menu, window, cx)),
        }
    }

    fn mode_popup(&self, menu: &ModeMenu, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let rows = MODES
            .into_iter()
            .enumerate()
            .map(|(i, (label, caption, key))| {
                div()
                    .id(("mode-option", i))
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(label)
                    .when(i == self.mode, |d| d.aria_toggled(gpui::Toggled::True))
                    .when(i == menu.selected, |d| d.aria_active_descendant())
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(12.))
                    .when(i == menu.selected, |d| d.bg(t.menu_item_hover))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(text(label, 14., 20., 430., t.text))
                            .when(!caption.is_empty(), |d| {
                                d.child(text(caption, 12., 16., 400., t.text_tertiary))
                            }),
                    )
                    .child(text(key, 12., 16., 400., t.text_tertiary))
                    .child(
                        div()
                            .size(px(16.))
                            .children((i == self.mode).then(|| icon("menu-check", 16., t.text))),
                    )
                    .on_hover(cx.listener(move |s, hovered: &bool, _, cx| {
                        if *hovered && let Some(menu) = &mut s.mode_menu {
                            menu.selected = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |s, _, window, cx| s.set_mode(i, window, cx)))
            });
        let popup = crate::composer::popover(&t, 260., |d| d.children(rows))
            .id("mode-menu")
            .role(gpui::Role::Menu)
            .aria_label("Switch mode")
            .track_focus(&menu.focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                let Some(menu) = &mut s.mode_menu else {
                    return;
                };
                let n = MODES.len();
                match e.keystroke.key.as_str() {
                    "escape" => s.close_mode_menu(window, cx),
                    "up" => menu.selected = (menu.selected + n - 1) % n,
                    "down" => menu.selected = (menu.selected + 1) % n,
                    "enter" | "space" => {
                        let item = menu.selected;
                        s.set_mode(item, window, cx);
                    }
                    _ => return,
                }
                cx.notify();
                cx.stop_propagation();
            }));
        let below = trigger_bounds().bottom_left() + point(px(0.), px(4.));
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
