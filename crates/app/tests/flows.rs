//! Flow harness (docs/flows.md): the real [`app::Root`] on a replayed app-server
//! (fixtures/readback.jsonl, plus a recorded session for turns), driven by mouse and key
//! events at the original's element bounds (refs/<screen>/dark@1280x832.spec.json) and by the
//! menu bar's actions. Each parity flow checks the reached route and fails on a pixel
//! mismatch under the same strict thresholds as the separate static screen gate.
//!
//! Runs every flow, then fails if any did. `cargo test -p app --test flows -- <filter>` runs
//! only the flows whose name contains `<filter>`.

mod common;

use anyhow::{Context as _, Result, ensure};
use app::{Route, SidePanel, ThemeSetting};
use bridge::types::{ThreadItem, TurnStatus};
use common::*;
use gpui::{Focusable as _, ScrollDelta, TouchPhase, point, px, size};
use image::RgbaImage;
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

type Flow = fn() -> Result<String>;

const FLOWS: &[(&str, Flow)] = &[
    ("launch_home", launch_home),
    ("parity_home", || parity_flow("home", |_| Ok(()))),
    ("rail_home_from_thread", rail_home_from_thread),
    ("rail_thread_family", rail_thread_family),
    ("rail_home_reselect_resets", rail_home_reselect_resets),
    ("rail_hover_fill", rail_hover_fill),
    ("rail_drag_reorders_pinned", rail_drag_reorders_pinned),
    ("resize_three_widths", resize_three_widths),
    ("sidebar_toggle_keys", sidebar_toggle_keys),
    ("sidebar_toggle_titlebar", sidebar_toggle_titlebar),
    ("thread_row_opens_thread", thread_row_opens_thread),
    ("thread_row_hover", thread_row_hover),
    ("cmd_digit_goes_to_chat", cmd_digit_goes_to_chat),
    ("previous_next_chat", previous_next_chat),
    ("new_chat_row", new_chat_row),
    ("new_chat_same_route_history", new_chat_same_route_history),
    ("your_dot_row", your_dot_row),
    ("sidebar_empty_lists", sidebar_empty_lists),
    ("cmd_n_new_chat", cmd_n_new_chat),
    ("editor_click_focuses", editor_click_focuses),
    ("model_picker_popover", model_picker_popover),
    ("composer_button_hover", composer_button_hover),
    ("model_list_loading", model_list_loading),
    ("submit_enter_starts_thread", submit_enter_starts_thread),
    ("submit_click_send", submit_click_send),
    ("shift_enter_newline", shift_enter_newline),
    ("parity_composer", || {
        parity_flow("composer", |fx| {
            fx.type_text("Refactor src/math.py")?;
            let (x, y) = fx.at("home", "GPT-6 Astra")?;
            // The original's pill moves with the centered column and the window bottom.
            fx.click((x + (fx.size.0 - 1280.) / 2., y + fx.size.1 - 832.))?;
            ensure!(fx.composer_text()? == "Refactor src/math.py");
            let composer = fx.composer();
            ensure!(
                fx.cx.update_window(fx.window, |_, window, cx| {
                    composer.read(cx).menu_state(window).0
                })?,
                "the composer menu did not open"
            );
            Ok(())
        })
    }),
    ("parity_screenshot", || {
        parity_flow("screenshot", |fx| {
            // The selection result of `screenshotToCurrentChat`: the s017 recording's PNG.
            let rec: Value = serde_json::from_slice(&std::fs::read(format!(
                "{ROOT}/refs/interactions/s017.json"
            ))?)?;
            let png: Vec<u8> =
                serde_json::from_value(rec["steps"][1]["clipboard"]["pngBytes"].clone())?;
            let composer = fx.composer();
            fx.cx
                .update(|cx| composer.update(cx, |composer, cx| composer.attach_png(png, cx)));
            fx.cx.run_until_parked();
            Ok(())
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
    ("parity_automations", || {
        parity_flow("automations", open_recorded_automations)
    }),
    ("rail_explore_menu", rail_explore_menu),
    ("voice_button_notice", voice_button_notice),
    ("home_beacon", home_beacon),
    ("rail_focus_ring", rail_focus_ring),
    ("profile_menu_settings", profile_menu_settings),
    ("profile_menu_dismiss", profile_menu_dismiss),
    ("profile_menu_mobile", profile_menu_mobile),
    ("mobile_button_hover", mobile_button_hover),
    ("mobile_focus_ring", mobile_focus_ring),
    ("mobile_resize_three_widths", mobile_resize_three_widths),
    ("parity_mobile", || parity_flow("mobile", open_mobile)),
    ("command_menu_keys", command_menu_keys),
    ("command_menu_search_click", command_menu_search_click),
    ("command_menu_settings_page", command_menu_settings_page),
    ("sidebar_section_collapse", sidebar_section_collapse),
    ("changes_panel_keys", changes_panel_keys),
    ("view_changes_opens_panel", view_changes_opens_panel),
    ("thread_card_focus_ring", thread_card_focus_ring),
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
    ("settings_language_menu", settings_language_menu),
    ("parity_sidebar", || {
        parity_flow("sidebar", |fx| {
            open_recorded_sidebar_search(fx)?;
            Ok(())
        })
    }),
    ("worked_for_expands", worked_for_expands),
    ("copy_response", copy_response),
    ("fork_chat", fork_chat),
    ("earlier_turn_actions_hover", earlier_turn_actions_hover),
    ("undo_reverts_turn", undo_reverts_turn),
    ("approval_round_trip", approval_round_trip),
    ("archive_chat", archive_chat),
    ("recent_chat_keys", recent_chat_keys),
    ("effort_pick_writes_config", effort_pick_writes_config),
    ("settings_value_round_trip", settings_value_round_trip),
    ("scroll_sidebar_extent", scroll_sidebar_extent),
    ("scroll_sidebar_momentum", scroll_sidebar_momentum),
    ("scroll_thread", scroll_thread),
    ("scroll_diff", scroll_diff),
    ("scroll_settings", scroll_settings),
    ("scroll_automations", scroll_automations),
    ("resize_live", resize_live),
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
    fx.click(RAIL_HOME)?;
    ensure!(fx.route() == Route::Home, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail) == Some("home"));
    ensure!(fx.composer_focused()?);
    Ok(String::new())
}

fn rail_thread_family() -> Result<String> {
    let mut mismatches = 0;
    for (screen, panel) in [
        ("thread", None),
        ("diff", Some(SidePanel::Changes)),
        ("terminal", Some(SidePanel::Terminal)),
    ] {
        for theme in ["dark", "light"] {
            for size in [(1280., 832.), (1440., 900.)] {
                let mut fx = Fx::with(theme, size, None, |_| {})?;
                match screen {
                    "thread" => {
                        fx.click(fx.at("home", "Add a subtract")?)?;
                        fx.wait_thread(SUBTRACT)?;
                    }
                    "diff" => open_changes(&mut fx)?,
                    _ => open_terminal(&mut fx)?,
                }
                ensure!(fx.read(|r, _| matches!(r.route(), Route::Thread(id) if id == SUBTRACT)));
                ensure!(side(&fx) == panel);
                let reference = image::open(format!(
                    "{ROOT}/refs/{screen}/{theme}@{}x{}.png",
                    size.0, size.1
                ))?
                .into_rgba8();
                let port = fx.shot()?;
                port.save(format!(
                    "{}/rail-{screen}-{theme}@{}x{}.png",
                    env!("CARGO_TARGET_TMPDIR"),
                    size.0,
                    size.1
                ))?;
                // The Terminal unread dot ends at y=66; compare the shared selection below it.
                let rail = if screen == "terminal" {
                    [8., 66., 36., 22.]
                } else {
                    [8., 52., 36., 36.]
                };
                let n = changed(&reference, &port, rail);
                println!("{screen} {theme} {}x{} rail={n}", size.0, size.1);
                mismatches += usize::from(n != 0);
            }
        }
    }
    ensure!(mismatches == 0, "{mismatches} original rail regions differ");
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

/// Dragging pinned `Code Review` onto `Sites` swaps them, without opening Code Review; a drag
/// back restores the order.
fn rail_drag_reorders_pinned() -> Result<String> {
    // refs/home/dark@1280x832.spec.json: the two pinned items below the divider.
    const CODE_REVIEW: (f32, f32) = (26., 299.);
    const SITES: (f32, f32) = (26., 343.);
    // The window is 2x: a 36 px item is 72 device px.
    let crop = |shot: &RgbaImage, (x, y): (f32, f32)| {
        let (x, y) = (((x - 18.) * 2.) as u32, ((y - 18.) * 2.) as u32);
        image::imageops::crop_imm(shot, x, y, 72, 72).to_image()
    };
    let rest = |fx: &mut Fx| -> Result<RgbaImage> {
        fx.hover((640., 22.))?;
        fx.advance(ui::motion::BASIC)?;
        fx.shot()
    };
    let mut fx = Fx::new()?;
    let before = rest(&mut fx)?;
    fx.capture_evidence("CGX_RAIL_DRAG_EVIDENCE", "before-rail")?;
    ensure!(
        crop(&before, CODE_REVIEW) != crop(&before, SITES),
        "the pinned items look alike"
    );
    fx.drag(CODE_REVIEW, SITES, 8)?;
    let after = rest(&mut fx)?;
    fx.capture_evidence("CGX_RAIL_DRAG_EVIDENCE", "after-rail")?;
    ensure!(
        fx.route() == Route::Home,
        "the drag opened {:?}",
        fx.route()
    );
    ensure!(
        crop(&after, SITES) == crop(&before, CODE_REVIEW)
            && crop(&after, CODE_REVIEW) == crop(&before, SITES),
        "the pinned items did not swap"
    );
    fx.drag(CODE_REVIEW, SITES, 8)?;
    let back = rest(&mut fx)?;
    let n = changed(&before, &back, [8., 281., 36., 80.]);
    ensure!(n == 0, "a drag back did not restore the order ({n} px)");
    Ok(String::new())
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
        let before = fx.shot()?;
        fx.keys("cmd-b")?;
        ensure!(fx.sidebar_open() != wide, "{w} px: cmd-b did not toggle");
        // Below the breakpoint the panel floats: the main surface keeps its place.
        ensure!(fx.shell(|s| s.overlay) != wide, "{w} px: overlay flag");
        if !wide {
            let n = changed(&before, &fx.shot()?, [452., 44., 148., 784.]);
            ensure!(
                n == 0,
                "{w} px: the overlay moved the main surface ({n} px)"
            );
        }
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

fn your_dot_row() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Your dot")?)?;
    ensure!(
        fx.cx.take_opened_url().as_deref() == Some("https://chatgpt.com/dots/home"),
        "Your dot did not open the dot home page"
    );
    ensure!(fx.route() == Route::Home, "route {:?}", fx.route());
    Ok(String::new())
}

fn new_chat_same_route_history() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.type_text("Discard this draft")?;
    fx.click(fx.at("home", "New chat")?)?;
    ensure!(fx.route() == Route::Home);
    ensure!(fx.composer_focused()?);
    ensure!(fx.composer_text()?.is_empty());
    ensure!(fx.shell(|shell| shell.can_go_back && !shell.can_go_forward));
    fx.capture_evidence("CGX_HISTORY_PROOF", "new-chat")?;
    fx.keys("cmd-[")?;
    ensure!(fx.route() == Route::Home);
    ensure!(fx.shell(|shell| !shell.can_go_back && shell.can_go_forward));
    fx.keys("cmd-]")?;
    ensure!(fx.route() == Route::Home);
    ensure!(fx.shell(|shell| shell.can_go_back && !shell.can_go_forward));
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
    ensure!(
        fx.read(|r, _| r.dialog().is_some()),
        "close confirmation missing"
    );
    fx.keys("tab enter")?;
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

fn open_recorded_automations(fx: &mut Fx) -> Result<()> {
    let host = app::host::ReplayHost::from_recording(include_str!(
        "../../../refs/interactions/host.json"
    ))?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(std::sync::Arc::new(host), cx)
    });
    fx.click(RAIL_SCHEDULED)
}

fn rail_scheduled() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.click(RAIL_SCHEDULED)?;
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("scheduled") && s.can_go_back && !s.new_tab));
    fx.click(BACK)?;
    ensure!(fx.route() == Route::Home, "Back: {:?}", fx.route());
    Ok(String::new())
}

