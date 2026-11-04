//! Settings ▸ Computer history (`chronicle`, `ComputerHistorySettingsPage`): the off state with
//! its example carousel and `Turn on`, the consent, Memories and permissions dialogs, the on
//! state (toggle, pause/resume or setup, permissions) and the History timeline. The app installs
//! [`Service`], which answers the original's `electronBridge.chronicle` methods.
use super::{
    Layer, SettingsPage,
    account_page::{Kind, action, frame},
    settings_row_content, switch,
    worktrees::card,
};
use crate::{
    composer::Composer,
    confirmation::{Confirmation, Outcome},
    dialog::{ArtworkCache, gradient_artwork},
    icon::icon,
    settings_kit::{Choice, Select, Selected},
    text,
    toast::Toast,
    tooltip::{Side, tooltip},
};
use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Entity, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
    px, rgba,
};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc, time::Duration};
use theme::{ActiveTheme as _, Theme};

pub type Reply = Box<dyn FnOnce(anyhow::Result<Value>, &mut App)>;
pub type Dispatch = Arc<dyn Fn(&'static str, Value, Reply, &mut App)>;

/// The app's Computer history service (`electronBridge.chronicle`).
#[derive(Default)]
pub struct Service {
    pub dispatch: Option<Dispatch>,
}

impl Global for Service {}

/// Opens a new chat with `0` in the composer (`prefillPrompt`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = settings, no_json)]
pub struct PrefillPrompt(pub String);

/// The nav row: macOS with the `3488289778` Statsig gate on (`b_c`).
pub fn visible(evaluations: &Value, gates_loaded: bool) -> bool {
    cfg!(target_os = "macos")
        && gates_loaded
        && evaluations["feature_gates"]["3488289778"]["value"] == true
}

const LEARN_MORE: &str = "https://learn.chatgpt.com/docs/customization/computer-history";
const PLUGIN: &str = "[@Computer History](plugin://computer-history@openai-bundled)";
const INFO: &str = "ChatGPT records text summaries of your computer activity. You can control what ChatGPT can reference by deleting individual records or clearing recent history.";
const FOOTER: &str = "When turned on, ChatGPT saves text summaries of your activity across allowed apps and websites, potentially including communications. Audio and private-mode web browsing are never included, and you can pause or clear your history and manage what’s included at any time. This feature increases token usage.";

/// `Kt`: the off state's examples (user question, assistant answer).
const EXAMPLES: [(&str, &str); 4] = [
    (
        "What did I promise to send Sarah after yesterday’s meeting?",
        "You told Sarah in Slack you’d send the Q3 budget by Friday. The latest version is in Google Sheets, and your meeting notes in Google Docs list two numbers to confirm: hiring and travel.",
    ),
    (
        "What was I working on before lunch?",
        "You were revising the launch presentation in Google Slides, checking campaign numbers in Google Sheets, and reviewing feedback from Maya in Slack. You left off updating the rollout timeline.",
    ),
    (
        "What was that planning document I was looking at today?",
        "You were reviewing the Q4 launch plan in Google Docs this morning. Here’s the document: Q4 Launch Planning",
    ),
    (
        "Where was I on the home remodel?",
        "You narrowed the backsplash to two options in Chrome, compared quotes in Google Sheets, and saved the contractor’s revised estimate in Google Drive. The next decision is white subway tile or blue zellige.",
    ),
];

/// `Ask about your history` picks one of these at random.
const PROMPTS: [&str; 14] = [
    "Give me a list of tasks I’ve worked on today and their status",
    "Prepare a summary of what I did yesterday for standup",
    "Where can I find the proposal document I was looking for earlier today?",
    "Who contacted me today that I need to follow up with?",
    "Roast my computer behavior",
    "How often do I get distracted, and what usually pulls me off task?",
    "Give me feedback on how I operate day to day",
    "Analyse the writing samples you have from me and describe my writing style with examples and guidelines",
    "Create a reusable skill that writes like me",
    "What keyboard shortcuts do I use most? And which should I be using more?",
    "Where am I being ineffective, and what should I be using AI for instead?",
    "What is my most commonly used phrase?",
    "How do I split my time between focused work, switching tabs, and returning to unfinished tasks?",
    "What was I working on before my last break?",
];

const CONSENT: [&str; 3] = [
    "You can pause or clear your history and manage which apps are included at any time.",
    "Consider pausing or excluding sensitive apps, such as health or finance, from your history.",
    "You also agree that you'll pause Computer History during communications with others, such as in video conference or text messaging apps, unless you have their prior express consent.",
];

/// `Clear history` choices: scope and the confirmation title.
const CLEARS: [(&str, &str, &str); 4] = [
    (
        "last_ten_minutes",
        "Clear last 10 minutes",
        "Clear the last 10 minutes?",
    ),
    ("last_hour", "Clear last hour", "Clear the last hour?"),
    ("last_day", "Clear last day", "Clear the last day?"),
    ("all", "Clear all", "Clear all history?"),
];

pub fn render(page: Entity<SettingsPage>, window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("computer-history-settings", cx, |window, cx| {
            ComputerHistory::new(page, window, cx)
        })
        .into_any_element()
}

#[derive(Clone)]
enum Load<T> {
    Loading,
    Failed,
    Ready(T),
}

impl<T> Load<T> {
    fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(value) => Some(value),
            _ => None,
        }
    }
}

enum Modal {
    Consent,
    Permissions(Permissions),
}

/// The permissions dialog's draft of the recorder's `observation` settings.
struct Permissions {
    observation: Value,
    apps: Entity<Select>,
    site: Option<Entity<Composer>>,
    saving: bool,
    /// Customize apps from the consent dialog turns the feature on after a save.
    enable_after: bool,
}

pub struct ComputerHistory {
    page: Entity<SettingsPage>,
    service: Load<bool>,
    state: Load<Value>,
    memories: Load<Value>,
    settings: Load<Value>,
    history: Load<Vec<Value>>,
    applications: Vec<Value>,
    pending_enable: bool,
    busy: bool,
    collapsed: HashSet<String>,
    example: usize,
    carousel: Option<Task<()>>,
    modal: Option<Modal>,
    confirm: Option<Entity<Confirmation>>,
    deleting: Option<String>,
    chip_menu: Option<(String, String)>,
    clear_menu: Option<Entity<Select>>,
    layer: Layer,
    artwork: ArtworkCache,
}

