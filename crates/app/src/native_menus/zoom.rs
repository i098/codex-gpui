use std::time::{Duration, Instant};

use crate::{Root, State};
use gpui::{App, AppContext as _, Div, actions};
use gpui::{
    Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, point,
    prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;
use ui::{icon::icon, motion, text};

static NUMERALS: std::sync::LazyLock<gpui::FontFeatures> =
    std::sync::LazyLock::new(|| gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));

#[derive(Clone, Copy)]
pub enum Change {
    Step(i32),
    Reset,
}

pub struct ZoomBanner {
    zoom: f32,
    visible: bool,
    hovered: bool,
    timed_out: bool,
    generation: u64,
    exiting: Option<Instant>,
    controls: [FocusHandle; 3],
}

impl EventEmitter<Change> for ZoomBanner {}

impl ZoomBanner {
    pub fn new(zoom: f32, cx: &mut Context<Self>) -> Self {
        let mut banner = Self {
            zoom,
            visible: false,
            hovered: false,
            timed_out: false,
            generation: 0,
            exiting: None,
            controls: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
        };
        banner.show(zoom, cx);
        banner
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.zoom = zoom;
        self.visible = true;
        self.timed_out = false;
        self.exiting = None;
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.timed_out = true;
                    if !this.hovered {
                        this.exit(cx);
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn exit(&mut self, cx: &mut Context<Self>) {
        if self.exiting.is_some() {
            return;
        }
        self.exiting = Some(cx.background_executor().now());
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.visible = false;
                    this.exiting = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn activate(&self, index: usize, cx: &mut Context<Self>) {
        if index == 2 && (self.zoom - 1.).abs() < 0.001 {
            return;
        }
        cx.emit(match index {
            0 => Change::Step(-1),
            1 => Change::Step(1),
            _ => Change::Reset,
        });
    }
}

impl Render for ZoomBanner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().into_any_element();
        }
        let theme = cx.theme().clone();
        let inverse = 1. / window.ui_scale();
        let length = |value| px(value * inverse);
        let progress = self
            .exiting
            .map_or(0., |start| {
                (cx.background_executor().now() - start).as_secs_f32() / 0.15
            })
            .clamp(0., 1.);
        if self.exiting.is_some() {
            window.request_animation_frame();
        }
        let opacity = if motion::reduced(cx) {
            1.
        } else {
            1. - motion::EASE_OUT(progress)
        };
        let hover = theme.ghost_hover;
        let [zoom_out, zoom_in, reset] = std::array::from_fn(|index| {
            let label = ["Zoom out", "Zoom in", "Reset"][index];
            let disabled = index == 2 && (self.zoom - 1.).abs() < 0.001;
            let focused =
                self.controls[index].is_focused(window) && window.last_input_was_keyboard();
            let content = match index {
                0 => icon("zoom-minus", 16. * inverse, theme.text_tertiary)
                    .runtime()
                    .into_any_element(),
                1 => icon("zoom-plus", 16. * inverse, theme.text_tertiary)
                    .runtime()
                    .into_any_element(),
                _ => text(
                    "Reset",
                    13. * inverse,
                    18. * inverse,
                    430.,
                    theme.text_tertiary,
                )
                .into_any_element(),
            };
            div()
                .id(("zoom-control", index))
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .track_focus(&self.controls[index])
                .h(length(28.))
                .min_w(length(28.))
                .px(length(if index == 2 { 9. } else { 0. }))
                .rounded(length(12.5))
                .flex()
                .items_center()
                .justify_center()
                .when(disabled, |d| d.opacity(0.4))
                .when(!disabled, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .when(focused, |d| d.border_1().border_color(theme.focus_ring))
                .on_click(cx.listener(move |this, _, _, cx| this.activate(index, cx)))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.activate(index, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(content)
        });
        ui::composited_bounds(
            div()
                .id("window-zoom-banner")
                .absolute()
                .top(length(
                    8. - if motion::reduced(cx) {
                        0.
                    } else {
                        4. * motion::EASE_OUT(progress)
                    },
                ))
                .right(length(16.))
                .h(length(44.))
                .px(length(8.5))
                .flex()
                .items_center()
                .opacity(opacity)
                .layer()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.hovered = *hovered;
                    if !*hovered && this.timed_out {
                        this.exit(cx);
                    }
                }))
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(length(20.)),
                    fill: Some(theme.surface_secondary),
                    border: Some((length(0.5), theme.border)),
                    shadows: vec![shapes::Shadow {
                        color: gpui::hsla(0., 0., 0., 0.12),
                        offset: point(length(0.), length(8.)),
                        blur: length(16.),
                        spread: length(-4.),
                        inset: false,
                    }],
                    ..Default::default()
                }))
                .child(
                    div().w(length(48.)).flex().justify_center().child(
                        text(
                            format!("{}%", (self.zoom * 100.).round() as u32),
                            13. * inverse,
                            18.571_428 * inverse,
                            600.,
                            theme.text,
                        )
                        .font_features(NUMERALS.clone()),
                    ),
                )
                .child(
                    div()
                        .ml(length(4.))
                        .flex()
                        .items_center()
                        .children([zoom_out, zoom_in]),
                )
                .child(
                    div()
                        .mx(length(4.))
                        .h(length(16.))
                        .w(length(1.))
                        .bg(theme.border),
                )
                .child(reset),
        )
        .into_any_element()
    }
}

actions!(app, [ZoomIn, ZoomOut, ActualSize]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-+", ZoomIn, None),
        gpui::KeyBinding::new("cmd-=", ZoomIn, None),
        gpui::KeyBinding::new("cmd-shift-=", ZoomIn, None),
        gpui::KeyBinding::new("cmd--", ZoomOut, None),
        gpui::KeyBinding::new("cmd-0", ActualSize, None),
    ]);
}

pub(super) fn actions(root: Div, owner: &Root, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &ZoomIn, window, cx| this.change_zoom(Some(1), window, cx)),
    )
    .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.change_zoom(Some(-1), window, cx)))
    .on_action(cx.listener(|this, _: &ActualSize, window, cx| this.change_zoom(None, window, cx)))
    .children(
        owner
            .zoom_banner
            .as_ref()
            .filter(|banner| banner.read(cx).visible())
            .cloned(),
    )
}

pub(crate) fn initialize_window(state: &mut State, window: &mut Window) {
    if !state.zoom.is_finite() {
        state.zoom = 1.;
    }
    state.zoom = state.zoom.clamp(0.5, 3.);
    window.set_ui_scale(state.zoom);
}

impl Root {
    fn change_zoom(&mut self, step: Option<i32>, window: &mut Window, cx: &mut Context<Self>) {
        self.state.zoom = step.map_or(1., |step| {
            ((self.state.zoom * 100. + step as f32 * 10.).round() / 100.).clamp(0.5, 3.)
        });
        window.set_ui_scale(self.state.zoom);
        self.fit(window, cx);
        self.save(window, cx);
        if let Some(banner) = &self.zoom_banner {
            banner.update(cx, |banner, cx| banner.show(self.state.zoom, cx));
        } else {
            let banner = cx.new(|cx| ZoomBanner::new(self.state.zoom, cx));
            cx.subscribe_in(&banner, window, |this, _, change: &Change, window, cx| {
                this.change_zoom(
                    match change {
                        Change::Step(step) => Some(*step),
                        Change::Reset => None,
                    },
                    window,
                    cx,
                );
            })
            .detach();
            self.zoom_banner = Some(banner);
        }
        cx.notify();
    }
}
