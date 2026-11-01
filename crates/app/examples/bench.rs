//! Frame-time benchmark: the real root view on a replayed app-server, headless at 1280x832
//! @2x (dark). The replay is fixtures/readback.jsonl with its Recents list grown to 60 threads
//! and one chat grown to [`TURNS`] turns. Each scenario sends real input events and prints,
//! over its frames, p50 / p95 / max ms of the main-thread work from an input to its frame:
//! dispatch, render, layout, prepaint and paint (shapes rasterization and text shaping
//! included). The 120 Hz frame budget is 8.3 ms.
//!
//! Run on the Mac: `cargo run -p app --release --example bench`. GPU time per frame: run the
//! binary under `xcrun xctrace record --template 'Metal System Trace' --launch -- <bench>`.
//! `BENCH_SHOT=<dir>` saves every 20th frame as `<dir>/<scenario>-<frame>.png`, to compare the
//! pixels of two builds.

use anyhow::{Context as _, Result, ensure};
use app::{Root, Route, State};
use bridge::Bridge;
use gpui::{
    AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, ScrollDelta,
    ScrollWheelEvent, TouchPhase, point, px, size,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use std::{fs, sync::Arc, thread};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const EXPLAIN: &str = "01a13de9-35ed-7c72-a36a-68eeb2f4e1ee";
/// Recents rows (`Change the greeting`, `Add a subtract`) in the sidebar, CSS px.
const ROWS: [(f32, f32); 2] = [(196., 305.), (196., 336.)];
const EXPLAIN_ROW: (f32, f32) = (196., 367.);
const THREAD: (f32, f32) = (640., 400.);
/// The Home composer's text field.
const COMPOSER: (f32, f32) = (808., 737.);
/// Animation triggers per animation scenario; even, so each ends in its starting state.
const TOGGLES: usize = 10;
/// One frame of a 120 Hz display.
const TICK: Duration = Duration::from_nanos(8_333_333);
const FRAMES: usize = 120;
/// Turns of the Explain thread (6 items each): a long chat, as real histories hold.
const TURNS: usize = 40;

fn main() -> Result<()> {
    // Projects in the user's own Codex home would move the sidebar rows: use an empty one.
    let home = std::env::temp_dir().join(format!("cgx-bench-home-{}", std::process::id()));
    fs::create_dir_all(&home)?;
    // SAFETY: no other thread runs yet.
    unsafe { std::env::set_var("CODEX_HOME", &home) };
    let (mut b, startup) = Bench::open()?;
    let mut rows = Vec::new();
    rows.push(b.run("full redraw", |b| b.update(|w, _| w.refresh()))?);
    b.click(COMPOSER)?;
    let mut n = 0;
    rows.push(b.run("key in composer", |b| {
        n += 1;
        b.key(if n % 20 == 0 { " " } else { "a" })
    })?);
    let mut n = 0;
    rows.push(b.run("hover sidebar", |b| {
        n += 1;
        b.input(mouse_move((196., 290. + (n % 6) as f32 * 31.)))
    })?);
    let mut n = 0;
    rows.push(b.run("click sidebar", |b| {
        n += 1;
        let at = ROWS[n % 2];
        Ok(b.input(mouse_move(at))? + b.input(mouse_down(at))? + b.input(mouse_up(at))?)
    })?);
    b.click(EXPLAIN_ROW)?;
    b.wait("thread", |b| {
        b.root.read_with(&b.cx, |r, cx| {
            *r.route() == Route::Thread(EXPLAIN.into())
                && r.thread().is_some_and(|t| t.read(cx).id() == EXPLAIN)
        })
    })?;
    let mut n = 0;
    rows.push(b.run("scroll thread", |b| {
        n += 1;
        // Down the chat for half the frames, then back up.
        let dy = if n <= FRAMES / 2 { -12. } else { 12. };
        b.input(wheel(THREAD, dy))
    })?);
    let mut n = 0;
    rows.push(b.run("key in thread", |b| {
        n += 1;
        b.key(if n % 20 == 0 { " " } else { "a" })
    })?);
    rows.push(b.animate("sidebar in chat", |b| {
        b.update(|w, cx| w.dispatch_action(Box::new(ui::ToggleSidebar), cx))
    })?);
    rows.push(b.animate("changes panel", |b| {
        b.update(|w, cx| w.dispatch_action(Box::new(ui::ToggleChangesPanel), cx))
    })?);

    println!(
        "startup to first ready frame: {:.0} ms",
        startup.as_secs_f64() * 1000.
    );
    println!(
        "{:<16} {:>22} {:>12}",
        "scenario", "cpu p50 / p95 / max ms", "fps @120Hz"
    );
    for (name, [p50, p95, max, fps]) in rows {
        println!("{name:<16} {p50:>6.2} {p95:>7.2} {max:>7.2} {fps:>12.0}");
    }
    drop(b);
    fs::remove_dir_all(&home)?;
    Ok(())
}

type Row = (&'static str, [f64; 4]);

struct Bench {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
}

impl Bench {
    /// Opens the root and returns it with the time from launch to its first ready frame.
    fn open() -> Result<(Self, Duration)> {
        let launched = Instant::now();
        let path = std::env::temp_dir().join(format!("cgx-bench-{}.jsonl", std::process::id()));
        fs::write(&path, script()?)?;
        let (bridge, events) = Bridge::replay(&path)?;
        fs::remove_file(&path)?;
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
            app::init(cx);
        });
        cx.allow_parking();
        let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
            let state = State {
                theme: app::ThemeSetting::Dark,
                ..State::default()
            };
            cx.new(|cx| {
                Root::new(
                    state,
                    bridge,
                    events,
                    "/tmp/cgx-mac/proj".into(),
                    window,
                    cx,
                )
            })
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        let mut b = Self {
            cx,
            root,
            window: window.into(),
        };
        b.wait("startup", |b| b.root.read_with(&b.cx, |r, _| r.ready()))?;
        let startup = launched.elapsed();
        // Let the Recents list and every startup read land before measuring.
        thread::sleep(Duration::from_millis(300));
        b.cx.run_until_parked();
        Ok((b, startup))
    }

    /// Runs `trigger` [`TOGGLES`] times and then, at simulated 120 Hz ticks, every animation
    /// frame it starts until none is requested; the row covers the trigger and those frames.
    fn animate(
        &mut self,
        name: &'static str,
        mut trigger: impl FnMut(&mut Self) -> Result<Duration>,
    ) -> Result<Row> {
        let mut cpu = Vec::new();
        for _ in 0..TOGGLES {
            cpu.push(trigger(self)?);
            self.cx.run_until_parked();
            for _ in 0..240 {
                self.cx.advance_clock(TICK);
                let mut requested = 0;
                let d = self.update(|w, cx| requested = w.simulate_next_frame(cx))?;
                self.cx.run_until_parked();
                if requested == 0 {
                    break;
                }
                cpu.push(d);
            }
        }
        Ok((name, stats(cpu)))
    }

    /// Runs `frame` [`FRAMES`] times; each returns the CPU time of its input and frame.
    fn run(
        &mut self,
        name: &'static str,
        mut frame: impl FnMut(&mut Self) -> Result<Duration>,
    ) -> Result<Row> {
        let mut cpu = Vec::new();
        let shots = std::env::var_os("BENCH_SHOT");
        for i in 0..FRAMES {
            cpu.push(frame(self)?);
            self.cx.run_until_parked();
            if let Some(dir) = shots.as_ref().filter(|_| i % 20 == 19) {
                let path = std::path::Path::new(dir).join(format!("{name}-{i}.png"));
                self.cx.capture_screenshot(self.window)?.save(path)?;
            }
        }
        Ok((name, stats(cpu)))
    }

    fn key(&mut self, key: &str) -> Result<Duration> {
        self.update(|w, cx| {
            w.dispatch_keystroke(
                Keystroke {
                    key: key.into(),
                    ..Default::default()
                },
                cx,
            );
        })
    }

    /// `f` and the frame it invalidates (drawn when the update's effects flush).
    fn update(&mut self, f: impl FnOnce(&mut gpui::Window, &mut gpui::App)) -> Result<Duration> {
        let t = Instant::now();
        self.cx.update_window(self.window, |_, w, cx| f(w, cx))?;
        Ok(t.elapsed())
    }

    fn input(&mut self, event: PlatformInput) -> Result<Duration> {
        self.update(|w, cx| {
            w.dispatch_event(event, cx);
        })
    }

    fn click(&mut self, at: (f32, f32)) -> Result<()> {
        for e in [mouse_move(at), mouse_down(at), mouse_up(at)] {
            self.input(e)?;
        }
        self.cx.run_until_parked();
        Ok(())
    }

    fn wait(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) -> Result<()> {
        let started = Instant::now();
        loop {
            self.cx.run_until_parked();
            if done(self) {
                return Ok(());
            }
            ensure!(
                started.elapsed() < Duration::from_secs(30),
                "timed out waiting for {what}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// readback.jsonl with its `thread/list` grown to 60 threads (copies of the 3 recorded ones,
/// older than them), the Explain thread grown to [`TURNS`] turns (copies of its one turn), its
/// thread reads repeated so every sidebar click gets an answer, and one `thread/resume` reply
/// for each thread the bench opens, in the order it opens them.
fn script() -> Result<String> {
    let text = fs::read_to_string(format!("{ROOT}/fixtures/readback.jsonl"))?;
    let mut lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let mut opened = Vec::new();
    for v in &mut lines {
        if let Some(Value::Array(data)) = v.pointer_mut("/msg/result/data")
            && data.first().is_some_and(|t| t.get("sessionId").is_some())
        {
            let recorded = data.clone();
            opened = recorded.iter().map(|t| t["id"].clone()).collect();
            for i in 0..57 {
                let mut t = recorded[i % recorded.len()].clone();
                t["id"] = format!("00000000-0000-7000-8000-{i:012}").into();
                t["preview"] = format!("{} ({i})", t["preview"].as_str().unwrap_or("")).into();
                for k in ["createdAt", "updatedAt", "recencyAt"] {
                    t[k] = (t[k].as_i64().unwrap_or(0) - 3600 * (i as i64 + 1)).into();
                }
                data.push(t);
            }
        }
    }
    let reads = lines
        .iter()
        .position(|v| v["msg"]["method"] == "thread/read")
        .unwrap_or(lines.len());
    let tail = lines[reads..].to_vec();
    for _ in 0..=FRAMES / 2 {
        lines.extend(tail.iter().cloned());
    }
    let explain = lines
        .iter_mut()
        .find_map(|v| {
            let t = v.pointer_mut("/msg/result/thread")?;
            (t["id"] == EXPLAIN).then_some(t)
        })
        .context("no Explain thread/read")?;
    let turn = explain["turns"][0].clone();
    explain["turns"] = (0..TURNS)
        .map(|i| {
            let mut t = turn.clone();
            t["id"] = format!("{}-{i}", t["id"].as_str().unwrap_or("")).into();
            for item in t["items"].as_array_mut().into_iter().flatten() {
                item["id"] = format!("{}-{i}", item["id"].as_str().unwrap_or("")).into();
            }
            t
        })
        .collect();
    // The run settings of fixtures/sessions/thread-settings.jsonl's resume. A click that leaves
    // a thread before its resume lands resumes it again on the next open.
    let opens = opened
        .iter()
        .flat_map(|t| std::iter::repeat_n(t, FRAMES / 2 + 1));
    for (i, thread) in opens.enumerate() {
        let id = 900_000 + i;
        lines.push(
            json!({"dir": "send", "msg": {"id": id, "method": "thread/resume",
            "params": {"threadId": thread, "excludeTurns": true}}}),
        );
        lines.push(json!({"dir": "recv", "msg": {"id": id, "result": {
            "thread": {"id": thread}, "model": "gpt-6-astra", "approvalPolicy": "never",
            "sandbox": {"type": "workspaceWrite", "writableRoots": [], "networkAccess": false,
                "excludeTmpdirEnvVar": false, "excludeSlashTmp": false},
            "activePermissionProfile": null, "reasoningEffort": "low",
            "collaborationMode": {"mode": "default", "settings": {"model": "gpt-6-astra",
                "reasoning_effort": "low", "developer_instructions": null}}}}}));
    }
    Ok(lines.iter().map(|v| format!("{v}\n")).collect())
}

fn stats(mut d: Vec<Duration>) -> [f64; 4] {
    let fps = fps_at_120hz(&d);
    d.sort();
    let ms = |i: usize| d[i.min(d.len() - 1)].as_secs_f64() * 1000.;
    [
        ms(d.len() / 2),
        ms(d.len() * 95 / 100),
        ms(d.len() - 1),
        fps,
    ]
}

/// Frames per second a 120 Hz frame source delivers when each frame takes its CPU time:
/// a frame that overruns its tick drops the ticks it covers.
fn fps_at_120hz(frames: &[Duration]) -> f64 {
    let ticks: u32 = frames
        .iter()
        .map(|d| (d.as_secs_f64() / TICK.as_secs_f64()).ceil().max(1.) as u32)
        .sum();
    frames.len() as f64 / (ticks as f64 * TICK.as_secs_f64())
}

fn mouse_move((x, y): (f32, f32)) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position: point(px(x), px(y)),
        pressed_button: None,
        modifiers: Modifiers::default(),
    })
}

fn mouse_down((x, y): (f32, f32)) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    })
}

fn mouse_up((x, y): (f32, f32)) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
    })
}

fn wheel((x, y): (f32, f32), dy: f32) -> PlatformInput {
    PlatformInput::ScrollWheel(ScrollWheelEvent {
        position: point(px(x), px(y)),
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    })
}