fn call<T: 'static>(
    method: &'static str,
    params: Value,
    cx: &mut Context<T>,
    done: impl FnOnce(&mut T, anyhow::Result<Value>, &mut Context<T>) + 'static,
) {
    let entity = cx.weak_entity();
    let reply: Reply = Box::new(move |result, cx| {
        entity
            .update(cx, |this, cx| {
                done(this, result, cx);
                cx.notify();
            })
            .ok();
    });
    match cx.try_global::<Service>().and_then(|s| s.dispatch.clone()) {
        Some(dispatch) => dispatch(method, params, reply, cx),
        None => reply(Err(anyhow::anyhow!("Computer history isn’t available")), cx),
    }
}

impl ComputerHistory {
    fn new(page: Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let entity = cx.entity();
        window.defer(cx, move |_, cx| entity.update(cx, |this, cx| this.load(cx)));
        let mut this = Self {
            page,
            service: Load::Loading,
            state: Load::Loading,
            memories: Load::Loading,
            settings: Load::Loading,
            history: Load::Loading,
            applications: Vec::new(),
            pending_enable: false,
            busy: false,
            collapsed: HashSet::new(),
            example: 0,
            carousel: None,
            modal: None,
            confirm: None,
            deleting: None,
            chip_menu: None,
            clear_menu: None,
            layer: Layer::default(),
            artwork: ArtworkCache::default(),
        };
        this.schedule_example(cx);
        this
    }

