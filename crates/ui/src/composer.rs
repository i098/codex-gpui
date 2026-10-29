//! The prompt composer: a multi-line editor, attachment chips, slash commands,
//! permission and model menus, and approval prompts.
//! Numbers use the CSS pixels of refs/home and refs/composer.
//! Clipboard text of at least 5,000 UTF-16 units becomes a saved text attachment.
//! Clipboard images retain their bytes and show a thumbnail.
//! S024: the first Escape shows `Esc` for two seconds; the next Escape interrupts the turn.
//! Composition and open suggestions consume Escape without stopping.
//! Permission presets apply to start requests. Full access requires the shared warning dialog.
//! `cargo test -p app --test s024` replays `refs/interactions/s024.json`, including the footer focus rings.

use crate::{MONO, confirmation::Confirmation, icon::icon, text};
use bridge::types::{
    ApprovalDecision, CommandApprovalRequest, FileChangeApprovalRequest, Model,
    PermissionsApprovalRequest, QueuedSubmission,
};
use gpui::{
    AnyElement, AnyView, App, AvailableSpace, Bounds, ClipboardItem, Context, Corners, CursorStyle,
    Div, ElementId, ElementInputHandler, Entity, EntityInputHandler, ExternalPaths, FocusHandle,
    Focusable, GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathPromptOptions, Pixels, Point, Render, ScrollHandle, SharedString, Size, Stateful,
    StatefulInteractiveElement, Style, Styled, Task, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, WrappedLine, actions, deferred, div, fill, point,
    prelude::FluentBuilder as _, px, relative, size,
};
use serde_json::{Value, json};
use std::{ops::Range, path::PathBuf, rc::Rc, time::Duration};
use theme::{ActiveTheme as _, Appearance, Theme};

pub(crate) fn normalize_newlines(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\r') {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            normalized.push('\n');
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else {
            normalized.push(ch);
        }
    }
    std::borrow::Cow::Owned(normalized)
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum EnterBehavior {
    #[default]
    Enter,
    CmdIfMultiline,
    CmdAlways,
}

impl EnterBehavior {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "cmdIfMultiline" => Self::CmdIfMultiline,
            "cmdAlways" => Self::CmdAlways,
            _ => Self::Enter,
        }
    }

    fn inserts_newline(self, content: &str) -> bool {
        self == Self::CmdAlways || self == Self::CmdIfMultiline && content.contains('\n')
    }
}

impl gpui::Global for EnterBehavior {}
actions!(
    composer,
    [
        /// Send the prompt (Enter).
        Submit,
        /// Home composer only (`composer.submitInBackground`): send the prompt and stay on Home.
        SubmitInBackground,
        ModifiedSubmit,
        Stop,
        RequestFullAccess,
        /// Insert a line break (Shift+Enter).
        Newline,
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        LineStart,
        LineEnd,
        /// Option+Arrow: the caret moves by word; Shift extends the selection.
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        /// Option+Backspace, Option+Delete, Command+Backspace.
        DeleteWordBackward,
        DeleteWordForward,
        DeleteToLineStart,
        /// Close the open menu.
        Dismiss,
        CompleteCommand,
        NextCommand,
        PreviousCommand,
    ]
);

mod context_usage;
pub use context_usage::ContextUsage;
mod attach;
pub(crate) use attach::codex_home;
mod chat_shell;
pub mod computer_use;
pub mod dictation;
mod diff_comments;
mod editor;
pub use diff_comments::DiffCommentsChanged;
mod footer;
mod requests;
pub(crate) mod menu {
    //! Focus rules for the recorded Radix menus: arrows stop at the ends, Escape returns
    //! focus to the trigger, and Enter or Space activates the focused item.

    use gpui::{App, BoxShadow, FocusHandle, StyleRefinement, Styled, Window, point, px, rgb};
    use std::time::{Duration, Instant};

    pub(super) struct MenuFocus {
        pub content: FocusHandle,
        pub items: Vec<FocusHandle>,
        pub trigger: FocusHandle,
        search: String,
        last_typed: Option<Instant>,
    }

    impl MenuFocus {
        pub fn new(cx: &mut App) -> Self {
            Self {
                content: cx.focus_handle(),
                items: Vec::new(),
                trigger: cx.focus_handle(),
                search: String::new(),
                last_typed: None,
            }
        }

        pub fn prepare(&mut self, count: usize, cx: &mut App) {
            self.items.resize_with(count, || cx.focus_handle());
            self.search.clear();
            self.last_typed = None;
        }

        pub fn focused_item(&self, window: &Window) -> Option<usize> {
            self.items.iter().position(|item| item.is_focused(window))
        }

        pub fn move_focus(&self, forward: bool, window: &mut Window, cx: &mut App) {
            if self.items.is_empty() {
                return;
            }
            let last = self.items.len() - 1;
            let index = match self.focused_item(window) {
                Some(i) if forward => (i + 1).min(last),
                Some(i) => i.saturating_sub(1),
                None if forward => 0,
                None => last,
            };
            self.items[index].focus(window, cx);
        }

        pub fn searching(&self) -> bool {
            self.last_typed
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        }

        pub fn typeahead(&mut self, key: &str, labels: &[&str], window: &mut Window, cx: &mut App) {
            if !self.searching() {
                self.search.clear();
            }
            self.search.extend(key.chars().flat_map(char::to_lowercase));
            self.last_typed = Some(Instant::now());
            let Some(first) = self.search.chars().next() else {
                return;
            };
            // Repeating one letter cycles through matching items, as in Radix.
            let query = if self.search.chars().all(|c| c == first) {
                &self.search[..first.len_utf8()]
            } else {
                &self.search
            };
            let start = self
                .focused_item(window)
                .map_or(0, |i| i + usize::from(query.chars().count() == 1));
            if let Some(index) = (0..labels.len())
                .map(|n| (start + n) % labels.len())
                .find(|&i| {
                    let mut label = labels[i].chars().flat_map(char::to_lowercase);
                    query.chars().all(|c| label.next() == Some(c))
                })
            {
                self.items[index].focus(window, cx);
            }
        }

        pub fn restore(&self, window: &mut Window, cx: &mut App) {
            self.trigger.focus(window, cx);
        }
    }

    pub(crate) fn trigger_focus(style: StyleRefinement) -> StyleRefinement {
        style.shadow(vec![BoxShadow {
            color: rgb(0x3a83f7).into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(2.),
            inset: false,
        }])
    }
}
mod mention;
mod menus;
mod slash;
pub use slash::SlashCommand;

const KEY_CONTEXT: &str = "Composer";
const REQUEST_CONTEXT: &str = "ComposerRequest";
const REQUEST_FORM_CONTEXT: &str = "ComposerRequestForm";
/// `@container composer-footer` (the card's width, CSS px) at or below which the footer hides
/// its `collapse=sm` labels (permissions) and its `collapse=xs` labels (model and effort).
const SM_WIDTH: f32 = 440.;
const XS_WIDTH: f32 = 300.;
/// Card width at or below which the Home utility bar hides its chip labels: its own
/// `composer-footer` container (the card less 26 px rail inset and 8 px padding) reaches
/// Electron's 475 px `collapse=sm|xs` limit.
const UTILITY_WIDTH: f32 = 509.;
const FONT_SIZE: Pixels = px(14.);
const LINE_HEIGHT: Pixels = px(20.);
/// Smallest content height the textarea grip resizes to: the original's `min-h-24` (96 px)
/// border box less its 1 px borders and 8 px vertical padding.
const MIN_TEXTAREA_CONTENT_HEIGHT: Pixels = px(78.);

/// Blink's `DrawPlatformResizerImage` at 2x: two unantialiased 2 px strokes in 60% black, each
/// repeated 2 device px lower in 60% white. Each stroke starts at its top row's 1 px tip, runs
/// down-left 3 px wide, and ends 2 px wide on the row 7 device px above the border box's bottom.
/// shortcut: the device pattern is measured at scale 2; derive Blink's geometry if 1x matters.
fn paint_resizer(bounds: Bounds<Pixels>, window: &mut Window) {
    let scale = window.scale_factor();
    let (right, bottom) = (
        (f32::from(bounds.right()) * scale).round(),
        (f32::from(bounds.bottom()) * scale).round(),
    );
    for (top, color) in [
        (-21, gpui::black()),
        (-14, gpui::black()),
        (-19, gpui::white()),
        (-12, gpui::white()),
    ] {
        for row in top..=-7 {
            let left = -7 - (row - top);
            let (x, width) = match row {
                _ if row == top => (left, 1),
                -7 => (left + 1, 2),
                _ => (left, 3),
            };
            window.paint_quad(fill(
                Bounds::new(
                    point(
                        px((right + x as f32) / scale),
                        px((bottom + row as f32) / scale),
                    ),
                    size(px(width as f32 / scale), px(1. / scale)),
                ),
                color.opacity(0.6),
            ));
        }
    }
}

/// Composer keys (inventory §7.2): Enter sends, Shift+Enter inserts a newline. Edit menu
/// commands (`crate::Cut`, `Copy`, `Paste`, `SelectAll`) are bound by the app.
pub fn bind_keys(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("enter", Submit, c),
        KeyBinding::new("cmd-enter", ModifiedSubmit, c),
        KeyBinding::new("escape", Dismiss, Some(REQUEST_CONTEXT)),
        KeyBinding::new("cmd-enter", Submit, Some(REQUEST_FORM_CONTEXT)),
        KeyBinding::new("ctrl-enter", Submit, Some(REQUEST_FORM_CONTEXT)),
        KeyBinding::new("shift-enter", Newline, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("home", LineStart, c),
        KeyBinding::new("end", LineEnd, c),
        KeyBinding::new("cmd-left", LineStart, c),
        KeyBinding::new("cmd-right", LineEnd, c),
        KeyBinding::new("alt-left", WordLeft, c),
        KeyBinding::new("alt-right", WordRight, c),
        KeyBinding::new("alt-shift-left", SelectWordLeft, c),
        KeyBinding::new("alt-shift-right", SelectWordRight, c),
        KeyBinding::new("alt-backspace", DeleteWordBackward, c),
        KeyBinding::new("alt-delete", DeleteWordForward, c),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, c),
        KeyBinding::new("escape", Dismiss, c),
        KeyBinding::new("tab", CompleteCommand, c),
        KeyBinding::new("ctrl-n", NextCommand, c),
        KeyBinding::new("ctrl-p", PreviousCommand, c),
    ]);
}

/// Efforts the model picker slider offers: the `enabled-reasoning-efforts` setting default
/// (`low medium high xhigh ultra`) intersected with the model's supported efforts.
const ENABLED_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "ultra"];

/// `composer.mode.local.reasoning.*.label`.
pub fn effort_label(effort: &str) -> &str {
    match effort {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Light",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        "ultra" => "Ultra",
        "persistent" => "Persistent",
        other => other,
    }
}

/// The original's model name formatter: `GPT-6-Astra` → `GPT-6 Astra`, `gpt-5.6-sol` →
/// `GPT-5.6 Sol`.
pub fn model_label(name: &str) -> String {
    let trimmed = name.trim_start().to_lowercase();
    if !trimmed.starts_with("gpt") {
        return name.into();
    }
    let versioned = trimmed
        .strip_prefix("gpt-")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    let sep = if versioned { " " } else { "-" };
    let label = name
        .split(' ')
        .map(|word| {
            word.split('-')
                .enumerate()
                .map(|(i, part)| match part.to_lowercase().as_str() {
                    "gpt" => "GPT".to_string(),
                    "oai" => "OAI".to_string(),
                    _ if i > 0 => {
                        let mut chars = part.chars();
                        chars
                            .next()
                            .map(|c| c.to_uppercase().chain(chars).collect())
                            .unwrap_or_default()
                    }
                    _ => part.to_string(),
                })
                .collect::<Vec<_>>()
                .join(sep)
        })
        .collect::<Vec<_>>()
        .join(" ");
    match label.strip_prefix("GPT ") {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => format!("GPT-{rest}"),
        _ => label,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FieldStyle {
    Settings,
    Form,
    Compact,
    /// A settings-row value: frameless, right-aligned, secondary text.
    Inline,
    /// Like `Inline`, but left-aligned in primary text.
    Dialog,
}

/// One `model/list` entry as the picker shows it.
#[derive(Clone)]
struct ModelOption {
    id: String,
    label: SharedString,
    efforts: Vec<String>,
}

impl ModelOption {
    fn new(m: &Model) -> Self {
        let efforts = m
            .supported_reasoning_efforts
            .iter()
            .filter_map(|e| e["reasoningEffort"].as_str())
            .filter(|e| ENABLED_EFFORTS.contains(e))
            .map(String::from)
            .collect();
        Self {
            id: m.id.clone(),
            label: model_label(&m.display_name).into(),
            efforts,
        }
    }
}

/// Approval mode of the permissions dropdown (`composer.permissionsDropdown.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Ask,
    ApproveForMe,
    FullAccess,
    Custom,
}

impl Permission {
    const ALL: [Self; 4] = [
        Self::Ask,
        Self::ApproveForMe,
        Self::FullAccess,
        Self::Custom,
    ];

    /// The preset `config/read` matches; anything else is `Custom (config.toml)`.
    pub fn from_config(config: &Value) -> Self {
        match (
            config["approval_policy"].as_str(),
            config["sandbox_mode"].as_str(),
            config["approvals_reviewer"].as_str(),
        ) {
            (Some("on-request"), Some("workspace-write"), Some(r)) if r != "user" => {
                Self::ApproveForMe
            }
            (Some("on-request"), Some("workspace-write"), _) => Self::Ask,
            (Some("never"), Some("danger-full-access"), _) => Self::FullAccess,
            _ => Self::Custom,
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::Ask => "Ask for approval",
            Self::ApproveForMe => "Approve for me",
            Self::FullAccess => "Full access",
            Self::Custom => "Custom",
        }
    }

    fn option_label(self) -> &'static str {
        match self {
            Self::Custom => "Custom (config.toml)",
            other => other.short_label(),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Ask => "Always ask to edit external files and use the internet",
            Self::ApproveForMe => "Only ask for actions detected as potentially unsafe",
            Self::FullAccess => "Unrestricted access to the internet and any file on your computer",
            Self::Custom => "Uses permissions defined in config.toml",
        }
    }
}

