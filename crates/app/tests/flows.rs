//! Flow harness (docs/flows.md): the real [`app::Root`] on a replayed app-server
//! (fixtures/readback.jsonl, plus a recorded session for turns), driven by mouse and key
//! events at the original's element bounds (refs/<screen>/dark@1280x832.spec.json) and by the
//! menu bar's actions. Where a ref exists for the state reached, the parity diff runs and its
//! numbers print; they are reported, not gated (the pixel lanes own them).
//!
//! Runs every flow, then fails if any did. `cargo test -p app --test flows -- <filter>` runs
//! only the flows whose name contains `<filter>`.

use anyhow::{Context as _, Result, ensure};
use app::{Root, Route, SidePanel, State, ThemeSetting};
use bridge::Bridge;
use bridge::types::{ThreadItem, TurnStatus};
use gpui::{
    Action, AnyWindowHandle, App, AppContext as _, Entity, EntityInputHandler as _, Focusable as _,
    HeadlessAppContext, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, Point, point, px, size,
};
use image::RgbaImage;
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{fs, thread};
use ui::Shell;
use ui::composer::Composer;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// The fixtures' project (`thread/start` cwd).
const CWD: &str = "/tmp/cgx-mac/proj";
/// Recents order of readback.jsonl's `thread/list`.
const GREETING: &str = "01a13de9-c48a-7b20-8382-db900881cb99";
const SUBTRACT: &str = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";
const EXPLAIN: &str = "01a13de9-35ed-7c72-a36a-68eeb2f4e1ee";
const GREETING_PROMPT: &str = "Change the greeting in hello.py to \"Hello, Codex\".";
const TIMEOUT: Duration = Duration::from_secs(30);

/// Titlebar and rail buttons (inventory §2.2, §3): 28 px titlebar buttons at y 8, 36 px rail
/// buttons at x 8.
const BACK: (f32, f32) = (102., 22.);
const FORWARD: (f32, f32) = (136., 22.);
const TOGGLE_SIDEBAR: (f32, f32) = (170., 22.);
const RAIL_HOME: (f32, f32) = (26., 70.);
const RAIL_SPACE: (f32, f32) = (26., 114.);
const RAIL_SCHEDULED: (f32, f32) = (26., 158.);
/// Sidebar `Search`.
const SEARCH: (f32, f32) = (314., 70.);
/// The main panel at 1280x832 with the sidebar open.
const MAIN_PANEL: [f32; 4] = [340., 44., 936., 784.];
/// The sidebar panel.
const SIDEBAR_PANEL: [f32; 4] = [52., 44., 288., 784.];

type Flow = fn() -> Result<String>;

const FLOWS: &[(&str, Flow)] = &[
    ("launch_home", launch_home),
    ("parity_home", || parity_flow("home", |_| Ok(()))),
    ("rail_home_from_thread", rail_home_from_thread),
    ("rail_home_reselect_resets", rail_home_reselect_resets),
    ("rail_hover_fill", rail_hover_fill),
    ("resize_three_widths", resize_three_widths),
    ("sidebar_toggle_keys", sidebar_toggle_keys),
    ("sidebar_toggle_titlebar", sidebar_toggle_titlebar),
    ("thread_row_opens_thread", thread_row_opens_thread),
    ("thread_row_hover", thread_row_hover),
    ("cmd_digit_goes_to_chat", cmd_digit_goes_to_chat),
    ("previous_next_chat", previous_next_chat),
    ("new_chat_row", new_chat_row),
    ("sidebar_empty_lists", sidebar_empty_lists),
    ("cmd_n_new_chat", cmd_n_new_chat),
    ("editor_click_focuses", editor_click_focuses),
    ("model_picker_popover", model_picker_popover),
    ("permissions_popover", permissions_popover),
    ("composer_button_hover", composer_button_hover),
    ("model_list_loading", model_list_loading),
    ("submit_enter_starts_thread", submit_enter_starts_thread),
    ("submit_click_send", submit_click_send),
    ("shift_enter_newline", shift_enter_newline),
    ("parity_composer", || {
        parity_flow("composer", |fx| {
            fx.type_text("Refactor src/math.py")?;
            fx.click(fx.at("home", "GPT-6 Astra")?)
        })
    }),
    ("back_forward_titlebar", back_forward_titlebar),
    ("back_forward_keys", back_forward_keys),
    ("parity_thread", || {
        parity_flow("thread", |fx| {
            fx.click(fx.at("home", "Add a subtract")?)?;
            fx.wait_thread(SUBTRACT)
        })
    }),
    ("cmd_comma_opens_settings", cmd_comma_opens_settings),
    ("settings_back", settings_back),
    ("settings_nav_click", settings_nav_click),
    ("settings_nav_hover", settings_nav_hover),
    ("settings_search", settings_search),
    ("parity_settings", || {
        parity_flow("settings", |fx| fx.keys("cmd-,"))
    }),
    ("signed_out_shows_login", signed_out_shows_login),
    ("menu_bar_actions", menu_bar_actions),
    ("cmd_w_closes_window", cmd_w_closes_window),
    ("theme_switch", theme_switch),
    ("rail_scheduled", rail_scheduled),
    ("scheduled_card_hover", scheduled_card_hover),
    ("parity_automations", || {
        parity_flow("automations", |fx| fx.click(RAIL_SCHEDULED))
    }),
    ("profile_menu_settings", profile_menu_settings),
    ("profile_menu_dismiss", profile_menu_dismiss),
    ("profile_menu_mobile", profile_menu_mobile),
    ("mobile_button_hover", mobile_button_hover),
    ("parity_mobile", || parity_flow("mobile", open_mobile)),
    ("command_menu_keys", command_menu_keys),
    ("command_menu_search_click", command_menu_search_click),
    ("command_menu_settings_page", command_menu_settings_page),
    ("sidebar_section_collapse", sidebar_section_collapse),
    ("changes_panel_keys", changes_panel_keys),
    ("view_changes_opens_panel", view_changes_opens_panel),
    ("diff_file_header_toggle", diff_file_header_toggle),
    ("parity_diff", || {
        parity_flow("diff", |fx| {
            open_changes(fx)?;
            // The reference shows the file collapsed.
            fx.click((panel_left(fx.size.0) + 100., 111.))
        })
    }),
    ("background_terminal_tab", background_terminal_tab),
    ("parity_terminal", || parity_flow("terminal", open_terminal)),
    ("appearance_theme_control", appearance_theme_control),
    ("settings_select_menu", settings_select_menu),
    ("parity_sidebar", || {
        parity_flow("sidebar", |fx| {
            fx.click(SEARCH)?;
            fx.keys("m a t h")
        })
    }),
];

