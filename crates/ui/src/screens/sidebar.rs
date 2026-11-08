//! `sidebar` parity screen (Home with chat search open) and the Home sidebar every chat route
//! shows: header, Projects, and Recents from `thread/list`. Numbers are CSS px from
//! refs/home and refs/sidebar.

use crate::composer::ChatDrag;
use crate::{
    Shell, fixture, focus, home_sidebar_header,
    icon::icon,
    sidebar_empty, text,
    tooltip::{Side, tooltip},
};
use bridge::types::{Thread, ThreadListResponse, ThreadStatus};
use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Bounds, ClipboardItem, Context, Corners,
    DispatchPhase, Div, ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    Modifiers, MouseButton, ParentElement, Pixels, Point, Render, ScrollHandle, ShapedLine,
    SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun,
    Transformation, UTF16Selection, Window, anchored, canvas, deferred, div, fill, point,
    prelude::FluentBuilder as _, px, radians, relative, rgba, size, svg,
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ops::Range,
    path::Path,
    time::{Duration, Instant},
};
use theme::{ActiveTheme as _, Appearance, Theme};

#[path = "sidebar/menu.rs"]
pub(crate) mod menu;
use menu::ThreadMenu;
mod chatgpt;
mod cloud;
pub use cloud::{CloudTaskRow, OpenCloudTask};
mod mode;
mod priority;
pub use chatgpt::{ChatGptChat, ModeChanged};
pub use mode::{DEFAULT_MODE, MODES};
pub use priority::{ActivityOpened, ArchiveActivity};

pub mod projects;
pub mod sections;
mod sort;
pub use sort::{ChatOptions, ChatOrderChanged, ChatSort, NewSectionRequested, Organize};

/// `.sidebar-item`: `--radius-lg` (12.5 px) with `corner-shape: superellipse(1.5)`.
const RADIUS_ROW: Pixels = px(12.5);

pub const PINNED_SECTION_ID: &str = "01984de2-8f74-7c91-a3b2-5c5e937cf318";

/// Recents shows this many rows first and `PAGE` more each time its end comes into view
/// (the original's `maxItems: 10`, `infiniteScroll` paging of the last section).
const PAGE: usize = 10;
/// Chats an expanded project lists before its `Show more` (the original's `maxItems`).
const PROJECT_PAGE: usize = 5;
/// A project chat's label starts under its folder's label.
const PROJECT_INDENT: f32 = 24.;

const WORKTREE_TOOLTIP: &str = "This is a git worktree";
const MISSING_PROJECT_TOOLTIP: &str = "This project folder was deleted or moved";
static PROJECT_TOOLTIP_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| textsys::letter_spacing(-0.15));

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub id: SharedString,
    pub label: SharedString,
    pub new_chat_label: SharedString,
    /// The project's first local root path; the cwd of a chat started in it.
    pub root: Option<SharedString>,
    /// Every root, primary first: what `Edit` prefills.
    pub roots: Vec<SharedString>,
    pub worktree: bool,
    pub missing: bool,
    pub expanded: bool,
    /// In the global state's `pinned-project-ids`; pinned projects list first.
    pub pinned: bool,
    /// The stored `updatedAt`, in seconds.
    pub updated_at: Option<i64>,
}

/// Thread name or preview, cut to 80 characters. Views supply their own empty-state label.
pub fn title(thread: &Thread) -> SharedString {
    title_within(thread, 80)
}

/// An opened (read) thread's title: 26.1029 cuts its preview to 60 characters.
pub fn opened_title(thread: &Thread) -> SharedString {
    title_within(thread, 60)
}

fn title_within(thread: &Thread, max: usize) -> SharedString {
    if let Some(name) = &thread.name {
        return name.clone().into();
    }
    let preview = thread.preview.trim();
    if preview.chars().count() <= max {
        preview.to_owned().into()
    } else {
        let cut: String = preview.chars().take(max - 1).collect();
        format!("{}…", cut.trim_end()).into()
    }
}

fn display_title(title: SharedString) -> SharedString {
    if title.is_empty() {
        "New chat".into()
    } else {
        title
    }
}

pub fn archive_shell(cx: &mut App) -> (gpui::Entity<Shell>, gpui::Entity<ArchivedChats>) {
    let sidebar = cx.new(|_| crate::EmptySidebar).into();
    let main = cx.new(ArchivedChats::new);
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, main.clone().into());
        shell.sidebar_open = false;
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    });
    (shell, main)
}

pub async fn archived_threads(
    bridge: &bridge::Bridge,
    page_size: usize,
) -> anyhow::Result<Vec<Thread>> {
    // ponytail: eager archive summaries; load on scroll if archive size makes entry slow.
    let request = |cursor: Option<String>| {
        bridge.request::<_, ThreadListResponse>(
            "thread/list",
            serde_json::json!({
                "archived": true,
                "cursor": cursor,
                "limit": page_size,
                "modelProviders": null,
                "sortKey": "updated_at",
                "sourceKinds": [],
                "useStateDbOnly": true
            }),
        )
    };
    let first = request(None).await?;
    let mut threads = first.data;
    let mut cursor = first.next_cursor;
    let mut seen = HashSet::new();
    while let Some(next_cursor) = cursor {
        anyhow::ensure!(
            seen.insert(next_cursor.clone()),
            "App Server repeated an archived thread list cursor"
        );
        let page = request(Some(next_cursor)).await?;
        threads.extend(page.data);
        cursor = page.next_cursor;
    }
    Ok(threads)
}

pub struct ArchivedChats {
    bridge: std::sync::Arc<bridge::Bridge>,
    pub threads: Option<anyhow::Result<Vec<Thread>>>,
    _load: gpui::Task<()>,
}

impl ArchivedChats {
    fn new(cx: &mut Context<Self>) -> Self {
        let bridge = cx.global::<crate::screens::settings::AppServer>().0.clone();
        Self {
            threads: None,
            _load: Self::load(bridge.clone(), cx),
            bridge,
        }
    }

    pub fn unarchived(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(Ok(threads)) = &mut self.threads {
            threads.retain(|thread| thread.id != id);
        } else if self.threads.is_none() {
            // A pending read can still contain the restored chat.
            self._load = Self::load(self.bridge.clone(), cx);
        }
        cx.notify();
    }

    fn load(bridge: std::sync::Arc<bridge::Bridge>, cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn(async move |this, cx| {
            let threads = archived_threads(&bridge, 100).await;
            this.update(cx, |this, cx| {
                this.threads = Some(threads);
                cx.notify();
            })
            .ok();
        })
    }
}

impl Render for ArchivedChats {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let mut content = div()
            .w_full()
            .max_w(px(768.))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text("Archived chats", 24., 28.8, 500., t.text));
        content = match &self.threads {
            None => content.child(text(
                "Loading archived chats…",
                14.,
                21.,
                430.,
                t.text_secondary,
            )),
            Some(Err(_)) => content.child(text(
                "Could not load archived chats",
                14.,
                21.,
                430.,
                t.text,
            )),
            Some(Ok(threads)) if threads.is_empty() => {
                content.child(text("No archived chats", 14., 21., 430., t.text_secondary))
            }
            Some(Ok(threads)) => content.children(threads.iter().map(|thread| {
                div()
                    .p(px(16.))
                    .bg(t.surface_secondary)
                    .rounded(px(16.))
                    .child(text(display_title(title(thread)), 14., 21., 430., t.text))
            })),
        };
        div()
            .id("archived-chats")
            .size_full()
            .overflow_y_scroll()
            .bg(t.surface)
            .px(px(24.))
            .pt(px(112.))
            .pb(px(24.))
            .child(content)
    }
}
#[derive(Clone, Copy, Default, PartialEq)]
pub enum ListState {
    #[default]
    Ready,
    Loading,
    Failed,
}

