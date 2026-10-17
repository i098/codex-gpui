//! Shared harness of the headless app tests: the real [`app::Root`] on a replayed
//! app-server, driven by mouse and key events. Host history starts from the recorded oracle.

#![allow(dead_code)]

use anyhow::{Context as _, Result, ensure};
use app::{Root, Route, SidePanel, State, ThemeSetting};
use bridge::{Bridge, Event, RpcError};
use gpui::{
    Action, AnyWindowHandle, App, AppContext as _, Entity, EntityInputHandler as _, Focusable as _,
    HeadlessAppContext, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, Point, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px, size,
};
use image::RgbaImage;
use serde_json::{Value, json};
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Wake, Waker};
use std::time::{Duration, Instant};
use std::{env, fs, io, pin::pin, thread};
use ui::Shell;
use ui::composer::Composer;
use ui::screens::settings::AppServer;

pub const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// The fixtures' project (`thread/start` cwd).
pub const CWD: &str = "/tmp/cgx-mac/proj";
/// Recents order of readback.jsonl's `thread/list`.
pub const GREETING: &str = "01a13de9-c48a-7b20-8382-db900881cb99";
pub const SUBTRACT: &str = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";
pub const EXPLAIN: &str = "01a13de9-35ed-7c72-a36a-68eeb2f4e1ee";
pub const GREETING_PROMPT: &str = "Change the greeting in hello.py to \"Hello, Codex\".";
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The original capture clock's base, not its unrecorded historical screenshot instant.
pub const CAPTURE_CLOCK: ui::screens::thread::view::Clock = ui::screens::thread::view::Clock {
    utc_offset: -7 * 3600,
    pinned_now: Some(1_792_083_600),
    elapsed_since: None,
};

pub fn replay_adapter(recording: &Path) -> Result<Vec<Value>> {
    let stem = recording
        .file_stem()
        .and_then(|s| s.to_str())
        .context("recording name")?;
    let target = match stem {
        "ms4" => "stream_fade",
        name => name,
    };
    ensure!(
        target != "replay"
            && Path::new(ROOT)
                .join("crates/app/tests")
                .join(format!("{target}.rs"))
                .is_file(),
        "unsupported recording contract {stem:?}: no dedicated adapter"
    );
    let log = (stem == "s017").then(|| {
        Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("replay-{stem}-{}.log", std::process::id()))
    });
    let mut command = Command::new(env!("CARGO"));
    command.current_dir(ROOT).args([
        "test",
        "-p",
        "app",
        "--test",
        target,
        "--",
        "--test-threads=1",
        "--nocapture",
    ]);
    if let Some(log) = &log {
        command.env("CGX_REPLAY_REQUEST_LOG", log);
    }
    let status = command
        .status()
        .with_context(|| format!("run {target} adapter for {stem}"))?;
    ensure!(status.success(), "{target} adapter failed: {status}");
    let Some(log) = log else {
        return Ok(Vec::new());
    };
    let requests = fs::read_to_string(&log)?
        .lines()
        .map(serde_json::from_str)
        .collect::<serde_json::Result<Vec<Value>>>()?;
    fs::remove_file(log)?;
    Ok(requests)
}

/// Titlebar and rail buttons (inventory §2.2, §3): 28 px titlebar buttons at y 8, 36 px rail
/// buttons at x 8.
pub const BACK: (f32, f32) = (102., 22.);
pub const FORWARD: (f32, f32) = (136., 22.);
pub const TOGGLE_SIDEBAR: (f32, f32) = (170., 22.);
pub const RAIL_HOME: (f32, f32) = (26., 70.);
pub const RAIL_SPACE: (f32, f32) = (26., 114.);
pub const RAIL_SCHEDULED: (f32, f32) = (26., 158.);
/// Sidebar `Search`.
pub const SEARCH: (f32, f32) = (314., 70.);
/// The main panel at 1280x832 with the sidebar open.
pub const MAIN_PANEL: [f32; 4] = [340., 44., 936., 784.];
/// The sidebar panel.
pub const SIDEBAR_PANEL: [f32; 4] = [52., 44., 288., 784.];

