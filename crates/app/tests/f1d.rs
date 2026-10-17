mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT, script};
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Focusable as _, HeadlessAppContext,
    InteractiveElement, IntoElement, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PlatformInput, PlatformWindow as _, Render,
    Styled, Window, div, point, px, size,
};
use serde_json::Value;
use std::{fs, ops::Range, path::PathBuf, sync::Arc, time::Duration};

#[test]
fn proxy() {
    common::proxy_child();
}

fn rect(value: &Value) -> Result<[f32; 4]> {
    let number = |key: &str| {
        value[key]
            .as_f64()
            .map(|n| n as f32)
            .with_context(|| format!("missing {key}"))
    };
    Ok([number("x")?, number("y")?, number("w")?, number("h")?])
}

fn step(fx: &mut Fx, input: &Value) -> Result<()> {
    match input["type"].as_str() {
        Some("key") => {
            let prefix = if input["shift"] == true { "shift-" } else { "" };
            let key = input["key"].as_str().context("key")?.to_lowercase();
            let key = Keystroke::parse(&format!("{prefix}{key}"))?;
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_keystroke(key.clone(), cx);
                window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: key }), cx);
            })?;
        }
        kind => {
            let [x, y, w, h] = rect(&input["target"]["bbox"])?;
            let at = (x + w / 2., y + h / 2.);
            match kind {
                Some("click") => fx.click(at)?,
                Some("hover") => fx.hover(at)?,
                _ => anyhow::bail!("unsupported input {input}"),
            }
        }
    }
    fx.cx.run_until_parked();
    fx.advance(Duration::from_millis(
        input["waitMs"].as_u64().unwrap_or(160),
    ))
}

#[test]
fn skip_link_moves_focus_to_content() -> Result<()> {
    replay(0..8)
}

#[test]
fn full_label_tooltip_requires_overflow() -> Result<()> {
    replay(8..15)
}

fn replay(steps: Range<usize>) -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("f1d: requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/f1d.json"))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let shots = std::env::var_os("F1D_SHOTS").map(PathBuf::from);
    if let Some(dir) = &shots {
        fs::create_dir_all(dir)?;
    }
    fx.shot()?;
    let before = fx.sent()?.len();
    let mut home_focus = None;
    let mut skip_focus = None;
    let mut pixel_mismatches = Vec::new();
    for index in steps {
        let input = &recording["steps"][index];
        step(&mut fx, input)?;
        let image = fx.shot()?;
        if let Some(dir) = &shots {
            image.save(dir.join(format!("step-{index}.png")))?;
        }
        let observation = &recording["result"]["dom"][index];
        let composer = fx.composer();
        let expected_skip = observation["skipVisible"]
            .as_bool()
            .context("skip visibility")?;
        let (skip_focused, main_focused) = fx.cx.update_window(fx.window, |_, window, cx| {
            let current = window.focused(cx);
            if index == 0 {
                home_focus = current.clone();
            }
            if expected_skip && skip_focus.is_none() {
                skip_focus = current.clone();
            }
            let composer = composer.focus_handle(cx);
            let main_focused = current.as_ref().is_some_and(|focus| {
                focus != &composer
                    && focus.contains(&composer, window)
                    && home_focus
                        .as_ref()
                        .is_some_and(|home| !focus.contains(home, window))
            });
            (current.is_some() && current == skip_focus, main_focused)
        })?;
        ensure!(
            skip_focused
                == observation["skipVisible"]
                    .as_bool()
                    .context("skip visibility")?,
            "step {index}: skip link focus differs from the original"
        );
        ensure!(
            main_focused
                == observation["mainFocused"]
                    .as_bool()
                    .context("content focus")?,
            "step {index}: content focus differs from the original"
        );
        let scale = image.width() as f32 / 1280.;
        for sample in observation["pixels"].as_array().context("pixels")? {
            let x = sample["x"].as_f64().context("pixel x")? as f32;
            let y = sample["y"].as_f64().context("pixel y")? as f32;
            let got = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            let want: [u8; 3] = serde_json::from_value(sample["rgb"].clone())?;
            if got[..3] != want {
                pixel_mismatches.push(format!("step {index}: ({x}, {y}) {got:?} != {want:?}"));
            }
        }
        if let Some(expected) = observation.get("tooltip") {
            let shown = fx.cx.update(|cx| ui::tooltip::shown(cx).cloned());
            if expected.is_null() {
                ensure!(
                    shown.is_none(),
                    "step {index}: unexpected tooltip {shown:?}"
                );
            } else {
                let shown = shown.with_context(|| format!("step {index}: missing tooltip"))?;
                ensure!(shown.label == expected["label"].as_str().context("tooltip label")?);
                ensure!(
                    shown.shortcut.as_deref() == expected["shortcut"].as_str(),
                    "step {index}: wrong shortcut"
                );
                let got = [
                    shown.bounds.origin.x,
                    shown.bounds.origin.y,
                    shown.bounds.size.width,
                    shown.bounds.size.height,
                ]
                .map(f32::from);
                let want = rect(&expected["bbox"])?;
                ensure!(
                    got.iter().zip(want).all(|(a, b)| (a - b).abs() <= 1.),
                    "step {index}: tooltip bounds {got:?} != {want:?}"
                );
            }
        }
    }
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .skip(before)
        .filter(|r| {
            r["method"].as_str().is_some_and(|m| {
                m != "initialized"
                    && !["/read", "/list", "/get", "/search"]
                        .iter()
                        .any(|s| m.ends_with(s))
            })
        })
        .collect();
    ensure!(serde_json::to_value(writes)? == recording["result"]["requests"]);
    ensure!(fx.route() == app::Route::Home);
    ensure!(
        pixel_mismatches.is_empty(),
        "recorded pixel mismatches:\n{}",
        pixel_mismatches.join("\n")
    );
    println!("f1d: recorded interaction and request checks passed");
    Ok(())
}

