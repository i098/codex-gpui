use crate::{
    SANS,
    composer::{Composer, popover},
    focus,
    icon::icon,
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, prelude::FluentBuilder as _,
    px,
};
use serde_json::Value;
use std::sync::Arc;
use theme::ActiveTheme as _;

#[derive(Clone, Debug)]
pub struct MemorySettings {
    pub available: bool,
    pub feature_enabled: bool,
    pub generate: bool,
    pub use_memories: bool,
    pub disable_on_external_context: bool,
    pub file_path: Option<String>,
}

impl MemorySettings {
    pub fn enabled(&self) -> bool {
        self.feature_enabled && self.generate && self.use_memories
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MemoryChange {
    Enabled(bool),
    ToolAssisted(bool),
}

pub struct WritingPlugin {
    pub id: String,
}

pub struct WritingPreference {
    pub account_id: String,
    pub user_id: String,
    pub enabled: bool,
    pub installed: bool,
    pub can_install: bool,
    pub plugin: Option<WritingPlugin>,
}

enum WritingState {
    Loading,
    Hidden,
    Failed,
    Ready(Arc<WritingPreference>),
}

#[derive(Clone)]
pub enum Event {
    Load(u64),
    Save(String),
    Leave,
    Section(&'static str),
    LoadMemory,
    Memory(MemoryChange),
    DeleteMemory,
    LoadWriting,
    Writing(bool),
}

pub struct Personalization {
    opened: bool,
    loading: bool,
    saving: bool,
    load_error: bool,
    load_generation: u64,
    has_override: bool,
    unified_instructions: bool,
    show_source: bool,
    saved: String,
    editor: Option<Entity<Composer>>,
    memory_requested: bool,
    memory: Option<Result<MemorySettings, String>>,
    memory_pending: Option<MemoryChange>,
    host_menu: bool,
    writing: WritingState,
    writing_pending: bool,
    writing_error: bool,
    writing_retry: Option<bool>,
}

impl EventEmitter<Event> for Personalization {}

impl Personalization {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            opened: false,
            loading: false,
            saving: false,
            load_error: false,
            load_generation: 0,
            has_override: false,
            unified_instructions: false,
            show_source: false,
            saved: String::new(),
            editor: None,
            memory_requested: false,
            memory: None,
            memory_pending: None,
            host_menu: false,
            writing: WritingState::Loading,
            writing_pending: false,
            writing_error: false,
            writing_retry: None,
        }
    }

    pub fn dirty(&self, cx: &App) -> bool {
        self.opened
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.read(cx).field_text() != self.saved)
    }

    pub fn saving(&self) -> bool {
        self.saving
    }