// --- Harness ---

/// Returns in-memory host settings with the original baseline's recorded announcement history.
/// Fails if the oracle has no history array. Fresh recordings must supply their own initial history.
pub fn recorded_host_settings() -> Result<app::host_settings::Settings> {
    let mut original: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/tools/route1/oracle.json"))?)?;
    let history = original["persisted-atoms"]["state"]["seen-model-upgrade-list"].take();
    ensure!(history.is_array(), "original host history is missing");
    let mut settings = app::host_settings::Settings::default();
    settings.write("seen-model-upgrade-list", history)?;
    Ok(settings)
}

/// The app on a replayed app-server in a headless window.
pub struct Fx {
    pub cx: HeadlessAppContext,
    pub root: Entity<Root>,
    pub window: AnyWindowHandle,
    pub theme: &'static str,
    /// Window size in CSS px.
    pub size: (f32, f32),
    /// Client-message log of the proxy app-server ([`Fx::captured`]).
    log: Option<PathBuf>,
}

impl Fx {
    /// Dark, 1280x832, readback only.
    pub fn new() -> Result<Self> {
        Self::with("dark", (1280., 832.), None, |_| {})
    }

    /// `session` (under fixtures/) is appended to readback.jsonl; `edit` rewrites each
    /// recorded line first.
    pub fn with(
        theme: &'static str,
        size: (f32, f32),
        session: Option<&str>,
        edit: fn(&mut Value),
    ) -> Result<Self> {
        Self::open(theme, size, script(session, edit)?)
    }

    /// Dark, 1280x832, on the replay script `lines` ([`script`] plus crafted lines).
    pub fn scripted(lines: Vec<Value>) -> Result<Self> {
        Self::open("dark", (1280., 832.), lines)
    }

    pub fn open(theme: &'static str, size: (f32, f32), lines: Vec<Value>) -> Result<Self> {
        let (bridge, events) = Bridge::replay(&write_script(&lines)?)?;
        Self::boot(theme, size, bridge, events, recorded_host_settings()?)
    }

    /// Like [`Fx::open`], but the app-server is this test binary again as a proxy
    /// ([`proxy_child`]) that logs every client message: read them with [`Fx::sent`]. The
    /// binary must have a `#[test] fn proxy() { proxy_child() }`. Uses the recorded host history.
    pub fn captured(theme: &'static str, size: (f32, f32), lines: Vec<Value>) -> Result<Self> {
        Self::captured_with_host_settings(theme, size, lines, recorded_host_settings()?)
    }