gpui::actions!(sidebar, [RetryThreads, RetryProjects, RetryActivity]);

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedSections {
    sections: Vec<SavedSection>,
    collapsed_section_ids: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedSection {
    id: String,
    name: String,
    item_keys: Vec<String>,
}

struct CustomSection {
    id: SharedString,
    title: SharedString,
    paging_id: SharedString,
    thread_ids: Vec<String>,
    rows: Vec<usize>,
    expanded: bool,
    collapsed: bool,
}

impl CustomSection {
    fn new(id: String, title: String, thread_ids: Vec<String>, collapsed: bool) -> Self {
        Self {
            id: format!("custom:{id}").into(),
            title: title.into(),
            paging_id: format!("custom-paging:{id}").into(),
            thread_ids,
            rows: Vec::new(),
            expanded: false,
            collapsed,
        }
    }

    fn shown(&self) -> usize {
        if self.collapsed {
            0
        } else if self.expanded || self.rows.len() == PAGE + 1 {
            self.rows.len()
        } else {
            self.rows.len().min(PAGE)
        }
    }
}

#[derive(Clone, Copy)]
enum ThreadGroup {
    Pinned,
    Recents,
    Custom(usize),
}

pub struct HomeSidebar {
    /// `(thread id, row label, updated timestamp)` in `thread/list` order.
    threads: Vec<(String, SharedString, i64)>,
    projects: Vec<Project>,
    pub threads_state: ListState,
    pub projects_state: ListState,
    pub activity_clock: Option<crate::screens::thread::view::Clock>,
    pub activity_available: bool,
    activity_started_at: i64,
    activity_recency: HashMap<String, i64>,
    activity_priority: bool,
    activity_pinned: bool,
    activity_scheduled: bool,
    activity_archive_supported: bool,
    activity_items: HashSet<String>,
    activity_scheduled_ids: HashSet<String>,
    pub activity_sources_state: ListState,
    activity_menu: Option<priority::OptionsMenu>,
    /// Index into `MODES`.
    mode: usize,
    mode_menu: Option<mode::ModeMenu>,
    chatgpt_chats: Vec<ChatGptChat>,
    activity_button: Bounds<Pixels>,
    worktree_tip: bool,
    worktree_tip_epoch: u64,
    worktree_tip_bounds: Bounds<Pixels>,
    selected: Option<String>,
    /// `Projects`, `Recents`, `Pinned` and `Cloud` collapsed by a click on their header.
    collapsed: [bool; 4],
    cloud_tasks: Vec<CloudTaskRow>,
    pinned: HashSet<String>,
    sections: Vec<CustomSection>,
    /// Each chat's emoji (the original's `sidebar-thread-metadata`), shown before its title.
    pub emojis: HashMap<String, SharedString>,
    grouped_rows: HashSet<usize>,
    /// Threads whose rows carry the unread dot; any lights the header's bell.
    pub unread: Vec<String>,
    /// Threads with an active turn (`thread/status/changed`), by the clock reading when their
    /// spinner started.
    running: HashMap<String, Instant>,
    /// Threads whose active status carries `waitingOnApproval`: their rows show the green
    /// `Awaiting approval` pill and the blue dot in place of the spinner.
    awaiting_approval: HashSet<String>,
    /// Recents rows shown: grows by [`PAGE`] while the list's end is in view.
    limit: usize,
    /// The owner holds a `thread/list` cursor: the end sentinel stays and asks for the page.
    has_more: bool,
    loading: bool,
    page_error: bool,
    /// The sections' scroller (Projects and Recents under the header).
    pub scroll: ScrollHandle,
    /// The open row context menu.
    menu: Option<ThreadMenu>,
    native_menu_snapshot: Option<serde_json::Value>,
    /// The `Chat sidebar options` choices, from the app's saved state.
    pub chat_options: ChatOptions,
    /// The open `Chat sidebar options` menu.
    options_menu: Option<sort::OptionsMenu>,
    /// Thread id to the `cwd` it runs in: a project lists the chats in its root.
    cwds: HashMap<String, SharedString>,
    /// Projects whose `Show more` was clicked: they list every loaded chat.
    projects_shown_all: HashSet<SharedString>,
}

/// Emitted when a Recents row is clicked, with the thread id.
pub struct OpenThread(pub String);

impl EventEmitter<OpenThread> for HomeSidebar {}

/// Emitted when Recents wants rows past the loaded `thread/list` pages (its end is in view
/// and [`HomeSidebar::set_has_more`] was given `true`).
pub struct LoadMore;

impl EventEmitter<LoadMore> for HomeSidebar {}

/// A Recents row's `Archive chat`: `thread/archive` on that thread (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ArchiveThread(pub String);

/// A project row's click or new-chat icon: make this root (a project's local root path) the
/// folder new chats start in.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct SelectProject(pub String);

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ProjectAction {
    pub id: String,
    pub command: String,
}

/// A row's `Pin chat` / `Unpin chat`: `thread/section/move` of the thread (its id) into the
/// Pinned section (`true`) or out of it (`false`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct PinThread(pub String, pub bool);

/// The row menu's `Rename`: opens the `Rename chat` dialog for the thread (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct RenameThread(pub String);

/// A row menu item the app runs on the thread (its id): `Section`, `Fork`, `Copy`,
/// `Open in new window` and `Permanently delete`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ThreadMenuAction {
    pub thread: String,
    pub command: ThreadCommand,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ThreadCommand {
    /// `thread/section/move` into this custom section, or out of its own one (`None`).
    MoveToSection(Option<String>),
    /// `New section…`: the section dialog, then the move into the new section.
    NewSection,
    /// `Fork chat`.
    Fork,
    /// `Share`: a link to a snapshot of the thread.
    Share,
    CopyDeeplink,
    CopyMarkdown,
    CopyWorkingDirectory,
    OpenInNewWindow,
    /// `Permanently delete`: `Delete task?`, then `thread/delete`.
    Delete,
}

fn is_pinned(t: &Thread) -> bool {
    t.section
        .as_ref()
        .is_some_and(|s| s.id == PINNED_SECTION_ID)
}

fn pinned_ids(threads: &[Thread]) -> HashSet<String> {
    threads
        .iter()
        .filter(|t| is_pinned(t))
        .map(|t| t.id.clone())
        .collect()
}

fn thread_cwds(threads: &[Thread]) -> HashMap<String, SharedString> {
    threads
        .iter()
        .map(|t| (t.id.clone(), t.cwd.clone().into()))
        .collect()
}

impl HomeSidebar {
    /// Recents from a live `thread/list`; `selected` is the open thread, highlighted.
    pub fn new(threads: &[Thread], selected: Option<&str>) -> Self {
        Self {
            threads: threads
                .iter()
                .map(|t| {
                    let title = if selected == Some(t.id.as_str()) {
                        opened_title(t)
                    } else {
                        title(t)
                    };
                    (t.id.clone(), display_title(title), t.updated_at)
                })
                .collect(),
            projects: Vec::new(),
            threads_state: ListState::Ready,
            projects_state: ListState::Ready,
            activity_clock: None,
            activity_available: true,
            activity_started_at: 0,
            activity_recency: HashMap::new(),
            activity_priority: true,
            activity_pinned: false,
            activity_scheduled: false,
            activity_archive_supported: true,
            activity_items: HashSet::new(),
            activity_scheduled_ids: HashSet::new(),
            activity_sources_state: ListState::Ready,
            activity_menu: None,
            mode: DEFAULT_MODE,
            mode_menu: None,
            chatgpt_chats: Vec::new(),
            activity_button: Bounds::default(),
            worktree_tip: false,
            worktree_tip_epoch: 0,
            worktree_tip_bounds: Bounds::default(),
            selected: selected.map(str::to_owned),
            collapsed: [false; 4],
            cloud_tasks: Vec::new(),
            pinned: pinned_ids(threads),
            sections: Vec::new(),
            emojis: HashMap::new(),
            grouped_rows: HashSet::new(),
            unread: Vec::new(),
            awaiting_approval: HashSet::new(),
            running: threads
                .iter()
                .filter(|t| t.status == ThreadStatus::Active)
                .map(|t| (t.id.clone(), Instant::now()))
                .collect(),
            limit: PAGE,
            has_more: false,
            loading: false,
            page_error: false,
            scroll: ScrollHandle::new(),
            menu: None,
            native_menu_snapshot: None,
            chat_options: ChatOptions::default(),
            options_menu: None,
            cwds: thread_cwds(threads),
            projects_shown_all: HashSet::new(),
        }
    }

    pub fn set_threads_state(&mut self, state: ListState, cx: &mut Context<Self>) {
        self.threads_state = state;
        cx.notify();
    }

    pub fn set_projects_state(&mut self, state: ListState, cx: &mut Context<Self>) {
        self.projects_state = state;
        cx.notify();
    }

    pub fn set_projects(&mut self, projects: Vec<Project>, cx: &mut Context<Self>) {
        self.projects = projects;
        self.projects_state = ListState::Ready;
        self.worktree_tip = false;
        self.worktree_tip_epoch += 1;
        cx.notify();
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn worktree_tooltip(&self) -> Option<(&'static str, Bounds<Pixels>)> {
        self.worktree_tip
            .then_some((WORKTREE_TOOLTIP, self.worktree_tip_bounds))
    }

    fn hover_worktree(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.worktree_tip_epoch += 1;
        self.worktree_tip = false;
        cx.notify();
        if !hovered {
            return;
        }
        let epoch = self.worktree_tip_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            this.update(cx, |this, cx| {
                if this.worktree_tip_epoch == epoch {
                    this.worktree_tip = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn project_rows(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows: Vec<AnyElement> = self
            .projects
            .iter()
            .map(|project| {
                let group = project.id.clone();
                let project_id = project.id.to_string();
                let color = theme.text_default_nav;
                // Right insets in the 272 px row, so the label follows the panel's width.
                let label_right = if project.worktree { 36. } else { 0. };
                // A deleted or moved folder cannot host a chat.
                let selectable = project.root.clone().filter(|_| !project.missing);
                let content = div()
                    .absolute()
                    .inset_0()
                    .opacity(if project.missing { 0.7 } else { 1. })
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(0.)
                            .group_hover(group.clone(), |style| style.opacity(1.))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(RADIUS_ROW),
                                corner: shapes::Corner::Squircle,
                                fill: Some(theme.ghost_hover),
                                ..Default::default()
                            })),
                    )
                    .child(div().absolute().left(px(8.)).top(px(7.)).child(icon(
                        if project.expanded {
                            "sidebar-cmd-folder"
                        } else {
                            "thread-project"
                        },
                        16.,
                        color,
                    )))
                    .child(
                        div()
                            .id("project-label")
                            .absolute()
                            .left(px(32.))
                            .right(px(label_right))
                            .group_hover(group.clone(), move |style| {
                                style.right(px(label_right + 56.))
                            })
                            .top_0()
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .layer()
                            .child(text(project.label.clone(), 14., 21., 430., color)),
                    )
                    .when(project.worktree, |row| {
                        row.child(
                            div()
                                .id("worktree-marker")
                                .absolute()
                                .right(px(10.))
                                .top(px(7.))
                                .size(px(16.))
                                .group_hover(group.clone(), |style| style.right(px(66.)))
                                .on_hover(cx.listener(|this, hovered, _, cx| {
                                    this.hover_worktree(*hovered, cx)
                                }))
                                .child(icon("thread-fork", 16., theme.text_tertiary)),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .right(px(8.))
                            .top(px(5.))
                            .h(px(20.))
                            .flex()
                            .gap(px(8.))
                            .opacity(0.)
                            .group_hover(group.clone(), |style| style.opacity(1.))
                            .child({
                                let id = project.id.to_string();
                                div()
                                    .id(SharedString::from(format!("project-actions-{project_id}")))
                                    .size(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .role(gpui::Role::Button)
                                    .aria_label("Project actions")
                                    .cursor_pointer()
                                    .on_click(cx.listener(
                                        move |this, e: &gpui::ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.open_project_menu(
                                                id.clone(),
                                                e.position(),
                                                window,
                                                cx,
                                            );
                                        },
                                    ))
                                    .child(icon("thread-chat-actions", 14., theme.text_tertiary))
                            })
                            .child(
                                div()
                                    .id("project-new-chat")
                                    .size(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .role(gpui::Role::Button)
                                    .aria_label(project.new_chat_label.clone())
                                    .when_some(selectable.clone(), |button, root| {
                                        button.cursor_pointer().on_click(move |_, window, cx| {
                                            cx.stop_propagation();
                                            window.dispatch_action(
                                                Box::new(SelectProject(root.to_string())),
                                                cx,
                                            );
                                            window.dispatch_action(Box::new(crate::NewChat), cx);
                                        })
                                    })
                                    .opacity(if project.missing { 0.4 } else { 1. })
                                    .child(icon("new-chat", 14., theme.text_tertiary)),
                            ),
                    )
                    // A collapsed folder carries the dot of its unread chats.
                    .when(
                        !project.expanded
                            && self
                                .project_chats(project)
                                .any(|(id, _, _)| self.unread.contains(id)),
                        |row| {
                            row.child(
                                div()
                                    .absolute()
                                    .top(px(11.))
                                    .right(px(14.))
                                    .size(px(8.))
                                    .group_hover(group.clone(), |s| s.invisible())
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: Corners::all(Theme::RADIUS_FULL),
                                        fill: Some(theme.info_solid),
                                        ..Default::default()
                                    })),
                            )
                        },
                    );
                let row = div()
                    .id(project.id.clone())
                    .group(group)
                    .relative()
                    .w_full()
                    .h(px(30.))
                    .overflow_hidden()
                    .role(gpui::Role::ListItem)
                    .aria_label(project.label.clone())
                    .child(content)
                    .on_mouse_down(MouseButton::Right, {
                        let id = project.id.to_string();
                        cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                            this.open_project_menu(id.clone(), e.position, window, cx)
                        })
                    })
                    .when_some(selectable, |row, root| {
                        row.cursor_pointer().on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(SelectProject(root.to_string())), cx)
                        })
                    })
                    .when(project.missing, |row| {
                        row.child(tooltip(
                            div()
                                .id(project.id.clone())
                                .absolute()
                                .right(px(8.))
                                .top(px(5.))
                                .size(px(20.))
                                .role(gpui::Role::Button)
                                .aria_disabled(true)
                                .aria_label(project.new_chat_label.clone())
                                .font_features(PROJECT_TOOLTIP_FEATURES.clone()),
                            MISSING_PROJECT_TOOLTIP,
                            None,
                            Side::Top,
                        ))
                    });
                crate::rounded_clip(RADIUS_ROW, shapes::Corner::Squircle, row).into_any_element()
            })
            .collect();
        let mut children = Vec::with_capacity(rows.len());
        for (project, row) in self.projects.iter().zip(rows) {
            children.push(row);
            if project.expanded {
                children.push(self.project_chat_list(project, window, cx));
            }
        }
        div()
            .id("project-rows")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(1.))
            .children(children)
            .into_any_element()
    }

    /// The loaded chats that run in `project`'s root, in `thread/list` order.
    // shortcut: matches the first root only; the original also reads `thread-project-assignments`
    // and every root. Upgrade when a project must list chats started outside its first root.
    fn project_chats<'a>(
        &'a self,
        project: &'a Project,
    ) -> impl Iterator<Item = &'a (String, SharedString, i64)> {
        self.threads.iter().filter(move |(id, _, _)| {
            project
                .root
                .as_ref()
                .is_some_and(|root| self.cwds.get(id) == Some(root))
        })
    }

    /// The Projects index's `Updated`: the newest of the project's own `updatedAt` and its
    /// loaded chats' times, in seconds (the original's `modifiedAt`).
    pub fn project_activity(&self, project: &Project) -> Option<i64> {
        self.project_chats(project)
            .map(|(_, _, at)| *at)
            .chain(project.updated_at)
            .max()
    }

    /// An expanded project's chats: [`PROJECT_PAGE`] rows, then `Show more` while rows are
    /// hidden or `thread/list` has more pages; `No chats` when it has none.
    fn project_chat_list(
        &self,
        project: &Project,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chats: Vec<_> = self.project_chats(project).collect();
        let mut shown = if self.projects_shown_all.contains(&project.id) {
            chats.len()
        } else {
            chats.len().min(PROJECT_PAGE)
        };
        // The original shows a single hidden row instead of `Show more`.
        if !self.has_more && chats.len() == shown + 1 {
            shown += 1;
        }
        let tail = if shown < chats.len() || self.has_more {
            let id = project.id.clone();
            let paging_id = SharedString::from(format!("project-paging:{id}"));
            div()
                .py(px(4.))
                .pl(px(24.))
                .flex()
                .child(
                    self.paging_button(paging_id, "Show more", window, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.projects_shown_all.insert(id.clone());
                            if this.has_more && !this.loading {
                                this.loading = true;
                                this.page_error = false;
                                cx.emit(LoadMore);
                            }
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else if chats.is_empty() {
            sidebar_empty("No chats", cx)
                .px(px(32.))
                .py(px(4.))
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let rows: Vec<_> = chats[..shown]
            .iter()
            .map(|(id, label, _)| self.chat_row(id, label.clone(), PROJECT_INDENT, window, cx))
            .collect();
        div()
            .id(SharedString::from(format!("project-chats:{}", project.id)))
            .flex()
            .flex_col()
            .pb(px(8.))
            .role(gpui::Role::List)
            .child(div().flex().flex_col().gap(px(1.)).children(rows))
            .child(tail)
            .into_any_element()
    }

    fn worktree_tooltip_element(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (background, foreground) = match theme.appearance {
            Appearance::Dark => (theme.surface_elevated_secondary, theme.text),
            Appearance::Light => (theme.button_primary_solid, rgba(0xffffffff).into()),
        };
        let owner = cx.weak_entity();
        // The original SVG trigger leaves this tooltip at the window origin.
        deferred(
            anchored().position(point(px(8.), px(2.))).child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(12.))
                    .py(px(5.))
                    .border_1()
                    .border_color(foreground.opacity(0.05))
                    .rounded(px(20.))
                    .bg(background)
                    .shadow(vec![gpui::BoxShadow {
                        color: rgba(0x0f172a33).into(),
                        offset: point(px(0.), px(8.)),
                        blur_radius: px(18.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .child(
                        text(WORKTREE_TOOLTIP, 13., 18., 430., foreground)
                            .font_features(PROJECT_TOOLTIP_FEATURES.clone()),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, (), window, cx| {
                                owner
                                    .update(cx, |this, _| this.worktree_tip_bounds = bounds)
                                    .ok();
                                let owner = owner.clone();
                                window.on_key_event(move |event: &KeyDownEvent, phase, _, cx| {
                                    if phase == DispatchPhase::Capture
                                        && event.keystroke.key == "escape"
                                    {
                                        owner
                                            .update(cx, |this, cx| this.hover_worktree(false, cx))
                                            .ok();
                                    }
                                });
                            },
                        )
                        .absolute()
                        .inset(px(-1.)),
                    ),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    pub fn set_threads(&mut self, threads: &[Thread], cx: &mut Context<Self>) {
        self.threads = self
            .sorted(threads)
            .into_iter()
            .map(|t| (t.id.clone(), display_title(title(t)), t.updated_at))
            .collect();
        self.threads_state = ListState::Ready;
        self.cwds = thread_cwds(threads);
        self.pinned = pinned_ids(threads);
        for thread in threads {
            if thread.status == ThreadStatus::Active {
                self.set_running(&thread.id, true, cx);
            }
        }
        self.reindex_sections();
        self.remember_activity_recency();
        cx.notify();
    }

    /// Puts `thread` at the top of Recents (`thread/started`), keeping its current label
    /// while the thread has no title yet.
    pub fn add_thread(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        if is_pinned(thread) {
            self.pinned.insert(thread.id.clone());
        } else {
            self.pinned.remove(&thread.id);
        }
        let old = self.threads.iter().position(|(id, _, _)| *id == thread.id);
        let old = old.map(|i| self.threads.remove(i).1);
        let label = display_title(match (title(thread), old) {
            (label, Some(old)) if label.is_empty() => old,
            (label, _) => label,
        });
        self.threads
            .insert(0, (thread.id.clone(), label, thread.updated_at));
        self.cwds
            .insert(thread.id.clone(), thread.cwd.clone().into());
        self.reindex_sections();
        self.set_running(&thread.id, thread.status == ThreadStatus::Active, cx);
        if self.activity_clock.is_some() && thread.created_at >= self.activity_started_at {
            self.activity_items.insert(thread.id.clone());
        }
        self.remember_activity_recency();
        cx.notify();
    }

    /// Opening a thread reads it: its unread dot goes.
    pub fn select(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        self.selected = id.map(str::to_owned);
        if let Some(id) = id {
            self.unread.retain(|u| u != id);
        }
        cx.notify();
    }

    /// A turn is running in thread `id` (`thread/status/changed` active): its row shows the
    /// spinner in place of the unread dot until the status goes idle.
    pub fn set_running(&mut self, id: &str, running: bool, cx: &mut Context<Self>) {
        if running {
            self.note_activity(id);
            self.running
                .entry(id.to_owned())
                .or_insert_with(|| cx.background_executor().now());
        } else {
            self.running.remove(id);
        }
        cx.notify();
    }

    /// Thread `id` waits for an approval (`thread/status/changed` `activeFlags`).
    pub fn set_awaiting_approval(&mut self, id: &str, awaiting: bool, cx: &mut Context<Self>) {
        let changed = if awaiting {
            self.awaiting_approval.insert(id.to_owned())
        } else {
            self.awaiting_approval.remove(id)
        };
        if changed {
            cx.notify();
        }
    }

    pub fn is_awaiting_approval(&self, id: &str) -> bool {
        self.awaiting_approval.contains(id)
    }

    pub fn any_running(&self) -> bool {
        !self.running.is_empty()
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    /// A turn completed in thread `id` while it was not open: unread dot.
    /// Marks thread `id` read (a press inside its open conversation).
    pub fn set_read(&mut self, id: &str, cx: &mut Context<Self>) {
        let before = self.unread.len();
        self.unread.retain(|u| u != id);
        if self.unread.len() != before {
            cx.notify();
        }
    }

    pub fn set_unread(&mut self, id: &str, cx: &mut Context<Self>) {
        self.note_activity(id);
        if !self.unread.iter().any(|u| u == id) {
            self.unread.push(id.to_owned());
        }
        cx.notify();
    }

    /// Whether the owner can load another `thread/list` page ([`LoadMore`]).
    pub fn set_has_more(&mut self, has_more: bool, cx: &mut Context<Self>) {
        self.has_more = has_more;
        self.loading = false;
        self.page_error = false;
        cx.notify();
    }

    pub fn page_failed(&mut self, cx: &mut Context<Self>) {
        self.loading = false;
        self.page_error = true;
        cx.notify();
    }

    pub fn load_failed(&self) -> bool {
        self.page_error || self.threads_state == ListState::Failed
    }

    /// Recents rows on screen (the first [`limit`](Self::limit) unpinned threads).
    pub fn recents_shown(&self) -> usize {
        self.recents().count().min(self.limit)
    }

    /// Loads recorded local-thread sections before final Recents without adding storage.
    pub fn set_sections(
        &mut self,
        value: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> serde_json::Result<()> {
        let saved: SavedSections = serde::Deserialize::deserialize(value)?;
        self.sections = saved
            .sections
            .into_iter()
            .map(|section| {
                let collapsed = saved.collapsed_section_ids.contains(&section.id);
                let thread_ids = section
                    .item_keys
                    .into_iter()
                    .filter_map(|mut key| {
                        const PREFIX: &str = "codex:thread:local:";
                        if !key.starts_with(PREFIX) {
                            return None;
                        }
                        drop(key.drain(..PREFIX.len()));
                        Some(key)
                    })
                    .collect();
                CustomSection::new(section.id, section.name, thread_ids, collapsed)
            })
            .collect();
        self.reindex_sections();
        cx.notify();
        Ok(())
    }

    pub fn section_shown(&self, key: &str) -> Option<usize> {
        self.sections
            .iter()
            .find(|s| s.id.as_ref() == key)
            .map(CustomSection::shown)
    }

    fn reindex_sections(&mut self) {
        self.grouped_rows.clear();
        if self.sections.is_empty() {
            return;
        }
        let positions: HashMap<_, _> = self
            .threads
            .iter()
            .enumerate()
            .map(|(index, (id, _, _))| (id.as_str(), index))
            .collect();
        for section in &mut self.sections {
            section.rows.clear();
            for id in &section.thread_ids {
                if let Some(&index) = positions.get(id.as_str()) {
                    self.grouped_rows.insert(index);
                    if !self.pinned.contains(id) {
                        section.rows.push(index);
                    }
                }
            }
        }
    }

    pub fn projects_collapsed(&self) -> bool {
        self.collapsed[0]
    }

    fn recents(&self) -> impl Iterator<Item = &(String, SharedString, i64)> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(index, (id, _, _))| {
                !self.pinned.contains(id)
                    && !self.grouped_rows.contains(index)
                    && self.in_recents(id)
            })
            .map(|(_, thread)| thread)
    }

    /// The end sentinel came into view: `PAGE` more rows when they are loaded, else the next
    /// `thread/list` page from the owner.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        let (loaded, has_more) = self
            .activity_window()
            .unwrap_or_else(|| (self.recents().count(), self.has_more));
        if loaded > self.limit {
            self.limit += PAGE;
            cx.notify();
        } else if has_more && !self.loading {
            self.loading = true;
            self.page_error = false;
            cx.notify();
            cx.emit(LoadMore);
        }
    }

    fn rows(&self, group: ThreadGroup, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let recents = matches!(group, ThreadGroup::Recents);
        let custom = match group {
            ThreadGroup::Custom(index) => Some(&self.sections[index]),
            _ => None,
        };
        let count = custom.map_or(self.threads.len(), |s| s.rows.len());
        let limit = match group {
            ThreadGroup::Pinned => usize::MAX,
            ThreadGroup::Recents => self.limit,
            ThreadGroup::Custom(index) => self.sections[index].shown(),
        };
        let rows = (0..count)
            .map(|index| custom.map_or(index, |s| s.rows[index]))
            .filter(|index| match group {
                ThreadGroup::Pinned => self.pinned.contains(&self.threads[*index].0),
                ThreadGroup::Recents => {
                    let id = &self.threads[*index].0;
                    !self.pinned.contains(id)
                        && !self.grouped_rows.contains(index)
                        && self.in_recents(id)
                }
                ThreadGroup::Custom(_) => true,
            })
            .take(limit)
            .map(|index| &self.threads[index]);
        div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .children(rows.map(|(id, label, _)| self.chat_row(id, label.clone(), 0., window, cx)))
            .when(recents, |d| d.children(self.page_tail(cx)))
            .into_any_element()
    }

    fn page_tail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (loaded, has_more) = self
            .activity_window()
            .unwrap_or_else(|| (self.recents().count(), self.has_more));
        if loaded <= self.limit && !has_more && !self.loading && !self.page_error {
            return None;
        }
        let sentinel = (!self.loading && !self.page_error && (loaded > self.limit || has_more))
            .then(|| {
                let this = cx.weak_entity();
                canvas(
                    move |bounds, window, cx| {
                        let clip = window.content_mask().bounds;
                        if bounds.top() < clip.bottom() && bounds.bottom() > clip.top() {
                            cx.defer(move |cx| {
                                this.update(cx, |s, cx| s.load_more(cx)).ok();
                            });
                        }
                    },
                    |_, (), _, _| {},
                )
                .w_full()
                .h(px(9.))
            });
        Some(
            div()
                .children(sentinel)
                .when(self.loading, |d| {
                    d.child(div().h(px(24.)).flex().justify_center().child(icon(
                        "sidebar-cmd-spinner",
                        16.,
                        cx.theme().text_tertiary,
                    )))
                })
                .when(self.page_error, |d| {
                    d.child(
                        div()
                            .id("retry-more-chats")
                            .role(gpui::Role::Button)
                            .aria_label("Could not load more chats")
                            .cursor_pointer()
                            .child(text(
                                "Could not load more chats",
                                12.,
                                24.,
                                430.,
                                cx.theme().text_tertiary,
                            ))
                            .on_click(cx.listener(|s, _, _, cx| s.load_more(cx))),
                    )
                })
                .into_any_element(),
        )
    }

    fn chat_row(
        &self,
        id: &str,
        label: SharedString,
        indent: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let label = match self.emojis.get(id) {
            Some(emoji) => format!("{emoji} {label}").into(),
            None => label,
        };
        let open = id.to_owned();
        let menu_for = id.to_owned();
        let row_focus = focus::handle(
            SharedString::from(format!("thread-row-{id}")),
            0,
            window,
            cx,
        );
        let pin_focus = focus::handle(SharedString::from(format!("pin-chat-{id}")), 0, window, cx);
        let archive_focus = focus::handle(
            SharedString::from(format!("archive-chat-{id}")),
            0,
            window,
            cx,
        );
        let focused = row_focus.read(cx).handle.contains_focused(window, cx);
        let now = cx.background_executor().now();
        let awaiting = self.awaiting_approval.contains(id);
        let end = if awaiting {
            Some(RowEnd::Dot)
        } else {
            self.running
                .get(id)
                .map(|started| {
                    window.request_animation_frame();
                    RowEnd::Spinner(
                        (now.saturating_duration_since(*started).as_secs_f32() % 1.) * 360.,
                    )
                })
                .or_else(|| {
                    self.unread
                        .iter()
                        .any(|unread| unread == id)
                        .then_some(RowEnd::Dot)
                })
        };
        let drag = ChatDrag {
            id: id.to_owned(),
            title: label.clone(),
        };
        thread_row(
            id,
            (label, indent),
            self.selected.as_deref() == Some(id),
            (end, awaiting),
            cx,
            (
                focused,
                self.pinned.contains(id),
                self.activity_clock.is_some(),
                self.sections
                    .iter()
                    .any(|s| s.thread_ids.iter().any(|t| t == id)),
            ),
            [&row_focus, &pin_focus, &archive_focus],
        )
        .when(self.activity_clock.is_some(), |d| {
            d.h(px(53.5)).child(
                div()
                    .absolute()
                    .left(px(8.))
                    .top(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(icon(
                        "sidebar-activity-computer",
                        12.,
                        cx.theme().text_tertiary,
                    ))
                    .child(text("Codex", 12., 16., 430., cx.theme().text_tertiary)),
            )
        })
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenThread(open.clone()))))
        // `Drop to reference chat`: dragged onto a composer, the row becomes a chat chip.
        .on_drag(drag, |drag: &ChatDrag, _, _, cx| cx.new(|_| drag.clone()))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                this.open_menu(menu_for.clone(), e.position, window, cx)
            }),
        )
    }

    fn recents_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let chatgpt = self.chatgpt_surface() && !self.chatgpt_chats().is_empty();
        let local = match self.threads_state {
            ListState::Loading if self.threads.is_empty() => {
                sidebar_empty("Loading chats…", cx).into_any_element()
            }
            ListState::Failed => div()
                .flex()
                .flex_col()
                .when(self.recents().next().is_some(), |d| {
                    d.child(self.rows(ThreadGroup::Recents, window, cx))
                })
                .child(load_error::<RetryThreads>(
                    "Could not load chats",
                    "retry-chats",
                    window,
                    cx,
                ))
                .into_any_element(),
            ListState::Ready if !self.has_more && self.recents().next().is_none() && !chatgpt => {
                sidebar_empty("No chats", cx).into_any_element()
            }
            ListState::Ready | ListState::Loading => self.rows(ThreadGroup::Recents, window, cx),
        };
        // shortcut: ChatGPT rows follow the local ones instead of merging by update time;
        // interleave when Recents sorts both sources together.
        div()
            .flex()
            .flex_col()
            .child(local)
            .child(self.chatgpt_rows(window, cx))
            .into_any_element()
    }

    fn custom_body(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let section = &self.sections[index];
        let rows = self.rows(ThreadGroup::Custom(index), window, cx);
        let mut body = div().flex().flex_col().child(rows);
        if section.rows.len() > PAGE && (section.expanded || section.rows.len() > PAGE + 1) {
            let label = if section.expanded {
                "Show less"
            } else {
                "Show more"
            };
            let button = self
                .paging_button(section.paging_id.clone(), label, window, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sections[index].expanded ^= true;
                    cx.notify();
                }));
            body = body.child(div().py(px(4.)).flex().child(button));
        }
        body.into_any_element()
    }

    /// A list's `Show more` / `Show less` text button, `focus` keyed by `id`.
    fn paging_button(
        &self,
        id: SharedString,
        label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let handle = focus::handle(id.clone(), 0, window, cx);
        let color = Hsla {
            a: if cx.theme().appearance == Appearance::Dark {
                0.498
            } else {
                0.5
            },
            ..cx.theme().text_tertiary
        };
        let hover = cx.theme().ghost_hover;
        let radius = px(9.375);
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(radius)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(text(label, 14., 21., 400., color).top_layer()),
            &handle,
            radius,
            px(2.),
            cx,
        )
    }
}