/// The rail's Explore `…` item.
const RAIL_EXPLORE: (f32, f32) = (26., 246.);

fn rail_explore_menu() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.capture_evidence("CGX_BEACON_EVIDENCE", "before-rail")?;
    fx.click(RAIL_EXPLORE)?;
    ensure!(
        fx.shell(|s| s.explore_menu),
        "Explore did not open its menu"
    );
    fx.capture_evidence("CGX_BEACON_EVIDENCE", "after-explore-menu")?;
    fx.click(RAIL_EXPLORE)?;
    ensure!(fx.shell(|s| !s.explore_menu), "a second click left it open");
    fx.click(RAIL_EXPLORE)?;
    fx.click((800., 400.))?;
    ensure!(
        fx.shell(|s| !s.explore_menu),
        "a click outside left it open"
    );
    ensure!(fx.route() == Route::Home);
    Ok(String::new())
}

/// With an empty draft the send button is `Start new voice chat`: it starts no turn and
/// answers with the BOUNDARY notice (no realtime backend).
fn voice_button_notice() -> Result<String> {
    let mut fx = Fx::new()?;
    let at = fx.at("home", "Start new voice chat")?;
    fx.click(at)?;
    ensure!(fx.route() == Route::Home, "route {:?}", fx.route());
    ensure!(
        fx.read(|root, cx| root.toasts(cx).iter().any(|t| t.contains("voice chat"))),
        "no voice notice"
    );
    Ok(String::new())
}

