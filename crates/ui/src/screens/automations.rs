//! `automations` parity screen: rail `Scheduled` (`/automations`). Sidebar `ScheduledSidebar`
//! and the page's empty state with suggested tasks, from `refs/automations/*.spec.json`.
//! Host data (`list-automations`, `/suggested_automations`) comes from the recorded answers
//! in `tools/route1/oracle.json`.

use crate::{Shell, icon::icon, icon_button, motion::ButtonVisual, text};
use gpui::{
    Action, AnyView, App, AppContext as _, Context, Corners, Div, Entity, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, actions, canvas, div, img,
    prelude::FluentBuilder as _, px,
};
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::Path};
use theme::{ActiveTheme as _, Theme};

actions!(
    automations,
    [
        /// Sidebar `New task`: opens the create flow.
        NewTask,
        /// Sidebar `Search scheduled tasks`.
        SearchTasks,
        /// Sidebar `Filter tasks` (ownership `Personal` / `Team`).
        FilterTasks,
    ]
);

/// A suggested-task card was clicked: the page shows the card as `Starting…` and the app
/// starts the guided setup conversation from its prompt.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct StartSuggestedTask {
    pub prompt: SharedString,
}

/// A sidebar task row was clicked: `/scheduled?automationId=<id>`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct OpenAutomation {
    pub id: String,
}

/// One `list-automations` item, as the sidebar `Upcoming` list and the task thread show it.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub name: SharedString,
    pub prompt: SharedString,
    #[serde(default)]
    pub rrule: String,
    /// Unix ms.
    #[serde(default)]
    pub created_at: i64,
    pub next_run_at: Option<i64>,
}

/// The items of a `list-automations` body that parse; others are skipped like the original's
/// malformed definitions.
pub fn tasks(list: &Value) -> Vec<Task> {
    list["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect()
}

/// A squircle `background-color` over the parent's box, blended like Blink (`shapes`), not a
/// GPUI quad: the fill px match the original to the unit.
fn fill(radius: gpui::Pixels, color: gpui::Hsla) -> Div {
    div()
        .absolute()
        .inset_0()
        .child(shapes::decoration(shapes::Decoration {
            radius: Corners::all(radius),
            corner: shapes::Corner::Squircle,
            fill: Some(color),
            ..Default::default()
        }))
}

/// `(day, time)` of a unix-ms instant at the `utc_offset` (seconds) wall clock:
/// `("Oct 17", "9:00 AM")`.
fn day_time(at_ms: i64, utc_offset: i64) -> (String, String) {
    let t = chrono::DateTime::from_timestamp(at_ms / 1000 + utc_offset, 0).unwrap_or_default();
    (
        t.format("%b %-d").to_string(),
        t.format("%-I:%M %p").to_string(),
    )
}

impl Task {
    fn rule(&self, key: &str) -> Option<&str> {
        self.rrule
            .split(';')
            .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
    }

    /// Sidebar cadence: `scheduled.sidebar.weekly` / `.daily`.
    /// ponytail: the original's `pr()` labels every rrule shape; the port covers the
    /// `New task` dialog's weekly and daily rules and reads everything else as daily.
    fn cadence(&self) -> &'static str {
        match self.rule("FREQ") {
            Some("WEEKLY") => "Weekly",
            _ => "Daily",
        }
    }

    /// Card schedule: `Fridays at 9:00 AM`, `Daily at 8:00 AM`.
    fn schedule(&self) -> String {
        let hour: u32 = self
            .rule("BYHOUR")
            .and_then(|h| h.parse().ok())
            .unwrap_or(9);
        let minute: u32 = self
            .rule("BYMINUTE")
            .and_then(|m| m.parse().ok())
            .unwrap_or(0);
        let time = format!(
            "{}:{minute:02} {}",
            match hour % 12 {
                0 => 12,
                h => h,
            },
            if hour < 12 { "AM" } else { "PM" }
        );
        let day = match self.rule("BYDAY") {
            Some("MO") => "Mondays",
            Some("TU") => "Tuesdays",
            Some("WE") => "Wednesdays",
            Some("TH") => "Thursdays",
            Some("FR") => "Fridays",
            Some("SA") => "Saturdays",
            Some("SU") => "Sundays",
            _ => "Daily",
        };
        format!("{day} at {time}")
    }
}

