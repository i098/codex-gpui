//! The prompt composer (`_ComposerLayoutRoot`): a multi-line editor, attachment chips, the
//! permissions and model menus, and approval prompts for server approval requests. All
//! numbers are the CSS px of refs/home and refs/composer.

use crate::{MONO, icon::icon, text};
use bridge::types::{ApprovalDecision, CommandApprovalRequest, FileChangeApprovalRequest, Model};
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ClipboardItem, Context, Corners, CursorStyle, Div,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable,
    GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathPromptOptions, Pixels, Point, Render, SharedString, Size, Stateful,
    StatefulInteractiveElement, Style, Styled, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window, WrappedLine, actions, deferred, div, fill, point, prelude::FluentBuilder as _, px,
    relative, size,
};
use serde_json::Value;
use std::{ops::Range, path::PathBuf, rc::Rc};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(
    composer,
    [
        /// Send the prompt (Enter).
        Submit,
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
        /// Close the open menu.
        Dismiss,
    ]
);

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
        KeyBinding::new("escape", Dismiss, c),
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

type SubmitHandler = Rc<dyn Fn(&Submission, &mut Window, &mut App)>;
type ApprovalHandler = Rc<dyn Fn(ApprovalDecision, &mut Window, &mut App)>;

pub struct Composer {
    focus_handle: FocusHandle,
    placeholder: SharedString,
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    is_selecting: bool,
    /// Shaped lines and editor bounds of the last paint, for hit testing.
    lines: Vec<WrappedLine>,
    bounds: Option<Bounds<Pixels>>,
    attachments: Vec<PathBuf>,
    models: Vec<ModelOption>,
    model: usize,
    effort: String,
    default_selection: (usize, String),
    fast: bool,
    permission: Permission,
    menu: Option<Menu>,
    /// The card's (`composer-footer` container's) width, CSS px: see `SM_WIDTH`, `XS_WIDTH`.
    footer_width: f32,
    /// The pointer is over the open menu (it lies outside the card's hitbox).
    menu_hovered: bool,
    approval: Option<Approval>,
    on_submit: Option<SubmitHandler>,
    on_approval: Option<ApprovalHandler>,
}

impl Composer {
    /// Home composer: placeholder `Do anything`.
    pub fn home(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new("Do anything", cx)
    }

    /// Thread composer, docked at the bottom of a thread: placeholder `Work with Codex`.
    pub fn thread(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new("Work with Codex", cx)
    }

