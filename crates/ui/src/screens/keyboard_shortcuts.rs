use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use super::{Layer, deco, on_card, probe};
use crate::{
    composer::Composer,
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Div, Entity, FocusHandle,
    Focusable, Global, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, Modifiers,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use shapes::Corner::{Round, Squircle};
use theme::{ActiveTheme as _, Theme};

pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub section: &'static str,
    pub keys: &'static [&'static str],
    pub visible: bool,
}

include!("keyboard_shortcuts_data.rs");

pub fn commands() -> &'static [Command] {
    COMMANDS
}

const LINK_TITLE: &str = "Open web link in default browser";
const LINK_DESCRIPTION: &str =
    "Hold the assigned keys while clicking a web link to open it in your default system browser";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub bindings: BTreeMap<String, Vec<String>>,
    pub primary_number_shortcut_target: NumberTarget,
    pub external_browser_link_shortcut: LinkModifier,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NumberTarget {
    Sidebar,
    #[default]
    Tabs,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LinkModifier {
    Unassigned,
    #[default]
    Primary,
    Alt,
    PrimaryShift,
}

impl LinkModifier {
    pub fn matches(self, modifiers: Modifiers) -> bool {
        match self {
            Self::Unassigned => false,
            Self::Primary => {
                modifiers.platform && !modifiers.alt && !modifiers.shift && !modifiers.control
            }
            Self::Alt => {
                modifiers.alt && !modifiers.platform && !modifiers.shift && !modifiers.control
            }
            Self::PrimaryShift => {
                modifiers.platform && modifiers.shift && !modifiers.alt && !modifiers.control
            }
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Unassigned => "Unassigned",
            Self::Primary => "⌘+Click",
            Self::Alt => "⌥+Click",
            Self::PrimaryShift => "⇧⌘+Click",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Change {
    Bindings { command: String, keys: Vec<String> },
    ResetCommand(String),
    Reset,
    NumberTarget(NumberTarget),
    LinkModifier(LinkModifier),
}

pub type Dispatch = Arc<dyn Fn(Change, &mut App) + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PreferenceKind {
    Number,
    Link,
}

#[derive(Clone, Default)]
pub struct KeyboardState {
    pub loaded: bool,
    pub preferences: Preferences,
    pub active_commands: BTreeSet<&'static str>,
    pub capture_owner: Option<gpui::EntityId>,
    pub error: Option<String>,
    pub preference_error: Option<PreferenceKind>,
    /// The Home route is open: its composer-only commands count as registered.
    pub home: bool,
    pub dispatch: Option<Dispatch>,
}
impl Global for KeyboardState {}

const CMD_NUMBERS: &[&str] = &[
    "Command+1",
    "Command+2",
    "Command+3",
    "Command+4",
    "Command+5",
    "Command+6",
    "Command+7",
    "Command+8",
    "Command+9",
];
const CTRL_NUMBERS: &[&str] = &[
    "Control+1",
    "Control+2",
    "Control+3",
    "Control+4",
    "Control+5",
    "Control+6",
    "Control+7",
    "Control+8",
    "Control+9",
];

impl Preferences {
    pub fn defaults(&self, command: &Command) -> &'static [&'static str] {
        let numbered = command
            .id
            .strip_prefix("thread")
            .map(|n| (n, false))
            .or_else(|| command.id.strip_prefix("focusTab").map(|n| (n, true)));
        if let Some((number, tab)) = numbered
            && let Ok(index @ 1..=9) = number.parse::<usize>()
        {
            let primary = tab == (self.primary_number_shortcut_target == NumberTarget::Tabs);
            let numbers = if primary { CMD_NUMBERS } else { CTRL_NUMBERS };
            return &numbers[index - 1..index];
        }
        command.keys
    }

    pub fn keys<'a>(&'a self, command: &'a Command) -> impl Iterator<Item = &'a str> {
        let custom = self.bindings.get(command.id);
        custom.into_iter().flatten().map(String::as_str).chain(
            self.defaults(command)
                .iter()
                .copied()
                .filter(move |_| custom.is_none()),
        )
    }

    pub fn validate(&self) -> Result<(), String> {
        for (command, keys) in &self.bindings {
            if keys.iter().any(|key| !valid_accelerator(command, key)) {
                return Err(format!("Invalid shortcut for {command}"));
            }
        }
        Ok(())
    }

    pub fn change(&mut self, change: &Change) -> Result<(), String> {
        match change {
            Change::Bindings { command, keys } => {
                let definition = COMMANDS
                    .iter()
                    .find(|entry| entry.id == command)
                    .ok_or_else(|| "Unknown shortcut".to_owned())?;
                for key in keys {
                    if !valid_accelerator(command, key) {
                        return Err("Invalid shortcut".into());
                    }
                    if let Some(other) = self.conflict(definition, key) {
                        return Err(format!("Used by {other}"));
                    }
                }
                let mut seen = BTreeSet::new();
                self.bindings.insert(
                    command.clone(),
                    keys.iter()
                        .filter(|key| seen.insert(canonical(key)))
                        .cloned()
                        .collect(),
                );
            }
            Change::ResetCommand(command) => {
                self.bindings.remove(command);
            }
            Change::Reset => {
                self.bindings.clear();
                self.external_browser_link_shortcut = LinkModifier::default();
            }
            Change::NumberTarget(target) => self.primary_number_shortcut_target = *target,
            Change::LinkModifier(modifier) => self.external_browser_link_shortcut = *modifier,
        }
        Ok(())
    }

    fn conflict(&self, command: &Command, key: &str) -> Option<&'static str> {
        let key = canonical(key);
        match key.split_whitespace().next()? {
            "cmd-f" => return Some("Find"),
            "cmd-g" => return Some("Find Next"),
            "cmd-shift-g" => return Some("Find Previous"),
            _ => {}
        }
        COMMANDS
            .iter()
            .find(|other| {
                other.id != command.id
                    && !shared_context(command.id, other.id, &key)
                    && self
                        .keys(other)
                        .any(|other_key| overlaps(&key, &canonical(other_key)))
            })
            .map(|other| other.title)
    }

    fn number_conflict(&self) -> bool {
        COMMANDS
            .iter()
            .filter(|c| c.id.starts_with("thread") || c.id.starts_with("focusTab"))
            .flat_map(|c| self.defaults(c))
            .any(|key| {
                self.bindings.iter().any(|(id, keys)| {
                    !id.starts_with("thread")
                        && !id.starts_with("focusTab")
                        && keys
                            .iter()
                            .any(|custom| canonical(key) == canonical(custom))
                })
            })
    }
}

fn overlaps(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|rest| rest.starts_with(' '))
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with(' '))
}