    pub fn opened(&self) -> bool {
        self.opened
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    pub fn editor_focus(&self, cx: &App) -> Option<gpui::FocusHandle> {
        self.editor.as_ref().map(|editor| editor.focus_handle(cx))
    }

    pub fn set_unified_instructions(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.unified_instructions = enabled;
        if let Some(editor) = &self.editor {
            editor.update(cx, |editor, cx| editor.set_document_style(enabled, cx));
        }
        cx.notify();
    }
    pub fn memory(&self) -> Option<&MemorySettings> {
        self.memory.as_ref()?.as_ref().ok()
    }

    pub fn memory_loaded(
        &mut self,
        result: Result<MemorySettings, String>,
        cx: &mut Context<Self>,
    ) {
        self.memory = Some(result);
        self.memory_pending = None;
        cx.notify();
    }

    fn reload_memory(&mut self, cx: &mut Context<Self>) {
        self.memory = None;
        cx.emit(Event::LoadMemory);
        cx.notify();
    }

    fn change_memory(&mut self, change: MemoryChange, cx: &mut Context<Self>) {
        if self.memory_pending.is_some() {
            return;
        }
        self.memory_pending = Some(change);
        cx.emit(Event::Memory(change));
        cx.notify();
    }

    pub fn can_retry_writing(&self) -> bool {
        self.writing_error && !self.writing_pending
    }

    pub fn writing_preference(&self) -> Option<Arc<WritingPreference>> {
        match &self.writing {
            WritingState::Ready(preference) => Some(preference.clone()),
            _ => None,
        }
    }

    pub fn writing_loaded(
        &mut self,
        result: Result<Option<WritingPreference>, String>,
        cx: &mut Context<Self>,
    ) {
        self.writing_error = result.is_err();
        self.writing = match result {
            Ok(Some(preference)) => WritingState::Ready(Arc::new(preference)),
            Ok(None) => WritingState::Hidden,
            Err(_) => WritingState::Failed,
        };
        self.writing_pending = false;
        self.writing_retry = None;
        cx.notify();
    }

    pub fn writing_changed(
        &mut self,
        result: Result<Option<WritingPreference>, String>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.writing_error = error.is_some() || result.is_err();
        match result {
            Ok(Some(preference)) => self.writing = WritingState::Ready(Arc::new(preference)),
            Ok(None) => self.writing = WritingState::Hidden,
            Err(_) => {}
        }
        self.writing_pending = false;
        if !self.writing_error {
            self.writing_retry = None;
        }
        cx.notify();
    }

    pub fn writing_awaiting_authorization(&mut self, cx: &mut Context<Self>) {
        self.writing_pending = true;
        self.writing_error = false;
        cx.notify();
    }

    fn change_writing(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.writing_pending {
            return;
        }
        self.writing_pending = true;
        self.writing_error = false;
        self.writing_retry = Some(enabled);
        cx.emit(Event::Writing(enabled));
        cx.notify();
    }

    fn retry_writing(&mut self, cx: &mut Context<Self>) {
        if let Some(enabled) = self.writing_retry {
            self.change_writing(enabled, cx);
        } else {
            self.writing = WritingState::Loading;
            self.writing_error = false;
            cx.emit(Event::LoadWriting);
            cx.notify();
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.opened = true;
        self.loading = true;
        self.load_error = false;
        self.has_override = false;
        self.show_source = false;
        self.load_generation = self.load_generation.wrapping_add(1);
        self.editor = None;
        cx.emit(Event::Load(self.load_generation));
        cx.notify();
    }

    pub fn loaded(
        &mut self,
        generation: u64,
        result: Result<Value, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.opened || generation != self.load_generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(value) if value["contents"].is_string() && value["path"].is_string() => {
                self.has_override = value["hasOverride"].as_bool().unwrap_or(false);
                self.saved = value["contents"].as_str().unwrap().to_owned();
                let editor = cx.new(|cx| {
                    Composer::settings_field("Add your custom instructions…", &self.saved, true, cx)
                        .with_field_min_height(px(241.40625))
                });
                editor.update(cx, |editor, cx| {
                    editor.set_document_style(self.unified_instructions, cx)
                });
                cx.observe(&editor, |_, _, cx| cx.notify()).detach();
                window.focus(&editor.focus_handle(cx), cx);
                self.editor = Some(editor);
            }
            _ => self.load_error = true,
        }
        cx.notify();
    }

    pub fn discard(&mut self, cx: &mut Context<Self>) {
        self.opened = false;
        self.loading = false;
        self.editor = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || !self.dirty(cx) {
            return;
        }
        let contents = self
            .editor
            .as_ref()
            .unwrap()
            .read(cx)
            .field_text()
            .to_owned();
        self.saving = true;
        cx.emit(Event::Save(contents));
        cx.notify();
    }

    pub fn saved(&mut self, contents: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.saved = contents;
        if !self.dirty(cx) {
            self.discard(cx);
        }
        cx.notify();
    }

    pub fn save_failed(&mut self, cx: &mut Context<Self>) {
        self.saving = false;
        cx.notify();
    }
}

fn preference_switch(
    label: &'static str,
    checked: bool,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let focus = focus::handle(label, 0, window, cx);
    focus.update(cx, |focus, _| focus.handle.tab_stop = !disabled);
    focus::ring(
        super::switch(label, checked, disabled, &cx.theme().clone())
            .role(gpui::Role::Switch)
            .aria_label(label)
            .aria_disabled(disabled)
            .aria_toggled(if checked {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            }),
        &focus,
        theme::Theme::RADIUS_FULL,
        px(2.),
        cx,
    )
    .when(!disabled, |button| {
        button.on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
    })
}

fn control(
    (id, label): (&'static str, &'static str),
    fill: Hsla,
    color: Hsla,
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let focus = focus::handle(id, 0, window, cx);
    focus.update(cx, |focus, _| focus.handle.tab_stop = enabled);
    let handle = focus.read(cx).handle.clone();
    let surface = super::surface(
        12.5,
        shapes::Corner::Squircle,
        fill,
        (label == "Save").then_some(cx.theme().border),
    );
    // `disabled:opacity-40`: the compositor blends the surface's fill and border as one
    // group at 102/255 with its integer alpha scaling, not each paint at 40%.
    let surface: AnyElement = if enabled {
        shapes::decoration(surface).into_any_element()
    } else {
        canvas(
            |_, _, _| {},
            move |b, (), window, cx| {
                shapes::with_composited_opacity(102, b, window, cx, |window| {
                    shapes::paint(&surface, None, b, window, cx)
                })
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    };
    let opacity = if enabled { 1. } else { 0.4 };
    let button = div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .track_focus(&handle)
        .h(px(28.))
        .px(px(9.))
        .flex()
        .gap(px(4.))
        .items_center()
        .justify_center()
        .relative()
        .child(surface)
        .when(label == "Back", |d| {
            d.child(icon("instructions-back", 16., color.opacity(opacity)))
        })
        .child(text(label, 13., 18., 430., color).opacity(opacity));
    if !enabled {
        return button;
    }
    focus::ring(button.relative(), &focus, px(12.5), px(2.), cx)
        .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
}

fn document_control(
    (id, label): (&'static str, &'static str),
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let t = cx.theme().clone();
    let focus = focus::handle(id, 0, window, cx);
    focus.update(cx, |focus, _| focus.handle.tab_stop = enabled);
    let handle = focus.read(cx).handle.clone();
    let primary = label == "Save" && enabled;
    let fill = if primary {
        t.button_primary_solid
    } else {
        t.surface_elevated_secondary
    };
    let color = if !enabled {
        t.text_tertiary
    } else if primary {
        t.text_inverse
    } else {
        t.text
    };
    focus::ring(
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .track_focus(&handle)
            .relative()
            .h(px(32.))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .child(shapes::backdrop_blur(
                px(24.),
                shapes::Fill::Gradient,
                false,
                shapes::Decoration {
                    radius: gpui::Corners::all(px(9999.)),
                    corner: shapes::Corner::Round,
                    fill: Some(fill),
                    shadows: vec![
                        crate::composer::shadow(t.border, 0., 0., 0.5, false),
                        crate::composer::shadow(t.border, 0., 0., 0.5, false),
                        crate::composer::shadow(gpui::black().opacity(0.12), 8., 16., -4., false),
                    ],
                    ..Default::default()
                },
            ))
            .child(text(label, 13., 20., 430., color)),
        &focus,
        px(9999.),
        px(2.),
        cx,
    )
    .when(enabled, |d| {
        d.on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
    })
}

fn preference_group<const N: usize>(rows: [Div; N], t: &theme::Theme) -> Div {
    let border = crate::over(t.border, t.background_panel);
    div()
        .relative()
        .flex()
        .flex_col()
        .p(px(1.))
        .child(super::on_card(
            20.,
            shapes::Corner::Squircle,
            t.background_panel,
            Some(t.border),
        ))
        .child(crate::rounded_clip(
            px(19.),
            shapes::Corner::Squircle,
            div()
                .flex()
                .flex_col()
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    row.when(index + 1 < N, |row| {
                        row.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left(px(16.))
                                .right(px(16.))
                                .h(px(1.))
                                .bg(border),
                        )
                    })
                })),
        ))
}