    fn new(placeholder: &'static str, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            content: String::new(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
            lines: Vec::new(),
            bounds: None,
            attachments: Vec::new(),
            models: Vec::new(),
            model: 0,
            effort: "medium".into(),
            default_selection: (0, "medium".into()),
            fast: false,
            permission: Permission::Custom,
            menu: None,
            footer_width: f32::INFINITY,
            menu_hovered: false,
            approval: None,
            on_submit: None,
            on_approval: None,
        }
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
        cx.notify();
    }

    pub fn on_submit(&mut self, f: impl Fn(&Submission, &mut Window, &mut App) + 'static) {
        self.on_submit = Some(Rc::new(f));
    }

    pub fn on_approval(&mut self, f: impl Fn(ApprovalDecision, &mut Window, &mut App) + 'static) {
        self.on_approval = Some(Rc::new(f));
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

    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.content = s.into();
        self.selected_range = s.len()..s.len();
        self.marked_range = None;
        cx.notify();
    }

    /// Open the model picker (`Select effort`), as `⌘⇧M`-style commands and the menu do.
    pub fn open_model_picker(&mut self, cx: &mut Context<Self>) {
        self.menu = Some(Menu::Effort);
        cx.notify();
    }

    fn current_model(&self) -> Option<&ModelOption> {
        self.models.get(self.model)
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        let submission = Submission {
            text: std::mem::take(&mut self.content),
            attachments: std::mem::take(&mut self.attachments),
            model: self
                .current_model()
                .map(|m| m.id.clone())
                .unwrap_or_default(),
            effort: self.effort.clone(),
            permission: self.permission,
            fast: self.fast,
        };
        self.selected_range = 0..0;
        self.marked_range = None;
        self.menu = None;
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

    fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = (self.menu != Some(menu)).then_some(menu);
        cx.notify();
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
    }

    fn add_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| {
                    this.attachments.extend(paths);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    // Editing.

    fn cursor(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    // ponytail: char boundaries, not grapheme clusters; add unicode-segmentation if
    // combining sequences need one-step deletion.
    fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .chars()
            .next()
            .map_or(offset, |c| offset + c.len_utf8())
    }

    fn insert(&mut self, s: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, s, window, cx);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.previous_boundary(self.cursor())
        } else {
            self.selected_range.start
        };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.next_boundary(self.cursor())
        } else {
            self.selected_range.end
        };
        self.move_to(to, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(-1.).unwrap_or(0);
        self.move_to(to, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(1.).unwrap_or(self.content.len());
        self.move_to(to, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(-1.).unwrap_or(0), cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(1.).unwrap_or(self.content.len()), cx);
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.content[..self.cursor()]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        self.move_to(to, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.cursor();
        let to = self.content[at..]
            .find('\n')
            .map_or(self.content.len(), |i| at + i);
        self.move_to(to, cx);
    }

    fn select_all(&mut self, _: &crate::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn copy(&mut self, _: &crate::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            let s = self.content[self.selected_range.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(s));
        }
    }

    fn cut(&mut self, _: &crate::Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&crate::Copy, window, cx);
        self.insert("", window, cx);
    }

    fn paste(&mut self, _: &crate::Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&s, window, cx);
        }
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;
        self.menu = None;
        let at = self.index_for_point(e.position);
        if e.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_point(e.position), cx);
        }
    }

    /// Byte offset where each shaped line starts (lines are split at `\n`).
    fn line_starts(&self) -> impl Iterator<Item = (usize, Pixels, &WrappedLine)> {
        let mut start = 0;
        let mut y = px(0.);
        self.lines.iter().map(move |line| {
            let item = (start, y, line);
            start += line.len() + 1;
            y += line.size(LINE_HEIGHT).height;
            item
        })
    }

    /// Editor-local position of the caret before byte `offset`.
    fn point_for_index(&self, offset: usize) -> Option<Point<Pixels>> {
        self.line_starts()
            .find(|(start, _, line)| offset <= start + line.len())
            .and_then(|(start, y, line)| {
                let p = line.position_for_index(offset - start, LINE_HEIGHT)?;
                Some(point(p.x, p.y + y))
            })
    }

    fn index_for_local(&self, local: Point<Pixels>) -> usize {
        let mut last = 0;
        for (start, y, line) in self.line_starts() {
            let h = line.size(LINE_HEIGHT).height;
            if local.y < y + h {
                let p = point(local.x, (local.y - y).max(px(0.)));
                let i = line
                    .closest_index_for_position(p, LINE_HEIGHT)
                    .unwrap_or_else(|i| i);
                return start + i;
            }
            last = start + line.len();
        }
        last
    }

    fn index_for_point(&self, position: Point<Pixels>) -> usize {
        match (self.bounds, self.content.is_empty()) {
            (Some(b), false) => self.index_for_local(position - b.origin),
            _ => 0,
        }
    }

    /// Offset one row above (`-1`) or below (`1`) the caret; `None` past the first/last row.
    fn vertical(&self, rows: f32) -> Option<usize> {
        let p = self.point_for_index(self.cursor())?;
        let y = p.y + LINE_HEIGHT * rows + LINE_HEIGHT / 2.;
        let total: Pixels = self.lines.iter().map(|l| l.size(LINE_HEIGHT).height).sum();
        (y >= px(0.) && y < total).then(|| self.index_for_local(point(p.x, y)))
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end)
    }
}