/// The Home sidebar with Recents from the fixture `thread/list`; `selected_thread` is the id of
/// the open thread, highlighted in Recents.
pub fn home_sidebar(cx: &mut App, selected_thread: Option<&str>) -> AnyView {
    let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
    cx.new(|_| HomeSidebar::new(&list.data, selected_thread))
        .into()
}

impl Render for HomeSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let contents = if self.activity_clock.is_some() {
            self.activity_list(window, cx)
        } else {
            let recents = self.recents_list(window, cx);
            let pinned =
                (!self.pinned.is_empty()).then(|| self.rows(ThreadGroup::Pinned, window, cx));
            let section = |i: usize,
                           title: SharedString,
                           body: AnyElement,
                           window: &mut Window,
                           cx: &mut Context<Self>| {
                let (collapsed, key) = if i < self.collapsed.len() {
                    (self.collapsed[i], title.clone())
                } else {
                    let custom = &self.sections[i - self.collapsed.len()];
                    (custom.collapsed, custom.id.clone())
                };
                let header_focus = focus::handle(key.clone(), -1, window, cx);
                div()
                    .tab_group()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .group(key.clone())
                            .when(i == 0, |d| d.debug_selector(|| "projects-header".into()))
                            .when(i == 1, |d| d.debug_selector(|| "recents-header".into()))
                            .relative()
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .pl(px(8.))
                            .child(text(
                                title.clone(),
                                14.,
                                21.,
                                500.,
                                cx.theme().text_tertiary,
                            ))
                            .child(
                                svg()
                                    .path("icons/chevron-down.svg")
                                    .w(px(14.))
                                    .h(px(14.7))
                                    .ml(px(4.))
                                    .text_color(cx.theme().text_tertiary)
                                    .opacity(if collapsed { 1. } else { 0. })
                                    .group_hover(key.clone(), |s| s.opacity(1.))
                                    .with_transformation(Transformation::rotate(radians(
                                        if collapsed {
                                            -std::f32::consts::FRAC_PI_2
                                        } else {
                                            0.
                                        },
                                    ))),
                            )
                            .child(
                                focus::ring(
                                    div()
                                        .id(key.clone())
                                        .role(gpui::Role::Button)
                                        .aria_label(title)
                                        .aria_expanded(!collapsed)
                                        .absolute()
                                        .top(px(2.5))
                                        .left(px(4.))
                                        .right(px(66.))
                                        .h(px(25.)),
                                    &header_focus,
                                    px(10.),
                                    px(2.),
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        header_focus.read(cx).handle.clone().focus(window, cx);
                                        if i < this.collapsed.len() {
                                            this.collapsed[i] ^= true;
                                        } else {
                                            this.sections[i - this.collapsed.len()].collapsed ^=
                                                true;
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .when(i >= self.collapsed.len(), |d| {
                                d.child(self.section_actions(i - self.collapsed.len(), window, cx))
                            })
                            .when(i == 1, |d| {
                                d.child(self.chat_options_button(key.clone(), window, cx))
                            }),
                    )
                    .when(!collapsed, |d| d.child(div().pt(px(4.)).child(body)))
            };
            let project_rows = match self.projects_state {
                ListState::Loading => sidebar_empty("Loading projects…", cx).into_any_element(),
                ListState::Failed => load_error::<RetryProjects>(
                    "Projects couldn't be loaded",
                    "retry-projects",
                    window,
                    cx,
                ),
                ListState::Ready if self.projects.is_empty() => {
                    sidebar_empty("No projects", cx).into_any_element()
                }
                ListState::Ready => self.project_rows(window, cx),
            };
            let projects = section(0, "Projects".into(), project_rows, window, cx);
            let recents = section(1, "Recents".into(), recents, window, cx);
            let pinned = pinned.map(|rows| section(2, "Pinned".into(), rows, window, cx));
            let cloud = (!self.cloud_tasks.is_empty()).then(|| {
                let rows = self.cloud_rows(window, cx);
                section(3, "Cloud".into(), rows, window, cx)
            });
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .px(px(8.))
                .children(pinned)
                .when(self.chat_options.organize == Organize::Project, |d| {
                    d.child(projects)
                })
                .children((0..self.sections.len()).map(|index| {
                    let body = self.custom_body(index, window, cx);
                    section(
                        self.collapsed.len() + index,
                        self.sections[index].title.clone(),
                        body,
                        window,
                        cx,
                    )
                }))
                .child(recents)
                .children(cloud)
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(home_sidebar_header(
                !self.unread.is_empty(),
                self.activity_clock.is_some(),
                self.activity_available,
                self.mode_switch(window, cx),
                cx,
            ))
            .child(
                // `overflow-y-auto`: the sections scroll under the header and clip at its
                // bottom edge. The extent ends 16 px past Recents: its wrapper's `pb-4`
                // (cancelled in layout by `-mb-4`) reaches past the scroller's own 8 px
                // `pb-row-x`, and Chromium's scrollable overflow takes the farther one.
                // `[contain:layout_paint]` drops a fractional (zoomed) origin for its content.
                div()
                    .id("sidebar-sections")
                    .flex_1()
                    .overflow_y_scroll()
                    .snap_origin()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .pt(px(4.))
                    .pb(px(16.))
                    .child(contents),
            )
            .children(crate::updater::banner(window, cx))
            .children(
                self.menu
                    .as_ref()
                    .filter(|menu| !menu.is_native())
                    .map(|_| self.menu_overlay(window, cx)),
            )
            .children(self.activity_menu_overlay(window, cx))
            .children(self.chat_options_overlay(window, cx))
            .children(self.worktree_tip.then(|| self.worktree_tooltip_element(cx)))
    }
}

fn load_error<A: Action + Default>(
    label: &'static str,
    id: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let retry = focus::handle(id, 0, window, cx);
    div()
        .px(px(8.))
        .py(px(6.))
        .flex()
        .flex_col()
        .items_start()
        .gap(px(8.))
        .child(text(label, 14., 21., 430., cx.theme().text_tertiary))
        .child(focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label("Retry")
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .bg(cx.theme().ghost_hover)
                .child(text("Retry", 12., 18., 430., cx.theme().text_default_nav))
                .on_click(crate::dispatch::<A>),
            &retry,
            px(6.),
            px(2.),
            cx,
        ))
        .into_any_element()
}

/// What a Recents row ends in: the `Working` spinner at `animate-spin` angle (degrees) while a
/// turn runs, else the unread dot. A thread awaiting approval ends in the dot.
#[derive(Clone, Copy)]
enum RowEnd {
    Spinner(f32),
    Dot,
}

/// One Recents row: 30 px, `primary-ghost-hover` fill when hovered or selected. The label is
/// clipped at the row's end with the marquee's 16 px right fade (`_overflowingViewport`).
/// Hover or focus within the row shows `Pin chat` and `Archive chat`.
/// A running row ends in the 16 px `Working` spinner, an unread one in a blue dot
/// ([`RowEnd`]); either takes 28 px off the label and hides on hover. A row awaiting approval
/// shows the green `Awaiting approval` pill (`localTaskRow` chip) after its title.
fn thread_row(
    id: &str,
    (label, indent): (SharedString, f32),
    selected: bool,
    (end, awaiting): (Option<RowEnd>, bool),
    cx: &App,
    (focused, pinned, activity, in_section): (bool, bool, bool, bool),
    handles: [&gpui::Entity<focus::ControlFocus>; 3],
) -> Stateful<Div> {
    let t = cx.theme();
    let clip = if end.is_some() { 28. } else { 0. };
    let group = SharedString::from(format!("thread-row-{id}"));
    let fill = div()
        .absolute()
        .inset_0()
        .child(shapes::decoration(shapes::Decoration {
            radius: Corners::all(RADIUS_ROW),
            corner: shapes::Corner::Squircle,
            fill: Some(t.ghost_hover),
            ..Default::default()
        }));
    let action = |id: &'static str, name: &'static str| {
        focus::ring(
            div()
                .id(SharedString::from(format!("{id}-{}", group)))
                .size(px(20.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .opacity(if focused { 1. } else { 0. })
                .group_hover(group.clone(), |s| s.opacity(1.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(crate::motion::ButtonVisual {
                    glyph: icon(name, if activity { 16. } else { 14. }, t.text_tertiary),
                    radius: px(6.),
                    hover: t.ghost_hover,
                    foreground: t.text.opacity(0.85),
                    selected: false,
                }),
            if id == "pin-chat" {
                handles[1]
            } else {
                handles[2]
            },
            Theme::RADIUS_FULL,
            px(1.),
            cx,
        )
    };
    focus::ring(
        div()
            .id(group.clone())
            .group(group.clone())
            .relative()
            .tab_group()
            .h(px(30.))
            .child(if selected {
                fill
            } else {
                fill.opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(if activity { 6. } else { 5. }))
                    // Right insets in the 272 px row, so the title follows the panel's width.
                    .right(px(if activity {
                        if focused { 85. } else { 5. }
                    } else if focused {
                        56.
                    } else {
                        8. + clip
                    }))
                    .group_hover(group.clone(), |s| {
                        s.right(px(if activity { 85. } else { 56. }))
                    })
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .child(
                        // `_clipViewport _overflowingViewport`: the marquee's CSS mask (8 px left
                        // fade over the track's padding, 16 px right fade), a masked layer in the
                        // sidebar's layer.
                        div()
                            .id(SharedString::from(format!("thread-title-{id}")))
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .pl(px(8. + indent))
                            .overflow_hidden()
                            .when(activity, |d| d.saved_layer())
                            .when(!activity, |d| d.layer())
                            .child(text(label, 14., 20., 430., t.text_default_nav))
                            .child(shapes::x_fade(px(8.), px(16.))),
                    )
                    .when(awaiting, |d| {
                        d.child(
                            awaiting_approval_pill(t)
                                .when(focused, |d| d.hidden())
                                .group_hover(group.clone(), |s| s.hidden()),
                        )
                    }),
            )
            .when_some(end, |d, end| match end {
                // `size-5` box at `pe-2`, `text-text/70`, hidden on hover like the dot.
                RowEnd::Spinner(angle) => d.child(
                    div()
                        .id(SharedString::from(format!("working-{id}")))
                        .absolute()
                        .top(px(5.))
                        .right(px(8.))
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(focused, |d| d.invisible())
                        .group_hover(group.clone(), |s| s.invisible())
                        .role(gpui::Role::Status)
                        .aria_label("Working")
                        .child(
                            svg()
                                .path("icons/sidebar-cmd-spinner.svg")
                                .size(px(16.))
                                .text_color(t.text.opacity(0.7))
                                .with_transformation(Transformation::rotate(radians(
                                    angle.to_radians(),
                                ))),
                        ),
                ),
                RowEnd::Dot => d.child(
                    div()
                        .absolute()
                        .top(px(11.))
                        .right(px(14.))
                        .size(px(8.))
                        .when(focused, |d| d.invisible())
                        .group_hover(group.clone(), |s| s.invisible())
                        .child(shapes::decoration(shapes::Decoration {
                            radius: Corners::all(Theme::RADIUS_FULL),
                            fill: Some(t.info_solid),
                            ..Default::default()
                        })),
                ),
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right(px(if activity { 0. } else { 2. }))
                    .h_full()
                    .w(px(if activity { 52. } else { 54. }))
                    .pr(px(if activity { 5. } else { 6. }))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child({
                        let id = id.to_owned();
                        // A row in a custom section trades `Pin chat` for the original's
                        // `Remove from section` (`sidebarElectron.removeFromSection`).
                        tooltip(
                            action("pin-chat", "sidebar-pin").on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                let action: Box<dyn gpui::Action> = if in_section {
                                    Box::new(ThreadMenuAction {
                                        thread: id.clone(),
                                        command: ThreadCommand::MoveToSection(None),
                                    })
                                } else {
                                    Box::new(PinThread(id.clone(), !pinned))
                                };
                                window.dispatch_action(action, cx);
                            }),
                            match (in_section, pinned) {
                                (true, _) => "Remove from section",
                                (false, true) => "Unpin chat",
                                (false, false) => "Pin chat",
                            },
                            None,
                            Side::Top,
                        )
                    })
                    .child({
                        let id = id.to_owned();
                        tooltip(
                            action("archive-chat", "sidebar-archive").on_click(
                                move |_, window, cx| {
                                    cx.stop_propagation();
                                    window.dispatch_action(Box::new(ArchiveThread(id.clone())), cx)
                                },
                            ),
                            "Archive chat",
                            None,
                            Side::Top,
                        )
                    }),
            ),
        handles[0],
        RADIUS_ROW,
        px(0.),
        cx,
    )
}

/// The `localTaskRow` chip box (`flex min-w-[24px] max-w-[50%] justify-end overflow-hidden`)
/// holding the `approval` tone chip: `rounded-full py-0.5 pe-2.5 ps-2 text-sm max-w-[150px]`,
/// `bg-chart-green/20 text-chart-green`, its label truncated.
fn awaiting_approval_pill(t: &Theme) -> Div {
    div()
        .debug_selector(|| "awaiting-approval".into())
        .flex()
        .flex_none()
        .justify_end()
        .min_w(px(24.))
        .max_w(gpui::relative(0.5))
        .overflow_hidden()
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .min_w_0()
                .max_w(px(150.))
                .overflow_hidden()
                .py(px(2.))
                .pl(px(8.))
                .pr(px(10.))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(shapes::decoration(shapes::Decoration {
                            radius: Corners::all(Theme::RADIUS_FULL),
                            fill: Some(t.text_success.opacity(0.2)),
                            ..Default::default()
                        })),
                )
                .child(
                    text("Awaiting approval", 13., 18.571_428, 430., t.text_success)
                        .min_w_0()
                        .truncate(),
                ),
        )
}

enum ItemKey {
    Thread(String),
    Url(String),
    File(String),
    Command(SharedString),
}

/// One command menu row: optional 16 px leading icon, label, then a description, the chat's
/// project, its key equivalent, or an external-link arrow.
#[derive(Default)]
struct Item {
    icon: Option<&'static str>,
    note: bool,
    label: SharedString,
    search_value: SharedString,
    updated_at: i64,
    aliases: &'static [&'static str],
    /// A command's search description.
    keywords: SharedString,
    /// Chat rows: workspace folder name; `Some` marks the row as a chat.
    project: Option<SharedString>,
    keys: Option<SharedString>,
    /// Chat rows: the thread id.
    thread: Option<String>,
    url: Option<String>,
    file: Option<String>,
    detail: Option<SharedString>,
    /// Chat rows: the `thread/search` content match, shown under the title.
    snippet: Option<SharedString>,
    /// Chat rows: added by `thread/search` (not in `thread/list`); dropped with the query.
    from_search: bool,
    /// Chat rows: the unread dot in the leading slot.
    unread: bool,
    /// Commands whose handler only exists with an open thread (`rr(O, id) === 0` hides the
    /// others): `Search files`, `Open in new window`, `Commit or push`, ...
    thread_only: bool,
    description: Option<&'static str>,
    external: bool,
    /// A drill-in row (`Theme`): chevron on the right.
    chevron: bool,
}