/// One `/suggested_automations` entry (or a built-in landing example).
#[derive(Clone, Deserialize)]
struct Suggestion {
    title: SharedString,
    description: SharedString,
    user_message: SharedString,
    emoji: SharedString,
    icon_url: Option<String>,
}

/// The recorded answer of the first `tools/route1/oracle.json` entry whose key contains
/// `key`: a host fetch body (`bodyJsonString`) or a ChatGPT backend body (`__response.body`).
fn oracle(key: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/route1/oracle.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let all: serde_json::Map<String, Value> = serde_json::from_str(&text).expect("oracle JSON");
    let entry = all
        .iter()
        .find(|(k, _)| k.contains(key))
        .unwrap_or_else(|| panic!("oracle.json: no entry for {key}"))
        .1;
    let body = entry["bodyJsonString"]
        .as_str()
        .or_else(|| entry["value"]["response"]["__response"]["body"].as_str())
        .unwrap_or_else(|| panic!("oracle.json: {key} has no body"));
    serde_json::from_str(body).expect("oracle body JSON")
}

/// `scheduled.landing.example.*` in bundle order: the page fills up to 6 cards with these
/// after the server's suggestions. ponytail: the original shuffles them per mount
/// (`Math.random`); the port keeps bundle order.
fn landing_examples() -> Vec<Suggestion> {
    [
        (
            "🥗",
            "Weekly meal plan",
            "Plan meals for the week and make a grocery list.",
            "Help me set up a weekly meal plan with a grocery list. Ask about my dietary preferences, household size, and when I’d like to receive it before scheduling.",
        ),
        (
            "💰",
            "Weekly finances update",
            "Review my recent spending, and anything that needs attention.",
            "Help me set up a weekly review of my recent spending, recurring charges, and anything that needs attention. Ask which financial information I want to use and what day and time works for me before scheduling.",
        ),
        (
            "😴",
            "Sleep coaching",
            "Track my sleep quality and help me improve my habits.",
            "Help me set up a regular sleep check-in. Ask about my sleep goals, the information I’m comfortable sharing, and when I’d like to check in before scheduling.",
        ),
        (
            "🛩",
            "Find flights for a trip",
            "Find the best deal on flights for an upcoming trip.",
            "Help me set up a flight price watch for an upcoming trip. Ask where I’m flying from and to, my dates, budget, and how often to check. Notify me when there’s a good match.",
        ),
        (
            "✉️",
            "Email monitor",
            "Scan my emails and let me know anything that needs my attention.",
            "Help me set up an email monitor that flags messages needing my attention. Ask which inbox or senders to watch, what counts as important, and how often to check before scheduling. Help me connect email if needed.",
        ),
    ]
    .into_iter()
    .map(|(emoji, title, description, prompt)| Suggestion {
        title: title.into(),
        description: description.into(),
        user_message: prompt.into(),
        emoji: emoji.into(),
        icon_url: None,
    })
    .collect()
}

/// Server suggestions, then landing examples whose title is not already shown, at most 6.
fn cards(mut all: Vec<Suggestion>) -> Vec<Suggestion> {
    let key = |s: &Suggestion| s.title.trim().to_lowercase();
    let taken: Vec<String> = all.iter().map(key).collect();
    all.extend(
        landing_examples()
            .into_iter()
            .filter(|s| !taken.contains(&key(s))),
    );
    all.truncate(6);
    all
}

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let server = serde_json::from_value(oracle("\"url\":\"/suggested_automations\""))
        .expect("suggested_automations shape");
    let (shell, _, _) = shell_with(server, Vec::new(), None, 0, cx);
    shell.update(cx, |shell, _| shell.can_go_back = true);
    shell.into()
}