impl Focusable for Composer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Composer {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_range = self.range_from_utf16(&range_utf16);
        self.selection_reversed = false;
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_and_mark_text_in_range(range_utf16, new_text, None, window, cx);
        self.marked_range = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content.replace_range(range.clone(), new_text);
        let end = range.start + new_text.len();
        self.marked_range = (!new_text.is_empty()).then_some(range.start..end);
        self.selected_range = match new_selected_range_utf16 {
            Some(sel) => {
                let base = self.offset_to_utf16(range.start);
                self.range_from_utf16(&(base + sel.start..base + sel.end))
            }
            None => end..end,
        };
        self.selection_reversed = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.point_for_index(range.start)?;
        let end = self.point_for_index(range.end)?;
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin + point(end.x, end.y + LINE_HEIGHT),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = p - self.bounds?.origin;
        Some(self.offset_to_utf16(self.index_for_local(local)))
    }
}

/// The text area: wraps to its width, grows one row per line.
struct Editor(Entity<Composer>);

impl IntoElement for Editor {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Shapes `content` (or the placeholder) for one paint.
fn shape(
    composer: &Composer,
    color: Hsla,
    placeholder: Hsla,
    width: Option<Pixels>,
    window: &Window,
) -> Vec<WrappedLine> {
    let style = window.text_style();
    let (s, color) = if composer.content.is_empty() {
        (composer.placeholder.to_string(), placeholder)
    } else {
        (composer.content.clone(), color)
    };
    let run = TextRun {
        len: s.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = match &composer.marked_range {
        Some(m) if !composer.content.is_empty() => vec![
            TextRun {
                len: m.start,
                ..run.clone()
            },
            TextRun {
                len: m.end - m.start,
                underline: Some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.),
                    wavy: false,
                }),
                ..run.clone()
            },
            TextRun {
                len: s.len() - m.end,
                ..run
            },
        ],
        _ => vec![run],
    };
    window
        .text_system()
        .shape_text(s.into(), FONT_SIZE, &runs, width, None)
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default()
}

