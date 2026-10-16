//! Replays CP-110 from the original renderer recording.

mod common;

use anyhow::{Context as _, Result};
use bridge::types::TurnStatus;
use common::{Fx, GREETING, ROOT, exchange, proxy_child, script};
use serde_json::{Value, json};
use std::{fs, path::Path};
use ui::screens::settings::AppServer;

#[test]
fn proxy() {
    proxy_child();
}

fn notify(fx: &mut Fx, method: &'static str) {
    fx.cx.update(|cx| {
        let bridge = cx.global::<AppServer>().0.clone();
        cx.spawn(async move |_| {
            bridge.request::<_, Value>(method, json!({})).await.unwrap();
        })
        .detach();
    });
}

#[test]
fn stop_interrupts_only_the_active_turn() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s023.json"))?)?;
    let started = &recording["steps"][1]["message"];
    let completed = &recording["steps"][3]["message"];
    let mut lines = script(None, |_| {})?;
    for (method, notification) in [("s023/start", started), ("s023/complete", completed)] {
        lines.extend(exchange(method, json!({})));
        lines.push(json!({"dir":"recv", "msg":{
            "method":notification["method"], "params":notification["params"]
        }}));
    }
    let mut interrupt = exchange("turn/interrupt", json!({}));
    interrupt[1]["msg"]
        .as_object_mut()
        .unwrap()
        .remove("result");
    interrupt[1]["msg"]["error"] =
        json!({"code":-32000,"message":"route1: turn/interrupt is not allowed"});
    lines.extend(interrupt);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.shot()?;
    fx.click(fx.at("home", "Change the greeting")?)?;
    fx.wait_thread(GREETING)?;
    notify(&mut fx, "s023/start");
    fx.wait("running turn", |fx| {
        fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .unwrap()
                .status
                == TurnStatus::InProgress
        })
    })?;
    let bounds = &recording["steps"][2]["target"]["bbox"];
    let x = bounds["x"].as_f64().context("stop x")? as f32;
    let y = bounds["y"].as_f64().context("stop y")? as f32;
    let at = (
        x + bounds["w"].as_f64().unwrap() as f32 / 2.,
        y + bounds["h"].as_f64().unwrap() as f32 / 2.,
    );
    let frame = fx.shot()?;
    assert_eq!(
        frame
            .get_pixel(((x + 3.) * 2.) as u32, (at.1 * 2.) as u32)
            .0,
        [255, 255, 255, 255]
    );
    assert_eq!(
        frame
            .get_pixel(((at.0 - 3.) * 2.) as u32, ((at.1 - 3.) * 2.) as u32)
            .0,
        [0, 0, 0, 255]
    );
    frame.save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("s023-running.png"))?;
    fx.click(at)?;
    fx.wait("interrupt error toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|s| s.as_ref() == "Failed to stop chat")
        })
    })?;
    let expected = &recording["result"]["requests"][0];
    let requests: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "turn/interrupt")
        .collect();
    assert_eq!(
        requests,
        vec![json!({"method":expected["method"],"params":expected["params"]})]
    );
    assert!(fx.composer_focused()?);
    notify(&mut fx, "s023/complete");
    fx.wait("interrupted turn", |fx| {
        fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .unwrap()
                .status
                == TurnStatus::Interrupted
        })
    })?;
    fx.shot()?
        .save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("s023-interrupted.png"))?;
    fx.click(at)?;
    assert_eq!(
        fx.sent()?
            .into_iter()
            .filter(|r| r["method"] == "turn/interrupt")
            .count(),
        1
    );
    Ok(())
}
