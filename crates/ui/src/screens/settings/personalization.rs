use crate::{SANS, composer::Composer, focus, icon::icon, text};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use theme::ActiveTheme as _;

#[derive(Clone)]
pub enum Event {
    Load,
    Save(String),
    Leave,
    Section(&'static str),
}

pub struct Personalization {
    opened: bool,
    loading: bool,
    saving: bool,
    load_error: bool,
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
        self.editor = None;
        cx.emit(Event::Load);
        cx.notify();
    }

    pub fn loaded(
        &mut self,
        result: Result<Value, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        match result {
            Ok(value) if value["contents"].is_string() && value["path"].is_string() => {
                self.saved = value["contents"].as_str().unwrap().to_owned();
                let editor = cx.new(|cx| {
                    Composer::settings_field("Add your custom instructions…", &self.saved, true, cx)
                        .with_field_min_height(px(242.))
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
        .rounded(px(10.))
        .bg(fill)
        .when(label == "Back", |d| d.child(icon("arrow-left", 16., color)))
        .child(text(label, 13., 18., 430., color));
    if !enabled {
        return button.opacity(0.5);
    }
    focus::ring(button.relative(), &focus, px(12.), px(2.), cx)
        .on_mouse_down(gpui::MouseButton::Left, super::focus_on_pointer(focus))
}

impl Render for Personalization {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let toolbar = if self.opened {
            let save = self.editor.is_some() && !self.loading && !self.saving && self.dirty(cx);
            div()
                .h(px(46.))
                .flex_none()
                .px(px(20.))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    control("Back", t.ghost_hover, t.text, !self.saving, window, cx)
                        .when(!self.saving, |d| {
                            d.on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Leave)))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            control("Cancel", t.list_hover, t.text, !self.saving, window, cx)
                                .when(!self.saving, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| this.discard(cx)))
                                }),
                        )
                        .child(
                            control(
                                "Save",
                                t.button_primary_solid,
                                t.text_inverse,
                                save,
                                window,
                                cx,
                            )
                            .when(save, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                            }),
                        ),
                )
        } else {
            div().h(px(46.)).flex_none()
        };
        let content = if !self.opened {
            div().flex().flex_col().gap(px(24.))
                .child(text("Personalization", 28., 33.6, 500., t.text))
                .child(div().p(px(16.)).rounded(px(20.)).border_1().border_color(t.border)
                    .flex().items_center().justify_between().gap(px(12.))
                    .child(div().flex().flex_col().gap(px(4.))
                        .child(text("Codex", 14., 20., 500., t.text))
                        .child(text("Edit the AGENTS.md file on the selected machine. Repository instructions may also apply.", 14., 20., 430., t.text_secondary).whitespace_normal()))
                    .child(control("Edit Codex custom instructions", t.list_hover, t.text, true, window, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx)))))
        } else {
            let mut body = div().flex().flex_col().gap(px(32.))
                .child(div().flex().flex_col().gap(px(8.))
                    .child(text("Codex instructions", 28., 33.6, 500., t.text))
                    .child(text("Edit the AGENTS.md file on the selected machine. Repository instructions may also apply.", 14., 20., 430., t.text_secondary).whitespace_normal()));
            if self.loading {
                body = body.child(
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
                body = body.child(
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
            } else if let Some(editor) = self.editor.clone() {
                body = body.child(editor);
            }
            body
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