    /// Like [`Fx::captured`], but installs `settings` before [`Root`] starts.
    /// Use the genuine recording's initial host settings for fresh-state or reload checks.
    pub fn captured_with_host_settings(
        theme: &'static str,
        size: (f32, f32),
        lines: Vec<Value>,
        settings: app::host_settings::Settings,
    ) -> Result<Self> {
        let script = write_script(&lines)?;
        let log = env::var_os("CGX_REPLAY_REQUEST_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| script.with_extension("log"));
        fs::write(&log, "")?;
        let mut cmd = Command::new(env::current_exe()?);
        // Quiet and uncaptured: libtest then prints only a "running 1 test" line to stdout,
        // which the bridge skips as unparsable, before the proxy's JSON lines.
        cmd.args(["--exact", "proxy", "--nocapture", "--quiet"])
            .env(PROXY_SCRIPT, &script)
            .env(PROXY_LOG, &log);
        let (bridge, events) = Bridge::spawn(cmd)?;
        let mut fx = Self::boot(theme, size, bridge, events, settings)?;
        fx.log = Some(log);
        Ok(fx)
    }

    fn boot(
        theme: &'static str,
        (w, h): (f32, f32),
        bridge: Bridge,
        events: async_channel::Receiver<Event>,
        settings: app::host_settings::Settings,
    ) -> Result<Self> {
        let clock = ui::screens::thread::view::Clock {
            elapsed_since: Some(Instant::now()),
            ..CAPTURE_CLOCK
        };
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(match theme {
                "light" => theme::Theme::light(),
                _ => theme::Theme::dark(),
            });
            cx.set_global(settings);
            app::init(cx);
        });
        // The replay answers on its own threads: let the scheduler wait for real I/O.
        cx.allow_parking();
        let window = cx.open_window(size(px(w), px(h)), |window, cx| {
            let theme = match theme {
                "light" => ThemeSetting::Light,
                _ => ThemeSetting::Dark,
            };
            let state = State {
                theme,
                ..State::default()
            };
            cx.new(|cx| Root::new(state, bridge, events, CWD.into(), window, cx).with_clock(clock))
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        let mut fx = Self {
            cx,
            root,
            window: window.into(),
            theme,
            size: (w, h),
            log: None,
        };
        fx.wait("startup", |fx| fx.read(|r, _| r.ready()))?;
        Ok(fx)
    }

    /// Every message the app sent to the app-server, oldest first, as `{method, params?}`
    /// (`id` stripped) or, for an answer to a server request, `{reply: id, result}`. Needs
    /// [`Fx::captured`].
    pub fn sent(&mut self) -> Result<Vec<Value>> {
        let log = self.log.as_ref().context("not a captured Fx")?;
        // The proxy handles its stdin in order, so its answer to the barrier means that every
        // earlier message is in the log.
        let bridge = self.cx.update(|cx| cx.global::<AppServer>().0.clone());
        block_on(bridge.request::<_, Value>(PROXY_BARRIER, json!({})))?;
        fs::read_to_string(log)?
            .lines()
            .map(|l| Ok(serde_json::from_str(l)?))
            .collect()
    }

    pub fn read<R>(&self, f: impl FnOnce(&Root, &App) -> R) -> R {
        self.root.read_with(&self.cx, f)
    }

    pub fn route(&self) -> Route {
        self.read(|r, _| r.route().clone())
    }

    pub fn shell<R>(&self, f: impl FnOnce(&Shell) -> R) -> R {
        self.read(|r, cx| f(r.shell().read(cx)))
    }

    pub fn sidebar_open(&self) -> bool {
        self.shell(|s| s.sidebar_open)
    }

    pub fn composer(&self) -> Entity<Composer> {
        self.read(|r, cx| r.composer(cx))
    }

    pub fn composer_text(&mut self) -> Result<String> {
        let composer = self.composer();
        self.cx.update_window(self.window, |_, window, cx| {
            composer
                .update(cx, |c, cx| {
                    c.text_for_range(0..usize::MAX, &mut None, window, cx)
                })
                .unwrap_or_default()
        })
    }

    pub fn composer_focused(&mut self) -> Result<bool> {
        let composer = self.composer();
        self.cx.update_window(self.window, |_, window, cx| {
            composer.focus_handle(cx).is_focused(window)
        })
    }

    pub fn thread_id(&self) -> Option<String> {
        self.read(|r, cx| Some(r.thread()?.read(cx).id().to_owned()))
    }

    /// Runs the app until `done`, polling the replay's I/O, for at most [`TIMEOUT`].
    pub fn wait(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) -> Result<()> {
        let started = Instant::now();
        loop {
            self.cx.run_until_parked();
            if done(self) {
                return Ok(());
            }
            ensure!(started.elapsed() < TIMEOUT, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The route shows thread `id`, read with `thread/read`.
    pub fn wait_thread(&mut self, id: &str) -> Result<()> {
        self.wait(&format!("thread {id}"), |fx| {
            fx.route() == Route::Thread(id.into()) && fx.thread_id().as_deref() == Some(id)
        })
    }

    pub fn input(&mut self, event: PlatformInput) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_event(event, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    /// Advances the test clock and renders the next animation frame.
    pub fn advance(&mut self, duration: Duration) -> Result<()> {
        self.cx.advance_clock(duration);
        // Present virtual-time captures without the inactive window's wall-clock throttle.
        self.cx
            .test_window(self.window)
            .simulate_frame_request(gpui::RequestFrameOptions {
                require_presentation: true,
                ..Default::default()
            });
        self.cx.run_until_parked();
        Ok(())
    }

    pub fn hover(&mut self, (x, y): (f32, f32)) -> Result<()> {
        self.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(x), px(y)),
            pressed_button: None,
            modifiers: Modifiers::default(),
        }))
    }

    /// Scrolls the wheel at `at` by `dy` CSS px (negative: down the page).
    pub fn scroll(&mut self, at: (f32, f32), dy: f32) -> Result<()> {
        self.hover(at)?;
        self.wheel(
            at,
            ScrollDelta::Pixels(point(px(0.), px(dy))),
            TouchPhase::Moved,
        )
    }

    /// One wheel event at `at`, the pointer already there. macOS sends a trackpad flick's
    /// momentum events after the `Ended` phase; gpui_macos reports them as `Moved` pixels.
    pub fn wheel(
        &mut self,
        at: (f32, f32),
        delta: ScrollDelta,
        touch_phase: TouchPhase,
    ) -> Result<()> {
        self.input(PlatformInput::ScrollWheel(ScrollWheelEvent {
            position: point(px(at.0), px(at.1)),
            delta,
            modifiers: Modifiers::default(),
            touch_phase,
        }))
    }

    pub fn click(&mut self, at: (f32, f32)) -> Result<()> {
        self.press(at, MouseButton::Left)
    }

    pub fn right_click(&mut self, at: (f32, f32)) -> Result<()> {
        self.press(at, MouseButton::Right)
    }

    /// Moves to `at`, then presses and releases `button`.
    pub fn press(&mut self, at: (f32, f32), button: MouseButton) -> Result<()> {
        self.hover(at)?;
        let position: Point<_> = point(px(at.0), px(at.1));
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }))?;
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
        }))
    }

    /// Presses the left button at `from`, moves to `to` in `moves` equal steps, releases.
    pub fn drag(&mut self, from: (f32, f32), to: (f32, f32), moves: u32) -> Result<()> {
        self.hover(from)?;
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(from.0), px(from.1)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }))?;
        let moves = moves.max(1);
        for i in 1..=moves {
            let t = i as f32 / moves as f32;
            self.input(PlatformInput::MouseMove(MouseMoveEvent {
                position: point(
                    px(from.0 + (to.0 - from.0) * t),
                    px(from.1 + (to.1 - from.1) * t),
                ),
                pressed_button: Some(MouseButton::Left),
                modifiers: Modifiers::default(),
            }))?;
        }
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(to.0), px(to.1)),
            modifiers: Modifiers::default(),
            click_count: 1,
        }))
    }

    /// Resizes the window to `w` x `h` CSS px.
    pub fn resize(&mut self, w: f32, h: f32) -> Result<()> {
        self.cx
            .simulate_window_resize(self.window, size(px(w), px(h)));
        self.cx.run_until_parked();
        self.size = (w, h);
        Ok(())
    }

    /// Space-separated keystrokes, e.g. `"cmd-shift-["`.
    pub fn keys(&mut self, keys: &str) -> Result<()> {
        for key in keys.split(' ') {
            let stroke = Keystroke::parse(key)?;
            self.cx.update_window(self.window, |_, window, cx| {
                window.dispatch_keystroke(stroke, cx);
            })?;
            self.cx.run_until_parked();
        }
        Ok(())
    }

    pub fn type_text(&mut self, s: &str) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            for c in s.chars() {
                let key = c.to_string();
                window.dispatch_keystroke(
                    Keystroke {
                        key,
                        ..Default::default()
                    },
                    cx,
                );
            }
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    /// Clicks `menu` ▸ `item` of [`app::menus`]: dispatches its action like the menu bar.
    pub fn menu(&mut self, menu: &str, item: &str) -> Result<()> {
        let action = app::menus()
            .into_iter()
            .filter(|m| m.name == menu)
            .flat_map(|m| m.items)
            .find_map(|i| match i {
                gpui::MenuItem::Action { name, action, .. } if name == item => Some(action),
                _ => None,
            })
            .with_context(|| format!("no menu item {menu} ▸ {item}"))?;
        self.action(action)
    }

    pub fn action(&mut self, action: Box<dyn Action>) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_action(action, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    pub fn shot(&mut self) -> Result<RgbaImage> {
        self.advance(Duration::ZERO)?;
        self.cx.capture_screenshot(self.window)
    }

    /// Matches the reference pointer and caret preparation without changing keyboard focus.
    pub fn reference_shot(&mut self) -> Result<RgbaImage> {
        self.hover(((self.size.0 * 0.6).round(), 6.))?;
        self.cx.update_window(self.window, |_, window, cx| {
            Composer::set_caret_transparent(cx, true);
            window.refresh();
        })?;
        let image = self.shot();
        self.cx.update_window(self.window, |_, window, cx| {
            Composer::set_caret_transparent(cx, false);
            window.refresh();
        })?;
        image
    }

    /// Center of the original's element: a node with this `aria-label`, else the first text
    /// starting with `key`, in refs/<screen>/dark@1280x832.spec.json.
    pub fn at(&self, screen: &str, key: &str) -> Result<(f32, f32)> {
        let path = format!("{ROOT}/refs/{screen}/dark@1280x832.spec.json");
        let spec: Value = serde_json::from_slice(&fs::read(&path).context(path.clone())?)?;
        let rect = spec["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|n| n["label"] == key)
            .or_else(|| {
                spec["texts"].as_array().into_iter().flatten().find(|t| {
                    t["text"]
                        .as_str()
                        .is_some_and(|s| s.trim().starts_with(key))
                })
            })
            .map(|n| &n["rect"])
            .with_context(|| format!("{path}: no element {key:?}"))?;
        let r: [f32; 4] = serde_json::from_value(rect.clone())?;
        Ok((r[0] + r[2] / 2., r[1] + r[3] / 2.))
    }

    /// Diffs the window against `refs/<screen>/<theme>@<W>x<H>.png`.
    pub fn parity(&mut self, screen: &str) -> Result<String> {
        let port = self.reference_shot()?;
        parity_image(screen, self.theme, &port)
    }
}

/// readback.jsonl, then `session` (under fixtures/), each recorded line rewritten by `edit`.
pub fn script(session: Option<&str>, edit: fn(&mut Value)) -> Result<Vec<Value>> {
    let mut lines = Vec::new();
    for file in std::iter::once("readback.jsonl").chain(session) {
        let path = format!("{ROOT}/fixtures/{file}");
        let text = fs::read_to_string(&path).with_context(|| path.clone())?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let mut v: Value = serde_json::from_str(line)?;
            edit(&mut v);
            lines.push(v);
        }
    }
    Ok(lines)
}

