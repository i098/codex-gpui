//! Replay the original Home menu interactions in `refs/interactions/f2a.json`.

use anyhow::{Result, ensure};
use app::{Root, State, ThemeSetting};
use bridge::Bridge;
use gpui::{
    AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px, size,
};
use serde_json::Value;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use ui::composer::Permission;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

const REPLAY_SERVER: &str = r#"
import json, sys
rows = [json.loads(line) for line in open(sys.argv[1])]
replies = {}
for i, row in enumerate(rows):
    message = row['msg']
    if row['dir'] != 'send' or 'method' not in message or 'id' not in message:
        continue
    reply = next((r['msg'] for r in rows[i+1:] if r['dir'] == 'recv' and r['msg'].get('id') == message['id']), None)
    if reply:
        replies.setdefault(message['method'], []).append(reply)
for line in sys.stdin:
    message = json.loads(line)
    with open(sys.argv[2], 'a') as log:
        log.write(line)
    if 'id' in message:
        choices = replies.get(message.get('method'), [])
        reply = choices.pop(0) if choices else {'error': {'code': -32601, 'message': 'Unrecorded request'}}
        print(json.dumps(dict(reply, id=message['id'])), flush=True)
"#;

struct Fx {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
    requests: std::path::PathBuf,
}

impl Fx {
    fn new() -> Result<Self> {
        let requests =
            std::env::temp_dir().join(format!("f2a-requests-{}.jsonl", std::process::id()));
        std::fs::write(&requests, "")?;
        let mut command = std::process::Command::new("python3");
        command
            .args(["-u", "-c", REPLAY_SERVER])
            .arg(format!("{ROOT}/fixtures/readback.jsonl"))
            .arg(&requests);
        let (bridge, events) = Bridge::spawn(command)?;
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
            cx.new(|cx| {
                Root::new(
                    State {
                        theme: ThemeSetting::Dark,
                        ..State::default()
                    },
                    bridge,
                    events,
                    "/tmp/cgx-mac/proj".into(),
                    window,
                    cx,
                )
            })
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        let fx = Self {
            cx,
            root,
            window: window.into(),
            requests,
        };
        let start = Instant::now();
        while !fx.root.read_with(&fx.cx, |r, _| r.ready()) {
            fx.cx.run_until_parked();
            ensure!(
                start.elapsed() < Duration::from_secs(30),
                "startup timed out"
            );
            thread::sleep(Duration::from_millis(5));
        }
        Ok(fx)
    }

    fn input(&mut self, input: PlatformInput) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_event(input, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    fn click(&mut self, x: f32, y: f32) -> Result<()> {
        let position = point(px(x), px(y));
        self.input(PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Modifiers::default(),
        }))?;
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }))?;
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        }))
    }

    fn key(&mut self, key: &str) -> Result<()> {
        let key = Keystroke::parse(key)?;
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_keystroke(key, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }
}

#[test]
fn home_menu_matches_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/f2a.json"))?;
    let steps = rec["steps"].as_array().unwrap();
    let observed = rec["result"]["dom"].as_array().unwrap();
    let before_requests = std::fs::read_to_string(&fx.requests)?;
    let mut unhighlighted = None;
    for (i, step) in steps.iter().enumerate() {
        match step["type"].as_str().unwrap() {
            "click" => {
                let b = &step["target"]["bbox"];
                fx.click(
                    (b["x"].as_f64().unwrap() + b["w"].as_f64().unwrap() / 2.) as f32,
                    (b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() / 2.) as f32,
                )?;
            }
            "key" => fx.key(match step["key"].as_str().unwrap() {
                "ArrowDown" => "down",
                "ArrowUp" => "up",
                "Escape" => "escape",
                "Enter" => "enter",
                "Space" => "space",
                other => anyhow::bail!("unsupported key {other}"),
            })?,
            other => anyhow::bail!("unsupported step {other}"),
        }
        let expected = &observed[i];
        let (state, permission) = fx.cx.update_window(fx.window, |_, window, cx| {
            let composer = fx.root.read(cx).composer(cx);
            let composer = composer.read(cx);
            (composer.menu_state(window), composer.permission())
        })?;
        ensure!(
            state.0 == expected["open"].as_bool().unwrap(),
            "step {i}: open state {state:?}"
        );
        ensure!(
            state.1.map_or(-1, |n| n as i64) == expected["focusedItem"].as_i64().unwrap(),
            "step {i}: item focus {state:?}"
        );
        ensure!(
            state.2 == (expected["active"]["aria"] == "Change permissions"),
            "step {i}: trigger focus {state:?}"
        );
        let label = match permission {
            Permission::Ask => "Ask for approval",
            Permission::ApproveForMe => "Approve for me",
            Permission::FullAccess => "Full access",
            Permission::Custom => "Custom",
        };
        ensure!(
            label == expected["trigger"]["text"].as_str().unwrap(),
            "step {i}: selected {label}"
        );
        let image = fx.cx.capture_screenshot(fx.window)?;
        if i == 0 {
            unhighlighted = Some(image.clone());
        }
        if let Some(item) = state.1 {
            let rgba = expected["active"]["computed"]["backgroundColor"]
                .as_str()
                .unwrap();
            let channels: Vec<f32> = rgba
                .strip_prefix("rgba(")
                .unwrap()
                .trim_end_matches(')')
                .split(',')
                .map(|s| s.trim().parse())
                .collect::<Result<_, _>>()?;
            // The port keeps its existing 320 px menu. Sample the same empty area in each row.
            let scale = image.width() as f32 / 1280.;
            let x = (790. * scale) as u32;
            let y = ([580., 628., 684., 740.][item] * scale) as u32;
            let base = unhighlighted.as_ref().unwrap().get_pixel(x, y);
            let painted = image.get_pixel(x, y);
            for channel in 0..3 {
                let blended =
                    channels[channel] * channels[3] + base[channel] as f32 * (1. - channels[3]);
                ensure!(
                    (painted[channel] as f32 - blended).abs() <= 2.,
                    "step {i}: focus fill {painted:?}, expected {blended}"
                );
            }
        }
        if state.2 {
            let shadow = expected["active"]["computed"]["boxShadow"]
                .as_str()
                .unwrap();
            let color: Vec<u8> = shadow
                .rsplit_once("rgb(")
                .unwrap()
                .1
                .split(')')
                .next()
                .unwrap()
                .split(',')
                .map(|s| s.trim().parse())
                .collect::<Result<_, _>>()?;
            let scale = image.width() as f32 / 1280.;
            let pixel = image.get_pixel((481. * scale) as u32, (790. * scale) as u32);
            ensure!(
                pixel.0[..3] == color,
                "step {i}: trigger ring {pixel:?}, expected {color:?}"
            );
        }
    }
    ensure!(rec["result"]["requests"].as_array().unwrap().is_empty());
    ensure!(
        std::fs::read_to_string(&fx.requests)? == before_requests,
        "menu input sent an app-server request"
    );
    std::fs::remove_file(&fx.requests)?;
    Ok(())
}
