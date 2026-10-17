//! `sidebar` parity screen (Home with chat search open) and the Home sidebar every chat route
//! shows: header, Projects, and Recents from `thread/list`. Numbers are CSS px from
//! refs/home and refs/sidebar.

use crate::{
    Shell, blit, fixture, focus, home_sidebar_header,
    icon::icon,
    sidebar_empty, text,
    tooltip::{Side, tooltip},
};
use bridge::types::{Thread, ThreadListResponse, ThreadStatus};
use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Context, Corners, Div, EventEmitter,
    FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Rgba, ScrollHandle, SharedString,
    Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun, Transformation, Window,
    canvas, div, point, prelude::FluentBuilder as _, px, radians, relative, rgba, svg,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::Path,
    time::{Duration, Instant},
};
use theme::{ActiveTheme as _, Appearance, Theme};

#[path = "sidebar/menu.rs"]
mod menu;
use menu::ThreadMenu;

/// `.sidebar-item`: `--radius-lg` (12.5 px) with `corner-shape: superellipse(1.5)`.
const RADIUS_ROW: Pixels = px(12.5);

pub const PINNED_SECTION_ID: &str = "01984de2-8f74-7c91-a3b2-5c5e937cf318";

/// Recents shows this many rows first and `PAGE` more each time its end comes into view
/// (the original's `maxItems: 10`, `infiniteScroll` paging of the last section).
const PAGE: usize = 10;

/// Recents row label: the thread name, else its preview cut to 80 chars with `…`.
pub fn title(thread: &Thread) -> SharedString {
    if let Some(name) = &thread.name {
        return name.clone().into();
    }
    let preview = thread.preview.trim();
    if preview.chars().count() <= 80 {
        preview.to_owned().into()
    } else {
        format!("{}…", preview.chars().take(79).collect::<String>()).into()
    }
}

pub struct HomeSidebar {
    /// `(thread id, row label)` in `thread/list` order.
    threads: Vec<(String, SharedString)>,
    selected: Option<String>,
    /// `Projects`, `Recents` collapsed by a click on their header.
    collapsed: [bool; 3],
    pinned: HashSet<String>,
    /// Threads whose rows carry the unread dot; any lights the header's bell.
    pub unread: Vec<String>,
    /// Threads with an active turn (`thread/status/changed`), by the clock reading when their
    /// spinner started.
    running: HashMap<String, Instant>,
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

/// A row's `Pin chat` / `Unpin chat`: `thread/section/move` of the thread (its id) into the
/// Pinned section (`true`) or out of it (`false`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct PinThread(pub String, pub bool);

/// The row menu's `Rename`: opens the `Rename chat` dialog for the thread (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct RenameThread(pub String);

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

impl HomeSidebar {
    /// Recents from a live `thread/list`; `selected` is the open thread, highlighted.
    pub fn new(threads: &[Thread], selected: Option<&str>) -> Self {
        Self {
            threads: threads.iter().map(|t| (t.id.clone(), title(t))).collect(),
            selected: selected.map(str::to_owned),
            collapsed: [false; 3],
            pinned: pinned_ids(threads),
            unread: Vec::new(),
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
        }
    }

    pub fn set_threads(&mut self, threads: &[Thread], cx: &mut Context<Self>) {
        self.threads = threads.iter().map(|t| (t.id.clone(), title(t))).collect();
        self.pinned = pinned_ids(threads);
        for thread in threads {
            if thread.status == ThreadStatus::Active {
                self.set_running(&thread.id, true, cx);
            }
        }
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
        let old = self.threads.iter().position(|(id, _)| *id == thread.id);
        let old = old.map(|i| self.threads.remove(i).1);
        let label = match (title(thread), old) {
            (label, Some(old)) if label.is_empty() => old,
            (label, _) => label,
        };
        self.threads.insert(0, (thread.id.clone(), label));
        self.set_running(&thread.id, thread.status == ThreadStatus::Active, cx);
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
            self.running
                .entry(id.to_owned())
                .or_insert_with(|| cx.background_executor().now());
        } else {
            self.running.remove(id);
        }
        cx.notify();
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    /// A turn completed in thread `id` while it was not open: unread dot.
    pub fn set_unread(&mut self, id: &str, cx: &mut Context<Self>) {
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
        self.page_error
    }

