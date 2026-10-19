//! `automations` parity screen: rail `Scheduled` (`/automations`). Sidebar `ScheduledSidebar`
//! and the page's empty state with suggested tasks, from `refs/automations/*.spec.json`.
//! Host data (`list-automations`, `/suggested_automations`) comes from the recorded answers
//! in `tools/route1/oracle.json`.

use crate::{Shell, icon::icon, icon_button, motion::ButtonVisual, text};
use gpui::{
    Action, Anchor, AnyView, App, AppContext as _, Bounds, BoxShadow, ClickEvent, Context, Corners,
    Div, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Point, Render, Role, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, actions, anchored, canvas, deferred, div, img, point,
    prelude::FluentBuilder as _, px, rgb,
};
use serde::Deserialize;
use serde_json::Value;
use std::{cell::Cell, collections::HashMap, fs, path::Path, rc::Rc};
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

/// Row menu `Delete`: asks to delete the task `id` (AM-026); the app confirms first.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct DeleteAutomation {
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
pub struct Suggestion {
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
/// third). Host suggestions replace the landing examples when the read completes.
/// Dates show at `utc_offset` seconds.
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
        triggers: HashMap::new(),
        menu: None,
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
    /// Each row's `Scheduled task actions` button, by task id.
    triggers: HashMap<String, Trigger>,
    menu: Option<TaskMenu>,
}

/// A row's `Scheduled task actions` button: its focus and where it was last laid out.
struct Trigger {
    focus: FocusHandle,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

/// The open row menu: its task, where it opens, and its focus while open.
struct TaskMenu {
    id: String,
    position: Point<Pixels>,
    active: bool,
    focus: FocusHandle,
}

/// The original's trash icon (`settings.automations.delete`), 16 px.
const DELETE_ICON: &[u8] = br##"<svg height="16" viewBox="0 0 16 16" width="16" xmlns="http://www.w3.org/2000/svg"><path d="M6.66724 6.80762C6.95696 6.80788 7.19263 7.04322 7.19263 7.33301V10.6729C7.19256 10.9626 6.95692 11.197 6.66724 11.1973C6.37733 11.1973 6.14192 10.9627 6.14185 10.6729V7.33301C6.14185 7.04306 6.37729 6.80762 6.66724 6.80762Z" fill="currentColor"></path> <path d="M9.33325 6.80762C9.6232 6.80762 9.85864 7.04306 9.85864 7.33301V10.6729C9.85857 10.9627 9.62316 11.1973 9.33325 11.1973C9.04335 11.1973 8.80793 10.9627 8.80786 10.6729V7.33301C8.80786 7.04306 9.0433 6.80762 9.33325 6.80762Z" fill="currentColor"></path> <path fill-rule="evenodd" clip-rule="evenodd" d="M8.00024 1.25391C9.66003 1.25407 11.0236 2.52186 11.177 4.1416H13.6663C13.9562 4.1416 14.1917 4.37704 14.1917 4.66699C14.1914 4.95672 13.956 5.19238 13.6663 5.19238H13.1692L12.6995 12.1787C12.6105 13.5045 11.5096 14.5349 10.1809 14.5352H5.8313C4.50407 14.535 3.40273 13.5068 3.31177 12.1826L2.83228 5.19238H2.33325C2.04347 5.19238 1.80813 4.95672 1.80786 4.66699C1.80786 4.37704 2.0433 4.1416 2.33325 4.1416H4.82251C4.976 2.52175 6.34031 1.25391 8.00024 1.25391ZM4.35962 12.1104C4.41275 12.8838 5.05604 13.4842 5.8313 13.4844H10.1809C10.957 13.4841 11.6006 12.8828 11.6526 12.1084L12.1165 5.19238H3.88403L4.35962 12.1104ZM8.00024 2.30469C6.92101 2.30469 6.03114 3.10331 5.88306 4.1416H10.1174C9.96932 3.10342 9.07933 2.30485 8.00024 2.30469Z" fill="currentColor"></path></svg>"##;

impl ScheduledSidebar {
    pub fn set_tasks(&mut self, tasks: Vec<Task>, cx: &mut Context<Self>) {
        self.tasks = tasks;
        cx.notify();
    }

    /// Where the open row menu's top-right corner is.
    pub fn menu_corner(&self) -> Option<Point<Pixels>> {
        self.menu.as_ref().map(|menu| menu.position)
    }

    /// The row's `Scheduled task actions` button, where focus returns after its menu or a
    /// dialog from it closes.
    pub fn trigger_focus(&self, id: &str) -> Option<FocusHandle> {
        self.triggers.get(id).map(|trigger| trigger.focus.clone())
    }