fn main() {
    // The flows render through the Metal headless renderer and the CoreText text system;
    // other platforms (Linux CI) have neither.
    if !cfg!(target_os = "macos") {
        println!("flows: macOS only, skipped");
        return;
    }
    let filter = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let flows: Vec<_> = FLOWS
        .iter()
        .filter(|(name, _)| filter.as_deref().is_none_or(|f| name.contains(f)))
        .collect();
    let mut failed = 0;
    for (name, flow) in &flows {
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(flow))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("panicked")));
        let secs = started.elapsed().as_secs_f32();
        match result {
            Ok(note) => println!("PASS {name} ({secs:.1} s) {note}"),
            Err(err) => {
                failed += 1;
                println!("FAIL {name} ({secs:.1} s): {err:#}");
            }
        }
    }
    println!(
        "{} flows, {} passed, {failed} failed",
        flows.len(),
        flows.len() - failed
    );
    if failed > 0 {
        std::process::exit(1);
    }
}

// --- Harness ---

/// The app on a replayed app-server in a headless window.
struct Fx {
    cx: HeadlessAppContext,
    root: Entity<Root>,
    window: AnyWindowHandle,
    theme: &'static str,
    /// Window size in CSS px.
    size: (f32, f32),
}

impl Fx {
    /// Dark, 1280x832, readback only.
    fn new() -> Result<Self> {
        Self::with("dark", (1280., 832.), None, |_| {})
    }