/// Tab reaches every rail item and the avatar, each drawing a ring around its 36 px button.
fn rail_focus_ring() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    let items: Vec<_> = [
        RAIL_HOME,
        RAIL_SPACE,
        RAIL_SCHEDULED,
        (26., 202.),
        RAIL_EXPLORE,
    ]
    .into_iter()
    .collect();
    let mut seen = vec![false; items.len()];
    for _ in 0..60 {
        fx.keys("tab")?;
        let now = fx.shot()?;
        for (i, (x, y)) in items.iter().enumerate() {
            // The 2 px ring lies outside the 36 px button: sample its 4 px band.
            let band = [x - 22., y - 22., 44., 4.];
            seen[i] |= changed(&before, &now, band) >= 20;
        }
    }
    ensure!(
        seen.iter().all(|s| *s),
        "rail items without a ring: {seen:?}"
    );
    Ok(String::new())
}

/// Beacon `Start Voice` raises the voice request; `×` hides the banner.
fn home_beacon() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.capture_evidence("CGX_BEACON_EVIDENCE", "before-home")?;
    let home = fx.read(|root, _| root.home().clone());
    fx.cx
        .update(|cx| home.update(cx, |home, cx| home.set_beacon(true, cx)));
    fx.cx.run_until_parked();
    fx.capture_evidence("CGX_BEACON_EVIDENCE", "after-beacon")?;
    let (x, y) = fx.at("home", "Choose project")?;
    // The buttons sit 550 and 622 px right of `Choose project`, one slot above it.
    fx.click((x + 550., y - 61.))?;
    ensure!(
        fx.read(|root, cx| !root.toasts(cx).is_empty()),
        "Start Voice raised no request"
    );
    fx.click((x + 622., y - 61.))?;
    ensure!(
        !fx.cx.update(|cx| home.read(cx).beacon),
        "× left the beacon open"
    );
    Ok(String::new())
}

/// The profile avatar, at the rail's bottom.
fn avatar(fx: &Fx) -> (f32, f32) {
    (26., fx.size.1 - 26.)
}