    /// Advances the example every 14 s, paused under reduced motion.
    fn schedule_example(&mut self, cx: &mut Context<Self>) {
        if crate::motion::reduced(cx) {
            self.carousel = None;
            return;
        }
        self.carousel = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(14))
                .await;
            this.update(cx, |this, cx| this.step_example(1, cx)).ok();
        }));
    }

    fn step_example(&mut self, by: usize, cx: &mut Context<Self>) {
        self.example = (self.example + by) % EXAMPLES.len();
        self.schedule_example(cx);
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        call("readMemories", Value::Null, cx, |this, result, _| {
            this.memories = result.map_or(Load::Failed, Load::Ready);
        });
        call("availability", Value::Null, cx, |this, result, cx| {
            let Ok(availability) = result else {
                this.service = Load::Ready(false);
                return;
            };
            if availability["allowed"] == false {
                // A Computer history blocked by requirements is hidden: open General instead.
                this.page
                    .update(cx, |page, cx| page.select_section("General", cx));
                return;
            }
            let service = availability["service"] == true;
            this.service = Load::Ready(service);
            if service {
                this.refresh(cx);
            }
        });
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        call("getState", Value::Null, cx, |this, result, _| {
            this.state = result.map_or(Load::Failed, Load::Ready);
        });
        self.load_settings(cx);
        self.load_history(cx);
        call("listApplications", Value::Null, cx, |this, result, _| {
            if let Ok(Value::Array(apps)) = result {
                this.applications = apps;
            }
        });
    }

    fn load_settings(&mut self, cx: &mut Context<Self>) {
        call("getSettings", Value::Null, cx, |this, result, _| {
            this.settings = result.map_or(Load::Failed, Load::Ready);
        });
    }

    fn load_history(&mut self, cx: &mut Context<Self>) {
        self.history = Load::Loading;
        call("listHistory", Value::Null, cx, |this, result, _| {
            this.history = match result {
                Ok(Value::Array(items)) => Load::Ready(items),
                _ => Load::Failed,
            };
        });
    }

    fn toast(&self, toast: Toast, cx: &mut App) {
        if let Some(toaster) = self.page.read(cx).toaster.clone() {
            toaster.update(cx, |toaster, cx| toaster.push(toast, cx));
        }
    }

    fn enabled_now(&self) -> bool {
        self.pending_enable || self.state.ready().is_some_and(|s| s["enabled"] == true)
    }

    /// `L`: Memories is on with generation and use.
    fn memories_ready(&self) -> bool {
        self.memories.ready().is_some_and(|m| {
            m["available"] == true
                && m["featureEnabled"] == true
                && m["generate"] == true
                && m["use"] == true
        })
    }

    /// The `Turn on` tooltip (`service-unavailable`, configuration, Memories), in that order.
    fn disabled_reason(&self) -> Option<&'static str> {
        if !matches!(self.service, Load::Ready(true)) {
            return Some("Computer history isn’t available. Restart ChatGPT and try again.");
        }
        match &self.memories {
            Load::Loading => Some("Loading your settings…"),
            Load::Failed => {
                Some("Your settings couldn’t be loaded. Restart ChatGPT and try again.")
            }
            Load::Ready(m) if m["available"] != true => {
                Some("Computer history requires Memories, which isn’t available right now")
            }
            Load::Ready(_) => None,
        }
    }

    /// `enable()`: straight on when Memories is ready, else ask to turn Memories on first.
    fn enable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.memories_ready() {
            return self.set_enabled(true, cx);
        }
        self.confirm_memories(window, cx);
    }

    fn confirm_memories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Turn on Memories to use Computer history?",
                "Computer history requires Memories so ChatGPT can remember your activity across tasks. Turning this on also lets ChatGPT generate memories from your chats and use them in future chats.",
                "Turn on Memories",
                window,
                cx,
            )
            .settings_style()
            .with_documentation("Learn more", LEARN_MORE, cx)
        });
        cx.subscribe_in(&dialog, window, |this, _, outcome: &Outcome, _, cx| {
            this.confirm = None;
            if *outcome == Outcome::Confirm {
                this.turn_on_memories(cx);
            }
            cx.notify();
        })
        .detach();
        self.confirm = Some(dialog);
        cx.notify();
    }

    /// `Ue`, then `getChronicleState`: enable, retry activation, or nothing when already active.
    fn turn_on_memories(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        call("enableMemories", Value::Null, cx, |this, result, cx| {
            this.busy = false;
            match result {
                Ok(memories) => {
                    this.memories = Load::Ready(memories);
                    call("getState", Value::Null, cx, |this, result, cx| {
                        let Ok(state) = result else {
                            return this.toast(
                                Toast::danger("Computer history could not be turned on. Restart ChatGPT, and try again"),
                                cx,
                            );
                        };
                        let active =
                            matches!(state["recorderState"].as_str(), Some("running" | "paused"))
                                || state["activationState"] == "starting";
                        let enabled = state["enabled"] == true;
                        this.state = Load::Ready(state);
                        if !enabled {
                            this.set_enabled(true, cx);
                        } else if !active {
                            this.retry(cx);
                        }
                    });
                }
                Err(_) => this.toast(Toast::danger("Memories could not be turned on"), cx),
            }
        });
    }

    fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.busy = true;
        self.pending_enable = enabled;
        call(
            "setEnabled",
            json!({ "enabled": enabled }),
            cx,
            move |this, result, cx| {
                this.busy = false;
                this.pending_enable = false;
                match result {
                Ok(state) => {
                    if enabled && state["enabled"] == false {
                        this.toast(
                            Toast::danger("Computer history wasn’t turned on because required permissions weren’t granted"),
                            cx,
                        );
                    }
                    this.state = Load::Ready(state);
                    this.load_settings(cx);
                    this.load_history(cx);
                }
                Err(_) if enabled => this.toast(
                    Toast::danger("Computer history could not be turned on. Restart ChatGPT, and try again"),
                    cx,
                ),
                Err(_) => this.toast(Toast::danger("Computer history could not be turned off"), cx),
            }
            },
        );
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        call("retryActivation", Value::Null, cx, |this, result, cx| {
            this.busy = false;
            match result {
                Ok(state) => this.state = Load::Ready(state),
                Err(_) => this.toast(Toast::danger("Computer history could not be resumed"), cx),
            }
        });
    }

    fn pause_resume(&mut self, cx: &mut Context<Self>) {
        let paused = self
            .state
            .ready()
            .is_some_and(|s| s["recorderState"] == "paused");
        self.busy = true;
        call(
            if paused { "resume" } else { "pause" },
            Value::Null,
            cx,
            |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(state) => this.state = Load::Ready(state),
                    Err(_) => this.toast(
                        Toast::danger("Computer history could not be paused or resumed"),
                        cx,
                    ),
                }
            },
        );
    }

    fn ask(&self, window: &mut Window, cx: &mut App) {
        let index = chrono::Utc::now().timestamp_subsec_nanos() as usize % PROMPTS.len();
        window.dispatch_action(
            Box::new(PrefillPrompt(format!("{PLUGIN} {}", PROMPTS[index]))),
            cx,
        );
    }

    // ── Clear and delete ─────────────────────────────────────────────────────────────────

    fn confirm_clear(
        &mut self,
        title: &'static str,
        params: Value,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| {
            Confirmation::new(
                title,
                "Your actions and any memories created from them will be deleted. This cannot be undone.",
                if delete { "Delete" } else { "Clear" },
                window,
                cx,
            )
            .settings_style()
            .destructive()
        });
        cx.subscribe_in(&dialog, window, move |this, _, outcome: &Outcome, _, cx| {
            this.confirm = None;
            this.deleting = None;
            if *outcome == Outcome::Confirm {
                call("clearHistory", params.clone(), cx, |this, result, cx| {
                    match result {
                        Ok(_) => this.toast(
                            Toast::success(
                                "History cleared — ChatGPT memory may take up to 6 hours to update",
                            ),
                            cx,
                        ),
                        Err(_) => {
                            this.toast(Toast::danger("Computer history could not be cleared"), cx)
                        }
                    }
                    this.load_history(cx);
                });
            }
            cx.notify();
        })
        .detach();
        self.confirm = Some(dialog);
        cx.notify();
    }

    // ── Permissions ──────────────────────────────────────────────────────────────────────

    fn open_permissions(
        &mut self,
        enable_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.modal = None;
        self.load_settings(cx);
        let observation = self
            .settings
            .ready()
            .map(|s| s["observation"].clone())
            .unwrap_or_else(|| {
                json!({
                    "defaultApplicationBehavior": "observe", "defaultURLBehavior": "observe",
                    "allowlist": [], "blocklist": []
                })
            });
        let apps = cx
            .new(|cx| Select::new("Add app", Vec::new(), Some("Search apps"), cx).with_width(240.));
        cx.subscribe(&apps, |this, _, selected: &Selected, cx| {
            let id = selected.0.to_string();
            this.edit_list("app", &id, true, cx);
        })
        .detach();
        let _ = window;
        self.modal = Some(Modal::Permissions(Permissions {
            observation,
            apps,
            site: None,
            saving: false,
            enable_after,
        }));
        cx.notify();
    }

    /// The draft's list for `scope`: the blocklist while the default is to observe, else the
    /// allowlist.
    fn list_key(observation: &Value, scope: &str) -> &'static str {
        let key = if scope == "app" {
            "defaultApplicationBehavior"
        } else {
            "defaultURLBehavior"
        };
        if observation[key] == "do_not_observe" {
            "allowlist"
        } else {
            "blocklist"
        }
    }

    fn edit_list(&mut self, scope: &str, value: &str, add: bool, cx: &mut Context<Self>) {
        let Some(Modal::Permissions(permissions)) = &mut self.modal else {
            return;
        };
        let field = if scope == "app" {
            "bundleID"
        } else {
            "urlDomain"
        };
        let key = Self::list_key(&permissions.observation, scope);
        let list = permissions.observation[key]
            .as_array_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let mut list: Vec<Value> = list
            .into_iter()
            .filter(|entry| !(entry["scope"] == scope && entry[field] == value))
            .collect();
        if add {
            let mut entry = json!({ "scope": scope, "bundleID": null, "urlDomain": null });
            entry[field] = value.into();
            list.push(entry);
        }
        permissions.observation[key] = list.into();
        cx.notify();
    }

    /// The policy menu moves the listed entries between the allowlist and the blocklist.
    fn set_policy(&mut self, scope: &str, only: bool, cx: &mut Context<Self>) {
        let Some(Modal::Permissions(permissions)) = &mut self.modal else {
            return;
        };
        let observation = &mut permissions.observation;
        let from = Self::list_key(observation, scope);
        let key = if scope == "app" {
            "defaultApplicationBehavior"
        } else {
            "defaultURLBehavior"
        };
        observation[key] = if only { "do_not_observe" } else { "observe" }.into();
        let to = Self::list_key(observation, scope);
        if from != to {
            let moved: Vec<Value> = observation[from]
                .as_array_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            let (moving, staying): (Vec<_>, Vec<_>) =
                moved.into_iter().partition(|entry| entry["scope"] == scope);
            observation[from] = staying.into();
            if let Some(target) = observation[to].as_array_mut() {
                target.extend(moving);
            }
        }
        cx.notify();
    }

    fn save_permissions(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::Permissions(permissions)) = &mut self.modal else {
            return;
        };
        permissions.saving = true;
        let settings = json!({ "observation": permissions.observation });
        call(
            "updateSettings",
            json!({ "settings": settings }),
            cx,
            |this, result, cx| {
                let enable = match &mut this.modal {
                    Some(Modal::Permissions(p)) => {
                        p.saving = false;
                        p.enable_after
                    }
                    _ => false,
                };
                match result {
                    Ok(settings) => {
                        this.settings = Load::Ready(settings);
                        this.modal = None;
                        if enable {
                            this.pending_enable = true;
                            if let Some(window) = cx.active_window() {
                                let entity = cx.entity();
                                cx.defer(move |cx| {
                                    window
                                        .update(cx, |_, window, cx| {
                                            entity.update(cx, |this, cx| this.enable(window, cx))
                                        })
                                        .ok();
                                });
                            }
                        }
                    }
                    Err(_) => this.toast(
                        Toast::danger("Computer history permissions could not be saved"),
                        cx,
                    ),
                }
            },
        );
    }

    /// The app chip's menu: exclude the app from, or include it in, future history.
    fn toggle_app(&mut self, bundle: String, cx: &mut Context<Self>) {
        self.chip_menu = None;
        let Some(settings) = self.settings.ready() else {
            return;
        };
        let mut observation = settings["observation"].clone();
        let observed = observed(&observation, &bundle);
        let key = Self::list_key(&observation, "app");
        let mut list: Vec<Value> = observation[key]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| !(e["scope"] == "app" && e["bundleID"] == bundle.as_str()))
            .collect();
        // Observing by default, the blocklist excludes; otherwise the allowlist includes.
        let listed = (key == "blocklist") == observed;
        if listed {
            list.push(json!({ "scope": "app", "bundleID": bundle, "urlDomain": null }));
        }
        observation[key] = list.into();
        call(
            "updateSettings",
            json!({ "settings": { "observation": observation } }),
            cx,
            |this, result, cx| match result {
                Ok(settings) => this.settings = Load::Ready(settings),
                Err(_) => this.toast(
                    Toast::danger("The app's Computer history permission could not be updated"),
                    cx,
                ),
            },
        );
    }

    fn app_name(&self, bundle: &str) -> String {
        self.applications
            .iter()
            .find(|app| app["bundleIdentifier"] == bundle)
            .and_then(|app| app["displayName"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| bundle.rsplit('.').next().unwrap_or(bundle).to_owned())
    }
}

