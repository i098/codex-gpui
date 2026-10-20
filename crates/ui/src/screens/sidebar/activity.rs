use super::*;
use crate::screens::thread::view::Clock;
use chrono::{DateTime, Days, NaiveDate};

pub struct ActivityOpened;
impl EventEmitter<ActivityOpened> for HomeSidebar {}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ArchiveActivity(pub Vec<String>);

struct OptionsMenu {
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    selected: usize,
}

pub(super) struct State {
    priority: bool,
    pinned: bool,
    scheduled: bool,
    archive_supported: bool,
    items: HashSet<String>,
    scheduled_ids: HashSet<String>,
    menu: Option<OptionsMenu>,
    button: Bounds<Pixels>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            priority: true,
            pinned: false,
            scheduled: false,
            archive_supported: true,
            items: HashSet::new(),
            scheduled_ids: HashSet::new(),
            menu: None,
            button: Bounds::default(),
        }
    }
}

impl HomeSidebar {
    pub fn toggle_activity(&mut self, clock: Clock, cx: &mut Context<Self>) {
        self.activity_clock = if self.activity_clock.is_some() {
            None
        } else {
            Some(clock)
        };
        self.activity.items = self
            .threads
            .iter()
            .filter(|(id, _, _)| self.needs_attention(id))
            .map(|(id, _, _)| id.clone())
            .collect();
        self.activity.menu = None;
        self.menu = None;
        self.limit = PAGE;
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.activity_clock.is_some() {
            cx.emit(ActivityOpened);
        }
        cx.notify();
    }

    pub fn set_activity_archive_supported(&mut self, supported: bool) {
        self.activity.archive_supported = supported;
    }

    pub fn set_scheduled_threads(&mut self, ids: HashSet<String>, cx: &mut Context<Self>) {
        self.activity.scheduled_ids = ids;
        cx.notify();
    }

    pub(super) fn note_activity(&mut self, id: &str) {
        if self.activity_clock.is_some() {
            self.activity.items.insert(id.to_owned());
        }
    }

    fn needs_attention(&self, id: &str) -> bool {
        self.unread.iter().any(|unread| unread == id) || self.running.contains_key(id)
    }

    fn activity_visible(&self, id: &str) -> bool {
        self.activity.scheduled || !self.activity.scheduled_ids.contains(id)
    }

    fn in_priority(&self, id: &str) -> bool {
        self.activity.priority
            && self.activity.items.contains(id)
            && (!self.activity.pinned || !self.pinned.contains(id))
    }

    fn in_recent(&self, id: &str, at: i64, oldest: NaiveDate, offset: i64) -> bool {
        self.activity_visible(id)
            && !self.in_priority(id)
            && (!self.activity.pinned || !self.pinned.contains(id))
            && (day(at, offset) >= oldest || self.activity.items.contains(id))
    }

    pub(super) fn activity_window(&self) -> Option<(usize, bool)> {
        let clock = self.activity_clock?;
        let oldest = day(clock.now(), clock.utc_offset) - Days::new(6);
        let recent = self
            .threads
            .iter()
            .filter(|(id, _, at)| self.in_recent(id, *at, oldest, clock.utc_offset))
            .count();
        let has_more = self.has_more
            && self
                .threads
                .iter()
                .all(|(_, _, at)| day(*at, clock.utc_offset) >= oldest);
        Some((recent, has_more))
    }