/// Profile menu rows `Settings` and `Set up mobile`, above the `Keyboard Shortcuts` row.
fn profile_item(fx: &Fx, n: usize) -> (f32, f32) {
    (118., fx.size.1 - [192., 100.][n])
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

/// Pixels of the 2 px `#3a83f7` keyboard ring in `[x, y, w, h]`.
fn ring_pixels(shot: &RgbaImage, [x, y, w, h]: [f32; 4]) -> usize {
    let (x1, y1) = (((x + w) * 2.) as u32, ((y + h) * 2.) as u32);
    ((y * 2.) as u32..y1)
        .flat_map(|py| ((x * 2.) as u32..x1).map(move |px| (px, py)))
        .filter(|&(px, py)| {
            let p = shot.get_pixel(px, py);
            [0x3a, 0x83, 0xf7]
                .iter()
                .zip(p.0)
                .all(|(want, got)| (*want - got as i32).abs() <= 8)
        })
        .count()
}

/// Tab reaches `Get started`, which draws the 2 px ring (`focus-visible:ring-2`) above its
/// pill; a pointer press clears it.
fn mobile_focus_ring() -> Result<String> {
    let mut fx = Fx::new()?;
    open_mobile(&mut fx)?;
    fx.hover((640., 22.))?;
    // The original's 400 × 36 pill starts at y 560.25: its ring is the 2 px above it.
    let band = [152., 557.25, 400., 3.];
    ensure!(
        ring_pixels(&fx.shot()?, band) == 0,
        "a ring shows without focus"
    );
    let mut step = None;
    for n in 1..=40 {
        fx.keys("tab")?;
        if ring_pixels(&fx.shot()?, band) > 1000 {
            step = Some(n);
            break;
        }
    }
    let step = step.context("Tab never drew a ring on Get started")?;
    fx.capture_evidence("CGX_MOBILE_EVIDENCE", "focus-get-started")?;
    fx.click((900., 200.))?;
    let left = ring_pixels(&fx.shot()?, band);
    ensure!(left == 0, "a pointer press left {left} ring px");
    Ok(format!("tab {step}"))
}

/// At 1280, 767 and 600 px the setup column stays centered in its area. Below 1024 px the
/// original hides the artwork (`hidden lg:block`), so the column takes the whole main surface.
fn mobile_resize_three_widths() -> Result<String> {
    let mut centers = Vec::new();
    for w in [1280., 767., 600.] {
        let mut fx = Fx::with("dark", (w, 832.), None, |_| {})?;
        open_mobile(&mut fx)?;
        ensure!(
            fx.route() == Route::Mobile,
            "{w} px: route {:?}",
            fx.route()
        );
        fx.hover((w / 2., 22.))?;
        fx.capture_evidence("CGX_MOBILE_EVIDENCE", &format!("resize-{w}"))?;
        let shot = fx.shot()?;
        // The main surface spans 52..w - 4 px; at 1280 px the column has its left half.
        let center = if w >= 1024. {
            352.
        } else {
            (52. + w - 4.) / 2.
        };
        // `Get started`'s near-white pill, 400 px wide on the row through its label.
        let y = 578 * 2;
        let light = |x: f32| {
            let [r, g, b, _] = shot.get_pixel((x * 2.) as u32, y).0;
            r.min(g).min(b) > 200 && r.max(g).max(b) - r.min(g).min(b) < 16
        };
        ensure!(
            light(center - 150.) && light(center + 150.),
            "{w} px: Get started is not centered at {center}"
        );
        let (mut left, mut right) = (center - 150., center + 150.);
        while light(left - 0.5) {
            left -= 0.5;
        }
        while light(right + 0.5) {
            right += 0.5;
        }
        let mid = (left + right) / 2.;
        ensure!(
            (mid - center).abs() <= 1.,
            "{w} px: pill {left}..{right}, center {mid}, expected {center}"
        );
        centers.push(mid);
    }
    Ok(format!("{centers:?}"))
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

/// Shift-Tab from the composer reaches the diff card's `View changes`, then `Undo`; each
/// draws the 2 px `#3a83f7` ring only while it has keyboard focus.
fn thread_card_focus_ring() -> Result<String> {
    // The original's button boxes (refs/thread spec nodes 310 and 312), plus the 2 px ring.
    const UNDO: [f32; 4] = [982.95, 334.8, 72.71, 32.];
    const VIEW: [f32; 4] = [1059.66, 334.8, 106.59, 32.];
    let ring = ring_pixels;
    let mut fx = Fx::new()?;
    open_subtract(&mut fx)?;
    fx.focus_composer()?;
    let rest = fx.shot()?;
    ensure!(
        ring(&rest, UNDO) + ring(&rest, VIEW) == 0,
        "a ring shows without focus"
    );
    let mut reached = Vec::new();
    for step in 1..=40 {
        fx.keys("shift-tab")?;
        let shot = fx.shot()?;
        let (undo, view) = (ring(&shot, UNDO), ring(&shot, VIEW));
        ensure!(undo == 0 || view == 0, "both card buttons ringed");
        if let Some((name, n)) = [("View changes", view), ("Undo", undo)]
            .into_iter()
            .find(|(_, n)| *n > 300)
        {
            fx.capture_evidence("CGX_CARD_FOCUS_EVIDENCE", &format!("{step}-{name}"))?;
            reached.push((name, step, n));
            if reached.len() == 2 {
                break;
            }
        }
    }
    let order: Vec<_> = reached.iter().map(|(name, ..)| *name).collect();
    ensure!(
        order == ["View changes", "Undo"],
        "Shift-Tab reached {reached:?}"
    );
    fx.hover((640., 22.))?;
    let moved = ring(&fx.shot()?, UNDO);
    ensure!(moved > 300, "a pointer move cleared the ring ({moved} px)");
    Ok(format!("{reached:?}"))
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
    let status: Vec<_> = include_str!("../../../fixtures/sessions/subtract.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .filter(|line| line["msg"]["method"] == "thread/status/changed")
        .map(|line| bridge::Event::Notification {
            method: "thread/status/changed".into(),
            params: line["msg"]["params"].clone(),
        })
        .collect();
    let mut events = ui::screens::terminal::running_command_replay();
    for event in &mut events {
        if let bridge::Event::Notification { params, .. } = event
            && params["turn"].is_object()
        {
            params["turn"]["rootTurnId"] = params["turn"]["id"].clone();
        }
    }
    ensure!(status.len() == 2, "recorded active and idle statuses");
    events.insert(0, status[0].clone());
    events.push(status[1].clone());
    for (index, event) in events.into_iter().enumerate() {
        fx.root.update(&mut fx.cx, |r, cx| r.on_event(event, cx));
        fx.cx.run_until_parked();
        let observed = fx.read(|r, cx| {
            (
                r.sidebar().read(cx).is_running(SUBTRACT),
                r.sidebar().read(cx).unread.iter().any(|id| id == SUBTRACT),
                r.shell().read(cx).attention,
            )
        });
        let expected = (index < 5, index >= 4, index >= 4);
        ensure!(
            observed == expected,
            "Terminal event {index}: running/unread/attention {observed:?}, expected {expected:?}"
        );
    }
    // refs/thread: `Toggle summary` 63 px from the window's right edge; the summary popover
    // ends 12 px from it (refs/terminal, shifted to the full-width thread).
    let w = fx.size.0;
    fx.click((w - 63., 22.))?;
    fx.click((w - 202., 179.7))?;
    ensure!(
        fx.read(|r, cx| {
            let view = r.thread().expect("open thread").read(cx);
            view.summary_open
                && matches!(
                    view.background_terminals().as_slice(),
                    [(command, "2\n")]
                        if command == "python3 -c 'from src.math import subtract; print(subtract(5, 3))'"
                )
        }),
        "Terminal lost the open summary, running command, or recorded output"
    );
    ensure!(
        fx.read(|r, cx| r.sidebar().read(cx).unread == [SUBTRACT]) && fx.shell(|s| s.attention),
        "Opening Terminal cleared the selected unread thread or Home attention"
    );
    Ok(())
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
    ensure!(
        fx.read(|r, cx| r.sidebar().read(cx).unread == [SUBTRACT]) && fx.shell(|s| s.attention),
        "Changing side tabs cleared the selected unread thread or Home attention"
    );
    fx.click(RAIL_HOME)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(
        fx.read(|r, cx| r.sidebar().read(cx).unread.is_empty()) && !fx.shell(|s| s.attention),
        "Opening the unread thread did not clear its dot and Home attention"
    );
    Ok("six Root events; selected unread and Home attention set, then cleared on open".into())
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

/// Choosing Never updates the saved close preference; Escape leaves the value unchanged.
fn settings_select_menu() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    let at = fx.at("settings", "Close shortcut only")?;
    fx.click(at)?;
    fx.capture_evidence("CGX_GENERAL_CONTROLS_EVIDENCE", "close-menu-open")?;
    fx.keys("end enter")?;
    fx.wait("saved close preference", |fx| {
        fx.read(|root, _| root.config()["host"]["confirm-before-close"] == "never")
    })?;
    fx.capture_evidence("CGX_GENERAL_CONTROLS_EVIDENCE", "close-menu-selected")?;
    fx.click(at)?;
    fx.keys("home escape")?;
    ensure!(
        fx.read(|root, _| root.config()["host"]["confirm-before-close"] == "never"),
        "Escape changed the close preference"
    );
    Ok("Never persists; Escape cancels selection".into())
}

/// The searchable Language menu saves a filtered choice, Escape keeps it, and `Auto detect`
/// clears the override.
fn settings_language_menu() -> Result<String> {
    let mut lines = script(None, |_| {})?;
    for _ in 0..2 {
        lines.extend(exchange("config/value/write", json!({ "status": "ok" })));
    }
    let mut fx = Fx::scripted(lines)?;
    let locale = |fx: &Fx| fx.read(|root, _| root.config()["desktop"]["localeOverride"].clone());
    fx.keys("cmd-,")?;
    let at = fx.at("settings", "Auto detect")?;
    fx.capture_evidence("CGX_GENERAL_CONTROLS_EVIDENCE", "general-default")?;
    fx.click(at)?;
    fx.capture_evidence("CGX_GENERAL_CONTROLS_EVIDENCE", "language-menu")?;
    fx.type_text("French")?;
    fx.capture_evidence("CGX_GENERAL_CONTROLS_EVIDENCE", "language-filter")?;
    fx.keys("enter")?;
    fx.wait("saved language", |fx| locale(fx) == "fr-CA")?;
    fx.click(at)?;
    fx.type_text("Deutsch")?;
    fx.keys("escape")?;
    ensure!(locale(&fx) == "fr-CA", "Escape changed the language");
    fx.click(at)?;
    fx.keys("up enter")?;
    fx.wait("cleared language", |fx| locale(fx).is_null())?;
    Ok("French (Canada) persists by alias; Auto detect clears it".into())
}

// --- Thread actions ---

fn open_subtract(fx: &mut Fx) -> Result<()> {
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)
}

/// The open thread holds agent message `id`.
fn has_item(fx: &Fx, id: &str) -> bool {
    fx.read(|r, cx| {
        r.thread().is_some_and(|v| {
            v.read(cx)
                .turns()
                .iter()
                .flat_map(|t| &t.items)
                .any(|i| matches!(i, ThreadItem::AgentMessage { id: item, .. } if item == id))
        })
    })
}

/// `Worked for 17s` expands the turn's hidden work, and a second click folds it again. Only
/// the transcript above the docked composer is compared (its caret blinks).
fn worked_for_expands() -> Result<String> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s014.json"))?;
    let wait = std::time::Duration::from_millis(
        recording["steps"][2]["waitMs"]
            .as_u64()
            .context("S014 disclosure waitMs")?,
    );
    let transcript = [340., 44., 936., 600.];
    let mut fx = Fx::new()?;
    open_subtract(&mut fx)?;
    let at = fx.at("thread", "Worked for")?;
    fx.hover((640., 22.))?;
    let folded = fx.shot()?;
    fx.click(at)?;
    fx.advance(wait)?;
    fx.hover((640., 22.))?;
    let n = changed(&folded, &fx.shot()?, transcript);
    ensure!(n > 1000, "expanding repainted {n} px");
    fx.click(at)?;
    fx.advance(wait)?;
    fx.hover((640., 22.))?;
    let left = changed(&folded, &fx.shot()?, transcript);
    ensure!(left == 0, "folding left {left} px");
    Ok(format!("{n} px"))
}

fn copy_response() -> Result<String> {
    let mut fx = Fx::new()?;
    open_subtract(&mut fx)?;
    fx.click(fx.at("thread", "Copy")?)?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    let answer = fx.read(|r, cx| {
        let view = r.thread().expect("thread").read(cx);
        match view.turns().last().and_then(|t| t.items.last()) {
            Some(ThreadItem::AgentMessage { text, .. }) => text.clone(),
            _ => String::new(),
        }
    });
    ensure!(
        !answer.is_empty() && copied.as_deref() == Some(answer.as_str()),
        "clipboard {copied:?}"
    );
    Ok(String::new())
}

/// More actions > `Branch in new chat` under the last turn: `thread/fork` (no rollback), the fork opens
/// and heads Recents.
fn fork_chat() -> Result<String> {
    const FORK: &str = "fork-of-subtract";
    let mut thread = recorded_thread(SUBTRACT)?;
    thread["id"] = FORK.into();
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/fork", json!({ "thread": thread })));
    let mut fx = Fx::scripted(lines)?;
    open_subtract(&mut fx)?;
    let copy = fx.at("thread", "Copy")?;
    fx.click((copy.0 + 84., copy.1))?;
    fx.keys("enter")?;
    fx.wait_thread(FORK)?;
    let recents = fx.read(|r, _| r.recents());
    ensure!(
        recents.first().map(String::as_str) == Some(FORK),
        "{recents:?}"
    );
    Ok(String::new())
}

/// A thread with two turns: hovering the first one shows its message actions.
fn earlier_turn_actions_hover() -> Result<String> {
    let mut fx = Fx::with("dark", (1280., 832.), None, |v| {
        if let Some(thread) = v.pointer_mut("/msg/result/thread")
            && thread["id"] == SUBTRACT
            && let Some(turns) = thread["turns"].as_array_mut()
        {
            let mut later = turns[0].clone();
            later["id"] = "later-turn".into();
            turns.push(later);
        }
    })?;
    open_subtract(&mut fx)?;
    // The first turn sits where refs/thread shows the only one.
    let actions = fx.at("thread", "Copy")?;
    let row = [actions.0 - 16., actions.1 - 16., 120., 32.];
    let at = fx.at("thread", "Worked for")?;
    let n = assert_hover(&mut fx, at, row, 50)?;
    Ok(format!("{n} px"))
}

/// Undo and Reapply use the host Git worker without deleting the turn or changing the index.
fn undo_reverts_turn() -> Result<String> {
    use std::{fs, process::Command, sync::Arc};
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    let git = |args: &[&str]| -> Result<()> {
        let output = Command::new("git").current_dir(root).args(args).output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    };
    git(&["init", "-q"])?;
    fs::create_dir(root.join("src"))?;
    let before = "def add(a, b):\n    return a + b\n";
    let after = format!("{before}\n\ndef subtract(a, b):\n    return a - b\n");
    fs::write(root.join("src/math.py"), before)?;
    fs::write(root.join("keep.txt"), "base\n")?;
    git(&["add", "."])?;
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-qm",
        "base",
    ])?;
    fs::write(root.join("src/math.py"), &after)?;
    fs::write(root.join("keep.txt"), "unrelated\n")?;
    let index = fs::read(root.join(".git/index"))?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        let Some(thread) = line.pointer_mut("/msg/result/thread") else {
            continue;
        };
        if thread["id"] != SUBTRACT {
            continue;
        }
        thread["cwd"] = json!(root);
        for item in thread["turns"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .flat_map(|turn| turn["items"].as_array_mut().unwrap())
        {
            if let Some(changes) = item["changes"].as_array_mut() {
                for change in changes {
                    change["path"] = json!(root.join("src/math.py"));
                }
            }
        }
    }
    let mut fx = Fx::scripted(lines)?;
    let host = Arc::new(app::host::LocalHost::new(root.join(".git/profile")));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    open_subtract(&mut fx)?;
    let at = fx.at("thread", "Undo")?;
    fx.click(at)?;
    fx.wait("file undo", |_| {
        fs::read_to_string(root.join("src/math.py")).is_ok_and(|text| text == before)
    })?;
    ensure!(
        fx.read(|r, cx| r.thread().is_some_and(|v| !v.read(cx).turns().is_empty())),
        "Undo removed the turn"
    );
    fx.wait("undo result", |fx| {
        fx.read(|r, cx| r.toasts(cx).iter().any(|text| text == "Changes reverted"))
    })?;
    fx.click(at)?;
    fx.wait("file reapply", |_| {
        fs::read_to_string(root.join("src/math.py")).is_ok_and(|text| text == after)
    })?;
    fx.wait("reapply result", |fx| {
        fx.read(|r, cx| r.toasts(cx).iter().any(|text| text == "Changes reapplied"))
    })?;
    ensure!(
        fs::read(root.join(".git/index"))? == index,
        "the real index changed"
    );
    ensure!(fs::read_to_string(root.join("keep.txt"))? == "unrelated\n");
    fs::write(root.join("src/math.py"), "local edit\n")?;
    fx.click(at)?;
    fx.wait("failed undo", |fx| {
        fx.read(|r, cx| {
            r.toasts(cx)
                .iter()
                .any(|text| text == "Failed to revert changes")
        })
    })?;
    ensure!(fs::read_to_string(root.join("src/math.py"))? == "local edit\n");
    fs::write(root.join("src/math.py"), &after)?;
    fx.click(at)?;
    fx.wait("retry undo", |_| {
        fs::read_to_string(root.join("src/math.py")).is_ok_and(|text| text == before)
    })?;
    ensure!(fx.route() == Route::Thread(SUBTRACT.into()));
    Ok(String::new())
}

