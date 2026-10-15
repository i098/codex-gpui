//! Live proof: the real root view on a real `codex app-server`, headless. Types the prompt
//! into the Home composer with key events, presses Enter, waits for `turn/completed`,
//! checks the thread view shows `pong`, prints the transcript and saves
//! `live-1-home.png`, `live-2-streaming.png` and `live-3-done.png` to `OUT`.
//! Run on the Mac: `cargo run -p app --example live_turn`.

use anyhow::{Context as _, Result, ensure};
use app::{Root, State};
use bridge::types::{ThreadItem, Turn, TurnStatus, UserInput};
use gpui::{AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, Keystroke, px, size};
use std::time::{Duration, Instant};
use std::{fs, sync::Arc};

const CODEX_HOME: &str = "/tmp/cgx-mac/home-live";
const CWD: &str = "/tmp/cgx-mac/proj-live";
const OUT: &str = "/tmp/cgx-mac/live";
const PROMPT: &str = "Reply with the single word: pong";
const TIMEOUT: Duration = Duration::from_secs(120);

fn main() -> Result<()> {
    let mut cmd = bridge::default_command();
    cmd.env("CODEX_HOME", CODEX_HOME).current_dir(CWD);
    let (bridge, events) = bridge::Bridge::spawn(cmd)?;

    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        ui::composer::bind_keys(cx);
    });
    // The app-server answers on its own threads: let the scheduler wait for real I/O.
    cx.allow_parking();
    let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        cx.new(|cx| Root::new(State::default(), bridge, events, CWD.into(), window, cx))
    })?;
    let root = cx.read_window(&window, |root, _| root)?;
    let handle: AnyWindowHandle = window.into();
    fs::create_dir_all(OUT)?;
    let shot = |cx: &mut HeadlessAppContext, name: &str| -> Result<()> {
        let path = format!("{OUT}/{name}");
        cx.capture_screenshot(handle)?.save(&path)?;
        println!("saved {path}");
        Ok(())
    };

    let started = Instant::now();
    pump(&mut cx, started, "startup", |cx| {
        root.read_with(cx, |r, _| r.ready())
    })?;
    cx.update_window(handle, |_, window, cx| {
        for c in PROMPT.chars() {
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
    cx.run_until_parked();
    shot(&mut cx, "live-1-home.png")?;
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_keystroke(Keystroke::parse("enter")?, cx);
        anyhow::Ok(())
    })??;

    // Streaming: re-shoot whenever the running turn gains an item or text.
    let mut seen = 0;
    pump(&mut cx, started, "turn/completed", |cx| {
        let Some(turn) = last_turn(cx, &root) else {
            return false;
        };
        if turn.status != TurnStatus::InProgress {
            return true;
        }
        let progress = turn.items.len() + agent_text(&turn).len();
        if progress > seen {
            seen = progress;
            shot(cx, "live-2-streaming.png").expect("streaming screenshot");
        }
        false
    })?;
    ensure!(seen > 0, "no streaming frame before the turn completed");
    shot(&mut cx, "live-3-done.png")?;

    let turn = last_turn(&cx, &root).context("no turn")?;
    for item in &turn.items {
        match item {
            ThreadItem::UserMessage { content, .. } => {
                for input in content {
                    if let UserInput::Text { text } = input {
                        println!("user:  {text}");
                    }
                }
            }
            ThreadItem::AgentMessage { text, .. } => println!("codex: {text}"),
            other => println!("item:  {other:?}"),
        }
    }
    println!(
        "turn {:?} in {:.1} s",
        turn.status,
        started.elapsed().as_secs_f32()
    );
    ensure!(
        turn.status == TurnStatus::Completed,
        "turn ended {:?}: {:?}",
        turn.status,
        turn.error
    );
    ensure!(
        agent_text(&turn).to_lowercase().contains("pong"),
        "the thread view does not show pong"
    );
    Ok(())
}

/// Runs the app until `done`, polling real I/O, for at most [`TIMEOUT`] since `started`.
fn pump(
    cx: &mut HeadlessAppContext,
    started: Instant,
    what: &str,
    mut done: impl FnMut(&mut HeadlessAppContext) -> bool,
) -> Result<()> {
    let mut last = Instant::now();
    loop {
        cx.run_until_parked();
        if done(cx) {
            return Ok(());
        }
        ensure!(started.elapsed() < TIMEOUT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
        // Real time passes for timers too (`Working for` ticks).
        cx.advance_clock(last.elapsed());
        last = Instant::now();
    }
}

fn last_turn(cx: &HeadlessAppContext, root: &Entity<Root>) -> Option<Turn> {
    root.read_with(cx, |r, _| r.thread().cloned())?
        .read_with(cx, |v, _| v.turns().last().cloned())
}

fn agent_text(turn: &Turn) -> String {
    turn.items
        .iter()
        .filter_map(|i| match i {
            ThreadItem::AgentMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