impl Personalization {
    fn memory_toggle(
        &self,
        label: &'static str,
        checked: bool,
        disabled: bool,
        change: MemoryChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        preference_switch(label, checked, disabled, window, cx)
            .when(!disabled, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| this.change_memory(change, cx)))
            })
            .into_any_element()
    }

    fn memory_host(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let focus = focus::handle("memory-host", 0, window, cx);
        let menu = self.host_menu.then(|| {
            div()
                .absolute()
                .top(px(32.))
                .right_0()
                .child(popover(&t, 160., |menu| {
                    menu.child(
                        text("Local", 13., 18., 430., t.text)
                            .id("memory-host-local")
                            .role(gpui::Role::MenuItemRadio)
                            .aria_toggled(gpui::Toggled::True)
                            .tab_index(0)
                            .px(px(10.))
                            .py(px(8.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.host_menu = false;
                                cx.notify();
                            })),
                    )
                }))
        });
        focus::ring(
            super::select(
                "Local",
                Some(icon("memory-computer", 16., t.text).into_any_element()),
                false,
                self.host_menu,
                &t,
            )
            .id("memory-host")
            .relative()
            .aria_label("Memory computer")
            .aria_expanded(self.host_menu),
            &focus,
            px(10.),
            px(2.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
        .on_click(cx.listener(|this, _, _, cx| {
            this.host_menu = !this.host_menu;
            cx.notify();
        }))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if this.host_menu {
                this.host_menu = false;
                cx.notify();
            }
        }))
        .children(menu)
        .into_any_element()
    }

    fn memory_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let header = div()
            .h(px(53.))
            .pb(px(12.))
            .flex()
            .justify_between()
            .items_start()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text("Codex memory", 14., 21., 500., t.text))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(text(
                                "Configure how Codex manages memory for Local. ",
                                13.,
                                18.,
                                400.,
                                t.text_secondary,
                            ))
                            .child(
                                text("Learn more", 13., 18., 400., t.text_accent)
                                    .id("memory-help")
                                    .role(gpui::Role::Link)
                                    .tab_index(0)
                                    .on_click(|_, _, cx| {
                                        cx.open_url("https://developers.openai.com/codex/memories")
                                    }),
                            ),
                    ),
            )
            .child(self.memory_host(window, cx));
        let flags = self.memory().map(|memory| {
            (
                memory.available,
                memory.enabled(),
                memory.feature_enabled,
                !memory.disable_on_external_context,
            )
        });
        let body = if self.memory.is_none() {
            div()
                .id("memory-loading")
                .role(gpui::Role::Status)
                .aria_label("Loading Codex memory settings")
                .child(preference_group(
                    std::array::from_fn::<_, 3, _>(|_| {
                        div()
                            .h(px(60.5714))
                            .px(px(16.))
                            .py(px(12.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .w(px(160.))
                                    .h(px(12.))
                                    .rounded(px(4.))
                                    .bg(t.text_alpha_10),
                            )
                            .child(
                                div()
                                    .w(px(420.))
                                    .h(px(10.))
                                    .rounded(px(4.))
                                    .bg(t.text_alpha_5),
                            )
                    }),
                    &t,
                ))
        } else if self.memory.as_ref().is_some_and(Result::is_err) {
            div()
                .id("memory-load-error")
                .role(gpui::Role::Alert)
                .flex()
                .items_center()
                .justify_between()
                .child(text(
                    "Unable to load Codex memory settings",
                    13.,
                    18.,
                    430.,
                    t.text_secondary,
                ))
                .child(
                    control(
                        ("memory-retry", "Retry"),
                        t.text_alpha_5,
                        t.text,
                        true,
                        window,
                        cx,
                    )
                    .aria_label("Retry memory")
                    .on_click(cx.listener(|this, _, _, cx| this.reload_memory(cx))),
                )
        } else if let Some((true, enabled, feature, tools)) = flags {
            let pending = self.memory_pending;
            let busy = pending.is_some();
            let enabled = match pending {
                Some(MemoryChange::Enabled(value)) => value,
                _ => enabled,
            };
            let tools = match pending {
                Some(MemoryChange::ToolAssisted(value)) => value,
                _ => tools,
            };
            let enable = self.memory_toggle(
                "Enable Codex memories",
                enabled,
                busy,
                MemoryChange::Enabled(!enabled),
                window,
                cx,
            );
            let tools = self.memory_toggle(
                "Allow memories from tool-assisted chats",
                tools,
                busy || !feature,
                MemoryChange::ToolAssisted(!tools),
                window,
                cx,
            );
            let delete = control(
                ("memory-delete", "Delete"),
                t.text_error.opacity(0.1),
                t.text_error,
                !busy,
                window,
                cx,
            )
            .when(!busy, |button| {
                button.on_click(cx.listener(|_, _, _, cx| cx.emit(Event::DeleteMemory)))
            })
            .into_any_element();
            div().id("memory-ready").child(preference_group([
                super::settings_row("Enable Codex memories",
                    Some("Create memories from chats on Local and use them to personalize future chats on Local"), enable, &t),
                super::settings_row("Allow memories from tool-assisted chats",
                    Some("Generate memories from chats that used MCP tools or web search"), tools, &t),
                super::settings_row("Delete Codex memories",
                    Some("Delete all Codex memories for Local"), delete, &t),
            ], &t))
        } else {
            text(
                "Codex memory isn't available on this computer",
                13.,
                18.,
                430.,
                t.text_secondary,
            )
            .id("memory-unavailable")
        };
        div().flex().flex_col().child(header).child(body)
    }
}