/// Center of the approval card's `Allow once` pill (`button_primary_solid`, white in dark):
/// the first run of its color, from the top, on a vertical line 6 px inside the pill's right
/// end (the card ends 12 px inside the 736 px content column's right edge, x 1176). Runs of
/// at least 18 CSS px skip glyph stems; the composer's send button is lower.
fn allow_once(shot: &RgbaImage) -> Result<(f32, f32)> {
    let x = 1158.;
    let c = gpui::Rgba::from(theme::Theme::dark().button_primary_solid);
    let want = [c.r, c.g, c.b].map(|v| (v * 255.).round() as i32);
    let hit = |y: u32| {
        let p = shot.get_pixel((x * 2.) as u32, y);
        (0..3).all(|i| (p[i] as i32 - want[i]).abs() <= 2)
    };
    let mut run = 0;
    for y in 0..shot.height() {
        if hit(y) {
            run += 1;
        } else if run >= 36 {
            return Ok((x - 4., (y as f32 - run as f32 / 2.) / 2.));
        } else {
            run = 0;
        }
    }
    anyhow::bail!("no Allow once pill at x {x}")
}

/// A command approval request (crafted from `CommandApprovalRequest`: the sessions ran with
/// `approvalPolicy: never`) arrives with the greeting turn; `Allow once` answers it, and the
/// replay's reply to that answer (matched by request id) reaches the thread. The turn ends
/// after the answer, because the end of a turn withdraws its pending requests.
fn approval_round_trip() -> Result<String> {
    const TURN: &str = "01a13de9-c4f9-7370-99bd-0512afdf8126";
    let mut lines = script(Some("sessions/greeting.jsonl"), |_| {})?;
    let end = lines
        .iter()
        .rposition(|v| v["msg"]["method"] == "turn/completed")
        .context("no turn/completed")?;
    let end = lines.remove(end);
    let reply = lines
        .iter()
        .position(|v| v["dir"] == "recv" && v["msg"]["result"]["turn"]["id"] == TURN)
        .context("no turn/start reply")?;
    let params = json!({
        "threadId": GREETING, "turnId": TURN, "itemId": "call-approval",
        "command": "python3 hello.py", "cwd": CWD, "reason": "Run the edited script",
    });
    let method = "item/commandExecution/requestApproval";
    let request = json!({ "id": "approval-1", "method": method, "params": params });
    lines.insert(reply + 1, json!({ "dir": "recv", "msg": request }));
    let answer = json!({ "id": "approval-1", "result": { "decision": "accept" } });
    lines.push(json!({ "dir": "send", "msg": answer }));
    let item = json!({ "type": "agentMessage", "id": "approval-ack", "text": "Accepted." });
    let ack = json!({ "threadId": GREETING, "turnId": TURN, "item": item });
    lines.push(json!({ "dir": "recv", "msg": { "method": "item/completed", "params": ack } }));
    lines.push(end);
    let mut fx = Fx::scripted(lines)?;
    fx.type_text(GREETING_PROMPT)?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let composer = fx.composer();
    let pending = |fx: &Fx| fx.read(|_, cx| composer.read(cx).approval().is_some());
    fx.wait("the approval card", |fx| pending(fx))?;
    ensure!(
        !has_item(&fx, "approval-ack"),
        "acknowledged before the answer"
    );
    fx.hover((640., 22.))?;
    let at = allow_once(&fx.shot()?)?;
    fx.click(at)?;
    ensure!(!pending(&fx), "the card stayed after Allow once at {at:?}");
    fx.wait("the answer's ack", |fx| has_item(fx, "approval-ack"))?;
    Ok(String::new())
}

