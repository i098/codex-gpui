//! The `Cloud` sidebar group (the original's `cloudTaskRow` in its cloud connection group):
//! the account's current cloud tasks from `/wham/tasks/list`, each with its status; a click
//! opens the task at `/remote/:taskId`.
use super::*;

/// One `/wham/tasks/list` item as the row shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudTaskRow {
    pub id: String,
    pub title: SharedString,
    /// The latest turn is still `in_progress` or `pending`.
    pub running: bool,
    pub unread: bool,
}

/// Emitted when a Cloud row is clicked, with the task id.
pub struct OpenCloudTask(pub String);

impl EventEmitter<OpenCloudTask> for HomeSidebar {}

impl HomeSidebar {
    pub fn set_cloud_tasks(&mut self, tasks: Vec<CloudTaskRow>, cx: &mut Context<Self>) {
        self.cloud_tasks = tasks;
        cx.notify();
    }

    pub fn cloud_tasks(&self) -> &[CloudTaskRow] {
        &self.cloud_tasks
    }

    pub(super) fn cloud_rows(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<_> = self
            .cloud_tasks
            .iter()
            .map(|task| cloud_row(task, window, cx))
            .collect();
        div().flex().flex_col().children(rows).into_any_element()
    }
}

/// A 30 px row like Recents: hover fill, the title clipped with the right fade, and at the
/// right a spinner while the task runs or the unread dot.
fn cloud_row(
    task: &CloudTaskRow,
    window: &mut Window,
    cx: &mut Context<HomeSidebar>,
) -> gpui::Stateful<gpui::Div> {
    let id = SharedString::from(format!("cloud-task-{}", task.id));
    let handle = focus::handle(id.clone(), 0, window, cx);
    let t = cx.theme();
    let status = if task.running {
        Some(icon("sidebar-cmd-spinner", 14., t.text_tertiary).into_any_element())
    } else if task.unread {
        Some(
            div()
                .size(px(6.))
                .rounded_full()
                .bg(t.info_solid)
                .into_any_element(),
        )
    } else {
        None
    };
    let row = div()
        .id(id.clone())
        .debug_selector(|| format!("cloud-task-{}", task.id))
        .group(id.clone())
        .role(gpui::Role::Link)
        .aria_label(task.title.clone())
        .relative()
        .h(px(30.))
        .child(
            div()
                .absolute()
                .inset_0()
                .opacity(0.)
                .group_hover(id.clone(), |s| s.opacity(1.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(RADIUS_ROW),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.ghost_hover),
                    ..Default::default()
                })),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .right(px(if status.is_some() { 28. } else { 0. }))
                .top(px(5.))
                .h(px(20.))
                .pl(px(8.))
                .overflow_hidden()
                .layer()
                .child(text(task.title.clone(), 14., 20., 430., t.text_default_nav))
                .child(shapes::x_fade(px(8.), px(16.))),
        )
        .children(status.map(|status| {
            div()
                .absolute()
                .right(px(8.))
                .top_0()
                .h(px(30.))
                .w(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .child(status)
        }));
    let task_id = task.id.clone();
    focus::ring(row, &handle, RADIUS_ROW, px(2.), cx)
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenCloudTask(task_id.clone()))))
}
