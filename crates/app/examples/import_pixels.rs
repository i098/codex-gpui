#[path = "../tests/common/mod.rs"]
mod common;

use anyhow::{Context as _, Result};
use bridge::imports::{DetectedItem, Histories, Policy};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};
use ui::screens::settings::{computer_use::Load, imports::ImportState};

fn main() -> Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let input = std::env::var("CGX_IMPORT_FIXTURE").context("CGX_IMPORT_FIXTURE")?;
    let output = PathBuf::from(std::env::var("CGX_IMPORT_SHOTS").context("CGX_IMPORT_SHOTS")?);
    let fixture: Value = serde_json::from_slice(&std::fs::read(input)?)?;
    let items: Vec<DetectedItem> = serde_json::from_value(fixture["detection"]["items"].clone())?;
    let history: Histories = serde_json::from_value(fixture["history"].clone())?;
    let state = ImportState {
        policy: Policy::from_evaluations(&fixture["policy"]),
        detected: Load::Ready(Arc::new(items)),
        histories: Load::Ready(Arc::new(history.data)),
        mcp_auth: Load::Ready(Arc::new(Vec::new())),
        sync_state: serde_json::from_value(fixture["syncState"].clone())?,
        preferences: serde_json::from_value(fixture["preferences"].clone())?,
        sync_enabled: fixture["syncEnabled"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    let mut fx = common::Fx::with("dark", (1280., 832.), None, |_| {})?;
    fx.cx.update(|cx| cx.set_global(state));
    fx.keys("cmd-k")?;
    fx.type_text("Import")?;
    let index = fx
        .read(|root, cx| {
            root.command_menu()
                .unwrap()
                .read(cx)
                .describe()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|(text, _)| {
                    text.split(" | ").next().unwrap().replace(['[', ']'], "") == "Import"
                })
        })
        .context("Import command")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    fx.hover((0., 0.))?;
    fx.advance(Duration::from_millis(500))?;
    std::fs::create_dir_all(&output)?;
    fx.shot()?.save(output.join("import.png"))?;
    if let Some(actions) = fixture["actions"].as_array() {
        for action in actions {
            if let Some(keys) = action["keys"].as_str() {
                fx.keys(keys)?;
            } else {
                fx.click((
                    action["x"].as_f64().context("action x")? as f32,
                    action["y"].as_f64().context("action y")? as f32,
                ))?;
            }
            fx.hover((0., 0.))?;
            fx.advance(Duration::from_millis(500))?;
            fx.shot()?.save(output.join(format!(
                "{}.png",
                action["name"].as_str().context("action name")?
            )))?;
        }
    }
    println!("native Import frames saved to {}", output.display());
    Ok(())
}
