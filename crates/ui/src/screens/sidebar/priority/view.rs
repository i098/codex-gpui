use super::super::{HomeSidebar, ListState, PAGE, RetryActivity, RetryThreads, load_error};
use super::local_date;
use crate::{focus, icon::icon, sidebar_empty, text};
use chrono::Days;
use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, canvas, div, prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;

impl HomeSidebar {
    fn pending_activity(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = match self.activity_sources_state {
            ListState::Ready => return None,
            ListState::Loading => sidebar_empty("Checking unread chats…", cx).into_any_element(),
            ListState::Failed => load_error::<RetryActivity>(
                "Unread chats couldn't be loaded",
                "retry-activity-sources",
                window,
                cx,
            ),
        };
        Some(
            div()
                .px(px(8.))
                .flex()
                .flex_col()
                .top_layer()
                .child(self.activity_heading(
                    if self.activity_priority {
                        "Priority"
                    } else {
                        "Today"
                    },
                    true,
                    window,
                    cx,
                ))
                .child(notice)
                .into_any_element(),
        )
    }

    pub(in crate::screens::sidebar) fn activity_list(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(pending) = self.pending_activity(window, cx) {
            return pending;
        }
        let pinned_visible = self.activity_pinned
            && self
                .threads
                .iter()
                .any(|(id, _, _)| self.pinned.contains(id) && self.activity_visible(id));
        // Sticky activity contents have their own transform layer in the original.
        let mut list = div().px(px(8.)).flex().flex_col().top_layer();
        if self.activity_priority {
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
                            .map(|(id, label, _)| self.chat_row(id, label.clone(), 0., window, cx)),
                    ),
                );
            }
        }
        if pinned_visible {
            list = list
                .child(div().h(px(16.)))
                .child(self.activity_heading("Pinned", !self.activity_priority, window, cx))
                .child(
                    div().flex().flex_col().gap(px(1.)).children(
                        self.threads
                            .iter()
                            .filter(|(id, _, _)| {
                                self.pinned.contains(id) && self.activity_visible(id)
                            })
                            .map(|(id, label, _)| self.chat_row(id, label.clone(), 0., window, cx)),
                    ),
                );
        }
        self.activity_recents(list, pinned_visible, window, cx)
            .children(self.page_tail(cx))
            .into_any_element()
    }

    fn activity_recents(
        &self,
        mut list: Div,
        pinned_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let clock = self.activity_clock.expect("activity view");
        let today = local_date(clock.now(), clock.utc_offset);
        let oldest = local_date(self.activity_started_at, clock.utc_offset) - Days::new(6);
        let mut recent: Vec<_> = self
            .threads
            .iter()
            .filter(|(id, _, at)| self.in_recent(id, *at, oldest, clock.utc_offset))
            .collect();
        recent.sort_by_key(|(id, _, at)| std::cmp::Reverse(self.activity_updated_at(id, *at)));
        let mut previous = None;
        for (id, label, at) in recent.into_iter().take(self.limit) {
            let date = local_date(self.activity_updated_at(id, *at), clock.utc_offset);
            if previous != Some(date) {
                let title = if date == today {
                    "Today".to_owned()
                } else if date == today - Days::new(1) {
                    "Yesterday".to_owned()
                } else {
                    date.format("%A").to_string()
                };
                if self.activity_priority || pinned_visible || previous.is_some() {
                    list = list.child(div().h(px(16.)));
                }
                list = list.child(self.activity_heading(
                    title,
                    !self.activity_priority && !pinned_visible && previous.is_none(),
                    window,
                    cx,
                ));
                previous = Some(date);
            }
            list = list.child(self.chat_row(id, label.clone(), 0., window, cx).mb(px(1.)));
        }
        if !self.activity_priority && previous.is_none() && !pinned_visible {
            list = list.child(self.activity_heading("Today", true, window, cx));
        }
        list
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
                                    owner.update(cx, |s, _| s.activity_button = bounds).ok();
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
                        .activity_items
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
                                s.activity_items
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
}