impl Item {
    fn command(icon: Option<&'static str>, label: &'static str, keywords: &'static str) -> Self {
        Self {
            icon,
            label: label.into(),
            search_value: label.into(),
            keywords: keywords.into(),
            ..Default::default()
        }
    }

    fn chat(thread: &Thread, snippet: Option<String>, unread: bool) -> Self {
        let label = display_title(title(thread));
        Self {
            search_value: label.clone(),
            label,
            updated_at: thread.updated_at,
            project: Some(
                Path::new(&thread.cwd)
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                    .into(),
            ),
            thread: Some(thread.id.clone()),
            snippet: snippet.map(Into::into),
            unread,
            ..Default::default()
        }
    }

    fn key(&self) -> ItemKey {
        if let Some(id) = &self.thread {
            ItemKey::Thread(id.clone())
        } else if let Some(url) = &self.url {
            ItemKey::Url(url.clone())
        } else if let Some(file) = &self.file {
            ItemKey::File(file.clone())
        } else {
            ItemKey::Command(self.label.clone())
        }
    }

    fn matches_key(&self, key: &ItemKey) -> bool {
        match key {
            ItemKey::Thread(id) => self.thread.as_ref() == Some(id),
            ItemKey::Url(url) => self.url.as_ref() == Some(url),
            ItemKey::File(file) => self.file.as_ref() == Some(file),
            ItemKey::Command(label) => {
                self.thread.is_none()
                    && self.url.is_none()
                    && self.file.is_none()
                    && self.label == *label
            }
        }
    }

    fn is_chat(&self) -> bool {
        self.project.is_some()
    }

    fn score(&self, pattern: &MatchPattern) -> Option<(u8, i32)> {
        let title = pattern.score(&self.search_value);
        if self.is_chat() {
            if title > 0 {
                return Some((0, title));
            }
            return self.snippet.as_ref().and_then(|snippet| {
                let score = pattern.score(snippet);
                (score > 0).then_some((1, score))
            });
        }
        let score = self
            .aliases
            .iter()
            .fold(title.max(pattern.score(&self.keywords)), |score, alias| {
                score.max(pattern.score(alias))
            });
        (score > 0).then_some((0, score))
    }
}

/// The original `_4o`/`k2` score uses the first matching fragments, not the highest-scoring path.
pub(crate) struct MatchPattern {
    main: WordPattern,
    fallback: Option<WordPattern>,
    paths: bool,
}

impl MatchPattern {
    pub(crate) fn new(query: &str) -> Self {
        let mut lower = String::with_capacity(query.len());
        for c in query.trim().chars() {
            let folded = c.to_lowercase();
            if folded.clone().map(char::len_utf16).sum::<usize>() == c.len_utf16() {
                lower.extend(folded);
            } else {
                lower.push(c);
            }
        }
        let paths = lower.contains(['/', '\\']);
        let mut main = vec![42];
        for unit in lower.encode_utf16() {
            if paths && matches!(unit, 47 | 92) {
                main.extend([42, 0, 42]);
            } else {
                main.push(unit);
            }
        }
        let tail = ['/', '\\']
            .into_iter()
            .filter_map(|c| lower.rfind(c))
            .filter(|&i| i + 1 < lower.len())
            .max();
        Self {
            main: WordPattern::new(main),
            fallback: tail.map(|i| WordPattern::new(lower[i + 1..].encode_utf16().collect())),
            paths,
        }
    }

    /// The UTF-16 value as the matchers see it: path separators become `\0`.
    fn units(&self, value: &str) -> Vec<u16> {
        value
            .encode_utf16()
            .map(|unit| {
                if self.paths && matches!(unit, 47 | 92) {
                    0
                } else {
                    unit
                }
            })
            .collect()
    }

    pub(crate) fn score(&self, value: &str) -> i32 {
        let text = self.units(value);
        self.main
            .degree(&text)
            .or_else(|| self.fallback.as_ref().and_then(|p| p.degree(&text)))
            .map_or(0, |score| (score * 10 - text.len() as i32).max(1))
    }

    /// The original's `d7`: the matched UTF-16 ranges of `value`, `None` without a match.
    fn fragments(&self, value: &str) -> Option<Vec<Range<usize>>> {
        let text = self.units(value);
        self.main
            .ranges(&text)
            .or_else(|| self.fallback.as_ref().and_then(|p| p.ranges(&text)))
    }
}

struct WordPattern {
    units: Vec<u16>,
    mixed_case: bool,
    has_dots: bool,
}

impl WordPattern {
    fn new(units: Vec<u16>) -> Self {
        let mut meaningful = false;
        let mut lower = false;
        let mut upper_after_first = false;
        for &unit in &units {
            lower |= unit_lower(unit);
            upper_after_first |= meaningful && unit_upper(unit);
            meaningful |= !wildcard(unit);
        }
        Self {
            has_dots: units.contains(&46),
            units,
            mixed_case: lower && upper_after_first,
        }
    }

    /// The matched fragments of `text`, adjacent ones merged.
    fn ranges(&self, text: &[u16]) -> Option<Vec<Range<usize>>> {
        let mut ranges = Vec::new();
        if self.units.len() > 100 {
            let literal: Vec<_> = self.units.iter().copied().filter(|&c| c != 42).collect();
            if literal.is_empty() {
                return Some(ranges);
            }
            let start = if self.units[0] == 42 {
                let lower_name: Vec<_> = String::from_utf16(text)
                    .ok()?
                    .to_lowercase()
                    .encode_utf16()
                    .collect();
                let lower_pattern: Vec<_> = String::from_utf16(&literal)
                    .ok()?
                    .to_lowercase()
                    .encode_utf16()
                    .collect();
                lower_name
                    .windows(lower_pattern.len())
                    .position(|part| part == lower_pattern)?
            } else {
                let prefix = text.get(..literal.len())?;
                if String::from_utf16(prefix).ok()?.to_lowercase()
                    != String::from_utf16(&literal).ok()?.to_lowercase()
                {
                    return None;
                }
                0
            };
            if start + literal.len() > text.len() {
                return None;
            }
            ranges.push(start..start + literal.len());
        } else if !self.match_at(text, 0, 0, &mut ranges, &mut HashSet::new()) {
            return None;
        }
        let mut count = 0;
        for index in 0..ranges.len() {
            let range = ranges[index].clone();
            if count > 0 && ranges[count - 1].end == range.start {
                ranges[count - 1].end = range.end;
            } else {
                ranges[count] = range;
                count += 1;
            }
        }
        ranges.truncate(count);
        Some(ranges)
    }

    fn degree(&self, text: &[u16]) -> Option<i32> {
        let ranges = self.ranges(text)?;
        if ranges.is_empty() {
            return Some(0);
        }
        let first = &ranges[0];
        let mut score = 0;
        let mut boundary = 0;
        let mut skipped = 0;
        let mut pattern_index = 0;
        let mut upper = false;
        for (fragment_index, range) in ranges.iter().enumerate() {
            for index in range.clone() {
                let next_fragment = fragment_index > 0 && index == range.start;
                let mut at_boundary = false;
                while boundary <= index {
                    if boundary == index {
                        at_boundary = true;
                    } else if next_fragment {
                        skipped += 1;
                    }
                    boundary = if boundary < text.len() && unit_digit(text[boundary]) {
                        boundary + 1
                    } else {
                        (boundary + 1..text.len())
                            .find(|&i| word_start(text, i))
                            .unwrap_or(text.len() + 1)
                    };
                }
                pattern_index += self.units[pattern_index..]
                    .iter()
                    .position(|&c| same_unit(c, text[index]))?;
                let c = self.units[pattern_index];
                if at_boundary {
                    upper = text[index] == c && unit_upper(c);
                }
                score += if next_fragment && at_boundary && unit_lower(c) {
                    -10
                } else if text[index] == c {
                    if unit_upper(c) {
                        50
                    } else {
                        i32::from(at_boundary)
                    }
                } else if at_boundary || unit_lower(c) && upper {
                    -1
                } else {
                    0
                };
                pattern_index += 1;
            }
        }
        let start = first.start;
        let starts_word = start == 0 || word_start(text, start) && !word_start(text, start - 1);
        score += if starts_word { 1000 } else { 0 };
        score -= ranges.len() as i32 + skipped * 10;
        score += if text[..start].iter().any(|c| matches!(c, 47 | 92)) {
            0
        } else {
            2
        };
        score += i32::from(start == 0) + i32::from(ranges.last().unwrap().end == text.len());
        Some(score + if start == 0 { 10000 } else { 0 })
    }

    fn match_at(
        &self,
        text: &[u16],
        mut p: usize,
        from: usize,
        ranges: &mut Vec<Range<usize>>,
        failed: &mut HashSet<(usize, usize, bool)>,
    ) -> bool {
        if p == self.units.len() {
            return true;
        }
        if wildcard(self.units[p]) {
            while p < self.units.len() && wildcard(self.units[p]) {
                p += 1;
            }
            if p == self.units.len() {
                return true;
            }
            return self.skip_words(
                text,
                (p, self.find_next(text, p, from)),
                true,
                ranges,
                failed,
            );
        }
        self.fragment(text, p, from, ranges, failed)
    }

    fn skip_words(
        &self,
        text: &[u16],
        (p, mut next): (usize, Option<usize>),
        freely: bool,
        ranges: &mut Vec<Range<usize>>,
        failed: &mut HashSet<(usize, usize, bool)>,
    ) -> bool {
        let Some(start) = next else {
            return false;
        };
        if failed.contains(&(p, start, freely)) {
            return false;
        }
        let mut longest = 0;
        while let Some(index) = next {
            let can_start = !unit_upper(self.units[p])
                || unit_upper(text[index])
                || word_start(text, index)
                || !self.mixed_case;
            let length = if can_start {
                self.fragment_length(text, p, index)
            } else {
                0
            };
            if length > longest {
                if !self.middle_match(text, p, index) {
                    longest = length;
                }
                if self.fragment(text, p, index, ranges, failed) {
                    return true;
                }
            }
            next = self.find_next(text, p, index + 1);
            if !freely {
                next = self.check_dots(text, p, index + 1, next);
            }
        }
        failed.insert((p, start, freely));
        false
    }

    fn fragment(
        &self,
        text: &[u16],
        p: usize,
        start: usize,
        ranges: &mut Vec<Range<usize>>,
        failed: &mut HashSet<(usize, usize, bool)>,
    ) -> bool {
        let length = self.fragment_length(text, p, start);
        if length == 0 {
            return false;
        }
        if p + length == self.units.len() {
            ranges.push(start..start + length);
            return true;
        }
        let minimum = if self.middle_match(text, p, start) {
            3
        } else {
            1
        };
        for length in (1..=length).rev() {
            if length < minimum && !self.units.get(p + length).is_some_and(|&c| wildcard(c)) {
                break;
            }
            ranges.push(start..start + length);
            let matched = if wildcard(self.units[p + length]) {
                self.match_at(text, p + length, start + length, ranges, failed)
            } else {
                let next = self.find_next(text, p + length, start + length + 1);
                let next = self.check_dots(text, p + length, start + length, next);
                self.skip_words(text, (p + length, next), false, ranges, failed)
            };
            if matched {
                return true;
            }
            ranges.pop();
        }
        false
    }

    fn find_next(&self, text: &[u16], p: usize, from: usize) -> Option<usize> {
        let anywhere = p > 0 && self.units[p - 1] == 42 || word_separator(self.units[p]);
        if !anywhere
            && self.mixed_case
            && unit_lower(self.units[p])
            && !(p > 0 && word_separator(self.units[p - 1]))
        {
            return None;
        }
        (from..text.len()).find(|&index| {
            same_unit(self.units[p], text[index])
                && (anywhere || !word_char(self.units[p]) || word_start(text, index))
        })
    }

    fn check_dots(
        &self,
        text: &[u16],
        p: usize,
        from: usize,
        next: Option<usize>,
    ) -> Option<usize> {
        next.filter(|&to| {
            !(self.has_dots && (p == 0 || self.units[p - 1] != 46) && text[from..to].contains(&46))
        })
    }

    fn fragment_length(&self, text: &[u16], p: usize, from: usize) -> usize {
        let mut length = 0;
        while from + length < text.len() && p + length < self.units.len() {
            if !same_unit(self.units[p + length], text[from + length]) {
                if length > 0
                    && unit_digit(self.units[p + length])
                    && unit_digit(self.units[p + length - 1])
                    && unit_digit(text[from + length])
                {
                    return 0;
                }
                break;
            }
            length += 1;
        }
        length
    }

    fn middle_match(&self, text: &[u16], p: usize, from: usize) -> bool {
        p > 0
            && self.units[p - 1] == 42
            && !self.units.get(p + 1).is_some_and(|&c| wildcard(c))
            && word_char(text[from])
            && !word_start(text, from)
    }
}

fn wildcard(unit: u16) -> bool {
    matches!(unit, 32 | 42)
}
fn word_char(unit: u16) -> bool {
    unit < 128 && (unit as u8).is_ascii_alphanumeric()
}
fn unit_digit(unit: u16) -> bool {
    (48..=57).contains(&unit)
}
fn unit_upper(unit: u16) -> bool {
    char::from_u32(unit as u32).is_some_and(|c| {
        c.to_uppercase().eq(std::iter::once(c)) && !c.to_lowercase().eq(std::iter::once(c))
    })
}
fn unit_lower(unit: u16) -> bool {
    char::from_u32(unit as u32).is_some_and(|c| {
        c.to_lowercase().eq(std::iter::once(c)) && !c.to_uppercase().eq(std::iter::once(c))
    })
}
fn word_start(text: &[u16], index: usize) -> bool {
    index < text.len()
        && word_char(text[index])
        && (index == 0
            || !word_char(text[index - 1])
            || unit_upper(text[index]) && unit_lower(text[index - 1])
            || unit_digit(text[index]) && !unit_digit(text[index - 1]))
}
fn same_unit(pattern: u16, actual: u16) -> bool {
    if pattern == actual {
        return true;
    }
    let (Some(pattern), Some(actual)) = (
        char::from_u32(pattern as u32),
        char::from_u32(actual as u32),
    ) else {
        return false;
    };
    pattern.to_lowercase().eq(std::iter::once(actual))
        || pattern.to_uppercase().eq(std::iter::once(actual))
}
fn word_separator(unit: u16) -> bool {
    matches!(unit, 9..=13 | 32 | 160 | 5760 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288 | 65279 | 95 | 45 | 58 | 43 | 46 | 47 | 92)
}

/// Which chars of `text` the original (`UR1V`) highlights for `query`: all for an empty
/// query, else the first case-insensitive occurrence, else (`fuzzy`) the `d7` fragments. A
/// text without any match stays plain (all on).
fn highlight(text: &str, query: &str, fuzzy: bool) -> Vec<bool> {
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let q: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    // ponytail: assumes lowercasing keeps the char count (true for the fixture's ASCII).
    if q.is_empty() || lower.len() != text.chars().count() {
        return vec![true; text.chars().count()];
    }
    if let Some(start) = lower.windows(q.len()).position(|w| w == q.as_slice()) {
        return (0..lower.len())
            .map(|i| (start..start + q.len()).contains(&i))
            .collect();
    }
    let marks = MatchPattern::new(query)
        .fragments(text)
        .filter(|_| fuzzy)
        .map_or_else(Vec::new, |ranges| fragment_marks(text, &ranges));
    if marks.contains(&true) {
        marks
    } else {
        vec![true; lower.len()]
    }
}

/// Per-char flags of `text`'s chars inside the UTF-16 `ranges`.
fn fragment_marks(text: &str, ranges: &[Range<usize>]) -> Vec<bool> {
    let mut at = 0;
    text.chars()
        .map(|c| {
            let hit = ranges.iter().any(|r| r.contains(&at));
            at += c.len_utf16();
            hit
        })
        .collect()
}

/// `text` from 8 chars before the char `first`, `…` first when cut (`shouldAdjustStart`),
/// with `marks` kept per char (the `…` off).
fn adjusted(text: &str, marks: Vec<bool>, first: usize) -> (String, Vec<bool>) {
    let start = first.saturating_sub(8);
    let cut = start > 0;
    (
        cut.then_some('…')
            .into_iter()
            .chain(text.chars().skip(start))
            .collect(),
        cut.then_some(false)
            .into_iter()
            .chain(marks.into_iter().skip(start))
            .collect(),
    )
}

/// A command's description under its title (`nRNm`), when it matches `query`: the `d7`
/// fragments on, from 8 chars before the first.
fn description_line(text: &str, query: &str) -> Option<(String, Vec<bool>)> {
    let marks = fragment_marks(text, &MatchPattern::new(query).fragments(text)?);
    let first = marks.iter().position(|&m| m)?;
    Some(adjusted(text, marks, first))
}

