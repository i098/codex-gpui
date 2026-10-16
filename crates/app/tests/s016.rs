mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, GREETING, ROOT, changed};
use serde_json::Value;
use std::{fs, time::Duration};

fn center(value: &Value) -> (f32, f32) {
    let n = |key| value[key].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn header_interactions_match_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s016.json"))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let row = fx.at("home", "Change the greeting")?;
    fx.click(row)?;
    fx.wait_thread(GREETING)?;
    fx.hover((800., 400.))?;
    let before = fx.shot()?;
    assert_eq!(
        fx.read(|r, cx| r.thread().unwrap().read(cx).header_title())
            .as_ref(),
        recording["result"]["title"]["text"].as_str().unwrap()
    );
    let sent = fx.sent()?.len();
    fx.click(center(&recording["steps"][1]["target"]["bbox"]))?;
    let opened = fx.shot()?;
    opened.save(format!("{}/s016-menu.png", env!("CARGO_TARGET_TMPDIR")))?;
    let scale = opened.width() as f64 / 1280.;
    for sample in recording["result"]["menuPixels"]
        .as_array()
        .context("menu pixels")?
    {
        let x = (sample["x"].as_f64().unwrap() * scale) as u32;
        let y = (sample["y"].as_f64().unwrap() * scale) as u32;
        let rgba: [u8; 4] = serde_json::from_value(sample["rgba"].clone())?;
        assert_eq!(opened.get_pixel(x, y).0, rgba, "menu fill at {x}, {y}");
    }
    let labels = fx.read(|r, cx| {
        r.thread()
            .unwrap()
            .read(cx)
            .chat_action_labels()
            .map(|items| items.collect::<Vec<_>>())
    });
    let expected = recording["result"]["menu"]["items"]
        .as_array()
        .context("menu items")?
        .iter()
        .map(|item| item["text"].as_str().unwrap().lines().next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(labels, Some(expected));
    let requests = fx
        .sent()?
        .into_iter()
        .skip(sent)
        .filter(|r| {
            r["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("thread/"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        requests,
        recording["result"]["requests"].as_array().unwrap().clone()
    );
    ensure!(
        changed(&before, &opened, [1053., 38., 220., 301.]) > 10000,
        "Chat actions did not open the recorded menu"
    );
    fx.keys("escape")?;
    fx.hover((800., 400.))?;
    let closed = fx.shot()?;
    ensure!(
        changed(&before, &closed, [1053., 38., 220., 301.]) == 0,
        "Escape did not close Chat actions"
    );
    fx.click(center(&recording["steps"][1]["target"]["bbox"]))?;
    fx.shot()?;
    fx.click((800., 400.))?;
    fx.shot()?;
    assert!(fx.read(|r, cx| r.thread().unwrap().read(cx).chat_action_labels().is_none()));
    for observation in recording["result"]["tooltips"]
        .as_array()
        .context("tooltips")?
    {
        let step = &recording["steps"][observation["afterStep"].as_u64().unwrap() as usize];
        fx.hover((800., 400.))?;
        fx.cx.advance_clock(Duration::from_millis(500));
        fx.cx.run_until_parked();
        fx.shot()?;
        fx.hover(center(&step["target"]["bbox"]))?;
        fx.cx.advance_clock(Duration::from_millis(500));
        fx.cx.run_until_parked();
        let image = fx.shot()?;
        image.save(format!("{}/s016-tooltip.png", env!("CARGO_TARGET_TMPDIR")))?;
        let shown = fx.cx.update(|cx| ui::tooltip::shown(cx).cloned());
        let expected = observation["tooltip"].as_array().context("tooltip list")?;
        if expected.is_empty() {
            ensure!(shown.is_none(), "Unexpected header tooltip: {shown:?}");
        } else {
            let shown = shown.context("The summary tooltip did not open")?;
            assert_eq!(shown.label.as_ref(), expected[0]["text"].as_str().unwrap());
            assert!(shown.shortcut.is_none());
            let bbox = &expected[0]["bbox"];
            assert!(
                (f32::from(shown.bounds.left()) - bbox["x"].as_f64().unwrap() as f32).abs() < 2.
            );
            assert!(
                (f32::from(shown.bounds.top()) - bbox["y"].as_f64().unwrap() as f32).abs() < 2.
            );
            let scale = image.width() as f32 / 1280.;
            let x = ((bbox["x"].as_f64().unwrap() as f32 + 16.) * scale) as u32;
            let y = ((bbox["y"].as_f64().unwrap() as f32 + 8.) * scale) as u32;
            assert_eq!(&image.get_pixel(x, y).0[..3], &[45, 45, 45]);
        }
    }
    let row = fx.at("home", "Add a subtract")?;
    fx.click(row)?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.shot()?;
    assert_eq!(
        fx.read(|r, cx| r.thread().unwrap().read(cx).header_title())
            .as_ref(),
        recording["result"]["longTitle"]["text"].as_str().unwrap()
    );
    Ok(())
}