// --- Sidebar, shortcuts ---

/// A Recents row's `Archive chat`, then `⌘⇧A` on the open chat: `thread/archive`, the row
/// leaves Recents, the open chat leaves for Home.
fn archive_chat() -> Result<String> {
    let mut lines = script(None, |_| {})?;
    for _ in 0..2 {
        lines.extend(exchange("thread/archive", json!({})));
    }
    let mut fx = Fx::scripted(lines)?;
    // The row's actions end 8 px inside its right edge (x 332): Archive is the last, 20 px.
    let row = fx.at("home", "Change the greeting")?;
    fx.click((314., row.1))?;
    fx.wait("thread/archive", |fx| {
        fx.read(|r, _| r.recents() == [SUBTRACT, EXPLAIN])
    })?;
    ensure!(fx.route() == Route::Home, "{:?}", fx.route());
    fx.keys("cmd-1")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-shift-a")?;
    fx.wait("cmd-shift-a", |fx| fx.read(|r, _| r.recents() == [EXPLAIN]))?;
    ensure!(fx.route() == Route::Home, "{:?}", fx.route());
    Ok(String::new())
}

/// `⌘⌥1`…`⌘⌥6` follow the last update, not the Recents order: the explain chat is made the
/// newest.
fn recent_chat_keys() -> Result<String> {
    let mut fx = Fx::with("dark", (1280., 832.), None, |v| {
        if let Some(data) = v
            .pointer_mut("/msg/result/data")
            .and_then(Value::as_array_mut)
        {
            for t in data.iter_mut().filter(|t| t["id"] == EXPLAIN) {
                t["updatedAt"] = 1_792_050_000.into();
            }
        }
    })?;
    fx.keys("cmd-alt-1")?;
    fx.wait_thread(EXPLAIN)?;
    fx.keys("cmd-alt-2")?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-alt-6")?;
    ensure!(
        fx.route() == Route::Thread(GREETING.into()),
        "cmd-alt-6 moved"
    );
    Ok(String::new())
}

