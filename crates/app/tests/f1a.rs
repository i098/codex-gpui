//! Slice F1a (button chrome a): the shared tooltip on the titlebar, the rail and the sidebar
//! `Search` button. Replays `refs/interactions/f1a.json`, recorded from the original in
//! headless Chrome (route1), on the real [`app::Root`] and asserts the recorded outcome: which
//! tooltip is open (text and shortcut chip), where its pill is, its fill pixel, when it opens
//! and when it closes.
//!
//! The recording is the oracle for macOS only: the port renders through the Metal headless
//! renderer, so this test is skipped elsewhere (as `flows` is).

use anyhow::{Context as _, Result, ensure};
use app::{Root, State, ThemeSetting};
use bridge::Bridge;
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, HeadlessAppContext, InteractiveElement,
    IntoElement, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, PlatformInput, PlatformWindow as _, Render, Styled,
    Window, div, point, px, size,
};
use image::RgbaImage;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fs, thread};
use theme::ActiveTheme as _;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const CWD: &str = "/tmp/cgx-mac/proj";
const TIMEOUT: Duration = Duration::from_secs(30);
/// Radix `delayDuration`; the recorded cold opens (200 ms plus the recorder's latency) must
/// not be faster.
const OPEN_DELAY_MS: u64 = 200;

/// The port's clock, mouse and keys on the replayed app-server (the helpers of `flows.rs`).
struct Fx {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
}

impl Fx {
    fn new() -> Result<Self> {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("f1a-script.jsonl");
        fs::create_dir_all(path.parent().context("tmpdir")?)?;
        fs::copy(format!("{ROOT}/fixtures/readback.jsonl"), &path)?;
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
        let fx = Self {
            cx,
            root,
            window: window.into(),
        };
        let started = Instant::now();
        while !fx.root.read_with(&fx.cx, |r, _| r.ready()) {
            fx.cx.run_until_parked();
            ensure!(started.elapsed() < TIMEOUT, "timed out waiting for startup");
            thread::sleep(Duration::from_millis(5));
        }
        Ok(fx)
    }

