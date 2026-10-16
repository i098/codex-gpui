//! Slice F3a (toast region, Undo toast after archive): replays the steps of the original's
//! recording `refs/interactions/f3a.json` on the real [`app::Root`] and asserts the same
//! outcome: toasts shown, their size and colours (pixels at the recorded boxes), the lifetime
//! and hover pause, the swipe, and the app-server requests (the replay only answers a request
//! whose params equal the recorded ones; any other gets an error that shows as a toast).
//!
//! The recording has no native copy menu on the port side (the context menu is another
//! slice), so its copy action is replayed through [`app::Root::toast`], which a copy action
//! calls.
//!
//! Needs the Metal headless renderer: macOS only.
#![cfg(target_os = "macos")]

use anyhow::{Context as _, Result, ensure};
use app::{Root, State, ThemeSetting};
use bridge::Bridge;
use gpui::{
    AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px, size,
};
use image::RgbaImage;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{fs, path::Path, thread};
use ui::toast::Toast;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const CWD: &str = "/tmp/cgx-mac/proj";
const TIMEOUT: Duration = Duration::from_secs(30);

type Pt = (f32, f32);

fn num(v: &Value) -> f32 {
    v.as_f64().unwrap_or_default() as f32
}

/// A recorded `bbox` object as `[x, y, w, h]`.
fn rect(v: &Value) -> [f32; 4] {
    [num(&v["x"]), num(&v["y"]), num(&v["w"]), num(&v["h"])]
}

fn center([x, y, w, h]: [f32; 4]) -> Pt {
    (x + w / 2., y + h / 2.)
}

/// `rgb(r, g, b)` or `oklab(...)` computed colours: the recorded `rgb()` ones.
fn rgb(css: &str) -> Option<[u8; 3]> {
    let inner = css.strip_prefix("rgb(")?.strip_suffix(')')?;
    let v: Vec<u8> = inner
        .split(',')
        .filter_map(|c| c.trim().parse().ok())
        .collect();
    v.try_into().ok()
}

/// The first recorded DOM entry after step `after` (`copy-error`, `archive`, ...).
fn dom<'a>(rec: &'a Value, after: &str) -> Result<&'a Value> {
    rec["result"]["dom"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["after"] == after)
        .with_context(|| format!("recording has no {after}"))
}

fn button<'a>(toast: &'a Value, text: &str) -> Result<&'a Value> {
    toast["buttons"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|b| b["text"] == text)
        .with_context(|| format!("recorded toast has no {text} button"))
}