// --- Config writes ---

/// The effort slider's last dot: `config/batchWrite` of the model and effort; the app keeps
/// the written values once the server took them.
fn effort_pick_writes_config() -> Result<String> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/batchWrite", json!({ "status": "ok" })));
    let mut fx = Fx::scripted(lines)?;
    let before = fx.read(|r, _| r.config()["model_reasoning_effort"].clone());
    fx.click(fx.at("home", "GPT-6 Astra")?)?;
    // refs/composer: the menu's effort label; the 230 px track starts 127 - 12 px left of
    // its center, 52 px lower, dots at 13 + 51·i.
    let (x, y) = fx.at("composer", "Light")?;
    fx.click((x - 115. + 13. + 51. * 4., y + 51.75))?;
    fx.wait("config/batchWrite", |fx| {
        fx.read(|r, _| r.config()["model_reasoning_effort"] != before)
    })?;
    let (model, effort) = fx.read(|r, _| {
        let c = r.config();
        (c["model"].clone(), c["model_reasoning_effort"].clone())
    });
    ensure!(model.is_string() && effort.is_string(), "{model} {effort}");
    Ok(format!("{before} -> {effort}"))
}

/// General ▸ Default terminal location ▸ `Right`: `config/value/write`; leaving and reopening
/// Settings shows the written value.
fn settings_value_round_trip() -> Result<String> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/value/write", json!({ "status": "ok" })));
    let mut fx = Fx::scripted(lines)?;
    let open = |fx: &mut Fx| -> Result<RgbaImage> {
        fx.keys("cmd-,")?;
        fx.scroll((800., 400.), -300.)?;
        fx.hover((640., 22.))?;
        fx.shot()
    };
    let before = open(&mut fx)?;
    let right = fx.at("settings", "Right")?;
    fx.click((right.0, right.1 - 300.))?;
    fx.wait("config/value/write", |fx| {
        fx.read(|r, _| r.config()["desktop"]["defaultTerminalLocation"] == "right")
    })?;
    fx.hover((640., 22.))?;
    let set = fx.shot()?;
    let n = changed(&before, &set, MAIN_PANEL);
    ensure!(n > 50, "the segment repainted {n} px");
    fx.keys("cmd-[")?;
    ensure!(fx.route() == Route::Home);
    let left = changed(&set, &open(&mut fx)?, MAIN_PANEL);
    ensure!(left < 50, "reopened Settings differ by {left} px");
    Ok(format!("{n} px"))
}

// --- Scrolling ---

/// A window short enough that readback's sidebar sections (Projects, then Recents' 3 rows)
/// overflow their scroller.
const SHORT: (f32, f32) = (1280., 320.);
/// In the sidebar sections, between Projects and Recents.
const SIDEBAR_LIST: (f32, f32) = (196., 250.);
/// The original's sidebar sections scroller at [`SHORT`], recorded with tools/route1 (headless
/// Chromium, readback's `thread/list`): `scrollHeight` 233 - `clientHeight` 151, and a wheel
/// past the end leaves `scrollTop` there.
const ORIGINAL_SIDEBAR_MAX: f32 = 82.;

/// The sidebar scroller's offset down the list and its largest offset, CSS px.
fn sidebar_scroll(fx: &Fx) -> (f32, f32) {
    fx.read(|r, cx| {
        let scroll = &r.sidebar().read(cx).scroll;
        (
            -f32::from(scroll.offset().y),
            f32::from(scroll.max_offset().y),
        )
    })
}