    fn input(&mut self, event: PlatformInput) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_event(event, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    fn hover(&mut self, (x, y): (f32, f32)) -> Result<()> {
        self.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(x), px(y)),
            pressed_button: None,
            modifiers: Modifiers::default(),
        }))
    }

    fn click(&mut self, at: (f32, f32)) -> Result<()> {
        self.hover(at)?;
        let position = point(px(at.0), px(at.1));
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }))?;
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
        }))
    }

    fn key(&mut self, key: &str) -> Result<()> {
        let stroke = Keystroke::parse(&key.to_lowercase())?;
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_keystroke(stroke, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    /// Runs the app's clock forward `ms`.
    fn advance(&mut self, ms: u64) {
        self.cx.advance_clock(Duration::from_millis(ms));
        self.cx.run_until_parked();
    }

    /// Draws the window, then reads the open tooltip.
    fn shown(&mut self) -> Result<(Option<ui::tooltip::Shown>, RgbaImage)> {
        let image = self.cx.capture_screenshot(self.window)?;
        Ok((self.cx.update(|cx| ui::tooltip::shown(cx).cloned()), image))
    }

    fn sidebar_open(&self) -> bool {
        self.root
            .read_with(&self.cx, |r, cx| r.shell().read(cx).sidebar_open)
    }
}

fn rect(v: &Value) -> [f32; 4] {
    ["x", "y", "w", "h"].map(|k| v[k].as_f64().unwrap_or_default() as f32)
}

fn center(bbox: &Value) -> (f32, f32) {
    let [x, y, w, h] = rect(bbox);
    (x + w / 2., y + h / 2.)
}

#[test]
fn tooltips_match_the_recording() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("f1a: macOS only, skipped");
        return Ok(());
    }
    let path = format!("{ROOT}/refs/interactions/f1a.json");
    let rec: Value = serde_json::from_slice(&fs::read(&path).context(path.clone())?)?;
    let steps = rec["steps"].as_array().context("steps")?;
    let observed = rec["result"]["dom"].as_array().context("result.dom")?;
    ensure!(
        rec["result"]["openMsCold"]
            .as_array()
            .context("openMsCold")?
            .iter()
            .all(|ms| ms.as_u64().is_some_and(|ms| ms >= OPEN_DELAY_MS)),
        "a recorded cold open is faster than the {OPEN_DELAY_MS} ms the port waits"
    );
    let mut fx = Fx::new()?;
    let mut checked = 0;
    for (i, step) in steps.iter().enumerate() {
        let obs = observed.iter().find(|o| o["afterStep"] == i);
        let wait = step["waitMs"].as_u64().unwrap_or_default();
        let at = center(&step["target"]["bbox"]);
        let what = format!("step {i} ({} {})", step["type"], step["target"]["text"]);
        match step["type"].as_str().context("step type")? {
            "hover" | "click" => {
                let click = step["type"] == "click";
                if click {
                    fx.click(at)?;
                } else {
                    fx.hover(at)?;
                }
                let cold = obs.is_some_and(|o| o["openMs"].is_u64());
                let warm = obs.is_some_and(|o| o["warmOpenMs"].is_u64());
                if warm {
                    ensure!(
                        fx.shown()?.0.is_some(),
                        "{what}: a tooltip was open a moment ago, so the next opens at once"
                    );
                }
                if cold {
                    // Nothing before the delay, the tooltip once it has passed.
                    fx.advance(OPEN_DELAY_MS - 50);
                    ensure!(fx.shown()?.0.is_none(), "{what}: opened before the delay");
                    fx.advance(60);
                    ensure!(fx.shown()?.0.is_some(), "{what}: not open after the delay");
                    fx.advance(wait.saturating_sub(OPEN_DELAY_MS + 10));
                } else {
                    fx.advance(wait);
                }
            }
            "key" => {
                fx.key(step["key"].as_str().context("key")?)?;
                fx.advance(wait);
            }
            other => anyhow::bail!("{what}: unknown step type {other}"),
        }
        let Some(obs) = obs else { continue };
        if let Some(open) = obs["sidebarOpen"].as_bool() {
            ensure!(fx.sidebar_open() == open, "{what}: sidebar open != {open}");
        }
        let (shown, image) = fx.shown()?;
        let tip = &obs["tooltip"];
        if tip.is_null() {
            ensure!(
                shown.is_none(),
                "{what}: expected no tooltip, got {shown:?}"
            );
            continue;
        }
        let shown = shown.with_context(|| format!("{what}: expected a tooltip, none is open"))?;
        ensure!(
            shown.label == tip["label"].as_str().context("label")?,
            "{what}: label {:?} != {}",
            shown.label,
            tip["label"]
        );
        ensure!(
            shown.shortcut.as_deref() == tip["kbd"].as_str(),
            "{what}: shortcut {:?} != {}",
            shown.shortcut,
            tip["kbd"]
        );
        // The pill sits where the original put it (a box's size follows its text widths).
        let [x, y, w, h] = rect(&tip["bbox"]);
        let b = shown.bounds;
        let got = [b.origin.x, b.origin.y, b.size.width, b.size.height].map(f32::from);
        ensure!(
            (got[0] - x).abs() <= 2.
                && (got[1] - y).abs() <= 1.
                && (got[2] - w).abs() <= 3.
                && (got[3] - h).abs() <= 1.,
            "{what}: pill {got:?} != recorded {:?}",
            [x, y, w, h]
        );
        // Its fill (`--color-background-tooltip`) at the recorded box, left of the label.
        let scale = image.width() as f32 / 1280.;
        let (px_x, px_y) = (((x + 6.) * scale) as u32, ((y + h / 2.) * scale) as u32);
        let pixel = image.get_pixel(px_x, px_y).0;
        let want = parse_rgb(tip["computed"]["backgroundColor"].as_str().context("bg")?)?;
        if !(0..3).all(|c| pixel[c].abs_diff(want[c]) <= 1) {
            let png = Path::new(env!("CARGO_TARGET_TMPDIR")).join("f1a-fail.png");
            image.save(&png)?;
            anyhow::bail!(
                "{what}: pixel ({px_x}, {px_y}) {pixel:?} != recorded fill {want:?} (window: {})",
                png.display()
            );
        }
        checked += 1;
    }
    println!(
        "f1a: {} steps replayed, {checked} tooltips matched",
        steps.len()
    );
    ensure!(
        checked >= 10,
        "the recording holds too few tooltips: {checked}"
    );
    Ok(())
}

struct BorderedTrigger(Option<Pixels>);

impl Render for BorderedTrigger {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tip = ui::tooltip::tooltip(
            ui::composer::pill("bordered-tooltip", cx.theme())
                .absolute()
                .left(px(448.25))
                .top(px(776.))
                .w(px(28.)),
            "Add files and more",
            Some("@"),
            ui::tooltip::Side::Top,
        );
        let tip = match self.0 {
            Some(offset) => tip.side_offset(offset),
            None => tip,
        };
        div().size_full().child(tip)
    }
}

