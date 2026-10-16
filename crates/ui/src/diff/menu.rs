use super::*;
use gpui::{Anchor, Point, Role, anchored, point};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Header,
    Body,
}

pub(super) struct Menu {
    file: usize,
    kind: Kind,
    position: Point<Pixels>,
    active: Option<usize>,
    selected: String,
    return_focus: Option<FocusHandle>,
}

impl DiffPanel {
    pub fn menu_items(
        &self,
    ) -> impl ExactSizeIterator<Item = (&'static str, bool)> + Clone + use<> {
        let (items, count) = match &self.menu {
            None => ([("", false); 4], 0),
            Some(menu) if menu.kind == Kind::Header => (
                [
                    ("Copy path", true),
                    ("Open in editor", false),
                    (
                        if self.files[menu.file].open {
                            "Collapse file"
                        } else {
                            "Expand file"
                        },
                        true,
                    ),
                    ("", false),
                ],
                3,
            ),
            Some(menu) => (
                [
                    ("Copy selection", !menu.selected.is_empty()),
                    ("Copy path", true),
                    ("Copy relative path", true),
                    ("Toggle line wrap", true),
                ],
                4,
            ),
        };
        items.into_iter().take(count)
    }

    pub fn wraps_lines(&self) -> bool {
        self.wrap
    }

    pub(super) fn open_menu(
        &mut self,
        file: usize,
        kind: Kind,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selecting = false;
        let mut selected = self.selected_text();
        // The original native menu trims selection; keyboard Copy preserves it.
        selected.truncate(selected.trim_end().len());
        selected.drain(..selected.len() - selected.trim_start().len());
        self.menu = Some(Menu {
            file,
            kind,
            position,
            active: None,
            selected,
            return_focus: window.focused(cx),
        });
        if let Some(focus) = &self.focus {
            window.focus(focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self.menu_items();
        let Some(menu) = &mut self.menu else { return };
        match event.keystroke.key.as_str() {
            "escape" => self.close_menu(window, cx),
            "enter" | "space" => {
                if let Some(index) = menu.active {
                    self.activate_menu(index, window, cx);
                }
            }
            "down" | "up" | "home" | "end" => {
                let forward = matches!(event.keystroke.key.as_str(), "down" | "home");
                let mut index = match event.keystroke.key.as_str() {
                    "home" | "end" => None,
                    _ => menu.active,
                };
                for _ in 0..items.len() {
                    let next = match index {
                        None => {
                            if forward {
                                0
                            } else {
                                items.len() - 1
                            }
                        }
                        Some(i) => {
                            if forward {
                                (i + 1) % items.len()
                            } else {
                                (i + items.len() - 1) % items.len()
                            }
                        }
                    };
                    index = Some(next);
                    if items.clone().nth(next).is_some_and(|item| item.1) {
                        menu.active = index;
                        break;
                    }
                }
            }
            "tab" => {}
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take()
            && let Some(focus) = menu.return_focus
        {
            window.focus(&focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn activate_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((label, true)) = self.menu_items().nth(index) else {
            return;
        };
        let Some(menu) = self.menu.take() else { return };
        if let Some(focus) = menu.return_focus {
            window.focus(&focus, cx);
        }
        let file = &mut self.files[menu.file];
        match label {
            "Copy selection" => cx.write_to_clipboard(ClipboardItem::new_string(menu.selected)),
            "Copy path" | "Copy relative path" => {
                let relative = format!("{}{}", file.dir, file.name);
                let path = if menu.kind == Kind::Header || label == "Copy relative path" {
                    relative
                } else {
                    absolute_path(&self.workspace, &relative)
                };
                cx.write_to_clipboard(ClipboardItem::new_string(path));
            }
            "Collapse file" | "Expand file" => {
                file.open = !file.open;
                self.selection = None;
            }
            "Toggle line wrap" => self.wrap = !self.wrap,
            _ => unreachable!(),
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn menu_overlay(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let menu = self.menu.as_ref().expect("open diff menu");
        let t = cx.theme();
        let hover = t.menu_item_hover;
        let rows = self.menu_items().enumerate().map(|(i, (label, enabled))| {
            div()
                .id(("diff-menu-item", i))
                .role(Role::MenuItem)
                .aria_label(label)
                .aria_disabled(!enabled)
                .when(menu.active == Some(i), |d| {
                    d.aria_active_descendant().bg(hover)
                })
                .h(px(29.))
                .px(px(8.))
                .rounded(px(12.))
                .flex()
                .items_center()
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .on_hover(cx.listener(move |this, hovered, _, cx| {
                            if *hovered && let Some(menu) = &mut this.menu {
                                menu.active = Some(i);
                                cx.notify();
                            }
                        }))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.activate_menu(i, window, cx)))
                .child(text(
                    label,
                    13.,
                    20.,
                    400.,
                    if enabled { t.text } else { t.text_tertiary },
                ))
        });
        let popup = crate::composer::popover(t, 220., |d| d.children(rows))
            .ml_0()
            .pl_0()
            .id("diff-menu")
            .role(Role::Menu)
            .aria_label("File actions")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                    )
                    .child(
                        anchored()
                            .position(menu.position)
                            .anchor(if menu.kind == Kind::Header {
                                Anchor::TopRight
                            } else {
                                Anchor::TopLeft
                            })
                            .snap_to_window_with_margin(px(8.))
                            .child(popup),
                    ),
            ),
        )
        .with_priority(4)
    }
}

fn absolute_path(workspace: &str, relative: &str) -> String {
    let mut path = PathBuf::new();
    for component in Path::new(workspace).join(relative).components() {
        match component {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            other => path.push(other.as_os_str()),
        }
    }
    path.to_string_lossy().into_owned()
}