/// The app's `/scheduled` route with the host's `tasks`: the task thread of `selected`
/// (`?automationId=`, with the composer docked under it), else the empty page (returned
/// third). The port has no ChatGPT backend for `/suggested_automations`, so the cards are the
/// landing examples, as the original shows them without server suggestions. Dates show at
/// `utc_offset` seconds.
pub fn shell(
    tasks: Vec<Task>,
    selected: Option<(Task, AnyView)>,
    utc_offset: i64,
    cx: &mut App,
) -> (
    Entity<Shell>,
    Entity<ScheduledSidebar>,
    Option<Entity<AutomationsPage>>,
) {
    shell_with(Vec::new(), tasks, selected, utc_offset, cx)
}

fn shell_with(
    server: Vec<Suggestion>,
    tasks: Vec<Task>,
    selected: Option<(Task, AnyView)>,
    utc_offset: i64,
    cx: &mut App,
) -> (
    Entity<Shell>,
    Entity<ScheduledSidebar>,
    Option<Entity<AutomationsPage>>,
) {
    let sidebar = cx.new(|_| ScheduledSidebar {
        tasks,
        selected: selected.as_ref().map(|(task, _)| task.id.clone()),
        utc_offset,
    });
    let mut page = None;
    let main: AnyView = match selected {
        Some((task, composer)) => cx
            .new(|_| AutomationThread {
                task,
                composer,
                utc_offset,
            })
            .into(),
        None => page
            .insert(cx.new(|_| AutomationsPage {
                cards: cards(server),
                pending: None,
            }))
            .clone()
            .into(),
    };
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar.clone().into(), main);
        shell.selected_rail = Some("scheduled");
        shell.new_tab = false;
        shell
    });
    (shell, sidebar, page)
}

/// `ScheduledSidebar`: header `Scheduled` + search, `New task`, section `Upcoming` with one
/// 48 px row per task (`automation-row`): name, `Oct 17 · Weekly`, and on hover the list
/// fill with the `Scheduled task actions` button fading in. The selected row keeps the fill.
pub struct ScheduledSidebar {
    tasks: Vec<Task>,
    selected: Option<String>,
    utc_offset: i64,
}

impl ScheduledSidebar {
    pub fn set_tasks(&mut self, tasks: Vec<Task>, cx: &mut Context<Self>) {
        self.tasks = tasks;
        cx.notify();
    }