#[test]
fn bordered_trigger_offset_matches_native_add_without_changing_the_default() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(proof)?;
    // Native original: 1280x832 @2, trigger y776..804, tooltip y742..772.
    // The pill's 1 px border puts its absolute-positioning edge at y777.
    for (offset, expected_top, name) in [
        (Some(px(5.)), 742., "add-offset"),
        (None, 745., "default-offset"),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
        });
        let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
            cx.new(|_| BorderedTrigger(offset))
        })?;
        let window: AnyWindowHandle = window.into();
        cx.capture_screenshot(window)?;
        cx.update_window(window, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(462.25), px(790.)),
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
        cx.advance_clock(Duration::from_millis(OPEN_DELAY_MS));
        cx.run_until_parked();
        let image = cx.capture_screenshot(window)?;
        image.save(proof.join(format!("f1a-{name}.png")))?;
        let shown = cx
            .update(|cx| ui::tooltip::shown(cx).cloned())
            .context("bordered trigger tooltip")?;
        assert_eq!(image.dimensions(), (2560, 1664));
        assert_eq!(shown.bounds.top(), px(expected_top));
        assert_eq!(shown.bounds.bottom(), px(expected_top + 30.));
        assert_eq!(
            &image.get_pixel(774, (expected_top * 2.) as u32 + 30).0[..3],
            &[45, 45, 45]
        );
    }
    Ok(())
}

struct FocusTriggers;

impl Render for FocusTriggers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(ui::SANS)
            .bg(gpui::rgb(0x181818))
            .on_key_down(ui::focus::tab)
            .children(
                [
                    ("focus-first", 100., "Focus first"),
                    ("focus-second", 300., "Focus second"),
                    ("focus-plain", 500., "Hover only"),
                ]
                .map(|(id, left, label)| {
                    let focus = ui::focus::handle(id, 0, window, cx);
                    let tip = ui::tooltip::tooltip(
                        div()
                            .id(id)
                            .absolute()
                            .left(px(left))
                            .top(px(100.))
                            .w(px(80.))
                            .h(px(30.))
                            .track_focus(&focus.read(cx).handle),
                        label,
                        None,
                        ui::tooltip::Side::Top,
                    );
                    if id == "focus-plain" {
                        tip
                    } else {
                        tip.focus_handle(&focus.read(cx).handle)
                    }
                }),
            )
    }
}

