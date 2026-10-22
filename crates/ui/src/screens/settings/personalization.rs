use crate::{SANS, composer::Composer, focus, icon::icon, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, prelude::FluentBuilder as _,
    px,
};
use serde_json::Value;
use theme::ActiveTheme as _;

#[derive(Clone)]
pub enum Event {
    Load(u64),
    Save(String),
    Leave,
    Section(&'static str),
}

pub struct Personalization {
    opened: bool,
    loading: bool,
    saving: bool,
    load_error: bool,
    load_generation: u64,
    saved: String,
    editor: Option<Entity<Composer>>,
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
            saved: String::new(),
            editor: None,
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

    fn load(&mut self, cx: &mut Context<Self>) {
        self.opened = true;
        self.loading = true;
        self.load_error = false;
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
                self.saved = value["contents"].as_str().unwrap().to_owned();
                let editor = cx.new(|cx| {
                    Composer::settings_field("Add your custom instructions…", &self.saved, true, cx)
                        .with_field_min_height(px(241.40625))
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

fn control(
    label: &'static str,
    fill: Hsla,
    color: Hsla,
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let focus = focus::handle(label, 0, window, cx);
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
        .id(label)
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

impl Personalization {
    /// The editor's Back, Cancel and Save controls; an empty bar until it opens.
    fn toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if !self.opened {
            return div().h(px(46.)).flex_none();
        }
        let t = cx.theme().clone();
        let button_text = match t.appearance {
            theme::Appearance::Dark => gpui::rgb(0x2d2d2d).into(),
            theme::Appearance::Light => t.text_inverse,
        };
        let save = self.editor.is_some() && !self.loading && !self.saving && self.dirty(cx);
        div()
            .h(px(46.))
            .flex_none()
            .px(px(20.))
            .flex()
            .items_center()
            .justify_between()
            .child(
                control(
                    "Back",
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
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        control("Cancel", t.text_alpha_5, t.text, !self.saving, window, cx)
                            .when(!self.saving, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.discard(cx)))
                            }),
                    )
                    .child(
                        control(
                            "Save",
                            t.button_primary_solid,
                            button_text,
                            save,
                            window,
                            cx,
                        )
                        .when(save, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                        }),
                    ),
            )
    }

    /// The open editor's page: its heading over the loading state, load error or editor.
    fn editor_page(&self, description: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let body = div().flex().flex_col().gap(px(33.)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(text("Codex instructions", 28., 33.6, 500., t.text))
                .child(description),
        );
        if self.loading {
            return body.child(
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
        }
        if self.load_error {
            return body.child(
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
                        control("Retry", t.list_hover, t.text, true, window, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    ),
            );
        }
        body.children(self.editor.clone())
    }
}

impl Render for Personalization {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
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
        let toolbar = self.toolbar(window, cx);
        let content = if !self.opened {
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
                        16.,
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
                            .child(text("Codex instructions", 13., 130. / 7., 500., t.text))
                            .child(description),
                    )
                    .child(icon("chevron-right", 16., t.text)),
                &focus,
                px(16.),
                px(0.),
                cx,
            )
            .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
            .on_click(cx.listener(|this, _, _, cx| this.load(cx)));
            div()
                .flex()
                .flex_col()
                .gap(px(48.))
                .child(text("Personalization", 28., 33.6, 500., t.text))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(text("Custom instructions", 14., 21., 500., t.text))
                        .child(row),
                )
        } else {
            self.editor_page(description, window, cx)
        };
        div()
            .size_full()
            .font_family(SANS)
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .id("personalization-page")
                    .overflow_y_scroll()
                    .flex_1()
                    .pt(px(28.))
                    .px(px(20.))
                    .child(div().mx_auto().w_full().max_w(px(728.)).child(content)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
