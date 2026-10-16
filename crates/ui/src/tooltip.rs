//! The shared tooltip (Radix `Tooltip` inside `Pn`): a 13 px pill with the label and the
//! command's shortcut chip, opened by hovering its trigger. Values are the DOM of
//! `refs/interactions/f1a.json`, recorded from the original.
//!
//! - Opens `OPEN_DELAY` after the pointer enters the trigger; at once when another tooltip
//!   was open or closed less than `SKIP_DELAY` ago (Radix `skipDelayDuration`), so moving
//!   along a row of buttons shows each at once.
//! - Closes when the pointer leaves, on a press on the trigger, and on Escape.
//! - Sits `OFFSET` px from the trigger, centred above, below, or to its right.
//!
//! The tooltip stays at least 8 px inside the window, including narrow titlebar layouts.

use gpui::{
    Anchor, App, Bounds, Context, DispatchPhase, Div, ElementId, Entity, Global, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, RenderOnce,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Window, anchored, canvas, deferred,
    div, point, prelude::FluentBuilder as _, px, relative, rgba,
};
use std::time::Duration;
use theme::{ActiveTheme as _, Appearance};

/// Radix `delayDuration` (`app-shared` `w5`).
const OPEN_DELAY: Duration = Duration::from_millis(200);
/// Radix `skipDelayDuration`.
const SKIP_DELAY: Duration = Duration::from_millis(300);
/// Gap between the trigger and the pill (recorded: 2 px on both sides).
const OFFSET: Pixels = px(2.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Bottom,
    Right,
}

/// The tooltip on screen, for tests: its text and the pill's bounds as painted.
#[derive(Clone, Debug)]
pub struct Shown {
    pub label: SharedString,
    pub shortcut: Option<SharedString>,
    pub bounds: Bounds<Pixels>,
    owner: ElementId,
}

/// What every tooltip shares: the tooltip on screen and whether the delay is skipped.
#[derive(Default)]
struct Hub {
    shown: Option<Shown>,
    skip_delay: bool,
    /// Tells the pending end of the skip window from a stale one.
    skip_gen: u64,
}

impl Global for Hub {}

/// The open tooltip, if any.
pub fn shown(cx: &App) -> Option<&Shown> {
    cx.try_global::<Hub>()?.shown.as_ref()
}

/// Who a tooltip is for and what it says.
#[derive(Clone)]
struct Spec {
    id: ElementId,
    label: SharedString,
    shortcut: Option<SharedString>,
}

#[derive(Default)]
struct TipState {
    open: bool,
    /// Bumped by every enter and leave: a delay timer only opens the tooltip it started for.
    epoch: u64,
}

