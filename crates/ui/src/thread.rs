//! Conversation view: one thread's turns drawn from app-server data the way refs/thread shows
//! them (timestamp, user bubble, `Worked for` disclosure, final answer markdown, turn diff card,
//! message actions, docked composer), kept live by `item/*` and `turn/*` notifications.
//! Numbers are the spec's CSS px.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bridge::Event;
use bridge::types::{
    AgentMessageDeltaNotification, ItemNotification, Thread, ThreadItem, Turn, TurnNotification,
    TurnStatus, UserInput,
};
use gpui::{
    Action, AnyView, App, Bounds, ClipboardItem, Context, Corners, Div, FontWeight, Hsla, Image,
    ImageFormat, IntoElement, ParentElement, Pixels, Render, Rgba, ScrollHandle, SharedString,
    Size, Styled, Task, TextRun, Window, div, font, prelude::*, px, relative, rgb, rgba, white,
};
use theme::{ActiveTheme as _, Appearance, Theme};

use crate::icon::icon;
use crate::screens::sidebar::title;
use crate::{MONO, SANS, composer::shadow, icon_button, layout_color, over, text};

#[path = "thread/actions.rs"]
mod actions;
#[path = "thread/header.rs"]
mod header;
#[path = "thread/items.rs"]
mod items;
#[path = "thread/markdown.rs"]
mod markdown;

use actions::action_row;
pub use actions::{ForkThread, RevertTurn};
use items::{activity, diff_card, disclosure, user_bubble, words, wrap_row};
use markdown::{markdown, tones};

/// `--thread-content-max-width`
const CONTENT_MAX_WIDTH: Pixels = px(768.);

/// Wall clock for timestamps: the local UTC offset and, for fixture screens, a pinned "now".
#[derive(Clone, Copy)]
pub struct Clock {
    pub utc_offset: i64,
    pub pinned_now: Option<i64>,
}

impl Clock {
    fn now(&self) -> i64 {
        self.pinned_now.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        })
    }
}

pub struct ThreadView {
    thread_id: String,
    /// Workspace folder; its name heads the summary panel.
    cwd: String,
    /// The `Toggle summary` panel is open.
    pub summary_open: bool,
    /// The view fills the main surface (no side panel): it draws its px at the surface's
    /// rounded right corners through the clip (`Shell::main_clips_right`).
    pub clip_right: bool,
    title: SharedString,
    chat_actions: Option<gpui::FocusHandle>,
    chat_actions_trigger: Option<gpui::FocusHandle>,
    chat_actions_hovered: bool,
    turns: Vec<Turn>,
    clock: Clock,
    composer: AnyView,
    _tick: Option<Task<()>>,
    /// Turns whose `Worked for` disclosure is expanded.
    expanded: HashSet<String>,
    scroll: ScrollHandle,
    /// The turns as last laid out, to stand in for the ones far off screen.
    laid_out: Rc<RefCell<LaidOut>>,
}

/// Where the last frame laid out each turn, in the scroller's content (top, height), and the
/// window size and side panel state it had: a turn far off screen renders as an empty box of
/// its height, so a long chat lays out and paints only what is near the viewport. The column
/// keeps every turn's box, so the turns on screen land where a full layout puts them.
#[derive(Default)]
struct LaidOut {
    key: Option<(Size<Pixels>, bool)>,
    width: Pixels,
    turns: HashMap<String, (Pixels, Pixels)>,
}

impl ThreadView {
    /// `composer` is docked under the transcript.
    pub fn new(thread: Thread, clock: Clock, composer: AnyView) -> Self {
        Self {
            title: title(&thread),
            chat_actions: None,
            chat_actions_trigger: None,
            chat_actions_hovered: false,
            thread_id: thread.id,
            cwd: thread.cwd,
            summary_open: false,
            clip_right: false,
            turns: thread.turns,
            clock,
            composer,
            _tick: None,
            expanded: HashSet::new(),
            scroll: ScrollHandle::new(),
            laid_out: Rc::default(),
        }
    }

    pub fn id(&self) -> &str {
        &self.thread_id
    }