/// What the composer sends: model input or a confirmed shell command.
#[derive(Clone, Debug)]
pub struct Submission {
    pub text: String,
    /// Set only after the user confirms the shell command.
    pub shell_command: bool,
    pub attachments: Vec<PathBuf>,
    /// File references added as context, not image uploads.
    pub context_files: Vec<PathBuf>,
    pub model: String,
    pub effort: Option<String>,
    pub permission: Permission,
    /// Fast mode (bolt toggle in the model picker).
    pub fast: bool,
    /// Plan mode (`collaborationMode.mode` `plan`).
    pub plan: bool,
    /// `composer.submitInBackground`: create the chat and start its turn without opening it.
    pub background: bool,
}

/// Enter in a single-line settings form field.
pub struct SettingsFieldSubmit;

impl gpui::EventEmitter<SettingsFieldSubmit> for Composer {}

#[derive(Debug, PartialEq)]
struct RequestIdentity {
    id: Value,
    thread: String,
}

#[derive(Default)]
struct RequestDraft {
    fields: Vec<Entity<Composer>>,
    selected: Vec<Vec<Value>>,
    accept_focus: Option<FocusHandle>,
    index: usize,
    error: Option<String>,
}

/// A pending server request, shown above the composer with its connection request id.
#[derive(Clone, Debug)]
pub struct Approval {
    title: SharedString,
    /// Command line or edited paths.
    detail: Option<String>,
    reason: Option<String>,
    /// Session-wide approve label (`acceptForSession`).
    scoped_label: SharedString,
    permissions: Option<Value>,
    form: Option<requests::Form>,
    computer_use: Option<computer_use::ComputerUseApproval>,
    identity: Rc<RequestIdentity>,
}

impl Approval {
    pub fn command(id: Value, req: &CommandApprovalRequest) -> Self {
        Self {
            title: "Allow ChatGPT to run this command?".into(),
            detail: req.command.clone(),
            reason: req.reason.clone(),
            scoped_label: "Allow similar commands".into(),
            permissions: None,
            form: None,
            computer_use: None,
            identity: Rc::new(RequestIdentity {
                id,
                thread: req.thread_id.clone(),
            }),
        }
    }

    /// `paths`: the files of the pending `fileChange` item.
    pub fn file_change(id: Value, req: &FileChangeApprovalRequest, paths: &[String]) -> Self {
        let files = if paths.len() == 1 {
            "the following file"
        } else {
            "the following files"
        };
        Self {
            title: format!("Allow ChatGPT to edit {files}?").into(),
            detail: (!paths.is_empty()).then(|| paths.join("\n")),
            reason: req.reason.clone(),
            scoped_label: "Allow all edits".into(),
            permissions: None,
            form: None,
            computer_use: None,
            identity: Rc::new(RequestIdentity {
                id,
                thread: req.thread_id.clone(),
            }),
        }
    }

    pub fn permissions(id: Value, req: &PermissionsApprovalRequest) -> Self {
        let mut actions = Vec::new();
        if req.permissions["network"]["enabled"] == true {
            actions.push("connect to the internet".to_owned());
        }
        if let Some(files) = req.permissions["fileSystem"].as_object() {
            for (access, verb) in [("read", "view"), ("write", "edit")] {
                if let Some(paths) = files.get(access).and_then(Value::as_array) {
                    let paths: Vec<_> = paths.iter().filter_map(Value::as_str).collect();
                    if !paths.is_empty() {
                        actions.push(format!("{verb} the contents of {}", paths.join(", ")));
                    }
                }
            }
            if let Some(entries) = files.get("entries").filter(|v| !v.is_null()) {
                actions.push(format!("access these files: {entries}"));
            }
        }
        Self {
            title: if actions.is_empty() {
                "Allow ChatGPT to use these permissions?".into()
            } else {
                format!("Allow ChatGPT to {}?", actions.join(" and ")).into()
            },
            detail: None,
            reason: req.reason.clone(),
            scoped_label: "Allow for this session".into(),
            permissions: Some(req.permissions.clone()),
            form: None,
            computer_use: None,
            identity: Rc::new(RequestIdentity {
                id,
                thread: req.thread_id.clone(),
            }),
        }
    }

