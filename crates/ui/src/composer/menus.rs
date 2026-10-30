use super::*;
use crate::motion::{self, HoverFade};
use gpui::{SpringConfig, SpringState};
use std::time::Instant;

/// `_model-picker-dropdown-content-enter`: `.32s cubic-bezier(.23, 1, .32, 1) 30ms both`
/// from opacity 0 (its `scale(.98)` has no GPUI transform).
const PICKER_ENTER_DELAY: Duration = Duration::from_millis(30);
const PICKER_ENTER: Duration = Duration::from_millis(320);
/// `_Menu_tovi5_1`'s height and the views' `translate`: `.32s cubic-bezier(.19, 1, .22, 1)`.
const PICKER_MOVE: Duration = Duration::from_millis(320);
/// The simple view's box: 4 px panel padding, the 48 px top row, the 40 px slider row.
const SIMPLE_VIEW_HEIGHT: f32 = 96.;
/// `_ThumbSpring` grows to `scale(32 / 28)` while hovered or pressed (spring `jt`) and
/// shrinks back with spring `Mt`.
const THUMB_GROWN: f32 = 32. / 28.;
const THUMB_GROW: SpringConfig = SpringConfig::new(420., 38., 1.);
const THUMB_SHRINK: SpringConfig = SpringConfig::new(220., 26., 1.);

/// The picker popover's motion, alive while the popover renders.
pub(super) struct PickerMotion {
    epoch: Instant,
    advanced: bool,
    height: HoverFade,
    /// Opacity and translate tracks (1: active) of the top row, the slider and the model list.
    views: [[HoverFade; 2]; 3],
    /// The thumb's position in effort steps: from, to, start (Motion tween `St`).
    thumb: (f32, f32, Duration),
    /// `_ThumbSpring`'s scale: start state, target, spring, start.
    scale: (SpringState, f32, SpringConfig, Duration),
    thumb_hovered: bool,
    pressed: bool,
}

/// One frame of [`PickerMotion`].
struct PickerFrame {
    enter: f32,
    height: f32,
    /// Opacity and translate progress (1: in place) of the top row, slider and model list.
    views: [(f32, f32); 3],
    thumb: f32,
    scale: f32,
    switching: bool,
    animating: bool,
}

impl PickerMotion {
    fn new(advanced: bool, thumb: f32, epoch: Instant) -> Self {
        let track = |active: bool| [0; 2].map(|_| HoverFade::settled(active as u8 as f32));
        Self {
            epoch,
            advanced,
            height: HoverFade::settled(advanced as u8 as f32),
            views: [track(!advanced), track(!advanced), track(advanced)],
            thumb: (thumb, thumb, Duration::ZERO),
            scale: (
                SpringState {
                    position: 1.,
                    velocity: 0.,
                },
                1.,
                THUMB_SHRINK,
                Duration::ZERO,
            ),
            thumb_hovered: false,
            pressed: false,
        }
    }

    fn now(&self, cx: &App) -> Duration {
        cx.background_executor().now() - self.epoch
    }

    /// `_Menu_tovi5_1[data-transitions-ready]` for a popper on the trigger's top side: the
    /// views fade (`.2s`) and slide 10 px (the slider rows up, the list down) after per-view
    /// delays.
    fn switch(&mut self, advanced: bool, now: Duration, reduced: bool) {
        self.advanced = advanced;
        if reduced {
            *self = Self {
                thumb: self.thumb,
                ..Self::new(advanced, self.thumb.1, self.epoch)
            };
            return;
        }
        let target = |active: bool| active as u8 as f32;
        self.height
            .transition(target(advanced), now, PICKER_MOVE, motion::EASE_ENTER);
        let delays = if advanced { [0, 16, 40] } else { [56, 40, 0] };
        for (i, (track, delay)) in self.views.iter_mut().zip(delays).enumerate() {
            let active = target((i == 2) == advanced);
            let delay = Duration::from_millis(delay);
            track[0].transition_after(
                active,
                now,
                delay,
                motion::MODERATE,
                motion::EASE_ENTER_SNAPPY,
            );
            track[1].transition_after(active, now, delay, PICKER_MOVE, motion::EASE_ENTER);
        }
    }

