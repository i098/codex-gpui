use super::*;
use crate::screens::sidebar::OpenThread;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct MarkAllRunsRead {
    pub id: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct ArchiveAllRuns {
    pub id: String,
}

pub struct RunHistory {
    pub id: String,
    items: Vec<Value>,
    loading: bool,
    error: bool,
    busy: bool,
    menu: bool,
    selected: usize,
    focus: FocusHandle,
}

impl gpui::EventEmitter<OpenThread> for RunHistory {}

impl gpui::Focusable for RunHistory {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn archiveable(item: &Value) -> bool {
    item["threadId"].as_str().is_some()
        && !matches!(item["status"].as_str(), Some("ARCHIVED" | "IN_PROGRESS"))
}

impl RunHistory {
    pub fn new(id: String, cx: &mut Context<Self>) -> Self {
        Self {
            id,
            items: Vec::new(),
            loading: true,
            error: false,
            busy: false,
            menu: false,
            selected: 0,
            focus: cx.focus_handle(),
        }
    }

    pub fn items(&self) -> &[Value] {
        &self.items
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.items.clear();
        self.loading = true;
        self.error = false;
        cx.notify();
    }

    pub fn set_items(&mut self, result: Result<Vec<Value>, String>, cx: &mut Context<Self>) {
        self.loading = false;
        self.error = result.is_err();
        if let Ok(items) = result {
            self.items = items;
        }
        cx.notify();
    }

    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        cx.notify();
    }

    fn enabled(&self, item: usize) -> bool {
        !self.loading
            && !self.error
            && !self.busy
            && if item == 0 {
                self.items.iter().any(|run| run["readAt"].is_null())
            } else {
                self.items.iter().any(archiveable)
            }
    }

    fn activate(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(item) {
            return;
        }
        self.menu = false;
        if item == 0 {
            window.dispatch_action(
                Box::new(MarkAllRunsRead {
                    id: self.id.clone(),
                }),
                cx,
            );
        } else {
            window.dispatch_action(
                Box::new(ArchiveAllRuns {
                    id: self.id.clone(),
                }),
                cx,
            );
        }
        cx.notify();
    }
}

impl Render for RunHistory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let actions_focus = focus::handle("previous-run-actions", 0, window, cx);
        self.focus = actions_focus.read(cx).handle.clone();
        let rows = self
            .items
            .iter()
            .enumerate()
            .map(|(index, run)| {
                let id = run["threadId"].as_str().map(str::to_owned);
                let row_focus = focus::handle(
                    SharedString::from(format!("previous-run-{}", id.as_deref().unwrap_or(""))),
                    0,
                    window,
                    cx,
                );
                let title = run["title"].as_str().unwrap_or("Scheduled run").to_owned();
                let row = div()
                    .id(("previous-run", index))
                    .role(Role::Button)
                    .aria_label(title.clone())
                    .relative()
                    .py(px(10.))
                    .px(px(12.))
                    .rounded(px(10.))
                    .hover(|s| s.bg(t.list_hover))
                    .flex()
                    .items_center()
                    .child(text(title, 14., 20., 430., t.text))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        if let Some(id) = &id {
                            cx.emit(OpenThread(id.clone()));
                        }
                    }));
                focus::ring(row, &row_focus, px(10.), px(2.), cx)
            })
            .collect::<Vec<_>>();
        div()
            .mt(px(24.))
            .border_t_1()
            .border_color(t.border)
            .pt(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Previous runs", 14., 20., 500., t.text_secondary))
                    .child(
                        div()
                            .relative()
                            .child(focus::ring(
                                icon_button(
                                    "previous-run-actions",
                                    icon("thread-chat-actions", 16., t.text_secondary),
                                    &t,
                                )
                                .role(Role::Button)
                                .aria_label("Previous run actions")
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        match event.keystroke.key.as_str() {
                                            "escape" => this.menu = false,
                                            "down" | "up" => {
                                                if this.menu {
                                                    this.selected = 1 - this.selected;
                                                }
                                                this.menu = true;
                                            }
                                            "enter" | "space" => {
                                                if this.menu {
                                                    this.activate(this.selected, window, cx);
                                                } else {
                                                    this.menu = true;
                                                    this.selected = 0;
                                                }
                                            }
                                            _ => return,
                                        }
                                        cx.stop_propagation();
                                        cx.notify();
                                    },
                                ))
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.menu = !this.menu;
                                        this.focus.focus(window, cx);
                                        cx.notify();
                                    },
                                )),
                                &actions_focus,
                                px(8.),
                                px(2.),
                                cx,
                            ))
                            .when(self.menu, |d| {
                                d.child(deferred(
                                    anchored().anchor(Anchor::TopRight).child(
                                        div()
                                            .id("previous-run-menu")
                                            .role(Role::Menu)
                                            .top_layer()
                                            .snap_origin()
                                            .w(px(224.))
                                            .p(px(4.))
                                            .rounded(px(12.))
                                            .bg(t.menu_background)
                                            .border_1()
                                            .border_color(t.border)
                                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                                this.menu = false;
                                                cx.notify();
                                            }))
                                            .children(
                                                ["Mark all as read", "Archive all"]
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(index, label)| {
                                                        let enabled = self.enabled(index);
                                                        div()
                                                            .id(("previous-run-menu-item", index))
                                                            .role(Role::MenuItem)
                                                            .aria_label(label)
                                                            .aria_disabled(!enabled)
                                                            .h(px(30.))
                                                            .px(px(8.))
                                                            .rounded(px(8.))
                                                            .flex()
                                                            .items_center()
                                                            .gap(px(8.))
                                                            .opacity(if enabled { 1. } else { 0.4 })
                                                            .when(
                                                                self.selected == index && enabled,
                                                                |d| d.bg(t.list_hover),
                                                            )
                                                            .child(icon(
                                                                if index == 0 {
                                                                    "thread-check"
                                                                } else {
                                                                    "sidebar-archive"
                                                                },
                                                                16.,
                                                                t.text,
                                                            ))
                                                            .child(text(
                                                                label, 13., 18., 430., t.text,
                                                            ))
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.activate(index, window, cx)
                                                                },
                                                            ))
                                                    }),
                                            ),
                                    ),
                                ))
                            }),
                    ),
            )
            .when(self.loading, |d| {
                d.child(text(
                    "Loading previous runs…",
                    13.,
                    20.,
                    400.,
                    t.text_tertiary,
                ))
            })
            .when(self.error, |d| {
                d.child(text(
                    "Could not load previous runs",
                    13.,
                    20.,
                    400.,
                    t.text_error,
                ))
            })
            .children(rows)
    }
}