    pub(super) fn activity_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let clock = self.activity_clock.expect("activity view");
        let today = day(clock.now(), clock.utc_offset);
        let oldest = today - Days::new(6);
        let pinned_visible = self.activity.pinned
            && self
                .threads
                .iter()
                .any(|(id, _, _)| self.pinned.contains(id) && self.activity_visible(id));
        // Sticky activity contents have their own transform layer in the original.
        let mut list = div().px(px(8.)).flex().flex_col().top_layer();
        if self.activity.priority {
            list = list.child(self.activity_heading("Priority", true, window, cx));
            let priority: Vec<_> = self
                .threads
                .iter()
                .filter(|(id, _, _)| self.activity_visible(id) && self.in_priority(id))
                .collect();
            if priority.is_empty() {
                list = list.child(match self.threads_state {
                    ListState::Loading => {
                        sidebar_empty("Checking unread chats…", cx).into_any_element()
                    }
                    ListState::Failed => load_error::<RetryThreads>(
                        "Unread chats couldn't be loaded",
                        "retry-activity",
                        window,
                        cx,
                    ),
                    ListState::Ready => {
                        sidebar_empty("Nothing needs attention", cx).into_any_element()
                    }
                });
            } else {
                list = list.child(
                    div().flex().flex_col().gap(px(1.)).children(
                        priority
                            .into_iter()
                            .map(|(id, label, _)| self.chat_row(id, label.clone(), window, cx)),
                    ),
                );
            }
        }
        if pinned_visible {
            list = list
                .child(div().h(px(16.)))
                .child(self.activity_heading("Pinned", !self.activity.priority, window, cx))
                .child(
                    div().flex().flex_col().gap(px(1.)).children(
                        self.threads
                            .iter()
                            .filter(|(id, _, _)| {
                                self.pinned.contains(id) && self.activity_visible(id)
                            })
                            .map(|(id, label, _)| self.chat_row(id, label.clone(), window, cx)),
                    ),
                );
        }
        let mut recent: Vec<_> = self
            .threads
            .iter()
            .filter(|(id, _, at)| self.in_recent(id, *at, oldest, clock.utc_offset))
            .collect();
        recent.sort_by_key(|(_, _, at)| std::cmp::Reverse(*at));
        let mut previous = None;
        for (id, label, at) in recent.into_iter().take(self.limit) {
            let date = day(*at, clock.utc_offset);
            if previous != Some(date) {
                let title = if date == today {
                    "Today".to_owned()
                } else if date == today - Days::new(1) {
                    "Yesterday".to_owned()
                } else {
                    date.format("%A").to_string()
                };
                if self.activity.priority || pinned_visible || previous.is_some() {
                    list = list.child(div().h(px(16.)));
                }
                list = list.child(self.activity_heading(
                    title,
                    !self.activity.priority && !pinned_visible && previous.is_none(),
                    window,
                    cx,
                ));
                previous = Some(date);
            }
            list = list.child(self.chat_row(id, label.clone(), window, cx).mb(px(1.)));
        }
        if !self.activity.priority && previous.is_none() && !pinned_visible {
            list = list.child(self.activity_heading("Today", true, window, cx));
        }
        list.children(self.page_tail(cx)).into_any_element()
    }

    fn activity_heading(
        &self,
        label: impl Into<SharedString>,
        options: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let owner = cx.weak_entity();
        let focus = focus::handle("activity-view-options", 0, window, cx);
        let hover = cx.theme().ghost_hover;
        div()
            .h(px(30.))
            .pl(px(8.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .top_layer()
            .child(text(label, 14., 21., 500., cx.theme().text_tertiary).flex_1())
            .when(options, |d| {
                d.child(focus::ring(
                    div()
                        .id("activity-view-options")
                        .role(gpui::Role::Button)
                        .aria_label("Activity view options")
                        .size(px(24.))
                        .relative()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .hover(move |s| s.bg(hover))
                        .child(icon("diff-more", 16., cx.theme().text_tertiary))
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, (), _, cx| {
                                    owner.update(cx, |s, _| s.activity.button = bounds).ok();
                                },
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .on_click(
                            cx.listener(|s, _, window, cx| s.open_activity_options(window, cx)),
                        ),
                    &focus,
                    px(8.),
                    px(0.),
                    cx,
                ))
            })
            .when(
                options
                    && self
                        .activity
                        .items
                        .iter()
                        .any(|id| !self.needs_attention(id)),
                |d| {
                    let clear = focus::handle("clear-read-chats", 0, window, cx);
                    d.child(focus::ring(
                        div()
                            .id("clear-read-chats")
                            .role(gpui::Role::Button)
                            .aria_label("Clear read chats")
                            .size(px(24.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(move |s| s.bg(hover))
                            .child(icon("composer-reset", 16., cx.theme().text_tertiary))
                            .on_click(cx.listener(|s, _, _, cx| {
                                let unread = &s.unread;
                                let running = &s.running;
                                s.activity
                                    .items
                                    .retain(|id| unread.contains(id) || running.contains_key(id));
                                s.limit = PAGE;
                                cx.notify();
                            })),
                        &clear,
                        px(8.),
                        px(0.),
                        cx,
                    ))
                },
            )
    }

    fn open_activity_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn activity_menu_overlay(
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

fn day(at: i64, offset: i64) -> NaiveDate {
    DateTime::from_timestamp(at + offset, 0)
        .unwrap_or_default()
        .date_naive()
}
