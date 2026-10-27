use crate::{icon::icon, text};
use gpui::{
    App, Context, Div, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use theme::Theme;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Initializing,
    Idle,
    Listening,
    Transcribing,
    Error {
        message: String,
        retry: bool,
    },
    PasteFailed {
        text: String,
        copied: bool,
        accessibility: bool,
    },
    Recovery {
        state: Recovery,
        saved: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Recovery {
    Recovering,
    Recovered(String),
    Failed,
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Retry,
    Dismiss,
    Copy,
    AccessibilitySettings,
    Recordings,
    Region(gpui::Bounds<gpui::Pixels>),
}

pub struct View {
    pub phase: Phase,
    levels: [f32; 24],
    shortcut: String,
    hovered: bool,
    focus: FocusHandle,
    region: Option<gpui::Bounds<gpui::Pixels>>,
}

impl EventEmitter<Action> for View {}
impl Focusable for View {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl View {
    pub fn new(shortcut: String, cx: &mut Context<Self>) -> Self {
        Self {
            phase: Phase::Idle,
            levels: [0.; 24],
            shortcut,
            hovered: false,
            focus: cx.focus_handle(),
            region: None,
        }
    }

    pub fn set_shortcut(&mut self, shortcut: String, cx: &mut Context<Self>) {
        self.shortcut = shortcut;
        cx.notify();
    }

    pub fn set_phase(&mut self, phase: Phase, cx: &mut Context<Self>) {
        self.phase = phase;
        self.levels.fill(0.);
        cx.notify();
    }

    pub fn push_level(&mut self, level: f32, cx: &mut Context<Self>) {
        self.levels.rotate_left(1);
        self.levels[23] = level.clamp(0., 1.);
        cx.notify();
    }

    fn button(
        &self,
        label: &'static str,
        glyph: &'static str,
        action: Action,
        t: &Theme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        div()
            .id(label)
            .role(gpui::Role::Button)
            .aria_label(label)
            .focusable()
            .tab_index(0)
            .size(px(20.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .justify_center()
            .hover(|style| style.bg(gpui::rgba(0xffffff14)))
            .child(icon(glyph, 14., t.text_secondary))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(action)))
            .on_key_down(cx.listener(move |_, event: &gpui::KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.emit(action);
                    cx.stop_propagation();
                }
            }))
    }

    fn text_button(
        &self,
        label: &'static str,
        action: Action,
        t: &Theme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        div()
            .id(label)
            .role(gpui::Role::Button)
            .aria_label(label)
            .focusable()
            .tab_index(0)
            .h(px(28.))
            .px(px(8.))
            .rounded(px(8.))
            .bg(t.ghost_hover)
            .flex()
            .items_center()
            .child(text(label, 12., 16., 500., t.text))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(action)))
            .on_key_down(cx.listener(move |_, event: &gpui::KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.emit(action);
                    cx.stop_propagation();
                }
            }))
    }

    fn recovery(
        &self,
        text_value: &str,
        copied: bool,
        accessibility: bool,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let title = if copied {
            "Text copied — ⌘V to paste"
        } else {
            "Your clipboard changed"
        };
        let detail = if !copied {
            "Copy your transcript to paste it"
        } else if accessibility {
            "Enable Accessibility to paste automatically"
        } else {
            "Couldn't paste automatically"
        };
        div().w_full().max_w(px(576.)).p(px(8.)).child(
            div()
                .id("dictation-paste-recovery")
                .role(gpui::Role::Status)
                .rounded(px(16.))
                .border_1()
                .border_color(gpui::rgba(0xffffff14))
                .bg(gpui::rgb(0x181818))
                .p(px(12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(text(title, 13., 18., 500., t.text))
                        .child(text(detail, 12., 16., 400., t.text_secondary)),
                )
                .when(accessibility, |row| {
                    row.child(self.text_button(
                        "Open Settings",
                        Action::AccessibilitySettings,
                        t,
                        cx,
                    ))
                })
                .when(!copied && !text_value.is_empty(), |row| {
                    row.child(self.text_button("Copy transcript", Action::Copy, t, cx))
                })
                .child(self.button("Dismiss", "close", Action::Dismiss, t, cx)),
        )
    }
}

impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::dark();
        let body = match &self.phase {
            Phase::PasteFailed {
                text,
                copied,
                accessibility,
            } => self.recovery(text, *copied, *accessibility, &t, cx),
            Phase::Recovery { state, saved } => {
                let title = match state {
                    Recovery::Recovering => "Recovering text…",
                    Recovery::Recovered(_) => "Text recovered",
                    Recovery::Failed => "Dictation stopped",
                };
                div().w_full().max_w(px(576.)).p(px(8.)).child(
                    div()
                        .id("dictation-recovery")
                        .role(gpui::Role::Status)
                        .rounded(px(16.))
                        .border_1()
                        .border_color(gpui::rgba(0xffffff14))
                        .bg(gpui::rgb(0x181818))
                        .p(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .flex_wrap()
                        .when(matches!(state, Recovery::Recovering), |row| {
                            row.child(icon("sidebar-cmd-spinner", 16., t.text))
                        })
                        .child(div().flex_1().child(text(title, 13., 18., 500., t.text)))
                        .when(matches!(state, Recovery::Recovered(_)), |row| {
                            row.child(self.text_button("Copy text", Action::Copy, &t, cx))
                        })
                        .when(matches!(state, Recovery::Failed), |row| {
                            row.child(self.text_button("Retry", Action::Retry, &t, cx))
                        })
                        .when(*saved, |row| {
                            row.child(self.text_button(
                                "View recording",
                                Action::Recordings,
                                &t,
                                cx,
                            ))
                        })
                        .child(self.button("Dismiss", "close", Action::Dismiss, &t, cx)),
                )
            }
            Phase::Error { message, retry } => div()
                .h(px(32.))
                .max_w(px(304.))
                .px(px(8.))
                .rounded(px(16.))
                .bg(gpui::black())
                .border_1()
                .border_color(gpui::rgba(0xffffff10))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().overflow_hidden().child(text(
                    message.clone(),
                    12.,
                    16.,
                    500.,
                    t.text_error,
                )))
                .when(*retry, |row| {
                    row.child(self.button("Retry", "diff-refresh", Action::Retry, &t, cx))
                })
                .child(self.button("Dismiss", "close", Action::Dismiss, &t, cx)),
            phase => {
                let idle = matches!(phase, Phase::Idle | Phase::Initializing);
                let expanded = !idle || self.hovered;
                let pill = div()
                    .relative()
                    .w(px(if expanded { 72. } else { 40. }))
                    .h(px(if expanded { 30. } else { 8. }))
                    .rounded(px(if expanded { 15. } else { 4. }))
                    .border_1()
                    .border_color(gpui::rgba(if expanded { 0xffffff10 } else { 0xffffff74 }))
                    .bg(if expanded {
                        gpui::rgba(0x000000ff)
                    } else {
                        gpui::rgba(0x000000b3)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(if expanded { 8. } else { 0. }))
                    .when(matches!(phase, Phase::Idle) && expanded, |pill| {
                        pill.child(icon("mic", 12., t.text_secondary))
                    })
                    .when(matches!(phase, Phase::Transcribing), |pill| {
                        pill.child(icon("sidebar-cmd-spinner", 12., t.text_secondary))
                    })
                    .when(matches!(phase, Phase::Listening), |pill| {
                        pill.child(
                            div()
                                .h(px(16.))
                                .flex_1()
                                .flex()
                                .items_center()
                                .justify_between()
                                .children(self.levels.iter().enumerate().map(|(index, level)| {
                                    div()
                                        .id(("dictation-level", index))
                                        .w(px(1.))
                                        .h(px(2. + 14. * level))
                                        .rounded(px(1.))
                                        .bg(gpui::white())
                                })),
                        )
                    });
                let label = match phase {
                    Phase::Idle => "Global dictation ready",
                    Phase::Initializing => "Preparing dictation",
                    Phase::Listening => "Global dictation waveform",
                    _ => "Transcribing…",
                };
                let trigger = div()
                    .id("global-dictation-hitbox")
                    .w(px(120.))
                    .h(px(30.))
                    .role(gpui::Role::Status)
                    .aria_label(label)
                    .flex()
                    .items_end()
                    .justify_center()
                    .child(pill)
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.hovered = *hovered;
                        cx.notify();
                    }));
                if matches!(phase, Phase::Idle) && !self.shortcut.is_empty() {
                    div().child(crate::tooltip::tooltip(
                        trigger,
                        self.shortcut.clone(),
                        None,
                        crate::tooltip::Side::Top,
                    ))
                } else {
                    div().child(trigger)
                }
            }
        };
        let view = cx.weak_entity();
        div()
            .size_full()
            .font_family(crate::SANS)
            .flex()
            .items_end()
            .justify_center()
            .child(body)
            .on_children_prepainted(move |bounds, _, cx| {
                if let Some(bounds) = bounds.first().copied() {
                    view.update(cx, |view, cx| {
                        if view.region != Some(bounds) {
                            view.region = Some(bounds);
                            cx.emit(Action::Region(bounds));
                        }
                    })
                    .ok();
                }
            })
    }
}
