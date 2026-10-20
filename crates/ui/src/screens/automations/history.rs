use super::*;
use crate::screens::sidebar::OpenThread;
use crate::screens::thread::view::Clock;

static AGE_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));

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

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct SetRunRead {
    pub automation: String,
    pub id: String,
    pub is_read: bool,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct ArchiveRun {
    pub automation: String,
    pub thread: String,
}

pub struct RunHistory {
    pub id: String,
    items: Vec<Value>,
    loading: bool,
    error: bool,
    busy: bool,
    menu: bool,
    menu_run: Option<usize>,
    selected: Option<usize>,
    focus: FocusHandle,
    latest_focus: FocusHandle,
    clock: Clock,
    latest_read_attempt: Option<String>,
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
            menu_run: None,
            selected: None,
            focus: cx.focus_handle(),
            latest_focus: cx.focus_handle(),
            latest_read_attempt: None,
            clock: Clock {
                utc_offset: 0,
                pinned_now: None,
                elapsed_since: None,
            },
        }
    }

    pub fn items(&self) -> &[Value] {
        &self.items
    }

    pub fn take_latest_unread(&mut self) -> Option<String> {
        let run = self.items.first()?;
        let id = run["id"].as_str()?;
        if self.latest_read_attempt.as_deref() == Some(id) {
            return None;
        }
        self.latest_read_attempt = Some(id.to_owned());
        run["readAt"].is_null().then(|| id.to_owned())
    }

    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    pub fn set_items(&mut self, result: Result<Vec<Value>, String>, cx: &mut Context<Self>) {
        self.loading = false;
        self.error = result.is_err();
        if let Ok(mut items) = result {
            items.sort_by_key(|run| {
                std::cmp::Reverse(run["createdAt"].as_i64().unwrap_or_default())
            });
            self.items = items;
        }
        cx.notify();
    }

    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        cx.notify();
    }

    fn enabled(&self, item: usize) -> bool {
        if self.loading || self.error || self.busy {
            return false;
        }
        match self.menu_run.and_then(|index| self.items.get(index)) {
            Some(run) => item == 0 || archiveable(run),
            None if item == 0 => self.items.iter().any(|run| run["readAt"].is_null()),
            None => self.items.iter().any(archiveable),
        }
    }

    fn activate(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(item) {
            return;
        }
        self.menu = false;
        match self.menu_run.and_then(|index| self.items.get(index)) {
            Some(run) if item == 0 => {
                if let Some(id) = run["id"].as_str() {
                    window.dispatch_action(
                        Box::new(SetRunRead {
                            automation: self.id.clone(),
                            id: id.to_owned(),
                            is_read: run["readAt"].is_null(),
                        }),
                        cx,
                    );
                }
            }
            Some(run) => {
                if let Some(thread) = run["threadId"].as_str() {
                    window.dispatch_action(
                        Box::new(ArchiveRun {
                            automation: self.id.clone(),
                            thread: thread.to_owned(),
                        }),
                        cx,
                    );
                }
            }
            None if item == 0 => window.dispatch_action(
                Box::new(MarkAllRunsRead {
                    id: self.id.clone(),
                }),
                cx,
            ),
            None => window.dispatch_action(
                Box::new(ArchiveAllRuns {
                    id: self.id.clone(),
                }),
                cx,
            ),
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.menu = false,
            "down" | "up" => {
                self.selected = Some(
                    self.selected
                        .map_or(usize::from(event.keystroke.key == "up"), |index| 1 - index),
                );
                self.menu = true;
            }
            "enter" | "space" => {
                if self.menu {
                    self.activate(self.selected.unwrap_or(0), window, cx);
                } else {
                    self.menu = true;
                    self.selected = Some(0);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn age(&self, run: &Value) -> String {
        let seconds =
            (self.clock.now() - run["createdAt"].as_i64().unwrap_or_default() / 1000).max(0);
        match seconds {
            0..60 => "now".into(),
            60..3600 => format!("{}m", seconds / 60),
            3600..86400 => format!("{}h", seconds / 3600),
            86400..604800 => format!("{}d", seconds / 86400),
            604800..2592000 => format!("{}w", seconds / 604800),
            2592000..31536000 => format!("{}mo", seconds / 2592000),
            _ => format!("{}y", seconds / 31536000),
        }
    }

    fn title(run: &Value) -> &str {
        run["displayTitle"]
            .as_str()
            .or_else(|| run["description"].as_str())
            .or_else(|| run["title"].as_str())
            .unwrap_or("Untitled")
    }

    fn actions(
        &self,
        run: Option<usize>,
        label: &'static str,
        t: &Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let open = self.menu && self.menu_run == run;
        let focus = if run.is_some() {
            self.latest_focus.clone()
        } else {
            self.focus.clone()
        };
        div()
            .relative()
            .child(
                icon_button(label, icon("thread-chat-actions", 16., t.text_tertiary), t)
                    .size(px(24.))
                    .role(Role::Button)
                    .aria_label(label)
                    .track_focus(&focus)
                    .on_key_down(cx.listener(move |this, event, window, cx| {
                        this.menu_run = run;
                        this.key(event, window, cx);
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = !(this.menu && this.menu_run == run);
                        this.menu_run = run;
                        this.selected = None;
                        focus.focus(window, cx);
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                d.child(
                    deferred(
                        div().absolute().top(px(26.)).right(px(1.)).child(
                            anchored()
                                .anchor(Anchor::TopRight)
                                .snap_to_window_with_margin(px(7.))
                                .child(
                                    div()
                                        .id("previous-run-menu")
                                        .role(Role::Menu)
                                        .occlude()
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.menu = false;
                                            cx.notify();
                                        }))
                                        .child(
                                            crate::composer::popover(t, 160., |d| {
                                                let labels = match run
                                                    .and_then(|index| self.items.get(index))
                                                {
                                                    Some(item) => [
                                                        if item["readAt"].is_null() {
                                                            "Mark as read"
                                                        } else {
                                                            "Mark as unread"
                                                        },
                                                        "Archive",
                                                    ],
                                                    None => ["Mark all as read", "Archive all"],
                                                };
                                                d.children(labels.into_iter().enumerate().map(
                                                    |(index, label)| {
                                                        let enabled = self.enabled(index);
                                                        div()
                                                            .id(("previous-run-menu-item", index))
                                                            .role(Role::MenuItem)
                                                            .aria_label(label)
                                                            .aria_disabled(!enabled)
                                                            .h(px(28.570_313))
                                                            .px(px(8.))
                                                            .rounded(px(15.))
                                                            .flex()
                                                            .items_center()
                                                            .gap(px(6.))
                                                            .opacity(if enabled { 1. } else { 0.4 })
                                                            .when(
                                                                self.selected == Some(index)
                                                                    && enabled,
                                                                |d| d.bg(t.menu_item_hover),
                                                            )
                                                            .when(enabled, |d| {
                                                                d.hover(|s| s.bg(t.menu_item_hover))
                                                            })
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
                                                                label, 13., 18.5714, 430., t.text,
                                                            ))
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.activate(index, window, cx)
                                                                },
                                                            ))
                                                    },
                                                ))
                                            })
                                            .top_layer()
                                            .snap_origin(),
                                        ),
                                ),
                        ),
                    )
                    .with_priority(1),
                )
            })
    }

    fn latest(&self, run: &Value, t: &Theme, cx: &Context<Self>) -> Div {
        let thread = run["threadId"].as_str().map(str::to_owned);
        let summary = run["description"]
            .as_str()
            .or_else(|| run["archivedAssistantMessage"].as_str())
            .unwrap_or_else(|| Self::title(run));
        div()
            .px(px(12.))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Last run", 13., 18.5714, 430., t.text_secondary))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(text(self.age(run), 13., 18.5714, 430., t.text_secondary))
                            .child(self.actions(Some(0), "Run actions", t, cx)),
                    ),
            )
            .child(
                text(summary.to_owned(), 13., 18.5714, 430., t.text)
                    .mt(px(8.))
                    .whitespace_normal(),
            )
            .child(
                div()
                    .id("open-latest-run")
                    .role(Role::Button)
                    .mt(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(text("Open chat", 13., 18.5714, 430., t.text_tertiary))
                    .child(icon("arrow-up-right", 12., t.text_tertiary))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        if let Some(thread) = &thread {
                            cx.emit(OpenThread(thread.clone()));
                        }
                    })),
            )
    }

    fn row(
        &self,
        index: usize,
        run: &Value,
        t: &Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let thread = run["threadId"].as_str().map(str::to_owned);
        let title = Self::title(run).to_owned();
        let source = run["sourceCwd"]
            .as_str()
            .and_then(|cwd| Path::new(cwd).file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let dot = if run["readAt"].is_null() {
            Some(if run["status"] == "ARCHIVED" {
                t.text_error
            } else {
                t.text_accent
            })
        } else {
            None
        };
        div()
            .id(("previous-run", index))
            .role(Role::Button)
            .relative()
            .ml(px(-12.))
            .pl(px(24.))
            .pr(px(12.))
            .py(px(8.))
            .rounded(px(10.))
            .hover(|s| s.bg(t.list_hover))
            .flex()
            .items_center()
            .gap(px(8.))
            .children(dot.map(|color| {
                div()
                    .absolute()
                    .left(px(6.))
                    .top(px(13.285156))
                    .size(px(8.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(px(4.)),
                        corner: shapes::Corner::Round,
                        fill: Some(color),
                        ..Default::default()
                    }))
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        text(title, 13., 16.25, 400., t.text)
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis(),
                    )
                    .children(source.map(|source| {
                        text(source, 13., 18.5714, 430., t.text_tertiary)
                            .max_w(px(96.))
                            .overflow_hidden()
                            .text_ellipsis()
                    })),
            )
            .child(
                div().w(px(72.)).flex_none().flex().justify_end().child(
                    text(self.age(run), 13., 18.5714, 430., t.text_tertiary)
                        .font_features(AGE_FEATURES.clone()),
                ),
            )
            .on_click(cx.listener(move |_, _, _, cx| {
                if let Some(thread) = &thread {
                    cx.emit(OpenThread(thread.clone()));
                }
            }))
    }
}

impl Render for RunHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .when(self.loading, |d| {
                d.child(text("Loading runs…", 13., 18.5714, 430., t.text_secondary))
            })
            .when(self.error, |d| {
                d.child(text(
                    "Could not load previous runs",
                    13.,
                    18.5714,
                    430.,
                    t.text_error,
                ))
            })
            .children(self.items.first().map(|run| self.latest(run, t, cx)))
            .when(self.items.len() > 1, |d| {
                d.child(
                    div()
                        .border_t_1()
                        .border_color(t.border)
                        .pt(px(24.))
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(
                            div()
                                .h(px(24.))
                                .px(px(12.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    text("Previous runs", 13., 18.5714, 500., t.text_secondary)
                                        .layer()
                                        .opacity(0.75),
                                )
                                .child(self.actions(None, "Previous run actions", t, cx)),
                        )
                        .child(
                            div().flex().flex_col().children(
                                self.items
                                    .iter()
                                    .enumerate()
                                    .skip(1)
                                    .map(|(index, run)| self.row(index, run, t, cx)),
                            ),
                        ),
                )
            })
    }
}
