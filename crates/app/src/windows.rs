//! Native window ownership and presentation. Page modules supply their own root views.

mod platform;

use crate::{Root, Route, State, host_settings, qa};
use anyhow::{Context as _, Result};
use async_channel::Receiver;
use bridge::{Bridge, Event};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Div, Entity, Global,
    InteractiveElement as _, Render, SharedString, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowId, WindowKind, WindowOptions, actions, point, px, size,
};
use std::{collections::HashMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Page,
    Detached,
    Secondary,
    Hud,
    HotkeyHome,
    HotkeyThread,
    Spotlight,
    AvatarOverlay,
    ScreenOverlay,
    GlobalDictation,
    BrowserCommentPopup,
}

impl Kind {
    fn panel(self) -> bool {
        matches!(
            self,
            Self::Hud
                | Self::HotkeyHome
                | Self::HotkeyThread
                | Self::Spotlight
                | Self::AvatarOverlay
                | Self::ScreenOverlay
                | Self::GlobalDictation
                | Self::BrowserCommentPopup
        )
    }

    fn retained(self) -> bool {
        matches!(
            self,
            Self::HotkeyHome
                | Self::HotkeyThread
                | Self::Spotlight
                | Self::AvatarOverlay
                | Self::ScreenOverlay
                | Self::GlobalDictation
        )
    }

    fn passive(self) -> bool {
        matches!(
            self,
            Self::AvatarOverlay | Self::ScreenOverlay | Self::GlobalDictation
        )
    }

    fn minimum(self) -> Option<gpui::Size<gpui::Pixels>> {
        let (width, height) = match self {
            Self::Primary | Self::Page => (480., 600.),
            Self::HotkeyHome => (320., 140.),
            Self::Detached | Self::Secondary | Self::Hud | Self::HotkeyThread => (400., 400.),
            _ => return None,
        };
        Some(size(px(width), px(height)))
    }

