//! `automations` parity screen: rail `Scheduled` (`/automations`). Sidebar `ScheduledSidebar`
//! and the page's empty state with suggested tasks, from `refs/automations/*.spec.json`.
//! Host data (`list-automations`, `/suggested_automations`) comes from the recorded answers
//! in `tools/route1/oracle.json`.

use crate::{
    Shell, focus,
    icon::icon,
    icon_button,
    motion::ButtonVisual,
    text,
    tooltip::{Side, tooltip},
};
use gpui::{
    Action, Anchor, AnyView, App, AppContext as _, ClickEvent, Context, Corners, Div, Entity,
    FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    Role, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, actions, anchored,
    canvas, deferred, div, img, prelude::FluentBuilder as _, px,
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, fs, path::Path};
use theme::{ActiveTheme as _, Theme};

pub mod editor;
mod history;
mod management;
mod search;
pub use history::{
    ArchiveAllRuns, ArchiveRun, MarkAllRunsRead, RunHistory, SetRunRead, archiveable,
};

#[derive(Clone, Copy, PartialEq)]
pub enum CreationMode {
    Codex,
    ChatGPT,
    Manual,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct CreateAutomation {
    pub mode: CreationMode,
}
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

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct OpenAutomationManagement {
    pub id: Option<String>,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct EditAutomation {
    pub id: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct ToggleAutomation {
    pub id: String,
}

/// `Run now` in a task row's menu: the app starts a run through the host's `automation-run-now`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct RunAutomationNow {
    pub id: String,
}

/// One `list-automations` item, as the sidebar `Upcoming` list and the task thread show it.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub name: SharedString,
    pub prompt: SharedString,
    pub status: String,
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

/// An event-triggered task: its prompt is the original's `## When this happens` / `## Perform
/// action` quote blocks (`zhi`). Only an event runs it, so `Run now` is off for it.
pub fn event_triggered(prompt: &str) -> bool {
    let quoted = |line: &str| line == ">" || line.starts_with("> ");
    prompt
        .strip_prefix("## When this happens\n")
        .and_then(|rest| rest.split_once("\n\n## Perform action\n"))
        .is_some_and(|(when, action)| {
            when.lines().all(quoted) && action.lines().next().is_some_and(quoted)
        })
}

const TRIGGER_TOOLTIP: &str = "Automations triggered by events can’t be run manually";

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
            .lines()
            .flat_map(|line| line.trim().trim_start_matches("RRULE:").split(';'))
            .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
    }