    fn row(&self, task: &Task, t: &Theme) -> impl IntoElement {
        let fill = fill(px(12.5), t.ghost_hover);
        let group = SharedString::from(format!("task-row-{}", task.id));
        let id = task.id.clone();
        let mut secondary = div().h(px(16.)).pr(px(5.)).flex().items_center();
        if let Some(next) = task.next_run_at {
            secondary = secondary
                .child(text(
                    day_time(next, self.utc_offset).0,
                    12.,
                    16.,
                    430.,
                    t.text_tertiary,
                ))
                .child(text("·", 12., 16., 430., t.text_tertiary).mx(px(4.)));
        }
        div()
            .id(group.clone())
            .group(group.clone())
            .relative()
            .h(px(48.))
            .pt(px(5.))
            .pl(px(8.))
            .pr(px(5.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(if self.selected.as_deref() == Some(&task.id) {
                fill
            } else {
                fill.opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
            })
            .on_click(move |_, window, cx| {
                window.dispatch_action(Box::new(OpenAutomation { id: id.clone() }), cx)
            })
            .child(
                text(task.name.clone(), 14., 20., 430., t.text_default_nav)
                    .overflow_hidden()
                    .text_ellipsis(),
            )
            .child(secondary.child(text(task.cadence(), 12., 16., 430., t.text_tertiary)))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right(px(2.))
                    .h_full()
                    .w(px(52.))
                    .pr(px(6.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
                    .child(
                        div()
                            .id(SharedString::from(format!("{group}-actions")))
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.375))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(ButtonVisual {
                                glyph: icon("thread-chat-actions", 16., t.text_tertiary),
                                radius: px(9.375),
                                hover: t.ghost_hover,
                                foreground: t.text.opacity(0.85),
                                selected: false,
                            }),
                    ),
            )
    }
}

impl Render for ScheduledSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let hover_bg = t.list_hover;
        let rows: Vec<_> = self.tasks.iter().map(|task| self.row(task, t)).collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .child(
                        div()
                            .h(px(52.))
                            .ml(px(8.))
                            .pr(px(4.))
                            .flex()
                            .items_center()
                            .child(div().flex_1().font_family("OpenAI Sans").child(text(
                                "Scheduled",
                                18.,
                                24.,
                                600.,
                                t.text_default_nav,
                            )))
                            .child(
                                icon_button(
                                    "search-tasks",
                                    icon("search", 16., t.text_tertiary),
                                    t,
                                )
                                .on_click(crate::dispatch::<SearchTasks>),
                            ),
                    )
                    .child(
                        div()
                            .id("new-task")
                            .group("new-task")
                            .relative()
                            .mb(px(8.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .rounded(px(12.5))
                            .child(
                                fill(px(12.5), hover_bg)
                                    .opacity(0.)
                                    .group_hover("new-task", |s| s.opacity(1.)),
                            )
                            .on_click(crate::dispatch::<NewTask>)
                            .child(icon("automations-plus", 16., t.text_default_nav))
                            .child(text("New task", 14., 21., 430., t.text_default_nav)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .px(px(8.))
                    .pb(px(8.))
                    .child(
                        div()
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(8.))
                            .pr(px(6.))
                            .child(text("Upcoming", 14., 21., 500., t.text_tertiary))
                            .child(
                                icon_button(
                                    "filter-tasks",
                                    icon("automations-filter", 16., t.text_tertiary),
                                    t,
                                )
                                .on_click(crate::dispatch::<FilterTasks>),
                            ),
                    )
                    .when(rows.is_empty(), |d| {
                        d.child(div().mt(px(1.)).px(px(8.)).py(px(4.)).child(text(
                            "No scheduled tasks",
                            13.,
                            18.571_428,
                            430.,
                            t.text_tertiary,
                        )))
                    })
                    .when(!rows.is_empty(), |d| {
                        d.child(div().flex().flex_col().gap(px(1.)).children(rows))
                    }),
            )
    }
}

/// Main column of the empty page: hero and the suggested-task grid. While `pending` (a card
/// index) starts its conversation, that card shows the rays spinner and `Starting…` instead
/// of its description, and every card is disabled (`disabled:opacity-40`, no hover fill, no
/// click).
pub struct AutomationsPage {
    cards: Vec<Suggestion>,
    pending: Option<usize>,
}

impl AutomationsPage {
    /// The card whose conversation is starting.
    pub fn pending(&self) -> Option<usize> {
        self.pending
    }

    pub fn set_pending(&mut self, pending: Option<usize>, cx: &mut Context<Self>) {
        self.pending = pending;
        cx.notify();
    }