/// `lt`: whether the recorder observes the app.
fn observed(observation: &Value, bundle: &str) -> bool {
    let listed = |key: &str| {
        observation[key]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| e["scope"] == "app" && e["bundleID"] == bundle)
    };
    if observation["defaultApplicationBehavior"] == "observe" {
        !listed("blocklist")
    } else {
        listed("allowlist")
    }
}

/// `Today`, `Yesterday`, or the weekday and date of `ms` in the local zone.
fn day_label(ms: i64, now: chrono::DateTime<chrono::Local>) -> String {
    let Some(day) = chrono::DateTime::from_timestamp_millis(ms) else {
        return String::new();
    };
    let day = day.with_timezone(&chrono::Local).date_naive();
    let today = now.date_naive();
    if day == today {
        "Today".into()
    } else if today.pred_opt() == Some(day) {
        "Yesterday".into()
    } else {
        day.format("%A, %B %-d").to_string()
    }
}

/// `h:mm` for ten-minute items; the part of the day for six-hour ones.
fn time_label(ms: i64, six_hour: bool) -> String {
    let Some(time) = chrono::DateTime::from_timestamp_millis(ms) else {
        return String::new();
    };
    let time = time.with_timezone(&chrono::Local);
    if !six_hour {
        return time.format("%-I:%M").to_string();
    }
    use chrono::Timelike as _;
    match time.hour() {
        4..10 => "Morning",
        10..16 => "Afternoon",
        16..22 => "Evening",
        _ => "Overnight",
    }
    .into()
}

fn description(s: impl Into<SharedString>, t: &Theme) -> AnyElement {
    text(s, 13., 18., 430., t.text_secondary)
        .whitespace_normal()
        .into_any_element()
}

fn learn_more(id: &'static str, t: &Theme) -> AnyElement {
    div()
        .id(id)
        .role(gpui::Role::Link)
        .cursor_pointer()
        .child(text("Learn more", 13., 18., 430., t.text_link))
        .on_click(|_, _, cx| cx.open_url(LEARN_MORE))
        .into_any_element()
}

// ── Rendering ────────────────────────────────────────────────────────────────────────────────