    /// `session` (under fixtures/) is appended to readback.jsonl; `edit` rewrites each
    /// recorded line first.
    fn with(
        theme: &'static str,
        (w, h): (f32, f32),
        session: Option<&str>,
        edit: fn(&mut Value),
    ) -> Result<Self> {
        static SCRIPTS: AtomicUsize = AtomicUsize::new(0);
        let mut script = String::new();
        for file in std::iter::once("readback.jsonl").chain(session) {
            let path = format!("{ROOT}/fixtures/{file}");
            let text = fs::read_to_string(&path).with_context(|| path.clone())?;
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let mut v: Value = serde_json::from_str(line)?;
                edit(&mut v);
                script.push_str(&format!("{v}\n"));
            }
        }
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("flows");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "script-{}-{}.jsonl",
            std::process::id(),
            SCRIPTS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, script)?;
        let (bridge, events) = Bridge::replay(&path)?;

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
            cx.new(|cx| Root::new(state, bridge, events, CWD.into(), window, cx))
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        let mut fx = Self {
            cx,
            root,
            window: window.into(),
            theme,
            size: (w, h),
        };
        fx.wait("startup", |fx| fx.read(|r, _| r.ready()))?;
        Ok(fx)
    }

    fn read<R>(&self, f: impl FnOnce(&Root, &App) -> R) -> R {
        self.root.read_with(&self.cx, f)
    }

    fn route(&self) -> Route {
        self.read(|r, _| r.route().clone())
    }

    fn shell<R>(&self, f: impl FnOnce(&Shell) -> R) -> R {
        self.read(|r, cx| f(r.shell().read(cx)))
    }

    fn sidebar_open(&self) -> bool {
        self.shell(|s| s.sidebar_open)
    }

    fn composer(&self) -> Entity<Composer> {
        self.read(|r, cx| r.composer(cx))
    }

    fn composer_text(&mut self) -> Result<String> {
        let composer = self.composer();
        self.cx.update_window(self.window, |_, window, cx| {
            composer
                .update(cx, |c, cx| {
                    c.text_for_range(0..usize::MAX, &mut None, window, cx)
                })
                .unwrap_or_default()
        })
    }

    fn composer_focused(&mut self) -> Result<bool> {
        let composer = self.composer();
        self.cx.update_window(self.window, |_, window, cx| {
            composer.focus_handle(cx).is_focused(window)
        })
    }

    fn thread_id(&self) -> Option<String> {
        self.read(|r, cx| Some(r.thread()?.read(cx).id().to_owned()))
    }

    /// Runs the app until `done`, polling the replay's I/O, for at most [`TIMEOUT`].
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

    /// The route shows thread `id`, read with `thread/read`.
    fn wait_thread(&mut self, id: &str) -> Result<()> {
        self.wait(&format!("thread {id}"), |fx| {
            fx.route() == Route::Thread(id.into()) && fx.thread_id().as_deref() == Some(id)
        })
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
        let position: Point<_> = point(px(at.0), px(at.1));
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

    /// Space-separated keystrokes, e.g. `"cmd-shift-["`.
    fn keys(&mut self, keys: &str) -> Result<()> {
        for key in keys.split(' ') {
            let stroke = Keystroke::parse(key)?;
            self.cx.update_window(self.window, |_, window, cx| {
                window.dispatch_keystroke(stroke, cx);
            })?;
            self.cx.run_until_parked();
        }
        Ok(())
    }

    fn type_text(&mut self, s: &str) -> Result<()> {
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
    fn menu(&mut self, menu: &str, item: &str) -> Result<()> {
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

    fn action(&mut self, action: Box<dyn Action>) -> Result<()> {
        self.cx.update_window(self.window, |_, window, cx| {
            window.dispatch_action(action, cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    fn shot(&mut self) -> Result<RgbaImage> {
        self.cx.capture_screenshot(self.window)
    }

    /// Center of the original's element: a node with this `aria-label`, else the first text
    /// starting with `key`, in refs/<screen>/dark@1280x832.spec.json.
    fn at(&self, screen: &str, key: &str) -> Result<(f32, f32)> {
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
    fn parity(&mut self, screen: &str) -> Result<String> {
        let port = self.shot()?;
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("flows-parity");
        let refs = PathBuf::from(format!("{ROOT}/refs"));
        let m = parity::diff_ref(&refs, &out, screen, self.theme, &port)?;
        Ok(format!(
            "[{screen} {} {}x{}: outside={} text={:.3}% {}]",
            m.theme,
            m.width,
            m.height,
            m.counts.diff_px_outside_text,
            m.counts.text_diff_ratio * 100.,
            if m.pass { "PASS" } else { "FAIL" }
        ))
    }
}

/// Device pixels that differ inside the CSS-px `rect` (the window is 2x).
fn changed(a: &RgbaImage, b: &RgbaImage, [x, y, w, h]: [f32; 4]) -> usize {
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

/// Hovering `at` repaints at least `min` device px of `rect`.
fn assert_hover(fx: &mut Fx, at: (f32, f32), rect: [f32; 4], min: usize) -> Result<usize> {
    // Rest the pointer on the titlebar's empty middle first.
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.hover(at)?;
    let after = fx.shot()?;
    let n = changed(&before, &after, rect);
    ensure!(n >= min, "hover at {at:?} repainted {n} px of {rect:?}");
    Ok(n)
}

/// Runs `reach` in each theme at each ref size of `screen`, then the parity diff.
fn parity_flow(screen: &str, reach: fn(&mut Fx) -> Result<()>) -> Result<String> {
    let mut notes = Vec::new();
    for theme in ["dark", "light"] {
        for size in [(1280., 832.), (1440., 900.)] {
            let mut fx = Fx::with(theme, size, None, |_| {})?;
            reach(&mut fx)?;
            fx.cx.run_until_parked();
            notes.push(fx.parity(screen)?);
        }
    }
    Ok(notes.join(" "))
}

// --- Shell, rail ---

fn launch_home() -> Result<String> {
    let mut fx = Fx::new()?;
    ensure!(fx.route() == Route::Home);
    ensure!(fx.shell(|s| s.selected_rail) == Some("home"));
    ensure!(fx.sidebar_open());
    ensure!(fx.shell(|s| !s.can_go_back && !s.can_go_forward));
    ensure!(fx.composer_focused()?, "Home composer not focused");
    let recents = fx.read(|r, _| r.recents());
    ensure!(
        recents == [GREETING, SUBTRACT, EXPLAIN],
        "recents {recents:?}"
    );
    Ok(String::new())
}

fn rail_home_from_thread() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(fx.shell(|s| s.selected_rail).is_none());
    fx.click(RAIL_HOME)?;
    ensure!(fx.route() == Route::Home, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail) == Some("home"));
    ensure!(fx.composer_focused()?);
    Ok(String::new())
}

fn rail_home_reselect_resets() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.type_text("draft")?;
    ensure!(fx.composer_text()? == "draft");
    fx.click(RAIL_HOME)?;
    ensure!(fx.route() == Route::Home);
    let text = fx.composer_text()?;
    ensure!(text.is_empty(), "draft kept: {text:?}");
    Ok(String::new())
}

fn rail_hover_fill() -> Result<String> {
    let mut fx = Fx::new()?;
    let n = assert_hover(&mut fx, RAIL_SPACE, [8., 96., 36., 36.], 100)?;
    Ok(format!("{n} px"))
}

/// 1280 (reference), 767 (below the 768 px sidebar breakpoint), 600 (below 40rem).
fn resize_three_widths() -> Result<String> {
    for w in [1280., 767., 600.] {
        let mut fx = Fx::with("dark", (w, 832.), None, |_| {})?;
        let wide = w >= 768.;
        ensure!(fx.sidebar_open() == wide, "{w} px: sidebar_open {}", !wide);
        // The rail keeps its 52 px column: rail Home still answers.
        fx.type_text("draft")?;
        fx.click(RAIL_HOME)?;
        ensure!(fx.composer_text()?.is_empty(), "{w} px: rail Home missed");
        // The sidebar still opens on demand below the breakpoint.
        fx.keys("cmd-b")?;
        ensure!(fx.sidebar_open() != wide, "{w} px: cmd-b did not toggle");
    }
    Ok(String::new())
}

// --- Sidebar ---

fn sidebar_toggle_keys() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-shift-s")?;
    ensure!(!fx.sidebar_open(), "cmd-shift-s did not hide");
    fx.keys("cmd-b")?;
    ensure!(fx.sidebar_open(), "cmd-b did not show");
    Ok(String::new())
}

fn sidebar_toggle_titlebar() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(TOGGLE_SIDEBAR)?;
    ensure!(!fx.sidebar_open(), "titlebar toggle did not hide");
    fx.click(TOGGLE_SIDEBAR)?;
    ensure!(fx.sidebar_open(), "titlebar toggle did not show");
    Ok(String::new())
}

fn thread_row_opens_thread() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(fx.shell(|s| s.can_go_back));
    Ok(String::new())
}

fn thread_row_hover() -> Result<String> {
    let mut fx = Fx::new()?;
    let at = fx.at("home", "Change the greeting")?;
    let n = assert_hover(&mut fx, at, [60., at.1 - 15., 272., 30.], 100)?;
    Ok(format!("{n} px"))
}

fn cmd_digit_goes_to_chat() -> Result<String> {
    let mut fx = Fx::new()?;
    for (key, id) in [("cmd-2", SUBTRACT), ("cmd-3", EXPLAIN), ("cmd-1", GREETING)] {
        fx.keys(key)?;
        fx.wait_thread(id)?;
    }
    fx.keys("cmd-9")?;
    ensure!(fx.route() == Route::Thread(GREETING.into()), "cmd-9 moved");
    Ok(String::new())
}

fn previous_next_chat() -> Result<String> {
    let mut fx = Fx::new()?;
    for id in [GREETING, SUBTRACT, EXPLAIN] {
        fx.keys("cmd-shift-]")?;
        fx.wait_thread(id)?;
    }
    fx.keys("cmd-shift-]")?;
    ensure!(
        fx.route() == Route::Thread(EXPLAIN.into()),
        "moved past the last row"
    );
    // The replay answers each `thread/read` once, so only the route is checked from here.
    fx.keys("cmd-shift-[")?;
    ensure!(
        fx.route() == Route::Thread(SUBTRACT.into()),
        "{:?}",
        fx.route()
    );
    fx.keys("cmd-alt-left")?;
    ensure!(
        fx.route() == Route::Thread(GREETING.into()),
        "{:?}",
        fx.route()
    );
    fx.keys("cmd-alt-left")?;
    ensure!(
        fx.route() == Route::Thread(GREETING.into()),
        "moved before the first row"
    );
    Ok(String::new())
}

fn new_chat_row() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.type_text("follow-up")?;
    fx.click(fx.at("home", "New chat")?)?;
    ensure!(fx.route() == Route::Home);
    ensure!(fx.composer_focused()?);
    ensure!(fx.composer_text()?.is_empty());
    Ok(String::new())
}

fn sidebar_empty_lists() -> Result<String> {
    let mut fx = Fx::with("dark", (1280., 832.), None, |v| {
        if v["dir"] == "recv" && v["msg"]["result"]["data"].is_array() {
            // Only the `thread/list` answer has `data` of threads with a `preview`.
            if v["msg"]["result"]["data"][0].get("preview").is_some() {
                v["msg"]["result"]["data"] = json!([]);
            }
        }
    })?;
    ensure!(fx.read(|r, _| r.recents().is_empty()), "recents not empty");
    fx.keys("cmd-1")?;
    ensure!(fx.route() == Route::Home);
    Ok(String::new())
}

// --- Home ---

fn cmd_n_new_chat() -> Result<String> {
    let mut fx = Fx::new()?;
    for key in ["cmd-n", "cmd-shift-o"] {
        fx.keys("cmd-1")?;
        if fx.thread_id().is_none() {
            fx.wait_thread(GREETING)?;
        }
        fx.type_text("x")?;
        fx.keys(key)?;
        ensure!(fx.route() == Route::Home, "{key}: route {:?}", fx.route());
        ensure!(fx.composer_focused()?, "{key}: composer not focused");
        ensure!(fx.composer_text()?.is_empty(), "{key}: draft kept");
    }
    Ok(String::new())
}

fn editor_click_focuses() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.cx
        .update_window(fx.window, |_, window, cx| window.blur(cx))?;
    fx.cx.run_until_parked();
    ensure!(!fx.composer_focused()?);
    fx.click(fx.at("home", "Do anything")?)?;
    ensure!(fx.composer_focused()?, "click did not focus the editor");
    Ok(String::new())
}