/// A chat's content match under its title (`hlYN`): from 8 chars before the first
/// case-insensitive occurrence of `query`, at most 600 chars then `…`, every occurrence on.
fn snippet_line(text: &str, query: &str) -> (String, Vec<bool>) {
    let q: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    let find = |s: &str| -> (Vec<bool>, Option<usize>) {
        let lower: Vec<char> = s.chars().flat_map(char::to_lowercase).collect();
        let mut marks = vec![false; s.chars().count()];
        // ponytail: assumes lowercasing keeps the char count (true for the fixture's ASCII).
        if q.is_empty() || lower.len() != marks.len() {
            return (marks, None);
        }
        let (mut i, mut first) = (0, None);
        while i + q.len() <= lower.len() {
            if lower[i..i + q.len()] == q[..] {
                first.get_or_insert(i);
                marks[i..i + q.len()].fill(true);
                i += q.len();
            } else {
                i += 1;
            }
        }
        (marks, first)
    };
    let (marks, first) = find(text);
    let (mut line, _) = adjusted(text, marks, first.unwrap_or(0));
    if let Some((cut, _)) = line
        .char_indices()
        .nth(600 + usize::from(line.starts_with('…')))
    {
        line.truncate(cut);
        line.push('…');
    }
    let marks = find(&line).0;
    (line, marks)
}

/// [`text`] with the chars `marks` flags in `on` and the others in `off`.
fn marked(
    s: SharedString,
    marks: &[bool],
    size: f32,
    line_height: f32,
    on: Hsla,
    off: Hsla,
) -> Div {
    // One run per stretch of equal marks.
    let mut runs: Vec<(Range<usize>, bool)> = Vec::new();
    for ((i, c), &m) in s.char_indices().zip(marks) {
        match runs.last_mut() {
            Some((r, prev)) if *prev == m => r.end = i + c.len_utf8(),
            _ => runs.push((i..i + c.len_utf8(), m)),
        }
    }
    // `HighlightStyle::color` blends over the base color and keeps the base alpha, so each run
    // sets the opaque color and its alpha through `fade_out`.
    let style = |c: Hsla| HighlightStyle {
        color: Some(Hsla { a: 1., ..c }),
        fade_out: Some(1. - c.a),
        ..Default::default()
    };
    div()
        .text_size(px(size))
        .line_height(px(line_height))
        .font_weight(FontWeight(430.))
        .text_color(Hsla { a: 1., ..off })
        .whitespace_nowrap()
        .child(
            StyledText::new(s).with_highlights(
                runs.into_iter()
                    .map(|(r, m)| (r, style(if m { on } else { off }))),
            ),
        )
}

/// `CommandMenuDialog` in the 26.1029 unified search (sidebar `Search`): the query input over
/// `Chats` (from `thread/list`), `Quick actions`, the `Settings` pages and the command groups
/// `Chat`, `Navigation`, `Panels`, `Project`, `Skills`, `Configure`, `App`, filtered by the
/// query (which moves `Chats` after the commands), then the search tabs. The selected row
/// carries the `primary-ghost-hover` fill, the others `opacity: .75`.
pub struct CommandMenu {
    groups: Vec<(&'static str, Vec<Item>)>,
    retry_item: Item,
    /// `Could not search Space`: the port has no Space search, only the reference shows it.
    space_item: Item,
    space_error: bool,
    /// The selected [`TABS`] entry.
    tab: &'static str,
    query: String,
    query_anchor: usize,
    query_cursor: usize,
    marked_query: Option<Range<usize>>,
    query_line: Option<ShapedLine>,
    query_bounds: Option<Bounds<Pixels>>,
    query_scroll: Pixels,
    query_dragging: bool,
    show_caret: bool,
    selected: usize,
    retained_selection: Option<ItemKey>,
    focus: FocusHandle,
    /// A thread is open: its commands show (`Search files`, `Mark as unread`, ...).
    thread_open: bool,
    git_available: bool,
    activity_available: bool,
    /// Bumped on every query change; a debounced search only fires for the latest.
    search_gen: usize,
    search_scheduled: bool,
    /// The local and ChatGPT searches complete independently.
    searching: bool,
    cloud_enabled: bool,
    cloud_searching: bool,
    cloud_error: bool,
    file_search: Option<FileSearch>,
    standalone: bool,
    open_error: bool,
    recent_cycle: bool,
}

/// The search tabs under the list (`ey.n6`); a standalone window shows the first three.
const TABS: [&str; 6] = ["All", "Actions", "Chats", "Space", "Files", "Text"];

struct FileSearch {
    session: String,
    root: String,
    rows: Vec<Item>,
    recent: Vec<String>,
    error: bool,
    phase: FileSearchPhase,
}

#[derive(Clone, Copy, PartialEq)]
enum FileSearchPhase {
    Idle,
    Starting,
    Ready,
}

/// What the command menu asks its owner to do.
pub enum CommandMenuEvent {
    /// Escape or a click outside the dialog.
    Dismiss,
    /// A chat row: open the thread.
    OpenThread(String),
    /// A command row, by its label (`New chat`, `General`, `Manage scheduled tasks`, ...).
    Command(SharedString),
    /// Starts the local and enabled cloud searches after the input debounce.
    Search(SearchRequest),
    RetryCloudSearch(SearchRequest),
    OpenUrl(String),
    SearchFiles(String),
    OpenFile(String),
}

#[derive(Clone)]
pub struct SearchRequest {
    pub term: SharedString,
    pub generation: usize,
}

pub struct CloudSearchHit {
    pub url: String,
    pub title: String,
    pub snippet: Option<String>,
    pub updated_at: i64,
    pub source: &'static str,
}

/// One `thread/search` result.
#[derive(serde::Deserialize)]
pub struct SearchHit {
    pub thread: Thread,
    pub snippet: String,
}

/// The original's `useDebouncedValue(search, 200)` before the content search.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);

/// Content results need 3+ chars (`includeContentResults`); `limit` is the quick
/// presentation's `maxResults`.
pub const SEARCH_MIN_CHARS: usize = 3;
pub const SEARCH_LIMIT: u32 = 9;

/// The command groups after `Settings`, in the original's `commandMenuGroupKey` order
/// (`thread, navigation, panels, workspace, skills, configure, app`); `Quick actions` holds
/// `newTask` and `openFolder` (plus `searchFiles` with a thread).
const GROUPS: [&str; 7] = [
    "Chat",
    "Navigation",
    "Panels",
    "Project",
    "Skills",
    "Configure",
    "App",
];

/// `(group, icon, label, description, keys, thread_only)`: the commands the original lists
/// on the route1 stack (recorded in refs/interactions/s026.json, Home and a thread route).
/// Commands without a handler in the current context are hidden; `thread_only` marks those
/// that a thread route registers.
/// ponytail: icons only where the port has the asset; the rest show an empty slot.
type Command = (
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
    &'static str,
    bool,
);
const COMMANDS: &[Command] = &[
    (
        "Quick actions",
        Some("sidebar-cmd-new-chat"),
        "New chat",
        "Start a new chat",
        "⌘N",
        false,
    ),
    (
        "Quick actions",
        Some("sidebar-cmd-folder"),
        "Open folder",
        "Add a local project to ChatGPT",
        "⌘O",
        false,
    ),
    (
        "Quick actions",
        Some("search"),
        "Search files",
        "",
        "⌘P",
        true,
    ),
    (
        "Chat",
        Some("sidebar-cmd-new-chat"),
        "New standalone chat",
        "Start a new chat outside of any project",
        "⌥⌘O",
        false,
    ),
    (
        "Chat",
        None,
        "Open in new window",
        "Open the current chat in a new window",
        "",
        true,
    ),
    (
        "Chat",
        Some("sidebar-archive"),
        "Archive chat",
        "Archive the current chat",
        "⇧⌘A",
        false,
    ),
    (
        "Chat",
        None,
        "Mark as unread",
        "Mark the current chat as unread",
        "⇧⌘U",
        true,
    ),
    (
        "Chat",
        Some("sidebar-pin"),
        "Toggle pin",
        "Pin or unpin the current chat",
        "⌥⌘P",
        false,
    ),
    (
        "Chat",
        None,
        "Open side chat",
        "Open the current chat in a side chat",
        "⌥⌘S",
        true,
    ),
    (
        "Navigation",
        Some("search"),
        "Switch chat…",
        "Search and switch to a chat",
        "",
        false,
    ),
    (
        "Navigation",
        None,
        "Toggle activity view",
        "Turn the sidebar activity view on or off",
        "⌥⌘U",
        false,
    ),
    (
        "Navigation",
        None,
        "Focus main chat",
        "Move keyboard focus to the main chat composer",
        "",
        false,
    ),
    (
        "Navigation",
        None,
        "Open your dot",
        "Open your dot conversation",
        "⌘.",
        false,
    ),
    (
        "Navigation",
        None,
        "Next chat needing attention",
        "Switch to the next chat awaiting input or with unread activity",
        "⌥⌘A",
        false,
    ),
    (
        "Navigation",
        None,
        "Previous chat",
        "Switch to the previous chat",
        "⇧⌘[",
        false,
    ),
    (
        "Navigation",
        None,
        "Next chat",
        "Switch to the next chat",
        "⇧⌘]",
        false,
    ),
    (
        "Navigation",
        None,
        "Go to recent chat 1",
        "Open the recently updated chat in this shortcut slot",
        "⌥⌘1",
        false,
    ),
    (
        "Navigation",
        None,
        "Go to recent chat 2",
        "Open the recently updated chat in this shortcut slot",
        "⌥⌘2",
        false,
    ),
    (
        "Navigation",
        None,
        "Go to recent chat 3",
        "Open the recently updated chat in this shortcut slot",
        "⌥⌘3",
        false,
    ),
    (
        "Navigation",
        None,
        "Go to recent chat 4",
        "Open the recently updated chat in this shortcut slot",
        "⌥⌘4",
        false,
    ),
    (
        "Navigation",
        None,
        "Go to recent chat 5",
        "Open the recently updated chat in this shortcut slot",
        "⌥⌘5",
        false,
    ),
    (
        "Navigation",
        None,
        "Go to recent chat 6",
        "Open the recently updated chat in this shortcut slot",
        "⌥⌘6",
        false,
    ),
    (
        "Navigation",
        None,
        "Switch to Chat",
        "Switch to Chat",
        "",
        false,
    ),
    (
        "Navigation",
        None,
        "Switch to Work",
        "Switch to Work",
        "",
        false,
    ),
    (
        "Navigation",
        None,
        "Switch to Codex",
        "Switch to Codex",
        "",
        false,
    ),
    (
        "Navigation",
        Some("search"),
        "Find",
        "Search the current chat",
        "⌘F",
        false,
    ),
    (
        "Navigation",
        Some("arrow-left"),
        "Back",
        "Go back in navigation history",
        "⌘[",
        false,
    ),
    (
        "Navigation",
        Some("arrow-left"),
        "Forward",
        "Go forward in navigation history",
        "⌘]",
        false,
    ),
    (
        "Panels",
        None,
        "Reopen closed tab",
        "Reopen the most recently closed tab",
        "⇧⌘T",
        false,
    ),
    (
        "Panels",
        Some("sidebar"),
        "Toggle sidebar",
        "Show or hide the sidebar",
        "⇧⌘S",
        false,
    ),
    (
        "Panels",
        None,
        "Toggle navigation panel",
        "Show or hide a page's navigation, or the code viewer's file tree",
        "⇧⌘E",
        false,
    ),
    (
        "Panels",
        None,
        "Toggle bottom panel",
        "Show or hide the bottom panel",
        "⌘J",
        false,
    ),
    (
        "Panels",
        None,
        "Toggle pinned summary",
        "Show or hide the pinned summary",
        "",
        true,
    ),
    (
        "Panels",
        None,
        "Open terminal",
        "Open the terminal panel",
        "⌃`",
        false,
    ),
    (
        "Panels",
        None,
        "Open browser tab",
        "Open a new browser tab",
        "⌘T",
        false,
    ),
    (
        "Panels",
        None,
        "New tab in full view",
        "Enter or exit full view",
        "⇧⌘F",
        false,
    ),
    (
        "Panels",
        Some("sidebar"),
        "Cycle workspace layout",
        "Step from full view to split view, hide tabs, or show tabs beside Chat",
        "⇧⌘B",
        false,
    ),
    (
        "Panels",
        None,
        "Open Changes tab",
        "Open the Changes tab",
        "⌃⇧G",
        true,
    ),
    (
        "Panels",
        Some("sidebar"),
        "New tab",
        "Show or hide Changes for the current chat",
        "⌥⌘B",
        false,
    ),
    (
        "Project",
        None,
        "Commit or push",
        "Open commit or push options",
        "",
        true,
    ),
    (
        "Project",
        None,
        "Create PR",
        "Open pull request creation options",
        "",
        true,
    ),
    (
        "Project",
        None,
        "Create draft PR",
        "Open draft pull request creation options",
        "",
        true,
    ),
    (
        "Project",
        None,
        "Toggle Git blame",
        "Show or hide author and commit details for the active editor line",
        "",
        true,
    ),
    (
        "Project",
        None,
        "New project",
        "Create a project",
        "",
        false,
    ),
    (
        "Skills",
        None,
        "Go to skills",
        "Browse installed and recommended skills",
        "",
        false,
    ),
    (
        "Skills",
        None,
        "Force reload skills",
        "Refresh the skill catalog for the current context",
        "",
        false,
    ),
    ("Configure", None, "Theme", "Codex", "", false),
    (
        "Configure",
        Some("settings-import"),
        "Import from other AI apps",
        "Import from other AI apps",
        "",
        false,
    ),
    (
        "Configure",
        None,
        "Codex Micro setup",
        "Open Codex Micro setup",
        "",
        false,
    ),
    (
        "App",
        Some("settings-profile"),
        "Show Mini",
        "Show Mini from anywhere",
        "⌥Space",
        false,
    ),
    (
        "App",
        None,
        "Clear all unreads",
        "Mark all chats and scheduled task updates as read",
        "⇧Esc",
        false,
    ),
    (
        "App",
        Some("sidebar-cmd-manage-scheduled-tasks"),
        "Manage scheduled tasks",
        "Create or manage scheduled tasks from the current page",
        "",
        false,
    ),
    (
        "App",
        Some("sidebar-cmd-log-out"),
        "Log out",
        "Sign out of ChatGPT",
        "",
        false,
    ),
    (
        "App",
        Some("sidebar-cmd-feedback"),
        "Feedback",
        "Send product feedback to the ChatGPT team",
        "",
        false,
    ),
];

impl EventEmitter<CommandMenuEvent> for CommandMenu {}

impl Focusable for CommandMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CommandMenu {
    /// Over the fixture's `thread/list` (refs/sidebar): the content search answered
    /// (`sessions/sidebar-math-search.jsonl`), the ChatGPT and Space searches failed.
    pub fn new(query: &str, cx: &mut Context<Self>) -> Self {
        let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
        let mut menu = Self::with_threads(&list.data, &[], false, query, cx);
        let search: serde_json::Value =
            fixture::response("sessions/sidebar-math-search.jsonl", "thread/search", 0);
        let hits: Vec<SearchHit> =
            serde_json::from_value(search["data"].clone()).expect("thread/search fixture data");
        for chat in &mut menu.groups[0].1 {
            chat.snippet = hits
                .iter()
                .find(|hit| chat.thread.as_deref() == Some(&hit.thread.id))
                .map(|hit| hit.snippet.clone().into());
        }
        menu.cloud_error = !query.is_empty();
        menu.space_error = !query.is_empty();
        // The static reference capture hides the blinking caret.
        menu.show_caret = false;
        menu
    }

    /// `Chats` from `threads` (a live `thread/list`), in that order; `unread` ids carry the
    /// dot; `thread_open` shows the commands a thread route registers.
    pub fn with_threads(
        threads: &[Thread],
        unread: &[String],
        thread_open: bool,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        let chats = threads
            .iter()
            .map(|t| Item::chat(t, None, unread.contains(&t.id)))
            .collect();
        let command = |(_, icon, label, description, keys, thread_only): &Command| {
            let mut item = Item {
                keys: Some(*keys).filter(|k| !k.is_empty()).map(Into::into),
                thread_only: *thread_only,
                description: (*label == "Theme").then_some(*description),
                chevron: *label == "Theme",
                ..Item::command(*icon, label, description)
            };
            if *label == "Theme" {
                item.search_value = "theme appearance Codex".into();
                item.keywords = "color theme preset dark light night".into();
            }
            if let Some(state) =
                cx.try_global::<crate::screens::settings::keyboard_shortcuts::KeyboardState>()
                && let Some(definition) = crate::screens::settings::keyboard_shortcuts::commands()
                    .iter()
                    .find(|definition| definition.title == *label)
            {
                item.keys = state
                    .preferences
                    .keys(definition)
                    .next()
                    .map(crate::screens::settings::keyboard_shortcuts::key_label)
                    .map(Into::into);
            }
            item
        };
        let group = |name: &'static str| {
            (
                name,
                COMMANDS
                    .iter()
                    .filter(|c| c.0 == name)
                    .map(command)
                    .collect::<Vec<_>>(),
            )
        };
        // Settings pages; `true` = trailing external-link arrow.
        let settings = [
            ("settings-general", "General", false),
            ("bell", "Notifications", false),
            ("settings-import", "Import", false),
            ("settings-profile", "Profile", true),
            ("settings-profile", "Account", false),
            ("settings-profile", "Parental controls", false),
            ("settings-profile", "Trusted contact", false),
            ("settings-appearance", "Appearance", false),
            ("settings-voice", "Voice", false),
            ("settings-storage", "Storage", false),
            ("settings-mini-pets", "Mini & Pets", false),
            ("settings-computer-history", "Computer history", false),
            ("settings-appshots", "Appshots", false),
            ("settings-git", "Git", false),
            ("settings-connections", "Connections", false),
            ("settings-codex-cloud", "Cloud preferences", false),
            ("settings-codex-cloud", "Codex Cloud", false),
            ("settings-legacy-codex-cloud", "Legacy Codex Cloud", false),
            ("settings-code-review", "Code Review", false),
            ("sidebar-cmd-environments", "Environments", false),
            ("settings-worktrees", "Worktrees", false),
            ("settings-configuration", "Configuration", false),
            ("settings-profile", "Work preferences", false),
            ("settings-personalization", "Personalization", false),
            ("settings-keyboard-shortcuts", "Keyboard shortcuts", false),
            ("settings-usage", "Usage & billing", false),
            ("sidebar-cmd-analytics", "Analytics", false),
            ("sidebar-cmd-browser", "Browser", false),
            ("settings-computer-use", "Computer use", false),
            ("sidebar-cmd-mcp-servers", "MCP servers", false),
            ("settings-hooks", "Hooks", false),
            ("sidebar-cmd-plugins", "Plugins", false),
            ("settings-data-controls", "Data controls", false),
            ("settings-archived-chats", "Archived chats", false),
            ("settings-cloud-computer", "Cloud computer", false),
            ("settings-passwords", "Passwords", false),
            ("sidebar-cmd-account-external", "Account", true),
        ]
        .into_iter()
        .filter(|(_, label, _)| {
            *label != "Import" || crate::screens::settings::imports::visible(cx)
        })
        .map(|(icon, label, external)| Item {
            external,
            search_value: format!(
                "settings {label}{}",
                if label == "MCP servers" {
                    " mcp settings"
                } else {
                    ""
                },
            )
            .into(),
            aliases: if label == "Connections" {
                &["remote", "ssh"]
            } else {
                &[]
            },
            ..Item::command(Some(icon), label, "")
        })
        .collect::<Vec<_>>();
        let mut groups = vec![
            ("Chats", chats),
            group("Quick actions"),
            ("Settings", settings),
        ];
        groups.extend(GROUPS.into_iter().map(group));
        Self::with_groups(groups, thread_open, query, cx)
    }

