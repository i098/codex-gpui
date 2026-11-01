//! `Codex ⌄` mode switch menu: the original's `ChatGPT` and `Codex` rows. `⌃1` (ChatGPT),
//! `⌃2` (Work) and `⌃3` (Codex) switch modes without the menu.

use super::HomeSidebar;
use crate::{ModeSwitch, RAIL_WIDTH, icon::icon, text};
use gpui::{
    AnyElement, Bounds, Context, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Pixels, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, point, prelude::FluentBuilder as _, px, size,
};
use theme::{ActiveTheme as _, Theme};

/// Label of each mode: ChatGPT and Work share the menu's ChatGPT row.
pub const MODES: [&str; 3] = ["ChatGPT", "Codex", "Work"];

/// The original's menu rows: label and description; row 1 is Codex.
const ROWS: [(&str, &str); 2] = [
    ("ChatGPT", "Create, learn, and explore"),
    ("Codex", "Build, debug, and ship"),
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
    /// The keyboard row; a pointer open highlights none, as Radix focuses the content.
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

    fn open_mode_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode_menu.is_some() {
            return self.close_mode_menu(window, cx);
        }
        let focus = cx.focus_handle();
        self.mode_menu = Some(ModeMenu {
            previous: window.focused(cx),
            focus: focus.clone(),
            selected: None,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// The checked [`ROWS`] index: ChatGPT for the ChatGPT surface (Chat and Work).
    fn current_row(&self) -> usize {
        if self.chatgpt_surface() { 0 } else { 1 }
    }

    /// The ChatGPT row keeps Work when it is the current mode.
    fn pick_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match row {
            0 if self.chatgpt_surface() => self.mode,
            0 => 0,
            _ => DEFAULT_MODE,
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
            current: MODES[self.mode],
            // The original's `Dns`: the Codex wordmark in Codex, else the ChatGPT one.
            wordmark: if self.chatgpt_surface() {
                ("chatgpt-wordmark", 77.13)
            } else {
                ("codex-wordmark", 53.22)
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
        let current = self.current_row();
        let rows = ROWS.into_iter().enumerate().map(|(i, (label, caption))| {
            let check = Hsla {
                a: t.text.a * 0.75,
                ..t.text
            };
            div()
                .id(("mode-option", i))
                .role(gpui::Role::MenuItemRadio)
                .aria_label(label)
                .when(i == current, |d| d.aria_toggled(gpui::Toggled::True))
                .when(menu.selected == Some(i), |d| {
                    d.aria_active_descendant().bg(t.menu_item_hover)
                })
                .px(px(8.))
                .py(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(15.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(text(label, 14., 20., 430., t.text))
                        .child(text(caption, 12., 14., 430., t.text_tertiary)),
                )
                .children((i == current).then(|| icon("menu-check", 16., check)))
                .on_hover(cx.listener(move |s, hovered: &bool, _, cx| {
                    if let Some(menu) = &mut s.mode_menu {
                        if *hovered {
                            menu.selected = Some(i);
                        } else if menu.selected == Some(i) {
                            menu.selected = None;
                        }
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |s, _, window, cx| s.pick_row(i, window, cx)))
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
                let last = ROWS.len() - 1;
                // Radix: the first Down focuses the first row, the first Up the last.
                match (e.keystroke.key.as_str(), menu.selected) {
                    ("escape", _) => s.close_mode_menu(window, cx),
                    ("up", None) => menu.selected = Some(last),
                    ("up", Some(i)) => menu.selected = Some(i.checked_sub(1).unwrap_or(last)),
                    ("down", None) => menu.selected = Some(0),
                    ("down", Some(i)) => menu.selected = Some((i + 1) % ROWS.len()),
                    ("enter" | "space", Some(i)) => s.pick_row(i, window, cx),
                    _ => return,
                }
                cx.notify();
                cx.stop_propagation();
            }));
        // `sideOffset: 4` under the trigger, plus the content's `m-px`.
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