/// Clicking `target` opens a menu repainting `region`; Escape closes it.
fn popover(target: &str, region: [f32; 4]) -> Result<String> {
    let mut fx = Fx::new()?;
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.click(fx.at("home", target)?)?;
    fx.hover((640., 22.))?;
    let open = fx.shot()?;
    let n = changed(&before, &open, region);
    ensure!(n > 2000, "{target}: menu repainted only {n} px");
    fx.keys("escape")?;
    let closed = fx.shot()?;
    let left = changed(&before, &closed, region);
    ensure!(left < 50, "{target}: Escape left {left} px");
    Ok(format!("{n} px"))
}

fn model_picker_popover() -> Result<String> {
    popover("GPT-6 Astra", [800., 300., 476., 460.])
}

fn permissions_popover() -> Result<String> {
    popover("Custom", [400., 300., 500., 460.])
}

/// With no `model/list` models the picker reads `Loading model…` (composer.rs): its pill
/// repaints against the loaded `GPT-6 Astra Light`.
fn model_list_loading() -> Result<String> {
    let pill = [940., 770., 200., 40.];
    let mut loaded = Fx::new()?;
    loaded.hover((640., 22.))?;
    let a = loaded.shot()?;
    let mut empty = Fx::with("dark", (1280., 832.), None, |v| {
        let data = &v["msg"]["result"]["data"];
        if v["dir"] == "recv" && data[0].get("defaultReasoningEffort").is_some() {
            v["msg"]["result"]["data"] = json!([]);
        }
    })?;
    empty.hover((640., 22.))?;
    let b = empty.shot()?;
    let n = changed(&a, &b, pill);
    ensure!(n > 100, "the model pill repainted {n} px");
    Ok(format!("{n} px"))
}