#[test]
fn keyboard_tooltip_ownership_and_dismissal_follow_focus() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
    });
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| cx.new(|_| FocusTriggers))?;
    let window: AnyWindowHandle = window.into();
    // TestWindow has no OS window; change only its simulated active status.
    cx.test_window(window).activate();
    cx.run_until_parked();
    let shown = |cx: &mut HeadlessAppContext| -> Result<Option<ui::tooltip::Shown>> {
        // Render the focus event, then the tooltip frame that its notification requests.
        for _ in 0..2 {
            cx.test_window(window)
                .simulate_frame_request(gpui::RequestFrameOptions {
                    require_presentation: true,
                    ..Default::default()
                });
            cx.run_until_parked();
        }
        let image = cx.capture_screenshot(window)?;
        let tip = cx.update(|cx| ui::tooltip::shown(cx).cloned());
        if let Some(tip) = &tip {
            let scale = image.width() as f32 / 1280.;
            let x = (f32::from(tip.bounds.origin.x) + 6.) * scale;
            let y =
                (f32::from(tip.bounds.origin.y) + f32::from(tip.bounds.size.height) / 2.) * scale;
            assert_eq!(&image.get_pixel(x as u32, y as u32).0[..3], &[45, 45, 45]);
        } else {
            assert!(
                image.pixels().all(|pixel| pixel.0[..3] == [24, 24, 24]),
                "closed tooltip still paints"
            );
        }
        Ok(tip)
    };
    let press = |cx: &mut HeadlessAppContext, key: &str| -> Result<()> {
        let stroke = Keystroke::parse(key)?;
        cx.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(stroke.clone(), cx);
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
        })?;
        cx.run_until_parked();
        Ok(())
    };
    let hover = |cx: &mut HeadlessAppContext, position| -> Result<()> {
        cx.update_window(window, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
        Ok(())
    };
    let click = |cx: &mut HeadlessAppContext, at| -> Result<()> {
        hover(cx, at)?;
        for event in [
            PlatformInput::MouseDown(MouseDownEvent {
                position: at,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            PlatformInput::MouseUp(MouseUpEvent {
                position: at,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
        ] {
            cx.update_window(window, |_, window, cx| {
                window.dispatch_event(event, cx);
            })?;
            cx.run_until_parked();
        }
        Ok(())
    };
    cx.capture_screenshot(window)?;
    click(&mut cx, point(px(540.), px(115.)))?;
    assert!(shown(&mut cx)?.is_none());
    for (key, expected) in [
        ("tab", Some("Focus first")),
        ("escape", None),
        ("tab", Some("Focus second")),
        ("tab", None),
        ("shift-tab", Some("Focus second")),
    ] {
        let before = cx.update_window(window, |_, window, cx| window.focused(cx))?;
        press(&mut cx, key)?;
        assert_eq!(
            shown(&mut cx)?.as_ref().map(|tip| tip.label.as_ref()),
            expected,
            "{key}"
        );
        let after = cx.update_window(window, |_, window, cx| window.focused(cx))?;
        assert!(after.is_some());
        if key == "escape" {
            assert!(after == before, "Escape moved focus");
            cx.advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            assert!(shown(&mut cx)?.is_none());
        } else {
            assert!(after != before, "Tab did not move focus");
        }
    }
    let at = point(px(140.), px(115.));
    click(&mut cx, at)?;
    cx.advance_clock(Duration::from_millis(250));
    cx.run_until_parked();
    assert!(
        shown(&mut cx)?.is_none(),
        "pointer focus reopened the tooltip"
    );

    press(&mut cx, "tab")?;
    let previous = shown(&mut cx)?.context("keyboard tooltip")?.bounds;
    let focused = cx.update_window(window, |_, window, cx| window.focused(cx))?;
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(proof)?;
    cx.capture_screenshot(window)?
        .save(proof.join("f1a-focus-keyboard.png"))?;
    hover(&mut cx, point(px(700.), px(300.)))?;
    hover(&mut cx, at)?;
    assert_eq!(
        shown(&mut cx)?.context("hover tooltip")?.label,
        "Focus first"
    );
    let image = cx.capture_screenshot(window)?;
    image.save(proof.join("f1a-focus-hover-takeover.png"))?;
    let scale = image.width() as f32 / 1280.;
    let x = (f32::from(previous.origin.x) + 6.) * scale;
    let y = (f32::from(previous.origin.y) + f32::from(previous.size.height) / 2.) * scale;
    assert_eq!(&image.get_pixel(x as u32, y as u32).0[..3], &[24, 24, 24]);
    for target in [None, Some(point(px(540.), px(115.)))] {
        if let Some(target) = target {
            hover(&mut cx, target)?;
            cx.advance_clock(Duration::from_millis(OPEN_DELAY_MS));
            cx.run_until_parked();
            assert_eq!(
                shown(&mut cx)?.context("default hover tooltip")?.label,
                "Hover only"
            );
        }
        press(&mut cx, "escape")?;
        assert!(shown(&mut cx)?.is_none());
        assert!(cx.update_window(window, |_, window, cx| window.focused(cx))? == focused);
        cx.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(shown(&mut cx)?.is_none());
    }
    press(&mut cx, "shift-tab")?;
    assert_eq!(
        shown(&mut cx)?
            .context("keyboard tooltip before blur")?
            .label,
        "Focus first"
    );
    let focused = cx.update_window(window, |_, window, cx| window.focused(cx))?;
    let other = cx.open_window(size(px(1.), px(1.)), |_, cx| cx.new(|_| FocusTriggers))?;
    cx.test_window(other.into()).activate();
    cx.run_until_parked();
    assert!(
        shown(&mut cx)?.is_none(),
        "window blur left the tooltip open"
    );
    assert!(cx.update_window(window, |_, window, cx| window.focused(cx))? == focused);
    Ok(())
}

/// `rgb(45, 45, 45)`.
fn parse_rgb(css: &str) -> Result<[u8; 3]> {
    let inner = css
        .strip_prefix("rgb(")
        .and_then(|s| s.strip_suffix(')'))
        .with_context(|| format!("not an rgb() color: {css}"))?;
    let channels = inner
        .split(',')
        .map(|p| p.trim().parse::<u8>())
        .collect::<Result<Vec<_>, _>>()?;
    channels
        .try_into()
        .map_err(|_| anyhow::anyhow!("rgb() needs 3 channels: {css}"))
}