    fn card(&self, i: usize, s: &Suggestion, cx: &mut Context<Self>) -> Stateful<Div> {
        let t: &Theme = cx.theme();
        let hover_bg = t.ghost_hover;
        let disabled = self.pending.is_some();
        let starting = self.pending == Some(i);
        // `icon_url` is shown from its bundled copy under assets/ui/automations, its paths
        // rasterized by Chromium's Skia like Blink draws an svg `<img>`. Elsewhere GPUI's own
        // image raster (the copies declare 40x40, the 20 px slot at 2x: GPUI rasterizes svg
        // images at their intrinsic size).
        let glyph = match &s.icon_url {
            // ponytail: the original spins the rays (`animate-spin`); animations are out of scope.
            _ if starting => icon("rays", 20., t.text_tertiary).into_any_element(),
            Some(url) => {
                let path = format!("ui/automations/{}", url.rsplit('/').next().unwrap_or(url));
                if cfg!(target_os = "macos") {
                    canvas(
                        |_, _, _| {},
                        move |b, (), window, cx| {
                            if let Ok(Some(src)) = cx.asset_source().load(&path) {
                                let white = gpui::white();
                                shapes::paint_icon(&src, &path, b, false, white, None, window, cx);
                            }
                        },
                    )
                    .size(px(20.))
                    .into_any_element()
                } else {
                    img(path).size(px(20.)).into_any_element()
                }
            }
            None => text(s.emoji.clone(), 28., 28., 430., t.text).into_any_element(),
        };
        let prompt = s.user_message.clone();
        let description = if starting {
            "Starting…".into()
        } else {
            s.description.clone()
        };
        div()
            .id(("suggestion", i))
            .flex_1()
            .min_w_0()
            .min_h(px(108.))
            .flex()
            .items_center()
            .gap(px(16.))
            .relative()
            // 20 px padding inside a 1 px dashed border.
            .p(px(21.))
            .rounded(px(24.))
            // `border: 1px dashed` (Blink's curved dashed side, `shapes`).
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(24.)),
                border: Some((px(1.), t.border)),
                dashed: true,
                ..Default::default()
            }))
            .when(disabled, |d| d.opacity(0.4))
            .when(!disabled, |d| {
                d.hover(move |st| st.bg(hover_bg)).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.set_pending(Some(i), cx);
                        window.dispatch_action(
                            Box::new(StartSuggestedTask {
                                prompt: prompt.clone(),
                            }),
                            cx,
                        )
                    },
                ))
            })
            .child(
                div()
                    .size(px(24.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        text(s.title.clone(), 16., 26., 400., t.text)
                            .font_features(textsys::letter_spacing(-0.32))
                            .truncate(),
                    )
                    .child(
                        text(description, 14., 20., 400., t.text_tertiary)
                            .whitespace_normal()
                            .line_clamp(2)
                            .text_ellipsis(),
                    ),
            )
    }
}

