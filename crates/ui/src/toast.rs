//! Toasts: the original's Sonner `Notifications` region (`codex-sonner-toaster`), recorded in
//! `refs/interactions/f3a.json`.
//!
//! The region is window-centred, 48 px under the window top. The newest toast is in front;
//! at rest older ones peek out under it, each 8 px lower and 5 % smaller (empty, only the
//! third is the last one drawn); under the pointer the region expands to a column with an 8 px
//! gap. A toast lives 5 s; the pointer over any toast stops the clocks. A drag to the right
//! moves a toast with the pointer and dismisses it from 45 px, or faster than 0.11 px/ms.

use gpui::{
    App, BoxShadow, ClickEvent, Context, DispatchPhase, Div, Hsla, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Pixels, Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Task,
    Window, canvas, div, point, prelude::FluentBuilder as _, px, rgba,
};
use std::{collections::HashSet, rc::Rc, time::Duration};
use theme::{ActiveTheme as _, Appearance, Theme};

/// How long a toast stays when the pointer is not over it.
pub const LIFETIME: Duration = Duration::from_secs(5);
/// Toasts drawn at once; older ones wait behind.
const VISIBLE: usize = 3;
/// `top` of the region: the toast's top edge.
const TOP: f32 = 48.;
const GAP: f32 = 8.;
/// A one-line toast: 24 px row, 8 px padding, 1 px border.
const HEIGHT: f32 = 42.;
const SWIPE_DISTANCE: f32 = 45.;
/// px per ms.
const SWIPE_SPEED: f32 = 0.11;

type Click = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Info,
    Danger,
}

/// A toast: an optional icon (`assets/icons/<name>.svg`, 16 px), one line of title and
/// buttons; the last button is the primary one, earlier ones are secondary.
pub struct Toast {
    kind: Kind,
    icon: Option<&'static str>,
    title: SharedString,
    actions: Vec<(SharedString, Click)>,
}

impl Toast {
    pub fn info(title: impl Into<SharedString>) -> Self {
        Self {
            kind: Kind::Info,
            icon: None,
            title: title.into(),
            actions: Vec::new(),
        }
    }

    /// Red toast with the alert icon (`toast.danger`).
    pub fn danger(title: impl Into<SharedString>) -> Self {
        Self {
            kind: Kind::Danger,
            icon: Some("toast-error"),
            ..Self::info(title)
        }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    /// A button; a click runs `on_click` (after the toast is gone) and dismisses the toast.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.actions.push((label.into(), Rc::new(on_click)));
        self
    }
}

struct Entry {
    id: u64,
    toast: Toast,
    /// Lifetime left when the clock last stopped.
    remaining: Duration,
    /// Time since the clock last started; `None` while it is stopped.
    running: Option<Rc<dyn Fn() -> Duration>>,
    _timer: Option<Task<()>>,
    /// Drag offset to the right.
    dragged: f32,
}

struct Swipe {
    id: u64,
    x: Pixels,
    age: Rc<dyn Fn() -> Duration>,
}

#[derive(Default)]
pub struct Toaster {
    /// Oldest first.
    toasts: Vec<Entry>,
    next: u64,
    hovered: HashSet<u64>,
    swipe: Option<Swipe>,
}

impl Toaster {
    pub fn new() -> Self {
        Self::default()
    }

    /// Titles, front (newest) first.
    pub fn titles(&self) -> Vec<SharedString> {
        self.toasts
            .iter()
            .rev()
            .map(|e| e.toast.title.clone())
            .collect()
    }

    pub fn push(&mut self, toast: Toast, cx: &mut Context<Self>) {
        let id = self.next;
        self.next += 1;
        self.toasts.push(Entry {
            id,
            toast,
            remaining: LIFETIME,
            running: None,
            _timer: None,
            dragged: 0.,
        });
        if self.hovered.is_empty() {
            self.run(id, cx);
        }
        cx.notify();
    }

    fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.toasts.retain(|e| e.id != id);
        self.hovered.remove(&id);
        if self.swipe.as_ref().is_some_and(|s| s.id == id) {
            self.swipe = None;
        }
        cx.notify();
    }

    fn run(&mut self, id: u64, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let Some(e) = self.toasts.iter_mut().find(|e| e.id == id) else {
            return;
        };
        let (started, clock) = (executor.now(), executor.clone());
        e.running = Some(Rc::new(move || clock.now().duration_since(started)));
        let left = e.remaining;
        e._timer = Some(cx.spawn(async move |this, cx| {
            executor.timer(left).await;
            this.update(cx, |this, cx| this.dismiss(id, cx)).ok();
        }));
    }

    fn stop(&mut self) {
        for e in &mut self.toasts {
            if let Some(running) = e.running.take() {
                e.remaining = e.remaining.saturating_sub(running());
            }
            e._timer = None;
        }
    }

    fn hover(&mut self, id: u64, over: bool, cx: &mut Context<Self>) {
        let was = !self.hovered.is_empty();
        if over {
            self.hovered.insert(id);
        } else {
            self.hovered.remove(&id);
        }
        match (was, !self.hovered.is_empty()) {
            (false, true) => self.stop(),
            (true, false) => {
                let ids: Vec<u64> = self.toasts.iter().map(|e| e.id).collect();
                ids.into_iter().for_each(|id| self.run(id, cx));
            }
            _ => {}
        }
        cx.notify();
    }

    fn press(&mut self, id: u64, x: Pixels, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let started = executor.now();
        self.swipe = Some(Swipe {
            id,
            x,
            age: Rc::new(move || executor.now().duration_since(started)),
        });
        // Repaint to mount the window-wide move and release listeners.
        cx.notify();
    }

    fn drag(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(swipe) = &self.swipe else { return };
        let dx = f32::from(x - swipe.x).max(0.);
        if let Some(e) = self.toasts.iter_mut().find(|e| e.id == swipe.id) {
            e.dragged = dx;
            cx.notify();
        }
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        let Some(swipe) = self.swipe.take() else {
            return;
        };
        let Some(e) = self.toasts.iter_mut().find(|e| e.id == swipe.id) else {
            return;
        };
        let ms = (swipe.age)().as_secs_f32() * 1000.;
        if e.dragged >= SWIPE_DISTANCE || (ms > 0. && e.dragged / ms > SWIPE_SPEED) {
            self.dismiss(swipe.id, cx);
        } else {
            e.dragged = 0.;
            cx.notify();
        }
    }
}