fn composer_button_hover() -> Result<String> {
    let mut fx = Fx::new()?;
    let at = fx.at("home", "Dictate")?;
    let n = assert_hover(&mut fx, at, [at.0 - 14., at.1 - 14., 28., 28.], 50)?;
    Ok(format!("{n} px"))
}

/// `thread/start` then `turn/start` from the Home composer; the route follows the new thread
/// and the replayed turn streams into it.
fn submit(send: fn(&mut Fx) -> Result<()>) -> Result<String> {
    let mut fx = Fx::with(
        "dark",
        (1280., 832.),
        Some("sessions/greeting.jsonl"),
        |_| {},
    )?;
    fx.type_text(GREETING_PROMPT)?;
    send(&mut fx)?;
    fx.wait_thread(GREETING)?;
    fx.wait("turn/completed", |fx| {
        fx.read(|r, cx| {
            r.thread()
                .and_then(|v| v.read(cx).turns().last().cloned())
                .is_some_and(|t| t.status == TurnStatus::Completed)
        })
    })?;
    let agent = fx.read(|r, cx| {
        let view = r.thread().expect("thread").read(cx);
        view.turns()
            .iter()
            .flat_map(|t| &t.items)
            .filter_map(|i| match i {
                ThreadItem::AgentMessage { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<String>()
    });
    ensure!(!agent.is_empty(), "no agent message streamed");
    ensure!(fx.read(|r, _| r.recents().first().map(String::as_str) == Some(GREETING)));
    // Sending cleared the Home draft (Back does not touch it).
    fx.keys("cmd-[")?;
    ensure!(fx.route() == Route::Home, "cmd-[: {:?}", fx.route());
    let draft = fx.composer_text()?;
    ensure!(draft.is_empty(), "Home draft kept after send: {draft:?}");
    Ok(format!(
        "agent: {:?}",
        agent.chars().take(40).collect::<String>()
    ))
}

fn submit_enter_starts_thread() -> Result<String> {
    submit(|fx| fx.keys("enter"))
}

fn submit_click_send() -> Result<String> {
    submit(|fx| {
        let at = fx.at("home", "Start new voice chat")?;
        fx.click(at)
    })
}

fn shift_enter_newline() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.type_text("a")?;
    fx.keys("shift-enter")?;
    fx.type_text("b")?;
    let home = fx.composer_text()?;
    ensure!(home == "a\nb", "Home composer: {home:?}");
    ensure!(fx.route() == Route::Home, "shift-enter sent");
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(fx.composer_focused()?, "thread composer not focused");
    fx.type_text("c")?;
    fx.keys("shift-enter")?;
    fx.type_text("d")?;
    let thread = fx.composer_text()?;
    ensure!(thread == "c\nd", "thread composer: {thread:?}");
    Ok(String::new())
}

// --- Thread ---

fn back_forward_titlebar() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    ensure!(fx.shell(|s| s.can_go_forward && !s.can_go_back));
    fx.click(FORWARD)?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(fx.shell(|s| s.can_go_back && !s.can_go_forward));
    Ok(String::new())
}

fn back_forward_keys() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-,")?;
    ensure!(fx.route() == Route::Settings);
    fx.keys("cmd-[ cmd-[")?;
    ensure!(fx.route() == Route::Home, "cmd-[ x2: {:?}", fx.route());
    fx.keys("cmd-]")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-]")?;
    ensure!(fx.route() == Route::Settings, "cmd-] x2: {:?}", fx.route());
    Ok(String::new())
}