    /// Recents rows on screen (the first [`limit`](Self::limit) unpinned threads).
    pub fn recents_shown(&self) -> usize {
        self.recents().count().min(self.limit)
    }

    pub fn projects_collapsed(&self) -> bool {
        self.collapsed[0]
    }

    fn recents(&self) -> impl Iterator<Item = &(String, SharedString)> {
        self.threads
            .iter()
            .filter(|(id, _)| !self.pinned.contains(id))
    }

    /// The end sentinel came into view: `PAGE` more rows when they are loaded, else the next
    /// `thread/list` page from the owner.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.recents().count() > self.limit {
            self.limit += PAGE;
            cx.notify();
        } else if self.has_more && !self.loading {
            self.loading = true;
            self.page_error = false;
            cx.notify();
            cx.emit(LoadMore);
        }
    }

    fn rows(&self, pinned: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let loaded = self.recents().count();
        let rows = self
            .threads
            .iter()
            .filter(|(id, _)| self.pinned.contains(id) == pinned)
            .take(if pinned { usize::MAX } else { self.limit });
        let now = cx.background_executor().now();
        if !self.running.is_empty() {
            window.request_animation_frame();
        }
        // The original's end sentinel (`IntersectionObserver` on the list's last item): while
        // it is inside the scroller's clip, the list pages.
        let sentinel = (!pinned
            && !self.loading
            && !self.page_error
            && (loaded > self.limit || self.has_more))
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
        div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .children(rows.map(|(id, label)| {
                let open = id.clone();
                let menu_for = id.clone();
                let row_focus = focus::handle(
                    SharedString::from(format!("thread-row-{id}")),
                    0,
                    window,
                    cx,
                );
                let pin_focus =
                    focus::handle(SharedString::from(format!("pin-chat-{id}")), 0, window, cx);
                let archive_focus = focus::handle(
                    SharedString::from(format!("archive-chat-{id}")),
                    0,
                    window,
                    cx,
                );
                let focused = row_focus.read(cx).handle.contains_focused(window, cx);
                // `animate-spin`: one turn per second, linear.
                let end = self
                    .running
                    .get(id)
                    .map(|started| {
                        RowEnd::Spinner(
                            (now.saturating_duration_since(*started).as_secs_f32() % 1.) * 360.,
                        )
                    })
                    .or_else(|| self.unread.contains(id).then_some(RowEnd::Dot));
                thread_row(
                    id,
                    label.clone(),
                    self.selected.as_ref() == Some(id),
                    end,
                    cx,
                    (focused, pinned),
                    [&row_focus, &pin_focus, &archive_focus],
                )
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenThread(open.clone()))))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                        this.open_menu(menu_for.clone(), e.position, window, cx)
                    }),
                )
            }))
            .children(sentinel)
            .when(!pinned && self.loading, |d| {
                d.child(div().h(px(24.)).flex().justify_center().child(icon(
                    "sidebar-cmd-spinner",
                    16.,
                    cx.theme().text_tertiary,
                )))
            })
            .when(!pinned && self.page_error, |d| {
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
            .into_any_element()
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
        let recents =
            if !self.has_more && self.threads.iter().all(|(id, _)| self.pinned.contains(id)) {
                sidebar_empty("No chats", cx).into_any_element()
            } else {
                self.rows(false, window, cx)
            };
        let pinned = (!self.pinned.is_empty()).then(|| self.rows(true, window, cx));
        // A header click collapses or expands its section (`group/nav-section`).
        let section = |i: usize,
                       title: &'static str,
                       body: AnyElement,
                       window: &mut Window,
                       cx: &mut Context<Self>| {
            let collapsed = self.collapsed[i];
            let header_focus = focus::handle(title, -1, window, cx);
            div()
                .tab_group()
                .px(px(8.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .group(title)
                        .relative()
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .pl(px(8.))
                        .child(text(title, 14., 21., 500., cx.theme().text_tertiary))
                        .child(
                            svg()
                                .path("icons/chevron-down.svg")
                                .w(px(14.))
                                .h(px(14.7))
                                .ml(px(4.))
                                .text_color(cx.theme().text_tertiary)
                                .opacity(if collapsed { 1. } else { 0. })
                                .group_hover(title, |s| s.opacity(1.))
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
                                    .id(title)
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
                                move |this, _, _, cx| {
                                    this.collapsed[i] ^= true;
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .when(!collapsed, |d| d.child(div().pt(px(4.)).child(body)))
        };
        let projects = section(
            0,
            "Projects",
            sidebar_empty("No projects", cx).into_any_element(),
            window,
            cx,
        );
        let recents = section(1, "Recents", recents, window, cx);
        let pinned = pinned.map(|rows| section(2, "Pinned", rows, window, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(home_sidebar_header(!self.unread.is_empty(), cx))
            .child(
                // `overflow-y-auto`: the sections scroll under the header and clip at its
                // bottom edge. The extent ends 16 px past Recents: its wrapper's `pb-4`
                // (cancelled in layout by `-mb-4`) reaches past the scroller's own 8 px
                // `pb-row-x`, and Chromium's scrollable overflow takes the farther one.
                div()
                    .id("sidebar-sections")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .pt(px(4.))
                    .pb(px(16.))
                    .children(pinned)
                    .child(projects)
                    .child(recents),
            )
            .children(self.menu.as_ref().map(|_| self.menu_overlay(window, cx)))
    }
}

/// What a Recents row ends in: the `Working` spinner at `animate-spin` angle (degrees) while a
/// turn runs, else the unread dot.
#[derive(Clone, Copy)]
enum RowEnd {
    Spinner(f32),
    Dot,
}

/// One Recents row: 30 px, `primary-ghost-hover` fill when hovered or selected. The label is
/// clipped at the row's end with the marquee's 16 px right fade (`_overflowingViewport`).
/// Hover or focus within the row shows `Pin chat` and `Archive chat`.
/// A running row ends in the 16 px `Working` spinner, an unread one in a blue dot
/// ([`RowEnd`]); either takes 28 px off the label and hides on hover.
fn thread_row(
    id: &str,
    label: SharedString,
    selected: bool,
    end: Option<RowEnd>,
    cx: &App,
    (focused, pinned): (bool, bool),
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
                    glyph: icon(name, 14., t.text_tertiary),
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
                // `_clipViewport _overflowingViewport`: the marquee's CSS mask (8 px left fade over
                // the track's padding, 16 px right fade), a masked layer in the sidebar's layer.
                div()
                    .id(SharedString::from(format!("thread-title-{id}")))
                    .absolute()
                    .left_0()
                    .top(px(5.))
                    .w(px(if focused { 216. } else { 264. - clip }))
                    .group_hover(group.clone(), |s| s.w(px(216.)))
                    .h(px(20.))
                    .pl(px(8.))
                    .overflow_hidden()
                    .layer()
                    .child(text(label, 14., 20., 430., t.text_default_nav))
                    .child(shapes::x_fade(px(8.), px(16.))),
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
                    .right(px(2.))
                    .h_full()
                    .w(px(54.))
                    .pr(px(6.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child({
                        let id = id.to_owned();
                        tooltip(
                            action("pin-chat", "sidebar-pin").on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                window
                                    .dispatch_action(Box::new(PinThread(id.clone(), !pinned)), cx);
                            }),
                            if pinned { "Unpin chat" } else { "Pin chat" },
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

/// One command menu row: optional 16 px leading icon, label, then a description, the chat's
/// project, its key equivalent, or an external-link arrow.
#[derive(Default)]
struct Item {
    icon: Option<&'static str>,
    label: SharedString,
    /// Text the query also matches: a command's description, a chat's first message.
    keywords: SharedString,
    /// Chat rows: workspace folder name; `Some` marks the row as a chat.
    project: Option<SharedString>,
    keys: Option<SharedString>,
    /// Chat rows: the thread id.
    thread: Option<String>,
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
            keywords: keywords.into(),
            ..Default::default()
        }
    }

    fn chat(thread: &Thread, snippet: Option<String>, unread: bool) -> Self {
        Self {
            label: title(thread),
            keywords: thread.preview.clone().into(),
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

    fn is_chat(&self) -> bool {
        self.project.is_some()
    }

    /// Chats match the query as a substring of their title or first message (or carry a
    /// content snippet), commands with [`fuzzy_match`] over the title or the description.
    fn matches(&self, query: &str) -> bool {
        if self.is_chat() {
            self.title_matches(query) || self.snippet.is_some()
        } else {
            fuzzy_match(&self.label, query) || fuzzy_match(&self.keywords, query)
        }
    }

    fn title_matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        [&self.label, &self.keywords]
            .iter()
            .any(|s| s.to_lowercase().contains(&query))
    }
}

/// The command menu's filter (a word-start matcher, `*query`): the query splits into
/// contiguous fragments; the first may start anywhere, each next one at a word start.
fn fuzzy_match(text: &str, query: &str) -> bool {
    fn word_start(t: &[char], i: usize) -> bool {
        i == 0 || !t[i - 1].is_alphanumeric() || (t[i].is_uppercase() && t[i - 1].is_lowercase())
    }
    fn rest(t: &[char], q: &[char], from: usize, first: bool) -> bool {
        if q.is_empty() {
            return true;
        }
        (from..t.len()).any(|i| {
            (first || word_start(t, i))
                && (1..=q.len()).any(|k| {
                    t.len() >= i + k
                        && t[i..i + k]
                            .iter()
                            .zip(&q[..k])
                            .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                        && rest(t, &q[k..], i + k, false)
                })
        })
    }
    let t: Vec<char> = text.chars().collect();
    let q: Vec<char> = query.trim().chars().collect();
    rest(&t, &q, 0, true)
}

/// Which chars of `text` the original highlights for `query`: all for an empty query, else
/// the first case-insensitive occurrence, else (`fuzzy`) a greedy in-order char match. A
/// text without any match stays plain (all on): the original dims the rest of a title only
/// around a match.
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
    let mut next = 0;
    let marks: Vec<bool> = lower
        .iter()
        .map(|c| {
            let hit = fuzzy && next < q.len() && *c == q[next];
            next += usize::from(hit);
            hit
        })
        .collect();
    if marks.contains(&true) {
        marks
    } else {
        vec![true; marks.len()]
    }
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

/// `CommandMenuDialog` in chat-search mode (sidebar `Search`): the query input over `Chats`
/// (from `thread/list`), `Quick actions`, the `Settings` pages and the command groups
/// `Chat`, `Navigation`, `Panels`, `Project`, `Skills`, `Configure`, `App`, filtered by the
/// query. The selected row carries the `primary-ghost-hover` fill, the others `opacity: .75`.
pub struct CommandMenu {
    groups: Vec<(&'static str, Vec<Item>)>,
    query: String,
    selected: usize,
    focus: FocusHandle,
    /// A thread is open: its commands show (`Search files`, `Mark as unread`, ...).
    thread_open: bool,
    /// Bumped on every query change; a debounced search only fires for the latest.
    search_gen: usize,
    /// `thread/search` in flight: the spinner next to `Chats`.
    searching: bool,
}

/// What the command menu asks its owner to do.
pub enum CommandMenuEvent {
    /// Escape or a click outside the dialog.
    Dismiss,
    /// A chat row: open the thread.
    OpenThread(String),
    /// A command row, by its label (`New chat`, `General`, `Manage scheduled tasks`, ...).
    Command(SharedString),
    /// The query has 3+ chars and rested [`SEARCH_DEBOUNCE`]: send `thread/search` for it and
    /// answer with [`CommandMenu::set_search_results`].
    Search(String),
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
        "Add a local project to Codex",
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
        "Sign out of Codex",
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
    /// Over the fixture's `thread/list` (refs/sidebar): the content search answered with the
    /// first message of every chat it matches, still pending on ChatGPT (the spinner).
    pub fn new(query: &str, cx: &mut Context<Self>) -> Self {
        let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
        let mut menu = Self::with_threads(&list.data, &[], false, query, cx);
        for chat in &mut menu.groups[0].1 {
            if !query.is_empty() && chat.title_matches(query) {
                chat.snippet = Some(chat.keywords.clone());
            }
        }
        menu.searching = !query.is_empty();
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
        let command = |(_, icon, label, description, keys, thread_only): &Command| Item {
            keys: Some(*keys).filter(|k| !k.is_empty()).map(Into::into),
            thread_only: *thread_only,
            description: (*label == "Theme").then_some(*description),
            chevron: *label == "Theme",
            ..Item::command(*icon, label, description)
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
        .map(|(icon, label, external)| Item {
            external,
            ..Item::command(Some(icon), label, "")
        });
        let mut groups = vec![
            ("Chats", chats),
            group("Quick actions"),
            ("Settings", settings.into()),
        ];
        groups.extend(GROUPS.into_iter().map(group));
        Self {
            groups,
            query: query.to_owned(),
            selected: 0,
            focus: cx.focus_handle(),
            thread_open,
            search_gen: 0,
            searching: false,
        }
    }

    /// Groups with the items matching the query; empty groups are dropped. `Chats` lists
    /// the title matches first, then the content-only matches of `thread/search`.
    /// ponytail: the original ranks every match by its matcher's score; list order here.
    fn visible(&self) -> Vec<(&'static str, Vec<&Item>)> {
        let query = self.query.trim();
        self.groups
            .iter()
            .map(|(heading, items)| {
                let mut items = items
                    .iter()
                    .filter(|i| self.thread_open || !i.thread_only)
                    .filter(|i| query.is_empty() || i.matches(query))
                    .collect::<Vec<_>>();
                if *heading == "Chats" && !query.is_empty() {
                    items.sort_by_key(|i| !i.title_matches(query));
                }
                (*heading, items)
            })
            .filter(|(_, items)| !items.is_empty())
            .collect()
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
        let mut index = 0;
        self.visible()
            .into_iter()
            .map(|(heading, items)| {
                let rows = items
                    .into_iter()
                    .map(|item| {
                        index += 1;
                        let mut parts = vec![bracket(
                            &item.label,
                            highlight(&item.label, query, !item.is_chat()),
                        )];
                        parts.extend(item.description.map(|d| format!("desc:{d}")));
                        parts.extend(item.project.as_ref().map(|p| p.to_string()));
                        parts.extend(
                            item.keys
                                .as_ref()
                                .map(|k| k.to_string())
                                .or_else(|| item.is_chat().then(|| format!("⌃{index}"))),
                        );
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
    pub fn set_search_results(&mut self, term: &str, hits: Vec<SearchHit>, cx: &mut Context<Self>) {
        if term != self.query.trim() {
            return;
        }
        self.searching = false;
        let selected = self
            .visible()
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .nth(self.selected)
            .map(|item| (item.thread.clone(), item.label.clone()));
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
        if let Some(selected) = selected
            && let Some(index) = self
                .visible()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|item| item.thread == selected.0 && item.label == selected.1)
        {
            self.selected = index;
        }
        cx.notify();
    }

    /// The query changed: drop the old search results and schedule the next search.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.selected = 0;
        let chats = &mut self.groups[0].1;
        chats.retain(|c| !c.from_search);
        chats.iter_mut().for_each(|c| c.snippet = None);
        self.search_gen += 1;
        self.searching = false;
        let term = self.query.trim().to_owned();
        if term.chars().count() < SEARCH_MIN_CHARS {
            return;
        }
        let generation = self.search_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.search_gen == generation {
                    this.searching = true;
                    cx.emit(CommandMenuEvent::Search(term));
                    cx.notify();
                }
            })
        })
        .detach();
    }

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match k.key.as_str() {
            "down" => self.selected += 1,
            "up" => self.selected = self.selected.saturating_sub(1),
            "backspace" => {
                self.query.pop();
                self.query_changed(cx);
            }
            "escape" => return cx.emit(CommandMenuEvent::Dismiss),
            "enter" => return self.confirm(cx),
            _ => match &k.key_char {
                Some(c) if !k.modifiers.platform && !k.modifiers.control => {
                    self.query.push_str(c);
                    self.query_changed(cx);
                }
                _ => return,
            },
        }
        let count = self
            .visible()
            .iter()
            .map(|(_, items)| items.len())
            .sum::<usize>();
        self.selected = self.selected.min(count.saturating_sub(1));
        cx.notify();
    }

    /// Runs the selected row.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        let event = self
            .visible()
            .into_iter()
            .flat_map(|(_, items)| items)
            .nth(self.selected)
            .map(|item| match &item.thread {
                Some(id) => CommandMenuEvent::OpenThread(id.clone()),
                None => CommandMenuEvent::Command(item.label.clone()),
            });
        if let Some(event) = event {
            cx.emit(event);
        }
    }
}

impl Render for CommandMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let viewport = window.viewport_size();
        let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
        // `.command-menu-dialog` / `.global-command-menu-dialog` sizing.
        let list_height = (0.9 * h - 64.).clamp(120., 440.);
        let width = 520f32.min(0.92 * w);
        let top = ((h - list_height - 64.) / 2.).max(16.);
        let surface = t.surface_elevated_secondary;
        // `border-color: var(--color-border)` in dark, transparent in light (still a 1 px
        // border: Blink paints the fill in its bleed-avoidance clip layer).
        let border_color = if t.appearance == Appearance::Dark {
            t.border
        } else {
            gpui::transparent_black()
        };
        let border = Some((px(1.), border_color));
        let query = self.query.trim();
        // The open dialog is its own composited layer (Radix's enter animation): it rasterizes
        // into transparent tiles that start at its ink overflow, i.e. the shadow's (3σ of the
        // 32 px blur, less the 8 px spread, minus the 16 px offset on top). Its ops blend in the
        // layer, which then composites once.
        let (ink_x, ink_y) = (40., 24.);
        // A row's text column: the dialog less its 1 px border and `p-1`, the row's `px-2`, and
        // the 16 px icon slot with its 8 px gap.
        let text_width = width - 2. * 5. - 2. * 8. - 16. - 8.;
        let mut index = 0;
        let groups = self.visible().into_iter().map(|(heading, items)| {
            // The pending content search shows its spinner next to `Chats`.
            let searching = heading == "Chats" && self.searching;
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                // `pt-2` over a 13 px / 18.57 line; 26.5 keeps the rows on the original's
                // device-pixel-snapped edges.
                .child(
                    div()
                        .h(px(26.5))
                        .pt(px(8.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(text(heading, 13., 18.5, 430., t.text_tertiary))
                        .when(searching, |d| {
                            d.child(icon("sidebar-cmd-spinner", 12., t.text_tertiary))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .children(items.into_iter().map(|item| {
                            let selected = index == self.selected;
                            index += 1;
                            // `Chats` is the first group, so a chat's ordinal is `index`.
                            menu_row(item, query, index, selected, text_width, t, window).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.selected = index - 1;
                                    this.confirm(cx);
                                }),
                            )
                        })),
                )
        });
        let input = if query.is_empty() {
            text("Search chats", 14., 21., 430., t.text_tertiary)
        } else {
            text(self.query.clone(), 14., 21., 430., t.text)
        };
        div()
            .absolute()
            .inset_0()
            .bg(rgba(OVERLAY))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(CommandMenuEvent::Dismiss)),
            )
            .child(
                div()
                    .id("command-menu")
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
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
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(20.)),
                                fill: Some(surface),
                                border,
                                shadows: vec![shapes::Shadow {
                                    color: rgba(0x00000030).into(),
                                    offset: point(px(0.), px(16.)),
                                    blur: px(32.),
                                    spread: px(-8.),
                                    inset: false,
                                }],
                                ..Default::default()
                            }))
                            // 1 px border + `p-1`.
                            .child(
                                div()
                                    .relative()
                                    .p(px(5.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .overflow_hidden()
                                    .child(div().h(px(33.)).px(px(10.)).py(px(6.)).child(input))
                                    .child(
                                        div()
                                            .id("command-menu-list")
                                            .max_h(px(list_height))
                                            .overflow_y_scroll()
                                            // Its contents end 0.1 px past the box, so Chromium
                                            // scrolls and composites them in their own
                                            // transparent layer, over the dialog's (cc layers
                                            // are flat): a row's opacity restores onto it.
                                            .top_layer()
                                            .flex()
                                            .flex_col()
                                            .children(groups),
                                    ),
                            ),
                    ),
            )
    }
}

/// `#0002` `--color-dialog-overlay` over the window behind the command menu.
const OVERLAY: u32 = 0x0000_0022;

/// `color` inside nested CSS `opacity` layers (outermost last), composited on opaque `bottom`
/// the way Skia does: each layer scales the 8-bit premultiplied pixel by
/// `(v * (round(opacity * 255) + 1)) >> 8`.
fn layered(color: Hsla, opacities: &[f32], bottom: Hsla) -> Hsla {
    let (c, b) = (Rgba::from(color), Rgba::from(bottom));
    let mut a = (c.a * 255.).round() as u32;
    let premul = |v: f32| (v * 255. * a as f32 / 255.).round() as u32;
    let mut rgb = [premul(c.r), premul(c.g), premul(c.b)];
    for o in opacities {
        let scale = (o * 255.).round() as u32 + 1;
        a = (a * scale) >> 8;
        rgb = rgb.map(|v| (v * scale) >> 8);
    }
    let blend = |v: u32, b: f32| (v as f32 + (b * 255. * (255 - a) as f32 / 255.).round()) / 255.;
    Rgba {
        r: blend(rgb[0], b.r),
        g: blend(rgb[1], b.g),
        b: blend(rgb[2], b.b),
        a: 1.,
    }
    .into()
}

/// A command menu row. CSS dims unselected rows with `opacity: .75` and the `kbd` with `.8`:
/// the glyphs blend through those layers (element `opacity`); fills are precomputed with
/// [`layered`] and icons take the alpha in their color (Chromium folds a lone draw's opacity).
/// Query matches show in the text color, the rest of a filtered title in the description
/// color; a chat matching the query shows its first message under the title. `text_width`:
/// the width of the row's text column.
fn menu_row(
    item: &Item,
    query: &str,
    ordinal: usize,
    selected: bool,
    text_width: f32,
    t: &Theme,
    window: &Window,
) -> Stateful<Div> {
    let surface = t.surface_elevated_secondary;
    let (bottom, row_opacity) = if selected {
        (blit(t.ghost_hover, surface), 1.)
    } else {
        (surface, 0.75)
    };
    let (fg, dim) = (t.text, t.text_tertiary);
    let snippet = item.snippet.clone();
    let label = marked(
        item.label.clone(),
        &highlight(&item.label, query, !item.is_chat()),
        14.,
        21.,
        fg,
        dim,
    )
    .opacity(row_opacity);
    let line = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .map(|d| match item.description {
            Some(description) => d
                .child(label.flex_none().max_w(relative(0.6)).truncate())
                .child(
                    text(description, 13., 130. / 7., 430., dim)
                        .opacity(row_opacity)
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                ),
            None => d.child(label.flex_1().min_w_0().truncate()),
        })
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
                .or_else(|| item.is_chat().then(|| format!("⌃{ordinal}").into())),
            |d, keys| {
                let kbd_bg = layered(t.text_alpha_10, &[0.8, row_opacity], bottom);
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
                                fill: Some(kbd_bg),
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
        .id(("command-row", ordinal))
        .relative()
        .py(px(5.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .when(selected, |d| {
            d.child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(12.5)),
                fill: Some(t.ghost_hover),
                ..Default::default()
            }))
        })
        .child(div().size(px(16.)).flex_none().map(|slot| {
            if item.unread {
                // `size-5` box with a `scale-50` `icon-xs` dot: 8 px `info-solid`, centred.
                slot.flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(8.))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(Theme::RADIUS_FULL),
                                fill: Some(t.info_solid.opacity(row_opacity)),
                                ..Default::default()
                            })),
                    )
            } else {
                slot.children(item.icon.map(|name| {
                    // `navigateForward` is the back arrow with `-scale-x-100`.
                    icon(name, 16., fg.opacity(row_opacity))
                        .when(item.label == "Forward", |i| i.flip_x())
                }))
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(line)
                .when_some(snippet, |d, snippet| {
                    // The original wraps every char in its own span: each starts where the
                    // previous span's LayoutUnit-ceiled width ends. `text-overflow: ellipsis`
                    // then drops the spans that leave no room for the `…` in `text_width`.
                    let marks = highlight(&snippet, query, false);
                    let ellipsis = span_widths("…", 12., window)[0];
                    let mut x = 0.;
                    let kept: Vec<_> = snippet
                        .chars()
                        .zip(marks)
                        .zip(span_widths(&snippet, 12., window))
                        .take_while(|&(_, w)| {
                            x += w;
                            x + ellipsis <= text_width
                        })
                        .collect();
                    let cut = kept.len() < snippet.chars().count();
                    let span = |s: String, w: f32, c: Hsla| {
                        div().flex_none().w(px(w)).child(text(s, 12., 16., 430., c))
                    };
                    d.child(
                        div()
                            .pt(px(2.))
                            .flex()
                            .overflow_hidden()
                            .opacity(row_opacity)
                            .children(kept.into_iter().map(|((c, m), w)| {
                                span(c.to_string(), w, if m { fg } else { dim })
                            }))
                            .when(cut, |d| d.child(span("…".into(), ellipsis, dim))),
                    )
                }),
        )
}

