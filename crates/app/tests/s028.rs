use anyhow::{Context as _, Result, ensure};
use app::{Root, State, ThemeSetting};
use bridge::{Bridge, Event};
use gpui::{
    AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px, size,
};
use serde_json::Value;
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

struct Fx {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
    events: async_channel::Sender<Event>,
}

impl Fx {
    fn new() -> Result<Self> {
        let (bridge, incoming) = Bridge::replay(&Path::new(ROOT).join("fixtures/readback.jsonl"))?;
        let (events, receiver) = async_channel::unbounded();
        let forwarding = events.clone();
        thread::spawn(move || {
            while let Ok(event) = incoming.recv_blocking() {
                if forwarding.send_blocking(event).is_err() {
                    break;
                }
            }
        });
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
                        ..Default::default()
                    },
                    bridge,
                    receiver,
                    "/tmp/cgx-mac/proj".into(),
                    window,
                    cx,
                )
            })
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        let mut fx = Self {
            cx,
            root,
            window: window.into(),
            events,
        };
        fx.wait(|fx| fx.root.read_with(&fx.cx, |root, _| root.ready()))?;
        Ok(fx)
    }

    fn wait(&mut self, done: impl Fn(&Self) -> bool) -> Result<()> {
        let started = Instant::now();
        loop {
            self.cx.run_until_parked();
            if done(self) {
                return Ok(());
            }
            ensure!(
                started.elapsed() < Duration::from_secs(30),
                "S028 state did not settle"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn click(&mut self, bbox: &Value) -> Result<()> {
        let x = bbox["x"].as_f64().context("x")? + bbox["w"].as_f64().context("w")? / 2.;
        let y = bbox["y"].as_f64().context("y")? + bbox["h"].as_f64().context("h")? / 2.;
        let position = point(px(x as f32), px(y as f32));
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    first_mouse: false,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    fn key(&mut self, key: &str) -> Result<()> {
        let stroke = Keystroke::parse(key)?;
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_keystroke(stroke, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    fn files(&self) -> Vec<(String, usize, usize)> {
        self.root.read_with(&self.cx, |root, cx| {
            root.diff_panel().map_or_else(Vec::new, |panel| {
                panel
                    .read(cx)
                    .files
                    .iter()
                    .map(|f| (format!("{}{}", f.dir, f.name), f.added, f.removed))
                    .collect()
            })
        })
    }

    fn search(&self) -> Option<(String, usize)> {
        self.root.read_with(&self.cx, |root, cx| {
            root.diff_panel()?
                .read(cx)
                .search()
                .map(|(query, count)| (query.to_owned(), count))
        })
    }

    fn check_find(&mut self, expected: &Value) -> Result<()> {
        let query = expected["input"]["value"]
            .as_str()
            .context("recorded query")?;
        let marks = expected["marks"].as_array().context("recorded marks")?;
        ensure!(
            self.search() == Some((query.to_owned(), marks.len())),
            "recorded search mismatch: {:?}",
            self.search()
        );
        let image = self.cx.capture_screenshot(self.window)?;
        let bar = &expected["bar"]["bbox"];
        let scale = image.width() / 1280;
        let x = (bar["x"].as_u64().context("bar x")? + bar["w"].as_u64().context("bar width")? / 2)
            as u32;
        let y = bar["y"].as_u64().context("bar y")? as u32 + 3;
        ensure!(
            image.get_pixel(x * scale, y * scale).0[..3] == [20, 20, 20],
            "find bar must use the recorded surface"
        );
        for mark in marks {
            let b = &mark["bbox"];
            let x = b["x"].as_u64().context("mark x")? as u32;
            let y = b["y"].as_u64().context("mark y")? as u32;
            let color = match mark["computed"]["backgroundColor"].as_str() {
                Some("rgb(251, 106, 34)") => [251, 106, 34],
                Some("rgb(255, 210, 64)") => [255, 210, 64],
                other => anyhow::bail!("unknown recorded mark color: {other:?}"),
            };
            let scale = image.width() / 1280;
            let found = (x.saturating_sub(3)..x + 25).any(|x| {
                (y.saturating_sub(3)..y + 20)
                    .any(|y| image.get_pixel(x * scale, y * scale).0[..3] == color)
            });
            ensure!(found, "missing recorded highlight at {x},{y}");
        }
        image.save(std::env::temp_dir().join("cgx-s028-port.png"))?;
        Ok(())
    }
}

#[test]
fn streamed_diff_and_find_replay() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S028 rendering requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s028.json"))?;
    let mut fx = Fx::new()?;
    for (i, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        match step["type"].as_str() {
            Some("click") => {
                if step["target"]["text"] == "View changes" {
                    let reference: Value = serde_json::from_str(include_str!(
                        "../../../refs/thread/dark@1280x832.spec.json"
                    ))?;
                    let rect = &reference["texts"]
                        .as_array()
                        .context("reference texts")?
                        .iter()
                        .find(|text| text["text"] == "View changes")
                        .context("View changes")?["rect"];
                    fx.click(
                        &serde_json::json!({"x":rect[0],"y":rect[1],"w":rect[2],"h":rect[3]}),
                    )?;
                    ensure!(
                        fx.root.read_with(&fx.cx, |root, _| root.side_panel())
                            == Some(app::SidePanel::Changes),
                        "View changes must open the panel"
                    );
                } else {
                    fx.click(&step["target"]["bbox"])?;
                }
                if i == 0 {
                    fx.wait(|fx| !fx.files().is_empty())?;
                }
            }
            Some("bridge") => {
                fx.events.send_blocking(Event::Notification {
                    method: step["method"].as_str().context("method")?.into(),
                    params: step["params"].clone(),
                })?;
                let expected = step["expect"]["text"].as_str().context("file list")?;
                let expected_lines: Vec<_> = expected.lines().collect();
                let path = expected_lines[3];
                let added = expected_lines[4].trim_start_matches('+').parse::<usize>()?;
                let removed = expected_lines[5].trim_start_matches('-').parse::<usize>()?;
                fx.wait(|fx| fx.files() == [(path.to_owned(), added, removed)])?;
            }
            Some("key") => fx.key(step["key"].as_str().context("key")?)?,
            Some("type") => {
                for c in step["text"].as_str().context("text")?.chars() {
                    fx.key(&c.to_string())?;
                }
            }
            other => anyhow::bail!("unknown step: {other:?}"),
        }
        let expected = &step["expect"];
        if expected.get("marks").is_some() {
            fx.check_find(expected)?;
        }
        if let Some(find) = expected.get("find") {
            fx.check_find(find)?;
        }
    }
    fx.check_find(&recording["result"])?;
    let files = fx.files();
    let delta = recording["steps"]
        .as_array()
        .context("steps")?
        .last()
        .context("last delta")?;
    for (field, value) in [("threadId", "another-thread"), ("turnId", "older-turn")] {
        let mut params = delta["params"].clone();
        params[field] = value.into();
        params["diff"] = "".into();
        fx.events.send_blocking(Event::Notification {
            method: "turn/diff/updated".into(),
            params,
        })?;
        fx.cx.run_until_parked();
        ensure!(
            fx.files() == files,
            "an unrelated diff must not replace the current turn"
        );
    }
    fx.key("alt-cmd-b")?;
    fx.key("alt-cmd-b")?;
    ensure!(
        fx.files() == files,
        "reopening must retain the streamed aggregate"
    );
    let panel_click = recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .find(|step| step["target"]["selector"] == "[role=tabpanel]")
        .context("panel click")?;
    fx.click(&panel_click["target"]["bbox"])?;
    fx.key("escape")?;
    ensure!(fx.search().is_none(), "Escape must close find");
    Ok(())
}
