use std::time::{Duration, Instant};

use crate::{icon::icon, motion, text};
use gpui::{
    Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, point,
    prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;

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
                .font_features(unzoomed(window.ui_scale(), false))
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
        crate::composited_bounds(
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
                // The original clips the contents to the banner's rounded padding box.
                .child(crate::rounded_clip(
                    length(19.5),
                    shapes::Corner::Round,
                    div()
                        .m(length(0.5))
                        .h(length(43.))
                        .px(length(8.))
                        .flex()
                        .items_center()
                        .child(
                            div().w(length(48.)).flex().justify_center().child(
                                text(
                                    format!("{}%", (self.zoom * 100.).round() as u32),
                                    13. * inverse,
                                    18.571_428 * inverse,
                                    600.,
                                    theme.text,
                                )
                                .font_features(unzoomed(window.ui_scale(), true)),
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
                                .relative()
                                .mx(length(4.))
                                .h(length(16.))
                                .w(length(1.))
                                // Composited like the border, not a GPUI quad.
                                .child(shapes::decoration(shapes::Decoration {
                                    fill: Some(theme.border),
                                    ..Default::default()
                                })),
                        )
                        .child(reset),
                )),
        )
        .into_any_element()
    }
}

/// The banner sits outside the original's zoomed viewport: its text keeps the 100% optical size.
fn unzoomed(ui_scale: f32, numerals: bool) -> gpui::FontFeatures {
    let mut features = textsys::outside_zoom(ui_scale).0.as_ref().clone();
    if numerals {
        features.push(("tnum".into(), 1));
    }
    gpui::FontFeatures(std::sync::Arc::new(features))
}