// --- Settings, login ---

fn cmd_comma_opens_settings() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    ensure!(fx.route() == Route::Settings);
    ensure!(fx.shell(|s| s.selected_rail.is_none() && !s.chat_chrome && s.can_go_back));
    Ok(String::new())
}

fn settings_back() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    ensure!(fx.composer_focused()?);
    Ok(String::new())
}

fn settings_nav_click() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.click(fx.at("settings", "Notifications")?)?;
    fx.hover((640., 22.))?;
    let after = fx.shot()?;
    let n = changed(&before, &after, MAIN_PANEL);
    ensure!(n > 1000, "the page repainted {n} px");
    Ok(format!("{n} px"))
}

fn settings_nav_hover() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    let at = fx.at("settings", "Import")?;
    let n = assert_hover(&mut fx, at, [60., at.1 - 15., 272., 30.], 100)?;
    Ok(format!("{n} px"))
}

fn settings_search() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.click(fx.at("settings", "Search settings")?)?;
    fx.type_text("zzz")?;
    let after = fx.shot()?;
    // The nav list empties down to `No results found`.
    let n = changed(&before, &after, [52., 140., 288., 680.]);
    ensure!(n > 5000, "the nav repainted {n} px");
    fx.keys("escape")?;
    let cleared = fx.shot()?;
    let left = changed(&before, &cleared, [52., 140., 288., 680.]);
    ensure!(left < 50, "Escape left {left} px");
    Ok(format!("{n} px"))
}

fn signed_out_shows_login() -> Result<String> {
    let fx = Fx::with("dark", (1280., 832.), None, |v| {
        if v["dir"] == "recv" && v["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            v["msg"]["result"] = json!({ "account": null, "requiresOpenaiAuth": true });
        }
    })?;
    ensure!(fx.read(|r, _| r.signed_out()), "no sign-in view");
    Ok(String::new())
}

// --- Menu bar, window, theme ---

fn menu_bar_actions() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.menu("View", "Next Chat")?;
    fx.wait_thread(GREETING)?;
    fx.menu("View", "Next Chat")?;
    fx.wait_thread(SUBTRACT)?;
    fx.menu("View", "Previous Chat")?;
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    fx.menu("Codex", "Settings…")?;
    ensure!(fx.route() == Route::Settings);
    fx.menu("View", "Back")?;
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    fx.menu("View", "Forward")?;
    ensure!(fx.route() == Route::Settings);
    fx.menu("File", "New Chat")?;
    ensure!(fx.route() == Route::Home);
    fx.menu("View", "Toggle Sidebar")?;
    ensure!(!fx.sidebar_open());
    Ok(String::new())
}

fn cmd_w_closes_window() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-w")?;
    let open = fx.cx.update(|cx| cx.windows().len());
    ensure!(open == 0, "{open} windows open");
    Ok(String::new())
}

/// The Appearance setting applies at once: forced Light and Dark, then System (the headless
/// window reports a light system appearance).
fn theme_switch() -> Result<String> {
    use theme::{Appearance, Theme};
    let mut fx = Fx::new()?;
    let mut notes = Vec::new();
    for (setting, want) in [
        (ThemeSetting::Light, Appearance::Light),
        (ThemeSetting::Dark, Appearance::Dark),
        (ThemeSetting::System, Appearance::Light),
    ] {
        fx.hover((640., 22.))?;
        let before = fx.shot()?;
        let root = fx.root.clone();
        fx.cx.update_window(fx.window, |_, window, cx| {
            root.update(cx, |r, cx| r.set_theme(setting, window, cx))
        })?;
        fx.cx.run_until_parked();
        let got = fx.cx.update(|cx| cx.global::<Theme>().appearance);
        ensure!(got == want, "{setting:?}: theme {got:?}");
        let n = changed(&before, &fx.shot()?, [0., 0., 1280., 832.]);
        notes.push(format!("{setting:?}: {n} px"));
    }
    Ok(notes.join(", "))
}

// --- Rail Scheduled, profile menu, Mobile ---

fn rail_scheduled() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(RAIL_SCHEDULED)?;
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("scheduled") && s.can_go_back && !s.new_tab));
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(String::new())
}

fn scheduled_card_hover() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(RAIL_SCHEDULED)?;
    // The first suggestion card (the app shows the landing examples in the same grid).
    let at = fx.at("automations", "AP Lang practice")?;
    let n = assert_hover(&mut fx, at, MAIN_PANEL, 1000)?;
    Ok(format!("{n} px"))
}

/// The profile avatar, at the rail's bottom.
fn avatar(fx: &Fx) -> (f32, f32) {
    (26., fx.size.1 - 26.)
}

/// Profile menu row `n`: `Settings`, then `Set up mobile` under the `Help` heading. The menu
/// sits 48 px above the window's bottom edge.
fn profile_item(fx: &Fx, n: usize) -> (f32, f32) {
    (118., fx.size.1 - [128., 68.][n])
}