/// A crafted `method` request that the replay answers with `result`. The fixtures hold no
/// recording of it; the replay matches it by method (the app's params are not compared).
pub fn exchange(method: &str, result: Value) -> [Value; 2] {
    static IDS: AtomicUsize = AtomicUsize::new(90_000);
    let id = IDS.fetch_add(1, Ordering::Relaxed);
    [
        json!({ "dir": "send", "msg": { "id": id, "method": method, "params": {} } }),
        json!({ "dir": "recv", "msg": { "id": id, "result": result } }),
    ]
}

/// readback.jsonl's `thread/read` thread `id`, with its turns.
pub fn recorded_thread(id: &str) -> Result<Value> {
    script(None, |_| {})?
        .into_iter()
        .find_map(|mut v| {
            let thread = v.pointer_mut("/msg/result/thread")?.take();
            (thread["id"] == id && thread["turns"].is_array()).then_some(thread)
        })
        .with_context(|| format!("no thread/read of {id}"))
}

/// Device pixels that differ inside the CSS-px `rect` (the window is 2x).
pub fn changed(a: &RgbaImage, b: &RgbaImage, [x, y, w, h]: [f32; 4]) -> usize {
    let (x0, y0) = ((x * 2.) as u32, (y * 2.) as u32);
    let (x1, y1) = (
        (((x + w) * 2.) as u32).min(a.width()),
        (((y + h) * 2.) as u32).min(a.height()),
    );
    (y0..y1)
        .flat_map(|py| (x0..x1).map(move |px| (px, py)))
        .filter(|&(px, py)| a.get_pixel(px, py) != b.get_pixel(px, py))
        .count()
}