fn shared_context(left: &str, right: &str, key: &str) -> bool {
    const GROUPS: &[&[&str]] = &[
        &["closeTab", "closeWindow"],
        &["nextTab", "nextThread"],
        &["nextTab", "nextRecentThread"],
        &["previousTab", "previousThread"],
        &["previousTab", "previousRecentThread"],
        &["goToLine", "focusBrowserAddressBar"],
        &["file.goToDefinition", "navigateForward"],
    ];
    (key == "enter"
        && [left, right]
            .iter()
            .all(|id| matches!(*id, "composer.submit" | "approval.approve")))
        || GROUPS
            .iter()
            .any(|group| group.contains(&left) && group.contains(&right))
}

fn allows_bare(id: &str) -> bool {
    matches!(id, "globalDictationHold" | "globalDictationSingleTap")
}

fn valid_accelerator(command: &str, key: &str) -> bool {
    let normalized = canonical(key);
    if normalized.is_empty() {
        return false;
    }
    let bare = allows_bare(command)
        && !key.contains(char::is_whitespace)
        && normalized
            .split('-')
            .all(|part| matches!(part, "ctrl" | "cmd" | "alt" | "shift" | "fn"));
    bare || normalized
        .split_whitespace()
        .all(|stroke| Keystroke::parse(stroke).is_ok())
}