/// Recorded `method` requests as their raw params.
fn requests(rec: &Value, method: &str) -> Vec<Value> {
    rec["result"]["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["method"] == method)
        .filter_map(|r| serde_json::from_str(r["params"].as_str()?).ok())
        .collect()
}

/// The replay answers `method` with `result` to the recorded `params` only. A request with other
/// params falls back to the first unused request of the method: the decoy before it, an error.
fn exchange(method: &str, params: &Value, result: Value) -> Vec<Value> {
    static IDS: AtomicUsize = AtomicUsize::new(90_000);
    let (decoy, good) = (
        IDS.fetch_add(1, Ordering::Relaxed),
        IDS.fetch_add(1, Ordering::Relaxed),
    );
    vec![
        json!({ "dir": "send", "msg": { "id": decoy, "method": method, "params": { "decoy": true } } }),
        json!({ "dir": "recv", "msg": { "id": decoy, "error": { "code": -1, "message": "params differ from the recording" } } }),
        json!({ "dir": "send", "msg": { "id": good, "method": method, "params": params } }),
        json!({ "dir": "recv", "msg": { "id": good, "result": result } }),
    ]
}

fn readback() -> Result<Vec<Value>> {
    let path = format!("{ROOT}/fixtures/readback.jsonl");
    fs::read_to_string(&path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Ok(serde_json::from_str(l)?))
        .collect()
}

/// A Recents thread of readback.jsonl's `thread/list`.
fn listed(lines: &[Value], id: &str) -> Result<Value> {
    lines
        .iter()
        .filter_map(|v| v.pointer("/msg/result/data")?.as_array())
        .flatten()
        .find(|t| t["id"] == id)
        .cloned()
        .with_context(|| format!("no listed thread {id}"))
}

struct Fx {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
}

impl Fx {
    fn open(lines: Vec<Value>) -> Result<Self> {
        let script: String = lines.iter().map(|v| format!("{v}\n")).collect();
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("f3a");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("script-{}.jsonl", std::process::id()));
        fs::write(&path, script)?;
        let (bridge, events) = Bridge::replay(&path)?;
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
                theme: ThemeSetting::Dark,
                ..State::default()
            };
            cx.new(|cx| Root::new(state, bridge, events, CWD.into(), window, cx))
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        let mut fx = Self {
            cx,
            root,
            window: window.into(),
        };
        fx.wait("startup", |fx| fx.root.read_with(&fx.cx, |r, _| r.ready()))?;
        Ok(fx)
    }

    /// Runs the app until `done`, for at most [`TIMEOUT`] of real time.
    fn wait(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) -> Result<()> {
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

    fn titles(&self) -> Vec<String> {
        self.root
            .read_with(&self.cx, |r, cx| r.toasts(cx))
            .iter()
            .map(|t| t.to_string())
            .collect()
    }

    fn recents(&self) -> Vec<String> {
        self.root.read_with(&self.cx, |r, _| r.recents())
    }

    /// Moves the app's clock on and runs what became due.
    fn advance(&mut self, by: Duration) {
        self.cx.advance_clock(by);
        self.cx.run_until_parked();
    }

    fn input(&mut self, event: PlatformInput) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_event(event, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    fn mouse_move(&mut self, (x, y): Pt, pressed: bool) -> Result<()> {
        self.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(x), px(y)),
            pressed_button: pressed.then_some(MouseButton::Left),
            modifiers: Modifiers::default(),
        }))
    }

    fn press(&mut self, (x, y): Pt) -> Result<()> {
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }))
    }

    fn release(&mut self, (x, y): Pt) -> Result<()> {
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
        }))
    }

    fn click(&mut self, at: Pt) -> Result<()> {
        self.mouse_move(at, false)?;
        self.press(at)?;
        self.release(at)
    }

    /// A drag of the pointer from `from` to `to` in `steps` moves, `ms` apart.
    fn drag(&mut self, from: Pt, to: Pt, steps: u32, ms: u64) -> Result<()> {
        self.mouse_move(from, false)?;
        self.press(from)?;
        for i in 1..=steps {
            self.advance(Duration::from_millis(ms));
            let f = i as f32 / steps as f32;
            self.mouse_move((from.0 + (to.0 - from.0) * f, from.1), true)?;
        }
        self.release(to)
    }

    fn shot(&mut self) -> Result<RgbaImage> {
        self.cx.capture_screenshot(self.window)
    }
}

/// The pixel at CSS px `(x, y)`; the window is 2x.
fn pixel(img: &RgbaImage, (x, y): Pt) -> [u8; 3] {
    let p = img.get_pixel((x * 2.) as u32, (y * 2.) as u32);
    [p[0], p[1], p[2]]
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(&a, b)| (i32::from(a) - i32::from(b)).abs() <= tolerance)
}

/// `[left, right]` css px of the fill pixels on row `y`, searched in `lo..hi` (css px): the first
/// one from the left and the last one from the right.
fn span(img: &RgbaImage, y: f32, fill: [u8; 3], lo: f32, hi: f32) -> Option<[f32; 2]> {
    let at = |x: u32| near(pixel(img, (x as f32 / 2., y)), fill, 3);
    let (lo, hi) = ((lo * 2.) as u32, (hi * 2.) as u32);
    let left = (lo..hi).find(|&x| at(x))?;
    let right = (lo..hi).rev().find(|&x| at(x))?;
    Some([left as f32 / 2., (right + 1) as f32 / 2.])
}