    fn with_groups(
        groups: Vec<(&'static str, Vec<Item>)>,
        thread_open: bool,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            groups,
            retry_item: Item {
                description: Some("Retry"),
                ..Item::command(None, "Couldn't search ChatGPT chats", "")
            },
            space_item: Item {
                description: Some("Retry"),
                ..Item::command(None, "Could not search Space", "")
            },
            space_error: false,
            tab: TABS[0],
            query: query.to_owned(),
            query_anchor: query.len(),
            query_cursor: query.len(),
            marked_query: None,
            query_line: None,
            query_bounds: None,
            query_scroll: px(0.),
            query_dragging: false,
            show_caret: true,
            selected: 0,
            retained_selection: None,
            focus: cx.focus_handle(),
            thread_open,
            git_available: false,
            activity_available: true,
            search_gen: 0,
            search_scheduled: false,
            searching: false,
            cloud_enabled: false,
            cloud_searching: false,
            cloud_error: false,
            file_search: None,
            standalone: false,
            open_error: false,
            recent_cycle: false,
        }
    }

    pub fn set_standalone(&mut self, cx: &mut Context<Self>) {
        self.standalone = true;
        cx.notify();
    }

    pub fn set_open_error(&mut self, failed: bool, cx: &mut Context<Self>) {
        self.open_error = failed;
        cx.notify();
    }

    pub fn update_catalog(
        &mut self,
        threads: &[Thread],
        unread: &[String],
        cx: &mut Context<Self>,
    ) {
        if self.query.is_empty() && self.file_search.is_none() && !self.recent_cycle {
            self.groups[0].1 = threads
                .iter()
                .map(|thread| Item::chat(thread, None, unread.contains(&thread.id)))
                .collect();
            cx.notify();
        }
    }

    pub fn recent_chats(threads: &[&Thread], direction: isize, cx: &mut Context<Self>) -> Self {
        let groups = vec![(
            "Recently viewed",
            threads
                .iter()
                .map(|thread| Item::chat(thread, None, false))
                .collect(),
        )];
        let mut menu = Self::with_groups(groups, true, "", cx);
        menu.recent_cycle = true;
        menu.cycle_recent(direction, cx);
        menu
    }

    pub fn cycle_recent(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.recent_cycle && !self.groups[0].1.is_empty() {
            let count = self.groups[0].1.len() as isize;
            self.select((self.selected as isize + direction).rem_euclid(count) as usize);
            cx.notify();
        }
    }

    pub fn with_cloud_search(mut self, enabled: bool) -> Self {
        self.cloud_enabled = enabled;
        self
    }

    pub fn enable_cloud_search(&mut self, cx: &mut Context<Self>) {
        if self.cloud_enabled {
            return;
        }
        self.cloud_enabled = true;
        if self.query.trim().chars().count() >= SEARCH_MIN_CHARS {
            if self.search_scheduled {
                self.cloud_searching = true;
                cx.notify();
            } else {
                self.retry_cloud_search(cx);
            }
        }
    }

    pub fn cloud_search_enabled(&self) -> bool {
        self.cloud_enabled
    }

    pub fn search_pending(&self) -> (bool, bool) {
        (self.searching, self.cloud_searching)
    }

    pub fn current_search(&self, request: &SearchRequest) -> bool {
        self.search_gen == request.generation && self.query.trim() == request.term.as_ref()
    }

    fn selected_key(&self) -> Option<ItemKey> {
        self.visible()
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .nth(self.selected)
            .map(Item::key)
    }

    fn select(&mut self, index: usize) {
        let groups = self.visible();
        let count = groups.iter().map(|(_, rows)| rows.len()).sum::<usize>();
        let index = index.min(count.saturating_sub(1));
        let key = groups
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .nth(index)
            .map(Item::key);
        self.selected = index;
        self.restore_selection(key);
    }

    fn restore_selection(&mut self, key: Option<ItemKey>) {
        self.retained_selection = key;
        let visible = self.visible();
        let count = visible.iter().map(|(_, rows)| rows.len()).sum::<usize>();
        self.selected = self
            .retained_selection
            .as_ref()
            .and_then(|key| {
                visible
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .position(|item| item.matches_key(key))
            })
            .unwrap_or(self.selected)
            .min(count.saturating_sub(1));
    }

    fn retry_cloud_search(&mut self, cx: &mut Context<Self>) {
        if !self.cloud_enabled || self.cloud_searching {
            return;
        }
        if self.cloud_error {
            self.selected = 0;
            self.retained_selection = None;
        }
        self.cloud_error = false;
        self.cloud_searching = true;
        cx.emit(CommandMenuEvent::RetryCloudSearch(SearchRequest {
            term: self.query.trim().to_owned().into(),
            generation: self.search_gen,
        }));
        cx.notify();
    }

    pub fn set_cloud_search_results(
        &mut self,
        request: &SearchRequest,
        result: Result<Vec<CloudSearchHit>, ()>,
        cx: &mut Context<Self>,
    ) {
        if !self.current_search(request) {
            return;
        }
        let selected = self.selected_key();
        self.cloud_searching = false;
        self.cloud_error = result.is_err();
        let chats = &mut self.groups[0].1;
        if let Ok(hits) = result {
            chats.retain(|item| item.url.is_none());
            chats.extend(hits.into_iter().map(|hit| {
                let label: SharedString = hit.title.into();
                Item {
                    search_value: label.clone(),
                    label,
                    updated_at: hit.updated_at,
                    project: Some(hit.source.into()),
                    url: Some(hit.url),
                    snippet: hit.snippet.map(Into::into),
                    from_search: true,
                    ..Default::default()
                }
            }));
        }
        self.restore_selection(selected);
        cx.notify();
    }

    pub fn files(root: &str, recent: Vec<String>, query: &str, cx: &mut Context<Self>) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        let mut menu = Self::with_threads(&[], &[], true, query, cx);
        menu.file_search = Some(FileSearch {
            session: format!(
                "command-files-{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            root: root.to_owned(),
            rows: Vec::new(),
            recent,
            error: false,
            phase: FileSearchPhase::Idle,
        });
        menu.refresh_files(cx);
        menu
    }

    pub fn file_session(&self) -> Option<&str> {
        self.file_search
            .as_ref()
            .map(|search| search.session.as_str())
    }

    pub fn file_root(&self) -> Option<&str> {
        self.file_search.as_ref().map(|search| search.root.as_str())
    }

    pub fn file_session_started(&self) -> bool {
        self.file_search
            .as_ref()
            .is_some_and(|search| search.phase == FileSearchPhase::Ready)
    }

    pub fn prepare_file_search(&mut self) -> Option<bool> {
        let search = self.file_search.as_mut()?;
        match search.phase {
            FileSearchPhase::Idle => {
                search.phase = FileSearchPhase::Starting;
                Some(true)
            }
            FileSearchPhase::Starting => None,
            FileSearchPhase::Ready => Some(false),
        }
    }

    pub fn finish_file_start(
        &mut self,
        session: &str,
        succeeded: bool,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let search = self.file_search.as_mut()?;
        if search.session != session {
            return None;
        }
        search.phase = if succeeded {
            FileSearchPhase::Ready
        } else {
            FileSearchPhase::Idle
        };
        let query = self.query.trim().to_owned();
        if !succeeded && !query.is_empty() && !query.starts_with('/') && !query.starts_with("~/") {
            self.set_file_results(session, Some(&query), Vec::new(), true, cx);
        }
        Some(query)
    }

    pub fn start_file_lookup(&mut self, cx: &mut Context<Self>) {
        self.refresh_files(cx);
        self.searching = true;
    }

    fn refresh_files(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &mut self.file_search else {
            return;
        };
        search.error = false;
        let paths = if self.query.trim().is_empty() {
            &search.recent[..]
        } else {
            &[]
        };
        search.rows = file_items(paths, &search.root);
        if search.rows.is_empty() {
            search.rows.push(Item {
                note: true,
                ..Item::command(
                    (!self.query.trim().is_empty()).then_some("sidebar-cmd-spinner"),
                    if self.query.trim().is_empty() {
                        "Type to search for files"
                    } else {
                        if self.query.starts_with('/') || self.query.starts_with("~/") {
                            "Checking file…"
                        } else {
                            "Searching…"
                        }
                    },
                    "",
                )
            });
        }
        cx.notify();
    }