    fn response(&self, decision: ApprovalDecision) -> Value {
        if let Some(request) = &self.computer_use {
            use computer_use::ComputerUseChoice;
            let choice = match decision {
                ApprovalDecision::Accept if request.allow_session => {
                    ComputerUseChoice::Conversation
                }
                ApprovalDecision::Accept => ComputerUseChoice::Once,
                ApprovalDecision::AcceptForSession => ComputerUseChoice::Always,
                ApprovalDecision::Decline => ComputerUseChoice::Decline,
                ApprovalDecision::Cancel => ComputerUseChoice::Cancel,
            };
            return request
                .response(choice)
                .unwrap_or_else(|| serde_json::json!({"action": "cancel"}));
        }
        let Some(permissions) = &self.permissions else {
            return serde_json::json!({ "decision": decision });
        };
        let granted = matches!(
            decision,
            ApprovalDecision::Accept | ApprovalDecision::AcceptForSession
        );
        let permissions = permissions
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, value)| {
                granted && matches!(key.as_str(), "network" | "fileSystem") && !value.is_null()
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        serde_json::json!({
            "permissions": permissions,
            "scope": if decision == ApprovalDecision::AcceptForSession { "session" } else { "turn" }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Permissions,
    Effort,
    Models,
    Comments,
}

/// Index of the `Speed` sub trigger among the effort menu's items.
const SPEED_ITEM: usize = 1;
/// The `Speed` submenu's items: title and description.
const SPEEDS: [(&str, &str); 3] = [
    ("Standard", "Default speed"),
    ("Fast", "1.5x speed, more usage"),
    (
        "Ultrafast",
        "The fastest available responses for latency-sensitive work.",
    ),
];

type SubmitHandler = Rc<dyn Fn(&Submission, &mut Window, &mut App)>;
type CancelEditHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type ApprovalHandler = Rc<dyn Fn(Value, String, Value, &mut Window, &mut App)>;
/// `(model id, effort)` after the user picks them.
type ModelHandler = Rc<dyn Fn(&str, &str, &mut App)>;
type SettingsPasteHandler = Rc<dyn Fn(&str, &mut App) -> bool>;

/// What a queued row's buttons and `Queued message actions` menu ask of the app.
#[derive(Clone)]
pub enum QueueAction {
    Steer(String),
    Retry(String),
    Delete(String),
    /// `Edit message`: the queued submission with this id returns to the composer.
    Edit(String),
    /// `Open in side chat`: the queued submission with this id continues in a side chat.
    SideChat(String),
    /// `Turn off queueing` / `Turn on queueing`: the new `Follow-up behavior` is queue (`true`).
    Queueing(bool),
}

impl gpui::EventEmitter<QueueAction> for Composer {}

/// One row of the queue tray: the app-server's submission and the text it shows.
pub struct QueuedMessage {
    pub submission: QueuedSubmission,
    pub text: SharedString,
    pub failed: bool,
    key: SharedString,
}

impl QueuedMessage {
    fn new(submission: QueuedSubmission, failed: bool) -> Self {
        let text = submission
            .input
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<String>();
        Self {
            key: submission.id.clone().into(),
            submission,
            text: text.trim_end().to_owned().into(),
            failed,
        }
    }
}

const QUEUE_ACTIONS: &str = "Queued message actions";
const QUEUE_EDIT: &str = "queuedEdit";
const QUEUE_SIDE_CHAT: &str = "queuedSideChat";
const QUEUE_SWITCH: &str = "queuedQueueing";

#[derive(Clone, Copy)]
struct EmbeddedField {
    font_size: Pixels,
    line_height: Pixels,
}
/// A composer's unsent input, carried to its replacement ([`Composer::take_draft`]).
pub struct Draft {
    content: String,
    mentions: Vec<mention::Mention>,
    attachments: Vec<attach::Attachment>,
    diff_comments: Vec<crate::screens::diff::panel::comments::Comment>,
}

pub struct Composer {
    bridge: Option<std::sync::Arc<bridge::Bridge>>,
    focus_handle: FocusHandle,
    placeholder: SharedString,
    /// Docked under a thread: `_ComposerLayoutRoot` paints the card and `_ComposerLayoutBody`
    /// (the content) is a `will-change: opacity` layer. On Home the body paints the card.
    docked: bool,
    shell_commands: bool,
    content: String,
    /// Mention chips in `content`, in order.
    mentions: Vec<mention::Mention>,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    history: editor::History,
    /// The right-click menu (Electron's native editable-field menu).
    context_menu: Option<crate::screens::thread::view::menu::ContextMenu>,
    context_spelling: Option<(Range<usize>, String)>,
    context_image: Option<crate::image_context::ImageContext>,
    is_selecting: bool,
    /// Shaped lines and editor bounds of the last paint, for hit testing.
    lines: Vec<WrappedLine>,
    bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    attachments: Vec<attach::Attachment>,
    /// The attachments of the last submission, until the next one: [`Self::restore_submission`]
    /// puts them back when starting the chat failed.
    submitted: Vec<attach::Attachment>,
    diff_comments: Vec<crate::screens::diff::panel::comments::Comment>,
    submitted_diff_comments: Vec<crate::screens::diff::panel::comments::Comment>,
    comments_close: Option<Task<()>>,
    pending_attachments: usize,
    attachment_generation: u64,
    attachment_error: Option<&'static str>,
    /// `Unable to send message` after a submit that uploads block.
    blocked: Option<Entity<Confirmation>>,
    /// A file drag is over the composer's drop target (`data-composer-drag-active`).
    drag_active: bool,
    models: Vec<ModelOption>,
    model: usize,
    effort: Option<String>,
    default_selection: (usize, Option<String>),
    fast: bool,
    /// Plan mode: the next turns run in the `plan` collaboration mode.
    plan: bool,
    permission: Permission,
    menu: Option<Menu>,
    slash: slash::Slash,
    menu_focus: menu::MenuFocus,
    /// The `Speed` submenu of the effort menu: open state, its items' focus, and the pending
    /// hover-open timer (Radix opens a sub trigger 100 ms after the pointer enters it).
    speed_open: bool,
    speed_focus: menu::MenuFocus,
    speed_timer: Option<Task<()>>,
    permissions_focus: FocusHandle,
    model_focus: FocusHandle,
    comments_focus: FocusHandle,
    /// The card's (`composer-footer` container's) width, CSS px: see `SM_WIDTH`, `XS_WIDTH`.
    footer_width: f32,
    menu_trigger_hovered: bool,
    approval: Option<Approval>,
    computer_use_icon: computer_use::IconCache,
    request_draft: RequestDraft,
    request_drafts: Vec<(Rc<RequestIdentity>, RequestDraft)>,
    request_focus_pending: bool,
    request_compact: bool,
    /// The sign-in `input`'s text color (set by [`Self::set_compact_field`]); its placeholder
    /// is `text-tertiary` at full opacity.
    field_text: Option<Hsla>,
    request_number: bool,
    request_label: Option<SharedString>,
    secret: bool,
    request_parent: Option<(gpui::WeakEntity<Composer>, Rc<RequestIdentity>, usize)>,
    running: bool,
    /// The open chat's context window use, while `Show context window usage` is on.
    context_usage: Option<ContextUsage>,
    /// Escape while a turn runs arms the stop for 2 s (the Stop button reads `Esc`); a second
    /// Escape in that window interrupts the turn. The task clears the arm.
    stop_armed: Option<Task<()>>,
    /// `Follow-up behavior` is `Queue`: a submit while a turn runs queues on the app-server
    /// instead of steering the turn.
    queue_mode: bool,
    /// The app-server's queued follow-ups of the open thread, shown in a tray over the card.
    queued: Vec<QueuedMessage>,
    /// The app can continue a queued row in a side chat: the row's menu offers `Open in side
    /// chat`, as the original does when its host passes `onOpenSideChatMessage`.
    side_chat: bool,
    queue_pending: bool,
    on_submit: Option<SubmitHandler>,
    on_cancel_edit: Option<CancelEditHandler>,
    edit_pending: bool,
    on_approval: Option<ApprovalHandler>,
    on_model: Option<ModelHandler>,
    plain_field: Option<bool>,
    field_style: FieldStyle,
    hotkey_controls: Option<AnyView>,
    document_field: bool,
    mini: bool,
    search_field: bool,
    field_min_height: Pixels,
    field_scroll: Option<ScrollHandle>,
    field_autoscroll: bool,
    /// The grip drag of a plain multiline field: pointer y and field height at its start.
    field_resize: Option<(Pixels, Pixels)>,
    dictation: Option<dictation::Active>,
    dictation_error: Option<String>,
    embedded_field: Option<EmbeddedField>,
    on_settings_paste: Option<SettingsPasteHandler>,
    /// Script and command fields use the original code textarea's monospace text.
    monospace: bool,
    /// A textarea in the original settings input style: strong border, unfaded placeholder.
    input_style: bool,
    /// The original's 28 px settings input (`h-7`, 8 px inline padding).
    small_input: bool,
}

impl Composer {
    /// Home composer: placeholder `Do anything`.
    pub fn home(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_editor_scroll(Self::new("Do anything", false, cx))
    }

    /// Thread composer, docked at the bottom of a thread: placeholder `Work with Codex`.
    pub fn thread(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_editor_scroll(Self::new("Work with Codex", true, cx))
    }

    /// Composer under a scheduled task's thread: placeholder `Reply about this task…`.
    pub fn automation(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_editor_scroll(Self::new("Reply about this task…", true, cx))
    }

    /// Gives the card's editor the scroll handle that keeps the caret visible once it
    /// reaches its maximum height.
    fn with_editor_scroll(mut self) -> Self {
        self.field_scroll = Some(ScrollHandle::new());
        self
    }

    fn new(placeholder: &'static str, docked: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            bridge: cx
                .try_global::<crate::screens::settings::AppServer>()
                .map(|server| server.0.clone()),
            placeholder: placeholder.into(),
            docked,
            shell_commands: false,
            content: String::new(),
            mentions: Vec::new(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            history: editor::History::default(),
            context_menu: None,
            context_spelling: None,
            context_image: None,
            is_selecting: false,
            lines: Vec::new(),
            bounds: None,
            line_height: LINE_HEIGHT,
            attachments: Vec::new(),
            submitted: Vec::new(),
            diff_comments: Vec::new(),
            submitted_diff_comments: Vec::new(),
            comments_close: None,
            pending_attachments: 0,
            attachment_generation: 0,
            attachment_error: None,
            blocked: None,
            drag_active: false,
            models: Vec::new(),
            model: 0,
            effort: Some("medium".into()),
            default_selection: (0, Some("medium".into())),
            fast: false,
            plan: false,
            permission: Permission::Custom,
            menu: None,
            slash: slash::Slash::default(),
            menu_focus: menu::MenuFocus::new(cx),
            speed_open: false,
            speed_focus: menu::MenuFocus::new(cx),
            speed_timer: None,
            permissions_focus: cx.focus_handle().tab_stop(true),
            model_focus: cx.focus_handle().tab_stop(true),
            comments_focus: cx.focus_handle().tab_stop(true),
            footer_width: f32::INFINITY,
            menu_trigger_hovered: false,
            approval: None,
            computer_use_icon: Default::default(),
            request_draft: RequestDraft::default(),
            request_drafts: Vec::new(),
            request_focus_pending: false,
            request_compact: false,
            field_text: None,
            request_number: false,
            request_label: None,
            secret: false,
            request_parent: None,
            running: false,
            context_usage: None,
            stop_armed: None,
            queue_mode: false,
            queued: Vec::new(),
            side_chat: false,
            queue_pending: false,
            on_submit: None,
            on_cancel_edit: None,
            edit_pending: false,
            on_approval: None,
            on_model: None,
            plain_field: None,
            field_style: FieldStyle::Settings,
            hotkey_controls: None,
            document_field: false,
            mini: false,
            search_field: false,
            field_min_height: px(48.),
            field_scroll: None,
            field_autoscroll: false,
            field_resize: None,
            dictation: None,
            dictation_error: None,
            embedded_field: None,
            on_settings_paste: None,
            monospace: false,
            input_style: false,
            small_input: false,
        }
    }

    pub fn bind_connection(&mut self, bridge: std::sync::Arc<bridge::Bridge>) {
        self.bridge = Some(bridge);
    }

    pub fn set_hotkey_home(
        &mut self,
        controls: AnyView,
        placeholder: String,
        cx: &mut Context<Self>,
    ) {
        self.hotkey_controls = Some(controls);
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn set_mini(&mut self, cx: &mut Context<Self>) {
        self.mini = true;
        self.search_field = true;
        self.placeholder = "Ask".into();
        cx.notify();
    }

    pub fn settings_field(
        placeholder: &'static str,
        value: &str,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::new(placeholder, false, cx);
        editor.plain_field = Some(multiline);
        if multiline {
            editor.field_scroll = Some(ScrollHandle::new());
            editor.field_min_height = MIN_TEXTAREA_CONTENT_HEIGHT;
        }
        editor.set_text(value, cx);
        editor
    }

    pub fn form_field(
        placeholder: &'static str,
        value: &str,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut field = Self::settings_field(placeholder, value, multiline, cx);
        field.field_style = FieldStyle::Form;
        field
    }

    pub fn settings_form_field(
        label: &'static str,
        placeholder: &'static str,
        value: &str,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut field = Self::settings_field(placeholder, value, multiline, cx);
        field.field_style = FieldStyle::Compact;
        field.request_compact = true;
        field.request_label = Some(label.into());
        field.line_height = px(18.571_428);
        field
    }

    pub fn on_settings_paste(&mut self, handler: impl Fn(&str, &mut App) -> bool + 'static) {
        self.on_settings_paste = Some(Rc::new(handler));
    }

    pub fn inline_field(placeholder: &'static str, value: &str, cx: &mut Context<Self>) -> Self {
        let mut field = Self::settings_field(placeholder, value, false, cx);
        field.field_style = FieldStyle::Inline;
        field
    }

    /// A frameless single-line field for a dialog that draws its own frame, labelled `label`.
    pub fn dialog_field(
        label: &'static str,
        placeholder: &'static str,
        value: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut field = Self::settings_field(placeholder, value, false, cx);
        field.field_style = FieldStyle::Dialog;
        field.request_label = Some(label.into());
        field
    }

    pub fn search_field(placeholder: &'static str, cx: &mut Context<Self>) -> Self {
        let mut field = Self::settings_field(placeholder, "", false, cx);
        field.search_field = true;
        field
    }

    pub fn embedded_field(
        placeholder: &'static str,
        value: &str,
        text_size: Pixels,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::settings_field(placeholder, value, false, cx);
        editor.embedded_field = Some(EmbeddedField {
            font_size: text_size,
            line_height,
        });
        editor
    }

    pub fn with_field_min_height(mut self, height: Pixels) -> Self {
        self.field_min_height = height;
        self
    }

    pub fn input_style(mut self) -> Self {
        self.input_style = true;
        self
    }

    pub fn small_input(mut self) -> Self {
        self.small_input = true;
        self
    }

    pub fn monospace_field(mut self) -> Self {
        self.monospace = true;
        self
    }

    pub fn field_label(mut self, label: impl Into<SharedString>) -> Self {
        self.request_label = Some(label.into());
        self
    }

    pub fn set_document_style(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.document_field = enabled;
        if enabled {
            self.normalize_document();
        }
        cx.notify();
    }

    /// A settings textarea: a multiline plain field that is not the diff comment editor, which
    /// draws inside its comment card instead.
    fn textarea(&self) -> bool {
        self.plain_field == Some(true) && self.on_cancel_edit.is_none()
    }

    /// 13 px text and 10 px insets: a settings textarea, a sign-in field, or a compact request
    /// field that is not secret.
    fn compact_text(&self) -> bool {
        self.textarea() || (self.request_compact && (self.request_parent.is_none() || !self.secret))
    }

    fn editor_font_size(&self) -> Pixels {
        if self.document_field {
            px(14.)
        } else if let Some(field) = self.embedded_field {
            field.font_size
        } else if self.compact_text() {
            px(13.)
        } else {
            FONT_SIZE
        }
    }

    fn editor_line_height(&self) -> Pixels {
        if self.document_field {
            px(23.59375)
        } else if let Some(field) = self.embedded_field {
            field.line_height
        } else if self.search_field || self.textarea() {
            px(18.571_428)
        } else {
            self.line_height
        }
    }

    pub fn settings_search(placeholder: &'static str, cx: &mut Context<Self>) -> Self {
        let mut editor = Self::settings_field(placeholder, "", false, cx);
        editor.search_field = true;
        editor.request_compact = true;
        editor
    }
    pub fn settings_compact_field(
        placeholder: &'static str,
        value: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::settings_field(placeholder, value, false, cx);
        editor.request_compact = true;
        editor
    }
    pub fn field_text(&self) -> &str {
        &self.content
    }

    pub fn set_secret(&mut self, secret: bool, cx: &mut Context<Self>) {
        self.secret = secret;
        cx.notify();
    }

    pub fn set_field_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.request_label = Some(label.into());
        cx.notify();
    }

    pub(crate) fn set_compact_field(&mut self, text: Hsla, cx: &mut Context<Self>) {
        self.request_compact = true;
        self.field_text = Some(text);
        cx.notify();
    }

    pub fn message_edit(
        message: &str,
        cancel: impl Fn(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::new("Edit message", false, cx);
        editor.set_text(message, cx);
        editor.on_cancel_edit = Some(Rc::new(cancel));
        editor.focus_handle.focus(window, cx);
        editor
    }

    pub fn edit_failed(&mut self, cx: &mut Context<Self>) {
        self.edit_pending = false;
        cx.notify();
    }

    /// `data-composer-drag-active`: the card surface drops to 50% while a file drag is over
    /// the drop target. Stale after the drag ends: see [`Self::drag_active`].
    pub fn set_drag_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.drag_active != active {
            self.drag_active = active;
            cx.notify();
        }
    }

    /// A file drag is over the drop target now: the overlay and the surface switch show.
    pub fn drag_active(&self, cx: &App) -> bool {
        self.drag_active && cx.has_active_drag()
    }

    /// Models from `model/list` and the selection from `config/read` (`model`,
    /// `model_reasoning_effort`, approval settings).
    pub fn set_models(&mut self, models: &[Model], config: &Value, cx: &mut Context<Self>) {
        self.models = models
            .iter()
            .filter(|m| !m.hidden)
            .map(ModelOption::new)
            .collect();
        let configured = config["model"].as_str();
        self.model = self
            .models
            .iter()
            .position(|m| Some(m.id.as_str()) == configured)
            .or_else(|| {
                models
                    .iter()
                    .filter(|m| !m.hidden)
                    .position(|m| m.is_default)
            })
            .unwrap_or(0);
        self.effort = config["model_reasoning_effort"]
            .as_str()
            .map(String::from)
            .or_else(|| {
                let id = &self.models.get(self.model)?.id;
                let m = models.iter().find(|m| &m.id == id)?;
                Some(m.default_reasoning_effort.clone())
            });
        self.default_selection = (self.model, self.effort.clone());
        self.permission = Permission::from_config(config);
        self.queue_mode = config["desktop"]["followUpQueueMode"] == "queue";
        if self.menu == Some(Menu::Models) {
            self.menu_focus.prepare(self.models.len(), cx);
        }
        cx.notify();
    }

    pub fn set_model_and_effort(&mut self, model: &str, effort: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.models.iter().position(|option| option.id == model) {
            self.model = index;
            self.effort = Some(effort.into());
            cx.notify();
        }
    }

    fn displayed_effort(&self) -> &str {
        self.effort.as_deref().unwrap_or("medium")
    }

    pub fn on_submit(&mut self, f: impl Fn(&Submission, &mut Window, &mut App) + 'static) {
        self.on_submit = Some(Rc::new(f));
    }

    pub fn on_approval(
        &mut self,
        f: impl Fn(Value, String, Value, &mut Window, &mut App) + 'static,
    ) {
        self.on_approval = Some(Rc::new(f));
    }

    /// Called when the user picks a model or an effort in the picker.
    pub fn on_model(&mut self, f: impl Fn(&str, &str, &mut App) + 'static) {
        self.on_model = Some(Rc::new(f));
    }

    /// The approval card's request, while one shows.
    pub fn approval(&self) -> Option<&Approval> {
        self.approval.as_ref()
    }

    pub(crate) fn is_computer_use_request(&self) -> bool {
        self.approval
            .as_ref()
            .is_some_and(|approval| approval.computer_use.is_some())
    }

    pub fn restart_ready(&self) -> bool {
        self.content.is_empty()
            && self.diff_comments.is_empty()
            && self.attachments.is_empty()
            && self.pending_attachments == 0
            && self.marked_range.is_none()
            && !self.running
            && self.approval.is_none()
            && self.queued.is_empty()
            && !self.edit_pending
            && self.on_cancel_edit.is_none()
            && self.menu.is_none()
            && self.context_menu.is_none()
            && !self.slash.open
            && !self.is_selecting
            && !self.drag_active
    }

    /// Icon-only footer for a known narrow composer; the editor sets the measured width.
    pub fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) {
        self.footer_width = if compact { 0. } else { f32::INFINITY };
        cx.notify();
    }

    pub fn set_approval(&mut self, approval: Option<Approval>, cx: &mut Context<Self>) {
        let same_request = self
            .approval
            .as_ref()
            .map(|approval| approval.identity.as_ref())
            == approval.as_ref().map(|approval| approval.identity.as_ref());
        if !same_request {
            self.cancel_dictation(cx);
        }
        if !same_request
            && let Some(old) = &self.approval
            && old.form.is_some()
            && !self.request_draft.fields.is_empty()
        {
            self.request_drafts.push((
                old.identity.clone(),
                std::mem::take(&mut self.request_draft),
            ));
        }
        self.approval = approval;
        if !same_request {
            self.prepare_request(cx);
        }
        cx.notify();
    }

    pub fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        if self.running != running {
            self.running = running;
            self.stop_armed = None;
            cx.notify();
        }
    }

    pub fn set_queue_mode(&mut self, queue: bool, cx: &mut Context<Self>) {
        self.queue_mode = queue;
        cx.notify();
    }

    pub fn set_queued(&mut self, queued: Vec<QueuedSubmission>, cx: &mut Context<Self>) {
        let failed = self
            .queued
            .extract_if(.., |row| row.failed)
            .collect::<Vec<_>>();
        self.queued = queued
            .into_iter()
            .map(|submission| QueuedMessage::new(submission, false))
            .chain(failed)
            .collect();
        cx.notify();
    }

    pub fn set_failed_queued(&mut self, queued: Vec<QueuedSubmission>, cx: &mut Context<Self>) {
        self.queued.retain(|row| !row.failed);
        self.queued
            .extend(queued.into_iter().map(|s| QueuedMessage::new(s, true)));
        cx.notify();
    }

    pub fn queued(&self) -> &[QueuedMessage] {
        &self.queued
    }

    pub fn set_side_chat(&mut self, available: bool) {
        self.side_chat = available;
    }

    /// `Edit message`: the queued draft returns to the editor. A draft already there stays
    /// below it ([`Self::restore_text`]) and the images join the current attachments.
    pub fn load_queued(
        &mut self,
        text: &str,
        attachments: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !text.is_empty() {
            self.restore_text(text, cx);
        }
        self.drop_files(attachments, cx);
        self.focus_handle.focus(window, cx);
    }

    /// A pick in the open context menu: the queue row's, else the editor's.
    fn activate_context(
        &mut self,
        id: &'static str,
        subject: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            QUEUE_EDIT => cx.emit(QueueAction::Edit(subject)),
            QUEUE_SIDE_CHAT => cx.emit(QueueAction::SideChat(subject)),
            QUEUE_SWITCH => cx.emit(QueueAction::Queueing(!self.queue_mode)),
            _ => self.activate_context_item(id, subject, window, cx),
        }
    }