    fn bounds(
        self,
        display: Bounds<gpui::Pixels>,
        owner: Bounds<gpui::Pixels>,
    ) -> Bounds<gpui::Pixels> {
        let (width, height) = match self {
            Self::Primary | Self::Secondary | Self::Hud => (1280., 820.),
            Self::Page => (800., 720.),
            Self::Detached => (960., 720.),
            Self::HotkeyHome => (470., 290.),
            Self::HotkeyThread => (470., 640.),
            Self::Spotlight => (760., 640.),
            Self::AvatarOverlay => (356., 320.),
            Self::ScreenOverlay => (display.size.width.into(), display.size.height.into()),
            Self::GlobalDictation => (720., 180.),
            Self::BrowserCommentPopup => (294., 208.),
        };
        let mut extent = size(px(width), px(height));
        if matches!(self, Self::Page | Self::Detached) {
            extent.width = extent
                .width
                .min(owner.size.width)
                .min((display.size.width * 0.8).round());
            extent.height = extent
                .height
                .min(owner.size.height)
                .min((display.size.height * 0.8).round());
        }
        if matches!(self, Self::HotkeyHome | Self::HotkeyThread) {
            extent.width = extent.width.min(display.size.width);
            extent.height = extent.height.min(display.size.height);
        }
        let mut origin = display.center() - gpui::Point::from(extent / 2.);
        match self {
            Self::HotkeyThread => origin.y = display.origin.y + px(52.),
            Self::HotkeyHome => origin.y = display.origin.y + px(52. + 640.) - extent.height,
            Self::GlobalDictation => {
                origin.y = (display.bottom() - extent.height - px(16.)).max(display.top())
            }
            Self::ScreenOverlay => origin = display.origin,
            _ => {}
        }
        if matches!(self, Self::HotkeyHome | Self::HotkeyThread) {
            origin.y = origin
                .y
                .min(display.bottom() - extent.height)
                .max(display.top());
        }
        Bounds {
            origin,
            size: extent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Foreground,
    Background,
    Hidden,
}

pub struct Spec {
    pub kind: Kind,
    pub title: SharedString,
    pub owner: Option<AnyWindowHandle>,
    pub bounds: Option<WindowBounds>,
    pub presentation: Presentation,
}

impl Spec {
    pub fn new(kind: Kind, title: impl Into<SharedString>) -> Self {
        Self {
            kind,
            title: title.into(),
            owner: None,
            bounds: None,
            presentation: Presentation::Foreground,
        }
    }
}

#[derive(Clone, Copy)]
struct Entry {
    handle: AnyWindowHandle,
    kind: Kind,
    owner: Option<WindowId>,
    hide_after_fullscreen: bool,
}

#[derive(Default)]
struct Windows {
    entries: HashMap<WindowId, Entry>,
    last_primary: Option<WindowId>,
    #[cfg(target_os = "macos")]
    primary_active_at: Option<std::time::Instant>,
}
impl Global for Windows {}

pub fn init(cx: &mut App) {
    if cx.has_global::<Windows>() {
        return;
    }
    cx.set_global(Windows::default());
    cx.on_window_closed(|cx, id| {
        let dependents = {
            let windows = cx.global_mut::<Windows>();
            windows.entries.remove(&id);
            if windows.last_primary == Some(id) {
                windows.last_primary = None;
            }
            windows
                .entries
                .values()
                .filter(|entry| entry.owner == Some(id) && entry.kind == Kind::BrowserCommentPopup)
                .map(|entry| entry.handle)
                .collect::<Vec<_>>()
        };
        for handle in dependents {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if !cfg!(target_os = "macos") && cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Open a native window without coupling its page state to its owner's view.
pub fn open<V: Render + 'static>(
    spec: Spec,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<WindowHandle<V>> {
    init(cx);
    let owner_bounds = spec
        .owner
        .map(|handle| handle.update(cx, |_, window, _| window.bounds()))
        .transpose()?;
    let bounds = spec.bounds.unwrap_or_else(|| {
        let display = platform::work_area(owner_bounds, cx);
        WindowBounds::Windowed(spec.kind.bounds(display, owner_bounds.unwrap_or(display)))
    });
    let foreground =
        spec.presentation == Presentation::Foreground && !qa::ENABLED && !spec.kind.passive();
    let panel = spec.kind.panel();
    let resizable = matches!(
        spec.kind,
        Kind::Primary | Kind::Page | Kind::Detached | Kind::Secondary | Kind::HotkeyThread
    );
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: (!panel || spec.kind == Kind::Hud).then(|| TitlebarOptions {
            title: Some(spec.title.clone()),
            appears_transparent: spec.kind != Kind::Secondary,
            traffic_light_position: (spec.kind != Kind::Secondary).then(|| {
                if spec.kind == Kind::Hud {
                    point(px(10.), px(10.))
                } else {
                    point(px(16.), px(15.))
                }
            }),
        }),
        focus: foreground,
        show: spec.presentation != Presentation::Hidden,
        kind: if panel {
            WindowKind::PopUp
        } else {
            WindowKind::Normal
        },
        is_resizable: resizable,
        is_minimizable: !panel,
        window_min_size: spec.kind.minimum(),
        app_owns_titlebar_drag: !panel && spec.kind != Kind::Secondary,
        window_background: if panel {
            gpui::WindowBackgroundAppearance::Transparent
        } else {
            gpui::WindowBackgroundAppearance::Opaque
        },
        // Chromium animates visible unfocused windows at the display rate; GPUI's default
        // caps them at 30 fps, and passive panels (Mini, overlays) never take focus.
        inactive_frame_interval: None,
        ..Default::default()
    };
    let kind = spec.kind;
    let handle = cx.open_window(options, |window, cx| {
        platform::configure(window, kind);
        if kind.retained() {
            window.on_window_should_close(cx, |window, _| !platform::hide(window));
        }
        let view = build(window, cx);
        view.update(cx, |_, cx| {
            cx.observe_window_activation(window, move |_, window, cx| {
                if kind == Kind::Primary {
                    apply_backdrop(window);
                }
                let id = window.window_handle().window_id();
                if let Some(entry) = cx.global::<Windows>().entries.get(&id)
                    && entry.kind == Kind::Primary
                {
                    let windows = cx.global_mut::<Windows>();
                    // Set on activation and on deactivation: the last moment it was active.
                    #[cfg(target_os = "macos")]
                    {
                        windows.primary_active_at = Some(std::time::Instant::now());
                    }
                    if window.is_window_active() {
                        windows.last_primary = Some(id);
                    }
                }
            })
            .detach();
            cx.observe_window_bounds(window, move |_, window, cx| {
                if kind == Kind::Primary {
                    apply_backdrop(window);
                }
                remember_bounds(window, cx)
            })
            .detach();
        });
        view
    })?;
    let id = handle.window_id();
    let windows = cx.global_mut::<Windows>();
    windows.entries.insert(
        id,
        Entry {
            handle: handle.into(),
            kind,
            owner: spec.owner.map(|owner| owner.window_id()),
            hide_after_fullscreen: false,
        },
    );
    if kind == Kind::Primary {
        windows.last_primary = Some(id);
    }
    Ok(handle)
}

/// The original's macOS backdrop (`applyWindowBackdrop`): `vibrancy: 'menu'` under the page
/// while focused; an opaque surface when blurred or at 3840x2160 device px or more.
fn apply_backdrop(window: &mut Window) {
    let size = window
        .bounds()
        .size
        .map(|v| f32::from(v) * window.scale_factor());
    let large = size.width.max(size.height) >= 3840. && size.width.min(size.height) >= 2160.;
    let backdrop = if cfg!(target_os = "macos") && window.is_window_active() && !large {
        gpui::WindowBackgroundAppearance::Blurred
    } else {
        gpui::WindowBackgroundAppearance::Opaque
    };
    if window.background_appearance() != backdrop {
        window.set_background_appearance(backdrop);
    }
}

pub fn handles(kind: Kind, cx: &App) -> Vec<AnyWindowHandle> {
    cx.try_global::<Windows>()
        .into_iter()
        .flat_map(|windows| windows.entries.values())
        .filter(|entry| entry.kind == kind)
        .map(|entry| entry.handle)
        .collect()
}

pub(crate) fn set_kind(window: &Window, kind: Kind, cx: &mut App) {
    if let Some(entry) = cx
        .try_global::<Windows>()
        .and_then(|windows| windows.entries.get(&window.window_handle().window_id()))
    {
        let id = entry.handle.window_id();
        cx.global_mut::<Windows>()
            .entries
            .get_mut(&id)
            .unwrap()
            .kind = kind;
        platform::configure(window, kind);
    }
}

/// Keep the final primary window and its draft alive on macOS.
pub fn allow_close(window: &mut Window, cx: &mut App) -> bool {
    let id = window.window_handle().window_id();
    ui::composer::dictation::Dispatcher::cancel(id, cx);
    let Some(windows) = cx.try_global::<Windows>() else {
        return true;
    };
    let Some(entry) = windows.entries.get(&id) else {
        return true;
    };
    let retain = entry.kind.retained()
        || (cfg!(target_os = "macos")
            && entry.kind == Kind::Primary
            && windows
                .entries
                .values()
                .filter(|entry| entry.kind == Kind::Primary)
                .count()
                == 1);
    if retain && cfg!(target_os = "macos") && window.is_fullscreen() {
        cx.global_mut::<Windows>()
            .entries
            .get_mut(&id)
            .unwrap()
            .hide_after_fullscreen = true;
        window.toggle_fullscreen();
        return false;
    }
    !retain || !platform::hide(window)
}

pub fn close(window: &mut Window, cx: &mut App) {
    if allow_close(window, cx) {
        window.remove_window();
    }
}

pub fn show(handle: AnyWindowHandle, presentation: Presentation, cx: &mut App) -> Result<()> {
    handle.update(cx, |_, window, _| {
        if presentation == Presentation::Hidden {
            anyhow::ensure!(
                platform::hide(window),
                "hiding native windows requires AppKit"
            );
        } else {
            platform::show(
                window,
                presentation == Presentation::Foreground && !qa::ENABLED,
            )?;
        }
        Ok(())
    })?
}

pub fn is_visible(handle: AnyWindowHandle, cx: &mut App) -> Result<bool> {
    handle.update(cx, |_, window, _| platform::is_visible(window))?
}

pub(crate) fn set_keyboard_interactive(window: &Window, interactive: bool) -> Result<()> {
    platform::set_keyboard_interactive(window, interactive)
}

pub(crate) fn set_pointer_interactive_window(window: &Window, interactive: bool) -> Result<()> {
    platform::set_pointer_interactive(window, interactive)
}

pub fn set_bounds(
    handle: AnyWindowHandle,
    bounds: Bounds<gpui::Pixels>,
    cx: &mut App,
) -> Result<Bounds<gpui::Pixels>> {
    anyhow::ensure!(
        [
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bounds.size.height
        ]
        .iter()
        .all(|value| f32::from(*value).is_finite())
            && bounds.size.width > px(0.)
            && bounds.size.height > px(0.),
        "invalid window bounds"
    );
    handle.update(cx, |_, window, cx| {
        platform::set_bounds(window, bounds)?;
        window.bounds_changed(cx);
        Ok(window.bounds())
    })?
}

pub fn set_pointer_interactive(
    handle: AnyWindowHandle,
    interactive: bool,
    cx: &mut App,
) -> Result<()> {
    handle.update(cx, |_, window, _| {
        platform::set_pointer_interactive(window, interactive)
    })?
}

pub fn reopen(cx: &mut App) -> Result<()> {
    if let Some(handle) = primary(cx) {
        return show(handle, Presentation::Foreground, cx);
    }
    let cwd = std::env::current_dir().context("read launch directory")?;
    open_primary(State::load(), cwd.to_string_lossy().into_owned(), cx)?;
    Ok(())
}

/// The last active primary window, else any primary: the window a deep link or Dock
/// reopen addresses.
pub fn primary(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Windows>().and_then(|windows| {
        windows
            .last_primary
            .and_then(|id| windows.entries.get(&id))
            .or_else(|| {
                windows
                    .entries
                    .values()
                    .find(|entry| entry.kind == Kind::Primary)
            })
            .map(|entry| entry.handle)
    })
}

/// Whether a primary window is active, or was within `within`: the original's
/// `wasPrimaryWindowFocusedWithin`.
#[cfg(target_os = "macos")]
pub(crate) fn primary_active_within(within: std::time::Duration, cx: &mut App) -> bool {
    let recent = cx
        .try_global::<Windows>()
        .and_then(|windows| windows.primary_active_at)
        .is_some_and(|at| at.elapsed() <= within);
    recent
        || primary(cx).is_some_and(|handle| {
            handle
                .update(cx, |_, window, _| window.is_window_active())
                .unwrap_or(false)
        })
}

/// `primary` as a typed root, opening a hidden primary when none exists: the
/// window screenshot capture addresses.
pub(crate) fn primary_or_open(cx: &mut App) -> Result<WindowHandle<Root>> {
    if let Some(handle) = primary(cx).and_then(|handle| handle.downcast::<Root>()) {
        return Ok(handle);
    }
    let cwd = std::env::current_dir().context("read launch directory")?;
    open_primary_with_presentation(
        State::load(),
        cwd.to_string_lossy().into_owned(),
        Presentation::Hidden,
        cx,
    )
}

pub(crate) fn connect(cwd: &Path) -> Result<(Bridge, Receiver<Event>)> {
    if let Some(mut command) = bridge::acp_command() {
        command.current_dir(cwd);
        Bridge::spawn_acp(command)
    } else {
        let mut command = bridge::default_command();
        qa::configure_command(&mut command)?;
        command.current_dir(cwd);
        Bridge::spawn(command)
    }
}

pub fn open_primary(state: State, cwd: String, cx: &mut App) -> Result<WindowHandle<Root>> {
    open_primary_with_presentation(state, cwd, Presentation::Foreground, cx)
}

fn open_primary_with_presentation(
    state: State,
    cwd: String,
    presentation: Presentation,
    cx: &mut App,
) -> Result<WindowHandle<Root>> {
    if !cx.has_global::<host_settings::Settings>() {
        let settings = match &state.path {
            Some(path) => host_settings::Settings::load(path.with_file_name("host-settings.json"))?,
            None => host_settings::Settings::default(),
        };
        cx.set_global(settings);
    }
    // An app-server that cannot start shows the crash page, whose Try again respawns it.
    let (bridge, events) = connect(Path::new(&cwd)).unwrap_or_else(|error| {
        eprintln!("codex-gpui: cannot start the app-server: {error:#}");
        Bridge::failed()
    });
    let mut spec = Spec::new(
        Kind::Primary,
        if qa::ENABLED {
            "Codex GPUI QA"
        } else {
            "Codex"
        },
    );
    spec.bounds = Some(primary_bounds(&state, cx));
    spec.presentation = presentation;
    let handle = open(spec, cx, |window, cx| {
        cx.new(|cx| Root::new(state, bridge, events, cwd, window, cx))
    })?;
    crate::hotkey::primary(handle, cx);
    Ok(handle)
}

fn primary_bounds(state: &State, cx: &App) -> WindowBounds {
    let fallback = Bounds::centered(None, size(px(state.width), px(state.height)), cx);
    let settings = cx.global::<host_settings::Settings>().read_all();
    let stored = settings
        .get("electron-main-window-bounds")
        .unwrap_or(&serde_json::Value::Null);
    let parse = || {
        let x = stored["x"].as_f64()? as f32;
        let y = stored["y"].as_f64()? as f32;
        let width = stored["width"].as_f64()? as f32;
        let height = stored["height"].as_f64()? as f32;
        [x, y, width, height]
            .iter()
            .all(|value| value.is_finite())
            .then_some(())?;
        Some(Bounds::new(
            point(px(x), px(y)),
            size(px(width.max(480.)), px(height.max(600.))),
        ))
    };
    let bounds = parse()
        .filter(|bounds| {
            cx.displays()
                .iter()
                .any(|display| display.bounds().intersect(bounds) == *bounds)
        })
        .unwrap_or(fallback);
    if stored["isMaximized"] == true {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

fn remember_bounds(window: &Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    if !window.is_fullscreen()
        && let Some(entry) = cx.global_mut::<Windows>().entries.get_mut(&id)
        && std::mem::take(&mut entry.hide_after_fullscreen)
    {
        platform::hide(window);
    }
    if !cx
        .global::<Windows>()
        .entries
        .get(&id)
        .is_some_and(|entry| entry.kind == Kind::Primary)
    {
        return;
    }
    let bounds = window.window_bounds().get_bounds();
    let value = serde_json::json!({
        "x": f32::from(bounds.origin.x), "y": f32::from(bounds.origin.y),
        "width": f32::from(bounds.size.width).max(480.), "height": f32::from(bounds.size.height).max(600.),
        "isMaximized": matches!(window.window_bounds(), WindowBounds::Maximized(_)),
    });
    if cx.has_global::<host_settings::Settings>() {
        let settings = cx.global_mut::<host_settings::Settings>();
        let changed = settings.read_all().get("electron-main-window-bounds") != Some(&value);
        if changed && let Err(error) = settings.write("electron-main-window-bounds", value) {
            eprintln!("codex-gpui: cannot save window bounds: {error:#}");
        }
    }
}

actions!(windows, [OpenThreadWindow, NewWindow]);

pub(crate) fn bind_root(window: &mut Window, cx: &mut Context<Root>) {
    cx.observe_window_activation(window, |root, window, cx| {
        if window.is_window_active() {
            cx.set_global(ui::screens::settings::AppServer(root.bridge.clone()));
        }
        crate::hotkey::activation_changed(root, window, cx);
    })
    .detach();
}

pub(crate) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &NewWindow, _, cx| {
        if let Err(error) = open_primary(new_window_state(State::load()), this.cwd.clone(), cx) {
            this.toast(
                ui::toast::Toast::danger("Could not open the window")
                    .description(format!("{error:#}")),
                cx,
            );
        }
    }))
    .on_action(cx.listener(|this, _: &OpenThreadWindow, _, cx| {
        if !matches!(this.route(), Route::Thread(_)) {
            return;
        }
        let state = State {
            width: this.state.width,
            height: this.state.height,
            sidebar_open: this.state.sidebar_open,
            sidebar_width: this.state.sidebar_width,
            route: this.route().clone(),
            theme: this.state.theme,
            confirm_before_close: this.state.confirm_before_close,
            project_root: this.state.project_root.clone(),
            zoom: this.state.zoom,
            prompt_history: this.state.prompt_history.clone(),
            path: this.state.path.clone(),
        };
        if let Err(error) = open_primary(state, this.cwd.clone(), cx) {
            this.toast(
                ui::toast::Toast::danger("Could not open the window")
                    .description(format!("{error:#}")),
                cx,
            );
        }
    }))
}

fn new_window_state(mut state: State) -> State {
    state.route = Route::Home;
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_geometry_stays_on_the_selected_display() {
        let display = Bounds::new(point(px(-1600.), px(25.)), size(px(1600.), px(875.)));
        let owner = Bounds::new(point(px(-1500.), px(100.)), size(px(700.), px(620.)));
        let detached = Kind::Detached.bounds(display, owner);
        assert_eq!(detached.size, size(px(700.), px(620.)));
        assert_eq!(display.intersect(&detached), detached);
        let home = Kind::HotkeyHome.bounds(display, owner);
        let thread = Kind::HotkeyThread.bounds(display, owner);
        assert_eq!(home.bottom(), thread.bottom());
        assert_eq!(thread.top(), px(77.));
        let dictation = Kind::GlobalDictation.bounds(display, owner);
        assert_eq!(display.bottom() - dictation.bottom(), px(16.));
        let small = Bounds::new(point(px(0.), px(0.)), size(px(300.), px(220.)));
        for kind in [Kind::HotkeyHome, Kind::HotkeyThread] {
            let bounds = kind.bounds(small, small);
            assert_eq!(small.intersect(&bounds), bounds);
        }
    }
    #[test]
    fn new_window_state_always_starts_on_home() {
        let state = new_window_state(State {
            route: Route::Thread("thread".into()),
            ..State::default()
        });
        assert!(matches!(state.route, Route::Home));
    }

    #[test]
    fn new_window_action_opens_an_additional_home_window() {
        use gpui::HeadlessAppContext;
        use serde_json::Value;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        // Replay the recorded app-server transcript so the primary window becomes
        // ready; only a ready Root renders the main tree that binds NewWindow.
        let recorded = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl"),
        )
        .expect("read the recorded app-server transcript");
        let lines = recorded
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<serde_json::Result<Vec<_>>>()
            .expect("parse the recorded app-server transcript");
        let requested = lines
            .iter()
            .filter(|line| {
                line["dir"] == "send"
                    && matches!(
                        line["msg"]["method"].as_str(),
                        Some(
                            "account/read"
                                | "thread/list"
                                | "model/list"
                                | "config/read"
                                | "skills/list"
                        )
                    )
            })
            .map(|line| line["msg"]["id"].clone())
            .collect::<Vec<_>>();
        let repeated = lines
            .iter()
            .filter(|line| !line["msg"]["id"].is_null() && requested.contains(&line["msg"]["id"]))
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let recording = tempfile::NamedTempFile::new().expect("create the replay file");
        std::fs::write(
            recording.path(),
            format!("{recorded}\n{repeated}\n{repeated}\n"),
        )
        .expect("write the replay file");
        let (bridge, events) =
            bridge::Bridge::replay(recording.path()).expect("build the replayed app-server bridge");

        // Linux checks need layout and input without a GPU; macOS captures render headlessly.
        #[cfg(not(target_os = "linux"))]
        let renderer = gpui_platform::current_headless_renderer;
        #[cfg(target_os = "linux")]
        let renderer = || Ok(None);
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            renderer,
        );
        // Mirror main.rs startup: fonts, theme global, then app::init.
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
            crate::init(cx);
        });
        cx.allow_parking();
        let handle = cx
            .open_window(gpui::size(gpui::px(900.), gpui::px(700.)), |window, cx| {
                cx.new(|cx| {
                    Root::new(
                        State {
                            theme: crate::ThemeSetting::Dark,
                            ..Default::default()
                        },
                        bridge,
                        events,
                        std::env::temp_dir().to_string_lossy().into_owned(),
                        window,
                        cx,
                    )
                })
            })
            .expect("open the primary window");
        let window: gpui::AnyWindowHandle = handle.into();
        let root = cx.read_window(&handle, |root, _| root).expect("read Root");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.read_with(&cx, |root, _| root.ready()) {
            assert!(
                Instant::now() < deadline,
                "the primary window never became ready"
            );
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(1));
        }
        // Draw the ready main tree so its NewWindow handler is in the dispatch tree.
        cx.advance_clock(Duration::from_millis(16));
        cx.test_window(window)
            .simulate_frame_request(gpui::RequestFrameOptions {
                require_presentation: true,
                ..Default::default()
            });
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.dispatch_action(Box::new(NewWindow), cx);
        })
        .expect("dispatch NewWindow");
        // The dispatch is deferred; let it run and open the second window.
        cx.run_until_parked();

        let windows_are_home = cx.update(|cx| {
            let windows = cx.windows();
            assert_eq!(windows.len(), 2);
            windows.into_iter().all(|handle| {
                handle.downcast::<Root>().is_some_and(|root| {
                    root.read(cx)
                        .is_ok_and(|root| matches!(root.route(), Route::Home))
                })
            })
        });
        assert!(windows_are_home);
    }
}
