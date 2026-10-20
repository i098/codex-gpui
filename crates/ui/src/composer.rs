//! The prompt composer: a multi-line editor, attachment chips, slash commands,
//! permission and model menus, and approval prompts.
//! Numbers use the CSS pixels of refs/home and refs/composer.
//! Clipboard text of at least 5,000 UTF-16 units becomes a saved text attachment.
//! Clipboard images retain their bytes and show a thumbnail.
//! S024: the first Escape shows `Esc` for two seconds; the next Escape interrupts the turn.
//! Composition and open suggestions consume Escape without stopping.
//! Permission presets apply to start requests. Full access requires the shared warning dialog.
//! `cargo test -p app --test s024` replays `refs/interactions/s024.json`, including the footer focus rings.

use crate::{MONO, icon::icon, text};
use bridge::types::{
    ApprovalDecision, CommandApprovalRequest, FileChangeApprovalRequest, Model, QueuedSubmission,
};
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ClipboardItem, Context, Corners, CursorStyle, Div,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, ExternalPaths, FocusHandle,
    Focusable, GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathPromptOptions, Pixels, Point, Render, SharedString, Size, Stateful,
    StatefulInteractiveElement, Style, Styled, Task, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, WrappedLine, actions, deferred, div, fill, point,
    prelude::FluentBuilder as _, px, relative, size,
};
use serde_json::Value;
use std::{ops::Range, path::PathBuf, rc::Rc, time::Duration};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(
    composer,
    [
        /// Send the prompt (Enter).
        Submit,
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

mod attach;
mod editor;
mod footer;
mod menu {
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

    pub(super) fn trigger_focus(style: StyleRefinement) -> StyleRefinement {
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
/// `@container composer-footer` (the card's width, CSS px) at or below which the footer hides
/// its `collapse=sm` labels (permissions) and its `collapse=xs` labels (model and effort).
const SM_WIDTH: f32 = 440.;
const XS_WIDTH: f32 = 300.;
const FONT_SIZE: Pixels = px(14.);
const LINE_HEIGHT: Pixels = px(20.);

/// Composer keys (inventory §7.2): Enter sends, Shift+Enter inserts a newline. Edit menu
/// commands (`crate::Cut`, `Copy`, `Paste`, `SelectAll`) are bound by the app.
pub fn bind_keys(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("enter", Submit, c),
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
    fn from_config(config: &Value) -> Self {
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

/// What the composer sends: the app turns it into `thread/start` + `turn/start`.
#[derive(Clone, Debug)]
pub struct Submission {
    pub text: String,
    pub attachments: Vec<PathBuf>,
    pub model: String,
    pub effort: String,
    pub permission: Permission,
    /// Fast mode (bolt toggle in the model picker).
    pub fast: bool,
}

/// A pending server approval request (`item/commandExecution/requestApproval`,
/// `item/fileChange/requestApproval`), shown above the composer.
#[derive(Clone, Debug)]
pub struct Approval {
    title: SharedString,
    /// Command line or edited paths.
    detail: Option<String>,
    reason: Option<String>,
    /// Session-wide approve label (`acceptForSession`).
    scoped_label: SharedString,
}

impl Approval {
    pub fn command(req: &CommandApprovalRequest) -> Self {
        Self {
            title: "Allow ChatGPT to run this command?".into(),
            detail: req.command.clone(),
            reason: req.reason.clone(),
            scoped_label: "Allow similar commands".into(),
        }
    }

    /// `paths`: the files of the pending `fileChange` item.
    pub fn file_change(req: &FileChangeApprovalRequest, paths: &[String]) -> Self {
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
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Permissions,
    Effort,
    Models,
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
type ApprovalHandler = Rc<dyn Fn(ApprovalDecision, &mut Window, &mut App)>;
/// `(model id, effort)` after the user picks them.
type ModelHandler = Rc<dyn Fn(&str, &str, &mut App)>;

/// What a queued row's buttons and `Queued message actions` menu ask of the app.
#[derive(Clone)]
pub enum QueueAction {
    Steer(String),
    Delete(String),
    /// `Edit message`: the queued submission with this id returns to the composer.
    Edit(String),
    /// `Turn off queueing` / `Turn on queueing`: the new `Follow-up behavior` is queue (`true`).
    Queueing(bool),
}

impl gpui::EventEmitter<QueueAction> for Composer {}

/// One row of the queue tray: the app-server's submission and the text it shows.
pub struct QueuedMessage {
    pub submission: QueuedSubmission,
    pub text: SharedString,
    key: SharedString,
}

const QUEUE_ACTIONS: &str = "Queued message actions";
const QUEUE_EDIT: &str = "queuedEdit";
const QUEUE_SWITCH: &str = "queuedQueueing";
const QUEUE_MENU_WIDTH: f32 = 220.;
pub struct Composer {
    focus_handle: FocusHandle,
    placeholder: SharedString,
    /// Docked under a thread: `_ComposerLayoutRoot` paints the card and `_ComposerLayoutBody`
    /// (the content) is a `will-change: opacity` layer. On Home the body paints the card.
    docked: bool,
    content: String,
    /// Mention chips in `content`, in order.
    mentions: Vec<mention::Mention>,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    history: editor::History,
    /// The right-click menu (Electron's native editable-field menu).
    context_menu: Option<crate::screens::thread::view::menu::ContextMenu>,
    is_selecting: bool,
    /// Shaped lines and editor bounds of the last paint, for hit testing.
    lines: Vec<WrappedLine>,
    bounds: Option<Bounds<Pixels>>,
    attachments: Vec<attach::Attachment>,
    /// The attachments of the last submission, until the next one: [`Self::restore_submission`]
    /// puts them back when starting the chat failed.
    submitted: Vec<attach::Attachment>,
    pending_attachments: usize,
    attachment_generation: u64,
    attachment_error: Option<&'static str>,
    /// A file drag is over the composer's drop target (`data-composer-drag-active`).
    drag_active: bool,
    models: Vec<ModelOption>,
    model: usize,
    effort: String,
    default_selection: (usize, String),
    fast: bool,
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
    /// The card's (`composer-footer` container's) width, CSS px: see `SM_WIDTH`, `XS_WIDTH`.
    footer_width: f32,
    menu_trigger_hovered: bool,
    approval: Option<Approval>,
    running: bool,
    /// Escape while a turn runs arms the stop for 2 s (the Stop button reads `Esc`); a second
    /// Escape in that window interrupts the turn. The task clears the arm.
    stop_armed: Option<Task<()>>,
    /// `Follow-up behavior` is `Queue`: a submit while a turn runs queues on the app-server
    /// instead of steering the turn.
    queue_mode: bool,
    /// The app-server's queued follow-ups of the open thread, shown in a tray over the card.
    queued: Vec<QueuedMessage>,
    queue_pending: bool,
    on_submit: Option<SubmitHandler>,
    on_cancel_edit: Option<CancelEditHandler>,
    edit_pending: bool,
    on_approval: Option<ApprovalHandler>,
    on_model: Option<ModelHandler>,
    plain_field: Option<bool>,
}

impl Composer {
    /// Home composer: placeholder `Do anything`.
    pub fn home(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new("Do anything", false, cx)
    }

    /// Thread composer, docked at the bottom of a thread: placeholder `Work with Codex`.
    pub fn thread(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new("Work with Codex", true, cx)
    }

    /// Composer under a scheduled task's thread: placeholder `Reply about this task…`.
    pub fn automation(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new("Reply about this task…", true, cx)
    }

    fn new(placeholder: &'static str, docked: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            placeholder: placeholder.into(),
            docked,
            content: String::new(),
            mentions: Vec::new(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            history: editor::History::default(),
            context_menu: None,
            is_selecting: false,
            lines: Vec::new(),
            bounds: None,
            attachments: Vec::new(),
            submitted: Vec::new(),
            pending_attachments: 0,
            attachment_generation: 0,
            attachment_error: None,
            drag_active: false,
            models: Vec::new(),
            model: 0,
            effort: "medium".into(),
            default_selection: (0, "medium".into()),
            fast: false,
            permission: Permission::Custom,
            menu: None,
            slash: slash::Slash::default(),
            menu_focus: menu::MenuFocus::new(cx),
            speed_open: false,
            speed_focus: menu::MenuFocus::new(cx),
            speed_timer: None,
            permissions_focus: cx.focus_handle().tab_stop(true),
            model_focus: cx.focus_handle().tab_stop(true),
            footer_width: f32::INFINITY,
            menu_trigger_hovered: false,
            approval: None,
            running: false,
            stop_armed: None,
            queue_mode: false,
            queued: Vec::new(),
            queue_pending: false,
            on_submit: None,
            on_cancel_edit: None,
            edit_pending: false,
            on_approval: None,
            on_model: None,
            plain_field: None,
        }
    }

    pub fn settings_field(
        placeholder: &'static str,
        value: &str,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::new(placeholder, false, cx);
        editor.plain_field = Some(multiline);
        editor.set_text(value, cx);
        editor
    }

    pub fn field_text(&self) -> &str {
        &self.content
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
            })
            .unwrap_or_else(|| "medium".into());
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
            self.effort = effort.into();
            cx.notify();
        }
    }

    pub fn on_submit(&mut self, f: impl Fn(&Submission, &mut Window, &mut App) + 'static) {
        self.on_submit = Some(Rc::new(f));
    }

    pub fn on_approval(&mut self, f: impl Fn(ApprovalDecision, &mut Window, &mut App) + 'static) {
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

    pub fn restart_ready(&self) -> bool {
        self.content.is_empty()
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
        self.approval = approval;
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
        self.queued = queued
            .into_iter()
            .map(|submission| {
                let text = submission
                    .input
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>();
                QueuedMessage {
                    key: submission.id.clone().into(),
                    submission,
                    text: text.trim_end().to_owned().into(),
                }
            })
            .collect();
        cx.notify();
    }

    pub fn queued(&self) -> &[QueuedMessage] {
        &self.queued
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
            QUEUE_SWITCH => cx.emit(QueueAction::Queueing(!self.queue_mode)),
            _ => self.activate_context_item(id, subject, window, cx),
        }
    }

    /// The row's `Queued message actions` menu (`composer.queuedMessage.more`): `Edit message`
    /// and the follow-up behavior switch, at the pointer like the editor's context menu.
    fn open_queue_menu(
        &mut self,
        id: &str,
        at: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::screens::thread::view::menu::{ContextMenu, Item};
        self.menu = None;
        self.slash.reset();
        let items = vec![
            Item::new(QUEUE_EDIT, "Edit message", true),
            Item::new(
                QUEUE_SWITCH,
                if self.queue_mode {
                    "Turn off queueing"
                } else {
                    "Turn on queueing"
                },
                true,
            ),
        ];
        // The original's dropdown is `align: end` under the button: the menu's right edge
        // meets the button's.
        let at = gpui::point(at.x - px(QUEUE_MENU_WIDTH - 12.), at.y + px(4.));
        self.context_menu = Some(ContextMenu::open(items, id.to_owned(), at, window, cx));
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
            _ => "queued-delete",
        };
        let key_action = action.clone();
        crate::tooltip::tooltip(
            div()
                .id(kind)
                .h(px(24.))
                .min_w(px(24.))
                .px(px(4.))
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
                .focus_visible(menu::trigger_focus)
                .opacity(if self.queue_pending { 0.4 } else { 1. })
                .when(!self.queue_pending, |d| d.hover(|s| s.bg(t.ghost_hover)))
                .child(match action {
                    QueueAction::Steer(_) => {
                        text("Steer", 12., 20., 400., t.text).into_any_element()
                    }
                    _ => icon("close", 14., t.text_tertiary).into_any_element(),
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
        cx: &mut Context<Self>,
    ) -> crate::tooltip::Tooltip {
        crate::tooltip::tooltip(
            div()
                .id(SharedString::from(format!("queued-actions-{id}")))
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
                .focus_visible(menu::trigger_focus)
                .opacity(if self.queue_pending { 0.4 } else { 1. })
                .when(!self.queue_pending, |d| d.hover(|s| s.bg(t.ghost_hover)))
                .child(icon("diff-more", 14., t.text_tertiary))
                // Enter would submit the draft: the key belongs to the button.
                .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                    }
                }))
                .on_click(
                    cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        if !this.queue_pending {
                            this.open_queue_menu(&id, event.position(), window, cx);
                        }
                    }),
                ),
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

    /// The submit button's accessible label (`composer.submitButtonTooltip.*`): `Stop` while a
    /// turn runs and the composer is empty, else the follow-up mode (`Queue` or `Steer`) while
    /// a turn runs, else `Send`. The original shows no tooltip on this button.
    pub fn send_label(&self) -> &'static str {
        let has_input = !self.content.is_empty() || !self.attachments.is_empty();
        match (self.running, has_input, self.queue_mode) {
            (false, _, _) => "Send",
            (true, false, _) => "Stop",
            (true, true, true) => "Queue",
            (true, true, false) => "Steer",
        }
    }

    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.content = s.into();
        self.mentions.clear();
        self.selected_range = s.len()..s.len();
        self.marked_range = None;
        self.history = editor::History::default();
        self.slash.reset();
        self.end_search(cx);
        cx.notify();
    }

    /// Clears account-bound drafts and invalidates pending attachment work.
    pub fn clear_session(&mut self, cx: &mut Context<Self>) {
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        self.attachments.clear();
        self.pending_attachments = 0;
        self.attachment_error = None;
        self.context_menu = None;
        self.menu = None;
        self.set_text("", cx);
        self.set_queued(Vec::new(), cx);
        self.set_running(false, cx);
        self.set_approval(None, cx);
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
        self.restore_text(text, cx);
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
        if !efforts.contains(&self.effort)
            && let Some(first) = efforts.first()
        {
            self.effort = first.clone();
        }
    }

    /// Reports the picked model and effort to [`Self::on_model`].
    fn picked(&self, cx: &mut App) {
        if let (Some(f), Some(model)) = (self.on_model.clone(), self.current_model()) {
            f(&model.id, &self.effort, cx);
        }
    }

    /// Enter: `approval.approve` accepts a pending request, and a draft still sends.
    fn enter(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(multiline) = self.plain_field {
            if multiline {
                self.insert("\n", window, cx);
            } else {
                window.blur(cx);
            }
            return;
        }
        if self.approval.is_some() && !self.slash.open {
            self.decide(ApprovalDecision::Accept, window, cx);
        }
        self.submit(&Submit, window, cx);
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash.open {
            self.activate_slash(window, cx);
            return;
        }
        if self.edit_pending
            || self.pending_attachments > 0
            || (self.content.trim().is_empty() && self.attachments.is_empty())
        {
            return;
        }
        let submission = Submission {
            text: if self.mentions.is_empty() && self.on_cancel_edit.is_none() {
                std::mem::take(&mut self.content)
            } else {
                self.prompt()
            },
            attachments: {
                self.submitted = std::mem::take(&mut self.attachments);
                self.submitted.iter().map(|a| a.path.clone()).collect()
            },
            model: self
                .current_model()
                .map(|m| m.id.clone())
                .unwrap_or_default(),
            effort: self.effort.clone(),
            permission: self.permission,
            fast: self.fast,
        };
        if self.on_cancel_edit.is_some() {
            self.edit_pending = true;
        } else {
            self.content.clear();
            self.mentions.clear();
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

    fn decide(&mut self, decision: ApprovalDecision, window: &mut Window, cx: &mut Context<Self>) {
        self.approval = None;
        cx.notify();
        if let Some(f) = self.on_approval.clone() {
            f(decision, window, cx);
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
            };
            self.menu_focus.prepare(count, cx);
            self.menu_focus.trigger = match menu {
                Menu::Permissions => self.permissions_focus.clone(),
                _ => self.model_focus.clone(),
            };
            self.menu_focus.content.focus(window, cx);
            cx.notify();
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
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
            if menu == Menu::Permissions || self.speed_open {
                self.menu_focus.restore(window, cx);
            } else {
                self.focus_handle.focus(window, cx);
            }
            self.close_speed();
            cx.notify();
        } else if self.approval.is_some() {
            // `approval.decline` (Escape), unless a menu has focus.
            self.decide(ApprovalDecision::Decline, window, cx);
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
fn tabular(d: Div) -> Div {
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

/// `_ComposerLayoutBody`: 22 px radius, composer surface, `--elevation-composer-*`.
pub(crate) fn card_decoration(t: &Theme) -> shapes::Decoration {
    let black = |a: u32| -> Hsla { gpui::rgba(a).into() };
    let shadows = match t.appearance {
        Appearance::Dark => vec![shadow(t.composer_inset_highlight, 0., 1., 0., true)],
        Appearance::Light => vec![
            shadow(black(0x0000000a), 0., 0., 1., false),
            shadow(black(0x0000000a), 2., 8., 0., false),
            shadow(black(0x00000006), 4., 80., 8., false),
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
        let card = div()
            .id("composer")
            .when(self.focus_handle.is_focused(window), |d| {
                d.key_context(KEY_CONTEXT)
            })
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::enter))
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
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::complete_command))
            .on_action(cx.listener(Self::next_command))
            .on_action(cx.listener(Self::previous_command))
            .relative();
        if self.plain_field.is_some() {
            return div().w_full().child(
                card.border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .bg(t.background_panel)
                    .py(px(8.))
                    .child(self.editor(cx)),
            );
        }
        let card = card
            .flex()
            .flex_col()
            .child(shapes::decoration(if editing {
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
                    ..card_decoration(&t)
                }
            } else {
                card_decoration(&t)
            }))
            // See `docked`: the docked body is a transparent cc layer over the card.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(self.docked, |d| d.layer())
                    .when(!editing, |d| d.child(self.attachments_row(&t, cx)))
                    .when_some(self.attachment_error, |d, error| {
                        d.child(label(error, t.text))
                    })
                    .child(
                        self.editor(cx)
                            .when(editing, |d| d.pt(px(12.)).pb_0().min_h(px(52.))),
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
                        self.footer(&t, cx).into_any_element()
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
                        .child(self.queue_button(
                            QueueAction::Steer(row.submission.id.clone()),
                            "Steer",
                            "Submit without interrupting the model",
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
                        .child(self.queue_menu_button(row.submission.id.clone(), &t, cx))
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
            .child(div().children(tray).child(card))
            .when_some(self.context_menu.as_ref(), |d, menu| {
                d.child(menu.overlay(
                    "composer-menu",
                    if menu.items.first().is_some_and(|item| item.id == QUEUE_EDIT) {
                        QUEUE_ACTIONS
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
                assert_eq!(composer.effort, "high");
                composer.set_model_and_effort("missing", "low", cx);
                assert_eq!(composer.current_model().unwrap().id, "announced");
                assert_eq!(composer.effort, "high");
                assert_eq!(composer.permission, Permission::FullAccess);
                assert!(composer.queue_mode);
                assert_eq!(composer.default_selection, (0, "medium".into()));
                assert_eq!(composer.content, "Keep this draft");
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