    /// The row's `Queued message actions` menu (`composer.queuedMessage.more`): `Edit message`,
    /// `Open in side chat` where the app offers it, and the follow-up behavior switch, below its
    /// button with the right edges aligned.
    fn open_queue_menu(
        &mut self,
        id: &str,
        button: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::screens::thread::view::menu::{ContextMenu, Item};
        self.menu = None;
        self.slash.reset();
        let item = |id, label, icon: &'static [u8]| Item {
            icon: Some(icon),
            ..Item::new(id, label, true)
        };
        let mut items = vec![item(
            QUEUE_EDIT,
            "Edit message",
            include_bytes!("../../../assets/icons/pencil.svg"),
        )];
        if self.side_chat {
            items.push(item(
                QUEUE_SIDE_CHAT,
                "Open in side chat",
                include_bytes!("../../../assets/icons/side-chat.svg"),
            ));
        }
        items.push(item(
            QUEUE_SWITCH,
            if self.queue_mode {
                "Turn off queueing"
            } else {
                "Turn on queueing"
            },
            include_bytes!("../../../assets/icons/queue-handle.svg"),
        ));
        self.context_menu = Some(ContextMenu::dropdown(
            items,
            id.to_owned(),
            button,
            window,
            cx,
        ));
        cx.notify();
    }

    pub fn set_queue_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        self.queue_pending = pending;
        cx.notify();
    }

    pub fn remove_queued(&mut self, id: &str, cx: &mut Context<Self>) {
        self.queued.retain(|row| row.submission.id != id);
        cx.notify();
    }

    fn activate_queued(
        &mut self,
        action: QueueAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.queue_pending {
            self.focus_handle.focus(window, cx);
            cx.emit(action);
        }
    }

    fn queue_button(
        &self,
        action: QueueAction,
        label: &'static str,
        tooltip: &'static str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> crate::tooltip::Tooltip {
        let kind = match &action {
            QueueAction::Steer(_) => "queued-steer",
            QueueAction::Retry(_) => "queued-retry",
            _ => "queued-delete",
        };
        let key_action = action.clone();
        crate::tooltip::tooltip(
            div()
                .id(kind)
                .h(px(24.))
                .min_w(px(24.))
                .px(px(
                    if matches!(action, QueueAction::Steer(_) | QueueAction::Retry(_)) {
                        8.
                    } else {
                        0.
                    },
                ))
                .border_1()
                .border_color(gpui::transparent_black())
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(self.queue_pending)
                .focusable()
                .tab_index(0)
                .focus_visible(crate::focus::trigger_focus)
                .opacity(if self.queue_pending { 0.4 } else { 1. })
                .when(!self.queue_pending, |d| d.hover(|s| s.bg(t.ghost_hover)))
                .child(match action {
                    QueueAction::Steer(_) => div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(icon("queue-steer", 14., t.text_tertiary))
                        .child(text("Steer", 13., 18., 430., t.text_tertiary))
                        .into_any_element(),
                    QueueAction::Retry(_) => div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(icon("diff-refresh", 14., t.text_tertiary))
                        .child(text("Retry", 13., 18., 430., t.text_tertiary))
                        .into_any_element(),
                    _ => icon("queue-delete", 14., t.text_tertiary).into_any_element(),
                })
                .on_key_down(
                    cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.activate_queued(key_action.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.activate_queued(action.clone(), window, cx);
                })),
            tooltip,
            None,
            crate::tooltip::Side::Top,
        )
    }

    fn queue_menu_button(
        &self,
        id: String,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> crate::tooltip::Tooltip {
        let bounds = window.use_keyed_state(
            SharedString::from(format!(
                "queue-menu-bounds-{:?}-{id}",
                cx.entity().entity_id()
            )),
            cx,
            |_, _| Bounds::<Pixels>::default(),
        );
        let measured = bounds.clone();
        crate::tooltip::tooltip(
            div()
                .id(SharedString::from(format!("queued-actions-{id}")))
                .relative()
                .child(
                    gpui::canvas(
                        move |rect, _, cx| measured.update(cx, |stored, _| *stored = rect),
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .size(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .role(gpui::Role::Button)
                .aria_label(QUEUE_ACTIONS)
                .aria_disabled(self.queue_pending)
                .focusable()
                .tab_index(0)
                .focus_visible(crate::focus::trigger_focus)
                .opacity(if self.queue_pending { 0.4 } else { 1. })
                .when(!self.queue_pending, |d| d.hover(|s| s.bg(t.ghost_hover)))
                .child(icon("diff-more", 14., t.text_tertiary))
                // Enter would submit the draft: the key belongs to the button.
                .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                    }
                }))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                    if !this.queue_pending {
                        this.open_queue_menu(&id, *bounds.read(cx), window, cx);
                    }
                })),
            QUEUE_ACTIONS,
            None,
            crate::tooltip::Side::Top,
        )
    }

    /// The empty editor's text (`Work with Codex` under a thread, also while a turn runs:
    /// `Working…` belongs to the original's compact floating composer only).
    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    /// Whether the Home utility bar above this card hides its chip labels.
    pub fn utility_compact(&self) -> bool {
        self.footer_width <= UTILITY_WIDTH
    }

    pub(crate) fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// The submit button's accessible label (`composer.submitButtonTooltip.*`): `Stop` while a
    /// turn runs and the composer is empty, else the follow-up mode (`Queue` or `Steer`) while
    /// a turn runs, else `Send`. The original shows no tooltip on this button.
    pub fn send_label(&self) -> &'static str {
        let has_input = !self.content.is_empty()
            || !self.attachments.is_empty()
            || !self.diff_comments.is_empty();
        match (self.running, has_input, self.queue_mode) {
            (false, _, _) => "Send",
            (true, false, _) => "Stop",
            (true, true, true) => "Queue",
            (true, true, false) => "Steer",
        }
    }

    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
        self.dictation_error = None;
        self.content = if self.document_field {
            normalize_newlines(s).into_owned()
        } else {
            s.into()
        };
        self.mentions.clear();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        self.history = editor::History::default();
        self.context_spelling = None;
        self.context_image = None;
        self.slash.reset();
        self.end_search(cx);
        cx.notify();
    }

    /// Clears account-bound drafts and invalidates pending attachment work.
    pub fn clear_session(&mut self, cx: &mut Context<Self>) {
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        self.attachments.clear();
        self.diff_comments.clear();
        self.submitted_diff_comments.clear();
        self.pending_attachments = 0;
        self.attachment_error = None;
        self.context_menu = None;
        self.menu = None;
        self.set_text("", cx);
        self.queued.clear();
        self.set_running(false, cx);
        self.set_approval(None, cx);
    }

    /// Takes the unsent input (text, mention chips, attachments, diff comments) out of a
    /// composer about to be replaced, as when the app-server restarts.
    pub fn take_draft(&mut self) -> Draft {
        Draft {
            content: std::mem::take(&mut self.content),
            mentions: std::mem::take(&mut self.mentions),
            attachments: std::mem::take(&mut self.attachments),
            diff_comments: std::mem::take(&mut self.diff_comments),
        }
    }

    /// Puts a [`Composer::take_draft`] input into this composer.
    pub fn put_draft(&mut self, draft: Draft, cx: &mut Context<Self>) {
        self.set_text(&draft.content, cx);
        self.mentions = draft.mentions;
        self.attachments = draft.attachments;
        self.diff_comments = draft.diff_comments;
        cx.emit(DiffCommentsChanged(self.diff_comments.clone()));
    }

    pub fn restore_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.content.is_empty() {
            self.set_text(text, cx);
        } else {
            let restored = format!("{text}\n\n{}", self.content);
            self.set_text(&restored, cx);
        }
    }

    /// Puts a failed submission back: its text and its attachments, ahead of what was typed
    /// since (the draft stays whole when starting the chat was refused).
    pub fn restore_submission(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut restored = std::mem::take(&mut self.submitted);
        restored.append(&mut self.attachments);
        self.attachments = restored;
        let mut comments = std::mem::take(&mut self.submitted_diff_comments);
        comments.append(&mut self.diff_comments);
        self.diff_comments = comments;
        cx.emit(DiffCommentsChanged(self.diff_comments.clone()));
        self.restore_text(crate::screens::diff::panel::comments::prompt_body(text), cx);
    }

    /// Open the model picker (`Select effort`), as `⌘⇧M`-style commands and the menu do.
    pub fn open_model_picker(&mut self, cx: &mut Context<Self>) {
        self.menu = Some(Menu::Effort);
        self.menu_focus.prepare(3, cx);
        self.menu_focus.trigger = self.model_focus.clone();
        cx.notify();
    }

    fn current_model(&self) -> Option<&ModelOption> {
        self.models.get(self.model)
    }

    /// Makes model `index` the selection, keeping the effort when the model supports it.
    fn select_model(&mut self, index: usize) {
        self.model = index;
        let efforts = &self.models[index].efforts;
        let effort = self.effort.get_or_insert_with(|| "medium".into());
        if !efforts.contains(effort)
            && let Some(first) = efforts.first()
        {
            *effort = first.clone();
        }
    }

    /// Reports the picked model and effort to [`Self::on_model`].
    fn picked(&self, cx: &mut App) {
        if let (Some(f), Some(model)) = (self.on_model.clone(), self.current_model()) {
            f(&model.id, self.displayed_effort(), cx);
        }
    }

    /// Enter sends the draft only when the approval leaves the composer visible.
    fn enter(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if self.answer_request_parent(ApprovalDecision::Accept, window, cx) {
            return;
        }
        if let Some(multiline) = self.plain_field {
            if multiline {
                self.insert("\n", window, cx);
            } else if self.field_style == FieldStyle::Compact {
                cx.emit(SettingsFieldSubmit);
            } else {
                window.blur(cx);
            }
            return;
        }
        if self.approval.is_some() && !self.slash.open {
            let replaces_composer = self.approval.as_ref().is_some_and(|approval| {
                approval.form.is_some()
                    || approval.permissions.is_some()
                    || approval.computer_use.is_some()
            });
            self.request_action(ApprovalDecision::Accept, window, cx);
            if replaces_composer {
                return;
            }
        }
        let newline = cx
            .try_global::<EnterBehavior>()
            .is_some_and(|behavior| behavior.inserts_newline(&self.content));
        if newline && !self.slash.open {
            self.newline(&Newline, window, cx);
        } else {
            self.submit(&Submit, window, cx);
        }
    }

    fn modified_enter(&mut self, _: &ModifiedSubmit, window: &mut Window, cx: &mut Context<Self>) {
        if self.plain_field.is_some() {
            self.enter(&Submit, window, cx);
        } else {
            self.submit(&Submit, window, cx);
        }
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        self.send_draft(false, window, cx);
    }

    /// `composer.submitInBackground`: the original enables it only for the Home composer
    /// (`kind === "home"`), never in an open chat, and not over an approval or request form.
    fn submit_in_background(
        &mut self,
        _: &SubmitInBackground,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.docked && self.approval.is_none() && self.plain_field.is_none() {
            self.send_draft(true, window, cx);
        }
    }

    fn send_draft(&mut self, background: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked.is_some() {
            return;
        }
        if self.slash.open {
            self.activate_slash(window, cx);
            return;
        }
        if self.edit_pending || (!self.running && self.current_model().is_none()) {
            return;
        }
        if self.upload_status().is_some() {
            self.show_submit_blocked(window, cx);
            return;
        }
        if self.content.trim().is_empty()
            && self.attachments.is_empty()
            && self.diff_comments.is_empty()
        {
            return;
        }
        if self.shell_commands && self.content.starts_with('!') {
            self.confirm_shell(window, cx);
            return;
        }
        self.finish_send_draft(background, window, cx);
    }

    fn finish_send_draft(&mut self, background: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
        let submission = self.take_submission(background);
        if self.on_cancel_edit.is_some() {
            self.edit_pending = true;
        } else {
            self.content.clear();
            self.mentions.clear();
            self.take_diff_comments(cx);
            self.selected_range = 0..0;
            self.marked_range = None;
            self.menu = None;
            self.slash.reset();
            self.end_search(cx);
        }
        cx.notify();
        if let Some(f) = self.on_submit.clone() {
            f(&submission, window, cx);
        }
    }

    /// The original's `onBlockedSubmit`: `Unable to send message` with the block reason and `OK`;
    /// closing it focuses the composer.
    fn show_submit_blocked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui::AppContext as _;
        let Some(reason) = self.upload_status() else {
            return;
        };
        let blocked = cx.new(|cx| {
            Confirmation::new("Unable to send message", reason, "OK", window, cx).notice(window, cx)
        });
        cx.subscribe_in(&blocked, window, |this, _, _, window, cx| {
            this.blocked = None;
            this.focus_handle.focus(window, cx);
            cx.notify();
        })
        .detach();
        self.blocked = Some(blocked);
        cx.notify();
    }

    fn take_submission(&mut self, background: bool) -> Submission {
        self.submitted = std::mem::take(&mut self.attachments);
        let mut attachments = Vec::new();
        let mut context_files = Vec::new();
        for attachment in &self.submitted {
            if attachment.is_context_file() {
                context_files.push(attachment.path.clone());
            } else {
                attachments.push(attachment.path.clone());
            }
        }
        Submission {
            shell_command: self.shell_commands && self.content.starts_with('!'),
            text: if self.mentions.is_empty()
                && self.on_cancel_edit.is_none()
                && self.diff_comments.is_empty()
            {
                std::mem::take(&mut self.content)
            } else {
                self.prompt()
            },
            attachments,
            context_files,
            model: self
                .current_model()
                .map(|m| m.id.clone())
                .unwrap_or_default(),
            effort: self.effort.clone(),
            permission: self.permission,
            fast: self.fast,
            plan: self.plan,
            background,
        }
    }

    fn decide(&mut self, decision: ApprovalDecision, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
        if self.approval.is_none() {
            return;
        }
        let response = match self.request_response(decision, cx) {
            Ok(response) => response,
            Err((field, error)) => {
                self.request_draft.index = field;
                self.request_draft.error = Some(error);
                self.request_focus_pending = true;
                self.focus_request_field(window, cx);
                cx.notify();
                return;
            }
        };
        let approval = self.approval.take().unwrap();
        if response.is_null() {
            self.request_drafts.push((
                approval.identity.clone(),
                std::mem::take(&mut self.request_draft),
            ));
        } else {
            self.request_draft = RequestDraft::default();
        }
        self.focus_handle.focus(window, cx);
        self.menu = None;
        cx.notify();
        if let Some(f) = self.on_approval.clone() {
            f(
                approval.identity.id.clone(),
                approval.identity.thread.clone(),
                response,
                window,
                cx,
            );
        }
    }

    fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        self.slash.reset();
        self.close_speed();
        self.end_search(cx);
        if self.menu == Some(menu) {
            self.menu = None;
            self.menu_focus.restore(window, cx);
            cx.notify();
        } else {
            self.menu = Some(menu);
            let count = match menu {
                Menu::Permissions => Permission::ALL.len(),
                Menu::Models => self.models.len(),
                Menu::Effort => 3,
                Menu::Comments => self.diff_comments.len(),
            };
            self.menu_focus.prepare(count, cx);
            self.menu_focus.trigger = match menu {
                Menu::Permissions => self.permissions_focus.clone(),
                Menu::Comments => self.comments_focus.clone(),
                _ => self.model_focus.clone(),
            };
            self.menu_focus.content.focus(window, cx);
            cx.notify();
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.dictation.is_some() {
            self.cancel_dictation(cx);
            return;
        }
        if self.marked_range.is_some() {
            return;
        }
        if self.answer_request_parent(ApprovalDecision::Cancel, window, cx) {
            return;
        }
        if self.slash.open {
            self.close_slash(cx);
            return;
        }
        if let Some(menu) = self.menu.take() {
            // Escape returns focus to the trigger of the Radix menus (permissions, the
            // `Speed` submenu: f2c.json step 24); the model picker's close focuses the editor
            // (refs/interactions/s033.json).
            if matches!(menu, Menu::Permissions | Menu::Comments) || self.speed_open {
                self.menu_focus.restore(window, cx);
            } else {
                self.focus_handle.focus(window, cx);
            }
            self.close_speed();
            cx.notify();
        } else if self.approval.is_some() {
            // `approval.decline` (Escape), unless a menu has focus.
            let decision = if self
                .approval
                .as_ref()
                .is_some_and(|approval| approval.form.is_some())
            {
                ApprovalDecision::Cancel
            } else {
                ApprovalDecision::Decline
            };
            self.decide(decision, window, cx);
        } else if self.running {
            if self.stop_armed.take().is_some() {
                window.dispatch_action(Box::new(Stop), cx);
            } else {
                self.stop_armed = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.stop_armed = None;
                        cx.notify();
                    });
                }));
            }
            cx.notify();
        } else if !self.edit_pending
            && let Some(cancel) = self.on_cancel_edit.clone()
        {
            cancel(window, cx);
        } else if self.plain_field.is_some() {
            cx.propagate();
        }
    }

    /// The focus set the arrows move in: the open `Speed` submenu, else the menu.
    fn active_focus(&self) -> &menu::MenuFocus {
        if self.speed_open {
            &self.speed_focus
        } else {
            &self.menu_focus
        }
    }

    fn menu_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        match key {
            "down" | "up" => {
                self.active_focus()
                    .move_focus(event.keystroke.key == "down", window, cx)
            }
            "home" | "end" => {
                let focus = self.active_focus();
                let item = if key == "home" {
                    focus.items.first()
                } else {
                    focus.items.last()
                };
                if let Some(item) = item {
                    item.focus(window, cx);
                }
            }
            "tab" => {}
            // `ArrowRight` on the `Speed` sub trigger opens its submenu on the first item;
            // `ArrowLeft` in the submenu closes it and focuses the trigger.
            "right" if self.menu == Some(Menu::Effort) && !self.speed_open => {
                if self.menu_focus.focused_item(window) == Some(SPEED_ITEM) {
                    self.open_speed(window, cx);
                }
            }
            "left" if self.speed_open => {
                self.close_speed();
                self.menu_focus.items[SPEED_ITEM].focus(window, cx);
            }
            "space" if self.menu == Some(Menu::Permissions) && self.menu_focus.searching() => {
                self.menu_focus.typeahead(
                    " ",
                    &Permission::ALL.map(|p| p.option_label()),
                    window,
                    cx,
                );
            }
            "escape" => self.dismiss(&Dismiss, window, cx),
            "enter" | "space" if self.speed_open => {
                if let Some(index) = self.speed_focus.focused_item(window) {
                    self.pick_speed(index, window, cx);
                }
            }
            "enter" | "space" => {
                if let Some(index) = self.menu_focus.focused_item(window) {
                    self.activate_menu_item(index, window, cx);
                }
            }
            key if self.menu == Some(Menu::Permissions)
                && key.chars().count() == 1
                && !modifiers.control
                && !modifiers.platform
                && !modifiers.alt =>
            {
                self.menu_focus.typeahead(
                    key,
                    &Permission::ALL.map(|p| p.option_label()),
                    window,
                    cx,
                );
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Opens the `Speed` submenu with its first item focused (keyboard).
    fn open_speed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.speed_timer = None;
        self.speed_open = true;
        self.speed_focus.prepare(SPEEDS.len(), cx);
        self.speed_focus.items[0].focus(window, cx);
    }

    /// Opens the `Speed` submenu 100 ms after the pointer enters its trigger, which keeps
    /// focus (`hover`).
    pub(super) fn hover_speed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_focus.items[SPEED_ITEM].focus(window, cx);
        if self.speed_open || self.speed_timer.is_some() {
            return;
        }
        self.speed_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.speed_timer = None;
                this.speed_open = true;
                this.speed_focus.prepare(SPEEDS.len(), cx);
                cx.notify();
            });
        }));
    }

    fn close_speed(&mut self) {
        self.speed_timer = None;
        self.speed_open = false;
    }

    /// A speed picked in the submenu closes it and focuses the `Speed` trigger.
    pub(super) fn pick_speed(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // ponytail: Ultrafast shares the `fast` flag; add a tier when `turn/start` sends it.
        self.fast = index > 0;
        self.close_speed();
        self.menu_focus.items[SPEED_ITEM].focus(window, cx);
        cx.notify();
    }

    fn trigger_key(
        &mut self,
        menu: Menu,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
            self.toggle_menu(menu, window, cx);
            if self.menu.is_some() {
                self.menu_focus.move_focus(true, window, cx);
            }
            cx.stop_propagation();
        }
    }

    fn activate_menu_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.menu {
            Some(Menu::Permissions) => {
                let permission = Permission::ALL[index];
                self.dismiss(&Dismiss, window, cx);
                if permission == Permission::FullAccess {
                    window.dispatch_action(Box::new(RequestFullAccess), cx);
                } else {
                    self.permission = permission;
                }
            }
            Some(Menu::Models) => {
                self.select_model(index);
                self.toggle_menu(Menu::Effort, window, cx);
                self.picked(cx);
            }
            // The original's DOM order: `Select model`, `Speed` (sub trigger), `Reset`.
            Some(Menu::Effort) => match index {
                0 => self.toggle_menu(Menu::Models, window, cx),
                SPEED_ITEM => self.open_speed(window, cx),
                2 => {
                    (self.model, self.effort) = self.default_selection.clone();
                    self.picked(cx);
                }
                _ => {}
            },
            Some(Menu::Comments) => self.open_comment_attachment(index, window, cx),
            None => {}
        }
        cx.notify();
    }

    /// Returns the open state, focused item, and trigger focus for interaction replay.
    pub fn menu_state(&self, window: &Window) -> (bool, Option<usize>, bool) {
        (
            self.menu.is_some(),
            self.menu_focus.focused_item(window),
            self.menu_focus.trigger.is_focused(window),
        )
    }

    /// Returns the `Speed` submenu's open state and focused item for interaction replay.
    pub fn speed_menu_state(&self, window: &Window) -> (bool, Option<usize>) {
        (self.speed_open, self.speed_focus.focused_item(window))
    }

    pub fn model_trigger_focused(&self, window: &Window) -> bool {
        self.model_focus.is_focused(window)
    }

    pub fn permission(&self) -> Permission {
        self.permission
    }

    pub fn run_settings(&self) -> (Value, Permission) {
        (
            json!({"mode": if self.plan { "plan" } else { "default" }, "settings": {
                "model": self.current_model().map_or("", |m| m.id.as_str()),
                "reasoning_effort": self.effort, "developer_instructions": null,
            }}),
            self.permission,
        )
    }

    pub fn restore_run_settings(
        &mut self,
        mode: &Value,
        permission: Permission,
        cx: &mut Context<Self>,
    ) {
        if let Some(model) = mode["settings"]["model"].as_str()
            && let Some(index) = self.models.iter().position(|option| option.id == model)
        {
            self.model = index;
            self.effort = mode["settings"]["reasoning_effort"]
                .as_str()
                .map(str::to_owned);
        }
        self.plan = mode["mode"] == "plan";
        self.permission = permission;
        cx.notify();
    }

    pub fn confirm_full_access(&mut self, cx: &mut Context<Self>) {
        self.permission = Permission::FullAccess;
        cx.notify();
    }

    /// The stop confirmation is armed (first Escape of two).
    pub fn stop_armed(&self) -> bool {
        self.stop_armed.is_some()
    }

    pub fn fast_mode(&self) -> bool {
        self.fast
    }

    pub fn plan_mode(&self) -> bool {
        self.plan
    }

    /// `Plan mode` (slash command, `@` row) and the footer's `Plan` chip turn it on and off.
    pub fn toggle_plan(&mut self, cx: &mut Context<Self>) {
        self.plan = !self.plan;
        cx.notify();
    }
}

