//! The prompt composer: a multi-line editor, attachment chips, slash commands,
//! permission and model menus, and approval prompts.
//! Numbers use the CSS pixels of refs/home and refs/composer.
//! Clipboard text of at least 5,000 UTF-16 units becomes a saved text attachment.
//! Clipboard images retain their bytes and show a thumbnail.

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

    pub(super) struct MenuFocus {
        pub content: FocusHandle,
        pub items: Vec<FocusHandle>,
        pub trigger: FocusHandle,
    }

    impl MenuFocus {
        pub fn new(cx: &mut App) -> Self {
            Self {
                content: cx.focus_handle(),
                items: Vec::new(),
                trigger: cx.focus_handle(),
            }
        }

        pub fn prepare(&mut self, count: usize, cx: &mut App) {
            self.items.resize_with(count, || cx.focus_handle());
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

type SubmitHandler = Rc<dyn Fn(&Submission, &mut Window, &mut App)>;
type ApprovalHandler = Rc<dyn Fn(ApprovalDecision, &mut Window, &mut App)>;
/// `(model id, effort)` after the user picks them.
type ModelHandler = Rc<dyn Fn(&str, &str, &mut App)>;

pub struct Composer {
    focus_handle: FocusHandle,
    placeholder: SharedString,
    /// Docked under a thread: `_ComposerLayoutRoot` paints the card and `_ComposerLayoutBody`
    /// (the content) is a `will-change: opacity` layer. On Home the body paints the card.
    docked: bool,
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    is_selecting: bool,
    /// Shaped lines and editor bounds of the last paint, for hit testing.
    lines: Vec<WrappedLine>,
    bounds: Option<Bounds<Pixels>>,
    attachments: Vec<attach::Attachment>,
    pending_attachments: usize,
    attachment_error: Option<&'static str>,
    models: Vec<ModelOption>,
    model: usize,
    effort: String,
    default_selection: (usize, String),
    fast: bool,
    permission: Permission,
    menu: Option<Menu>,
    slash: slash::Slash,
    menu_focus: menu::MenuFocus,
    permissions_focus: FocusHandle,
    model_focus: FocusHandle,
    /// The card's (`composer-footer` container's) width, CSS px: see `SM_WIDTH`, `XS_WIDTH`.
    footer_width: f32,
    menu_trigger_hovered: bool,
    approval: Option<Approval>,
    on_submit: Option<SubmitHandler>,
    on_approval: Option<ApprovalHandler>,
    on_model: Option<ModelHandler>,
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

    fn new(placeholder: &'static str, docked: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            docked,
            content: String::new(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
            lines: Vec::new(),
            bounds: None,
            attachments: Vec::new(),
            pending_attachments: 0,
            attachment_error: None,
            models: Vec::new(),
            model: 0,
            effort: "medium".into(),
            default_selection: (0, "medium".into()),
            fast: false,
            permission: Permission::Custom,
            menu: None,
            slash: slash::Slash::default(),
            menu_focus: menu::MenuFocus::new(cx),
            permissions_focus: cx.focus_handle(),
            model_focus: cx.focus_handle(),
            footer_width: f32::INFINITY,
            menu_trigger_hovered: false,
            approval: None,
            on_submit: None,
            on_approval: None,
            on_model: None,
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
        if self.menu == Some(Menu::Models) {
            self.menu_focus.prepare(self.models.len(), cx);
        }
        cx.notify();
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
        self.slash.reset();
        cx.notify();
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

    /// Reports the picked model and effort to [`Self::on_model`].
    fn picked(&self, cx: &mut App) {
        if let (Some(f), Some(model)) = (self.on_model.clone(), self.current_model()) {
            f(&model.id, &self.effort, cx);
        }
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash.has_selection() {
            self.activate_slash(window, cx);
            return;
        }
        if self.pending_attachments > 0
            || (self.content.trim().is_empty() && self.attachments.is_empty())
        {
            return;
        }
        let submission = Submission {
            text: std::mem::take(&mut self.content),
            attachments: self.attachments.drain(..).map(|a| a.path).collect(),
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
        self.slash.reset();
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
        if self.menu == Some(menu) {
            self.dismiss(&Dismiss, window, cx);
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
        if self.slash.open {
            self.close_slash(cx);
            return;
        }
        if self.menu.take().is_some() {
            self.menu_focus.restore(window, cx);
            cx.notify();
        }
    }

    fn menu_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "down" | "up" => self
                .menu_focus
                .move_focus(event.keystroke.key == "down", window, cx),
            "escape" => self.dismiss(&Dismiss, window, cx),
            "enter" | "space" => {
                if let Some(index) = self.menu_focus.focused_item(window) {
                    self.activate_menu_item(index, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
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
                self.permission = Permission::ALL[index];
                self.dismiss(&Dismiss, window, cx);
            }
            Some(Menu::Models) => {
                self.model = index;
                let efforts = &self.models[index].efforts;
                if !efforts.contains(&self.effort)
                    && let Some(first) = efforts.first()
                {
                    self.effort = first.clone();
                }
                self.toggle_menu(Menu::Effort, window, cx);
                self.picked(cx);
            }
            Some(Menu::Effort) => match index {
                0 => self.fast = !self.fast,
                1 => self.toggle_menu(Menu::Models, window, cx),
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

    pub fn permission(&self) -> Permission {
        self.permission
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

/// 13/18 composer footer label. Its text blends like a transparent cc layer onto the final
/// pill/bar pixel (the fills under it do not).
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
        let t = cx.theme().clone();
        let card = div()
            .id("composer")
            .when(self.focus_handle.is_focused(window), |d| {
                d.key_context(KEY_CONTEXT)
            })
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
            .on_action(cx.listener(Self::complete_command))
            .on_action(cx.listener(Self::next_command))
            .on_action(cx.listener(Self::previous_command))
            .relative()
            .flex()
            .flex_col()
            .child(shapes::decoration(card_decoration(&t)))
            // See `docked`: the docked body is a transparent cc layer over the card.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(self.docked, |d| d.layer())
                    .child(self.attachments_row(&t, cx))
                    .when_some(self.attachment_error, |d, error| {
                        d.child(label(error, t.text))
                    })
                    .child(self.editor(cx))
                    .child(self.footer(&t, cx)),
            );
        let card = card.when(self.slash.open, |d| d.child(self.slash_menu(&t, cx)));
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