/// The toast at recorded `alert` ([x, y, w, h]) is there: its `fill` spans the recorded width
/// less the 1 px border, has the recorded top edge and height. The recording is Chrome on Linux
/// with route1's SF fonts, whose text runs about 2 % wider than CoreText's: the width may
/// differ by that plus 3 css px (the toast is centred, so each edge moves by half).
fn assert_toast(img: &RgbaImage, alert: [f32; 4], fill: [u8; 3], what: &str) -> Result<[f32; 2]> {
    let [x, y, w, h] = alert;
    let [left, right] = span(img, y + h / 2., fill, x - 12., x + w + 12.)
        .with_context(|| format!("{what}: no toast fill near the recorded box"))?;
    let tolerance = 3. + 0.02 * w;
    ensure!(
        (left - (x + 1.)).abs() <= tolerance && (right - (x + w - 1.)).abs() <= tolerance,
        "{what}: fill spans {left}..{right}, recorded {}..{}",
        x + 1.,
        x + w - 1.
    );
    ensure!(
        near(pixel(img, (x + w / 2., y + 2.)), fill, 3),
        "{what}: no fill at the recorded top edge"
    );
    ensure!(
        !near(pixel(img, (x + w / 2., y + h + 3.)), fill, 1),
        "{what}: the toast is taller than recorded"
    );
    Ok([left, right])
}

/// The x offset of a recorded `matrix(a, b, c, d, tx, ty)` transform.
fn translate_x(css: &str) -> Option<f32> {
    css.strip_prefix("matrix(")?
        .split(", ")
        .nth(4)?
        .parse()
        .ok()
}