impl Render for AutomationsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .cards
            .chunks(2)
            .enumerate()
            .map(|(r, pair)| {
                div().flex().gap(px(20.)).children(
                    pair.iter()
                        .enumerate()
                        .map(|(c, s)| self.card(r * 2 + c, s, cx))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let t = cx.theme();
        div()
            .size_full()
            .flex()
            .flex_col()
            // Page header row (`--height-toolbar`), empty in this state.
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("automations-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .min_h_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .pt(px(24.))
                            .px(px(12.))
                            .pb(px(44.))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(738.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(44.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap(px(12.))
                                            .child(
                                                text("🕗", 48., 48., 400., t.text)
                                                    .font_features(textsys::letter_spacing(-1.2)),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .items_center()
                                                    .gap(px(2.))
                                                    .child(
                                                        text("Schedule a task", 18., 26., 400., t.text)
                                                            .font_features(textsys::letter_spacing(-0.45)),
                                                    )
                                                    .child(
                                                        text(
                                                            "ChatGPT can take care of ongoing tasks so you don’t have to.",
                                                            14.,
                                                            20.,
                                                            400.,
                                                            t.text_tertiary,
                                                        )
                                                        .font_features(textsys::letter_spacing(-0.14)),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .gap(px(20.))
                                            .children(rows),
                                    ),
                            ),
                    ),
            )
    }
}

/// Main column of `/scheduled?automationId=<id>`: the task's thread as the original shows it
/// before any run (date separator, the prompt as the user's message, `Your task is
/// scheduled…` with the automation card) over the docked composer.
/// ponytail: no scroll fades, corner clip, message hover actions or run items; add with the
/// run-history rows.
pub struct AutomationThread {
    task: Task,
    composer: AnyView,
    utc_offset: i64,
}

impl Render for AutomationThread {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let created_time = day_time(self.task.created_at, self.utc_offset).1;
        // `<day>{date}</day> at {time}` with the year: the separator of a thread from another year.
        let created =
            chrono::DateTime::from_timestamp(self.task.created_at / 1000 + self.utc_offset, 0)
                .unwrap_or_default()
                .format("%b %-d, %Y")
                .to_string();
        let next = self
            .task
            .next_run_at
            .map(|at| day_time(at, self.utc_offset))
            .map_or_else(|| "soon".to_owned(), |(day, time)| format!("{day}, {time}"));
        let column = |d: gpui::Div| d.mx_auto().w_full().max_w(px(768.)).px(px(16.));
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(t.surface)
            .child(
                div()
                    .id("automation-thread")
                    .size_full()
                    .overflow_y_scroll()
                    .pt(px(32.))
                    .child(
                        column(div())
                            .pb(px(32.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .justify_center()
                                    .py(px(16.))
                                    .child(text(created, 13., 20., 500., t.text_tertiary))
                                    .child(text(
                                        format!(" at {created_time}"),
                                        13.,
                                        20.,
                                        400.,
                                        t.text_tertiary,
                                    )),
                            )
                            .child(
                                div().flex().justify_end().child(
                                    div()
                                        .relative()
                                        .max_w(px(576.))
                                        .px(px(16.))
                                        .py(px(10.))
                                        .child(fill(px(22.), t.user_message_background))
                                        .child(
                                            text(
                                                self.task.prompt.clone(),
                                                14.,
                                                22.75,
                                                430.,
                                                t.text,
                                            )
                                            .whitespace_normal(),
                                        ),
                                ),
                            )
                            // The user message's action row, then the turn gap.
                            .child(div().h(px(46.)))
                            .child(
                                text(
                                    format!("Your task is scheduled. Its next run is {next}."),
                                    14.,
                                    22.75,
                                    430.,
                                    t.text,
                                )
                                .whitespace_normal(),
                            )
                            .child(
                                div()
                                    .relative()
                                    .mt(px(24.))
                                    .h(px(65.))
                                    .p(px(12.))
                                    .child(fill(px(20.), t.surface_elevated_secondary.opacity(0.5)))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .size(px(40.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .relative()
                                            .child(fill(
                                                px(12.5),
                                                t.surface_secondary.opacity(0.92),
                                            ))
                                            .child(icon("scheduled", 24., t.text_secondary)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                text(
                                                    self.task.name.clone(),
                                                    14.,
                                                    21.,
                                                    500.,
                                                    t.text,
                                                )
                                                .overflow_hidden()
                                                .text_ellipsis(),
                                            )
                                            .child(text(
                                                self.task.schedule(),
                                                13.,
                                                20.,
                                                430.,
                                                t.text_secondary,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .h(px(28.))
                                            .px(px(8.))
                                            .rounded(px(12.5))
                                            .border_1()
                                            .border_color(t.border)
                                            .flex()
                                            .items_center()
                                            .child(text("Open", 14., 18., 430., t.text)),
                                    ),
                            ),
                    )
                    // Room for the docked composer.
                    .child(div().h(px(114.))),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .pb(px(16.))
                    .bg(t.surface)
                    .child(column(div()).child(self.composer.clone())),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cards_fill_six_from_oracle() {
        let server = serde_json::from_value(super::oracle("\"url\":\"/suggested_automations\""))
            .expect("shape");
        let titles: Vec<_> = super::cards(server).into_iter().map(|s| s.title).collect();
        assert_eq!(titles.len(), 6);
        assert_eq!(titles[0], "AP Lang practice");
        assert_eq!(titles[3], "Weekly meal plan");
    }
}