    fn cadence(&self) -> SharedString {
        const MINUTES_PER_HOUR: u64 = 60;
        const MINUTES_PER_DAY: u64 = 24 * MINUTES_PER_HOUR;
        const MINUTES_PER_WEEK: u64 = 7 * MINUTES_PER_DAY;
        let factor = match self.rule("FREQ") {
            Some("MINUTELY") => 1,
            Some("HOURLY") => MINUTES_PER_HOUR,
            Some("DAILY") => return "Daily".into(),
            Some("WEEKLY") if self.rule("BYDAY") == Some("MO,TU,WE,TH,FR") => {
                return "Weekdays".into();
            }
            Some("WEEKLY") => return "Weekly".into(),
            Some("MONTHLY") => return "Monthly".into(),
            Some("YEARLY") => return "Yearly".into(),
            _ => return "Custom".into(),
        };
        let Some(minutes) = self
            .rule("INTERVAL")
            .unwrap_or("1")
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_mul(factor))
            .filter(|n| *n > 0)
        else {
            return "Custom schedule".into();
        };
        let fixed = match minutes {
            1 => Some("Every minute"),
            MINUTES_PER_HOUR => Some("Hourly"),
            MINUTES_PER_DAY => Some("Daily"),
            MINUTES_PER_WEEK => Some("Weekly"),
            _ => None,
        };
        if let Some(label) = fixed {
            return label.into();
        }
        let (count, unit) = if minutes.is_multiple_of(MINUTES_PER_WEEK) {
            (minutes / MINUTES_PER_WEEK, "weeks")
        } else if minutes.is_multiple_of(MINUTES_PER_DAY) {
            (minutes / MINUTES_PER_DAY, "days")
        } else if minutes.is_multiple_of(MINUTES_PER_HOUR) {
            (minutes / MINUTES_PER_HOUR, "hours")
        } else {
            (minutes, "minutes")
        };
        format!("Every {count} {unit}").into()
    }

    /// Card schedule: `Fridays at 9:00 AM`, `Daily at 8:00 AM`.
    fn schedule(&self) -> String {
        let hour = self.rule("BYHOUR").and_then(|h| h.parse::<u32>().ok());
        let minute = self.rule("BYMINUTE").and_then(|m| m.parse::<u32>().ok());
        let (Some(hour), Some(minute)) = (hour, minute) else {
            return self.cadence().to_string();
        };
        let time = format!(
            "{}:{minute:02} {}",
            match hour % 12 {
                0 => 12,
                h => h,
            },
            if hour < 12 { "AM" } else { "PM" }
        );
        let day = match (self.rule("FREQ"), self.rule("BYDAY")) {
            (Some("WEEKLY"), Some("MO")) => "Mondays".into(),
            (Some("WEEKLY"), Some("TU")) => "Tuesdays".into(),
            (Some("WEEKLY"), Some("WE")) => "Wednesdays".into(),
            (Some("WEEKLY"), Some("TH")) => "Thursdays".into(),
            (Some("WEEKLY"), Some("FR")) => "Fridays".into(),
            (Some("WEEKLY"), Some("SA")) => "Saturdays".into(),
            (Some("WEEKLY"), Some("SU")) => "Sundays".into(),
            _ => self.cadence(),
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
    let sidebar = scheduled_sidebar(
        tasks,
        selected.as_ref().map(|(task, _)| task.id.clone()),
        utc_offset,
        cx,
    );
    let mut page = None;
    let main: AnyView = match selected {
        Some((task, composer)) => {
            let history = cx.new(|cx| RunHistory::new(task.id.clone(), cx));
            sidebar.update(cx, |sidebar, _| sidebar.history = Some(history.clone()));
            cx.new(|_| AutomationThread {
                task,
                composer,
                utc_offset,
                history,
            })
            .into()
        }
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

fn scheduled_sidebar(
    tasks: Vec<Task>,
    selected: Option<String>,
    utc_offset: i64,
    cx: &mut App,
) -> Entity<ScheduledSidebar> {
    cx.new(|cx| ScheduledSidebar {
        history: None,
        tasks,
        selected,
        utc_offset,
        management: false,
        pending: HashSet::new(),
        starting: HashSet::new(),
        menu_item: None,
        menu: None,
        menu_button_hovered: false,
        menu_focus: cx.focus_handle(),
        return_focus: None,
        create_menu: cx.new(editor::CreateMenu::new),
    })
}

pub fn management_shell(
    tasks: Vec<Task>,
    selected: Option<String>,
    utc_offset: i64,
    cx: &mut App,
) -> (Entity<Shell>, Entity<ScheduledSidebar>) {
    let sidebar = scheduled_sidebar(tasks, selected.clone(), utc_offset, cx);
    sidebar.update(cx, |sidebar, _| sidebar.management = true);
    if let Some(id) = selected {
        let history = cx.new(|cx| RunHistory::new(id, cx));
        sidebar.update(cx, |sidebar, cx| {
            cx.observe(&history, |_, _, cx| cx.notify()).detach();
            sidebar.history = Some(history);
        });
    }
    let main = cx.new(|cx| management::Management::new(sidebar.clone(), cx));
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar.clone().into(), main.into());
        shell.selected_rail = Some("scheduled");
        shell.new_tab = false;
        shell
    });
    (shell, sidebar)
}