impl gpui::Element for Editor {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, LayoutId) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.0.clone();
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let lines = shape(input.read(cx), gpui::black(), gpui::black(), width, window);
            let height = lines.iter().map(|l| l.size(LINE_HEIGHT).height).sum();
            Size {
                width: width.unwrap_or_default(),
                height,
            }
        });
        (id, id)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let t = cx.theme().clone();
        let lines = shape(
            self.0.read(cx),
            t.text,
            t.text_tertiary,
            Some(bounds.size.width),
            window,
        );
        // The editor spans the card minus its 12 px side padding.
        let width = f32::from(bounds.size.width) + 24.;
        let collapse = |w: f32| (w <= SM_WIDTH, w <= XS_WIDTH);
        let changed = self.0.update(cx, |c, _| {
            c.lines = lines;
            c.bounds = Some(bounds);
            collapse(std::mem::replace(&mut c.footer_width, width)) != collapse(width)
        });
        if changed {
            let entity = self.0.clone();
            window.defer(cx, move |_, cx| entity.update(cx, |_, cx| cx.notify()));
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        id: &mut LayoutId,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.0.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        let selection_color = cx.theme().text_selection;
        let caret_color = cx.theme().text;
        let c = self.0.read(cx);
        if !c.selected_range.is_empty()
            && let (Some(a), Some(b)) = (
                c.point_for_index(c.selected_range.start),
                c.point_for_index(c.selected_range.end),
            )
        {
            let w = bounds.size.width;
            let mut row = a.y;
            while row <= b.y {
                let x0 = if row == a.y { a.x } else { px(0.) };
                let x1 = if row == b.y { b.x } else { w };
                let r = Bounds::from_corners(
                    bounds.origin + point(x0, row),
                    bounds.origin + point(x1, row + LINE_HEIGHT),
                );
                window.paint_quad(fill(r, selection_color));
                row += LINE_HEIGHT;
            }
        }
        let caret = (c.selected_range.is_empty() && focus.is_focused(window))
            .then(|| c.point_for_index(c.cursor()))
            .flatten();
        let lines = self.0.update(cx, |c, _| std::mem::take(&mut c.lines));
        // Glyphs land on the unsnapped LayoutUnit x, as Blink paints text.
        let x = window.layout_origin_residual_x(*id);
        let mut y = px(0.);
        for line in &lines {
            let origin = bounds.origin + point(x, y);
            line.paint(origin, LINE_HEIGHT, TextAlign::Left, None, window, cx)
                .ok();
            y += line.size(LINE_HEIGHT).height;
        }
        self.0.update(cx, |c, _| c.lines = lines);
        if let Some(p) = caret {
            let r = Bounds::new(
                bounds.origin + point(p.x, p.y + px(1.5)),
                size(px(1.), LINE_HEIGHT - px(3.)),
            );
            window.paint_quad(fill(r, caret_color));
        }
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

/// 13/18 composer footer label.
pub fn label(s: impl Into<SharedString>, color: Hsla) -> Div {
    text(s, 13., 18., 400., color)
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
fn card_decoration(t: &Theme) -> shapes::Decoration {
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

/// Popover surface `width` CSS px wide holding `content`: `--menu-background-color` at 90%
/// over the `backdrop-filter: blur(8px)` backdrop, `--menu-corner-shape`. Chromium composites
/// it as its own layer: the blurred backdrop, then the layer (shadows, fill and content
/// rasterized into transparent tiles) over it. The layer's tile grid starts at its shadow's
/// ink (`0 8px 16px -4px`: 12 px left of the box, 4 px above).
fn popover(t: &Theme, width: f32, content: impl FnOnce(Div) -> Div) -> Div {
    // `bg-surface-elevated-secondary/90` mixes in oklab: Chromium's round trip of white lands
    // 1 below 255 in green (as in thread.rs `Tones::card`).
    let fill = match t.appearance {
        Appearance::Light => gpui::rgba(0xfffeffe6).into(),
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
    let surface = div()
        .relative()
        .w(px(width))
        .flex()
        .flex_col()
        .p(px(4.))
        .child(shapes::backdrop_blur(
            px(8.),
            shapes::Fill::Color,
            shapes::Decoration {
                radius: d.radius,
                corner: shapes::Corner::Squircle,
                ..Default::default()
            },
        ))
        .child(shapes::decoration(shapes::Decoration {
            corner: shapes::Corner::Squircle,
            ..d
        }));
    div()
        .ml(px(-12.))
        .mt(px(-4.))
        .pl(px(12.))
        .pt(px(4.))
        .layer()
        .child(content(surface))
}

impl Composer {
    fn attachments_row(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .pt(px(8.))
            .px(px(8.))
            .pb(px(6.))
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(self.attachments.iter().enumerate().map(|(i, path)| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                div()
                    .h(px(28.))
                    .pl(px(8.))
                    .pr(px(2.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .relative()
                    .child(shapes::decoration(deco(10., t.text_alpha_5, vec![])))
                    .child(label(name, t.text))
                    .child(
                        pill(("remove-attachment", i), t)
                            .size(px(22.))
                            .justify_center()
                            .child(icon("close", 12., t.text_tertiary))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.attachments.remove(i);
                                cx.notify();
                            })),
                    )
            }))
    }

    fn editor(&self, cx: &mut Context<Self>) -> Div {
        div()
            .px(px(12.))
            .pb(px(4.))
            .min_h(px(48.))
            .cursor(CursorStyle::IBeam)
            .text_size(FONT_SIZE)
            .line_height(LINE_HEIGHT)
            .font_weight(gpui::FontWeight(430.))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            // `--opacity-composer-placeholder: .5` on the placeholder.
            .child(
                div()
                    .when(self.content.is_empty(), |d| d.opacity(0.5))
                    .child(Editor(cx.entity())),
            )
    }

    /// `Select effort` popover: effort title, model row, slider of the model's efforts.
    fn effort_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(model) = self.current_model() else {
            return div();
        };
        let efforts = model.efforts.clone();
        let selected = efforts.iter().position(|e| *e == self.effort).unwrap_or(0);
        let corner = |id: &'static str, name: &'static str, color: Hsla| {
            let hover = t.ghost_hover;
            div()
                .id(id)
                .absolute()
                .top_0()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(26.))
                        .rounded(px(7.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |s| s.bg(hover))
                        .child(icon(name, 16., color)),
                )
        };
        let bolt = if self.fast { t.accent } else { t.text_tertiary };
        let header = div()
            .relative()
            .h(px(48.))
            .px(px(32.))
            .flex()
            .justify_center()
            .child(
                corner("fast-mode", "composer-bolt", bolt)
                    .left_0()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fast = !this.fast;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("select-model")
                    .p(px(4.))
                    .rounded(px(8.))
                    .child(
                        div()
                            .py(px(2.))
                            .px(px(4.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(text(
                                effort_label(&self.effort).to_string(),
                                14.,
                                20.,
                                500.,
                                t.accent,
                            ))
                            .child(
                                div()
                                    .relative()
                                    .px(px(16.))
                                    .child(text(
                                        model.label.clone(),
                                        12.,
                                        16.,
                                        400.,
                                        t.text_secondary,
                                    ))
                                    .child(
                                        icon("composer-chevron-right-small", 12., t.text_tertiary)
                                            .absolute()
                                            .right(px(4.))
                                            .top(px(2.)),
                                    ),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Models, cx))),
            )
            .child(
                corner("reset-effort", "composer-reset", t.text_tertiary)
                    .right_0()
                    .on_click(cx.listener(|this, _, _, cx| {
                        (this.model, this.effort) = this.default_selection.clone();
                        cx.notify();
                    })),
            );
        // Track 230 px; dot centers 13 + 51·i; the thumb (28 px) centers on the selected dot.
        let n = efforts.len().max(2);
        let step = (230. - 26.) / (n - 1) as f32;
        let dot_x = |i: usize| 13. + step * i as f32;

        let slider = div().py(px(4.)).child(
            div()
                .h(px(32.))
                .py(px(2.))
                .px(px(8.))
                .flex()
                .items_center()
                .child(
                    div()
                        .id("effort-slider")
                        .relative()
                        .w_full()
                        .h(px(24.))
                        .child(shapes::decoration(deco(
                            12.,
                            t.text_alpha_10,
                            vec![shadow(t.border, 0., 0., 0.5, true)],
                        )))
                        .when(selected > 0, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(px(dot_x(selected)))
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: Corners {
                                            top_left: px(12.),
                                            bottom_left: px(12.),
                                            ..Corners::all(px(0.))
                                        },
                                        fill: Some(t.accent),
                                        ..Default::default()
                                    })),
                            )
                        })
                        .children((0..efforts.len()).map(|i| {
                            let color = if i <= selected {
                                gpui::rgba(0xffffff4c).into()
                            } else {
                                Hsla { a: 0.249, ..t.text }
                            };
                            div()
                                .absolute()
                                .left(px(dot_x(i) - 2.))
                                .top(px(10.))
                                .size(px(4.))
                                .child(shapes::decoration(deco(9999., color, vec![])))
                        }))
                        .child(
                            div()
                                .absolute()
                                .left(px(dot_x(selected) - 14.))
                                .top(px(-2.))
                                .size(px(28.))
                                .child(shapes::decoration(shapes::Decoration {
                                    border: Some((px(1.), t.border_strong)),
                                    ..deco(
                                        9999.,
                                        t.control_thumb,
                                        vec![shadow(
                                            gpui::rgba(0x0000001a).into(),
                                            0.,
                                            2.,
                                            0.,
                                            false,
                                        )],
                                    )
                                })),
                        )
                        .children(efforts.iter().enumerate().map(|(i, effort)| {
                            let effort = effort.clone();
                            div()
                                .id(("effort", i))
                                .absolute()
                                .left(px(dot_x(i) - step / 2.))
                                .top_0()
                                .w(px(step))
                                .h_full()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.effort = effort.clone();
                                    cx.notify();
                                }))
                        })),
                ),
        );
        popover(t, 254., |d| d.child(header).child(slider))
    }
}