fn profile_menu_settings() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(avatar(&fx))?;
    ensure!(
        fx.shell(|s| s.profile_menu),
        "the avatar did not open the menu"
    );
    fx.click(profile_item(&fx, 0))?;
    ensure!(fx.route() == Route::Settings, "route {:?}", fx.route());
    // The Settings shell has the avatar too; a second click closes its menu.
    fx.click(avatar(&fx))?;
    ensure!(
        fx.shell(|s| s.profile_menu),
        "no menu on the Settings route"
    );
    fx.click(avatar(&fx))?;
    ensure!(
        fx.shell(|s| !s.profile_menu),
        "a second avatar click left it open"
    );
    Ok(String::new())
}

fn profile_menu_dismiss() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(avatar(&fx))?;
    ensure!(fx.shell(|s| s.profile_menu));
    fx.click((800., 400.))?;
    ensure!(
        fx.shell(|s| !s.profile_menu),
        "a click outside left it open"
    );
    ensure!(fx.route() == Route::Home);
    Ok(String::new())
}

/// Profile menu ▸ Help ▸ `Set up mobile`.
fn open_mobile(fx: &mut Fx) -> Result<()> {
    fx.click(avatar(fx))?;
    fx.click(profile_item(fx, 1))
}

fn profile_menu_mobile() -> Result<String> {
    let mut fx = Fx::new()?;
    open_mobile(&mut fx)?;
    ensure!(fx.route() == Route::Mobile, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail.is_none() && !s.sidebar_open && s.can_go_back));
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(String::new())
}

fn mobile_button_hover() -> Result<String> {
    let mut fx = Fx::new()?;
    open_mobile(&mut fx)?;
    let at = fx.at("mobile", "Get started")?;
    let n = assert_hover(&mut fx, at, [at.0 - 200., at.1 - 18., 400., 36.], 1000)?;
    Ok(format!("{n} px"))
}

// --- Command menu, sidebar sections ---

fn command_menu_open(fx: &Fx) -> bool {
    fx.read(|r, _| r.command_menu().is_some())
}

fn command_menu_keys() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-k")?;
    let menu = fx
        .read(|r, _| r.command_menu().cloned())
        .context("cmd-k: no command menu")?;
    let focused = fx.cx.update_window(fx.window, |_, window, cx| {
        menu.focus_handle(cx).is_focused(window)
    })?;
    ensure!(focused, "the command menu is not focused");
    fx.keys("escape")?;
    ensure!(!command_menu_open(&fx), "Escape left it open");
    ensure!(fx.composer_focused()?, "Escape did not give the keys back");
    fx.keys("cmd-shift-p")?;
    ensure!(command_menu_open(&fx), "cmd-shift-p: no command menu");
    fx.keys("s u b t r a c t enter")?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(!command_menu_open(&fx), "selecting a chat left it open");
    Ok(String::new())
}

fn command_menu_search_click() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(SEARCH)?;
    ensure!(command_menu_open(&fx), "Search: no command menu");
    fx.click((100., 400.))?;
    ensure!(!command_menu_open(&fx), "a click outside left it open");
    // Live Recents: a chat row opens by click on the selected (first) match.
    fx.click(SEARCH)?;
    fx.keys("g r e e t enter")?;
    fx.wait_thread(GREETING)?;
    Ok(String::new())
}

/// A settings row opens Settings at that page: Appearance differs from `⌘,`'s General.
fn command_menu_settings_page() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-k")?;
    fx.keys("a p p e a r a n c e enter")?;
    ensure!(fx.route() == Route::Settings, "route {:?}", fx.route());
    fx.hover((640., 22.))?;
    let appearance = fx.shot()?;
    let mut general = Fx::new()?;
    general.keys("cmd-,")?;
    general.hover((640., 22.))?;
    let n = changed(&appearance, &general.shot()?, MAIN_PANEL);
    ensure!(n > 1000, "the page differs from General by {n} px");
    Ok(format!("{n} px"))
}

fn sidebar_section_collapse() -> Result<String> {
    let mut fx = Fx::new()?;
    let row = fx.at("home", "Add a subtract")?;
    let recents = fx.at("home", "Recents")?;
    fx.click(recents)?;
    fx.click(row)?;
    ensure!(
        fx.route() == Route::Home,
        "a collapsed row opened {:?}",
        fx.route()
    );
    fx.click(recents)?;
    fx.click(row)?;
    fx.wait_thread(SUBTRACT)?;
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.click(fx.at("home", "Projects")?)?;
    fx.hover((640., 22.))?;
    let n = changed(&before, &fx.shot()?, SIDEBAR_PANEL);
    ensure!(n > 1000, "collapsing Projects repainted {n} px");
    Ok(format!("Projects: {n} px"))
}

// --- Side panel: Changes, background terminal ---

/// Left edge of the side panel in a `w` px window (`ui::screens::diff::panel::panel_width`).
fn panel_left(w: f32) -> f32 {
    let main = w - 340. - 4.5;
    w - 4. - 664.17f32.min((main - 320.).max(320.))
}

/// The tab strip's close button.
fn close_tab(fx: &Fx) -> (f32, f32) {
    (panel_left(fx.size.0) + 229., 21.5)
}