    /// The thread's working directory, as the server reports it.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Commands still running after their turn ended (background terminals), oldest first:
    /// `(command as the user typed it, output so far)`.
    pub fn background_terminals(&self) -> Vec<(String, &str)> {
        self.turns
            .iter()
            .filter(|t| t.status != TurnStatus::InProgress)
            .flat_map(|t| &t.items)
            .filter_map(|i| match i {
                ThreadItem::CommandExecution {
                    command,
                    status,
                    aggregated_output,
                    ..
                } if status == "inProgress" => Some((
                    shell_command(command),
                    aggregated_output.as_deref().unwrap_or_default(),
                )),
                _ => None,
            })
            .collect()
    }

    /// `Toggle summary` panel: project changes and background processes.
    fn summary(&self, window: &Window, t: &Theme) -> Div {
        let project = self.cwd.rsplit('/').next().unwrap_or(&self.cwd).to_owned();
        // `sticky` headers: their background (and their `before:` 8 px above) paints again in
        // the panel's rounded `overflow: hidden` clip, over the panel's corner.
        let header = |label: String| {
            div()
                .relative()
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .top(px(-8.))
                        .child(shapes::decoration(shapes::Decoration {
                            fill: Some(t.surface_elevated_secondary),
                            ..Default::default()
                        })),
                )
                .h(px(28.))
                .pb(px(2.))
                .pl(px(20.))
                .pr(px(14.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().py(px(2.)).child(text(
                    label,
                    13.,
                    18.5714,
                    400.,
                    t.text_secondary,
                )))
        };
        // `trailing`: the row's hover-only actions keep their space. A label that overflows its
        // viewport gets the marquee's mask (`_overflowingViewport`: 0.571429em left fade over
        // the track's padding, 1.14286em right fade), a masked box in its own layer.
        let row = |name: &'static str, label: String, trailing: f32, ix: usize| {
            let clip = 260. - 30. - trailing;
            let overflows = f32::from(width(window, &label, 13., 430., None)) > clip;
            let pad = 13. * 0.571429;
            div()
                .id((name, ix))
                .h(px(28.))
                .px(px(20.))
                .py(px(4.))
                .child(
                    div()
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(div().mr(px(8.)).child(icon(name, 18., t.text)))
                        .child(
                            div()
                                .relative()
                                .ml(px(-pad))
                                .w(px(clip + pad))
                                .pl(px(pad))
                                .overflow_hidden()
                                .child(text(label, 13., 18.5714, 430., t.text))
                                .when(overflows, |d| {
                                    d.layer().child(shapes::x_fade(px(pad), px(13. * 1.14286)))
                                }),
                        ),
                )
        };
        let has_changes = self
            .turns
            .iter()
            .flat_map(|t| &t.items)
            .any(|i| matches!(i, ThreadItem::FileChange { .. }));
        let terminals = self.background_terminals();
        let mut sections = vec![
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    header(project).child(
                        div()
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("thread-summary-actions", 18., t.text_tertiary)),
                    ),
                )
                .when(has_changes, |d| {
                    d.child(
                        row("thread-summary-changes", "Changes".into(), 32., 0)
                            .on_click(crate::dispatch::<crate::OpenChangesPanel>),
                    )
                }),
        ];
        if !terminals.is_empty() {
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(header("Background processes".into()))
                    .children(terminals.into_iter().enumerate().map(|(ix, (command, _))| {
                        row("thread-summary-terminal", command, 22., ix)
                            .on_click(crate::dispatch::<crate::OpenBackgroundTerminal>)
                    })),
            );
        }
        let n = sections.len();
        // A fixed popover: its own composited layer, whose rect (and tile grid) starts at the
        // widest shadow's ink, 1.5 blur = 30 px out.
        let ink = px(30.);
        div()
            .absolute()
            // Anchored under `Toggle summary`, 8 px from the thread's right edge.
            .top(px(8.) - ink)
            .right(px(8.) - ink)
            .p(ink)
            .layer()
            // Floating UI's `translate` is rounded to device pixels (`roundByDPR`).
            .snap_origin()
            .child(
                div()
                    .relative()
                    .w(px(300.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(Theme::RADIUS_2XL),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.surface_elevated_secondary),
                        // `--elevation-prominent`
                        shadows: vec![
                            shadow(t.border_strong, 0., 0., 0.5, false),
                            shadow(rgba(0x0000000a).into(), 3., 7.5, 0., false),
                            shadow(rgba(0x0000000d).into(), 0., 20., 0., false),
                        ],
                        ..Default::default()
                    }))
                    .child(crate::rounded_clip(
                        Theme::RADIUS_2XL,
                        shapes::Corner::Squircle,
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .py(px(10.))
                            .children(sections.into_iter().enumerate().map(|(i, s)| {
                                s.when(i + 1 < n, |s| {
                                    // Half-px rule under every section but the last.
                                    s.relative().pb(px(8.)).child(
                                        div()
                                            .absolute()
                                            .bottom_0()
                                            .left(px(20.))
                                            .right(px(20.))
                                            .h(px(0.5))
                                            .bg(over(t.border, t.surface_elevated_secondary)),
                                    )
                                })
                            })),
                    )),
            )
    }

    /// Folds one app-server notification for this thread into the transcript.
    pub fn apply(&mut self, event: &Event, cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        if params["threadId"].as_str() != Some(&self.thread_id) {
            return;
        }
        match method.as_str() {
            "turn/started" | "turn/completed" => {
                let Ok(n) = serde_json::from_value::<TurnNotification>(params.clone()) else {
                    return;
                };
                match self.turns.iter_mut().find(|t| t.id == n.turn.id) {
                    Some(turn) => {
                        turn.status = n.turn.status;
                        turn.completed_at = n.turn.completed_at;
                        turn.duration_ms = n.turn.duration_ms;
                        turn.error = n.turn.error;
                        if turn.items.is_empty() {
                            turn.items = n.turn.items;
                        }
                    }
                    None => self.turns.push(n.turn),
                }
                if self
                    .turns
                    .iter()
                    .any(|t| t.status == TurnStatus::InProgress)
                {
                    self.tick(cx);
                }
            }
            "item/started" | "item/completed" => {
                let Ok(n) = serde_json::from_value::<ItemNotification>(params.clone()) else {
                    return;
                };
                let Some(turn) = self.turns.iter_mut().find(|t| t.id == n.turn_id) else {
                    return;
                };
                let id = item_id(&n.item).map(str::to_owned);
                match turn
                    .items
                    .iter_mut()
                    .find(|i| id.is_some() && item_id(i) == id.as_deref())
                {
                    Some(item) => *item = n.item,
                    None => turn.items.push(n.item),
                }
            }
            "item/agentMessage/delta" | "item/commandExecution/outputDelta" => {
                let Ok(n) = serde_json::from_value::<AgentMessageDeltaNotification>(params.clone())
                else {
                    return;
                };
                match self.item_mut(&n.item_id) {
                    Some(ThreadItem::AgentMessage { text, .. }) => text.push_str(&n.delta),
                    Some(ThreadItem::CommandExecution {
                        aggregated_output, ..
                    }) => aggregated_output.get_or_insert_default().push_str(&n.delta),
                    _ => {}
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn item_mut(&mut self, id: &str) -> Option<&mut ThreadItem> {
        self.turns
            .iter_mut()
            .flat_map(|t| t.items.iter_mut())
            .find(|i| item_id(i) == Some(id))
    }

    /// Re-renders every second while a turn runs, for its `Working for` time.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self._tick.is_some() {
            return;
        }
        self._tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let running = this.update(cx, |this, cx| {
                    cx.notify();
                    let running = this
                        .turns
                        .iter()
                        .any(|t| t.status == TurnStatus::InProgress);
                    if !running {
                        this._tick = None;
                    }
                    running
                });
                if running.ok() != Some(true) {
                    break;
                }
            }
        }));
    }

    fn turn(&self, turn: &Turn, last: bool, window: &Window, t: &Theme, cx: &Context<Self>) -> Div {
        let group = SharedString::from(format!("turn-{}", turn.id));
        let mut col = div().group(group.clone()).flex().flex_col();
        let user = turn.items.iter().find_map(|i| match i {
            ThreadItem::UserMessage { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|c| match c {
                        UserInput::Text { text } => Some(text.as_str()),
                        UserInput::Other => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        });
        if let Some(user) = user {
            if let Some(started) = turn.started_at {
                col = col.child(self.timestamp(started, window, t));
            }
            col = col.child(user_bubble(&user, t));
        }
        let final_answer = match turn.items.last() {
            Some(ThreadItem::AgentMessage { text, .. })
                if turn.status != TurnStatus::InProgress =>
            {
                Some(text.as_str())
            }
            _ => None,
        };
        if let Some(answer) = final_answer {
            let secs = turn.duration_ms.unwrap_or(0) / 1000;
            let open = self.expanded.contains(&turn.id);
            let id = turn.id.clone();
            col = col.child(div().h(px(16.))).child(
                disclosure(format!("Worked for {}", duration(secs)), open, window, t)
                    .id(group.clone())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if !view.expanded.remove(&id) {
                            view.expanded.insert(id.clone());
                        }
                        cx.notify();
                    })),
            );
            if open {
                // The work the disclosure hides: everything before the final answer.
                let work = &turn.items[..turn.items.len() - 1];
                col = col
                    .child(div().h(px(16.)))
                    .child(activity(work, false, window, t));
            }
            col = col
                .child(div().h(px(16.)))
                .child(markdown(answer, window, t));
            if let Some(card) = diff_card(&turn.id, &turn.items, window, t) {
                col = col.child(div().mt(px(12.)).pb(px(8.)).child(card));
            }
            return col.child(action_row(last, &group, &turn.id, answer, t));
        }
        if turn.status == TurnStatus::InProgress {
            let secs = self.clock.now() - turn.started_at.unwrap_or(self.clock.now());
            col = col
                .child(div().h(px(16.)))
                .child(disclosure(
                    format!("Working for {}", duration(secs)),
                    false,
                    window,
                    t,
                ))
                .child(div().h(px(16.)));
        }
        let running = turn.status == TurnStatus::InProgress;
        col.child(activity(&turn.items, running, window, t))
    }

    /// `Yesterday 9:54 PM` above a turn's user message.
    fn timestamp(&self, at: i64, window: &Window, t: &Theme) -> Div {
        let (day, time) = day_and_time(at, self.clock.now(), self.clock.utc_offset);
        // The space is the `<time>` text's (400), not the bold day span's.
        let time = format!(" {time}");
        div()
            .flex()
            .justify_center()
            .py(px(16.))
            .child(measured(window, day, 13., 20., 500., t.text_tertiary, None))
            .child(measured(
                window,
                time,
                13.,
                20.,
                400.,
                t.text_tertiary,
                None,
            ))
    }
}