impl Composer {
    /// `Select model` list.
    fn models_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        popover(t, 254., |d| {
            d.child(div().px(px(8.)).py(px(6.)).child(text(
                "Select model",
                12.,
                16.,
                400.,
                t.text_tertiary,
            )))
            .children(self.models.iter().enumerate().map(|(i, m)| {
                div()
                    .id(("model", i))
                    .h(px(32.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(12.))
                    .when(i == self.model, |d| d.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .child(text(m.label.clone(), 14., 20., 430., t.text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model = i;
                        let efforts = &this.models[i].efforts;
                        if !efforts.contains(&this.effort)
                            && let Some(first) = efforts.first()
                        {
                            this.effort = first.clone();
                        }
                        this.menu = Some(Menu::Effort);
                        cx.notify();
                    }))
            }))
        })
    }

    /// `How should ChatGPT actions be approved?`
    fn permissions_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        popover(t, 320., |d| {
            d.child(div().px(px(8.)).py(px(6.)).child(text(
                "How should ChatGPT actions be approved?",
                12.,
                16.,
                400.,
                t.text_tertiary,
            )))
            .children(Permission::ALL.into_iter().map(|p| {
                div()
                    .id(p.short_label())
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .when(p == self.permission, |d| d.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .child(text(p.option_label(), 14., 20., 430., t.text))
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(t.text_tertiary)
                            .child(p.description()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.permission = p;
                        this.menu = None;
                        cx.notify();
                    }))
            }))
        })
    }

    /// Pending approval request card (`PendingRequestItemPanel`).
    fn approval_card(&self, a: &Approval, t: &Theme, cx: &mut Context<Self>) -> Div {
        let button = |id: &'static str, s: SharedString, bg: Hsla, fg: Hsla| {
            pill(id, t).px(px(10.)).bg(bg).child(label(s, fg))
        };
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .relative()
            .child(shapes::decoration(card_decoration(t)))
            .child(text(a.title.clone(), 14., 20., 500., t.text))
            .when_some(a.detail.clone(), |d, detail| {
                d.child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(t.code_block_background)
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(t.text)
                        .child(detail),
                )
            })
            .when_some(a.reason.clone(), |d, reason| {
                d.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(label("Reason", t.text_tertiary))
                        .child(label(reason, t.text_secondary)),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button("deny", "Deny".into(), gpui::transparent_black(), t.text).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.decide(ApprovalDecision::Decline, window, cx)
                            }),
                        ),
                    )
                    .child(
                        button(
                            "allow-session",
                            a.scoped_label.clone(),
                            t.button_secondary,
                            t.text,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.decide(ApprovalDecision::AcceptForSession, window, cx)
                        })),
                    )
                    .child(
                        button(
                            "allow-once",
                            "Allow once".into(),
                            t.button_primary_solid,
                            t.text_inverse,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.decide(ApprovalDecision::Accept, window, cx)
                        })),
                    ),
            )
    }

    /// Popover anchored above a footer button. `width`: the box the original centers on the
    /// button (`None`: left-aligned).
    fn anchored_menu(menu: Div, center_width: Option<f32>, cx: &mut Context<Self>) -> AnyElement {
        let menu = div()
            .id("menu")
            .on_hover(cx.listener(|this, hovered: &bool, _, _| this.menu_hovered = *hovered))
            .child(menu);
        let slot = div().absolute().bottom(px(36.)).flex();
        deferred(match center_width {
            Some(w) => slot
                .left_0()
                .right_0()
                .justify_center()
                .child(div().flex_none().w(px(w)).pl(px(1.)).pb(px(1.)).child(menu)),
            None => slot.left_0().child(menu),
        })
        .with_priority(1)
        .into_any_element()
    }

    fn footer(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let open = matches!(self.menu, Some(Menu::Effort | Menu::Models));
        let model = self.current_model();
        let model_label = model.map_or("Loading model…".into(), |m| m.label.clone());
        let labels = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(label(model_label, t.text).when(open, |d| d.invisible()))
            .when(model.is_some(), |d| {
                d.child(
                    label(effort_label(&self.effort).to_string(), t.text_tertiary)
                        .when(open, |d| d.invisible()),
                )
            })
            .when(open, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .justify_center()
                        .child(label("Select effort", t.text_tertiary)),
                )
            });
        let trigger = if self.footer_width <= XS_WIDTH {
            pill("model-picker", t)
                .w(px(28.))
                .justify_center()
                .child(icon("composer-model", 16., t.text_tertiary))
        } else {
            pill("model-picker", t)
                .px(px(8.))
                .gap(px(4.))
                .child(labels)
                .child(icon("chevron-down-small", 16., t.text_tertiary))
        };
        let model_button = div()
            .relative()
            .when(open, |d| {
                d.child(shapes::decoration(deco(9999., t.ghost_hover, vec![])))
            })
            .child(
                trigger.on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Effort, cx))),
            )
            .when(self.menu == Some(Menu::Effort), |d| {
                d.child(Self::anchored_menu(
                    self.effort_menu(t, cx),
                    Some(EFFORT_MENU_SLOT),
                    cx,
                ))
            })
            .when(self.menu == Some(Menu::Models), |d| {
                d.child(Self::anchored_menu(
                    self.models_menu(t, cx),
                    Some(EFFORT_MENU_SLOT),
                    cx,
                ))
            });
        let has_input = !self.content.is_empty() || !self.attachments.is_empty();
        let left = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                pill("add-files", t)
                    .w(px(28.))
                    .justify_center()
                    .child(icon("plus", 16., t.text))
                    .on_click(cx.listener(|this, _, _, cx| this.add_files(cx))),
            )
            .child(
                div()
                    .relative()
                    .child(
                        pill("permissions", t)
                            .px(px(6.))
                            .gap(px(4.))
                            .child(icon("permissions", 16., t.text_tertiary))
                            .when(self.footer_width > SM_WIDTH, |d| {
                                d.child(label(self.permission.short_label(), t.text_tertiary))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_menu(Menu::Permissions, cx)
                            })),
                    )
                    .when(self.menu == Some(Menu::Permissions), |d| {
                        d.child(Self::anchored_menu(self.permissions_menu(t, cx), None, cx))
                    }),
            );
        let right = div()
            .flex()
            .items_center()
            .child(model_button)
            .child(
                pill("dictate", t)
                    .w(px(28.))
                    .justify_center()
                    .child(icon("mic", 16., t.text)),
            )
            .child(
                div()
                    .id("send")
                    .ml(px(8.))
                    .size(px(28.))
                    .p(px(2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .relative()
                    .child(shapes::decoration(deco(9999., t.composer_primary, vec![])))
                    .child(icon(
                        if has_input {
                            "composer-arrow-up"
                        } else {
                            "voice"
                        },
                        16.,
                        t.text_composer_primary,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.submit(&Submit, window, cx))),
            );
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(8.))
            .pb(px(8.))
            .child(left)
            .child(right)
    }
}

/// Width the original's popper positions the effort menu with, centered on the model button:
/// in refs/composer its 256 px box sits 1.19 px left of the button's center, snapped to
/// device px.
const EFFORT_MENU_SLOT: f32 = 258.5;

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let card = div()
            .id("composer")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::submit))
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
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::dismiss))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.menu_hovered && this.menu.take().is_some() {
                    cx.notify();
                }
            }))
            .relative()
            .flex()
            .flex_col()
            .child(shapes::decoration(card_decoration(&t)))
            .child(self.attachments_row(&t, cx))
            .child(self.editor(cx))
            .child(self.footer(&t, cx));
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .when_some(self.approval.clone(), |d, a| {
                d.child(self.approval_card(&a, &t, cx))
            })
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_model_names() {
        assert_eq!(model_label("GPT-6-Astra"), "GPT-6 Astra");
        assert_eq!(model_label("gpt-5.6-sol"), "GPT-5.6 Sol");
        assert_eq!(model_label("gpt-oss"), "GPT-Oss");
        assert_eq!(model_label("o3"), "o3");
    }
}
