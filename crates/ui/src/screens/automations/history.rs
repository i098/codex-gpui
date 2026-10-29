use crate::{icon::icon, icon_button, text};
use gpui::{
    Action, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use serde::Deserialize;
use theme::ActiveTheme as _;

#[derive(Clone, PartialEq)]
pub enum Operation {
    Load,
    MarkRead,
    Archive,
    View(String),
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct HistoryAction(pub Entity<History>, pub Operation);

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub thread_id: String,
    pub title: Option<String>,
    pub status: String,
    pub read_at: Option<i64>,
    pub created_at: i64,
}

impl Run {
    pub fn actionable(&self) -> bool {
        matches!(self.status.as_str(), "PENDING_REVIEW" | "ACCEPTED")
    }
}

pub struct History {
    pub automation_id: String,
    pub runs: Vec<Run>,
    pub busy: bool,
    utc_offset: i64,
    error: Option<String>,
    menu: bool,
    scroll: UniformListScrollHandle,
}

impl History {
    pub fn new(automation_id: String, utc_offset: i64) -> Self {
        Self {
            automation_id,
            utc_offset,
            runs: Vec::new(),
            busy: false,
            error: None,
            menu: false,
            scroll: UniformListScrollHandle::new(),
        }
    }

    pub fn loaded(&mut self, result: anyhow::Result<serde_json::Value>, cx: &mut Context<Self>) {
        match result
            .and_then(|value| Ok(serde_json::from_value::<Vec<Run>>(value["items"].clone())?))
        {
            Ok(mut runs) => {
                runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
                self.runs = runs;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        self.busy = false;
        cx.notify();
    }

    fn run_row(
        &self,
        index: usize,
        run: &Run,
        t: &theme::Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let id = run.thread_id.clone();
        let active = run.status != "ARCHIVED";
        let title: SharedString = run
            .title
            .clone()
            .unwrap_or_else(|| "Scheduled run".into())
            .into();
        let marker = if run.status == "IN_PROGRESS" {
            "Running"
        } else if run.status == "ARCHIVED" {
            "Archived"
        } else if run.read_at.is_none() {
            "Unread"
        } else {
            "Read"
        };
        let indicator = div()
            .w(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .child(if run.status == "IN_PROGRESS" {
                icon("sidebar-cmd-spinner", 12., t.text_tertiary).into_any_element()
            } else if run.status == "ARCHIVED" {
                icon("sidebar-archive", 12., t.text_tertiary).into_any_element()
            } else {
                div()
                    .size(px(8.))
                    .rounded_full()
                    .bg(if run.read_at.is_none() {
                        t.text_link
                    } else {
                        t.text_tertiary
                    })
                    .into_any_element()
            });
        let (date, _) = super::day_time(run.created_at, self.utc_offset);
        div()
            .id(("scheduled-run", index))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .h(px(40.))
            .w_full()
            .py(px(6.))
            .child(indicator)
            .child(
                history_button(
                    ("scheduled-run-view", index),
                    title,
                    active,
                    Operation::View(id),
                    cx,
                )
                .flex_1()
                .aria_label(format!(
                    "{}: {marker}",
                    run.title.as_deref().unwrap_or("Scheduled run")
                )),
            )
            .child(text(date, 12., 16., 400., t.text_tertiary))
    }

    fn dispatch(&mut self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = false;
        window.dispatch_action(Box::new(HistoryAction(cx.entity(), operation)), cx);
        cx.notify();
    }
}

impl Render for History {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.runs.is_empty() && self.error.is_none() {
            return div();
        }
        let t = cx.theme().clone();
        let unread = !self.busy
            && self
                .runs
                .iter()
                .any(|run| run.actionable() && run.read_at.is_none());
        let archiveable = !self.busy && self.runs.iter().any(Run::actionable);
        let page = div()
            .relative()
            .mt(px(24.))
            .border_t_1()
            .border_color(t.border)
            .pt(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.));
        let heading = div()
            .flex()
            .items_center()
            .justify_between()
            .child(text("Previous runs", 14., 20., 500., t.text_tertiary))
            .child(
                icon_button(
                    "previous-run-actions",
                    icon("thread-chat-actions", 16., t.text_tertiary),
                    &t,
                )
                .role(gpui::Role::Button)
                .aria_label("Previous run actions")
                .tab_index(0)
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.menu = !this.menu;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.menu = !this.menu;
                    cx.notify();
                })),
            );
        let mut page = page.child(heading);
        if self.menu {
            page = page.child(gpui::deferred(
                div()
                    .id("previous-run-menu")
                    .absolute()
                    .right_0()
                    .top(px(44.))
                    .w(px(180.))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .p(px(4.))
                    .occlude()
                    .flex()
                    .flex_col()
                    .role(gpui::Role::Menu)
                    .aria_label("Previous run actions")
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.menu = false;
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            this.menu = false;
                            cx.notify();
                        }
                    }))
                    .child(
                        history_button(
                            "mark-runs-read",
                            "Mark all as read",
                            unread,
                            Operation::MarkRead,
                            cx,
                        )
                        .role(gpui::Role::MenuItem),
                    )
                    .child(
                        history_button(
                            "archive-runs",
                            "Archive all",
                            archiveable,
                            Operation::Archive,
                            cx,
                        )
                        .role(gpui::Role::MenuItem),
                    ),
            ));
        }
        if self.error.is_some() {
            page = page
                .child(text(
                    "Could not load previous runs",
                    13.,
                    20.,
                    400.,
                    t.text_tertiary,
                ))
                .child(history_button(
                    "retry-runs",
                    "Retry",
                    !self.busy,
                    Operation::Load,
                    cx,
                ));
        }
        let entity = cx.entity();
        page = page.child(
            uniform_list("previous-runs", self.runs.len(), move |range, _, cx| {
                entity.update(cx, |this, cx| {
                    range
                        .map(|index| this.run_row(index, &this.runs[index], &t, cx))
                        .collect()
                })
            })
            .h(px(self.runs.len().min(8) as f32 * 40.))
            .w_full()
            .track_scroll(&self.scroll),
        );
        page
    }
}

fn history_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    operation: Operation,
    cx: &mut Context<History>,
) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    let key_operation = operation.clone();
    let focus_color = cx.theme().border_focus;
    div()
        .id(id)
        .tab_index(if enabled { 0 } else { -1 })
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .aria_disabled(!enabled)
        .focus_visible(move |style| style.border_2().border_color(focus_color))
        .cursor_pointer()
        .opacity(if enabled { 1. } else { 0.4 })
        .child(text(label, 13., 20., 400., cx.theme().text))
        .when(enabled, |button| {
            button
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(operation.clone(), window, cx)
                }))
                .on_key_down(
                    cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.dispatch(key_operation.clone(), window, cx);
                        }
                    }),
                )
        })
}