    fn thumb_at(&self, now: Duration) -> f32 {
        let (from, to, start) = self.thumb;
        let t = now.saturating_sub(start).as_secs_f32() / motion::RELAXED.as_secs_f32();
        from + (to - from) * motion::EASE_ENTER_SNAPPY(t.min(1.))
    }

    /// The slider's `useEffect`: animate to a new step while the simple view shows, else jump.
    fn slide(&mut self, to: f32, now: Duration, jump: bool) {
        let from = if jump { to } else { self.thumb_at(now) };
        self.thumb = (from, to, now);
    }

    fn scale_at(&self, now: Duration) -> SpringState {
        let (start, target, spring, at) = self.scale;
        spring.step(start, target, now.saturating_sub(at).as_secs_f32())
    }

    fn update_thumb_scale(&mut self, now: Duration) {
        let target = if self.thumb_hovered || self.pressed {
            THUMB_GROWN
        } else {
            1.
        };
        if target != self.scale.1 {
            let spring = if target > 1. {
                THUMB_GROW
            } else {
                THUMB_SHRINK
            };
            self.scale = (self.scale_at(now), target, spring, now);
        }
    }

    fn frame(&self, now: Duration, reduced: bool) -> PickerFrame {
        let enter = if reduced {
            1.
        } else {
            let t =
                now.saturating_sub(PICKER_ENTER_DELAY).as_secs_f32() / PICKER_ENTER.as_secs_f32();
            motion::EASE_ENTER_SNAPPY(t.min(1.))
        };
        let scale = self.scale_at(now);
        let settled = (scale.position - self.scale.1).abs() < 1e-4 && scale.velocity.abs() < 1e-3;
        let switching = self.height.is_animating(now)
            || self.views.iter().flatten().any(|f| f.is_animating(now));
        PickerFrame {
            enter,
            height: self.height.value(now),
            views: self
                .views
                .each_ref()
                .map(|[o, t]| (o.value(now), t.value(now))),
            thumb: self.thumb_at(now),
            scale: if settled {
                self.scale.1
            } else {
                scale.position
            },
            switching,
            animating: enter < 1. || switching || now < self.thumb.2 + motion::RELAXED || !settled,
        }
    }
}