pub fn canonical(accelerator: &str) -> String {
    accelerator
        .split_whitespace()
        .map(|chord| {
            let mut parts: Vec<_> = chord
                .split('+')
                .map(|part| match part {
                    "CmdOrCtrl" | "Command" | "Cmd" => "cmd",
                    "Control" | "Ctrl" => "ctrl",
                    "Option" | "Alt" => "alt",
                    "Shift" => "shift",
                    "Return" => "enter",
                    other => other,
                })
                .map(str::to_lowercase)
                .collect();
            if parts.len() > 1 {
                let key = parts.pop().unwrap_or_default();
                parts.sort_unstable();
                parts.push(key);
            }
            parts.join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn capture(stroke: &Keystroke) -> Option<String> {
    if matches!(
        stroke.key.as_str(),
        "shift" | "control" | "alt" | "cmd" | "fn"
    ) {
        return None;
    }
    let mut key = modifier_accelerator(stroke.modifiers);
    if !key.is_empty() {
        key.push('+');
    }
    key.push_str(&stroke.key);
    Some(key)
}

pub(super) fn modifier_accelerator(modifiers: Modifiers) -> String {
    [
        (modifiers.function, "Fn"),
        (modifiers.control, "Control"),
        (modifiers.alt, "Alt"),
        (modifiers.shift, "Shift"),
        (modifiers.platform, "Command"),
    ]
    .into_iter()
    .filter_map(|(held, name)| held.then_some(name))
    .collect::<Vec<_>>()
    .join("+")
}

pub fn key_label(accelerator: &str) -> String {
    accelerator
        .split_whitespace()
        .map(|chord| {
            let mut label = String::new();
            let pieces: Vec<_> = chord.split('+').collect();
            for (name, glyph) in [
                ("Fn", "fn"),
                ("Control", "⌃"),
                ("Ctrl", "⌃"),
                ("Alt", "⌥"),
                ("Shift", "⇧"),
                ("Command", "⌘"),
                ("CmdOrCtrl", "⌘"),
            ] {
                if pieces.contains(&name) {
                    label.push_str(glyph);
                }
            }
            let key = pieces.last().copied().unwrap_or("");
            let named = match key.to_ascii_lowercase().as_str() {
                "enter" | "return" => Some("⏎"),
                "escape" => Some("Esc"),
                "space" => Some("Space"),
                "tab" => Some("Tab"),
                "backspace" => Some("⌫"),
                "left" => Some("←"),
                "right" => Some("→"),
                "up" => Some("↑"),
                "down" => Some("↓"),
                "mouseback" => Some("Mouse Back"),
                "mouseforward" => Some("Mouse Forward"),
                "control" | "ctrl" | "alt" | "shift" | "command" | "cmdorctrl" | "fn" => Some(""),
                _ => None,
            };
            if let Some(named) = named {
                label.push_str(named);
            } else {
                label.push_str(&key.to_uppercase());
            }
            label
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Commands whose original handler exists only on the Home composer
/// (`backgroundSubmit.enabled` needs `kind === "home"`).
const HOME_ONLY: &[&str] = &["composer.submitInBackground"];

pub type Shortcut = (&'static str, &'static Command, Vec<String>);

pub fn active_shortcuts(cx: &App) -> Vec<Shortcut> {
    let Some(state) = cx.try_global::<KeyboardState>() else {
        return Vec::new();
    };
    let mut shortcuts = Vec::new();
    for section in [
        "thread",
        "navigation",
        "panels",
        "workspace",
        "skills",
        "configure",
        "app",
        "general",
    ] {
        for command in COMMANDS.iter().filter(|c| {
            c.section == section
                && state.active_commands.contains(c.id)
                && (state.home || !HOME_ONLY.contains(&c.id))
        }) {
            let keys: Vec<_> = state.preferences.keys(command).map(key_label).collect();
            if !keys.is_empty() {
                shortcuts.push((section_title(section), command, keys));
            }
        }
    }
    shortcuts
}

fn section_title(section: &str) -> &'static str {
    match section {
        "thread" => "Chat",
        "navigation" => "Navigation",
        "panels" => "Panels",
        "workspace" => "Project",
        "skills" => "Skills",
        "configure" => "Configure",
        "app" => "App",
        _ => "General",
    }
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("keyboard-shortcuts-settings", cx, |window, cx| {
            KeyboardPage::new(window, cx)
        })
        .into_any_element()
}

#[derive(Clone)]
enum Control {
    SearchMode,
    ClearSearch,
    Edit(&'static str, usize),
    Remove(&'static str, usize),
    ResetCommand(&'static str),
    ResetAll,
    CancelCapture,
    Advanced,
    NumberMenu,
    LinkMenu,
    Number(NumberTarget),
    Link(LinkModifier),
}

struct Editing {
    command: &'static str,
    index: usize,
    first: Option<String>,
    error: Option<String>,
}

struct KeyboardPage {
    search: Entity<Composer>,
    scroll: gpui::ScrollHandle,
    strokes: String,
    search_by_key: bool,
    editing: Option<Editing>,
    capture_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    advanced: bool,
    number_menu: bool,
    link_menu: bool,
    menu_hovered: bool,
    confirmation: Option<Entity<Confirmation>>,
    held_modifiers: Option<Modifiers>,
    capture_generation: u64,
    layer: Layer,
    _key_interceptor: gpui::Subscription,
}

impl KeyboardPage {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<KeyboardState>() {
            cx.set_global(KeyboardState::default());
        }
        let search = cx.new(|cx| Composer::search_field("Search shortcuts", cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<KeyboardState>(|_, cx| cx.notify())
            .detach();
        let capture_focus = cx.focus_handle().tab_stop(true);
        let owner = cx.entity_id();
        cx.on_release(move |_, cx| {
            if cx
                .try_global::<KeyboardState>()
                .is_some_and(|state| state.capture_owner == Some(owner))
            {
                cx.update_global::<KeyboardState, _>(|state, _| state.capture_owner = None);
            }
        })
        .detach();
        cx.on_focus(&capture_focus, window, |this, _, cx| {
            this.publish_capture(this.search_by_key || this.editing.is_some(), cx);
        })
        .detach();
        cx.on_blur(&capture_focus, window, |this, window, cx| {
            if !this.capture_focus.is_focused(window) {
                this.publish_capture(false, cx);
            }
            if !this.capture_focus.is_focused(window) && this.editing.take().is_some() {
                this.return_focus = None;
                this.held_modifiers = None;
                this.capture_generation += 1;
                cx.notify();
            }
        })
        .detach();
        let weak = cx.weak_entity();
        let key_interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            if matches!(
                event.keystroke.key.as_str(),
                "shift" | "control" | "alt" | "platform" | "function"
            ) {
                return;
            }
            if let Some(page) = weak.upgrade() {
                let state = page.read(cx);
                let capturing = state.capture_focus.is_focused(window)
                    && (state.search_by_key || state.editing.is_some());
                if capturing {
                    page.update(cx, |page, cx| {
                        page.capture_key(&event.keystroke, window, cx)
                    });
                    cx.stop_propagation();
                }
            }
        });
        Self {
            search,
            scroll: gpui::ScrollHandle::default(),
            strokes: String::new(),
            search_by_key: false,
            editing: None,
            capture_focus,
            return_focus: None,
            advanced: false,
            number_menu: false,
            link_menu: false,
            menu_hovered: false,
            confirmation: None,
            held_modifiers: None,
            capture_generation: 0,
            layer: Layer::default(),
            _key_interceptor: key_interceptor,
        }
    }

    fn publish_capture(&self, active: bool, cx: &mut Context<Self>) {
        let owner = cx.entity_id();
        let current = cx.global::<KeyboardState>().capture_owner;
        let next = if active {
            Some(owner)
        } else {
            current.filter(|id| *id != owner)
        };
        if next != current {
            cx.update_global::<KeyboardState, _>(|state, _| state.capture_owner = next);
        }
    }

    fn dispatch(&mut self, change: Change, cx: &mut Context<Self>) -> bool {
        let dispatch = cx
            .try_global::<KeyboardState>()
            .and_then(|state| state.dispatch.clone());
        if let Some(dispatch) = dispatch {
            dispatch(change, cx);
            cx.notify();
            cx.global::<KeyboardState>().error.is_none()
        } else {
            cx.update_global::<KeyboardState, _>(|state, _| {
                state.error = Some("Keyboard shortcut settings are unavailable.".into());
            });
            false
        }
    }

    fn activate(
        &mut self,
        action: Control,
        append: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(action, Control::Edit(..) | Control::CancelCapture) {
            self.editing = None;
            self.return_focus = None;
            self.held_modifiers = None;
            self.capture_generation += 1;
        }
        self.number_menu = matches!(action, Control::NumberMenu) && !self.number_menu;
        self.link_menu = matches!(action, Control::LinkMenu) && !self.link_menu;
        match action {
            Control::SearchMode => {
                self.search_by_key = !self.search_by_key;
                self.strokes.clear();
                self.search.update(cx, |search, cx| search.set_text("", cx));
                if self.search_by_key {
                    self.capture_focus.focus(window, cx);
                } else {
                    self.search.focus_handle(cx).focus(window, cx);
                }
            }
            Control::ClearSearch => {
                self.search.update(cx, |search, cx| search.set_text("", cx));
                self.search.focus_handle(cx).focus(window, cx);
            }
            Control::Edit(command, index) => {
                self.return_focus = window.focused(cx);
                let index = if append {
                    COMMANDS
                        .iter()
                        .find(|c| c.id == command)
                        .map(|c| cx.global::<KeyboardState>().preferences.keys(c).count())
                        .unwrap_or(index)
                } else {
                    index
                };
                self.editing = Some(Editing {
                    command,
                    index,
                    first: None,
                    error: None,
                });
                self.capture_generation += 1;
                self.capture_focus.focus(window, cx);
            }
            Control::Remove(command, index) => {
                if let Some(definition) = COMMANDS.iter().find(|c| c.id == command) {
                    let mut keys: Vec<_> = cx
                        .global::<KeyboardState>()
                        .preferences
                        .keys(definition)
                        .map(str::to_owned)
                        .collect();
                    if index < keys.len() {
                        keys.remove(index);
                    }
                    self.dispatch(
                        Change::Bindings {
                            command: command.into(),
                            keys,
                        },
                        cx,
                    );
                }
            }
            Control::ResetCommand(command) => {
                self.dispatch(Change::ResetCommand(command.into()), cx);
            }
            Control::ResetAll => {
                let modal = cx.new(|cx| {
                    Confirmation::new(
                        "Reset all keyboard shortcuts?",
                        "This will discard all custom shortcuts and restore the defaults",
                        "Reset all",
                        window,
                        cx,
                    )
                    .destructive()
                });
                cx.subscribe_in(&modal, window, |this, modal, outcome, window, cx| {
                    if *outcome == Outcome::Cancel || this.dispatch(Change::Reset, cx) {
                        this.confirmation = None;
                    } else {
                        let error = cx
                            .global::<KeyboardState>()
                            .error
                            .clone()
                            .unwrap_or_else(|| "Failed to reset keyboard shortcuts".into());
                        modal.update(cx, |modal, cx| modal.failed(error, window, cx));
                    }
                    cx.notify();
                })
                .detach();
                self.confirmation = Some(modal);
            }
            Control::CancelCapture => {
                self.editing = None;
                self.held_modifiers = None;
                self.capture_generation += 1;
                if let Some(focus) = self.return_focus.take() {
                    focus.focus(window, cx);
                }
            }
            Control::Advanced => self.advanced = !self.advanced,
            Control::Number(target) => {
                self.dispatch(Change::NumberTarget(target), cx);
            }
            Control::Link(modifier) => {
                self.dispatch(Change::LinkModifier(modifier), cx);
            }
            Control::NumberMenu | Control::LinkMenu => {}
        }
        self.publish_capture(
            self.capture_focus.is_focused(window) && (self.search_by_key || self.editing.is_some()),
            cx,
        );
        cx.notify();
    }

    fn save_capture(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = &self.editing else { return };
        let Some(command) = COMMANDS.iter().find(|c| c.id == editing.command) else {
            return;
        };
        let mut keys: Vec<_> = cx
            .global::<KeyboardState>()
            .preferences
            .keys(command)
            .map(str::to_owned)
            .collect();
        if editing.index < keys.len() {
            keys[editing.index] = key;
        } else {
            keys.push(key);
        }
        if self.dispatch(
            Change::Bindings {
                command: command.id.into(),
                keys,
            },
            cx,
        ) {
            self.editing = None;
            self.capture_generation += 1;
            self.publish_capture(false, cx);
            if let Some(focus) = self.return_focus.take() {
                focus.focus(window, cx);
            }
        } else if let Some(editing) = &mut self.editing {
            editing.error = cx.global::<KeyboardState>().error.clone();
            editing.first = None;
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.is_held {
            self.capture_key(&event.keystroke, window, cx);
        }
    }

    fn capture_key(&mut self, stroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let escape = stroke.key == "escape" && stroke.modifiers == Modifiers::default();
        if escape && (self.number_menu || self.link_menu) {
            self.number_menu = false;
            self.link_menu = false;
            self.menu_hovered = false;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.editing.is_some() && self.capture_focus.is_focused(window) {
            cx.stop_propagation();
            if escape {
                self.activate(Control::CancelCapture, false, window, cx);
                return;
            }
            let Some(key) = capture(stroke) else { return };
            let editing = self.editing.as_mut().unwrap();
            if let Some(first) = editing.first.take() {
                self.save_capture(format!("{first} {key}"), window, cx);
            } else if key.contains('+') || allows_bare(editing.command) {
                self.save_capture(key, window, cx);
            } else {
                editing.first = Some(key.clone());
                editing.error = None;
                self.capture_generation += 1;
                let generation = self.capture_generation;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.capture_generation == generation {
                            this.save_capture(key, window, cx);
                        }
                    });
                })
                .detach();
                cx.notify();
            }
        } else if self.search_by_key && self.capture_focus.is_focused(window) {
            cx.stop_propagation();
            if escape {
                self.search_by_key = false;
                self.strokes.clear();
                self.search.focus_handle(cx).focus(window, cx);
            } else if let Some(key) = capture(stroke) {
                let next = format!("{} {}", self.strokes, key_label(&key));
                let state = cx.global::<KeyboardState>();
                let sequence = !self.strokes.is_empty()
                    && COMMANDS.iter().any(|command| {
                        state
                            .preferences
                            .keys(command)
                            .any(|key| key_label(key).starts_with(&next))
                    });
                self.strokes = if sequence { next } else { key_label(&key) };
            }
            cx.notify();
        } else if escape && !self.search.read(cx).field_text().is_empty() {
            self.search.update(cx, |search, cx| search.set_text("", cx));
            cx.stop_propagation();
        }
    }

    fn modifiers(&mut self, modifiers: Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .editing
            .as_ref()
            .is_some_and(|e| allows_bare(e.command))
        {
            return;
        }
        if modifiers == Modifiers::default() {
            if let Some(held) = self.held_modifiers.take() {
                self.save_capture(modifier_accelerator(held), window, cx);
            }
        } else {
            *self.held_modifiers.get_or_insert_default() |= modifiers;
        }
    }

    fn control(
        &mut self,
        id: String,
        label: String,
        content: AnyElement,
        action: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = crate::focus::handle(SharedString::from(id.clone()), 0, window, cx);
        let handle = focus.read(cx).handle.clone();
        let keyboard_action = action.clone();
        let hover = if matches!(&action, Control::ClearSearch) {
            gpui::transparent_black()
        } else {
            cx.theme().list_hover
        };
        let control = div()
            .id(SharedString::from(id))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .relative()
            .rounded(Theme::RADIUS_LG)
            .hover(move |s| s.bg(hover))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    this.activate(
                        keyboard_action.clone(),
                        e.keystroke.modifiers.shift,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                handle.focus(window, cx);
                this.activate(action.clone(), e.modifiers().shift, window, cx);
            }))
            .child(content);
        crate::focus::ring(control, &focus, Theme::RADIUS_LG, px(0.), cx)
    }

    fn icon_control(
        &mut self,
        command: &Command,
        index: usize,
        name: &'static str,
        action: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let verb = match action {
            Control::Remove(..) => "Clear",
            Control::ResetCommand(..) => "Reset",
            _ => "Change",
        };
        self.control(
            format!("shortcut-{}-{index}-{verb}", command.id),
            format!("{verb} shortcut for {}", command.title),
            icon(name, 16., cx.theme().text_tertiary).into_any_element(),
            action,
            window,
            cx,
        )
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
    }

    fn row(
        &mut self,
        command: &'static Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let state = cx.global::<KeyboardState>();
        let keys: Vec<_> = state.preferences.keys(command).map(str::to_owned).collect();
        let customized = state.preferences.bindings.contains_key(command.id);
        let mut bindings = if allows_bare(command.id) {
            // The dictation rows size to their content (at least 160 px), end-aligned, with the
            // 8 px a hidden trash slot leaves.
            div()
                .min_w(px(160.))
                .flex_none()
                .pr(px(8.))
                .flex()
                .flex_col()
                .items_end()
        } else {
            div().w(px(384.)).max_w_full().flex().flex_col()
        };
        let count = keys.len().max(1)
            + usize::from(self.editing.as_ref().is_some_and(|e| {
                e.command == command.id && e.index == keys.len() && !keys.is_empty()
            }));
        for index in 0..count {
            let editing = self
                .editing
                .as_ref()
                .filter(|e| e.command == command.id && e.index == index);
            let control = if let Some(editing) = editing {
                let prompt = editing
                    .first
                    .as_ref()
                    .map(|key| format!("{} …", key_label(key)))
                    .unwrap_or_else(|| "Press shortcut".into());
                let error = editing.error.clone();
                let cancel = self.control(
                    format!("cancel-{}", command.id),
                    "Cancel".into(),
                    text("Cancel", 13., 18., 430., t.text).into_any_element(),
                    Control::CancelCapture,
                    window,
                    cx,
                );
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("shortcut-capture-input")
                                    .w(px(144.))
                                    .h(px(30.))
                                    .px(px(10.))
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(t.border)
                                    .track_focus(&self.capture_focus)
                                    .role(gpui::Role::TextInput)
                                    .aria_label(SharedString::from(format!(
                                        "Shortcut capture for {}",
                                        command.title
                                    )))
                                    .flex()
                                    .items_center()
                                    .child(text(prompt, 13., 18., 430., t.text)),
                            )
                            .child(cancel.px(px(8.)).h(px(28.)).flex().items_center()),
                    )
                    .children(error.map(|error| text(error, 12., 16., 430., t.text_warning)))
            } else {
                let key = keys.get(index);
                let label = key.map(|key| key_label(key)).unwrap_or_else(|| {
                    if allows_bare(command.id) {
                        "Off".into()
                    } else {
                        "Unassigned".into()
                    }
                });
                let keycap = div()
                    .flex()
                    .items_center()
                    .min_h(px(32.))
                    .child(if key.is_some() {
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .relative()
                            .child(on_card(10., Round, t.text_secondary.opacity(0.1), None))
                            .child(text(label, 12., 12., 430., t.text_secondary))
                            .into_any_element()
                    } else {
                        text(label, 13., 18.571_428, 430., t.text_secondary).into_any_element()
                    });
                let edit = self.icon_control(
                    command,
                    index,
                    "pencil",
                    Control::Edit(command.id, index),
                    window,
                    cx,
                );
                let mut row = div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(keycap)
                    .child(edit);
                // A spacer with nothing after it adds a trailing gap the original lacks.
                if key.is_some() || (customized && index == 0) {
                    row = row.child(div().flex_1());
                }
                if key.is_some() {
                    row = row.child(self.icon_control(
                        command,
                        index,
                        "shortcut-trash",
                        Control::Remove(command.id, index),
                        window,
                        cx,
                    ));
                }
                if customized && index == 0 {
                    row = row.child(self.icon_control(
                        command,
                        index,
                        "composer-reset",
                        Control::ResetCommand(command.id),
                        window,
                        cx,
                    ));
                }
                row
            };
            bindings = bindings.child(control);
        }
        let mut description = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(text(command.title, 13., 18.571_428, 500., t.text).truncate())
            .child(text(command.detail, 12., 16., 430., t.text_secondary).whitespace_normal());
        if command.id == "toggleSidebar" && keys.iter().any(|key| canonical(key) == "cmd-b") {
            description = description.child(
                text(
                    "⌘B applies bold, except at the start of the composer where it toggles the sidebar",
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                )
                .whitespace_normal(),
            );
        }
        div()
            .relative()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .items_center()
            .gap(px(24.))
            .child(description)
            .child(bindings)
    }

    fn card(&self, rows: Vec<Div>, t: &Theme) -> Div {
        let count = rows.len();
        div()
            .relative()
            .p(px(1.))
            .child(deco(
                &self.layer,
                20.,
                Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(crate::rounded_clip(
                px(19.),
                Squircle,
                div()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().enumerate().map(|(index, row)| {
                        row.when(index + 1 < count, |row| {
                            row.child(
                                div()
                                    .absolute()
                                    .left(px(16.))
                                    .right(px(16.))
                                    .bottom_0()
                                    .h(px(1.))
                                    .bg(crate::over(t.border, t.background_panel)),
                            )
                        })
                    })),
            ))
    }

    fn preference(&mut self, number: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let preferences = &cx.global::<KeyboardState>().preferences;
        let (title, detail, label, action, open) = if number {
            let tabs = preferences.primary_number_shortcut_target == NumberTarget::Tabs;
            (
                "Number shortcuts",
                if tabs {
                    "Use ⌘1–9 to switch tabs and ⌃1–9 to switch chats"
                } else {
                    "Use ⌘1–9 to switch chats and ⌃1–9 to switch tabs"
                },
                if tabs {
                    "⌘1-9 for Tabs"
                } else {
                    "⌘1-9 for Chats"
                },
                Control::NumberMenu,
                self.number_menu,
            )
        } else {
            (
                LINK_TITLE,
                LINK_DESCRIPTION,
                preferences.external_browser_link_shortcut.label(),
                Control::LinkMenu,
                self.link_menu,
            )
        };
        let current = label;
        let conflict = number && preferences.number_conflict();
        let kind = if number {
            PreferenceKind::Number
        } else {
            PreferenceKind::Link
        };
        let error = cx.global::<KeyboardState>().preference_error == Some(kind)
            && cx.global::<KeyboardState>().error.is_some();
        // The number select is a bordered button at the row's end; the link select is a bare
        // chip that starts the 384 px control column.
        let (chip_text, chip_icon, pad) = if number {
            (t.text, 14., 13.)
        } else {
            (t.text_secondary, 12., 9.)
        };
        let trigger = self
            .control(
                format!("shortcut-pref-{number}"),
                title.into(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(text(label, 13., 18., 430., chip_text))
                    .child(icon("chevron-down", chip_icon, t.text_tertiary))
                    .into_any_element(),
                action,
                window,
                cx,
            )
            .px(px(pad))
            .h(px(28.))
            .flex()
            .items_center();
        let fill = if open {
            Some(t.text.opacity(0.08))
        } else {
            number.then_some(t.primary_soft_alpha)
        };
        let trigger = div()
            .relative()
            .when(!number, |d| d.ml(px(-9.)))
            .children(fill.map(|fill| on_card(12.5, Squircle, fill, number.then_some(t.border))))
            .child(trigger);
        let options: Vec<(&str, Control)> = if number {
            vec![
                ("⌘1-9 for Tabs", Control::Number(NumberTarget::Tabs)),
                ("⌘1-9 for Chats", Control::Number(NumberTarget::Sidebar)),
            ]
        } else {
            vec![
                ("Unassigned", Control::Link(LinkModifier::Unassigned)),
                ("⌘+Click", Control::Link(LinkModifier::Primary)),
                ("⌥+Click", Control::Link(LinkModifier::Alt)),
                ("⇧⌘+Click", Control::Link(LinkModifier::PrimaryShift)),
            ]
        };
        let menu = if open {
            // The original popup sizes to its widest label (Chromium's measure, "⌘1-9 for Chats"
            // and "Unassigned"), 6 px and an 18 px check slot, in 8 px row padding inside the
            // popover's 4 px padding.
            let widest = if number { 93.02 } else { 72.83 };
            let rows: Vec<_> = options
                .into_iter()
                .enumerate()
                .map(|(index, (label, action))| {
                    self.control(
                        format!("shortcut-option-{number}-{index}"),
                        label.into(),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .w(px(widest))
                                    .child(text(label, 13., 18.571_428, 500., t.text)),
                            )
                            // The recorded check sits 5.57 px below the row top, not centered.
                            .child(
                                div()
                                    .size(px(18.))
                                    .flex_none()
                                    .relative()
                                    .top(px(0.28))
                                    .children(
                                        (label == current).then(|| icon("menu-check", 18., t.text)),
                                    ),
                            )
                            .into_any_element(),
                        action,
                        window,
                        cx,
                    )
                    .rounded(px(15.))
                    .pl(px(if number { 8.29 } else { 8.18 }))
                    .pr(px(if number { 7.71 } else { 7.82 }))
                    .py(px(5.))
                })
                .collect();
            Some(
                gpui::deferred(
                    div().absolute().top_0().right_0().child(
                        gpui::anchored()
                            .anchor(gpui::Anchor::TopRight)
                            .offset(gpui::point(px(-1.62), px(30.22)))
                            .child(
                                div()
                                    .id("shortcut-preference-menu")
                                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                        this.menu_hovered = *hovered
                                    }))
                                    // The popup's device-pixel snap differs by 1 px per menu:
                                    // shift and extend it to the original's pixels.
                                    .child(
                                        div()
                                            .relative()
                                            .left(px(if number { 0.5 } else { 1. }))
                                            .child(crate::composer::popover(
                                                &t,
                                                ((widest + 48.) * 2.).round() / 2.,
                                                |d| {
                                                    d.pb(px(if number { 4. } else { 4.5 }))
                                                        .children(rows)
                                                },
                                            )),
                                    ),
                            ),
                    ),
                )
                .into_any_element(),
            )
        } else {
            None
        };
        div()
            .relative()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .items_center()
            .gap(px(24.))
            .child(
                div()
                    .when(number, |d| d.flex_1().min_w_0())
                    .when(!number, |d| d.w(px(286.)).flex_none())
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(title, 13., 18.571_428, 500., t.text))
                    .child(text(detail, 12., 16., 430., t.text_secondary).whitespace_normal())
                    .when(error, |d| {
                        d.child(text(
                            "Could not save shortcut preference",
                            12.,
                            16.,
                            430.,
                            t.text_error,
                        ))
                    })
                    .when(conflict, |d| {
                        d.child(text(
                            "Some number shortcuts are already assigned to other actions",
                            12.,
                            16.,
                            430.,
                            t.text_warning,
                        ))
                    }),
            )
            .child(
                div()
                    .relative()
                    .when(!number, |d| d.w(px(384.)).flex())
                    .when(open, |d| {
                        d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if !this.menu_hovered {
                                this.number_menu = false;
                                this.link_menu = false;
                                cx.notify();
                            }
                        }))
                    })
                    .child(trigger.children(menu)),
            )
    }
}