impl Render for ThreadView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let n = self.turns.len();
        // Turns more than half a viewport above or below it stand in as empty boxes of their
        // last height while the window size and side panel stay as they were laid out; the
        // last turn (and a running one) can still grow, so it always renders.
        let key = (window.viewport_size(), self.clip_right);
        let (view, offset) = (self.scroll.bounds().size.height, self.scroll.offset().y);
        let laid_out = self.laid_out.borrow();
        let turns: Vec<Div> = self
            .turns
            .iter()
            .enumerate()
            .map(|(i, turn)| {
                let stand_in = (laid_out.key == Some(key)
                    && i + 1 < n
                    && turn.status != TurnStatus::InProgress)
                    .then(|| laid_out.turns.get(&turn.id))
                    .flatten()
                    .filter(|&&(top, h)| far(top + offset, h, view));
                match stand_in {
                    Some(&(_, h)) => div().h(h),
                    None => self.turn(turn, i + 1 == n, window, &t, cx),
                }
            })
            .collect();
        drop(laid_out);
        let ids: Vec<String> = self.turns.iter().map(|t| t.id.clone()).collect();
        let (laid_out, scroll) = (self.laid_out.clone(), self.scroll.clone());
        let measure = move |bounds: Vec<Bounds<Pixels>>, window: &mut Window, _: &mut App| {
            let origin = scroll.bounds().top() + scroll.offset().y;
            let width = bounds.first().map_or(px(0.), |b| b.size.width);
            let mut l = laid_out.borrow_mut();
            if l.key == Some(key) && l.width != width {
                // The column width changed with the window size and side panel unchanged (the
                // sidebar): the stand-ins' heights are stale, so lay every turn out again.
                *l = LaidOut::default();
                window.request_animation_frame();
                return;
            }
            l.key = Some(key);
            l.width = width;
            l.turns = ids
                .iter()
                .zip(&bounds)
                .map(|(id, b)| (id.clone(), (b.top() - origin, b.size.height)))
                .collect();
        };
        // The thread fills the main panel: window minus titlebar (44) and bottom inset (4).
        let viewport = window.viewport_size();
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height) - 48.);
        let scale = window.scale_factor();
        let fade = |a: f32| Hsla { a, ..t.surface };
        let clear = gpui::transparent_black();
        // `_MainContentTopFade`
        let fade_stops = [
            (t.surface, 0.),
            (fade(0.82), 0.1),
            (fade(0.48), 0.3),
            (fade(0.12), 0.6),
            (clear, 1.),
        ];
        let top_fade = skia_gradient(&fade_stops, t.surface, (width, 40.), (0., 0.), false, scale);
        // The px the main surface's rounded clip covers in part at its right corners: the top
        // fade's and the composer band's surface fill, drawn onto the page after the shell's
        // corners (`Shell::main_clips_right`), in a deferred draw. cc draws a band tile that
        // holds only the fill (no ink of the composer card, which is centered) as a color quad.
        // ponytail: the bottom corner's px all in the band's last tile (510 px stride).
        let card_ink = crate::composer::card_decoration(&t)
            .shadows
            .iter()
            .filter(|s| !s.inset)
            .map(|s| f32::from(s.offset.x + s.spread) + 1.5 * f32::from(s.blur))
            .fold(1., f32::max);
        let surface = Rgba::from(t.surface);
        let surface = [surface.r, surface.g, surface.b, 1.].map(|v| (v * 255.).round() as u8);
        let corners = gpui::canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let s = window.scale_factor();
                let stops = premul_stops(&fade_stops);
                let dev = |v: Pixels| (f32::from(v) * s).round() as i32;
                let (x0, y0) = (dev(bounds.left()), dev(bounds.top()));
                let h = (40. * s).round() as u32;
                let fade = |x: i32, y: i32, _: u8| {
                    let (i, j) = ((x - x0) as u32, (y - y0) as u32);
                    gradient_px(&stops, h, (0, 0), false, i, j)
                };
                let top = Corners {
                    top_right: Theme::RADIUS_XL_BASE,
                    ..Default::default()
                };
                shapes::paint_clipped_tile(&fade, false, top, bounds, window, cx);
                let width = bounds.size.width;
                let card_right = (width + width.min(CONTENT_MAX_WIDTH)) / 2. - px(16.);
                let w = dev(width);
                let solid = dev(card_right + px(card_ink)) < 510 * ((w - 1) / 510) - 1;
                let bottom = Corners {
                    bottom_right: Theme::RADIUS_XL_BASE,
                    ..Default::default()
                };
                shapes::paint_clipped_tile(&|_, _, _| surface, solid, bottom, bounds, window, cx);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        // Fade above the composer's surface band (`-top-8`, `to top`).
        let bottom_fade = skia_gradient(
            &[(t.surface, 0.), (clear, 1.)],
            t.surface,
            (width, 32.),
            (0., height - 146.),
            true,
            scale,
        );
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(t.surface)
            .child(
                div()
                    .id("conversation")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    // The scroller's contents are a composited layer (transparent tile).
                    .layer()
                    .pt(px(32.))
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(CONTENT_MAX_WIDTH)
                            .px(px(16.))
                            .pb(px(32.))
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .children(turns)
                            .on_children_prepainted(measure),
                    )
                    // Room for the docked composer.
                    .child(div().h(px(114.))),
            )
            .child(top_fade.absolute().top_0().left_0())
            .when(self.clip_right, |d| d.child(gpui::deferred(corners)))
            // A portal in the original: its shadow spills over the titlebar and the sidebar.
            .when(self.summary_open, |d| {
                d.child(gpui::deferred(self.summary(window, &t)))
            })
            // The `z-20` composer band is its own cc layer; its rect (and tile grid) starts at
            // the light composer shadow's ink, 84 px above the band (80 px blur + 8 spread - 4
            // y); the dark card has no outer shadow. Its background paints into the layer (the
            // shadows blit onto it there), so it is a `shapes` fill, not a GPUI quad.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .when(t.appearance == Appearance::Light, |d| d.pt(px(84.)))
                    .layer()
                    .child(
                        div()
                            .relative()
                            .pb(px(16.))
                            .child(shapes::decoration(shapes::Decoration {
                                fill: Some(t.surface),
                                ..Default::default()
                            }))
                            .child(bottom_fade.absolute().top(px(-32.)).left_0())
                            .child(
                                div()
                                    .mx_auto()
                                    .w_full()
                                    .max_w(CONTENT_MAX_WIDTH)
                                    .px(px(16.))
                                    .child(self.composer.clone()),
                            ),
                    ),
            )
    }
}