impl Personalization {
    fn writing_section(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        if matches!(self.writing, WritingState::Hidden) {
            return None;
        }
        let t = cx.theme().clone();
        let pending = self.writing_pending || matches!(self.writing, WritingState::Loading);
        let preference = match &self.writing {
            WritingState::Ready(preference) => Some(preference),
            _ => None,
        };
        let enabled = preference.is_some_and(|preference| preference.enabled);
        let disabled =
            pending || !enabled && !preference.is_some_and(|preference| preference.can_install);
        let toggle = preference_switch("Reference my writing style", enabled, disabled, window, cx)
            .when(!disabled, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| {
                    this.change_writing(!enabled, cx);
                }))
            });
        let toggle_control = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .when(pending, |control| {
                control.child(
                    div()
                        .id("writing-pending")
                        .role(gpui::Role::Status)
                        .aria_label("Checking writing style setting…")
                        .child(icon("sidebar-cmd-spinner", 16., t.text_secondary)),
                )
            })
            .child(toggle)
            .into_any_element();
        let error = self.can_retry_writing().then(|| {
            div()
                .id("writing-error")
                .role(gpui::Role::Alert)
                .h(px(28.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(text(
                    "Writing style couldn't be updated. Try again.",
                    16.,
                    24.,
                    430.,
                    t.text,
                ))
                .child(
                    control(
                        ("writing-retry", "Retry"),
                        t.text_alpha_5,
                        t.text,
                        true,
                        window,
                        cx,
                    )
                    .aria_label("Retry updating writing style")
                    .on_click(cx.listener(|this, _, _, cx| this.retry_writing(cx))),
                )
        });
        Some(div().flex().flex_col()
            .child(div().h(px(46.)).pb(px(6.)).flex().items_center()
                .child(text("Writing", 16., 224. / 9., 500., t.text)))
            .child(preference_group([super::settings_row("Reference my writing style",
                Some("Let ChatGPT use your chats, Library files, and connected apps to write in your style"),
                toggle_control, &t)], &t))
            .children(error))
    }

    /// The editor's Back, mode, Cancel and Save controls; an empty bar until it opens.
    fn toolbar(&self, document: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if !self.opened {
            return div().h(px(46.)).flex_none();
        }
        let t = cx.theme().clone();
        div()
            .h(px(if document { 48. } else { 46. }))
            .flex_none()
            .px(px(if document { 8. } else { 20. }))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        control(
                            ("instructions-back", "Back"),
                            gpui::transparent_black(),
                            t.text,
                            !self.saving,
                            window,
                            cx,
                        )
                        .when(!self.saving, |d| {
                            d.on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Leave)))
                        }),
                    )
                    .when(document, |d| {
                        d.child(text("AGENTS.md", 13., 130. / 7., 500., t.text))
                    }),
            )
            .child(self.editor_actions(document, window, cx))
    }

    /// The Markdown file's View source / View preview switch.
    fn mode_control(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = if self.show_source {
            "View preview"
        } else {
            "View source"
        };
        document_control(("instructions-mode", label), !self.saving, window, cx).when(
            !self.saving,
            |d| {
                d.on_click(cx.listener(|this, _, window, cx| {
                    this.show_source = !this.show_source;
                    if this.show_source
                        && let Some(editor) = &this.editor
                    {
                        window.focus(&editor.focus_handle(cx), cx);
                    }
                    cx.notify();
                }))
            },
        )
    }

    /// The open editor's mode switch, Cancel and Save.
    fn editor_actions(&self, document: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let button_text = match t.appearance {
            theme::Appearance::Dark => gpui::rgb(0x2d2d2d).into(),
            theme::Appearance::Light => t.text_inverse,
        };
        let save = self.editor.is_some() && !self.loading && !self.saving && self.dirty(cx);
        div()
            .flex()
            .gap(px(if document { 6. } else { 8. }))
            .when(self.unified_instructions, |d| {
                d.child(self.mode_control(window, cx))
            })
            .child(
                if document {
                    document_control(("instructions-cancel", "Cancel"), !self.saving, window, cx)
                } else {
                    control(
                        ("instructions-cancel", "Cancel"),
                        t.text_alpha_5,
                        t.text,
                        !self.saving,
                        window,
                        cx,
                    )
                }
                .when(!self.saving, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.discard(cx)))
                }),
            )
            .child(
                if document {
                    document_control(("instructions-save", "Save"), save, window, cx)
                } else {
                    control(
                        ("instructions-save", "Save"),
                        t.button_primary_solid,
                        button_text,
                        save,
                        window,
                        cx,
                    )
                }
                .when(save, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                }),
            )
    }

    /// The override notice above an open personal instructions file.
    fn override_notice(&self, t: &theme::Theme) -> Option<Stateful<Div>> {
        if !self.has_override {
            return None;
        }
        let warning_surface: Hsla = match t.appearance {
            theme::Appearance::Dark => gpui::rgb(0x281105).into(),
            theme::Appearance::Light => gpui::rgb(0xfff5f0).into(),
        };
        Some(
            div()
                .id("instructions-override")
                .relative()
                .py(px(16.))
                .pl(px(20.))
                .pr(px(12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(25.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.surface),
                    shadows: vec![
                        crate::composer::shadow(t.border_strong, 0., 0., 0.5, false),
                        crate::composer::shadow(gpui::black().opacity(0.05), 0., 2., 0., false),
                        crate::composer::shadow(gpui::black().opacity(0.02), 4., 6., 0., false),
                    ],
                    ..Default::default()
                }))
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(25.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(warning_surface.opacity(0.3)),
                    ..Default::default()
                }))
                .child(
                    div()
                        .pt(px(2.))
                        .child(icon("instructions-warning", 18., t.text_warning)),
                )
                .child(
                    text(
                        "These instructions are overridden by AGENTS.override.md",
                        13.,
                        21.125,
                        430.,
                        t.text,
                    )
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal(),
                ),
        )
    }

    /// The closed page: Memory, the custom instructions entry and Writing.
    fn entry_page(
        &mut self,
        description: Div,
        instructions_title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let focus = focus::handle("Edit Codex custom instructions", 0, window, cx);
        let row = focus::ring(
            div()
                .id("instructions-entry")
                .role(gpui::Role::Button)
                .aria_label("Edit Codex custom instructions")
                .relative()
                .flex()
                .items_center()
                .gap(px(24.))
                .px(px(17.))
                .py(px(13.))
                .child(super::on_card(
                    20.,
                    shapes::Corner::Squircle,
                    t.background_panel,
                    Some(t.border),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(instructions_title, 13., 130. / 7., 500., t.text))
                        .child(description),
                )
                .child(icon("chevron-right", 16., t.text)),
            &focus,
            px(20.),
            px(0.),
            cx,
        )
        .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
        .on_click(cx.listener(|this, _, _, cx| this.load(cx)));
        let memory = self.memory_section(window, cx);
        let writing = self.writing_section(window, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(33.))
            .child(div().h(px(36.)).flex().items_center().child(text(
                "Personalization",
                28.,
                33.6,
                500.,
                t.text,
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(40.))
                    .child(memory)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .h(px(46.))
                                    .pb(px(6.))
                                    .flex()
                                    .items_center()
                                    .child(text("Custom instructions", 14., 21., 500., t.text)),
                            )
                            .child(row),
                    )
                    .children(writing),
            )
    }

    /// The open editor's page: its heading over the override notice and the loading state,
    /// load error or editor.
    fn editor_page(
        &self,
        description: Div,
        instructions_title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let document = self.unified_instructions;
        let body = div()
            .flex()
            .flex_col()
            .gap(px(33.))
            .when(document, |d| d.h_full())
            .when(!document, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(text(instructions_title, 28., 33.6, 500., t.text))
                        .child(description),
                )
            });
        let mut editor_body = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(document, |d| d.h_full())
            .children(self.override_notice(&t));
        if self.loading {
            editor_body = editor_body.child(
                div()
                    .id("instructions-loading")
                    .role(gpui::Role::Status)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("sidebar-cmd-spinner", 16., t.text_secondary))
                    .child(text(
                        "Loading instructions…",
                        14.,
                        20.,
                        430.,
                        t.text_secondary,
                    )),
            );
        } else if self.load_error {
            editor_body = editor_body.child(
                div()
                    .id("instructions-load-error")
                    .role(gpui::Role::Alert)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text(
                        "Unable to load instructions",
                        14.,
                        20.,
                        430.,
                        t.text_secondary,
                    ))
                    .child(
                        control(
                            ("instructions-retry", "Retry"),
                            t.list_hover,
                            t.text,
                            true,
                            window,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    ),
            );
        } else if let Some(editor) = self.editor.clone() {
            editor_body = editor_body.child(if self.unified_instructions && !self.show_source {
                let source = editor.read(cx).field_text();
                crate::screens::thread::view::markdown_preview(source, window, &t)
                    .into_any_element()
            } else {
                div()
                    .when(document, |d| d.h_full())
                    .child(editor)
                    .into_any_element()
            });
        }
        body.child(editor_body)
    }
}

