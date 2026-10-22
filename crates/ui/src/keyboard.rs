//! The command keybinding store: the original command registry, the user's persisted
//! preferences and the rules that edit them. The app persists and applies it; the Settings
//! page edits it through [`KeyboardState::dispatch`].

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use gpui::{App, Global, Keystroke, Modifiers};
use serde::{Deserialize, Serialize};

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
    pub fn label(self) -> &'static str {
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
    pub dispatch: Option<Dispatch>,
    /// The Home route is open: its composer-only commands count as registered.
    pub home: bool,
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

    pub fn number_conflict(&self) -> bool {
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

pub fn allows_bare(id: &str) -> bool {
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

pub fn capture(stroke: &Keystroke) -> Option<String> {
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

pub fn modifier_accelerator(modifiers: Modifiers) -> String {
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
            let named = [
                ("enter", "⏎"),
                ("return", "⏎"),
                ("escape", "Esc"),
                ("space", "Space"),
                ("tab", "Tab"),
                ("backspace", "⌫"),
                ("left", "←"),
                ("right", "→"),
                ("up", "↑"),
                ("down", "↓"),
                ("mouseback", "Mouse Back"),
                ("mouseforward", "Mouse Forward"),
                ("control", ""),
                ("ctrl", ""),
                ("alt", ""),
                ("shift", ""),
                ("command", ""),
                ("cmdorctrl", ""),
                ("fn", ""),
            ]
            .into_iter()
            .find_map(|(name, label)| key.eq_ignore_ascii_case(name).then_some(label));
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

pub type Shortcut = (&'static str, &'static str, Vec<String>);

/// Commands the original's `⌘/` dialog leaves out of its recorded Home and thread states
/// (`refs/interactions/f3b.json`, `original-pool5/dialog-{home,thread}.txt`): all are `webview`
/// commands whose handler is not registered there (`temporaryChat` is also ChatGPT-feature
/// gated). Settings still lists them.
const DIALOG_HIDDEN: &[&str] = &[
    "temporaryChat",
    "openReviewTab",
    "undoAppAction",
    "redoAppAction",
];

/// Commands whose original handler exists only on the Home composer
/// (`backgroundSubmit.enabled` needs `kind === "home"`); recorded: the thread dialog omits them.
const HOME_ONLY: &[&str] = &["composer.submitInBackground"];

/// The dialog labels a few commands differently from Settings: `toggleSidePanel` is `New tab`
/// there, and the workspace tab view is `New tab in full view`.
pub fn dialog_title(command: &Command) -> &'static str {
    match command.id {
        "toggleSidePanel" => "New tab",
        "showWorkspaceTabView" => "New tab in full view",
        _ => command.title,
    }
}

/// The original dialog's row order within each section, recorded from the app
/// (`original-pool5/dialog-home.txt`). Titles the original does not list keep registry order
/// after it.
const DIALOG_ORDER: &[&str] = &[
    "New chat",
    "Archive chat",
    "New standalone chat",
    "Toggle pin",
    "Find",
    "Focus tab 1",
    "Focus tab 2",
    "Focus tab 3",
    "Focus tab 4",
    "Focus tab 5",
    "Focus tab 6",
    "Focus tab 7",
    "Focus tab 8",
    "Focus tab 9",
    "Back",
    "Forward",
    "Next recently viewed chat",
    "Next tab",
    "Next chat",
    "Next chat needing attention",
    "Open your dot",
    "Previous recently viewed chat",
    "Previous tab",
    "Previous chat",
    "Go to recent chat 1",
    "Go to recent chat 2",
    "Go to recent chat 3",
    "Go to recent chat 4",
    "Go to recent chat 5",
    "Go to recent chat 6",
    "Toggle activity view",
    "Open browser tab",
    "Reopen closed tab",
    "New tab in full view",
    "Cycle workspace layout",
    "Toggle bottom panel",
    "Toggle navigation panel",
    "Toggle sidebar",
    "New tab",
    "Open terminal",
    "Open folder",
    "Clear all unreads",
    "Show Mini",
    "Settings",
    "Close other tabs",
    "Close Tab",
    "Close",
    "Open project picker",
    "Toggle voice chat",
    "Send message in background",
    "Copy conversation path",
    "Copy deeplink",
    "Copy working directory",
    "Force Reload Browser Page",
    "Voice in Mini",
    "Open command menu",
    "Reload Browser Page",
    "Rename chat",
    "Screenshot selection to current chat",
    "Screenshot selection to new chat",
    "Search Files…",
    "Show keyboard shortcuts",
    "Go to chat 1",
    "Go to chat 2",
    "Go to chat 3",
    "Go to chat 4",
    "Go to chat 5",
    "Go to chat 6",
    "Go to chat 7",
    "Go to chat 8",
    "Go to chat 9",
    "Toggle Notch",
];

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
        let mut commands: Vec<_> = COMMANDS
            .iter()
            .filter(|c| {
                c.section == section
                    && state.active_commands.contains(c.id)
                    && !DIALOG_HIDDEN.contains(&c.id)
                    && (state.home || !HOME_ONLY.contains(&c.id))
            })
            .collect();
        commands.sort_by_key(|c| {
            DIALOG_ORDER
                .iter()
                .position(|title| *title == dialog_title(c))
                .unwrap_or(usize::MAX)
        });
        for command in commands {
            let keys: Vec<_> = state.preferences.keys(command).map(key_label).collect();
            if !keys.is_empty() {
                shortcuts.push((section_title(section), dialog_title(command), keys));
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