fn item_id(item: &ThreadItem) -> Option<&str> {
    match item {
        ThreadItem::UserMessage { id, .. }
        | ThreadItem::AgentMessage { id, .. }
        | ThreadItem::Plan { id, .. }
        | ThreadItem::Reasoning { id, .. }
        | ThreadItem::CommandExecution { id, .. }
        | ThreadItem::FileChange { id, .. }
        | ThreadItem::McpToolCall { id, .. }
        | ThreadItem::ContextCompaction { id } => Some(id),
        ThreadItem::Other => None,
    }
}

/// The command inside the `<shell> -lc "<command>"` wrapper the server reports.
fn shell_command(command: &str) -> String {
    let Some((_, inner)) = command.split_once(" -lc ") else {
        return command.to_owned();
    };
    ['"', '\'']
        .iter()
        .find_map(|&q| inner.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(inner)
        .to_owned()
}

/// `17s`, `2m 5s`
fn duration(secs: i64) -> String {
    match secs {
        ..60 => format!("{secs}s"),
        _ => format!("{}m {}s", secs / 60, secs % 60),
    }
}

/// (`Today` | `Yesterday` | `Oct 14`, `9:54 PM`) of a unix time in local time.
fn day_and_time(at: i64, now: i64, utc_offset: i64) -> (String, String) {
    let (local, today) = (at + utc_offset, now + utc_offset);
    let t = chrono::DateTime::from_timestamp(local, 0).unwrap_or_default();
    let day = match today.div_euclid(86_400) - local.div_euclid(86_400) {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ => t.format("%b %-d").to_string(),
    };
    (day, t.format("%-I:%M %p").to_string())
}

/// Exact layout width of a single-line run. GPUI rounds measured text boxes up to whole px;
/// inline runs placed after one another need the fractional advances Chromium uses.
fn width(window: &Window, s: &str, size: f32, weight: f32, family: Option<&str>) -> Pixels {
    let family = SharedString::from(family.unwrap_or(SANS));
    let run = TextRun {
        len: s.len(),
        font: gpui::Font {
            weight: FontWeight(weight),
            ..font(family)
        },
        color: white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .layout_line(s, px(size), &[run], None)
        .width
}

/// Text of a known run width (see [`width`]).
fn measured(
    window: &Window,
    s: impl Into<SharedString>,
    size: f32,
    line_height: f32,
    weight: f32,
    color: Hsla,
    family: Option<&'static str>,
) -> Div {
    let s = s.into();
    let w = width(window, &s, size, weight, family);
    let label = text(s, size, line_height, weight, color);
    div().flex_none().w(w).child(match family {
        Some(f) => label.font_family(f),
        None => label,
    })
}

/// A vertical CSS `linear-gradient` over the opaque `under`, painted the way Chromium's Skia
/// raster pipeline does it in the refs, as one opaque image at device resolution:
///
/// - colors interpolate premultiplied in sRGB at pixel centers;
/// - the gradient is rasterized into its composited layer: Skia's 8x8 ordered dither on the
///   premultiplied color (clamped to alpha), then color and alpha are stored as 8 bits;
/// - the layer is blended over `under` (`src + dst * (1 - src_alpha)`).
///
/// The dither phase comes from the pixel's position in the layer's raster tiles (cc tiles step
/// 510 device px; a tile owns its far edge pixel). `origin` is the image's top-left in css px
/// relative to that layer (for the thread view: the main panel's top-left). Stops are
/// `(color, position 0..=1)`; `to_top` flips the direction. `size` is in css px.
pub fn skia_gradient(
    stops: &[(Hsla, f32)],
    under: Hsla,
    size: (f32, f32),
    origin: (f32, f32),
    to_top: bool,
    scale: f32,
) -> gpui::Img {
    static CACHE: LazyLock<Mutex<HashMap<u64, Arc<Image>>>> = LazyLock::new(Default::default);
    let dev = |v: f32| (v * scale).round().max(0.) as u32;
    let (w, h, ox, oy) = (dev(size.0), dev(size.1), dev(origin.0), dev(origin.1));
    let mut key = DefaultHasher::new();
    for (c, p) in stops {
        let c = Rgba::from(*c);
        for v in [c.r, c.g, c.b, c.a, *p] {
            v.to_bits().hash(&mut key);
        }
    }
    let u = Rgba::from(under);
    for v in [u.r, u.g, u.b] {
        v.to_bits().hash(&mut key);
    }
    (w, h, ox, oy, to_top).hash(&mut key);
    let image = CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key.finish())
        .or_insert_with(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Pnm,
                gradient_ppm(stops, under, (w, h), (ox, oy), to_top),
            ))
        })
        .clone();
    gpui::img(image)
        .w(px(size.0))
        .h(px(size.1))
        .object_fit(gpui::ObjectFit::Fill)
}