/// `ScheduledSidebar`: header `Scheduled` + search, `New task`, section `Upcoming` with one
/// 48 px row per task (`automation-row`): name, `Oct 17 · Weekly`, and on hover the list
/// fill with the `Scheduled task actions` button fading in. The selected row keeps the fill.
pub struct ScheduledSidebar {
    tasks: Vec<Task>,
    pub history: Option<Entity<RunHistory>>,
    selected: Option<String>,
    utc_offset: i64,
    management: bool,
    pending: HashSet<String>,
    /// Tasks whose pending run disables `Run now`.
    starting: HashSet<String>,
    /// The keyboard-active menu item (`0` Run now, `1` Pause or Resume) once an arrow key moved it.
    menu_item: Option<usize>,
    /// The row whose `Scheduled task actions` menu is open.
    menu: Option<String>,
    /// The menu button is under the pointer: a press on it toggles the menu, not closes it.
    menu_button_hovered: bool,
    menu_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    create_menu: Entity<editor::CreateMenu>,
}

impl ScheduledSidebar {
    pub fn set_tasks(&mut self, tasks: Vec<Task>, cx: &mut Context<Self>) {
        self.tasks = tasks;
        cx.notify();
    }

    pub fn set_pending(&mut self, id: &str, pending: bool, cx: &mut Context<Self>) {
        if pending {
            self.pending.insert(id.to_owned());
        } else {
            self.pending.remove(id);
        }
        cx.notify();
    }

    pub fn set_starting(&mut self, starting: HashSet<String>, cx: &mut Context<Self>) {
        self.starting = starting;
        cx.notify();
    }

    /// The task's run is starting.
    pub fn is_starting(&self, id: &str) -> bool {
        self.starting.contains(id)
    }

    /// The task whose actions menu is open.
    pub fn open_menu(&self) -> Option<&str> {
        self.menu.as_deref()
    }

    fn run_now(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.starting.contains(id) {
            return;
        }
        window.dispatch_action(Box::new(RunAutomationNow { id: id.to_owned() }), cx);
        self.close_menu(window, cx);
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.menu_item = None;
        if let Some(focus) = self.return_focus.take() {
            window.focus(&focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn toggle_menu(
        &mut self,
        id: &str,
        button: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu.as_deref() == Some(id) {
            self.close_menu(window, cx);
            return;
        }
        self.menu = Some(id.to_owned());
        self.menu_item = None;
        self.return_focus = Some(button);
        window.focus(&self.menu_focus, cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn header_actions(
        &self,
        task: &Task,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let open = self.menu.as_deref() == Some(task.id.as_str());
        let focus = focus::handle("managed-task-actions", 0, window, cx);
        let return_focus = focus.clone();
        let id = task.id.clone();
        focus::ring(
            icon_button(
                "managed-task-actions",
                icon("thread-chat-actions", 16., t.text_tertiary),
                t,
            )
            .size(px(28.))
            .role(Role::Button)
            .aria_label("Scheduled task actions")
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_menu(&id, return_focus.read(cx).handle.clone(), window, cx);
            }))
            .when(open, |d| d.child(self.menu_popup(task, t, cx))),
            &focus,
            px(9.375),
            px(2.),
            cx,
        )
    }

    fn toggle(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        window.dispatch_action(Box::new(ToggleAutomation { id: id.to_owned() }), cx);
    }

    fn menu_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.close_menu(window, cx),
            "enter" | "space" => {
                if let Some(id) = self.menu.clone() {
                    if self.menu_item == Some(0) {
                        self.run_now(&id, window, cx);
                    } else if self.menu_item == Some(2) {
                        self.close_menu(window, cx);
                        window.dispatch_action(Box::new(EditAutomation { id }), cx);
                    } else {
                        self.toggle(&id, window, cx);
                    }
                }
            }
            "up" | "down" | "home" | "end" => {
                self.menu_item = Some(match event.keystroke.key.as_str() {
                    "home" => 0,
                    "end" => 2,
                    "down" => self.menu_item.map_or(1, |i| (i + 1) % 3),
                    _ => self.menu_item.map_or(0, |i| (i + 2) % 3),
                });
                cx.stop_propagation();
                cx.notify();
            }
            "tab" => cx.stop_propagation(),
            _ => {}
        }
    }