/// Checks the settled hover fill. `ms1.rs` checks the intermediate motion frames.
pub fn assert_hover(fx: &mut Fx, at: (f32, f32), rect: [f32; 4], min: usize) -> Result<usize> {
    // Rest the pointer on the titlebar's empty middle first.
    fx.hover((640., 22.))?;
    fx.advance(ui::motion::BASIC)?;
    let before = fx.shot()?;
    fx.hover(at)?;
    fx.advance(ui::motion::BASIC)?;
    let after = fx.shot()?;
    let n = changed(&before, &after, rect);
    ensure!(n >= min, "hover at {at:?} repainted {n} px of {rect:?}");
    Ok(n)
}

/// Compares a captured image with the original reference and fails on a strict pixel mismatch.
pub fn parity_image(screen: &str, theme: &str, port: &RgbaImage) -> Result<String> {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("flows-parity");
    let refs = PathBuf::from(format!("{ROOT}/refs"));
    let m = parity::diff_ref(&refs, &out, screen, theme, port)?;
    let note = format!(
        "[{screen} {} {}x{}: outside={} text={:.3}% {}]",
        m.theme,
        m.width,
        m.height,
        m.counts.diff_px_outside_text,
        m.counts.text_diff_ratio * 100.,
        if m.pass { "PASS" } else { "FAIL" }
    );
    ensure!(m.pass, "{note}");
    Ok(note)
}