impl Render for Personalization {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.opened && !self.memory_requested {
            self.memory_requested = true;
            cx.emit(Event::LoadMemory);
            cx.emit(Event::LoadWriting);
        }
        let t = cx.theme().clone();
        let instructions_title = if self.unified_instructions {
            "AGENTS.md"
        } else {
            "Codex instructions"
        };
        let (description_size, description_line_height) =
            if self.opened { (13., 24.) } else { (12., 16.) };
        let description = div().flex().items_center()
            .child(text("Edit the AGENTS.md file on the selected machine. Repository instructions may also apply. ", description_size, description_line_height, 430., t.text_secondary))
            .child(text("Learn more", description_size, description_line_height, 430., t.text_accent)
                .id("instructions-help")
                .tab_index(0)
                .role(gpui::Role::Link)
                .hover(|style| style.underline())
                .focus_visible(|style| style.border_2().border_color(gpui::rgb(0x3a83f7)))
                .on_click(|_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url("https://developers.openai.com/codex/guides/agents-md/#create-global-guidance");
                }));
        let document = self.opened && self.unified_instructions;
        let toolbar = self.toolbar(document, window, cx);
        // The open document fills a wider column without side padding.
        let (top, side, width) = if document {
            (40., 0., 768.)
        } else {
            (28., 20., 728.)
        };
        let content = if !self.opened {
            self.entry_page(description, instructions_title, window, cx)
        } else {
            self.editor_page(description, instructions_title, window, cx)
        };
        div()
            .size_full()
            .font_family(SANS)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.host_menu {
                    this.host_menu = false;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .id("personalization-page")
                    .overflow_y_scroll()
                    .flex_1()
                    .pt(px(top))
                    .px(px(side))
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(width))
                            .when(document, |d| d.h_full())
                            .child(content),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_instruction_reads_preserve_the_current_draft() -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut app = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
        });
        let window = app.open_window(gpui::size(px(1280.), px(832.)), |_, cx| {
            cx.new(Personalization::new)
        })?;
        window.update(&mut app, |view, window, cx| {
            view.load(cx);
            let old_generation = view.load_generation;
            view.discard(cx);
            view.load(cx);
            let current_generation = view.load_generation;
            view.loaded(
                current_generation,
                Ok(serde_json::json!({"path": "/fixture/AGENTS.md", "contents": "Saved instructions."})),
                window,
                cx,
            );
            view.editor.as_ref().unwrap().update(cx, |editor, cx| {
                editor.set_text("Current unsaved draft.", cx)
            });
            view.loaded(
                old_generation,
                Ok(serde_json::json!({"path": "/fixture/AGENTS.md", "contents": "Old instructions."})),
                window,
                cx,
            );
            assert_eq!(view.editor.as_ref().unwrap().read(cx).field_text(), "Current unsaved draft.");
            assert!(view.dirty(cx));
            view.discard(cx);
            view.loaded(
                current_generation,
                Ok(serde_json::json!({"path": "/fixture/AGENTS.md", "contents": "Late instructions."})),
                window,
                cx,
            );
            assert!(!view.opened());
            assert!(view.editor.is_none());
        })?;
        Ok(())
    }
}
