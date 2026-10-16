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
use serde_json::Value;
use std::time::{Duration, Instant};
use std::{fs, sync::Arc, thread};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const EXPLAIN: &str = "01a13de9-35ed-7c72-a36a-68eeb2f4e1ee";
/// Recents rows (`Change the greeting`, `Add a subtract`) in the sidebar, CSS px.
const ROWS: [(f32, f32); 2] = [(196., 305.), (196., 336.)];
const EXPLAIN_ROW: (f32, f32) = (196., 367.);
const THREAD: (f32, f32) = (640., 400.);
const FRAMES: usize = 120;
/// Turns of the Explain thread (6 items each): a long chat, as real histories hold.
const TURNS: usize = 40;

fn main() -> Result<()> {
    let mut b = Bench::open()?;
    let mut rows = Vec::new();
    rows.push(b.run("full redraw", |b| b.update(|w, _| w.refresh()))?);
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

    println!("{:<16} {:>22}", "scenario", "cpu p50 / p95 / max ms");
    for (name, [p50, p95, max]) in rows {
        println!("{name:<16} {p50:>6.2} {p95:>7.2} {max:>7.2}");
    }
    Ok(())
}

type Row = (&'static str, [f64; 3]);

struct Bench {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
}

impl Bench {
    fn open() -> Result<Self> {
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
        // Let the Recents list and every startup read land before measuring.
        thread::sleep(Duration::from_millis(300));
        b.cx.run_until_parked();
        Ok(b)
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
/// older than them), the Explain thread grown to [`TURNS`] turns (copies of its one turn) and
/// its thread reads repeated so every sidebar click gets an answer.
fn script() -> Result<String> {
    let text = fs::read_to_string(format!("{ROOT}/fixtures/readback.jsonl"))?;
    let mut lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for v in &mut lines {
        if let Some(Value::Array(data)) = v.pointer_mut("/msg/result/data")
            && data.first().is_some_and(|t| t.get("sessionId").is_some())
        {
            let recorded = data.clone();
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
    Ok(lines.iter().map(|v| format!("{v}\n")).collect())
}

fn stats(mut d: Vec<Duration>) -> [f64; 3] {
    d.sort();
    let ms = |i: usize| d[i.min(d.len() - 1)].as_secs_f64() * 1000.;
    [ms(d.len() / 2), ms(d.len() * 95 / 100), ms(d.len() - 1)]
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
