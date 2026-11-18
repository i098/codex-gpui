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

/// Moves the selection from the first row down to the first one `pick` accepts (its
/// heading, index in the group and recorder text): 26.1029 lists commands before `Chats`.
fn select_row(fx: &mut Fx, pick: impl Fn(&str, usize, &str) -> bool) -> Result<()> {
    let menu = fx
        .read(|root, _| root.command_menu().cloned())
        .context("menu")?;
    let index = fx
        .read(|_, cx| {
            menu.read(cx)
                .describe()
                .into_iter()
                .flat_map(|(heading, rows)| {
                    rows.into_iter()
                        .enumerate()
                        .map(move |(i, (text, _))| (heading, i, text))
                })
                .position(|(heading, i, text)| pick(heading, i, &text))
        })
        .context("row to select")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    Ok(())
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
    select_row(&mut fx, |_, _, row| row.starts_with("Add a subtract"))?;
    fx.keys("enter")?;
    fx.wait_thread(SUBTRACT)?;

    fx.click(common::SEARCH)?;
    fx.type_text("math")?;
    fx.advance(ui::screens::sidebar::SEARCH_DEBOUNCE)?;
    select_row(&mut fx, |_, _, row| row.contains("A cloud"))?;
    fx.keys("enter")?;
    ensure!(
        fx.cx.take_opened_url().as_deref() == Some("https://chatgpt.com/c/cloud%2Fchat"),
        "the cloud result did not open its encoded ChatGPT destination"
    );
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}

#[test]
fn a_cloud_reply_does_not_displace_the_selected_ninth_local_chat() -> Result<()> {
    // Boundary-only data; the parity cases retain their genuine recorded inputs.
    let mut fx = Fx::with("dark", (1280., 832.), None, |line| {
        if line["msg"]["result"]["data"][0].get("preview").is_some() {
            let prototype = line["msg"]["result"]["data"][0].clone();
            line["msg"]["result"]["data"] = Value::Array(
                (0..9)
                    .map(|index| {
                        let mut thread = prototype.clone();
                        thread["id"] = json!(format!("local-{index}"));
                        thread["name"] = json!("math");
                        thread["preview"] = json!("math");
                        thread["updatedAt"] = json!(1_792_040_101 - index);
                        thread
                    })
                    .collect(),
            );
        }
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["id"] = json!("local-8");
        }
    })?;
    let host = std::sync::Arc::new(common::SidebarSearchHost::recorded()?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.click(common::SEARCH)?;
    fx.type_text("math")?;
    fx.advance(ui::screens::sidebar::SEARCH_DEBOUNCE)?;
    select_row(&mut fx, |heading, i, _| heading == "Chats" && i == 8)?;
    *host.response.lock().unwrap() = cloud_success();
    fx.advance(Duration::from_secs(1))?;
    fx.keys("enter")?;
    fx.wait_thread("local-8")?;
    Ok(())
}

#[test]
fn enabling_cloud_search_during_debounce_sends_one_request() -> Result<()> {
    let mut fx = Fx::with(
        "dark",
        (1280., 832.),
        Some("sessions/sidebar-math-search.jsonl"),
        |_| {},
    )?;
    fx.click(common::SEARCH)?;
    let menu = fx
        .read(|root, _| root.command_menu().cloned())
        .context("menu")?;
    fx.type_text("math")?;
    let host = std::sync::Arc::new(common::SidebarSearchHost::recorded()?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    menu.update(&mut fx.cx, |menu, cx| menu.enable_cloud_search(cx));
    fx.advance(ui::screens::sidebar::SEARCH_DEBOUNCE)?;
    fx.wait("local reply and pending cloud search", |fx| {
        fx.read(|_, cx| menu.read(cx).search_pending()) == (false, true)
    })?;
    ensure!(
        host.requests.lock().unwrap().len() == 1,
        "cloud query started twice"
    );
    Ok(())
}

#[test]
fn a_pending_search_with_no_local_matches_keeps_a_visible_loader() -> Result<()> {
    use image::GenericImageView as _;

    let mut fx = Fx::with(
        "dark",
        (1280., 832.),
        Some("sessions/sidebar-math-search.jsonl"),
        |_| {},
    )?;
    let host = std::sync::Arc::new(common::SidebarSearchHost::recorded()?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.click(common::SEARCH)?;
    fx.type_text("xyzzymissing")?;
    let menu = fx
        .read(|root, _| root.command_menu().cloned())
        .context("menu")?;
    fx.advance(ui::screens::sidebar::SEARCH_DEBOUNCE)?;
    fx.wait("empty local reply and pending cloud search", |fx| {
        fx.read(|_, cx| menu.read(cx).search_pending()) == (false, true)
    })?;
    let frame = fx.reference_shot()?;
    frame.save(std::env::temp_dir().join("sidebar-cloud-empty.png"))?;
    let (low, high) = frame
        .view(1262, 442, 36, 36)
        .pixels()
        .fold((u8::MAX, 0), |(low, high), (_, _, pixel)| {
            (low.min(pixel[0]), high.max(pixel[0]))
        });
    ensure!(
        high.saturating_sub(low) > 32,
        "the centered loading indicator is not visible"
    );
    Ok(())
}