impl ComputerHistory {
    /// `Vt`: what the feature does, `Turn on`, an example and the privacy footer.
    fn off_state(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let reason = self.disabled_reason();
        let disabled = reason.is_some() || self.busy;
        let turn_on = div()
            .id("computer-history-turn-on")
            .debug_selector(|| "computer-history-turn-on".into())
            .role(gpui::Role::Button)
            .mt(px(16.))
            .h(px(30.))
            .px(px(12.))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(9999.))
            .bg(t.button_primary_solid)
            .child(text("Turn on", 14., 18., 500., t.text_inverse))
            .when(disabled, |b| b.opacity(0.5))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !disabled {
                    this.modal = Some(Modal::Consent);
                    cx.notify();
                }
            }));
        let turn_on = match reason {
            Some(reason) => tooltip(turn_on, reason, None, Side::Bottom).into_any_element(),
            None => turn_on.into_any_element(),
        };
        let (question, answer) = EXAMPLES[self.example];
        let nav = |id: &'static str, glyph: &'static str, by: usize, cx: &mut Context<Self>| {
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(if by == 1 {
                    "Next example"
                } else {
                    "Previous example"
                })
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9999.))
                .bg(rgba(0x0000_0033))
                .child(icon(glyph, 16., gpui::white()))
                .on_click(cx.listener(move |this, _, _, cx| this.step_example(by, cx)))
        };
        let dots = (0..EXAMPLES.len()).map(|i| {
            div()
                .size(px(6.))
                .rounded(px(9999.))
                .bg(gpui::white())
                .opacity(if i == self.example { 0.9 } else { 0.35 })
        });
        let example = div()
            .id("computer-history-example")
            .relative()
            .min_h(px(340.))
            .flex()
            .flex_col()
            .justify_center()
            .px(px(36.))
            .py(px(30.))
            .child(div().absolute().inset_0().child(gradient_artwork(
                self.artwork.clone(),
                px(16.),
                shapes::Corner::Squircle,
            )))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div().flex().justify_end().child(
                            div()
                                .max_w(px(260.))
                                .px(px(12.))
                                .py(px(8.))
                                .rounded(px(16.))
                                .bg(rgba(0xffff_ffe6))
                                .child(
                                    text(question, 13., 18., 430., gpui::black())
                                        .whitespace_normal(),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(10.))
                            .rounded(px(16.))
                            .bg(rgba(0x1a1a_1ad9))
                            .child(text(answer, 13., 18., 430., gpui::white()).whitespace_normal()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(12.))
                    .left_0()
                    .right_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .child(nav(
                        "computer-history-previous",
                        "chevron-left",
                        EXAMPLES.len() - 1,
                        cx,
                    ))
                    .child(
                        div()
                            .id("computer-history-example-status")
                            .role(gpui::Role::Status)
                            .aria_label(format!(
                                "Example {} of {}",
                                self.example + 1,
                                EXAMPLES.len()
                            ))
                            .flex()
                            .gap(px(6.))
                            .children(dots),
                    )
                    .child(nav("computer-history-next", "chevron-right", 1, cx)),
            );
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(px(24.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(text("Let ChatGPT follow your work", 16., 24., 500., t.text))
                            .child(
                                div()
                                    .mt(px(5.))
                                    .max_w(px(300.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .child(description("ChatGPT can summarize your activity across the apps and websites you use without ever recording your screen or audio.", t))
                                    .child(description("Ask about what you were working on, get help without explaining everything again, or discover opportunities to automate repeated tasks.", t)),
                            )
                            .child(div().flex().child(turn_on)),
                    )
                    .child(div().flex_1().min_w_0().child(example)),
            )
            .child(
                div()
                    .mt(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(description(FOOTER, t))
                    .child(learn_more("computer-history-footer-learn-more", t)),
            )
            .into_any_element()
    }

    /// `Dn`: the feature card and the History card.
    fn on_state(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = self
            .state
            .ready()
            .cloned()
            .unwrap_or_else(|| json!({ "enabled": true }));
        let recorder = state["recorderState"].as_str().unwrap_or("stopped");
        let activation = state["activationState"].as_str().unwrap_or("idle");
        let unavailable = matches!(self.state, Load::Failed) || activation == "failed";
        let waiting = activation == "waiting_for_permissions"
            || (recorder == "stopped" && activation == "idle");
        let activating = activation == "starting";
        let busy = self.busy;
        let mut controls = div().flex().items_center().gap(px(12.));
        let button = if activating || waiting || unavailable {
            let label = if activating {
                "Resume"
            } else if waiting {
                "Finish setup"
            } else {
                "Try again"
            };
            Some((label, true))
        } else if matches!(recorder, "running" | "paused") {
            Some((
                if recorder == "paused" {
                    "Resume"
                } else {
                    "Pause"
                },
                false,
            ))
        } else {
            None
        };
        if let Some((label, retry)) = button {
            controls = controls.child(
                action(
                    "computer-history-action",
                    label,
                    Kind::Secondary,
                    busy || activating,
                    t,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.busy {
                        return;
                    }
                    if retry {
                        this.retry(cx);
                    } else {
                        this.pause_resume(cx);
                    }
                })),
            );
        }
        controls = controls.child(
            switch("computer-history-toggle", true, busy, t)
                .debug_selector(|| "computer-history-toggle".into())
                .aria_label("Turn off Computer history")
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.busy {
                        this.set_enabled(false, cx);
                    }
                })),
        );
        let feature = if unavailable {
            "Computer history is temporarily unavailable."
        } else {
            "Augment memories with your computer activity so ChatGPT can help with anything you’re working on."
        };
        let detail = div()
            .flex()
            .flex_col()
            .items_start()
            .child(description(feature, t))
            .child(learn_more("computer-history-learn-more", t))
            .into_any_element();
        let indicator = self
            .page
            .read(cx)
            .get("host.chronicleMenuBarIndicatorEnabled")
            .as_bool()
            .unwrap_or(true);
        let page = self.page.clone();
        let indicator_switch = switch("computer-history-indicator", indicator, busy, t)
            .on_click(move |_, _, cx| {
                page.update(cx, |page, cx| {
                    page.set_host_batch(
                        vec![("host.chronicleMenuBarIndicatorEnabled", (!indicator).into())],
                        cx,
                    )
                });
            })
            .into_any_element();
        let count = self.settings.ready().map_or(0, |s| {
            ["allowlist", "blocklist"]
                .iter()
                .flat_map(|key| {
                    s["observation"][key]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                })
                .count()
        });
        let choose = action(
            "computer-history-permissions",
            if count == 0 {
                "Choose".to_owned()
            } else {
                format!("{count} selected")
            },
            Kind::Secondary,
            !matches!(self.service, Load::Ready(true)),
            t,
        )
        .on_click(cx.listener(|this, _, window, cx| this.open_permissions(false, window, cx)))
        .into_any_element();
        let feature_card = card(
            vec![
                settings_row_content(
                    "Computer history",
                    Some(detail),
                    controls.into_any_element(),
                    t,
                ),
                settings_row_content("Show indicator in menu bar", None, indicator_switch, t),
                settings_row_content(
                    "Permissions",
                    Some(description(
                        "Choose which apps and websites to exclude or include in computer history",
                        t,
                    )),
                    choose,
                    t,
                ),
            ],
            Some(&self.layer),
            t,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(feature_card)
            .child(self.history_card(t, window, cx))
            .into_any_element()
    }

    fn history_card(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let info = tooltip(
            div()
                .id("computer-history-info")
                .role(gpui::Role::Button)
                .aria_label("About computer history")
                .child(icon("info", 14., t.text_tertiary)),
            INFO,
            None,
            Side::Top,
        );
        let mut actions = Vec::new();
        let items = self.history.ready().cloned().unwrap_or_default();
        if !items.is_empty() {
            let clear = self
                .clear_menu
                .get_or_insert_with(|| {
                    let choices = CLEARS
                        .iter()
                        .map(|(scope, label, _)| Choice {
                            value: SharedString::new_static(scope),
                            label: SharedString::new_static(label),
                            checked: false,
                            disabled: false,
                        })
                        .collect();
                    let select = cx.new(|cx| Select::new("Clear history", choices, None, cx));
                    cx.subscribe_in(
                        &select,
                        window,
                        |this, _, selected: &Selected, window, cx| {
                            if let Some((scope, _, title)) =
                                CLEARS.iter().find(|c| c.0 == selected.0.as_ref())
                            {
                                this.confirm_clear(
                                    title,
                                    json!({ "scope": scope }),
                                    false,
                                    window,
                                    cx,
                                );
                            }
                        },
                    )
                    .detach();
                    select
                })
                .clone();
            clear.update(cx, |select, cx| select.set_value("Clear history", cx));
            actions.push(div().child(clear).into_any_element());
            actions.push(
                action(
                    "computer-history-ask",
                    "Ask about your history",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(cx.listener(|this, _, window, cx| this.ask(window, cx)))
                .into_any_element(),
            );
        }
        let body: AnyElement = match &self.history {
            Load::Loading => description("Loading history…", t),
            Load::Failed => div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(16.))
                .py(px(12.))
                .child(description("Computer history could not be loaded", t))
                .child(
                    action(
                        "computer-history-retry",
                        "Try again",
                        Kind::Secondary,
                        false,
                        t,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_history(cx))),
                )
                .into_any_element(),
            Load::Ready(items) if items.is_empty() => div()
                .px(px(16.))
                .py(px(12.))
                .child(description("No computer history yet", t))
                .into_any_element(),
            Load::Ready(_) => self.timeline(&items, t, cx),
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(text("History", 14., 21., 500., t.text))
            .child(info);
        div()
            .id("history")
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(title)
                    .child(div().flex().items_center().gap(px(8.)).children(actions)),
            )
            .child(body)
            .into_any_element()
    }

    /// `_n`: items grouped by local day, each group collapsible.
    fn timeline(&mut self, items: &[Value], t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = chrono::Local::now();
        let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
        for item in items {
            let label = day_label(
                item["interval"]["startMs"].as_i64().unwrap_or_default(),
                now,
            );
            match groups.last_mut() {
                Some((last, group)) if *last == label => group.push(item),
                _ => groups.push((label, vec![item])),
            }
        }
        let mut column = div().flex().flex_col().gap(px(16.));
        for (label, group) in groups {
            let open = !self.collapsed.contains(&label);
            let key = label.clone();
            let header = div()
                .id(SharedString::from(format!("computer-history-day-{label}")))
                .role(gpui::Role::Button)
                .aria_expanded(open)
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .child(text(label.clone(), 13., 18., 500., t.text_secondary))
                .child(icon(
                    if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    14.,
                    t.text_tertiary,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.collapsed.remove(&key) {
                        this.collapsed.insert(key.clone());
                    }
                    cx.notify();
                }));
            let mut rows = div().flex().flex_col();
            if open {
                for item in group {
                    rows = rows.child(self.item(item, t, cx));
                }
            }
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(header)
                    .child(rows),
            );
        }
        column.into_any_element()
    }

    /// `ln`: time, rail, title, description, app chips, Reveal/Delete and a suggestion.
    fn item(&mut self, item: &Value, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = item["id"].as_str().unwrap_or_default().to_owned();
        let title = item["title"].as_str().unwrap_or_default().to_owned();
        let six_hour = item["summaryLevel"] == "six_hour";
        let start = item["interval"]["startMs"].as_i64().unwrap_or_default();
        let end = item["interval"]["endMs"].as_i64().unwrap_or_default();
        let time = text(time_label(start, six_hour), 12., 16., 430., t.text_tertiary);
        let time = if six_hour {
            tooltip(
                div()
                    .id(SharedString::from(format!("computer-history-time-{id}")))
                    .child(time),
                format!("{} – {}", time_label(start, false), time_label(end, false)),
                None,
                Side::Top,
            )
            .into_any_element()
        } else {
            time.into_any_element()
        };
        let path = item["memoryFilePath"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let reveal = tooltip(
            div()
                .id(SharedString::from(format!("computer-history-reveal-{id}")))
                .role(gpui::Role::Button)
                .aria_label(format!("Reveal {title} in Finder"))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .hover(|d| d.bg(t.ghost_hover))
                .child(icon("folder", 14., t.text_secondary))
                .on_click(cx.listener(move |_, _, _, cx| {
                    call("reveal", json!({ "path": path }), cx, |_, _, _| {});
                })),
            "Reveal in Finder",
            None,
            Side::Top,
        );
        let confirming = self.deleting.as_deref() == Some(id.as_str());
        let delete_id = id.clone();
        let delete = div()
            .id(SharedString::from(format!("computer-history-delete-{id}")))
            .debug_selector({
                let id = id.clone();
                move || format!("computer-history-delete-{id}")
            })
            .role(gpui::Role::Button)
            .aria_label(if confirming {
                format!("Confirm delete {title}")
            } else {
                format!("Delete {title}")
            })
            .h(px(22.))
            .px(px(if confirming { 8. } else { 4. }))
            .flex()
            .items_center()
            .rounded(px(6.))
            .when(confirming, |d| d.bg(t.text_error))
            .hover(|d| d.bg(t.ghost_hover))
            .child(if confirming {
                text("Delete", 12., 16., 500., t.text_inverse).into_any_element()
            } else {
                icon("trash", 14., t.text_secondary).into_any_element()
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.deleting.as_deref() == Some(delete_id.as_str()) {
                    let params = json!({
                        "scope": "interval",
                        "interval": { "startMs": start, "endMs": end },
                    });
                    this.confirm_clear("Delete this history item?", params, true, window, cx);
                } else {
                    this.deleting = Some(delete_id.clone());
                    cx.notify();
                }
            }));
        let chips = item["applications"].as_array().cloned().unwrap_or_default();
        let observation = self
            .settings
            .ready()
            .map(|s| s["observation"].clone())
            .unwrap_or(Value::Null);
        let chips = chips
            .into_iter()
            .filter_map(|bundle| bundle.as_str().map(str::to_owned))
            .map(|bundle| {
                let name = self.app_name(&bundle);
                let excluded = !observation.is_null() && !observed(&observation, &bundle);
                let menu_open = self.chip_menu.as_ref() == Some(&(id.clone(), bundle.clone()));
                let key = (id.clone(), bundle.clone());
                let toggle_bundle = bundle.clone();
                div()
                    .relative()
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "computer-history-app-{id}-{bundle}"
                            )))
                            .role(gpui::Role::Button)
                            .aria_label(format!("Open {name} menu"))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(t.button_secondary)
                            .when(excluded, |d| d.opacity(0.5))
                            .child(text(
                                name.chars().next().unwrap_or('?').to_string(),
                                11.,
                                14.,
                                600.,
                                t.text_secondary,
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.chip_menu = if this.chip_menu.as_ref() == Some(&key) {
                                    None
                                } else {
                                    Some(key.clone())
                                };
                                cx.notify();
                            })),
                    )
                    .when(menu_open, |d| {
                        d.child(
                            div()
                                .id("computer-history-app-menu")
                                .absolute()
                                .top(px(24.))
                                .left_0()
                                .w(px(260.))
                                .p(px(4.))
                                .rounded(px(10.))
                                .bg(t.menu_background)
                                .border_1()
                                .border_color(t.border)
                                .occlude()
                                .child(
                                    div()
                                        .id("computer-history-app-menu-item")
                                        .px(px(8.))
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .hover(|d| d.bg(t.ghost_hover))
                                        .child(text(
                                            if excluded {
                                                format!("Include {name} in future history")
                                            } else {
                                                format!("Exclude {name} from future history")
                                            },
                                            13.,
                                            18.,
                                            430.,
                                            t.text,
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_app(toggle_bundle.clone(), cx)
                                        })),
                                ),
                        )
                    })
            });
        let suggestion = item["suggestion"].is_object().then(|| {
            let s = &item["suggestion"];
            let skill = s["type"] == "skill";
            let name = s["name"].as_str().unwrap_or_default().to_owned();
            let request = s["description"].as_str().unwrap_or_default().to_owned();
            let memory = item["memoryFilePath"].as_str().unwrap_or_default().to_owned();
            let prompt = if skill {
                format!("{PLUGIN} Create a new \"{name}\" skill based on the description below. Consult the memory at {memory} and the events.jsonl files cited there to understand the recorded workflow and my actions.\n\n\"{request}\"")
            } else {
                format!("{PLUGIN} Create a new \"{name}\" automation based on the description below. Consult the memory at {memory} and the events.jsonl files cited there to understand the recorded workflow and my actions.\n\n\"{request}\"\n\nUse my local timezone and a new chat for scheduled runs.")
            };
            div()
                .mt(px(8.))
                .p(px(12.))
                .rounded(px(12.))
                .border_1()
                .border_color(t.border)
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(text(if skill { "Suggested skill" } else { "Suggested automation" }, 12., 16., 500., t.text_secondary))
                .child(description(request.clone(), t))
                .child(
                    div()
                        .id(SharedString::from(format!("computer-history-suggestion-{}", item["id"].as_str().unwrap_or_default())))
                        .role(gpui::Role::Link)
                        .cursor_pointer()
                        .child(text(
                            if skill { format!("Create {name} skill") } else { format!("Create {name} automation") },
                            13.,
                            18.,
                            500.,
                            t.text_link,
                        ))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(PrefillPrompt(prompt.clone())), cx)
                        }),
                )
        });
        div()
            .id(SharedString::from(format!("computer-history-item-{id}")))
            .group("computer-history-item")
            .flex()
            .gap(px(12.))
            .py(px(8.))
            .child(div().w(px(64.)).flex_none().pt(px(2.)).child(time))
            .child(
                div()
                    .w(px(9.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .mt(px(6.))
                            .size(px(7.))
                            .rounded(px(9999.))
                            .bg(t.text_tertiary),
                    )
                    .child(div().flex_1().w(px(1.)).bg(t.border)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .child(text(title.clone(), 14., 20., 500., t.text).truncate())
                            .child(
                                div()
                                    .flex()
                                    .gap(px(2.))
                                    .when(!confirming, |d| {
                                        d.invisible()
                                            .group_hover("computer-history-item", |d| d.visible())
                                    })
                                    .child(reveal)
                                    .child(delete),
                            ),
                    )
                    .child(description(
                        item["description"].as_str().unwrap_or_default().to_owned(),
                        t,
                    ))
                    .child(div().mt(px(4.)).flex().gap(px(4.)).children(chips))
                    .children(suggestion),
            )
            .into_any_element()
    }

    fn consent(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = CONSENT.iter().map(|line| {
            div()
                .flex()
                .gap(px(8.))
                .child(text("•", 14., 21., 430., t.text_secondary))
                .child(
                    text(*line, 14., 21., 430., t.text_secondary)
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal(),
                )
        });
        let buttons = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .child(
                action(
                    "computer-history-consent-cancel",
                    "Cancel",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.modal = None;
                    cx.notify();
                })),
            )
            .child(
                action(
                    "computer-history-allow-all",
                    "Allow all apps",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.modal = None;
                    this.enable(window, cx);
                })),
            )
            .child(
                action(
                    "computer-history-customize",
                    "Customize apps",
                    Kind::Primary,
                    false,
                    t,
                )
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_permissions(true, window, cx)),
                ),
            );
        modal(
            "computer-history-consent",
            div()
                .w(px(420.))
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(text("You're in control.", 18., 24., 600., t.text))
                .child(div().flex().flex_col().gap(px(8.)).children(list))
                .child(buttons),
            t,
            cx,
        )
    }

    fn permissions(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(Modal::Permissions(permissions)) = &self.modal else {
            return div().into_any_element();
        };
        let observation = permissions.observation.clone();
        let saving = permissions.saving;
        let app_select = permissions.apps.clone();
        let site = permissions.site.clone();
        let listed = |scope: &str| -> Vec<String> {
            let field = if scope == "app" {
                "bundleID"
            } else {
                "urlDomain"
            };
            observation[Self::list_key(&observation, scope)]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["scope"] == scope)
                .filter_map(|e| e[field].as_str().map(str::to_owned))
                .collect()
        };
        let apps = listed("app");
        let sites = listed("url");
        let choices = self
            .applications
            .iter()
            .filter_map(|app| {
                let id = app["bundleIdentifier"].as_str()?;
                (!apps.iter().any(|a| a == id)).then(|| Choice {
                    value: SharedString::from(id.to_owned()),
                    label: SharedString::from(app["displayName"].as_str().unwrap_or(id).to_owned()),
                    checked: false,
                    disabled: false,
                })
            })
            .collect();
        app_select.update(cx, |select, cx| {
            select.set_choices(choices, cx);
            select.set_value("Add app", cx);
        });
        let policy = |scope: &'static str, cx: &mut Context<Self>| {
            let key = if scope == "app" {
                "defaultApplicationBehavior"
            } else {
                "defaultURLBehavior"
            };
            let only = observation[key] == "do_not_observe";
            let noun = if scope == "app" { "apps" } else { "websites" };
            let labels = [
                format!("Exclude these {noun}"),
                format!("Include only these {noun}"),
            ];
            div()
                .flex()
                .gap(px(4.))
                .children([false, true].into_iter().map(|choice| {
                    let selected = choice == only;
                    div()
                        .id(SharedString::from(format!(
                            "computer-history-policy-{scope}-{choice}"
                        )))
                        .role(gpui::Role::RadioButton)
                        .aria_selected(selected)
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .when(selected, |d| d.bg(t.button_secondary))
                        .child(text(
                            labels[choice as usize].clone(),
                            13.,
                            18.,
                            500.,
                            if selected { t.text } else { t.text_secondary },
                        ))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_policy(scope, choice, cx)),
                        )
                }))
        };
        let row = |scope: &'static str, value: String, label: String, cx: &mut Context<Self>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(12.))
                .py(px(8.))
                .child(text(label, 13., 18., 430., t.text))
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "computer-history-remove-{scope}-{value}"
                        )))
                        .role(gpui::Role::Button)
                        .aria_label("Remove")
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .hover(|d| d.bg(t.ghost_hover))
                        .child(icon("close", 12., t.text_secondary))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit_list(scope, &value, false, cx)
                        })),
                )
        };
        let app_rows: Vec<_> = apps
            .iter()
            .map(|id| row("app", id.clone(), self.app_name(id), cx))
            .collect();
        let site_rows: Vec<_> = sites
            .iter()
            .map(|d| row("url", d.clone(), d.clone(), cx))
            .collect();
        let add_site = match site {
            Some(field) => div()
                .flex()
                .gap(px(8.))
                .child(div().flex_1().child(field.clone()))
                .child(
                    action(
                        "computer-history-site-save",
                        "Save",
                        Kind::Primary,
                        false,
                        t,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let domain = field
                            .read(cx)
                            .field_text()
                            .trim()
                            .trim_start_matches("https://")
                            .trim_start_matches("http://")
                            .trim_end_matches('/')
                            .to_lowercase();
                        if !domain.is_empty() {
                            this.edit_list("url", &domain, true, cx);
                        }
                        if let Some(Modal::Permissions(p)) = &mut this.modal {
                            p.site = None;
                        }
                        cx.notify();
                    })),
                )
                .into_any_element(),
            None => action(
                "computer-history-add-site",
                "Add website",
                Kind::Secondary,
                false,
                t,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                let field = cx.new(|cx| Composer::settings_compact_field("example.com", "", cx));
                window.focus(&field.focus_handle(cx), cx);
                if let Some(Modal::Permissions(p)) = &mut this.modal {
                    p.site = Some(field);
                }
                cx.notify();
            }))
            .into_any_element(),
        };
        let column =
            |title: &'static str, policy: gpui::Div, rows: Vec<gpui::Div>, add: AnyElement| {
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text(title, 14., 21., 500., t.text))
                    .child(policy)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(t.border)
                            .children(rows),
                    )
                    .child(div().flex().child(add))
            };
        let body: AnyElement = match &self.settings {
            Load::Loading => description("Loading permissions…", t),
            Load::Failed => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(description("Computer history permissions could not be loaded. If the issue persists, please restart ChatGPT.", t))
                .child(
                    div().flex().child(
                        action("computer-history-permissions-retry", "Try again", Kind::Secondary, false, t)
                            .on_click(cx.listener(|this, _, _, cx| this.load_settings(cx))),
                    ),
                )
                .into_any_element(),
            Load::Ready(_) => div()
                .flex()
                .gap(px(24.))
                .child(column("Apps", policy("app", cx), app_rows, div().child(app_select).into_any_element()))
                .child(column("Websites", policy("url", cx), site_rows, add_site))
                .into_any_element(),
        };
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(description(
                "Private-mode web browsing activity is never included in computer history.",
                t,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        action(
                            "computer-history-permissions-cancel",
                            "Cancel",
                            Kind::Secondary,
                            saving,
                            t,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.modal = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        action(
                            "computer-history-permissions-save",
                            "Save",
                            Kind::Primary,
                            saving || self.settings.ready().is_none(),
                            t,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !matches!(&this.modal, Some(Modal::Permissions(p)) if p.saving) {
                                this.save_permissions(cx);
                            }
                        })),
                    ),
            );
        modal(
            "computer-history-permissions-dialog",
            div()
                .w(px(640.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(body)
                .child(footer),
            t,
            cx,
        )
    }
}