    /// The original's Radix menu opens under its button, right edges aligned, 2 px below it.
    fn open_menu(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(trigger) = self.triggers.get(&id) else {
            return;
        };
        let bounds = trigger.bounds.get();
        let position = point(bounds.right() - px(1.), bounds.bottom() + px(2.));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.menu = Some(TaskMenu {
            id,
            position,
            active: false,
            focus,
        });
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<TaskMenu> {
        let menu = self.menu.take()?;
        if let Some(trigger) = self.triggers.get(&menu.id) {
            window.focus(&trigger.focus, cx);
        }
        cx.notify();
        Some(menu)
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.close_menu(window, cx) {
            window.dispatch_action(Box::new(DeleteAutomation { id: menu.id }), cx);
        }
    }

    fn menu_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.close_menu(window, cx);
            }
            "enter" | "space" => self.delete(window, cx),
            "up" | "down" | "home" | "end" => {
                if let Some(menu) = &mut self.menu {
                    menu.active = true;
                }
            }
            "tab" => {}
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn menu_overlay(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let menu = self.menu.as_ref().expect("open task menu");
        let t = cx.theme();
        // ponytail: only `Delete` exists; Pause, Run now, Edit and Share join this list, in that
        // order above it, with their own slices.
        let item = div()
            .id("task-menu-delete")
            .role(Role::MenuItem)
            .aria_label("Delete")
            .when(menu.active, |d| {
                d.aria_active_descendant().bg(t.menu_item_hover)
            })
            .h(px(28.5625))
            .px(px(8.))
            .rounded(px(15.))
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if *hovered && let Some(menu) = &mut this.menu {
                    menu.active = true;
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx)))
            .child({
                let color = t.text_error;
                // `opacity-75`, full on hover and focus.
                let alpha = if menu.active { 1. } else { 0.75 };
                div().size(px(16.)).flex_none().child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, (), window, cx| {
                            shapes::paint_icon(
                                DELETE_ICON,
                                "task-menu-delete",
                                bounds,
                                false,
                                color.opacity(alpha),
                                None,
                                window,
                                cx,
                            );
                        },
                    )
                    .size(px(16.)),
                )
            })
            .child(text("Delete", 13., 18.571_428, 430., t.text_error));
        let popup = crate::composer::popover(t, 220., |d| d.child(item))
            .ml_0()
            .pl_0()
            .id("task-menu")
            .role(Role::Menu)
            .aria_label("Scheduled task actions")
            .track_focus(&menu.focus)
            .on_key_down(cx.listener(Self::menu_key))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_menu(window, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, window, cx| {
                            this.close_menu(window, cx);
                        }),
                    )
                    .child(
                        anchored()
                            .position(menu.position)
                            .anchor(Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(popup),
                    ),
            ),
        )
        .with_priority(4)
    }

    fn row(&self, task: &Task, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let fill = fill(px(12.5), t.ghost_hover);
        let group = SharedString::from(format!("task-row-{}", task.id));
        let id = task.id.clone();
        let (trigger, bounds) = {
            let trigger = &self.triggers[&task.id];
            (trigger.focus.clone(), trigger.bounds.clone())
        };
        let open = self.menu.as_ref().is_some_and(|menu| menu.id == task.id);
        let actions_shown = open || trigger.is_focused(window);
        let menu_id = task.id.clone();
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
            .child(if open || self.selected.as_deref() == Some(&task.id) {
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
                    .opacity(if actions_shown { 1. } else { 0. })
                    .group_hover(group.clone(), |s| s.opacity(1.))
                    .child(
                        div()
                            .relative()
                            .id(SharedString::from(format!("{group}-actions")))
                            .role(Role::Button)
                            .aria_label("Scheduled task actions")
                            .track_focus(&trigger)
                            .tab_index(0)
                            .focus_visible(|style| {
                                style.shadow(vec![BoxShadow {
                                    color: rgb(0x3a83f7).into(),
                                    offset: point(px(0.), px(0.)),
                                    blur_radius: px(0.),
                                    spread_radius: px(2.),
                                    inset: true,
                                }])
                            })
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.375))
                            // The original opens on pointer down; Enter and Space click.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let id = menu_id.clone();
                                    move |this, _, window, cx| {
                                        this.open_menu(id.clone(), window, cx);
                                        cx.stop_propagation();
                                    }
                                }),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                if event.is_keyboard() {
                                    this.open_menu(menu_id.clone(), window, cx)
                                }
                            }))
                            .child(
                                canvas(move |b, _, _| bounds.set(b), |_, (), _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                            .child(ButtonVisual {
                                glyph: icon("thread-chat-actions", 16., t.text_tertiary),
                                radius: px(9.375),
                                hover: t.ghost_hover,
                                foreground: t.text.opacity(0.85),
                                selected: open,
                            }),
                    ),
            )
    }
}

impl Render for ScheduledSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tasks = &self.tasks;
        self.triggers
            .retain(|id, _| tasks.iter().any(|task| task.id == *id));
        for task in tasks {
            self.triggers
                .entry(task.id.clone())
                .or_insert_with(|| Trigger {
                    focus: cx.focus_handle(),
                    bounds: Rc::default(),
                });
        }
        let t = cx.theme();
        let hover_bg = t.list_hover;
        let rows: Vec<_> = self
            .tasks
            .iter()
            .map(|task| self.row(task, window, cx))
            .collect();
        let overlay = self.menu.is_some().then(|| self.menu_overlay(window, cx));
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
            .children(overlay)
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
    pub fn set_suggestions(&mut self, server: Vec<Suggestion>, cx: &mut Context<Self>) {
        let pending = self.pending.map(|i| self.cards.swap_remove(i));
        self.cards = cards(server);
        if let Some(pending) = pending {
            self.pending = Some(
                self.cards
                    .iter()
                    .position(|card| card.user_message == pending.user_message)
                    .unwrap_or_else(|| {
                        self.cards.truncate(5);
                        self.cards.push(pending);
                        self.cards.len() - 1
                    }),
            );
        }
        cx.notify();
    }

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
        let bundled = (!starting)
            .then_some(s.icon_url.as_ref())
            .flatten()
            .and_then(|url| {
                let path = format!("ui/automations/{}", url.rsplit('/').next().unwrap_or(url));
                cx.asset_source()
                    .load(&path)
                    .ok()
                    .flatten()
                    .map(|source| (path, source))
            });
        let glyph = match bundled {
            // ponytail: the original spins the rays (`animate-spin`); animations are out of scope.
            _ if starting => icon("rays", 20., t.text_tertiary).into_any_element(),
            Some((path, source)) => {
                if cfg!(target_os = "macos") {
                    canvas(
                        |_, _, _| {},
                        move |b, (), window, cx| {
                            let white = gpui::white();
                            shapes::paint_icon(&source, &path, b, false, white, None, window, cx);
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
