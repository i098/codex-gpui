use super::search::{SearchEvent, SearchInput};
use super::*;

pub struct Management {
    sidebar: Entity<ScheduledSidebar>,
    search: Entity<SearchInput>,
    search_open: bool,
    filter_open: bool,
    status: &'static str,
    order: &'static str,
    utc_offset: i64,
}

impl Management {
    pub fn new(sidebar: Entity<ScheduledSidebar>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(SearchInput::new);
        let utc_offset = sidebar.read(cx).utc_offset;
        cx.observe(&sidebar, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, SearchEvent::Dismiss) {
                this.search_open = false;
            }
            cx.notify();
        })
        .detach();
        Self {
            sidebar,
            search,
            search_open: false,
            filter_open: false,
            status: "active",
            order: "default",
            utc_offset,
        }
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = !self.search_open;
        if self.search_open {
            use gpui::Focusable as _;
            self.search.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn filters(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        let statuses = [
            ("All", "all"),
            ("Active", "active"),
            ("Paused", "paused"),
            ("Completed", "finished"),
        ];
        let orders = [
            ("Default", "default"),
            ("Next run", "next"),
            ("Recently updated", "recent"),
            ("Name", "name"),
        ];
        let row = |label: &'static str, value: &'static str, status: bool| {
            let selected = if status {
                self.status == value
            } else {
                self.order == value
            };
            div()
                .id(label)
                .role(Role::MenuItem)
                .h(px(28.5625))
                .px(px(8.))
                .rounded(px(15.))
                .flex()
                .items_center()
                .gap(px(8.))
                .hover(|s| s.bg(t.menu_item_hover))
                .child(
                    div()
                        .w(px(16.))
                        .when(selected, |d| d.child(icon("thread-check", 16., t.text))),
                )
                .child(text(label, 13., 18.5714, 430., t.text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if status {
                        this.status = value;
                    } else {
                        this.order = value;
                    }
                    this.filter_open = false;
                    cx.notify();
                }))
        };
        deferred(
            div().absolute().top(px(36.)).right_0().child(
                anchored().anchor(Anchor::TopRight).child(
                    div()
                        .id("management-filter-menu")
                        .role(Role::Menu)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.filter_open = false;
                            cx.notify();
                        }))
                        .child(crate::composer::popover(t, 220., |d| {
                            d.child(
                                text("Status", 12., 18., 500., t.text_secondary)
                                    .px(px(8.))
                                    .py(px(4.)),
                            )
                            .children(statuses.map(|(label, value)| row(label, value, true)))
                            .child(div().my(px(4.)).border_t_1().border_color(t.border))
                            .child(
                                text("Sort by", 12., 18., 500., t.text_secondary)
                                    .px(px(8.))
                                    .py(px(4.)),
                            )
                            .children(orders.map(|(label, value)| row(label, value, false)))
                        })),
                ),
            ),
        )
        .with_priority(1)
    }

    fn task_row(
        &self,
        task: &Task,
        selected: bool,
        unread: bool,
        t: &Theme,
    ) -> gpui::Stateful<Div> {
        let id = task.id.clone();
        let key_id = id.clone();
        let next = task
            .next_run_at
            .and_then(|value| chrono::DateTime::from_timestamp(value / 1000 + self.utc_offset, 0));
        let next = next.map_or_else(
            || "—".to_owned(),
            |value| value.format("%b %-d, %Y, %-I:%M %p").to_string(),
        );
        div()
            .id(SharedString::from(format!("managed-task-{}", task.id)))
            .role(Role::Button)
            .py(px(4.))
            .border_b_1()
            .border_color(if t.appearance == theme::Appearance::Dark {
                gpui::Hsla {
                    a: 0.043,
                    ..t.border_subtle
                }
            } else {
                t.border_subtle
            })
            .child(
                div()
                    .relative()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(15.))
                    .when(selected, |d| {
                        d.child(shapes::decoration(shapes::Decoration {
                            radius: gpui::Corners::all(px(15.)),
                            corner: shapes::Corner::Round,
                            fill: Some(t.text.opacity(0.05)),
                            ..Default::default()
                        }))
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(t.list_hover)))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(text(task.name.clone(), 13., 18.5714, 430., t.text))
                            .when(unread, |d| {
                                d.child(
                                    div()
                                        .size(px(20.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .size(px(8.))
                                                .rounded_full()
                                                .bg(gpui::rgb(0x3a83f7)),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(text(next, 12., 16., 430., t.text_secondary).flex_none())
                            .child(text("·", 12., 16., 430., t.text_secondary).flex_none())
                            .child(
                                text(task.schedule(), 12., 16., 430., t.text_secondary)
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis(),
                            ),
                    ),
            )
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(OpenAutomationManagement {
                        id: Some(id.clone()),
                    }),
                    cx,
                )
            })
            .on_key_down(move |event, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    window.dispatch_action(
                        Box::new(OpenAutomationManagement {
                            id: Some(key_id.clone()),
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                }
            })
    }
}

