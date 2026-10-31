//! The internal debug panel (`toggleDebugModal`, Ctrl+D) and Help ▸ performance trace.
use gpui::{
    App, AppContext as _, Bounds, Context, FocusHandle, Focusable, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, actions, div, px, size,
};
use std::cell::RefCell;
use std::time::Instant;
use theme::ActiveTheme as _;

#[derive(Default)]
struct DebugWindow(Option<WindowHandle<DebugPanel>>);
impl Global for DebugWindow {}

thread_local! {
    static TRACE: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

actions!(debug, [TogglePanel, ToggleTraceRecording]);

pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &TogglePanel, cx| toggle_panel(cx));
    cx.on_action(|_: &ToggleTraceRecording, cx| toggle_trace(cx));
    cx.bind_keys([gpui::KeyBinding::new("ctrl-d", TogglePanel, None)]);
}

pub(crate) fn trace_running() -> bool {
    TRACE.with(|trace| trace.borrow().is_some())
}

fn toggle_trace(cx: &mut App) {
    let started = TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if trace.is_some() {
            *trace = None;
            false
        } else {
            *trace = Some(Instant::now());
            true
        }
    });
    let label = if started {
        "Performance trace started"
    } else {
        "Performance trace stopped"
    };
    crate::logging::write(&format!("codex-gpui: {label}"));
    toast(label, cx);
}

pub(crate) fn toggle_panel(cx: &mut App) {
    if let Some(handle) = cx.try_global::<DebugWindow>().and_then(|state| state.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(420.), px(320.)), cx);
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Debug".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            is_resizable: false,
            ..Default::default()
        },
        |window, cx| {
            let panel = cx.new(|cx| DebugPanel {
                focus: cx.focus_handle(),
                started: Instant::now(),
            });
            let focus = panel.read(cx).focus.clone();
            window.focus(&focus, cx);
            panel
        },
    ) {
        Ok(handle) => cx.default_global::<DebugWindow>().0 = Some(handle),
        Err(error) => crate::logging::write(&format!("codex-gpui: cannot open Debug: {error:#}")),
    }
}

struct DebugPanel {
    focus: FocusHandle,
    started: Instant,
}

impl Focusable for DebugPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn row(label: &'static str, value: SharedString) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .child(label)
        .child(value)
        .into_any_element()
}

impl Render for DebugPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let revision = crate::qa::REVISION;
        div()
            .id("debug-panel")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_2()
            .size_full()
            .p_4()
            .bg(t.menu_background)
            .text_color(t.text)
            .child("Debug")
            .child(row("Build revision", revision.to_owned().into()))
            .child(row("Trace recording", trace_running().to_string().into()))
            .child(row(
                "Session uptime (s)",
                self.started.elapsed().as_secs().to_string().into(),
            ))
            .into_any_element()
    }
}

fn toast(message: &'static str, cx: &mut App) {
    let Some(window) = cx.active_window() else {
        return;
    };
    window
        .update(cx, |root: &mut crate::Root, _, cx| {
            root.toast(ui::toast::Toast::info(message), cx);
        })
        .ok();
}

#[cfg(test)]
pub(crate) mod unit {
    use super::*;

    pub(crate) fn trace_toggle_roundtrip() {
        TRACE.with(|trace| {
            *trace.borrow_mut() = None;
        });
        assert!(!trace_running());
        TRACE.with(|trace| {
            *trace.borrow_mut() = Some(Instant::now());
        });
        assert!(trace_running());
        TRACE.with(|trace| {
            *trace.borrow_mut() = None;
        });
        assert!(!trace_running());
    }

    #[test]
    fn trace_toggle_flips_state() {
        trace_toggle_roundtrip();
    }
}