impl Composer {
    /// The model picker popover (`Select effort`): one box holding the simple view (effort
    /// title, model row, slider of the model's efforts) and the advanced view (`Select
    /// model` list), switched as the original's `_Menu_tovi5_1` transitions.
    pub(super) fn picker_menu(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(model) = self.current_model() else {
            return div();
        };
        let advanced = self.menu == Some(Menu::Models);
        let selected = model
            .efforts
            .iter()
            .position(|e| e == self.displayed_effort())
            .unwrap_or(0) as f32;
        let reduced = motion::reduced(cx);
        let state = window.use_keyed_state("effort-picker-motion", cx, |_, cx| {
            PickerMotion::new(advanced, selected, cx.background_executor().now())
        });
        let f = state.update(cx, |m, cx| {
            let now = m.now(cx);
            if m.advanced != advanced {
                m.switch(advanced, now, reduced);
            }
            if m.thumb.1 != selected {
                m.slide(selected, now, reduced || advanced);
            }
            m.frame(now, reduced)
        });
        if f.animating {
            window.request_animation_frame();
        }
        // The inactive view renders only while it fades out, without focus or input.
        let view = |(opacity, moved): (f32, f32), offset: f32, content: Div| {
            let dy = (1. - moved) * offset;
            content
                .when(opacity < 1., |d| d.opacity(opacity))
                .when(dy != 0., |d| d.relative().top(px(dy)))
        };
        let track = |content: Div| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h_full()
                .p(px(4.))
                .flex()
                .flex_col()
                .child(content)
        };
        let shown = |(opacity, _): (f32, f32), active: bool| active || opacity > 0.;
        let simple = shown(f.views[1], !advanced).then(|| {
            let (top_row, slider) = self.simple_view(t, model, &f, !advanced, &state, cx);
            track(
                div()
                    .flex()
                    .flex_col()
                    .child(view(f.views[0], -10., top_row))
                    .child(view(f.views[1], -10., slider)),
            )
        });
        let list = shown(f.views[2], advanced)
            .then(|| track(view(f.views[2], 10., self.models_view(t, advanced, cx))));
        let list_height = 8. + 28. + 32. * self.models.len() as f32;
        let height = SIMPLE_VIEW_HEIGHT + (list_height - SIMPLE_VIEW_HEIGHT) * f.height;
        popover(t, 254., |d| {
            d.p(px(0.)).child(
                div()
                    .relative()
                    .h(px(height))
                    .when(f.switching, |d| d.overflow_hidden())
                    .children(simple)
                    .children(list),
            )
        })
        .when(f.enter < 1., |d| d.opacity(f.enter))
    }

    /// The simple view's top row (`Speed`, effort title and model, `Reset`) and slider row.
    fn simple_view(
        &self,
        t: &Theme,
        model: &ModelOption,
        f: &PickerFrame,
        interactive: bool,
        state: &Entity<PickerMotion>,
        cx: &mut Context<Self>,
    ) -> (Div, Div) {
        let efforts = model.efforts.clone();
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
                    .when(interactive, |d| {
                        d.track_focus(&self.menu_focus.items[SPEED_ITEM])
                            .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                                if *hovered {
                                    this.hover_speed(window, cx);
                                } else {
                                    this.speed_timer = None;
                                }
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.activate_menu_item(SPEED_ITEM, window, cx)
                            }))
                            .when(self.speed_open, |d| {
                                // Right of the 32 px trigger plus 5 px, top-aligned with the
                                // menu's box (4 px above its padding box).
                                d.child(
                                    deferred(
                                        div()
                                            .absolute()
                                            .left(px(37.))
                                            .top(px(-4.))
                                            .child(self.speed_menu(t, cx)),
                                    )
                                    .with_priority(2),
                                )
                            })
                    }),
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
                                effort_label(self.displayed_effort()).to_string(),
                                14.,
                                20.,
                                500.,
                                t.accent,
                            ))
                            .child(
                                div()
                                    .relative()
                                    .px(px(16.))
                                    .child(tabular(text(
                                        model.label.clone(),
                                        12.,
                                        16.,
                                        400.,
                                        t.text_secondary,
                                    )))
                                    .child(
                                        icon("composer-chevron-right-small", 12., t.text_tertiary)
                                            .absolute()
                                            .right(px(4.))
                                            .top(px(2.)),
                                    ),
                            ),
                    )
                    .when(interactive, |d| {
                        d.track_focus(&self.menu_focus.items[0]).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.activate_menu_item(0, window, cx)
                            }),
                        )
                    }),
            )
            .child(
                corner("reset-effort", "composer-reset", t.text_tertiary)
                    .role(gpui::Role::Button)
                    .aria_label("Reset to default")
                    .right_0()
                    .when(interactive, |d| {
                        d.track_focus(&self.menu_focus.items[2]).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.activate_menu_item(2, window, cx)
                            }),
                        )
                    }),
            );
        // Track 230 px; dot centers 13 + 51·i; the thumb (28 px) centers on the thumb
        // position, `f.thumb` steps from the left.
        let n = efforts.len().max(2);
        let step = (230. - 26.) / (n - 1) as f32;
        let dot_x = |i: f32| 13. + step * i;
        let thumb_x = dot_x(f.thumb);
        let thumb = 28. * f.scale;
        let grow = |state: &Entity<PickerMotion>, set: fn(&mut PickerMotion, bool), on: bool| {
            let state = state.clone();
            move |cx: &mut App| {
                state.update(cx, |m, cx| {
                    let now = m.now(cx);
                    set(m, on);
                    m.update_thumb_scale(now);
                    cx.notify();
                })
            }
        };
        let press = grow(state, |m, on| m.pressed = on, true);
        let release = grow(state, |m, on| m.pressed = on, false);
        let release_out = grow(state, |m, on| m.pressed = on, false);
        let hover_on = grow(state, |m, on| m.thumb_hovered = on, true);
        let hover_off = grow(state, |m, on| m.thumb_hovered = on, false);

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
                            // `_Track`: `color(srgb 1 1 1 / 0.1)`, an sRGB mix (8-bit).
                            t.text.opacity(0.1),
                            vec![shadow(t.border, 0., 0., 0.5, true)],
                        )))
                        .when(f.thumb > 0., |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(px(thumb_x))
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
                            let color = if i as f32 <= f.thumb + 1e-3 {
                                gpui::rgba(0xffffff4c).into()
                            } else {
                                Hsla { a: 0.249, ..t.text }
                            };
                            div()
                                .absolute()
                                .left(px(dot_x(i as f32) - 2.))
                                .top(px(10.))
                                .size(px(4.))
                                .child(shapes::decoration(deco(9999., color, vec![])))
                        }))
                        // `_ThumbSpring` (a transform) is its own composited layer over the
                        // track, drawn into the popup's render surface (its backdrop-filter):
                        // a layer nested in the popover's. cc bounds its ink (the `0 0 2px`
                        // shadow) at 3 sigma, 3 px out.
                        .child(
                            div()
                                .absolute()
                                .left(px(thumb_x - thumb / 2. - 3.))
                                .top(px(12. - thumb / 2. - 3.))
                                .size(px(thumb + 6.))
                                .layer()
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(3.))
                                        .top(px(3.))
                                        .size(px(thumb))
                                        .child(shapes::decoration(shapes::Decoration {
                                            border: Some((px(0.5), t.border_strong)),
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
                                ),
                        )
                        .when(interactive, |d| {
                            d.on_mouse_down(MouseButton::Left, move |_, _, cx| press(cx))
                                .on_mouse_up(MouseButton::Left, move |_, _, cx| release(cx))
                                .on_mouse_up_out(MouseButton::Left, move |_, _, cx| release_out(cx))
                                // `_ThumbInput`: the 28 px hover target on the thumb.
                                .child(
                                    div()
                                        .id("effort-thumb")
                                        .absolute()
                                        .left(px(thumb_x - 14.))
                                        .top(px(-2.))
                                        .size(px(28.))
                                        .on_hover(move |hovered: &bool, _, cx| {
                                            if *hovered {
                                                hover_on(cx)
                                            } else {
                                                hover_off(cx)
                                            }
                                        }),
                                )
                                .children(efforts.iter().enumerate().map(|(i, effort)| {
                                    let effort = effort.clone();
                                    div()
                                        .id(("effort", i))
                                        .absolute()
                                        .left(px(dot_x(i as f32) - step / 2.))
                                        .top_0()
                                        .w(px(step))
                                        .h_full()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.effort = Some(effort.clone());
                                            this.picked(cx);
                                            cx.notify();
                                        }))
                                }))
                        }),
                ),
        );
        (header, slider)
    }

    /// The `Speed` submenu (Radix `MenuSub`): `Standard`, `Fast`, `Ultrafast`, the current one
    /// checked. Pointer movement focuses an item, as in the permissions menu.
    fn speed_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        let current = usize::from(self.fast);
        popover(t, 233., |d| {
            d.children(SPEEDS.iter().enumerate().map(|(i, &(title, description))| {
                div()
                    .id(("speed", i))
                    .track_focus(&self.speed_focus.items[i])
                    .focus(move |s| s.bg(hover))
                    .on_mouse_move(cx.listener(move |this, _, window, cx| {
                        this.speed_focus.items[i].focus(window, cx);
                        cx.stop_propagation();
                    }))
                    // The root menu's mouse-down-out guard: the submenu lies outside its box.
                    .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        this.menu_trigger_hovered = *hovered
                    }))
                    .px(px(8.))
                    .py(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(15.))
                    .hover(move |s| s.bg(hover))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text(title, 13., 18.5714, 430., t.text))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(14.))
                                    .text_color(t.text_tertiary)
                                    .child(description),
                            ),
                    )
                    .when(i == current, |d| {
                        d.child(icon("menu-check", 16., t.text).opacity(0.75))
                    })
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.pick_speed(i, window, cx)),
                    )
            }))
        })
    }

    /// The advanced view: the `Select model` list. Only the active view takes focus and input.
    fn models_view(&self, t: &Theme, interactive: bool, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        div()
            .flex()
            .flex_col()
            .child(div().px(px(8.)).py(px(6.)).child(text(
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
                    .when(interactive, |d| {
                        d.track_focus(&self.menu_focus.items[i])
                            .focus(move |s| s.bg(hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_menu_item(i, window, cx)
                            }))
                    })
            }))
    }

    /// `How should ChatGPT actions be approved?`
    pub(super) fn permissions_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        popover(t, 320., |d| {
            d.child(div().px(px(8.)).py(px(6.)).child(text(
                "How should ChatGPT actions be approved?",
                12.,
                16.,
                400.,
                t.text_tertiary,
            )))
            .children(
                Permission::ALL
                    .iter()
                    .map(|p| (p, p.description()))
                    .chain(
                        self.permission_profiles
                            .iter()
                            .map(|(p, description)| (p, description.as_str())),
                    )
                    .enumerate()
                    .map(|(i, (p, description))| {
                        div()
                            .id(("permission-option", i))
                            .track_focus(&self.menu_focus.items[i])
                            .focus(move |s| s.bg(hover))
                            .on_mouse_move(cx.listener(move |this, _, window, cx| {
                                this.menu_focus.items[i].focus(window, cx);
                                cx.stop_propagation();
                            }))
                            .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                                if !hovered && this.menu_focus.items[i].is_focused(window) {
                                    this.menu_focus.content.focus(window, cx);
                                }
                            }))
                            .px(px(8.))
                            .py(px(6.))
                            .flex()
                            .flex_col()
                            .rounded(px(12.))
                            .hover(move |s| s.bg(hover))
                            .child(text(p.option_label().to_owned(), 14., 20., 430., t.text))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .text_color(t.text_tertiary)
                                    .child(description.to_owned()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_menu_item(i, window, cx)
                            }))
                    }),
            )
        })
    }

    /// Pending approval request card (`PendingRequestItemPanel`).
    pub(super) fn approval_card(&self, a: &Approval, t: &Theme, cx: &mut Context<Self>) -> Div {
        let button = |id: &'static str, s: SharedString, bg: Hsla, fg: Hsla| {
            pill(id, t).px(px(10.)).bg(bg).child(label(s, fg))
        };
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .relative()
            .child(shapes::decoration(card_decoration(t, false)))
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
                    .children(a.choices().map(|decision| {
                        let (id, title, bg, fg) = match decision {
                            ApprovalDecision::Accept => (
                                "allow-once",
                                "Allow once".into(),
                                t.button_primary_solid,
                                t.text_inverse,
                            ),
                            ApprovalDecision::AcceptForSession => (
                                "allow-session",
                                a.scoped_label.clone(),
                                t.button_secondary,
                                t.text,
                            ),
                            ApprovalDecision::Decline => {
                                ("deny", "Deny".into(), gpui::transparent_black(), t.text)
                            }
                            ApprovalDecision::Cancel => {
                                ("cancel", "Cancel".into(), gpui::transparent_black(), t.text)
                            }
                        };
                        button(id, title, bg, fg).on_click(
                            self.request_listener(cx, move |this, _, window, cx| {
                                this.decide(decision, window, cx)
                            }),
                        )
                    })),
            )
    }

    /// Popover anchored above a footer button. `width`: the box the original centers on the
    /// button (`None`: left-aligned).
    pub(super) fn anchored_menu(
        &self,
        menu: Div,
        center_width: Option<f32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // `occlude`: a click in the menu must not reach the editor under it, whose mouse
        // down closes the menu.
        let menu = div()
            .id("menu")
            .occlude()
            .track_focus(&self.menu_focus.content)
            .on_key_down(cx.listener(Self::menu_key))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if !this.menu_trigger_hovered && this.menu.take().is_some() {
                    window.blur(cx);
                    cx.notify();
                }
            }))
            .child(menu);
        let slot = div().absolute().bottom(px(36.)).flex();
        // The original's popper places the menu with `transform: translate()` rounded to device
        // px (floating-ui `roundByDPR`), so its origin keeps no fraction of the anchor's.
        deferred(match center_width {
            Some(w) => slot.left_0().right_0().justify_center().child(
                div()
                    .flex_none()
                    .w(px(w))
                    .pl(px(1.))
                    .pb(px(1.))
                    .snap_origin()
                    .child(menu),
            ),
            None => slot.left_0().child(menu.snap_origin()),
        })
        .with_priority(1)
        .into_any_element()
    }
}