/// `_Button` pill (28 px, `rounded-full`, 1 px transparent border).
pub fn pill(id: impl Into<ElementId>, t: &Theme) -> Stateful<Div> {
    let hover = t.ghost_hover;
    div()
        .id(id)
        .flex_none()
        .h(px(28.))
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .hover(move |s| s.bg(hover))
}

/// A 13/18 label painted through a transparent compositing layer.
pub fn label(s: impl Into<SharedString>, color: Hsla) -> Div {
    text(s, 13., 18., 400., color).top_layer()
}

/// The model name sits in a `tabular-nums` span (`GPT-6`'s digit takes its tabular advance).
pub(crate) fn tabular(d: Div) -> Div {
    d.font_features(gpui::FontFeatures(std::sync::Arc::new(vec![(
        "tnum".into(),
        1,
    )])))
}

pub(crate) fn shadow(color: Hsla, y: f32, blur: f32, spread: f32, inset: bool) -> shapes::Shadow {
    shapes::Shadow {
        color,
        offset: point(px(0.), px(y)),
        blur: px(blur),
        spread: px(spread),
        inset,
    }
}

/// A plain rounded box: `border-radius`, `background-color` and `box-shadow`.
fn deco(radius: f32, fill: Hsla, shadows: Vec<shapes::Shadow>) -> shapes::Decoration {
    shapes::Decoration {
        radius: Corners::all(px(radius)),
        fill: Some(fill),
        shadows,
        ..Default::default()
    }
}