/// Pixels of [`skia_gradient`] as a binary PPM.
fn gradient_ppm(
    stops: &[(Hsla, f32)],
    under: Hsla,
    (w, h): (u32, u32),
    (ox, oy): (u32, u32),
    to_top: bool,
) -> Vec<u8> {
    let stops = premul_stops(stops);
    let u = Rgba::from(under);
    let under = [u.r, u.g, u.b].map(|v| (v * 255.).round() as u32);
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.reserve((w * h * 3) as usize);
    for j in 0..h {
        for i in 0..w {
            let [r, g, b, a] = gradient_px(&stops, h, (ox, oy), to_top, i, j);
            for (c, under) in [r, g, b].into_iter().zip(under) {
                let c = u32::from(c) + (under * (255 - u32::from(a)) + 127) / 255;
                out.push(c.min(255) as u8);
            }
        }
    }
    out
}

/// `stops` of [`skia_gradient`] as `(premultiplied color, alpha, position)`.
fn premul_stops(stops: &[(Hsla, f32)]) -> Vec<([f32; 3], f32, f32)> {
    stops
        .iter()
        .map(|(c, pos)| {
            let c = Rgba::from(*c);
            ([c.r * c.a, c.g * c.a, c.b * c.a], c.a, *pos)
        })
        .collect()
}