/// A centered settings dialog over a dimmed window; a click outside closes it.
fn modal(
    id: &'static str,
    content: gpui::Div,
    t: &Theme,
    cx: &mut Context<ComputerHistory>,
) -> AnyElement {
    div()
        .id("computer-history-scrim")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x0000_0022))
        .on_click(cx.listener(|this, _, _, cx| {
            if !matches!(&this.modal, Some(Modal::Permissions(p)) if p.saving) {
                this.modal = None;
                cx.notify();
            }
        }))
        .child(
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Dialog)
                .p(px(20.))
                .rounded(px(20.))
                .bg(t.menu_background)
                .border_1()
                .border_color(t.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
        .into_any_element()
}

impl Render for ComputerHistory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        // `showOn`: on when enabled and Memories is ready, or Memories cannot be checked.
        let memories_checkable = self
            .memories
            .ready()
            .is_some_and(|m| m["available"] == true);
        let show_on = self.enabled_now() && (self.memories_ready() || !memories_checkable);
        let content = if show_on {
            self.on_state(&t, window, cx)
        } else {
            self.off_state(&t, cx)
        };
        let overlay = match &self.modal {
            Some(Modal::Consent) => Some(self.consent(&t, cx)),
            Some(Modal::Permissions(_)) => Some(self.permissions(&t, cx)),
            None => None,
        };
        div()
            .size_full()
            .relative()
            .child(frame(
                "computer-history-page",
                "Computer history",
                Vec::new(),
                content,
                self.confirm.as_ref(),
                &self.layer,
                window,
                &t,
            ))
            .children(overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_policy_follows_the_default_behavior() {
        let blocked = json!({
            "defaultApplicationBehavior": "observe",
            "blocklist": [{ "scope": "app", "bundleID": "com.a" }], "allowlist": []
        });
        assert!(!observed(&blocked, "com.a"));
        assert!(observed(&blocked, "com.b"));
        let only = json!({
            "defaultApplicationBehavior": "do_not_observe",
            "allowlist": [{ "scope": "app", "bundleID": "com.a" }], "blocklist": []
        });
        assert!(observed(&only, "com.a"));
        assert!(!observed(&only, "com.b"));
        assert_eq!(ComputerHistory::list_key(&only, "app"), "allowlist");
        assert_eq!(ComputerHistory::list_key(&only, "url"), "blocklist");
    }

    #[test]
    fn six_hour_items_name_the_part_of_the_day() {
        use chrono::TimeZone as _;
        let at = |hour| {
            chrono::Local
                .with_ymd_and_hms(2026, 11, 4, hour, 0, 0)
                .unwrap()
                .timestamp_millis()
        };
        assert_eq!(time_label(at(4), true), "Morning");
        assert_eq!(time_label(at(10), true), "Afternoon");
        assert_eq!(time_label(at(21), true), "Evening");
        assert_eq!(time_label(at(22), true), "Overnight");
        assert_eq!(time_label(at(9), false), "9:00");
    }
}