/// `_ComposerLayoutBody`: 22 px radius, composer surface, `--elevation-composer-*`. Below a
/// 640 px (`40rem`) viewport (`narrow`), the light shadow's outer blur halves to 40 px.
pub(crate) fn card_decoration(t: &Theme, narrow: bool) -> shapes::Decoration {
    let black = |a: u32| -> Hsla { gpui::rgba(a).into() };
    let shadows = match t.appearance {
        Appearance::Dark => vec![shadow(t.composer_inset_highlight, 0., 1., 0., true)],
        Appearance::Light => vec![
            shadow(black(0x0000000a), 0., 0., 1., false),
            shadow(black(0x0000000a), 2., 8., 0., false),
            shadow(
                black(0x00000006),
                4.,
                if narrow { 40. } else { 80. },
                8.,
                false,
            ),
        ],
    };
    deco(22., t.composer_surface, shadows)
}

/// `target` as `composer`'s file drop target (the original's drag handlers on the page
/// column): while a file drag is over its box the card surface switches and the `Drop to
/// attach` overlay (`bg-codex-drop-overlay/60`, `z-40`: a centred pill on
/// `bg-codex-drop-prompt`, `rounded-md`, `border-border/50`, `shadow`, 13 px text) covers it;
/// a drop attaches the files. Call it last: the overlay paints over the children added so far.
pub fn drop_target(target: Div, composer: &Entity<Composer>, cx: &App) -> Div {
    let t = cx.theme();
    let over = composer.read(cx).drag_active(cx);
    let (moved, dropped) = (composer.clone(), composer.clone());
    let target = target
        .relative()
        .on_drag_move::<ExternalPaths>(move |e, _, cx| {
            let over = e.bounds.contains(&e.event.position);
            moved.update(cx, |c, cx| c.set_drag_active(over, cx));
        })
        .on_drop::<ExternalPaths>(move |paths, _, cx| {
            dropped.update(cx, |c, cx| c.drop_files(paths.paths(), cx));
        });
    if !over {
        return target;
    }
    let black = |a: u32| -> Hsla { gpui::rgba(a).into() };
    let pill = shapes::Decoration {
        border: Some((px(1.), t.border.opacity(0.5))),
        ..deco(
            10.,
            t.background_accent,
            vec![
                shadow(black(0x0000001a), 1., 3., 0., false),
                shadow(black(0x0000001a), 1., 2., -1., false),
            ],
        )
    };
    target.child(
        div()
            .absolute()
            .inset_0()
            .bg(t.background_accent.opacity(0.6))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    // `px-3 py-1` plus the 1 px border, painted by the decoration.
                    .px(px(13.))
                    .py(px(5.))
                    .child(shapes::decoration(pill))
                    .child(text("Drop to attach", 13., 18.5714, 430., t.text)),
            ),
    )
}

/// Popover surface `width` CSS px wide holding `content`: `--menu-background-color` at 90%
/// over the `backdrop-filter: blur(8px)` backdrop, `--menu-corner-shape`. Chromium composites
/// it as its own layer: the blurred backdrop, then the layer (shadows, fill and content
/// rasterized into transparent tiles) over it. The layer's tile grid starts at its shadow's
/// ink as cc bounds it, 3 sigma (`0 8px 16px -4px`: 20 px left of the box, 12 px above;
/// CDP `LayerTree`).
pub(crate) fn popover(t: &Theme, width: f32, content: impl FnOnce(Div) -> Div) -> Div {
    // `bg-surface-elevated-secondary/90` mixes in oklab: white's sRGB lands just outside
    // [0, 1] (red and blue above 1: the popup's edge px keep blue at red), so Skia blits it
    // with its float pipeline (theme `white_mix`).
    let fill = match t.appearance {
        Appearance::Light => gpui::Rgba {
            r: 1.00003,
            g: 0.99999,
            b: 1.00003,
            a: 0.9,
        }
        .into(),
        Appearance::Dark => Hsla {
            a: 0.9,
            ..t.menu_background
        },
    };
    let d = deco(
        20.,
        fill,
        vec![
            shadow(t.border, 0., 0., 0.5, false),
            shadow(t.border, 0., 0., 0.5, false),
            shadow(gpui::rgba(0x0000001f).into(), 8., 16., -4., false),
        ],
    );
    // Chromium draws the content under the box's anti-aliased overflow clip, which forces
    // AAA's run blitter (the tip px of the effort menu's model chevron).
    let surface = div()
        .relative()
        .w(px(width))
        .flex()
        .flex_col()
        .child(shapes::backdrop_blur(
            px(8.),
            shapes::Fill::Color,
            false,
            shapes::Decoration {
                radius: d.radius,
                corner: shapes::Corner::Squircle,
                ..Default::default()
            },
        ))
        .child(shapes::decoration(shapes::Decoration {
            corner: shapes::Corner::Squircle,
            ..d
        }))
        .child(crate::rounded_clip(
            d.radius.top_left,
            shapes::Corner::Squircle,
            content(div().flex().flex_col().p(px(4.))),
        ));
    div()
        .ml(px(-20.))
        .mt(px(-12.))
        .pl(px(20.))
        .pt(px(12.))
        .layer()
        .child(surface)
}