/// `bg-danger-surface`; the light value is not recorded [INFERENCE: the error colour at 10 %].
fn danger_surface(t: &Theme) -> Hsla {
    match t.appearance {
        Appearance::Dark => rgba(0x280b0aff).into(),
        Appearance::Light => crate::over(t.text_error.opacity(0.1), t.surface_elevated_secondary),
    }
}

fn button(
    id: (&'static str, u64),
    label: SharedString,
    primary: bool,
    fg: Hsla,
    t: &Theme,
) -> Stateful<Div> {
    let (bg, hover, color, border) = if primary {
        (
            t.button_primary_solid,
            t.text.opacity(0.8),
            t.surface_elevated_secondary,
            t.border,
        )
    } else {
        (
            t.text_alpha_5,
            t.text_alpha_10,
            fg,
            gpui::transparent_black(),
        )
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(24.))
        .px(px(8.))
        .rounded(px(12.5))
        .border_1()
        .border_color(border)
        .bg(bg)
        .hover(move |s| s.bg(hover))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(crate::text(label, 12., 16., 430., color))
}

impl Toaster {
    fn toast(&self, e: &Entry, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &cx.theme().clone();
        let (bg, border, fg) = match e.toast.kind {
            Kind::Info => (t.surface_elevated_secondary, t.border, t.text),
            Kind::Danger => (danger_surface(t), t.text_error.opacity(0.4), t.text_error),
        };
        let id = e.id;
        let last = e.toast.actions.len().saturating_sub(1);
        let actions: Vec<_> = e
            .toast
            .actions
            .iter()
            .enumerate()
            .map(|(i, (label, on_click))| {
                let on_click = on_click.clone();
                button(
                    ("toast-action", id * 8 + i as u64),
                    label.clone(),
                    i == last,
                    fg,
                    t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.dismiss(id, cx);
                    let on_click = on_click.clone();
                    window.defer(cx, move |window, cx| on_click(window, cx));
                }))
            })
            .collect();
        let ghost = t.ghost_hover;
        div()
            .id(("toast", id))
            .relative()
            .left(px(e.dragged))
            .occlude()
            .flex()
            .items_start()
            .gap(px(4.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(15.))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .shadow(vec![BoxShadow {
                color: gpui::black().opacity(0.1),
                offset: point(px(0.), px(4.)),
                blur_radius: px(12.),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_hover(cx.listener(move |this, over, _, cx| this.hover(id, *over, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.press(id, e.position.x, cx)
                }),
            )
            .children(e.toast.icon.map(|icon| {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(24.))
                    .child(crate::icon::icon(icon, 16., fg))
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .min_h(px(24.))
                            .child(crate::text(e.toast.title.clone(), 14., 21., 500., fg)),
                    )
                    .when_some((!actions.is_empty()).then_some(actions), |d, actions| {
                        d.child(div().flex().flex_none().gap(px(8.)).children(actions))
                    }),
            )
            .child(
                div()
                    .id(("toast-close", id))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(24.))
                    .rounded_full()
                    .hover(move |s| s.bg(ghost))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.dismiss(id, cx)))
                    // The original's 8 px cross in a 16 px box; `close.svg` is 20 px wide with a
                    // 9.28 px cross, so 17.24 px draws the same size.
                    .child(crate::icon::icon("close", 17.24, fg)),
            )
    }
}

impl Render for Toaster {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let region = div()
            .absolute()
            .top(px(TOP))
            .left_0()
            .w_full()
            .flex()
            .justify_center();
        if self.toasts.is_empty() {
            return region.size_0();
        }
        let (border, surface) = (cx.theme().border, cx.theme().surface_elevated_secondary);
        let shown: Vec<&Entry> = self.toasts.iter().rev().take(VISIBLE).collect();
        let stack: Div = if !self.hovered.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(GAP))
                .children(shown.iter().map(|e| self.toast(e, cx).into_any_element()))
        } else {
            // The front toast sets the stack's size; the others are empty boxes behind it, 8 px
            // lower and 5 % smaller each (scaled about their centre).
            let behind = shown.iter().enumerate().skip(1).rev().map(|(n, _)| {
                let scale = 1. - 0.05 * n as f32;
                let h = HEIGHT * scale;
                div()
                    .absolute()
                    .top(px(GAP * n as f32 + (HEIGHT - h) / 2.))
                    .left(gpui::relative((1. - scale) / 2.))
                    .w(gpui::relative(scale))
                    .h(px(h))
                    .rounded(px(15. * scale))
                    .border_1()
                    .border_color(border)
                    .bg(surface)
                    .into_any_element()
            });
            div()
                .relative()
                .children(behind)
                .child(self.toast(shown[0], cx))
        };
        let this = cx.entity().downgrade();
        let tracker = self.swipe.is_some().then(|| {
            canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    let (moved, released) = (this.clone(), this.clone());
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            moved.update(cx, |t, cx| t.drag(e.position.x, cx)).ok();
                        }
                    });
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            released.update(cx, |t, cx| t.release(cx)).ok();
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        });
        region.child(stack).children(tracker)
    }
}