/// Runs `reach` in each theme at each ref size of `screen`, then the parity diff.
pub fn parity_flow(screen: &str, reach: fn(&mut Fx) -> Result<()>) -> Result<String> {
    let route = match screen {
        "home" | "composer" | "sidebar" => Route::Home,
        "thread" | "diff" | "terminal" => Route::Thread(SUBTRACT.into()),
        "settings" => Route::Settings,
        "automations" => Route::Scheduled,
        "mobile" => Route::Mobile,
        _ => anyhow::bail!("no Root parity route for {screen}"),
    };
    let panel = match screen {
        "diff" => Some(SidePanel::Changes),
        "terminal" => Some(SidePanel::Terminal),
        _ => None,
    };
    let mut notes = Vec::new();
    let mut failed = false;
    for theme in ["dark", "light"] {
        for size in [(1280., 832.), (1440., 900.)] {
            let result = (|| {
                let mut fx = Fx::with(theme, size, None, |_| {})?;
                reach(&mut fx)?;
                fx.cx.run_until_parked();
                ensure!(
                    fx.route() == route,
                    "route {:?}, expected {route:?}",
                    fx.route()
                );
                ensure!(
                    fx.read(|r, _| r.side_panel()) == panel,
                    "side panel {:?}, expected {panel:?}",
                    fx.read(|r, _| r.side_panel())
                );
                ensure!(
                    fx.read(|r, _| r.command_menu().is_some()) == (screen == "sidebar"),
                    "unexpected command menu state"
                );
                println!(
                    "{screen} {theme} {}x{} route={route:?} panel={panel:?}",
                    size.0, size.1
                );
                fx.parity(screen)
            })();
            match result {
                Ok(note) => notes.push(note),
                Err(err) => {
                    failed = true;
                    notes.push(format!("[{screen} {theme} {}x{}: {err:#}]", size.0, size.1));
                }
            }
        }
    }
    let note = notes.join(" ");
    ensure!(!failed, "{note}");
    Ok(note)
}

// --- Request capture ---