struct FocusTriggers;

impl Render for FocusTriggers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = ui::focus::handle("focus-first", 0, window, cx);
        let second = ui::focus::handle("focus-second", 0, window, cx);
        let plain = ui::focus::handle("focus-plain", 0, window, cx);
        let trigger = |id, left| {
            div()
                .id(id)
                .absolute()
                .left(px(left))
                .top(px(100.))
                .w(px(80.))
                .h(px(30.))
        };
        div()
            .size_full()
            .font_family(ui::SANS)
            .bg(gpui::rgb(0x181818))
            .on_key_down(ui::focus::tab)
            .child(
                ui::tooltip::tooltip(
                    trigger("focus-first", 100.),
                    "Focus first",
                    None,
                    ui::tooltip::Side::Top,
                )
                .focus_handle(&first.read(cx).handle),
            )
            .child(
                ui::tooltip::tooltip(
                    trigger("focus-second", 300.),
                    "Focus second",
                    None,
                    ui::tooltip::Side::Top,
                )
                .focus_handle(&second.read(cx).handle),
            )
            .child(trigger("focus-plain", 500.).track_focus(&plain.read(cx).handle))
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
    // TestWindow has no OS window; this changes only its simulated active status.
    cx.test_window(window).activate();
    cx.run_until_parked();
    let shown = |cx: &mut HeadlessAppContext| -> Result<Option<ui::tooltip::Shown>> {
        cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx))?;
        cx.run_until_parked();
        cx.capture_screenshot(window)?;
        Ok(cx.update(|cx| ui::tooltip::shown(cx).cloned()))
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
    let click = |cx: &mut HeadlessAppContext, at| -> Result<()> {
        for event in [
            PlatformInput::MouseMove(MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers: Modifiers::default(),
            }),
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
    cx.run_until_parked();
    click(&mut cx, point(px(540.), px(115.)))?;
    assert!(shown(&mut cx)?.is_none());
    for (key, expected) in [
        ("tab", Some("Focus first")),
        ("escape", None),
        ("tab", Some("Focus second")),
        ("tab", None),
        ("shift-tab", Some("Focus second")),
    ] {
        press(&mut cx, key)?;
        let observed = shown(&mut cx)?;
        let (active, keyboard, focused) = cx.update_window(window, |_, window, cx| {
            (
                window.is_window_active(),
                window.last_input_was_keyboard(),
                window.focused(cx).is_some(),
            )
        })?;
        assert_eq!(
            observed.as_ref().map(|tip| tip.label.as_ref()),
            expected,
            "{key}: active={active}, keyboard={keyboard}, focused={focused}"
        );
        if key == "escape" {
            cx.advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            assert!(shown(&mut cx)?.is_none());
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
    for position in [point(px(700.), px(300.)), at] {
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
    }
    cx.advance_clock(Duration::from_millis(250));
    cx.run_until_parked();
    assert_eq!(
        shown(&mut cx)?.context("hover tooltip")?.label,
        "Focus first"
    );
    assert!(cx.update_window(window, |_, window, cx| window.focused(cx))? == focused);
    let image = cx.capture_screenshot(window)?;
    let scale = image.width() as f32 / 1280.;
    let x = (f32::from(previous.origin.x) + 6.) * scale;
    let y = (f32::from(previous.origin.y) + f32::from(previous.size.height) / 2.) * scale;
    assert_eq!(&image.get_pixel(x as u32, y as u32).0[..3], &[24, 24, 24]);
    press(&mut cx, "escape")?;
    assert!(shown(&mut cx)?.is_none());
    assert!(cx.update_window(window, |_, window, cx| window.focused(cx))? == focused);
    cx.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(shown(&mut cx)?.is_none());
    Ok(())
}