impl Render for Management {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let sidebar = self.sidebar.read(cx);
        let selected = sidebar.selected.clone();
        let task = selected
            .as_ref()
            .and_then(|id| sidebar.tasks.iter().find(|task| &task.id == id))
            .cloned();
        let history = sidebar.history.clone();
        let is_empty = sidebar.tasks.is_empty();
        let query = self.search.read(cx).value.to_lowercase();
        let mut tasks: Vec<_> = sidebar
            .tasks
            .iter()
            .filter(|task| {
                let status = match self.status {
                    "all" => true,
                    "paused" => task.status == "PAUSED",
                    "finished" => false,
                    _ => task.status == "ACTIVE",
                };
                status
                    && (query.is_empty()
                        || task.name.to_lowercase().contains(&query)
                        || task.prompt.to_lowercase().contains(&query))
            })
            .collect();
        match self.order {
            "name" => tasks.sort_by(|a, b| a.name.cmp(&b.name)),
            "next" => tasks.sort_by_key(|task| task.next_run_at.unwrap_or(i64::MAX)),
            "recent" => tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at)),
            _ => tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at)),
        }
        let unread = history.as_ref().is_some_and(|history| {
            history
                .read(cx)
                .items()
                .iter()
                .any(|run| run["readAt"].is_null())
        });
        let rows: Vec<_> = tasks
            .into_iter()
            .map(|task| {
                (
                    format!("managed-task-{}", task.id),
                    self.task_row(task, selected.as_deref() == Some(&task.id), unread, &t),
                )
            })
            .collect();
        let rows: Vec<_> = rows
            .into_iter()
            .map(|(id, row)| {
                let focus = crate::focus::handle(id, 0, window, cx);
                crate::focus::ring(row, &focus, px(15.), px(2.), cx)
            })
            .collect();
        let search_focus = crate::focus::handle("management-search", 0, window, cx);
        let filter_focus = crate::focus::handle("management-filter", 0, window, cx);
        let add_focus = crate::focus::handle("management-add", 0, window, cx);
        let mut list = div()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .px(px(30.))
            .pt(px(30.))
            .bg(t.surface);
        list = if task.is_some() {
            list.w(px(352.)).flex_none()
        } else {
            list.flex_1()
        };
        list = list
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        text("Scheduled", 24., 28.8, 500., t.text)
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .flex_none(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                div()
                                    .relative()
                                    .flex_none()
                                    .child(crate::focus::ring(
                                        icon_button(
                                            "management-filter",
                                            icon("automations-filter", 16., t.text_tertiary),
                                            &t,
                                        )
                                        .size(px(32.))
                                        .rounded_full()
                                        .role(Role::Button)
                                        .aria_label("Filter and sort tasks")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.filter_open = !this.filter_open;
                                                cx.notify();
                                            }),
                                        ),
                                        &filter_focus,
                                        px(16.),
                                        px(2.),
                                        cx,
                                    ))
                                    .when(self.filter_open, |d| d.child(self.filters(&t, cx))),
                            )
                            .child(crate::focus::ring(
                                div()
                                    .id("management-search")
                                    .role(Role::Button)
                                    .aria_label("Search scheduled tasks")
                                    .relative()
                                    .h(px(32.))
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(13.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: gpui::Corners::all(px(16.)),
                                        corner: shapes::Corner::Round,
                                        border: Some((px(1.), t.text.opacity(0.157))),
                                        ..Default::default()
                                    }))
                                    .child(icon(
                                        "search",
                                        16.,
                                        if t.appearance == theme::Appearance::Dark {
                                            t.text.opacity(0.65)
                                        } else {
                                            t.text_secondary
                                        },
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_search(window, cx)
                                    })),
                                &search_focus,
                                px(16.),
                                px(2.),
                                cx,
                            ))
                            .child(crate::focus::ring(
                                div()
                                    .id("management-add")
                                    .role(Role::Button)
                                    .aria_label("Add task")
                                    .relative()
                                    .h(px(32.))
                                    .w(px(78.679_69))
                                    .flex_none()
                                    .rounded_full()
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: gpui::Corners::all(px(16.)),
                                        corner: shapes::Corner::Round,
                                        fill: Some(t.text),
                                        ..Default::default()
                                    }))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        div()
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .child(
                                                icon("automations-plus", 16., t.menu_background)
                                                    .relative()
                                                    .left(px(-1.)),
                                            )
                                            .child(text("Add", 14., 14., 400., t.menu_background)),
                                    )
                                    .on_click(crate::dispatch::<NewTask>),
                                &add_focus,
                                px(16.),
                                px(2.),
                                cx,
                            )),
                    ),
            )
            .when(self.search_open, |d| {
                d.child(div().mt(px(12.)).child(self.search.clone()))
            })
            .child(
                div()
                    .id("management-task-list")
                    .mt(px(15.))
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(is_empty, |d| {
                        d.child(text(
                            "No scheduled tasks",
                            13.,
                            18.5714,
                            430.,
                            t.text_secondary,
                        ))
                    }),
            );
        let mut content = div().size_full().flex().bg(t.surface).child(list);
        if let (Some(task), Some(history)) = (task, history) {
            let close_focus = crate::focus::handle("management-close", 0, window, cx);
            let edit_focus = crate::focus::handle("management-edit", 0, window, cx);
            let edit_id = task.id.clone();
            let actions = self.sidebar.update(cx, |sidebar, cx| {
                sidebar.header_actions(&task, &t, window, cx)
            });
            content = content.child(
                div()
                    .h_full()
                    .min_w_0()
                    .flex_1()
                    .border_l_1()
                    .border_color(t.border)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(73.))
                            .flex_none()
                            .border_b_1()
                            .border_color(t.border)
                            .px(px(24.))
                            .py(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(
                                        div()
                                            .size(px(32.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(icon("automation-calendar", 16., t.text)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                text(task.name.clone(), 16., 28., 500., t.text)
                                                    .overflow_hidden()
                                                    .text_ellipsis(),
                                            )
                                            .child(text(
                                                task.schedule(),
                                                13.,
                                                20.,
                                                430.,
                                                t.text_tertiary,
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(actions)
                                    .child(crate::focus::ring(
                                        icon_button(
                                            "management-edit",
                                            icon("pencil", 20., t.text_tertiary),
                                            &t,
                                        )
                                        .size(px(28.))
                                        .role(Role::Button)
                                        .aria_label("Edit task")
                                        .on_click(
                                            move |_, window, cx| {
                                                window.dispatch_action(
                                                    Box::new(EditAutomation {
                                                        id: edit_id.clone(),
                                                    }),
                                                    cx,
                                                )
                                            },
                                        ),
                                        &edit_focus,
                                        px(12.5),
                                        px(2.),
                                        cx,
                                    ))
                                    .child(crate::focus::ring(
                                        icon_button(
                                            "management-close",
                                            icon("close", 20., t.text_tertiary),
                                            &t,
                                        )
                                        .size(px(28.))
                                        .role(Role::Button)
                                        .aria_label("Close task details")
                                        .on_click(
                                            |_, window, cx| {
                                                window.dispatch_action(
                                                    Box::new(OpenAutomationManagement { id: None }),
                                                    cx,
                                                )
                                            },
                                        ),
                                        &close_focus,
                                        px(12.5),
                                        px(2.),
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .px(px(16.))
                            .py(px(24.))
                            .overflow_hidden()
                            .child(history),
                    ),
            );
        }
        content
    }
}