/// Width of each char of `s` (430 weight, `size` px) inside the run shaped whole, ceiled to
/// Blink's LayoutUnit (1/64 device px) as an inline span's width is.
fn span_widths(s: &str, size: f32, window: &Window) -> Vec<f32> {
    let mut font = window.text_style().font();
    font.weight = FontWeight(430.);
    let run = TextRun {
        len: s.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(s.to_string().into(), px(size), &[run], None);
    let unit = window.scale_factor() * 64.;
    let ends = s.char_indices().skip(1).map(|(i, _)| i).chain([s.len()]);
    s.char_indices()
        .zip(ends)
        .map(|((start, _), end)| {
            let w = f32::from(line.x_for_index(end) - line.x_for_index(start));
            (w * unit).ceil() / unit
        })
        .collect()
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_matches_skia() {
        let t = Theme::dark();
        let red = |c: Hsla| (Rgba::from(c).r * 255.).round() as u8;
        let selected = blit(t.ghost_hover, t.surface_elevated_secondary);
        // refs/sidebar/dark@1280x832.png: `kbd` fill on the selected row 76, on the others 57.
        assert_eq!(red(layered(t.text_alpha_10, &[0.8, 1.], selected)), 76);
        assert_eq!(
            red(layered(
                t.text_alpha_10,
                &[0.8, 0.75],
                t.surface_elevated_secondary
            )),
            57
        );
    }

    #[test]
    fn math_query_matches_like_the_original() {
        // refs/sidebar: `math` keeps the subtract chat and `Manage scheduled tasks` only.
        assert!(fuzzy_match(
            "Manage scheduled tasks Create or manage scheduled tasks from the current page",
            "math"
        ));
        assert!(!fuzzy_match(
            "Clear all unreads Mark all chats and scheduled task updates as read",
            "math"
        ));
        assert!(!fuzzy_match("Computer history ", "math"));
        let marks = highlight("Manage scheduled tasks", "math", true);
        let lit: String = "Manage scheduled tasks"
            .chars()
            .zip(marks)
            .filter_map(|(c, m)| m.then_some(c))
            .collect();
        assert_eq!(lit, "Mat");
    }
}