impl TipState {
    fn hover(&mut self, hovered: bool, spec: Spec, cx: &mut Context<Self>) {
        self.epoch += 1;
        if !hovered {
            return self.close(&spec.id, cx);
        }
        if self.open {
            return;
        }
        if cx.default_global::<Hub>().skip_delay {
            return self.open(spec, cx);
        }
        let epoch = self.epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OPEN_DELAY).await;
            this.update(cx, |this, cx| {
                if this.epoch == epoch {
                    this.open(spec, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn open(&mut self, spec: Spec, cx: &mut Context<Self>) {
        self.open = true;
        let hub = cx.default_global::<Hub>();
        hub.skip_delay = true;
        hub.skip_gen += 1;
        hub.shown = Some(Shown {
            label: spec.label,
            shortcut: spec.shortcut,
            bounds: Bounds::default(),
            owner: spec.id,
        });
        cx.notify();
    }

    fn close(&mut self, id: &ElementId, cx: &mut Context<Self>) {
        self.epoch += 1;
        if !self.open {
            return;
        }
        self.open = false;
        let hub = cx.default_global::<Hub>();
        // Another tooltip may already have taken over (moving between triggers).
        if hub.shown.as_ref().is_some_and(|s| &s.owner == id) {
            hub.shown = None;
            hub.skip_gen += 1;
            let epoch = hub.skip_gen;
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(SKIP_DELAY).await;
                cx.update_global::<Hub, _>(|hub, _| {
                    if hub.skip_gen == epoch {
                        hub.skip_delay = false;
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }
}

/// `trigger` with a tooltip: `label` and the shortcut chip `shortcut` (`⌘K`) on `side`.
/// `trigger` needs an id, and no `on_hover` of its own.
pub fn tooltip(
    trigger: Stateful<Div>,
    label: impl Into<SharedString>,
    shortcut: Option<&'static str>,
    side: Side,
) -> Tooltip {
    Tooltip {
        trigger,
        label: label.into(),
        shortcut: shortcut.map(SharedString::new_static),
        side,
    }
}

#[derive(IntoElement)]
pub struct Tooltip {
    trigger: Stateful<Div>,
    label: SharedString,
    shortcut: Option<SharedString>,
    side: Side,
}

impl RenderOnce for Tooltip {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self
            .trigger
            .interactivity()
            .element_id
            .clone()
            .expect("a tooltip trigger has an id");
        let state = window.use_keyed_state(id.clone(), cx, |_, _| TipState::default());
        let spec = Spec {
            id,
            label: self.label,
            shortcut: self.shortcut,
        };
        let open = state.read(cx).open;
        let bubble = open.then(|| bubble(&spec, self.side, &state, cx));
        self.trigger
            .on_hover({
                let (state, spec) = (state.clone(), spec.clone());
                move |hovered, _, cx| {
                    state.update(cx, |s, cx| s.hover(*hovered, spec.clone(), cx));
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let (state, id) = (state.clone(), spec.id.clone());
                move |_, _, cx| state.update(cx, |s, cx| s.close(&id, cx))
            })
            .children(bubble)
    }
}

/// The pill, placed by an absolute box as large as the trigger (its containing block).
fn bubble(spec: &Spec, side: Side, state: &Entity<TipState>, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    // `--color-background-tooltip` over `--color-text-tooltip`. [INFERENCE: light values from
    // the CSS (`primary-solid` over its inverse); only dark is recorded.]
    let (bg, fg): (Hsla, Hsla) = match t.appearance {
        Appearance::Dark => (t.surface_elevated_secondary, t.text),
        Appearance::Light => (t.button_primary_solid, rgba(0xffffffff).into()),
    };
    let id = spec.id.clone();
    let owner = id.clone();
    let state = state.clone();
    let pill = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(5.))
        .border_1()
        .border_color(fg.opacity(0.05))
        .rounded(px(20.))
        .bg(bg)
        .shadow(vec![gpui::BoxShadow {
            color: rgba(0x0f172a33).into(),
            offset: point(px(0.), px(8.)),
            blur_radius: px(18.),
            spread_radius: px(0.),
            inset: false,
        }])
        .child(crate::text(spec.label.clone(), 13., 18., 430., fg))
        .when_some(spec.shortcut.clone(), |d, keys| {
            // `kbd`: 18 px chip, `-me-1.75` pulls its right edge 7 px into the padding.
            d.child(
                div()
                    .h(px(18.))
                    .min_w(px(16.))
                    .px(px(6.))
                    .mr(px(-7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .bg(fg.opacity(0.25))
                    .child(crate::text(keys, 13., 18., 430., fg)),
            )
        })
        // Paints nothing: reports the pill's bounds and closes the tooltip on Escape.
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, cx| {
                    if let Some(shown) = cx.default_global::<Hub>().shown.as_mut()
                        && shown.owner == owner
                    {
                        shown.bounds = bounds;
                    }
                    let (state, id) = (state.clone(), id.clone());
                    window.on_key_event(move |e: &KeyDownEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture && e.keystroke.key == "escape" {
                            state.update(cx, |s, cx| s.close(&id, cx));
                        }
                    });
                },
            )
            .absolute()
            .inset(px(-1.)),
        );
    // A zero-size point on the trigger's edge, centred along it; the pill hangs off it.
    let (point_on_edge, anchor, offset) = match side {
        Side::Top => (
            div().absolute().left(relative(0.5)).top_0(),
            Anchor::BottomCenter,
            point(px(0.), -OFFSET),
        ),
        Side::Bottom => (
            div().absolute().left(relative(0.5)).top(relative(1.)),
            Anchor::TopCenter,
            point(px(0.), OFFSET),
        ),
        Side::Right => (
            div().absolute().left(relative(1.)).top(relative(0.5)),
            Anchor::LeftCenter,
            point(OFFSET, px(0.)),
        ),
    };
    deferred(
        point_on_edge.child(
            anchored()
                .anchor(anchor)
                .offset(offset)
                .snap_to_window_with_margin(gpui::Edges::all(px(8.)))
                .child(pill),
        ),
    )
    .with_priority(2)
}