/// The premultiplied 8-bit RGBA Skia stores for px `(i, j)` of a [`skia_gradient`] `h` px
/// tall at `(ox, oy)` in its layer.
fn gradient_px(
    stops: &[([f32; 3], f32, f32)],
    h: u32,
    (ox, oy): (u32, u32),
    to_top: bool,
    i: u32,
    j: u32,
) -> [u8; 4] {
    let mut t = (j as f32 + 0.5) / h as f32;
    if to_top {
        t = 1. - t;
    }
    let (color, alpha) = gradient_at(stops, t);
    // Position inside the layer's tile; only the low 3 bits reach the dither.
    let tile = |v: u32| if v == 0 { 0 } else { (v - 1) % 510 + 1 };
    let d = dither(tile(i + ox), tile(j + oy)) / 255.;
    let [r, g, b] = color.map(|c| ((c + d).clamp(0., alpha) * 255.).round_ties_even() as u8);
    [r, g, b, (alpha * 255.).round_ties_even() as u8]
}

/// Premultiplied color and alpha of `stops` at `t`.
fn gradient_at(stops: &[([f32; 3], f32, f32)], t: f32) -> ([f32; 3], f32) {
    let last = stops.len() - 1;
    let i = stops
        .iter()
        .rposition(|s| s.2 <= t)
        .unwrap_or(0)
        .min(last.saturating_sub(1));
    let (c0, a0, p0) = stops[i];
    let (c1, a1, p1) = stops[(i + 1).min(last)];
    let f = if p1 > p0 {
        ((t - p0) / (p1 - p0)).clamp(0., 1.)
    } else {
        0.
    };
    let lerp = |a: f32, b: f32| a + (b - a) * f;
    ([0, 1, 2].map(|k| lerp(c0[k], c1[k])), lerp(a0, a1))
}