#[test]
fn toast_region_and_undo() -> Result<()> {
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/f3a.json"))?)?;
    let archives = requests(&rec, "thread/archive");
    let undos = requests(&rec, "thread/unarchive");
    ensure!(
        archives.len() == 3 && undos.len() == 1,
        "recorded requests changed"
    );
    let base = readback()?;
    let mut lines = base.clone();
    for params in &archives {
        lines.extend(exchange("thread/archive", params, json!({})));
    }
    let undone = undos[0]["threadId"].as_str().context("undo thread")?;
    lines.extend(exchange(
        "thread/unarchive",
        &undos[0],
        json!({ "thread": listed(&base, undone)? }),
    ));
    let mut fx = Fx::open(lines)?;
    let recents = fx.recents();
    ensure!(recents.len() == 3, "{recents:?}");

    // SH-136: a copy action shows a danger toast, 48 px under the top, window-centred, no
    // buttons but Close; 5 s later it is gone.
    let copy = &dom(&rec, "copy-error")?["toasts"][0];
    let title = copy["text"].as_str().context("copy toast text")?;
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root, cx| root.toast(Toast::danger(title), cx))
    });
    fx.cx.run_until_parked();
    ensure!(fx.titles() == [title], "{:?}", fx.titles());
    let bg = rgb(copy["alertComputed"]["backgroundColor"]
        .as_str()
        .unwrap_or_default())
    .context("recorded danger fill")?;
    let shot = fx.shot()?;
    assert_toast(&shot, rect(&copy["alert"]), bg, "copy-error")?;
    let life = num(&dom(&rec, "lifetime")?["samples"]["copyErrorMs"]);
    ensure!(
        (life - ui::toast::LIFETIME.as_millis() as f32).abs() < 50.,
        "recorded lifetime {life} ms, port {:?}",
        ui::toast::LIFETIME
    );
    fx.advance(ui::toast::LIFETIME - Duration::from_millis(100));
    ensure!(fx.titles() == [title], "gone early: {:?}", fx.titles());
    fx.advance(Duration::from_millis(200));
    ensure!(fx.titles().is_empty(), "still shown: {:?}", fx.titles());

    // SH-137: the row's `Archive chat` removes the row at once and shows `Archived chat` with
    // View and Undo (the recording's `thread/archive` params, or the replay answers an error).
    let archive_row = &rec["steps"][2]["target"]["bbox"];
    fx.click(center(rect(archive_row)))?;
    fx.wait("archive toast", |fx| fx.titles() == ["Archived chat"])?;
    ensure!(fx.recents() == recents[1..], "{:?}", fx.recents());
    let shown = &dom(&rec, "archive")?["toasts"][0];
    let alert = rect(&shown["alert"]);
    let fill = rgb(shown["alertComputed"]["backgroundColor"]
        .as_str()
        .unwrap_or_default())
    .context("recorded info fill")?;
    let shot = fx.shot()?;
    let [left, _] = assert_toast(&shot, alert, fill, "archive")?;
    let (view, undo) = (button(shown, "View")?, button(shown, "Undo")?);
    let undo_box = rect(&undo["bbox"]);
    ensure!(
        near(
            pixel(&shot, (undo_box[0] + 4., undo_box[1] + 12.)),
            rgb(undo["computed"]["backgroundColor"]
                .as_str()
                .unwrap_or_default())
            .context("undo fill")?,
            2
        ),
        "Undo is not the primary (white) button"
    );
    let view_box = rect(&view["bbox"]);
    // `text/5` over the toast fill: 45 + 0.05 * (255 - 45).
    ensure!(
        near(
            pixel(&shot, (view_box[0] + 4., view_box[1] + 12.)),
            [56, 56, 56],
            2
        ),
        "View is not the secondary button: {:?}",
        pixel(&shot, (view_box[0] + 4., view_box[1] + 12.))
    );

    // The pointer over the toast stops its clock; leaving restarts it with the time left.
    let hover = dom(&rec, "hover-6s")?;
    ensure!(
        hover["toasts"][0]["removed"] == "false",
        "recorded: the toast stays under the pointer"
    );
    fx.advance(Duration::from_millis(500));
    fx.mouse_move((left + 20., alert[1] + 8.), false)?;
    fx.advance(Duration::from_secs(6));
    ensure!(
        fx.titles() == ["Archived chat"],
        "hover did not hold it: {:?}",
        fx.titles()
    );
    fx.mouse_move((900., 500.), false)?;
    fx.advance(Duration::from_millis(4400));
    ensure!(
        fx.titles() == ["Archived chat"],
        "left early: {:?}",
        fx.titles()
    );
    fx.advance(Duration::from_millis(200));
    ensure!(fx.titles().is_empty(), "left late: {:?}", fx.titles());

    // A second archive; a slow 30 px swipe follows the pointer and snaps back, a 72 px one
    // dismisses the toast.
    fx.click(center(rect(archive_row)))?;
    fx.wait("second archive", |fx| fx.titles() == ["Archived chat"])?;
    let held = translate_x(
        dom(&rec, "swipe-30-held")?["transform"]
            .as_str()
            .unwrap_or_default(),
    );
    ensure!(held == Some(30.), "recorded swipe offset {held:?}");
    let grab = (left + 30., alert[1] + 8.);
    fx.mouse_move(grab, false)?;
    fx.press(grab)?;
    for i in 1..=6 {
        fx.advance(Duration::from_millis(100));
        fx.mouse_move((grab.0 + 5. * i as f32, grab.1), true)?;
    }
    let moved = assert_toast(
        &fx.shot()?,
        [alert[0] + 30., alert[1], alert[2], alert[3]],
        fill,
        "swiped 30",
    )?;
    ensure!(
        (moved[0] - left - 30.).abs() <= 1.,
        "toast moved {} px",
        moved[0] - left
    );
    fx.release((grab.0 + 30., grab.1))?;
    ensure!(fx.titles() == ["Archived chat"], "30 px swipe dismissed it");
    assert_toast(&fx.shot()?, alert, fill, "snapped back")?;
    fx.drag(grab, (grab.0 + 72., grab.1), 12, 100)?;
    ensure!(fx.titles().is_empty(), "72 px swipe left {:?}", fx.titles());

    // A third archive, then Undo: `thread/unarchive` with the recorded params restores the row.
    fx.click(center(rect(archive_row)))?;
    fx.wait("third archive", |fx| fx.titles() == ["Archived chat"])?;
    ensure!(fx.recents().is_empty(), "{:?}", fx.recents());
    fx.click(center(undo_box))?;
    fx.wait("undo", |fx| fx.recents().len() == 1)?;
    ensure!(fx.recents() == [undone], "{:?}", fx.recents());
    ensure!(fx.titles().is_empty(), "Undo left {:?}", fx.titles());
    Ok(())
}