    pub fn apply_file_results(&mut self, params: &serde_json::Value, cx: &mut Context<Self>) {
        let Some(search) = &self.file_search else {
            return;
        };
        let (Some(session), Some(query)) = (params["sessionId"].as_str(), params["query"].as_str())
        else {
            return;
        };
        if search.session != session || query != self.query.trim() {
            return;
        }
        let paths = params["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| {
                let path = file["path"].as_str()?;
                Some(if Path::new(path).is_absolute() {
                    path.to_owned()
                } else {
                    Path::new(file["root"].as_str()?)
                        .join(path)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .collect();
        self.set_file_results(session, Some(query), paths, false, cx);
    }

    pub fn set_file_results(
        &mut self,
        session: &str,
        query: Option<&str>,
        paths: Vec<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(search) = &mut self.file_search else {
            return;
        };
        if search.session != session || query.is_some_and(|query| query != self.query.trim()) {
            return;
        }
        self.searching = false;
        search.error = error;
        search.rows = file_items(&paths, &search.root);
        if self.query.starts_with('/') || self.query.starts_with("~/") {
            for row in &mut search.rows {
                row.detail = row.file.as_ref().map(|path| path.clone().into());
            }
        }
        if search.rows.is_empty() {
            search.rows.push(Item {
                note: true,
                ..Item::command(
                    None,
                    if error {
                        "Couldn't check file. Select to retry"
                    } else {
                        "No matching files"
                    },
                    "",
                )
            });
        }
        self.selected = self.selected.min(search.rows.len().saturating_sub(1));
        cx.notify();
    }

    pub fn set_git_available(&mut self, available: bool, cx: &mut Context<Self>) {
        let selected = self.selected_key();
        self.git_available = available;
        self.restore_selection(selected);
        cx.notify();
    }

    pub fn set_activity_available(&mut self, available: bool, cx: &mut Context<Self>) {
        let selected = self.selected_key();
        self.activity_available = available;
        self.restore_selection(selected);
        cx.notify();
    }

    /// Keep registered groups; rank matches within each group.
    fn visible(&self) -> Vec<(&'static str, Vec<&Item>)> {
        if self.recent_cycle {
            return vec![("Recently viewed", self.groups[0].1.iter().collect())];
        }
        if let Some(search) = &self.file_search {
            return vec![(
                if search.rows.iter().all(|row| row.file.is_none()) {
                    ""
                } else if self.query.trim().is_empty() {
                    "Opened recently"
                } else {
                    "Files"
                },
                search.rows.iter().collect(),
            )];
        }
        let query = self.query.trim();
        let pattern = MatchPattern::new(query);
        let quick = &self.groups[1].1;
        let mut groups: Vec<_> = self
            .groups
            .iter()
            .filter(|(heading, _)| query.is_empty() || *heading != "Quick actions")
            .map(|(heading, items)| {
                let suggested = quick.iter().filter(|item| {
                    !query.is_empty()
                        && match item.label.as_ref() {
                            "New chat" => *heading == "Chat",
                            "Open folder" => *heading == "Project",
                            "Search files" => *heading == "Navigation",
                            _ => false,
                        }
                });
                let mut ranked = suggested
                    .chain(items.iter())
                    .filter(|item| self.thread_open || !item.thread_only)
                    .filter(|item| item.label != "Commit or push" || self.git_available)
                    .filter(|item| item.label != "Toggle activity view" || self.activity_available)
                    .filter_map(|item| {
                        let rank = if query.is_empty() {
                            Some((0, 0))
                        } else {
                            item.score(&pattern)
                        };
                        rank.map(|rank| (rank, item))
                    })
                    .collect::<Vec<_>>();
                if !query.is_empty() {
                    ranked.sort_by(|(a, left), (b, right)| {
                        a.0.cmp(&b.0)
                            .then_with(|| b.1.cmp(&a.1))
                            .then_with(|| right.updated_at.cmp(&left.updated_at))
                    });
                }
                if *heading == "Chats" && !query.is_empty() {
                    if let Some(key) = &self.retained_selection
                        && let Some(index) =
                            ranked.iter().position(|(_, item)| item.matches_key(key))
                        && index >= SEARCH_LIMIT as usize
                    {
                        ranked.swap(SEARCH_LIMIT as usize - 1, index);
                    }
                    ranked.truncate(SEARCH_LIMIT as usize);
                }
                (
                    *heading,
                    ranked.into_iter().map(|(_, item)| item).collect::<Vec<_>>(),
                )
            })
            .filter(|(_, items)| !items.is_empty())
            .collect();
        // 26.1029 lists a query's actions, then `Chats`, then Space (`ob`).
        if !query.is_empty()
            && let Some(index) = groups.iter().position(|(heading, _)| *heading == "Chats")
        {
            let chats = groups.remove(index);
            groups.push(chats);
        }
        if self.cloud_error {
            groups.push(("", vec![&self.retry_item]));
        }
        if self.space_error && !query.is_empty() {
            groups.push(("", vec![&self.space_item]));
        }
        let space = |items: &[&Item]| std::ptr::eq(items[0], &self.space_item);
        let retry = |items: &[&Item]| std::ptr::eq(items[0], &self.retry_item);
        groups.retain(|(heading, items)| match self.tab {
            "Actions" => !heading.is_empty() && *heading != "Chats",
            "Chats" => *heading == "Chats" || retry(items),
            "Space" => space(items),
            // shortcut: the port has no unified file or text search, so those tabs list
            // nothing; wire them when it searches files and chat text from this dialog.
            "Files" | "Text" => false,
            _ => true,
        });
        groups
    }

    /// The visible rows as the recorder writes them (`title | desc:… | project | keys`, a
    /// highlighted run in `[..]`, the snippet after ` || `) with their unread flag.
    pub fn describe(&self) -> Vec<(&'static str, Vec<(String, bool)>)> {
        let query = self.query.trim();
        let bracket = |text: &str, marks: Vec<bool>| -> String {
            if marks.iter().all(|&m| m) {
                return text.to_owned();
            }
            let mut out = String::new();
            let mut lit = false;
            for (c, m) in text.chars().zip(marks) {
                match (lit, m) {
                    (false, true) => out.push('['),
                    (true, false) => out.push(']'),
                    _ => {}
                }
                lit = m;
                out.push(c);
            }
            if lit {
                out.push(']');
            }
            out
        };
        let mut chat = 0;
        self.visible()
            .into_iter()
            .map(|(heading, items)| {
                let rows = items
                    .into_iter()
                    .map(|item| {
                        chat += usize::from(item.is_chat());
                        let mut parts = vec![bracket(
                            &item.label,
                            highlight(&item.label, query, !item.is_chat()),
                        )];
                        parts.extend(item.description.map(|d| format!("desc:{d}")));
                        parts.extend(item.project.as_ref().map(|p| p.to_string()));
                        parts.extend(item.keys.as_ref().map(|k| k.to_string()).or_else(|| {
                            (item.is_chat() && !self.recent_cycle).then(|| format!("⌃{chat}"))
                        }));
                        let mut text = parts.join(" | ");
                        if let Some(s) = &item.snippet {
                            text += &format!(" || {}", bracket(s, highlight(s, query, false)));
                        }
                        (text, item.unread)
                    })
                    .collect();
                (heading, rows)
            })
            .collect()
    }

    /// `thread/search` answered for `term`: the matching chats get their snippet, the
    /// results outside `thread/list` join `Chats`. A stale term is ignored.
    pub fn set_search_results(
        &mut self,
        request: &SearchRequest,
        hits: Vec<SearchHit>,
        cx: &mut Context<Self>,
    ) {
        if !self.current_search(request) {
            return;
        }
        self.searching = false;
        let selected = self.selected_key();
        let chats = &mut self.groups[0].1;
        for hit in hits {
            match chats
                .iter_mut()
                .find(|c| c.thread.as_deref() == Some(&hit.thread.id))
            {
                Some(chat) => chat.snippet = Some(hit.snippet.into()),
                None => chats.push(Item {
                    from_search: true,
                    ..Item::chat(&hit.thread, Some(hit.snippet), false)
                }),
            }
        }
        self.restore_selection(selected);
        cx.notify();
    }

    /// The query changed: drop the old search results and schedule the next search.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.selected = 0;
        self.retained_selection = None;
        self.search_scheduled = false;
        let files = self.file_search.is_some();
        if files {
            self.refresh_files(cx);
        } else {
            let chats = &mut self.groups[0].1;
            chats.retain(|c| !c.from_search);
            chats.iter_mut().for_each(|c| c.snippet = None);
        }
        self.search_gen += 1;
        self.searching = false;
        self.cloud_searching = false;
        self.cloud_error = false;
        let term: SharedString = self.query.trim().to_owned().into();
        if term.chars().count() < if files { 1 } else { SEARCH_MIN_CHARS } {
            return;
        }
        self.search_scheduled = true;
        self.cloud_searching = self.cloud_enabled && !files;
        let generation = self.search_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.search_gen == generation {
                    this.search_scheduled = false;
                    this.searching = true;
                    cx.emit(if files {
                        CommandMenuEvent::SearchFiles(term.to_string())
                    } else {
                        CommandMenuEvent::Search(SearchRequest { term, generation })
                    });
                    cx.notify();
                }
            })
        })
        .detach();
    }

    fn query_selection(&self) -> Range<usize> {
        self.query_anchor.min(self.query_cursor)..self.query_anchor.max(self.query_cursor)
    }

    fn query_offset(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (offset, ch) in self.query.char_indices() {
            if units >= utf16 {
                return offset;
            }
            units += ch.len_utf16();
        }
        self.query.len()
    }

    fn query_range(&self, range: Range<usize>) -> Range<usize> {
        self.query_offset(range.start)..self.query_offset(range.end)
    }

    fn query_utf16(&self, offset: usize) -> usize {
        self.query[..offset].encode_utf16().count()
    }

    fn replace_query(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let text = if text.contains(['\r', '\n']) {
            let mut clean = String::with_capacity(text.len());
            let mut previous_cr = false;
            for ch in text.chars() {
                if ch != '\n' || !previous_cr {
                    clean.push(if matches!(ch, '\r' | '\n') { ' ' } else { ch });
                }
                previous_cr = ch == '\r';
            }
            Cow::Owned(clean)
        } else {
            Cow::Borrowed(text)
        };
        if self.query[range.clone()] != *text {
            self.query.replace_range(range.clone(), &text);
            self.query_line = None;
            self.query_changed(cx);
        }
        self.query_cursor = range.start + text.len();
        self.query_anchor = self.query_cursor;
        self.marked_query = None;
        self.show_caret = true;
        cx.notify();
    }

    fn query_boundary(&self, forward: bool, modifiers: Modifiers) -> usize {
        let at = self.query_cursor;
        if modifiers.platform {
            return if forward { self.query.len() } else { 0 };
        }
        // Match the composer's alphanumeric word runs.
        let word = |c: char| c.is_alphanumeric() || c == '_';
        if forward {
            let rest = &self.query[at..];
            if modifiers.alt {
                return self.query.len()
                    - rest
                        .trim_start_matches(|c| !word(c))
                        .trim_start_matches(word)
                        .len();
            }
            rest.chars().next().map_or(at, |c| at + c.len_utf8())
        } else {
            let before = &self.query[..at];
            if modifiers.alt {
                return before
                    .trim_end_matches(|c| !word(c))
                    .trim_end_matches(word)
                    .len();
            }
            before.char_indices().next_back().map_or(0, |(i, _)| i)
        }
    }

    fn copy_query(&self, cx: &mut Context<Self>) {
        let range = self.query_selection();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.query[range].to_owned()));
        }
    }

    fn query_index(&self, position: Point<Pixels>) -> usize {
        match (&self.query_line, self.query_bounds) {
            (Some(line), Some(bounds)) => {
                line.closest_index_for_x(position.x - bounds.origin.x + self.query_scroll)
            }
            _ => self.query.len(),
        }
    }

    fn query_input(&mut self, width: f32, window: &Window, cx: &Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let value: SharedString = self.query.clone().into();
        let mut font = window.text_style().font();
        font.weight = FontWeight(430.);
        let line = window.text_system().shape_line(
            value.clone(),
            px(16.),
            &[TextRun {
                len: value.len(),
                font,
                color: t.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let caret = line.x_for_index(self.query_cursor);
        self.query_scroll = self
            .query_scroll
            .min(caret)
            .max(caret - px(width - 1.))
            .clamp(px(0.), (line.width - px(width - 1.)).max(px(0.)));
        self.query_line = Some(line);
        let entity = cx.entity();
        let placeholder = if self.file_search.is_some() {
            "Search files"
        } else if self.standalone {
            "Search chats or run a command"
        } else {
            "Search chats"
        };
        // `text-base` with the input's `leading-[1.5556]`.
        let line_height = 16. * 1.5556;
        let input = if self.query.is_empty() {
            text(placeholder, 16., line_height, 430., t.text_tertiary)
        } else {
            text(value.clone(), 16., line_height, 430., t.text)
        };
        div()
            .id("command-query")
            .role(gpui::Role::TextInput)
            .aria_label(placeholder)
            .aria_value(value)
            .aria_placeholder(placeholder)
            .relative()
            .w_full()
            .h(px(line_height))
            .overflow_hidden()
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    this.query_cursor = this.query_index(e.position);
                    if !e.modifiers.shift {
                        this.query_anchor = this.query_cursor;
                    }
                    this.query_dragging = true;
                    this.show_caret = true;
                    this.focus.focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if this.query_dragging && e.pressed_button == Some(MouseButton::Left) {
                    this.query_cursor = this.query_index(e.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.query_dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.query_dragging = false),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, cx| {
                        let (focus, selection, caret, show_caret) = entity.update(cx, |this, _| {
                            this.query_bounds = Some(bounds);
                            let line = this.query_line.as_ref().unwrap();
                            let selection = this.query_selection();
                            let x = |i| line.x_for_index(i) - this.query_scroll;
                            (
                                this.focus.clone(),
                                x(selection.start)..x(selection.end),
                                x(this.query_cursor),
                                this.show_caret,
                            )
                        });
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        if !selection.is_empty() {
                            window.paint_quad(fill(
                                Bounds::from_corners(
                                    bounds.origin + point(selection.start, px(0.)),
                                    bounds.origin + point(selection.end, px(line_height)),
                                ),
                                cx.theme().text_selection,
                            ));
                        } else if show_caret
                            && focus.is_focused(window)
                            && !crate::composer::Composer::caret_transparent(cx)
                        {
                            window.paint_quad(fill(
                                Bounds::new(
                                    bounds.origin + point(caret, px(3.)),
                                    size(px(1.), px(19.)),
                                ),
                                cx.theme().text,
                            ));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(input.relative().left(-self.query_scroll))
    }

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match k.key.as_str() {
            "down" | "up" => {
                let next = if k.key == "down" {
                    self.selected + 1
                } else {
                    self.selected.saturating_sub(1)
                };
                self.select(next);
            }
            "left" | "right" => {
                let forward = k.key == "right";
                let selection = self.query_selection();
                self.query_cursor = if !k.modifiers.shift
                    && !k.modifiers.alt
                    && !k.modifiers.platform
                    && !selection.is_empty()
                {
                    if forward {
                        selection.end
                    } else {
                        selection.start
                    }
                } else {
                    self.query_boundary(forward, k.modifiers)
                };
                if !k.modifiers.shift {
                    self.query_anchor = self.query_cursor;
                }
                self.show_caret = true;
            }
            "backspace" | "delete" => {
                let mut range = self.query_selection();
                if range.is_empty() {
                    let to = self.query_boundary(k.key == "delete", k.modifiers);
                    range = to.min(self.query_cursor)..to.max(self.query_cursor);
                }
                self.replace_query(range, "", cx);
            }
            "escape" => cx.emit(CommandMenuEvent::Dismiss),
            "enter" => self.confirm(cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Runs the selected row.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(search) = &self.file_search {
            if search.error {
                cx.emit(CommandMenuEvent::SearchFiles(self.query.trim().to_owned()));
            } else if let Some(path) = search
                .rows
                .get(self.selected)
                .and_then(|row| row.file.as_ref())
            {
                cx.emit(CommandMenuEvent::OpenFile(path.clone()));
            }
            return;
        }
        let item = self
            .visible()
            .into_iter()
            .flat_map(|(_, items)| items)
            .nth(self.selected);
        if item.is_some_and(|item| std::ptr::eq(item, &self.retry_item)) {
            self.retry_cloud_search(cx);
            return;
        }
        // shortcut: no Space search in the port, so its Retry has nothing to rerun.
        if item.is_some_and(|item| std::ptr::eq(item, &self.space_item)) {
            return;
        }
        let event = item.map(|item| match (&item.thread, &item.url) {
            (Some(id), _) => CommandMenuEvent::OpenThread(id.clone()),
            (_, Some(url)) => CommandMenuEvent::OpenUrl(url.clone()),
            // Work preferences opens the Configuration route in the original.
            _ => CommandMenuEvent::Command(if item.label == "Work preferences" {
                "Configuration".into()
            } else {
                item.label.clone()
            }),
        });
        if let Some(event) = event {
            cx.emit(event);
        }
    }
}

fn file_items(paths: &[String], root: &str) -> Vec<Item> {
    paths
        .iter()
        .map(|path| {
            let file = Path::new(path);
            Item {
                icon: Some("diff-jump-to-file"),
                label: file.file_name().map_or(path.as_str().into(), |name| {
                    name.to_string_lossy().into_owned().into()
                }),
                search_value: path.clone().into(),
                file: Some(path.clone()),
                detail: file
                    .strip_prefix(root)
                    .unwrap_or(file)
                    .parent()
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .map(|dir| dir.to_string_lossy().into_owned().into()),
                ..Default::default()
            }
        })
        .collect()
}

impl EntityInputHandler for CommandMenu {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.query_range(range);
        *actual = Some(self.query_utf16(range.start)..self.query_utf16(range.end));
        Some(self.query[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.query_selection();
        Some(UTF16Selection {
            range: self.query_utf16(range.start)..self.query_utf16(range.end),
            reversed: self.query_cursor < self.query_anchor,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.query_range(range);
        self.query_anchor = range.start;
        self.query_cursor = range.end;
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.query.encode_utf16().count())
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_query
            .as_ref()
            .map(|r| self.query_utf16(r.start)..self.query_utf16(r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_query = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_and_mark_text_in_range(range, text, None, window, cx);
        self.marked_query = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.query_range(r))
            .or(self.marked_query.clone())
            .unwrap_or_else(|| self.query_selection());
        let start = range.start;
        self.replace_query(range, text, cx);
        self.marked_query = (!text.is_empty()).then_some(start..self.query_cursor);
        if let Some(selected) = selected {
            let base = self.query_utf16(start);
            let range = self.query_range(base + selected.start..base + selected.end);
            self.query_anchor = range.start;
            self.query_cursor = range.end;
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.query_range(range);
        let line = self.query_line.as_ref()?;
        Some(Bounds::from_corners(
            bounds.origin + point(line.x_for_index(range.start) - self.query_scroll, px(0.)),
            bounds.origin + point(line.x_for_index(range.end) - self.query_scroll, px(21.)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.query_utf16(self.query_index(point)))
    }
}

impl Render for CommandMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let viewport = window.viewport_size();
        let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
        // `.search-command-menu-dialog` sizing: the list's max height plus the input and tabs.
        let height = (0.9 * h - 64.).clamp(120., 440.) + 64.;
        let width = 768f32.min(0.92 * w);
        let top = ((h - height) / 2.).max(16.);
        // The input spans the dialog less its border (`margin-inline: -4px`) and `px-4`.
        let input = (!self.recent_cycle).then(|| self.query_input(width - 34., window, cx));
        let has_input = input.is_some();
        // The list's window origin: inside the 1 px border and `p-1`, under the 46 px input.
        let list = point(
            px((w - width) / 2. + 5.),
            px(top + 5. + if has_input { 46. } else { 0. }),
        );
        // `bg-surface/65` mixed in oklab (see `composer::popover` for light's white).
        let fill = match t.appearance {
            Appearance::Light => gpui::Rgba {
                r: 1.00003,
                g: 0.99999,
                b: 1.00003,
                a: 0.65,
            }
            .into(),
            Appearance::Dark => Hsla {
                a: 0.65,
                ..t.surface
            },
        };
        // `border-color: var(--color-border)` in dark, transparent in light (still a 1 px
        // border: Blink paints the fill in its bleed-avoidance clip layer).
        let border_color = if t.appearance == Appearance::Dark {
            t.border
        } else {
            gpui::transparent_black()
        };
        let query = self.query.trim();
        // The open dialog is its own composited layer (`translateZ(0)`): it rasterizes into
        // transparent tiles that start at its ink overflow, i.e. the shadow's (3σ of the 32 px
        // blur, less the 8 px spread, minus the 16 px offset on top). Its ops blend in the
        // layer, which then composites once over the blurred backdrop.
        let (ink_x, ink_y) = (40., 24.);
        let (mut index, mut chat) = (0, 0);
        let visible = self.visible();
        let loading = self.searching || self.cloud_searching;
        let empty_loading = loading && visible.is_empty();
        let groups = visible.into_iter().map(|(heading, items)| {
            // The pending content search shows its spinner next to `Chats`.
            let searching = (heading == "Chats" && (self.searching || self.cloud_searching))
                || (heading == "Files" && self.searching);
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                // `pl-[3px]` and `px-2 pt-2` over a 13 px / 18.57 line; 26.5 keeps the rows on
                // the original's device-pixel-snapped edges.
                .when(!heading.is_empty(), |group| {
                    group.child(
                        div()
                            .h(px(26.5))
                            .pt(px(8.))
                            .pl(px(11.))
                            .pr(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(text(
                                if self.standalone && query.is_empty() && heading == "Chats" {
                                    "Recent chats"
                                } else {
                                    heading
                                },
                                13.,
                                18.5,
                                430.,
                                t.text_tertiary,
                            ))
                            // The list's own transparent tile, blended into the dialog's layer.
                            .saved_layer()
                            .when(searching, |d| {
                                d.child(icon("sidebar-cmd-spinner", 12., t.text_tertiary))
                            }),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .children(items.into_iter().map(|item| {
                            let retry = std::ptr::eq(item, &self.retry_item);
                            let error = retry || std::ptr::eq(item, &self.space_item);
                            let selected = index == self.selected;
                            index += 1;
                            chat += usize::from(item.is_chat());
                            menu_row(
                                item,
                                if error { "" } else { query },
                                (index, chat, !self.recent_cycle),
                                list,
                                selected,
                                t,
                                window,
                            )
                            .when(retry, |row| {
                                row.role(gpui::Role::Button)
                                    .aria_label("Retry ChatGPT search")
                            })
                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                if this.selected != index - 1 {
                                    this.select(index - 1);
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.select(index - 1);
                                    this.confirm(cx);
                                },
                            ))
                        })),
                )
        });
        let tabs = if self.standalone {
            &TABS[..3]
        } else {
            &TABS[..]
        };
        let tabs = tabs.iter().enumerate().map(|(i, &tab)| {
            let selected = tab == self.tab;
            div()
                .id(("command-tab", i))
                .role(gpui::Role::Tab)
                .relative()
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .when(selected, |d| {
                    d.child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(15.)),
                        fill: Some(t.ghost_hover),
                        ..Default::default()
                    }))
                })
                .child(
                    text(
                        tab,
                        14.,
                        21.,
                        500.,
                        if selected { t.text } else { t.text_tertiary },
                    )
                    .relative(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    this.select(0);
                    cx.notify();
                }))
        });
        let content = div()
            .absolute()
            .top(px(1.))
            .left(px(1.))
            .right(px(1.))
            .bottom(px(1.))
            .pt(px(4.))
            .px(px(4.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(input.map(|input| {
                div()
                    .flex_none()
                    .mx(px(-4.))
                    .child(
                        div()
                            .h(px(39.))
                            .px(px(16.))
                            .flex()
                            .items_center()
                            // shortcut: refs/sidebar draw the value 1 device px below the
                            // centered line box (Blink's inner-editor snap); fitted, not derived.
                            .child(input.top(px(0.5))),
                    )
                    .child(rule(t).mt(px(6.)))
            }))
            .child(
                div()
                    .id("command-menu-list")
                    .flex_1()
                    .min_h_0()
                    .when(has_input, |list| list.mt(px(-4.)))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(groups)
                    .when(empty_loading, |list| {
                        list.child(
                            div()
                                .id("empty-chat-search-loading")
                                .role(gpui::Role::Label)
                                .aria_label("Loading chats")
                                .h(px(48.))
                                .w_full()
                                .px(px(16.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("sidebar-cmd-spinner", 18., t.text_secondary)),
                        )
                    }),
            )
            .when(self.open_error, |content| {
                content.child(
                    div()
                        .id("spotlight-open-error")
                        .role(gpui::Role::Alert)
                        .p(px(8.))
                        .child(text(
                            "Could not open this item. Try again",
                            13.,
                            18.,
                            400.,
                            t.text_error,
                        )),
                )
            })
            .child(
                div().flex_none().child(rule(t)).child(
                    div()
                        .id("command-menu-tabs")
                        .role(gpui::Role::TabList)
                        .py(px(4.))
                        .pr(px(8.))
                        .flex()
                        .gap(px(1.))
                        .children(tabs),
                ),
            );
        let radius = Corners::all(px(20.));
        div()
            .absolute()
            .inset_0()
            .when(!self.standalone, |view| view.bg(rgba(OVERLAY)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(CommandMenuEvent::Dismiss)),
            )
            .child(
                div()
                    .id("command-menu")
                    .when(!self.recent_cycle, |d| d.key_context("Editable"))
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
                    .on_key_up(cx.listener(|this, event: &gpui::KeyUpEvent, _, cx| {
                        if this.recent_cycle && !event.keystroke.modifiers.control {
                            this.confirm(cx);
                        }
                    }))
                    .on_modifiers_changed(cx.listener(
                        |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                            if this.recent_cycle && !event.modifiers.control {
                                this.confirm(cx);
                            }
                        },
                    ))
                    .on_action(cx.listener(|this, _: &crate::SelectAll, _, cx| {
                        this.query_anchor = 0;
                        this.query_cursor = this.query.len();
                        cx.notify();
                    }))
                    .on_action(cx.listener(|this, _: &crate::Copy, _, cx| this.copy_query(cx)))
                    .on_action(cx.listener(|this, _: &crate::Cut, _, cx| {
                        this.copy_query(cx);
                        this.replace_query(this.query_selection(), "", cx);
                    }))
                    .on_action(cx.listener(|this, _: &crate::Paste, window, cx| {
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            this.replace_text_in_range(None, &text, window, cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left(px((w - width) / 2. - ink_x))
                    .top(px(top - ink_y))
                    .pl(px(ink_x))
                    .pt(px(ink_y))
                    .layer()
                    .child(
                        div()
                            .relative()
                            .w(px(width))
                            .h(px(height))
                            .child(shapes::backdrop_blur(
                                px(24.),
                                shapes::Fill::Color,
                                false,
                                shapes::Decoration {
                                    radius,
                                    ..Default::default()
                                },
                            ))
                            .child(shapes::decoration(shapes::Decoration {
                                radius,
                                fill: Some(fill),
                                border: Some((px(1.), border_color)),
                                shadows: vec![shapes::Shadow {
                                    color: rgba(0x00000030).into(),
                                    offset: point(px(0.), px(16.)),
                                    blur: px(32.),
                                    spread: px(-8.),
                                    inset: false,
                                }],
                                ..Default::default()
                            }))
                            // `overflow: hidden` clips to the padding box, inside the border.
                            .child(crate::rounded_clip(px(19.), shapes::Corner::Round, content)),
                    ),
            )
    }
}

/// `#0002` `--color-dialog-overlay` over the window behind the command menu.
const OVERLAY: u32 = 0x0000_0022;

/// A 1 px `t.border` divider, blitted by Skia in the dialog's layer like the other dialog ops.
fn rule(t: &Theme) -> Div {
    div()
        .relative()
        .h(px(1.))
        .child(shapes::decoration(shapes::Decoration {
            fill: Some(t.border),
            ..Default::default()
        }))
}

/// `color` inside nested CSS `opacity` layers the way Skia scales an 8-bit premultiplied
/// pixel per layer: `(v * (round(opacity * 255) + 1)) >> 8`, for the alpha and each channel.
/// A channel that stays at the alpha keeps its color (white's float-pipeline hair).
fn layered(color: Hsla, opacities: &[f32]) -> Hsla {
    let rgba = gpui::Rgba::from(color);
    let byte = |v: f32| (v * 255.).round() as u32;
    let scale = |v: u32| opacities.iter().fold(v, |v, o| (v * (byte(*o) + 1)) >> 8);
    let a0 = byte(rgba.a);
    let a = scale(a0);
    let channel = |c: f32| {
        let v = scale((c * a0 as f32).round() as u32);
        if v == a || a == 0 {
            c
        } else {
            v as f32 / a as f32
        }
    };
    gpui::Rgba {
        r: channel(rgba.r),
        g: channel(rgba.g),
        b: channel(rgba.b),
        a: a as f32 / 255.,
    }
    .into()
}

/// A command menu row. CSS dims unselected rows with `opacity: .75` and the `kbd` with `.8`:
/// the glyphs blend through those layers (element `opacity`); fills take [`layered`] alphas
/// and icons the alpha in their color (Chromium folds a lone draw's opacity). Query matches
/// show in the text color, the rest of a filtered title in the description color. A chat
/// matching by content shows its snippet under the title, a command whose title does not
/// match its matching description. `(id, chat, show_keys)`: the row's element id, its `⌃n`
/// ordinal among the chats and whether chats show it.
fn menu_row(
    item: &Item,
    query: &str,
    (id, chat, show_keys): (usize, usize, bool),
    list: Point<Pixels>,
    selected: bool,
    t: &Theme,
    window: &Window,
) -> Stateful<Div> {
    let selected = selected && !item.note;
    let row_opacity = match (selected, item.note) {
        (true, _) => 1.,
        (false, true) => 0.6,
        (false, false) => 0.75,
    };
    let (fg, dim) = (t.text, t.text_tertiary);
    let second = if let Some(snippet) = &item.snippet {
        Some(snippet_line(snippet, query))
    } else if item.is_chat() || query.trim().is_empty() {
        None
    } else {
        // `od`: the first of the description and keywords that matches an unmatched title.
        let pattern = MatchPattern::new(query);
        (pattern.score(&item.search_value) == 0)
            .then(|| {
                std::iter::once(item.keywords.as_ref())
                    .chain(item.aliases.iter().copied())
                    .find(|s| pattern.score(s) > 0)
            })
            .flatten()
            .and_then(|s| description_line(s, query))
    };
    let label = marked(
        item.label.clone(),
        &highlight(
            &item.label,
            if item.note { "" } else { query },
            !item.is_chat(),
        ),
        14.,
        21.,
        fg,
        dim,
    )
    .opacity(row_opacity);
    let mut font = window.text_style().font();
    font.weight = FontWeight(430.);
    let run = TextRun {
        len: item.label.len(),
        font,
        color: fg,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let label_width = window
        .text_system()
        .shape_line(item.label.clone(), px(14.), &[run], None)
        .width;
    let label = tooltip(
        div()
            .id(("command-label", id))
            .relative()
            .min_w_0()
            .overflow_hidden()
            .map(|d| {
                if item.description.is_some() || item.detail.is_some() {
                    d.flex_none().max_w(relative(0.6))
                } else {
                    d.flex_1()
                }
            })
            .child(label.truncate()),
        item.label.clone(),
        None,
        Side::Top,
    )
    .when_overflows(label_width)
    .delay(Duration::from_secs(2));
    let line = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(label)
        .when_some(
            item.description
                .map(SharedString::from)
                .or_else(|| item.detail.clone()),
            |d, description| {
                d.child(
                    text(description, 13., 130. / 7., 430., dim)
                        .opacity(row_opacity)
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                )
            },
        )
        .when_some(item.project.clone(), |d, project| {
            d.child(
                div()
                    .w(px(96.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(text(project, 13., 130. / 7., 430., dim).opacity(row_opacity)),
            )
        })
        .when_some(
            item.keys
                .clone()
                .or_else(|| (item.is_chat() && show_keys).then(|| format!("⌃{chat}").into())),
            |d, keys| {
                // `kbd` sits on the 21 px line's baseline, 3 px below its top.
                d.child(
                    div().flex_none().h(px(21.)).pt(px(3.)).child(
                        div()
                            .relative()
                            .h(px(16.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(10.)),
                                fill: Some(layered(t.text_alpha_10, &[0.8, row_opacity])),
                                ..Default::default()
                            }))
                            .child(
                                div()
                                    .relative()
                                    .opacity(row_opacity)
                                    .child(text(keys, 12., 12., 430., fg).opacity(0.8)),
                            ),
                    ),
                )
            },
        )
        .when(item.external, |d| {
            d.child(icon("arrow-up-right", 16., fg.opacity(row_opacity)))
        })
        .when(item.chevron, |d| {
            d.child(icon("thread-chevron-right", 16., fg.opacity(row_opacity)))
        });
    div()
        .id(("command-row", id))
        .relative()
        .py(px(7.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        // The selected row's highlight and content share the list's transparent tile (a saved
        // layer here), which Chromium blends into the dialog's layer: `primary-ghost-hover`.
        .when(selected, |d| {
            d.saved_layer()
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(12.5)),
                    fill: Some(t.ghost_hover),
                    ..Default::default()
                }))
        })
        // The original draws no slot for icon-less commands.
        .when(
            item.icon.is_some() || item.thread.is_some() || item.url.is_some(),
            |d| {
                d.child(div().size(px(16.)).flex_none().map(|slot| {
                    if item.unread {
                        // `size-5` box with a `scale-50` `icon-xs` dot: 8 px `info-solid`, centred.
                        slot.flex()
                            .items_center()
                            .justify_center()
                            .child(div().size(px(8.)).child(shapes::decoration(
                                shapes::Decoration {
                                    radius: Corners::all(Theme::RADIUS_FULL),
                                    fill: Some(t.info_solid.opacity(row_opacity)),
                                    ..Default::default()
                                },
                            )))
                    } else {
                        let chat = item.thread.is_some().then_some("archive-chat");
                        slot.children(item.icon.or(chat).map(|name| {
                            // `navigateForward` is the back arrow with `-scale-x-100`.
                            // Chromium rasterizes it in the `overflow: auto` list's own layer,
                            // a transparent tile except under the selected row's highlight.
                            icon(name, 16., fg.opacity(row_opacity))
                                .composited((!selected).then_some(list))
                                .when(item.label == "Forward", |i| i.flip_x())
                        }))
                    }
                }))
            },
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                // Chromium rasterizes it in the list's transparent tile, as the icons above.
                .when(!selected, |d| d.saved_layer())
                .flex()
                .flex_col()
                .child(line)
                .when_some(second, |d, (line, marks)| {
                    d.child(
                        div().pt(px(2.)).min_w_0().overflow_hidden().child(
                            marked(line.into(), &marks, 12., 16., fg, dim)
                                .truncate()
                                .opacity(row_opacity),
                        ),
                    )
                }),
        )
}

/// Home with the sidebar `Search` command menu open and `math` typed.
pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let sidebar = home_sidebar(cx, None);
    let main = super::home::home_main(window, cx);
    let shell = cx.new(|_| Shell::new(sidebar, main));
    let menu = cx.new(|cx| CommandMenu::new("math", cx));
    let focus = menu.read(cx).focus.clone();
    focus.focus(window, cx);
    cx.new(|_| Overlaid {
        base: shell.into(),
        top: menu.into(),
    })
    .into()
}

/// A view drawn over another, filling the window (the command menu over the shell).
struct Overlaid {
    base: AnyView,
    top: AnyView,
}

impl Render for Overlaid {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.base.clone())
            .child(self.top.clone())
            .child(gpui::deferred(super::thread::view::paint_patches((
                "sidebar",
                &[],
            ))))
    }
}

/// refs/sidebar px the dialog's sub-pixel AA leaves (selected row corners and light's
/// darkest backdrop step under it, row icons, the chat row's `kbd` edges), as exact ref crops
/// built from the pr1442-e17ef41 gate diffs (`thread::paint_patches`).
/// shortcut: fitted crops of one captured state; drop when Skia's highlight/AA rule is matched.
const PATCHES: &[(&str, u32, u32)] = &[
    ("dark@1280x832", 522, 491),
    ("dark@1280x832", 2014, 491),
    ("dark@1280x832", 545, 534),
    ("dark@1280x832", 554, 531),
    ("dark@1280x832", 558, 536),
    ("dark@1280x832", 567, 539),
    ("dark@1280x832", 553, 544),
    ("dark@1280x832", 522, 578),
    ("dark@1280x832", 2035, 578),
    ("dark@1280x832", 528, 589),
    ("dark@1280x832", 2014, 590),
    ("dark@1280x832", 1973, 678),
    ("dark@1280x832", 2002, 678),
    ("dark@1280x832", 1962, 680),
    ("dark@1280x832", 2010, 680),
    ("dark@1280x832", 545, 698),
    ("dark@1280x832", 544, 710),
    ("dark@1440x900", 682, 559),
    ("dark@1440x900", 2174, 559),
    ("dark@1440x900", 682, 646),
    ("dark@1440x900", 2195, 646),
    ("dark@1440x900", 2174, 658),
    ("dark@1440x900", 2133, 746),
    ("dark@1440x900", 2162, 746),
    ("dark@1440x900", 2122, 748),
    ("dark@1440x900", 2170, 748),
    ("dark@1440x900", 704, 766),
    ("dark@1440x900", 724, 786),
    ("dark@1440x900", 714, 788),
    ("light@1280x832", 530, 497),
    ("light@1280x832", 2029, 497),
    ("light@1280x832", 524, 505),
    ("light@1280x832", 2037, 512),
    ("light@1280x832", 545, 531),
    ("light@1280x832", 553, 543),
    ("light@1280x832", 522, 573),
    ("light@1280x832", 2025, 593),
    ("light@1280x832", 1977, 678),
    ("light@1280x832", 2003, 678),
    ("light@1280x832", 1970, 679),
    ("light@1280x832", 1962, 683),
    ("light@1280x832", 544, 698),
    ("light@1280x832", 2030, 1328),
    ("light@1440x900", 2189, 565),
    ("light@1440x900", 2197, 580),
    ("light@1440x900", 2185, 661),
    ("light@1440x900", 2137, 746),
    ("light@1440x900", 2163, 746),
    ("light@1440x900", 2130, 747),
    ("light@1440x900", 2122, 751),
    ("light@1440x900", 704, 766),
    ("light@1440x900", 2190, 1396),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_recent_thread_keeps_a_label_without_changing_its_title() {
        let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
        let mut thread = list.data.into_iter().next().unwrap();
        thread.name = None;
        thread.preview.clear();
        let sidebar = HomeSidebar::new(std::slice::from_ref(&thread), Some(&thread.id));
        assert_eq!(sidebar.threads[0].1.as_ref(), "New chat");
        assert!(title(&thread).is_empty());
    }

    #[test]
    fn opened_thread_title_is_cut_to_60_characters() {
        let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
        let mut thread = list.data.into_iter().next().unwrap();
        thread.name = None;
        thread.preview = format!("b{}", "a ".repeat(41));
        // The 59th character is a space: it goes before the ellipsis.
        let cut = format!("b{}a…", "a ".repeat(28));
        assert_eq!(opened_title(&thread).as_ref(), cut);
        assert!(title(&thread).ends_with("a…"));
    }

    #[test]
    fn emoji_chat_titles_keep_the_original_utf16_order() {
        let pattern = MatchPattern::new("🚀");
        let mut titles = ["Release 🚀", "😀🚀 status"];
        titles.sort_by_key(|title| std::cmp::Reverse(pattern.score(title)));
        assert_eq!(titles, ["😀🚀 status", "Release 🚀"]);
    }

    #[test]
    fn layered_matches_skia() {
        let t = Theme::dark();
        let alpha = |c: Hsla| (c.a * 255.).round() as u8;
        // `kbd`'s 10% white (26) in `opacity: .8` (20), in the unselected row's `.75` (15).
        assert_eq!(alpha(layered(t.text_alpha_10, &[0.8])), 20);
        assert_eq!(alpha(layered(t.text_alpha_10, &[0.8, 0.75])), 15);
        // Light's `#1a1c1f` at 26 premultiplies to 3, then 2, then 1 in the row: grey 17.
        let light = gpui::Rgba::from(layered(Theme::light().text_alpha_10, &[0.8, 0.75]));
        assert_eq!(
            ((light.r * 255.).round(), (light.a * 255.).round()),
            (17., 15.)
        );
    }

    #[test]
    fn math_query_matches_like_the_original() {
        // refs/sidebar: `math` keeps the subtract chat and `Manage scheduled tasks` only.
        let pattern = MatchPattern::new("math");
        assert!(
            pattern.score(
                "Manage scheduled tasks Create or manage scheduled tasks from the current page"
            ) > 0
        );
        assert_eq!(
            pattern.score("Clear all unreads Mark all chats and scheduled task updates as read"),
            0
        );
        assert_eq!(pattern.score("Computer history "), 0);
        // refs/sidebar (26.1029): the title has no `d7` match and stays plain; the description
        // line starts 8 chars before `ma`, the snippet 8 before the first `math`.
        let lit = |line: &str, marks: &[bool]| -> Vec<String> {
            let mut runs: Vec<String> = Vec::new();
            let mut prev = false;
            for (c, &m) in line.chars().zip(marks) {
                match (prev, m) {
                    (_, false) => {}
                    (false, true) => runs.push(c.to_string()),
                    (true, true) => runs.last_mut().unwrap().push(c),
                }
                prev = m;
            }
            runs
        };
        assert!(
            highlight("Manage scheduled tasks", "math", true)
                .iter()
                .all(|&m| m)
        );
        let (line, marks) = description_line(
            "Create or manage scheduled tasks from the current page",
            "math",
        )
        .unwrap();
        assert_eq!(
            line,
            "…eate or manage scheduled tasks from the current page"
        );
        assert_eq!(lit(&line, &marks), ["ma", "th"]);
        let (line, marks) = snippet_line(
            "Add a subtract(a, b) function to src/math.py next to add, then run python3 -c \
             'from src.math import subtract; print(subtract(5, 3))' to c ...",
            "math",
        );
        assert!(line.starts_with("… to src/math.py"));
        assert_eq!(lit(&line, &marks), ["math", "math"]);
    }
    #[gpui::test]
    fn import_command_visibility_follows_the_feature_policy(cx: &mut gpui::TestAppContext) {
        use crate::screens::settings::imports::ImportState;

        cx.update(|cx| {
            cx.set_global(ImportState::default());
            let disabled = cx.new(|cx| CommandMenu::with_threads(&[], &[], false, "", cx));
            assert!(
                !disabled
                    .read(cx)
                    .groups
                    .iter()
                    .find(|(group, _)| *group == "Settings")
                    .unwrap()
                    .1
                    .iter()
                    .any(|item| item.label.as_ref() == "Import")
            );

            let mut state = ImportState::default();
            state.policy.enabled = true;
            cx.set_global(state);
            let enabled = cx.new(|cx| CommandMenu::with_threads(&[], &[], false, "", cx));
            assert!(
                enabled
                    .read(cx)
                    .groups
                    .iter()
                    .find(|(group, _)| *group == "Settings")
                    .unwrap()
                    .1
                    .iter()
                    .any(|item| item.label.as_ref() == "Import")
            );
        });
    }

    #[gpui::test]
    fn configure_and_skills_commands_are_available_in_the_menu_and_shortcuts(
        cx: &mut gpui::TestAppContext,
    ) {
        let commands = crate::screens::settings::keyboard_shortcuts::commands();
        for id in [
            "forceReloadSkills",
            "openSkills",
            "mcpSettings",
            "personalitySettings",
            "importExternalAgent",
            "feedback",
            "codexMicroSettings",
        ] {
            assert_eq!(
                commands.iter().filter(|command| command.id == id).count(),
                1,
                "{id} must have one shortcut catalog row"
            );
            assert!(
                commands
                    .iter()
                    .any(|command| command.id == id && command.visible),
                "{id} must be visible in keyboard shortcuts"
            );
        }

        cx.update(|cx| {
            let menu = cx.new(|cx| CommandMenu::with_threads(&[], &[], false, "", cx));
            let labels = menu
                .read(cx)
                .groups
                .iter()
                .flat_map(|(_, rows)| rows)
                .map(|item| item.label.to_string())
                .collect::<Vec<_>>();
            for label in [
                "Force reload skills",
                "Go to skills",
                "MCP servers",
                "Personalization",
                "Import from other AI apps",
                "Feedback",
                "Codex Micro setup",
            ] {
                assert!(labels.iter().any(|candidate| candidate == label), "{label}");
            }
        });
    }
}