/// SkRasterPipeline `dither` stage offset for device pixel (x, y), in (-0.5, 0.5).
fn dither(x: u32, y: u32) -> f32 {
    let y = y ^ x;
    let m = (y & 1) << 5 | (x & 1) << 4 | (y & 2) << 2 | (x & 2) << 1 | (y & 4) >> 1 | (x & 4) >> 2;
    m as f32 * (2. / 128.) - 63. / 128.
}

/// Whether a box at `top` (scroller viewport px) of height `h` lies more than half a viewport
/// of height `view` above or below it.
fn far(top: Pixels, h: Pixels, view: Pixels) -> bool {
    top + h < -view / 2. || top > view * 1.5
}

#[cfg(test)]
mod tests {
    use super::markdown::{Inline, inlines};
    use super::*;

    #[test]
    fn far_turns_lie_past_half_a_viewport() {
        let (view, h) = (px(800.), px(100.));
        assert!(!far(px(0.), h, view));
        assert!(!far(px(-500.), h, view));
        assert!(far(px(-501.), h, view));
        assert!(!far(px(1200.), h, view));
        assert!(far(px(1201.), h, view));
    }

    #[test]
    fn parses_final_answer() {
        let s =
            "Added `subtract(a, b)` next to `add` in [src/math.py](/tmp/p/src/math.py). Done `2`.";
        assert_eq!(
            inlines(s),
            [
                Inline::Text("Added "),
                Inline::Code("subtract(a, b)"),
                Inline::Text(" next to "),
                Inline::Code("add"),
                Inline::Text(" in "),
                Inline::Link {
                    label: "src/math.py",
                    target: "/tmp/p/src/math.py"
                },
                Inline::Text(". Done "),
                Inline::Code("2"),
                Inline::Text("."),
            ]
        );
        assert_eq!(
            inlines("a [b c `d"),
            [
                Inline::Text("a ["),
                Inline::Text("b c `"),
                Inline::Text("d")
            ]
        );
    }