    /// The original menu: opens under the button, right edge 1 px inside it, 220 px wide.
    fn menu_popup(&self, task: &Task, t: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        let paused = task.status == "PAUSED";
        let label = if paused { "Resume" } else { "Pause" };
        let (id, hover) = (task.id.clone(), t.menu_item_hover);
        let triggered = event_triggered(&task.prompt);
        let off = triggered || self.starting.contains(&task.id);
        let run_id = task.id.clone();
        let run = div()
            .id("run-now")
            .role(Role::MenuItem)
            .aria_label("Run now")
            .aria_disabled(off)
            .h(px(28.5625))
            .px(px(8.))
            .rounded(px(15.))
            .flex()
            .items_center()
            .when(self.menu_item == Some(0) && !off, |d| d.bg(hover))
            .when(off, |d| d.opacity(0.4))
            .when(!off, |d| {
                d.hover(move |s| s.bg(hover)).on_click(
                    cx.listener(move |this, _, window, cx| this.run_now(&run_id, window, cx)),
                )
            })
            .child(text("Run now", 13., 18.5714, 430., t.text));
        let run = if triggered {
            tooltip(run, TRIGGER_TOOLTIP, None, Side::Right).into_any_element()
        } else {
            run.into_any_element()
        };
        let item = div()
            .id("toggle-status")
            .group("toggle-status-item")
            .role(Role::MenuItem)
            .aria_label(label)
            .aria_active_descendant()
            .aria_disabled(self.pending.contains(&task.id))
            .h(px(28.5625))
            .px(px(8.))
            .rounded(px(15.))
            .flex()
            .items_center()
            .gap(px(6.))
            .when(self.menu_item == Some(1), |d| d.bg(hover))
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, window, cx| this.toggle(&id, window, cx)))
            .child(
                div()
                    .size(px(16.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .opacity(0.75)
                    .group_hover("toggle-status-item", |s| s.opacity(1.))
                    .child(status_icon(paused, t.text)),
            )
            .child(text(label, 13., 18.5714, 430., t.text));
        let edit_id = task.id.clone();
        let edit = div()
            .id("edit-automation")
            .role(Role::MenuItem)
            .aria_label("Edit")
            .h(px(28.5625))
            .px(px(8.))
            .rounded(px(15.))
            .flex()
            .items_center()
            .when(self.menu_item == Some(2), |d| d.bg(hover))
            .hover(move |s| s.bg(hover))
            .child(text("Edit", 13., 18.5714, 430., t.text))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.close_menu(window, cx);
                window.dispatch_action(
                    Box::new(EditAutomation {
                        id: edit_id.clone(),
                    }),
                    cx,
                );
            }));
        let menu = div()
            .id("task-actions-menu")
            .occlude()
            .role(Role::Menu)
            .aria_label("Scheduled task actions")
            .track_focus(&self.menu_focus)
            .on_key_down(cx.listener(Self::menu_key))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if !this.menu_button_hovered {
                    this.close_menu(window, cx);
                }
            }))
            .child(crate::composer::popover(t, 220., |menu| {
                menu.child(run).child(item).child(edit)
            }));
        deferred(
            div().absolute().top(px(30.)).right(px(1.)).child(
                anchored()
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(px(7.))
                    .child(menu),
            ),
        )
        .with_priority(1)
    }

    fn row(
        &self,
        task: &Task,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let has_history = self.history.as_ref().is_some_and(|history| {
            let history = history.read(cx);
            history.id == task.id && !history.items().is_empty()
        });
        let hover = if self.management && t.appearance == theme::Appearance::Dark {
            gpui::Hsla {
                a: 1. - (1. - 0.08),
                ..t.text_alpha_10
            }
        } else {
            t.ghost_hover
        };
        let fill = fill(px(12.5), hover);
        let group = SharedString::from(format!("task-row-{}", task.id));
        let id = task.id.clone();
        let management = self.management;
        let pending = self.pending.contains(&task.id);
        let paused = task.status == "PAUSED";
        let label = if paused { "Resume" } else { "Pause" };
        let status_id = SharedString::from(format!("{group}-status"));
        let status_focus = focus::handle(status_id.clone(), 0, window, cx);
        let menu_id = SharedString::from(format!("{group}-actions"));
        let menu_focus = focus::handle(menu_id.clone(), 0, window, cx);
        let status_focused = status_focus.read(cx).handle.is_focused(window);
        let status_focus_visible = status_focus.read(cx).is_focus_visible(window);
        let open = self.menu.as_deref() == Some(task.id.as_str());
        let show_controls =
            status_focus_visible || menu_focus.read(cx).is_focus_visible(window) || open;
        let status_task = task.id.clone();
        let (press_task, key_task) = (task.id.clone(), task.id.clone());
        let (press_return, key_return) = (menu_focus.clone(), menu_focus.clone());
        let status = focus::ring(
            div()
                .id(status_id.clone())
                .group(status_id.clone())
                .role(Role::Button)
                .aria_label(format!("{label} {}", task.name))
                .aria_disabled(pending)
                .relative()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.375))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.toggle(&status_task, window, cx)),
                )
                .child(if pending {
                    icon("sidebar-cmd-spinner", 16., t.text_tertiary).into_any_element()
                } else if paused {
                    status_icon(true, t.text_tertiary).into_any_element()
                } else {
                    div()
                        .relative()
                        .size(px(16.))
                        .child(
                            icon("scheduled", 16., t.text_tertiary)
                                .opacity(if status_focused { 0. } else { 1. })
                                .group_hover(status_id.clone(), |s| s.opacity(0.)),
                        )
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .opacity(if status_focused { 1. } else { 0. })
                                .group_hover(status_id.clone(), |s| s.opacity(1.))
                                .child(status_icon(false, t.text_tertiary)),
                        )
                        .into_any_element()
                }),
            &status_focus,
            px(9.375),
            px(2.),
            cx,
        );
        let menu = focus::ring(
            div()
                .id(menu_id)
                .relative()
                .role(Role::Button)
                .aria_label("Scheduled task actions")
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.375))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        let button = press_return.read(cx).handle.clone();
                        this.toggle_menu(&press_task, button, window, cx);
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.is_keyboard() {
                        let button = key_return.read(cx).handle.clone();
                        this.toggle_menu(&key_task, button, window, cx);
                    }
                }))
                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                    this.menu_button_hovered = *hovered;
                }))
                .child(ButtonVisual {
                    glyph: icon("thread-chat-actions", 16., t.text_tertiary),
                    radius: px(9.375),
                    hover: t.ghost_hover,
                    foreground: t.text.opacity(0.85),
                    selected: open,
                })
                .when(open, |d| d.child(self.menu_popup(task, t, cx))),
            &menu_focus,
            px(9.375),
            px(2.),
            cx,
        );
        let mut secondary = div().h(px(16.)).pr(px(5.)).flex().items_center();
        if !paused && let Some(next) = task.next_run_at {
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
                if management {
                    window.dispatch_action(
                        Box::new(OpenAutomationManagement {
                            id: Some(id.clone()),
                        }),
                        cx,
                    );
                } else {
                    window.dispatch_action(Box::new(OpenAutomation { id: id.clone() }), cx);
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        text(task.name.clone(), 14., 20., 430., t.text_default_nav)
                            .overflow_hidden()
                            .text_ellipsis(),
                    )
                    .when(has_history, |d| {
                        d.child(icon("chevron-down-small", 12., t.text_tertiary))
                    }),
            )
            .child(secondary.child(text(
                if paused {
                    "Paused".into()
                } else {
                    task.cadence()
                },
                12.,
                16.,
                430.,
                t.text_tertiary,
            )))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right(px(2.))
                    .h_full()
                    .w(px(80.))
                    .pr(px(6.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .opacity(if show_controls { 1. } else { 0. })
                    .group_hover(group.clone(), |s| s.opacity(1.))
                    .child(tooltip(
                        status
                            .opacity(if status_focus_visible { 1. } else { 0. })
                            .group_hover(group.clone(), |s| s.opacity(1.)),
                        label,
                        None,
                        Side::Top,
                    ))
                    .child(menu),
            )
    }
}