fn side(fx: &Fx) -> Option<SidePanel> {
    fx.read(|r, _| r.side_panel())
}

/// The subtract thread with `⌥⌘B`'s Changes panel.
fn open_changes(fx: &mut Fx) -> Result<()> {
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")
}

fn changes_panel_keys() -> Result<String> {
    let mut fx = Fx::new()?;
    open_changes(&mut fx)?;
    ensure!(
        side(&fx) == Some(SidePanel::Changes),
        "alt-cmd-b: {:?}",
        side(&fx)
    );
    ensure!(
        fx.shell(|s| !s.new_tab),
        "the tab strip brings its own New tab"
    );
    fx.keys("alt-cmd-b")?;
    ensure!(side(&fx).is_none(), "alt-cmd-b again: {:?}", side(&fx));
    ensure!(fx.shell(|s| s.new_tab));
    fx.menu("View", "Toggle Changes Panel")?;
    ensure!(
        side(&fx) == Some(SidePanel::Changes),
        "menu: {:?}",
        side(&fx)
    );
    fx.click(close_tab(&fx))?;
    ensure!(side(&fx).is_none(), "tab close: {:?}", side(&fx));
    Ok(String::new())
}

fn view_changes_opens_panel() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.click(fx.at("thread", "View changes")?)?;
    ensure!(side(&fx) == Some(SidePanel::Changes), "{:?}", side(&fx));
    Ok(String::new())
}

fn diff_file_header_toggle() -> Result<String> {
    let mut fx = Fx::new()?;
    open_changes(&mut fx)?;
    let left = panel_left(fx.size.0);
    let panel = [left, 44., fx.size.0 - left, 784.];
    fx.hover((640., 22.))?;
    let open = fx.shot()?;
    fx.click((left + 100., 111.))?;
    fx.hover((640., 22.))?;
    let n = changed(&open, &fx.shot()?, panel);
    ensure!(n > 1000, "the header click repainted {n} px");
    Ok(format!("{n} px"))
}

/// The subtract thread with the reference's running background command replayed into it,
/// then `Toggle summary` ▸ the command's row.
fn open_terminal(fx: &mut Fx) -> Result<()> {
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let view = fx.read(|r, _| r.thread().cloned()).context("no thread")?;
    fx.cx.update(|cx| {
        view.update(cx, |v, cx| {
            for event in ui::screens::terminal::running_command_replay() {
                v.apply(&event, cx);
            }
        })
    });
    fx.cx.run_until_parked();
    // refs/thread: `Toggle summary` 63 px from the window's right edge; the summary popover
    // ends 12 px from it (refs/terminal, shifted to the full-width thread).
    let w = fx.size.0;
    fx.click((w - 63., 22.))?;
    fx.click((w - 202., 179.7))
}

fn background_terminal_tab() -> Result<String> {
    let mut fx = Fx::new()?;
    open_terminal(&mut fx)?;
    ensure!(side(&fx) == Some(SidePanel::Terminal), "{:?}", side(&fx));
    fx.click(close_tab(&fx))?;
    ensure!(side(&fx).is_none(), "tab close: {:?}", side(&fx));
    // The summary stays open: its `Changes` row opens the Changes tab.
    fx.click((fx.size.0 - 202., 105.7))?;
    ensure!(side(&fx) == Some(SidePanel::Changes), "{:?}", side(&fx));
    Ok(String::new())
}

// --- Settings ▸ Appearance ---

/// Theme control on the Appearance page: Light, Dark, System (the headless window's system
/// appearance is light). The segmented control ends at the 768 px column's right padding.
fn appearance_theme_control() -> Result<String> {
    use theme::{Appearance, Theme};
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    fx.click(fx.at("settings", "Appearance")?)?;
    for (label, x, want) in [
        ("Light", 1013., Appearance::Light),
        ("Dark", 1064., Appearance::Dark),
        ("System", 1122., Appearance::Light),
    ] {
        fx.click((x, 258.))?;
        let got = fx.cx.update(|cx| cx.global::<Theme>().appearance);
        ensure!(got == want, "{label}: theme {got:?}");
    }
    Ok(String::new())
}

/// `Confirm before closing a window` select: its menu opens, `Never` sets the value and closes
/// it.
fn settings_select_menu() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    let at = fx.at("settings", "Close shortcut only")?;
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.click(at)?;
    fx.hover((640., 22.))?;
    let n = changed(&before, &fx.shot()?, MAIN_PANEL);
    ensure!(n > 2000, "the menu repainted {n} px");
    // The window's bottom edge is below the trigger, so the menu (3 rows of 32 px in 4 px
    // padding) opens above it: `Never` is its last row.
    fx.click((at.0, at.1 - 34.))?;
    fx.hover((640., 22.))?;
    let after = fx.shot()?;
    let label = changed(&before, &after, [at.0 - 80., at.1 - 14., 160., 28.]);
    ensure!(label > 50, "the trigger label repainted {label} px");
    let left = changed(&before, &after, MAIN_PANEL);
    ensure!(left < n / 2, "the menu stayed open ({left} px)");
    Ok(format!("{n} px"))
}