    /// refs/thread/dark@1280x832.png, top fade at device x 1792.., rows 88..: the pixels
    /// Chromium painted for `_MainContentTopFade`.
    #[test]
    fn gradient_matches_chromium_dither() {
        let s: Hsla = rgb(0x181818).into();
        let fade = |a: f32| Hsla { a, ..s };
        let stops = [
            (s, 0.),
            (fade(0.82), 0.1),
            (fade(0.48), 0.3),
            (fade(0.12), 0.6),
            (gpui::transparent_black(), 1.),
        ];
        let ppm = gradient_ppm(&stops, s, (1871, 80), (0, 0), false);
        let px = |x: usize, y: usize| ppm[ppm.len() - 1871 * 80 * 3 + (y * 1871 + x) * 3];
        let rows: Vec<String> = [96, 116, 131]
            .iter()
            .map(|&y| {
                (0..8)
                    .map(|i| px(1792 - 681 + i, y - 88).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect();
        assert_eq!(
            rows,
            [
                "25,24,25,24,25,24,25,24",
                "24,23,24,24,24,23,24,24",
                "23,24,23,24,23,24,23,24"
            ]
        );
    }

    #[test]
    fn formats_times() {
        // 2026-10-14 21:54:31 PDT, seen the next morning.
        let at = 1_792_040_071;
        assert_eq!(
            day_and_time(at, at + 36_000, -25_200),
            ("Yesterday".into(), "9:54 PM".into())
        );
        assert_eq!(day_and_time(at, at + 3 * 86_400, -25_200).0, "Oct 14");
        assert_eq!(duration(17), "17s");
        assert_eq!(
            shell_command(r#"/bin/zsh -lc "python3 -c 'print(2)'""#),
            "python3 -c 'print(2)'"
        );
    }
}