/// The sidebar sections scroll as far as the original's and stop at both ends.
fn scroll_sidebar_extent() -> Result<String> {
    let mut fx = Fx::with("dark", SHORT, None, |_| {})?;
    let (_, max) = sidebar_scroll(&fx);
    ensure!(
        max == ORIGINAL_SIDEBAR_MAX,
        "max offset {max}, original {ORIGINAL_SIDEBAR_MAX}"
    );
    fx.scroll(SIDEBAR_LIST, -1000.)?;
    let end = sidebar_scroll(&fx).0;
    ensure!(end == max, "past the end: {end} px");
    fx.scroll(SIDEBAR_LIST, 1000.)?;
    let top = sidebar_scroll(&fx).0;
    ensure!(top == 0., "past the top: {top} px");
    Ok(format!("max {max} px"))
}

/// A trackpad flick keeps gliding on its momentum events after release and stops at the end;
/// a mouse wheel scrolls by lines.
fn scroll_sidebar_momentum() -> Result<String> {
    let mut fx = Fx::with("dark", SHORT, None, |_| {})?;
    let at = SIDEBAR_LIST;
    let pixels = |dy: f32| ScrollDelta::Pixels(point(px(0.), px(dy)));
    fx.hover(at)?;
    fx.wheel(at, pixels(0.), TouchPhase::Started)?;
    for dy in [-4., -8.] {
        fx.wheel(at, pixels(dy), TouchPhase::Moved)?;
    }
    fx.wheel(at, pixels(0.), TouchPhase::Ended)?;
    let released = sidebar_scroll(&fx).0;
    ensure!(released == 12., "under the finger: {released} px");
    for dy in [-10., -6., -3., -1.] {
        fx.wheel(at, pixels(dy), TouchPhase::Moved)?;
    }
    let glided = sidebar_scroll(&fx).0;
    ensure!(glided == 32., "after the momentum events: {glided} px");
    for _ in 0..20 {
        fx.wheel(at, pixels(-10.), TouchPhase::Moved)?;
    }
    let (end, max) = sidebar_scroll(&fx);
    ensure!(end == max, "momentum past the end: {end} of {max} px");
    fx.wheel(at, ScrollDelta::Lines(point(0., 1.)), TouchPhase::Moved)?;
    let line = end - sidebar_scroll(&fx).0;
    ensure!(line > 0., "a wheel line up moved {line} px");
    Ok(format!(
        "released {released}, glided {glided}, line {line} px"
    ))
}

/// Wheels the pane `rect` at `at`: the first wheel repaints it, a wheel past the end leaves it
/// still (a further wheel repaints nothing), and a wheel back up shows the start again.
fn assert_scrolls(fx: &mut Fx, at: (f32, f32), rect: [f32; 4]) -> Result<String> {
    fx.hover(at)?;
    let start = fx.shot()?;
    fx.scroll(at, -100.)?;
    let moved = changed(&start, &fx.shot()?, rect);
    ensure!(moved > 0, "a wheel at {at:?} repainted nothing");
    fx.scroll(at, -100_000.)?;
    let end = fx.shot()?;
    fx.scroll(at, -100.)?;
    let past = changed(&end, &fx.shot()?, rect);
    ensure!(past == 0, "a wheel past the end repainted {past} px");
    fx.scroll(at, 100_000.)?;
    let back = changed(&start, &fx.shot()?, rect);
    ensure!(back == 0, "back at the top, {back} px differ");
    Ok(format!("{moved} px"))
}

/// Opens `reach` at 1280x832, then shrinks the window to `h` px: at 832 px readback's thread,
/// diff and Scheduled page fit their panes, as they do in the original's refs.
fn short(h: f32, reach: fn(&mut Fx) -> Result<()>) -> Result<Fx> {
    let mut fx = Fx::new()?;
    reach(&mut fx)?;
    fx.cx
        .simulate_window_resize(fx.window, size(px(fx.size.0), px(h)));
    fx.cx.run_until_parked();
    Ok(fx)
}

fn scroll_thread() -> Result<String> {
    let mut fx = short(SHORT.1, |fx| {
        fx.click(fx.at("home", "Add a subtract")?)?;
        fx.wait_thread(SUBTRACT)
    })?;
    // Above the docked composer.
    assert_scrolls(&mut fx, (640., 120.), [340., 44., 936., 150.])
}

fn scroll_diff() -> Result<String> {
    // The subtract diff's 5 lines overflow a 200 px window only.
    let h = 200.;
    let mut fx = short(h, open_changes)?;
    let left = panel_left(fx.size.0);
    let panel = [left, 44., fx.size.0 - left, h - 48.];
    assert_scrolls(&mut fx, (left + 200., 150.), panel)
}

/// The section list in the sidebar and the page.
fn scroll_settings() -> Result<String> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-,")?;
    let nav = assert_scrolls(&mut fx, (196., 400.), SIDEBAR_PANEL)?;
    let page = assert_scrolls(&mut fx, (800., 400.), MAIN_PANEL)?;
    Ok(format!("nav {nav}, page {page}"))
}

fn scroll_automations() -> Result<String> {
    let mut fx = short(SHORT.1, |fx| fx.click(RAIL_SCHEDULED))?;
    assert_scrolls(&mut fx, (800., 200.), [340., 44., 936., SHORT.1 - 48.])
}

// --- Window ---

/// Live resizes across the 768 px breakpoint: the sidebar hides below it and comes back as
/// the user left it.
fn resize_live() -> Result<String> {
    let mut fx = Fx::new()?;
    let resize = |fx: &mut Fx, w: f32| {
        fx.cx
            .simulate_window_resize(fx.window, size(px(w), px(832.)));
        fx.cx.run_until_parked();
    };
    for (w, open) in [(767., false), (1280., true), (600., false), (1440., true)] {
        resize(&mut fx, w);
        ensure!(fx.sidebar_open() == open, "{w} px: sidebar_open {}", !open);
    }
    fx.keys("cmd-b")?;
    resize(&mut fx, 767.);
    resize(&mut fx, 1280.);
    ensure!(!fx.sidebar_open(), "the closed sidebar came back");
    Ok(String::new())
}
