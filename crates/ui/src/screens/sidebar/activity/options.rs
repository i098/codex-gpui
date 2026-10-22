use super::super::HomeSidebar;
use super::{ArchiveActivity, OptionsMenu};
use crate::{icon::icon, text};
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, StatefulInteractiveElement, Styled, Window, anchored, deferred,
    div, point, prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;

impl HomeSidebar {
    pub(super) fn open_activity_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        self.activity.menu = Some(OptionsMenu {
            previous: window.focused(cx),
            focus: focus.clone(),
            selected: 0,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    fn close_activity_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.activity.menu.take()
            && let Some(previous) = menu.previous
        {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    fn activity_option_enabled(&self, item: usize) -> bool {
        match item {
            3 => self
                .unread
                .iter()
                .any(|id| self.activity.items.contains(id)),
            4 => {
                self.activity.archive_supported
                    && self.threads.iter().any(|(id, _, _)| {
                        self.activity.items.contains(id) && self.activity_visible(id)
                    })
            }
            _ => true,
        }
    }

    fn select_activity_option(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.activity_option_enabled(item) {
            return;
        }
        match item {
            0 => self.activity.priority = !self.activity.priority,
            1 => self.activity.pinned = !self.activity.pinned,
            2 => self.activity.scheduled = !self.activity.scheduled,
            3 => self.unread.retain(|id| !self.activity.items.contains(id)),
            4 => {
                let ids = self
                    .threads
                    .iter()
                    .filter(|(id, _, _)| {
                        self.activity.items.contains(id) && self.activity_visible(id)
                    })
                    .map(|(id, _, _)| id.clone())
                    .collect();
                window.dispatch_action(Box::new(ArchiveActivity(ids)), cx);
            }
            _ => return,
        }
        self.close_activity_options(window, cx);
    }

    pub(in crate::screens::sidebar) fn activity_menu_overlay(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.activity.menu.as_ref()?;
        let checked = [
            self.activity.priority,
            self.activity.pinned,
            self.activity.scheduled,
        ];
        let rows =
            [
                "Priority section",
                "Pinned",
                "Scheduled",
                "Mark all as read",
                "Archive chats",
            ]
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                let enabled = self.activity_option_enabled(i);
                div()
                    .when(i == 3, |d| {
                        d.child(
                            div()
                                .h(px(9.))
                                .border_t_1()
                                .border_color(cx.theme().text_tertiary.opacity(0.2)),
                        )
                    })
                    .child(
                        div()
                            .id(("activity-option", i))
                            .role(gpui::Role::MenuItem)
                            .aria_label(label)
                            .aria_disabled(!enabled)
                            .when_some(checked.get(i), |d, checked| {
                                d.role(gpui::Role::CheckBox).aria_toggled(if *checked {
                                    gpui::Toggled::True
                                } else {
                                    gpui::Toggled::False
                                })
                            })
                            .when(i == menu.selected, |d| d.aria_active_descendant())
                            .h(px(30.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(6.))
                            .opacity(if enabled { 1. } else { 0.5 })
                            .when(enabled && i == menu.selected, |d| {
                                d.bg(cx.theme().ghost_hover)
                            })
                            .child(
                                div().size(px(16.)).children(
                                    checked.get(i).copied().unwrap_or(false).then(|| {
                                        icon("menu-check", 16., cx.theme().text_default_nav)
                                    }),
                                ),
                            )
                            .child(text(label, 14., 20., 430., cx.theme().text_default_nav))
                            .on_hover(cx.listener(move |s, hovered: &bool, _, cx| {
                                if *hovered && enabled {
                                    if let Some(menu) = &mut s.activity.menu {
                                        menu.selected = i;
                                    }
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |s, _, window, cx| {
                                s.select_activity_option(i, window, cx)
                            })),
                    )
            });
        let popup = crate::composer::popover(cx.theme(), 240., |d| {
            d.child(text("Show", 12., 20., 500., cx.theme().text_tertiary).px(px(8.)))
                .children(rows)
        })
        .ml_0()
        .pl_0()
        .id("activity-options-popup")
        .role(gpui::Role::Menu)
        .aria_label("Activity view options")
        .track_focus(&menu.focus)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
            let Some(menu) = &mut s.activity.menu else {
                return;
            };
            match e.keystroke.key.as_str() {
                "escape" => s.close_activity_options(window, cx),
                "up" => {
                    menu.selected = (menu.selected + 4) % 5;
                    cx.notify();
                }
                "down" => {
                    menu.selected = (menu.selected + 1) % 5;
                    cx.notify();
                }
                "enter" | "space" => {
                    let item = menu.selected;
                    s.select_activity_option(item, window, cx);
                }
                _ => return,
            }
            cx.stop_propagation();
        }));
        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .w(window.viewport_size().width)
                        .h(window.viewport_size().height)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|s, _, window, cx| s.close_activity_options(window, cx)),
                        )
                        .child(
                            anchored()
                                .position(point(
                                    self.activity.button.origin.x,
                                    self.activity.button.bottom() + px(4.),
                                ))
                                .anchor(gpui::Anchor::TopLeft)
                                .snap_to_window_with_margin(px(8.))
                                .child(popup),
                        ),
                ),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }
}