impl Render for ScheduledSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let hover_bg = t.list_hover;
        let rows: Vec<_> = self
            .tasks
            .iter()
            .map(|task| {
                let mut row = div().child(self.row(task, &t, window, cx));
                if let Some(history) = self
                    .history
                    .as_ref()
                    .filter(|history| history.read(cx).id == task.id)
                {
                    for (index, run) in history
                        .read(cx)
                        .items()
                        .iter()
                        .filter(|run| run["status"] != "ARCHIVED")
                        .enumerate()
                    {
                        let Some(thread) = run["threadId"].as_str().map(str::to_owned) else {
                            continue;
                        };
                        let history = history.clone();
                        row = row.child(
                            div()
                                .id(("scheduled-run", index))
                                .role(Role::Button)
                                .h(px(30.))
                                .mt(px(1.))
                                .pl(px(24.))
                                .pr(px(8.))
                                .rounded(px(12.5))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .hover(|s| s.bg(t.list_hover))
                                .child(
                                    text(
                                        run["title"].as_str().unwrap_or(&task.name).to_owned(),
                                        14.,
                                        20.,
                                        430.,
                                        t.text_default_nav,
                                    )
                                    .flex_1()
                                    .overflow_hidden()
                                    .text_ellipsis(),
                                )
                                .when(run["readAt"].is_null(), |d| {
                                    d.child(
                                        div()
                                            .size(px(20.))
                                            .mr(px(-4.))
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
                                })
                                .on_click(move |_, _, cx| {
                                    history.update(cx, |_, cx| {
                                        cx.emit(crate::screens::sidebar::OpenThread(thread.clone()))
                                    })
                                }),
                        );
                    }
                }
                row
            })
            .collect();
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
                                    &t,
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
                            .child(text("New task", 14., 21., 430., t.text_default_nav))
                            .child(div().ml_auto().child(self.create_menu.clone())),
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
                                    &t,
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
                div()
                    .flex()
                    .gap(px(20.))
                    .children(
                        pair.iter()
                            .enumerate()
                            .map(|(c, s)| self.card(r * 2 + c, s, cx))
                            .collect::<Vec<_>>(),
                    )
                    .when(pair.len() == 1, |row| row.child(div().flex_1()))
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
pub struct AutomationThread {
    task: Task,
    composer: AnyView,
    utc_offset: i64,
    history: Entity<RunHistory>,
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
                            )
                            .child(self.history.clone()),
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

fn status_icon(paused: bool, color: gpui::Hsla) -> crate::icon::Icon {
    icon(
        if paused {
            "automation-resume"
        } else {
            "automation-pause"
        },
        16.,
        color,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn calendar_and_interval_rules_keep_distinct_and_equivalent_cadences() {
        let cadence = |rule: &str| {
            let task: super::Task = serde_json::from_value(serde_json::json!({
                "id": "interval", "name": "Monitor", "prompt": "Check", "status": "ACTIVE", "rrule": rule,
            })).unwrap();
            task.cadence()
        };
        assert_ne!(cadence("FREQ=MINUTELY;INTERVAL=30"), cadence("FREQ=DAILY"));
        assert_ne!(
            cadence("FREQ=MINUTELY;INTERVAL=30"),
            cadence("FREQ=HOURLY;INTERVAL=30")
        );
        assert_eq!(cadence("FREQ=MINUTELY;INTERVAL=60"), cadence("FREQ=HOURLY"));
        assert_eq!(
            cadence("FREQ=MINUTELY;INTERVAL=1440"),
            cadence("FREQ=DAILY")
        );
        assert_eq!(cadence("FREQ=HOURLY;INTERVAL=168"), cadence("FREQ=WEEKLY"));
        assert_ne!(cadence("FREQ=MONTHLY"), cadence("FREQ=DAILY"));
        assert_ne!(cadence("FREQ=YEARLY"), cadence("FREQ=MONTHLY"));
        assert_ne!(
            cadence("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
            cadence("FREQ=WEEKLY")
        );
    }

    #[test]
    fn only_the_original_event_prompt_is_event_triggered() {
        let event = "## When this happens\n> a PR opens\n>\n> in main\n\n## Perform action\n> review it\n\nTrailing";
        assert!(super::event_triggered(event));
        assert!(!super::event_triggered("Review the PR every day"));
        assert!(!super::event_triggered(
            "## When this happens\nunquoted\n\n## Perform action\n> review it"
        ));
        assert!(!super::event_triggered(
            "## When this happens\n> a PR opens\n\n## Perform action\nunquoted"
        ));
    }
}