const PROXY_SCRIPT: &str = "CGX_PROXY_SCRIPT";
const PROXY_LOG: &str = "CGX_PROXY_LOG";
const PROXY_BARRIER: &str = "proxy/barrier";

/// `lines` as a replay script file under the target tmp dir.
fn write_script(lines: &[Value]) -> Result<PathBuf> {
    static SCRIPTS: AtomicUsize = AtomicUsize::new(0);
    let script: String = lines.iter().map(|v| format!("{v}\n")).collect();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("flows");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "script-{}-{}.jsonl",
        std::process::id(),
        SCRIPTS.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, script)?;
    Ok(path)
}

/// Drives `f` to completion on the calling thread.
fn block_on<T>(f: impl Future<Output = T>) -> T {
    struct Unpark(thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(value) = f.as_mut().poll(&mut cx) {
            return value;
        }
        thread::park();
    }
}

/// Body of the `proxy` test of a binary that uses [`Fx::captured`]. When the binary was
/// started as the app-server of one, serves it and exits; otherwise returns at once.
///
/// The proxy forwards every client line to [`Bridge::replay`], so the answers are those of
/// [`Fx::open`], and appends each client message (but for the barrier) to the log file.
// ponytail: stands in for a request log in `Bridge` itself (another crate); replace with
// `Bridge::sent()` when one exists and delete this proxy.
pub fn proxy_child() {
    let (Some(script), Some(log)) = (env::var_os(PROXY_SCRIPT), env::var_os(PROXY_LOG)) else {
        return;
    };
    serve_proxy(script.as_ref(), log.as_ref()).expect("replay proxy");
    std::process::exit(0);
}

fn serve_proxy(script: &Path, log: &Path) -> Result<()> {
    let (bridge, events) = Bridge::replay(script)?;
    let bridge = Arc::new(bridge);
    let out = Arc::new(Mutex::new(io::stdout()));
    let send = |out: &Mutex<io::Stdout>, msg: Value| {
        let mut out = out.lock().unwrap();
        writeln!(out, "{msg}").and_then(|_| out.flush()).ok();
    };
    let server_out = out.clone();
    thread::spawn(move || {
        while let Ok(event) = events.recv_blocking() {
            let msg = match event {
                Event::Notification { method, params } => {
                    json!({ "method": method, "params": params })
                }
                Event::Request { id, method, params } => {
                    json!({ "id": id, "method": method, "params": params })
                }
            };
            send(&server_out, msg);
        }
    });
    let mut log = fs::OpenOptions::new().append(true).open(log)?;
    for line in io::stdin().lock().lines() {
        let msg: Value = serde_json::from_str(&line?)?;
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (msg["method"].as_str(), msg.get("id")) {
            (Some(PROXY_BARRIER), Some(id)) => send(&out, json!({ "id": id, "result": {} })),
            (Some(method), id) => {
                let mut logged = json!({ "method": method });
                if !params.is_null() {
                    logged["params"] = params.clone();
                }
                writeln!(log, "{logged}")?;
                let method = method.to_owned();
                match id {
                    Some(id) => {
                        let (bridge, out, id) = (bridge.clone(), out.clone(), id.clone());
                        thread::spawn(move || {
                            let reply = match block_on(bridge.request::<_, Value>(&method, params))
                            {
                                Ok(result) => json!({ "id": id, "result": result }),
                                Err(err) => {
                                    let (code, message) = match err.downcast_ref::<RpcError>() {
                                        Some(rpc) => (rpc.code, rpc.message.clone()),
                                        None => (-32000, err.to_string()),
                                    };
                                    json!({ "id": id, "error": { "code": code, "message": message } })
                                }
                            };
                            send(&out, reply);
                        });
                    }
                    None => bridge.notify(&method, params)?,
                }
            }
            (None, Some(id)) => {
                writeln!(log, "{}", json!({ "reply": id, "result": msg["result"] }))?;
                bridge.respond(id.clone(), msg["result"].clone())?;
            }
            (None, None) => {}
        }
    }
    Ok(())
}