impl Render for KeyboardPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.global::<KeyboardState>();
        let (loaded, preferences, error) =
            (state.loaded, state.preferences.clone(), state.error.clone());
        let preference_error = state.preference_error;
        let search_by_key = self.search_by_key;
        let query = if self.search_by_key {
            self.strokes.clone()
        } else {
            self.search.read(cx).field_text().to_owned()
        };
        let pattern = crate::screens::sidebar::MatchPattern::new(query.trim());
        let matches = |command: &&Command| {
            loaded
                && command.visible
                && (query.trim().is_empty()
                    || if search_by_key {
                        preferences.keys(command).any(|key| {
                            let label = key_label(key);
                            label == query || label.starts_with(&format!("{query} "))
                        })
                    } else {
                        [command.id, command.title, command.detail]
                            .iter()
                            .any(|value| pattern.score(value) > 0)
                    })
        };
        let mut pending_link = loaded
            && !search_by_key
            && (query.trim().is_empty()
                || [
                    LINK_TITLE,
                    LINK_DESCRIPTION,
                    preferences.external_browser_link_shortcut.label(),
                ]
                .iter()
                .any(|text| pattern.score(text) > 0));
        let mut rows = Vec::new();
        let mut dictation = Vec::new();
        for command in COMMANDS.iter().filter(matches) {
            if command.id == "globalDictationSingleTap" && !self.advanced && query.is_empty() {
                continue;
            }
            if allows_bare(command.id) {
                dictation.push(self.row(command, window, cx));
            } else {
                rows.push(self.row(command, window, cx));
                if command.id == "openBrowserTab" && pending_link {
                    rows.push(self.preference(false, window, cx));
                    pending_link = false;
                }
            }
        }
        if pending_link {
            rows.push(self.preference(false, window, cx));
        }
        let number = loaded
            && !self.search_by_key
            && (query.is_empty()
                || pattern.score("Number shortcuts Use ⌘1–9 to switch chats and tabs") > 0);
        let mut cards = div().flex().flex_col().gap(px(40.));
        if number {
            let row = self.preference(true, window, cx);
            cards = cards.child(self.card(vec![row], &t));
        }
        if !loaded {
            if error.is_none() {
                cards = cards.child(text("Loading shortcuts…", 13., 20., 430., t.text_secondary));
            }
        } else if rows.is_empty() && dictation.is_empty() && !number {
            cards = cards.child(self.card(
                vec![div()
                    .min_h(px(64.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text(
                        "No matching shortcuts",
                        13.,
                        18.571_428,
                        430.,
                        t.text_secondary,
                    ))],
                &t,
            ));
        } else if !rows.is_empty() {
            cards = cards.child(self.card(rows, &t));
        }
        if !dictation.is_empty() {
            if query.is_empty() {
                let advanced = self
                    .control(
                        "dictation-advanced".into(),
                        "Advanced".into(),
                        div()
                            .flex()
                            .gap(px(4.))
                            .child(icon("chevron-down", 14., t.text_tertiary))
                            .child(text("Advanced", 13., 18., 430., t.text_tertiary))
                            .into_any_element(),
                        Control::Advanced,
                        window,
                        cx,
                    )
                    .px(px(16.))
                    .py(px(8.));
                dictation.push(div().child(advanced));
            }
            cards = cards.child(
                div()
                    .flex()
                    .flex_col()
                    .child(self.card(dictation, &t))
                    .child(div().px(px(16.)).pt(px(6.)).child(text(
                        "Works in any app. Press Esc to cancel a recording.",
                        12.,
                        16.,
                        430.,
                        t.text_secondary,
                    ))),
            );
        }
        let toggle = self
            .control(
                "shortcut-search-mode".into(),
                "Search by keystrokes".into(),
                div()
                    .relative()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(self.search_by_key, |d| {
                        d.child(on_card(12.5, Squircle, t.text.opacity(0.05), None))
                    })
                    .child(icon(
                        "shortcut-keystrokes",
                        18.,
                        if self.search_by_key {
                            t.text
                        } else {
                            t.text_tertiary
                        },
                    ))
                    .into_any_element(),
                Control::SearchMode,
                window,
                cx,
            )
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center();
        let clear = (!self.search_by_key && !query.is_empty()).then(|| {
            self.control(
                "clear-shortcut-search".into(),
                "Clear search".into(),
                icon("shortcut-clear", 16., t.text_secondary).into_any_element(),
                Control::ClearSearch,
                window,
                cx,
            )
            .size(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
        });
        let search = div()
            .id("shortcut-search")
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(11.))
            .relative()
            .child(on_card(
                9999.,
                Round,
                t.control_background,
                Some(t.border_strong),
            ))
            .child(icon("shortcut-search", 18., t.text_secondary))
            .child(if self.search_by_key {
                div()
                    .id("shortcut-keystroke-input")
                    .flex_1()
                    .min_w_0()
                    .track_focus(&self.capture_focus)
                    .role(gpui::Role::TextInput)
                    .aria_label("Keystroke search capture")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.capture_focus.focus(window, cx)),
                    )
                    .child(text(
                        if query.is_empty() {
                            "Press shortcut to search"
                        } else {
                            &query
                        },
                        14.,
                        18.,
                        430.,
                        if query.is_empty() {
                            t.text_tertiary
                        } else {
                            t.text
                        },
                    ))
                    .into_any_element()
            } else {
                // The original input text sits one device pixel lower than the centered editor.
                div()
                    .flex_1()
                    .min_w_0()
                    .relative()
                    .top(px(0.5))
                    .child(self.search.clone())
                    .into_any_element()
            })
            .children(clear)
            .child(toggle);
        let search_bar = loaded.then(|| {
            div()
                .tab_group()
                .tab_index(1)
                .tab_stop(false)
                .absolute()
                .top(px(77_f32.max(-f32::from(self.scroll.offset().y))))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div().w_full().max_w(px(768.)).px(px(20.)).child(
                        div()
                            .relative()
                            .top_layer()
                            .bg(t.surface)
                            .pt(px(20.))
                            .pb(px(8.))
                            .child(search)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .top(px(60.))
                                    .h(px(32.))
                                    .bg(gpui::linear_gradient(
                                        180.,
                                        gpui::linear_color_stop(t.surface, 0.),
                                        gpui::linear_color_stop(t.surface.opacity(0.), 1.),
                                    )),
                            ),
                    ),
                )
        });
        let reset = if preferences.bindings.is_empty()
            && preferences.external_browser_link_shortcut == LinkModifier::default()
        {
            None
        } else {
            Some(
                self.control(
                    "reset-shortcuts".into(),
                    "Reset all to defaults".into(),
                    text("Reset all to defaults", 13., 18., 430., t.text).into_any_element(),
                    Control::ResetAll,
                    window,
                    cx,
                )
                .px(px(10.))
                .py(px(5.)),
            )
        };
        let scrolls =
            self.scroll.bounds().size.height <= px(0.) || self.scroll.max_offset().y > px(0.);
        div()
            .size_full()
            .relative()
            .tab_group()
            .tab_stop(false)
            .flex()
            .flex_col()
            .capture_key_down(cx.listener(Self::key))
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, window, cx| {
                    this.modifiers(event.modifiers, window, cx)
                },
            ))
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("keyboard-shortcuts-page")
                    .relative()
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    // Chromium composites a scroller only while its content overflows.
                    .when(scrolls, |d| d.layer())
                    .child(probe(&self.layer))
                    .child(
                        div().pt(px(16.)).pb(px(1.)).flex().justify_center().child(
                            div()
                                .w_full()
                                .max_w(px(768.))
                                .px(px(20.))
                                .min_h(px(60.))
                                .py(px(12.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(text("Keyboard shortcuts", 28., 33.6, 500., t.text))
                                .children(reset),
                        ),
                    )
                    .child(
                        div()
                            .tab_group()
                            .tab_index(2)
                            .tab_stop(false)
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(768.))
                                    .px(px(20.))
                                    .pb(px(20.))
                                    .flex()
                                    .flex_col()
                                    .when(loaded, |d| d.child(div().h(px(60.))))
                                    .child(div().pt(px(20.)).child(cards))
                                    .children(
                                        error
                                            .filter(|_| preference_error.is_none())
                                            .map(|error| text(error, 12., 16., 430., t.text_error)),
                                    ),
                            ),
                    )
                    .children(search_bar),
            )
            .children(self.confirmation.clone())
    }
}