/// Width the original's popper positions the effort menu with, centered on the model button:
/// in refs/composer its 256 px box sits 1.19 px left of the button's center, snapped to
/// device px.
const EFFORT_MENU_SLOT: f32 = 258.5;

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.on_cancel_edit.is_some();
        let t = cx.theme().clone();
        let narrow = window.viewport_size().width < px(640.);
        if self.request_focus_pending {
            self.request_focus_pending = false;
            self.focus_request_field(window, cx);
        }
        if let Some(approval) = self.approval.as_ref()
            && (approval.form.is_some()
                || approval.permissions.is_some()
                || approval.computer_use.is_some())
        {
            let context = match (
                approval.form.is_some(),
                self.focus_handle.is_focused(window),
            ) {
                (true, true) => "Composer ComposerRequest ComposerRequestForm",
                (true, false) => "ComposerRequest ComposerRequestForm",
                (false, true) => "Composer ComposerRequest",
                (false, false) => "ComposerRequest",
            };
            return div().w_full().child(
                div()
                    .id("server-request")
                    .key_context(context)
                    .track_focus(&self.focus_handle)
                    .on_action(self.request_listener(cx, Self::enter))
                    .on_action(self.request_listener(cx, Self::dismiss))
                    .child(if approval.computer_use.is_some() {
                        self.computer_use_card(approval, &t, cx)
                    } else if approval.form.is_some() {
                        self.request_card(approval, &t, cx)
                    } else {
                        self.permission_request_card(approval, &t, window, cx)
                    }),
            );
        }
        let card = div()
            .id("composer")
            .when(self.focus_handle.is_focused(window), |d| {
                d.key_context(KEY_CONTEXT)
            })
            .track_focus(&self.focus_handle)
            .on_action(self.request_listener(cx, Self::enter))
            .on_action(cx.listener(Self::submit_in_background))
            .on_action(self.request_listener(cx, Self::modified_enter))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(|this, _: &crate::Paste, window, cx| this.paste(window, cx)))
            .on_action(
                cx.listener(|this, _: &crate::PasteAndMatchStyle, window, cx| {
                    this.paste_and_match_style(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::DeleteSelection, window, cx| {
                if !this.selected_range.is_empty() {
                    this.insert("", window, cx);
                }
            }))
            .when(
                cfg!(target_os = "macos") && !self.secret && !self.content.is_empty(),
                |card| card.on_action(cx.listener(Self::speak_selection)),
            )
            .on_action(self.request_listener(cx, Self::dismiss))
            .on_action(cx.listener(Self::complete_command))
            .on_action(cx.listener(Self::next_command))
            .on_action(cx.listener(Self::previous_command))
            .relative();
        if self.mini {
            let enabled = self.current_model().is_some() && !self.content.trim().is_empty();
            return div().w_full().child(
                card.key_context(KEY_CONTEXT)
                    .role(gpui::Role::TextInput)
                    .aria_label("Chat")
                    .aria_placeholder(self.placeholder.clone())
                    .aria_value(self.content.clone())
                    .h(px(40.))
                    .px(px(6.))
                    .pl(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(20.)),
                        fill: Some(t.control_background),
                        ..Default::default()
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(self.editor(cx)),
                    )
                    .child(
                        div()
                            .id("mini-send")
                            .role(gpui::Role::Button)
                            .aria_label("Send chat")
                            .aria_disabled(!enabled)
                            .size(px(28.))
                            .rounded_full()
                            .bg(gpui::rgb(0x636363))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::icon::icon("composer-arrow-up", 16., gpui::white()))
                            .when(enabled, |button| {
                                button.on_click(cx.listener(|this, _, window, cx| {
                                    this.submit(&Submit, window, cx);
                                }))
                            }),
                    ),
            );
        }
        if self.field_style == FieldStyle::Compact {
            let multiline = self.plain_field == Some(true);
            return div().w_full().when(multiline, |d| d.pb(px(6.))).child(
                card.key_context(KEY_CONTEXT)
                    .role(if self.secret {
                        gpui::Role::PasswordInput
                    } else {
                        gpui::Role::TextInput
                    })
                    .aria_label(
                        self.request_label
                            .as_ref()
                            .unwrap_or(&self.placeholder)
                            .clone(),
                    )
                    .aria_placeholder(self.placeholder.clone())
                    .when(!self.secret, |d| d.aria_value(self.content.clone()))
                    .px(px(1.))
                    .when(multiline, |d| {
                        d.py(px(9.))
                            .max_h(px(120.))
                            .overflow_y_scroll()
                            .track_scroll(
                                self.field_scroll.as_ref().expect("multiline field scroll"),
                            )
                    })
                    .when(!multiline, |d| d.h(px(36.)).flex().items_center())
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(10.)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.control_background.opacity(0.96)),
                        border: Some((px(1.), t.border_strong)),
                        ..Default::default()
                    }))
                    .child(self.editor(cx)),
            );
        }
        if self.plain_field.is_some() && self.field_style != FieldStyle::Settings {
            let multiline = self.plain_field == Some(true);
            let inline = matches!(self.field_style, FieldStyle::Inline | FieldStyle::Dialog);
            let input = self
                .editor(cx)
                .w_full()
                .min_w_0()
                .px(px(if inline {
                    0.
                } else if multiline {
                    18.
                } else {
                    20.
                }))
                .pb_0()
                .min_h(px(20.))
                .when(inline, |d| d.h(px(20.)).overflow_hidden())
                .when(!inline && !multiline, |d| d.relative().top(px(0.5)))
                .font_weight(gpui::FontWeight(400.));
            let field = card
                .w_full()
                .role(gpui::Role::TextInput)
                .aria_label(
                    self.request_label
                        .clone()
                        .unwrap_or(self.placeholder.clone()),
                )
                .aria_placeholder(self.placeholder.clone())
                .aria_value(self.content.clone())
                .when(!inline, |d| {
                    d.p(px(1.)).child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(20.)),
                        corner: shapes::Corner::Squircle,
                        border: Some((px(1.), t.border)),
                        ..Default::default()
                    }))
                })
                .when(multiline, |d| d.min_h(px(122.)).py(px(17.)))
                .when(!multiline, |d| {
                    d.h(px(if inline { 40. } else { 48. }))
                        .flex()
                        .items_center()
                })
                .child(input);
            return div().w_full().child(field);
        }
        if self.embedded_field.is_some() {
            return div().w_full().child(
                card.key_context(KEY_CONTEXT)
                    .role(gpui::Role::TextInput)
                    .aria_label(self.placeholder.clone())
                    .aria_value(self.content.clone())
                    .h(self.editor_line_height())
                    .overflow_hidden()
                    .child(self.editor(cx)),
            );
        }
        if self.plain_field.is_some() {
            if self.document_field {
                return div().size_full().child(
                    card.size_full().child(
                        div()
                            .id("document-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(self.field_scroll.as_ref().expect("document scroll"))
                            .child(self.editor(cx).px_0().py_0()),
                    ),
                );
            }
            let card = if self.textarea() {
                let border = if self.focus_handle.is_focused(window) {
                    gpui::rgb(0x3a83f7).into()
                } else if self.input_style {
                    t.border_strong
                } else {
                    t.border
                };
                let scroll = self
                    .field_scroll
                    .as_ref()
                    .expect("multiline field scroll")
                    .clone();
                let line_height = self.editor_line_height();
                let mut content_height = px(0.);
                let scrolling = self.lines.iter().any(|line| {
                    content_height += line.size(line_height).height;
                    content_height > self.field_min_height
                });
                card.px(px(1.))
                    .py(px(1.))
                    .top_layer()
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(10.)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.control_background.opacity(0.96)),
                        border: Some((px(1.), border)),
                        ..Default::default()
                    }))
                    .child(
                        div()
                            .id("textarea-scroll")
                            .h(self.field_min_height + px(16.))
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .child(
                                div()
                                    .py(px(8.))
                                    .when(scrolling, |content| content.top_layer())
                                    .child(self.editor(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("textarea-resize")
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .size(px(16.))
                            .cursor(CursorStyle::ResizeUpDown)
                            .child(
                                gpui::canvas(
                                    |_, _, _| {},
                                    |bounds, (), window, _| paint_resizer(bounds, window),
                                )
                                .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    this.field_resize =
                                        Some((event.position.y, this.field_min_height));
                                    this.focus_handle.focus(window, cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .when_some(self.field_resize, |card, (start, height)| {
                        let moved = cx.entity().downgrade();
                        let released = moved.clone();
                        card.child(crate::drag_layer(
                            move |_, window, cx| {
                                let next = (height + window.mouse_position().y - start)
                                    .max(MIN_TEXTAREA_CONTENT_HEIGHT);
                                moved
                                    .update(cx, |this, cx| {
                                        if this.field_min_height != next {
                                            this.field_min_height = next;
                                            this.field_autoscroll = true;
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                            },
                            move |cx| {
                                released
                                    .update(cx, |this, cx| {
                                        this.field_resize = None;
                                        cx.notify();
                                    })
                                    .ok();
                            },
                        ))
                    })
            } else {
                card.key_context(KEY_CONTEXT)
                    .when(
                        self.request_parent.is_none()
                            && !self.request_compact
                            && !self.search_field
                            && !editing,
                        |d| {
                            d.border_1()
                                .border_color(t.border)
                                .focus_visible(|style| style.border_color(gpui::rgb(0x3a83f7)))
                                .rounded(px(8.))
                                .bg(t.background_panel)
                                .py(px(8.))
                        },
                    )
                    // The select search row is a bare input inside the menu, as in the original.
                    .when(
                        self.request_parent.is_none() && self.request_compact && !self.search_field,
                        |d| {
                            d.px(px(1.))
                                .py(px(if self.small_input { 4. } else { 8. }))
                                .child(shapes::decoration(shapes::Decoration {
                                    radius: Corners::all(Theme::RADIUS_MD),
                                    corner: shapes::Corner::Squircle,
                                    fill: Some(if t.appearance == Appearance::Dark {
                                        gpui::rgba(0x2d2d2df5).into()
                                    } else {
                                        t.background_panel
                                    }),
                                    border: Some((
                                        px(1.),
                                        if self.focus_handle.is_focused(window) {
                                            gpui::rgba(0x83c3ffc2).into()
                                        } else {
                                            t.border_strong
                                        },
                                    )),
                                    ..Default::default()
                                }))
                        },
                    )
                    .when(self.request_parent.is_some(), |d| {
                        d.px(px(1.))
                            .py(px(if self.secret {
                                8.449_219
                            } else if self.request_compact {
                                8.
                            } else {
                                13.
                            }))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(if self.request_compact && !self.secret {
                                    10.
                                } else {
                                    12.5
                                })),
                                corner: shapes::Corner::Squircle,
                                border: Some((px(1.), t.border_strong)),
                                fill: (self.request_parent.is_none() && self.request_compact)
                                    .then_some(t.control_background.opacity(0.96)),
                                ..Default::default()
                            }))
                    })
                    .role(if self.secret {
                        gpui::Role::PasswordInput
                    } else {
                        gpui::Role::TextInput
                    })
                    .aria_label(
                        self.request_label
                            .as_ref()
                            .unwrap_or(&self.placeholder)
                            .clone(),
                    )
                    .aria_placeholder(self.placeholder.clone())
                    .when(!self.secret, |d| d.aria_value(self.content.clone()))
                    .child(
                        self.editor(cx)
                            .id("composer-field-editor")
                            .when(editing, |d| {
                                d.max_h(window.viewport_size().height / 4.)
                                    .overflow_y_scroll()
                            })
                            .when(self.request_number, |editor| editor.pr(px(30.))),
                    )
                    .when(self.request_number, |d| {
                        d.child(self.request_number_stepper(&t, cx))
                    })
                    .when(
                        self.request_parent.is_some() && !self.secret && !self.request_number,
                        |d| d.child(self.dictation_button(&t, cx)),
                    )
                    .when_some(self.dictation_error.clone(), |d, error| {
                        d.child(text(error, 12., 16., 400., t.text))
                    })
            };
            return if self.textarea() {
                div().w_full().child(crate::recorded_bounds(card))
            } else {
                div().w_full().child(card)
            };
        }
        let card = card
            .flex()
            .flex_col()
            .when(self.hotkey_controls.is_none(), |card| {
                card.child(shapes::decoration(if editing {
                    shapes::Decoration {
                        radius: Corners::all(px(24.)),
                        fill: Some(match t.appearance {
                            Appearance::Dark => t.control_background.opacity(0.95),
                            Appearance::Light => t.text_alpha_5,
                        }),
                        ..Default::default()
                    }
                } else if self.drag_active(cx) {
                    // `[data-composer-drag-active]`: `surface-elevated-secondary` at 50%.
                    shapes::Decoration {
                        fill: Some(t.surface_elevated_secondary.opacity(0.5)),
                        ..card_decoration(&t, narrow)
                    }
                } else {
                    card_decoration(&t, narrow)
                }))
            })
            // See `docked`: the docked body is a transparent cc layer over the card.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(self.docked, |d| d.layer())
                    .when(
                        !editing
                            && (self.hotkey_controls.is_none() || !self.attachments.is_empty()),
                        |d| d.child(self.attachments_row(&t, cx)),
                    )
                    .when_some(self.upload_status(), |d, error| {
                        d.child(label(error, t.text))
                    })
                    .child(
                        self.editor(cx)
                            .when(self.hotkey_controls.is_some(), |d| d.px_0().min_h(px(48.)))
                            .when(editing, |d| d.pt(px(12.)).pb_0().min_h(px(52.)))
                            // The original's `max-h-[min(25dvh,…)] overflow-y-auto`.
                            .id("composer-editor")
                            .max_h(window.viewport_size().height / 4.)
                            .overflow_y_scroll()
                            .when_some(self.field_scroll.as_ref(), |d, scroll| {
                                d.track_scroll(scroll)
                            }),
                    )
                    .child(if editing {
                        let cancel = self.on_cancel_edit.clone().expect("edit callback");
                        let enabled = !self.edit_pending && !self.content.trim().is_empty();
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(6.))
                            .px(px(12.))
                            .pt(px(8.))
                            .pb(px(12.))
                            .child(
                                div()
                                    .id("cancel-message-edit")
                                    .h(px(28.))
                                    .px(px(10.))
                                    .role(gpui::Role::Button)
                                    .aria_label("Cancel")
                                    .focusable()
                                    .tab_index(0)
                                    .focus_visible(|s| {
                                        s.border_2().border_color(gpui::rgb(0x3a83f7))
                                    })
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(t.border)
                                    .child(text("Cancel", 13., 26., 400., t.text))
                                    .when(!self.edit_pending, |d| {
                                        d.on_click(move |_, window, cx| cancel(window, cx))
                                    }),
                            )
                            .child(
                                div()
                                    .id("send-message-edit")
                                    .h(px(28.))
                                    .px(px(10.))
                                    .role(gpui::Role::Button)
                                    .aria_label("Send")
                                    .focusable()
                                    .tab_index(0)
                                    .focus_visible(|s| {
                                        s.border_2().border_color(gpui::rgb(0x3a83f7))
                                    })
                                    .rounded(px(6.))
                                    .bg(t.text)
                                    .opacity(if enabled { 1. } else { 0.4 })
                                    .child(text("Send", 13., 28., 400., t.surface_elevated))
                                    .when(enabled, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
                                            this.submit(&Submit, window, cx)
                                        }))
                                    }),
                            )
                            .into_any_element()
                    } else {
                        self.footer(window, &t, cx).into_any_element()
                    }),
            );
        let card = card.when(self.slash.open, |d| d.child(self.slash_menu(&t, cx)));
        // The queued-follow-up tray (`bg-surface-elevated-secondary`, border-x border-t) sits
        // on the card's top edge, 13 px inside its sides; rows (handle, text, Steer, Delete,
        // `…` actions) have a 32 px height and a 1 px gap.
        let tray = (!self.queued.is_empty()).then(|| {
            div()
                .mx(px(13.))
                .border_t_1()
                .border_x_1()
                .border_color(t.border)
                .rounded_t(px(12.))
                .bg(t.surface_elevated_secondary)
                .flex()
                .flex_col()
                .gap(px(1.))
                .children(self.queued.iter().map(|row| {
                    div()
                        .id(row.key.clone())
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("queue-handle", 14., t.text_tertiary))
                        .child(div().flex_1().min_w_0().overflow_hidden().child(text(
                            row.text.clone(),
                            14.,
                            20.,
                            430.,
                            t.text,
                        )))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(self.queue_button(
                                    if row.failed {
                                        QueueAction::Retry(row.submission.id.clone())
                                    } else {
                                        QueueAction::Steer(row.submission.id.clone())
                                    },
                                    if row.failed { "Retry" } else { "Steer" },
                                    if row.failed {
                                        "Retry"
                                    } else {
                                        "Submit without interrupting the model"
                                    },
                                    &t,
                                    cx,
                                ))
                                .child(self.queue_button(
                                    QueueAction::Delete(row.submission.id.clone()),
                                    "Delete queued message",
                                    "Delete queued message",
                                    &t,
                                    cx,
                                ))
                                .child(self.queue_menu_button(
                                    row.submission.id.clone(),
                                    &t,
                                    window,
                                    cx,
                                )),
                        )
                }))
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .when_some(self.approval.clone(), |d, a| {
                d.child(self.approval_card(&a, &t, cx))
            })
            .child(
                div()
                    .children(tray)
                    .child(card)
                    .children(self.blocked.clone().map(|dialog| {
                        let viewport = window.viewport_size();
                        deferred(
                            gpui::anchored().position(point(px(0.), px(0.))).child(
                                div()
                                    .relative()
                                    .w(viewport.width)
                                    .h(viewport.height)
                                    .child(dialog),
                            ),
                        )
                    })),
            )
            .when_some(self.shell_disabled_reason(), |d, reason| {
                d.child(text(reason, 12., 16., 400., t.text))
            })
            .when_some(self.dictation_error.clone(), |d, error| {
                d.child(text(error, 12., 16., 400., t.text))
            })
            .when_some(self.context_menu.as_ref(), |d, menu| {
                d.child(menu.overlay(
                    "composer-menu",
                    if menu.items.first().is_some_and(|item| item.id == QUEUE_EDIT) {
                        QUEUE_ACTIONS
                    } else if self.context_image.is_some() {
                        "Image actions"
                    } else {
                        "Edit actions"
                    },
                    |this| &mut this.context_menu,
                    Self::activate_context,
                    window,
                    cx,
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_shortcut_controls_enter_without_changing_shift_enter() {
        use gpui::{AppContext as _, HeadlessAppContext, Keystroke};
        use std::{cell::RefCell, sync::Arc};

        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));

        let mut app =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        app.update(|cx| {
            cx.set_global(Theme::dark());
            theme::load_fonts(cx);
            bind_keys(cx);
        });
        let sent = Rc::new(RefCell::new(Vec::new()));
        let received = sent.clone();
        let window = app
            .open_window(size(px(600.), px(400.)), |window, cx| {
                let composer = cx.new(|cx| {
                    let mut composer = Composer::home(window, cx);
                    composer.models.push(ModelOption {
                        id: "gpt-6.1-sol".into(),
                        label: "GPT-6.1 Sol".into(),
                        efforts: vec!["medium".into()],
                    });
                    composer.on_submit(move |submission, _, _| {
                        received.borrow_mut().push(submission.text.clone());
                    });
                    composer
                });
                composer.read(cx).focus_handle(cx).focus(window, cx);
                composer
            })
            .unwrap();
        for (setting, draft, key, sends) in [
            ("enter", "one", "enter", true),
            ("enter", "!ordinary prompt", "enter", true),
            ("enter", "one\ntwo", "cmd-enter", true),
            ("cmdIfMultiline", "one", "enter", true),
            ("cmdIfMultiline", "one\ntwo", "enter", false),
            ("cmdIfMultiline", "one\ntwo", "cmd-enter", true),
            ("cmdAlways", "one", "enter", false),
            ("cmdAlways", "one", "cmd-enter", true),
            ("cmdAlways", "one", "shift-enter", false),
            ("unknown", "one", "enter", true),
        ] {
            sent.borrow_mut().clear();
            app.update(|cx| {
                cx.set_global(EnterBehavior::from_setting(setting));
                window
                    .update(cx, |composer, _, cx| composer.set_text(draft, cx))
                    .unwrap();
            });
            app.run_until_parked();
            app.update_window(window.into(), |_, window, cx| {
                window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
            })
            .unwrap();
            app.run_until_parked();
            app.update(|cx| {
                window
                    .update(cx, |composer, _, _| {
                        if sends {
                            assert_eq!(*sent.borrow(), [draft], "{setting}: {key}");
                            assert!(composer.content.is_empty());
                        } else {
                            assert!(sent.borrow().is_empty(), "{setting}: {key}");
                            assert_eq!(composer.content, format!("{draft}\n"));
                        }
                    })
                    .unwrap();
            });
        }
    }

    #[test]
    fn submit_blocked_by_uploads_shows_unable_to_send_message() {
        use gpui::{AppContext as _, HeadlessAppContext, Keystroke};
        use std::{cell::RefCell, sync::Arc};

        let mut app =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        app.update(|cx| {
            cx.set_global(Theme::dark());
            theme::load_fonts(cx);
            bind_keys(cx);
        });
        let sent = Rc::new(RefCell::new(Vec::new()));
        let received = sent.clone();
        let window = app
            .open_window(size(px(600.), px(400.)), |window, cx| {
                let composer = cx.new(|cx| {
                    let mut composer = Composer::home(window, cx);
                    composer.models.push(ModelOption {
                        id: "gpt-6.1-sol".into(),
                        label: "GPT-6.1 Sol".into(),
                        efforts: vec!["medium".into()],
                    });
                    composer.on_submit(move |submission, _, _| {
                        received.borrow_mut().push(submission.text.clone());
                    });
                    composer
                });
                composer.read(cx).focus_handle(cx).focus(window, cx);
                composer
            })
            .unwrap();
        let press = |app: &mut HeadlessAppContext, key: &str| {
            app.run_until_parked();
            app.update_window(window.into(), |_, window, cx| {
                window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
            })
            .unwrap();
            app.run_until_parked();
            app.update(|cx| {
                window
                    .update(cx, |composer, window, cx| {
                        let dialog = composer
                            .blocked
                            .as_ref()
                            .map(|dialog| dialog.focus_handle(cx).is_focused(window));
                        let focused = composer.focus_handle.is_focused(window);
                        (dialog, focused, composer.content.clone())
                    })
                    .unwrap()
            })
        };
        app.update(|cx| {
            window
                .update(cx, |composer, _, cx| {
                    composer.set_text("hi", cx);
                    composer.pending_attachments = 1;
                })
                .unwrap();
        });
        // Enter while a paste saves opens the dialog with OK focused and keeps the draft.
        assert_eq!(press(&mut app, "enter"), (Some(true), false, "hi".into()));
        assert!(sent.borrow().is_empty());
        // OK closes it and returns focus to the composer.
        assert_eq!(press(&mut app, "enter"), (None, true, "hi".into()));
        assert_eq!(press(&mut app, "enter"), (Some(true), false, "hi".into()));
        assert_eq!(press(&mut app, "escape"), (None, true, "hi".into()));
        assert!(sent.borrow().is_empty());
        app.update(|cx| {
            window
                .update(cx, |composer, _, _| {
                    composer.pending_attachments = 0;
                    composer.attachment_error = Some("Unable to upload file");
                })
                .unwrap();
        });
        assert_eq!(press(&mut app, "enter"), (Some(true), false, "hi".into()));
        assert!(sent.borrow().is_empty());
        assert_eq!(press(&mut app, "escape"), (None, true, "hi".into()));
        app.update(|cx| {
            window
                .update(cx, |composer, _, _| composer.attachment_error = None)
                .unwrap();
        });
        assert_eq!(press(&mut app, "enter"), (None, true, String::new()));
        assert_eq!(*sent.borrow(), ["hi"]);
    }

    #[test]
    fn editor_grows_to_a_quarter_of_the_window_then_scrolls() {
        use gpui::{AppContext as _, HeadlessAppContext};
        use std::sync::Arc;
        let mut app = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            cx.set_global(Theme::dark());
            theme::load_fonts(cx);
        });
        let window = app
            .open_window(size(px(600.), px(400.)), |window, cx| {
                cx.new(|cx| Composer::home(window, cx))
            })
            .unwrap();
        let mut measure = |lines: usize| {
            app.update(|cx| {
                window
                    .update(cx, |c, _, cx| c.set_text(&"line\n".repeat(lines - 1), cx))
                    .unwrap();
            });
            app.run_until_parked();
            app.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            app.update(|cx| {
                window
                    .update(cx, |c, _, _| {
                        let scroll = c.field_scroll.as_ref().unwrap();
                        (scroll.bounds().size.height, scroll.max_offset().y)
                    })
                    .unwrap()
            })
        };
        let (one, one_scroll) = measure(1);
        let (three, three_scroll) = measure(3);
        let (many, many_scroll) = measure(40);
        assert!(three > one && one_scroll == px(0.) && three_scroll == px(0.));
        assert_eq!(many, px(100.), "25% of the 400 px window");
        assert!(many_scroll > px(0.));
    }

    #[test]
    fn model_selection_preserves_other_composer_choices() {
        use gpui::{AppContext as _, HeadlessAppContext};
        let mut app = HeadlessAppContext::with_platform(
            textsys::text_system(),
            std::sync::Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            let composer = cx.new(|cx| Composer::new("Do anything", false, cx));
            composer.update(cx, |composer, cx| {
                composer.models = ["original", "announced"]
                    .map(|id| ModelOption {
                        id: id.into(),
                        label: id.into(),
                        efforts: vec!["medium".into(), "high".into()],
                    })
                    .into();
                composer.permission = Permission::FullAccess;
                composer.set_queue_mode(true, cx);
                composer.set_text("Keep this draft", cx);
                composer.set_model_and_effort("announced", "high", cx);
                assert_eq!(composer.current_model().unwrap().id, "announced");
                assert_eq!(composer.effort.as_deref(), Some("high"));
                composer.set_model_and_effort("missing", "low", cx);
                assert_eq!(composer.current_model().unwrap().id, "announced");
                assert_eq!(composer.effort.as_deref(), Some("high"));
                assert_eq!(composer.permission, Permission::FullAccess);
                assert!(composer.queue_mode);
                assert_eq!(composer.default_selection, (0, Some("medium".into())));
                assert_eq!(composer.content, "Keep this draft");
            });
        });
    }

    #[test]
    fn restored_effort_stays_unset_while_displaying_medium() {
        use gpui::{AppContext as _, HeadlessAppContext};
        let mut app = HeadlessAppContext::with_platform(
            textsys::text_system(),
            std::sync::Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            let composer = cx.new(|cx| Composer::new("Do anything", false, cx));
            composer.update(cx, |composer, cx| {
                composer.models = vec![ModelOption {
                    id: "gpt-6.1-sol".into(),
                    label: "GPT-6.1 Sol".into(),
                    efforts: vec!["low".into(), "medium".into()],
                }];
                composer.restore_run_settings(
                    &json!({"settings":{"model":"gpt-6.1-sol","reasoning_effort":null}}),
                    Permission::Ask,
                    cx,
                );
                assert!(composer.effort.is_none());
                assert_eq!(composer.displayed_effort(), "medium");
                assert!(composer.run_settings().0["settings"]["reasoning_effort"].is_null());
            });
        });
    }

    #[test]
    fn formats_model_names() {
        assert_eq!(model_label("GPT-6-Astra"), "GPT-6 Astra");
        assert_eq!(model_label("gpt-5.6-sol"), "GPT-5.6 Sol");
        assert_eq!(model_label("gpt-oss"), "GPT-Oss");
        assert_eq!(model_label("o3"), "o3");
    }
}
