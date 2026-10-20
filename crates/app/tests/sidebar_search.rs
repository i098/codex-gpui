#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, SUBTRACT, open_recorded_sidebar_search};
use serde_json::{Value, json};
use std::time::Duration;

fn cloud_success() -> Value {
    // A schema-level success checks consumption and navigation. Parity uses only recorded data.
    json!({"value": {"response": {"__response": {
        "status": 200,
        "headers": [["content-type", "application/json"]],
        "body": json!({"items": [{
            "title": "math",
            "snippet": "A cloud math discussion",
            "match_kind": "content",
            "update_time": 1792040102,
            "payload": {"conversation_id": "cloud/chat", "message_id": null}
        }], "cursor": null}).to_string()
    }}}})
}

#[test]
fn local_results_survive_cloud_failure_and_retry_stays_in_the_menu() -> Result<()> {
    let mut fx = Fx::with(
        "dark",
        (1280., 832.),
        Some("sessions/sidebar-math-search.jsonl"),
        |_| {},
    )?;
    let host = open_recorded_sidebar_search(&mut fx)?;
    let menu = fx
        .read(|root, _| root.command_menu().cloned())
        .context("menu")?;
    let chats = |fx: &Fx| {
        fx.read(|_, cx| {
            menu.read(cx)
                .describe()
                .into_iter()
                .find(|(heading, _)| *heading == "Chats")
                .map(|(_, rows)| rows)
                .unwrap_or_default()
        })
    };
    let local = chats(&fx);
    for seconds in [1, 2, 4] {
        fx.advance(Duration::from_secs(seconds))?;
    }
    ensure!(fx.read(|_, cx| menu.read(cx).search_pending()) == (false, false));
    ensure!(chats(&fx) == local, "cloud failure removed a local hit");
    ensure!(
        host.requests.lock().unwrap().len() == 4,
        "original retry sequence changed"
    );
    fx.shot()?
        .save(std::env::temp_dir().join("sidebar-cloud-error.png"))?;

    let rows = fx.read(|_, cx| {
        menu.read(cx)
            .describe()
            .iter()
            .map(|(_, rows)| rows.len())
            .sum::<usize>()
    });
    for _ in 0..rows {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    ensure!(
        fx.read(|root, _| root.command_menu().is_some()),
        "retry closed the menu"
    );
    ensure!(fx.read(|_, cx| menu.read(cx).search_pending()) == (false, true));
    ensure!(
        host.requests.lock().unwrap().len() == 5,
        "the retry control did not search again"
    );
    fx.keys("escape")?;
    fx.advance(Duration::from_secs(8))?;
    ensure!(
        host.requests.lock().unwrap().len() == 5,
        "a dismissed search sent another retry"
    );
    Ok(())
}

#[test]
fn cloud_results_preserve_local_selection_and_open_their_canonical_destination() -> Result<()> {
    let mut fx = Fx::with(
        "dark",
        (1280., 832.),
        Some("sessions/sidebar-math-search.jsonl"),
        |_| {},
    )?;
    let host = open_recorded_sidebar_search(&mut fx)?;
    *host.response.lock().unwrap() = cloud_success();
    fx.advance(Duration::from_secs(1))?;
    let menu = fx
        .read(|root, _| root.command_menu().cloned())
        .context("menu")?;
    ensure!(fx.read(|_, cx| menu.read(cx).search_pending()) == (false, false));
    fx.reference_shot()?
        .save(std::env::temp_dir().join("sidebar-cloud-results.png"))?;
    fx.keys("enter")?;
    fx.wait_thread(SUBTRACT)?;

    fx.click(common::SEARCH)?;
    fx.type_text("math")?;
    fx.advance(ui::screens::sidebar::SEARCH_DEBOUNCE)?;
    fx.keys("up enter")?;
    ensure!(
        fx.cx.take_opened_url().as_deref() == Some("https://chatgpt.com/c/cloud%2Fchat"),
        "the cloud result did not open its encoded ChatGPT destination"
    );
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}
