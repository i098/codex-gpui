//! Replays S001 against the original renderer's isolated fixture recording.
//! Pin click remains deferred under firstmate instruction 001.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::{Fx, ROOT, changed, exchange, proxy_child, script};
use serde_json::{Value, json};
use std::{fs, time::Duration};

fn center(b: &Value) -> (f32, f32) {
    let n = |key: &str| b[key].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}

fn css_pixel(image: &image::RgbaImage, at: &Value) -> [f32; 3] {
    let scale = image.width() / 1280;
    let x = at["x"].as_u64().unwrap() as u32 * scale;
    let y = at["y"].as_u64().unwrap() as u32 * scale;
    std::array::from_fn(|channel| {
        (0..scale)
            .flat_map(|dx| {
                (0..scale).map(move |dy| image.get_pixel(x + dx, y + dy).0[channel] as u32)
            })
            .sum::<u32>() as f32
            / (scale * scale) as f32
    })
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn unpin_then_archive_does_not_open_the_chat() -> Result<()> {
    let rec: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s001.json"))?)?;
    let id = rec["initial"]["pinnedThread"].as_str().unwrap();
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(threads) = line
            .pointer_mut("/msg/result/data")
            .and_then(Value::as_array_mut)
        {
            for thread in threads {
                if thread["id"] == id {
                    thread["section"] = json!({"id": rec["initial"]["sectionId"]});
                }
            }
        }
    }
    lines.extend(exchange("thread/section/move", json!({})));
    lines.extend(exchange("thread/archive", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let initial = fx.shot()?;
    let steps = rec["steps"].as_array().unwrap();
    fx.click(center(&steps[0]["target"]["bbox"]))?;
    fx.wait("unpin request", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|r| r["method"] == "thread/section/move"))
    })?;
    fx.advance(Duration::from_millis(300))?;
    ensure!(fx.route() == Route::Home, "Unpin opened the chat");
    ensure!(
        fx.read(|r, cx| r.toasts(cx)).is_empty(),
        "Unpin added an unrecorded toast"
    );
    let unpinned = fx.shot()?;
    ensure!(
        changed(&initial, &unpinned, [60., 203., 272., 30.]) > 100,
        "Pinned row did not leave its section"
    );

    fx.hover(center(&steps[1]["target"]["bbox"]))?;
    fx.advance(Duration::from_millis(150))?;
    let rest = fx.shot()?;
    fx.hover(center(&steps[2]["target"]["bbox"]))?;
    fx.advance(Duration::from_millis(50))?;
    let fading = fx.shot()?;
    fx.advance(Duration::from_millis(100))?;
    let visible = fx.shot()?;
    let archive_button = [304., 295., 20., 20.];
    ensure!(
        changed(&rest, &fading, archive_button) > 0,
        "The Archive button did not fade on hover"
    );
    ensure!(
        changed(&fading, &visible, archive_button) > 0,
        "The Archive button skipped its 150 ms transition"
    );
    fx.click(center(&steps[3]["target"]["bbox"]))?;
    fx.wait("archive toast", |fx| {
        fx.read(|r, cx| r.toasts(cx))
            .iter()
            .any(|t| t.as_ref() == "Archived chat")
    })?;
    fx.advance(Duration::from_millis(1000))?;
    ensure!(fx.route() == Route::Home, "Archive opened the chat");
    assert_eq!(
        json!(fx.read(|r, _| r.recents())),
        rec["result"]["state"]["recents"]
    );

    let sent = fx.sent()?;
    let mutations: Vec<_> = sent
        .iter()
        .filter(|r| {
            matches!(
                r["method"].as_str(),
                Some("thread/section/move" | "thread/archive")
            )
        })
        .cloned()
        .collect();
    let unpin = &rec["result"]["requests"][0]["params"];
    let archive = &rec["result"]["requests"][1];
    assert_eq!(
        mutations,
        vec![
            json!({"method":"thread/section/move","params":{"threadId":unpin["threadId"],"sectionId":null,"beforeThreadId":null}}),
            json!({"method":archive["method"],"params":archive["params"]}),
        ]
    );
    ensure!(
        !sent.iter().any(|r| r["method"] == "thread/resume"),
        "A row action resumed the chat"
    );
    let image = fx.shot()?;
    image.save(format!("{}/s001.png", env!("CARGO_TARGET_TMPDIR")))?;
    for sample in rec["result"]["pixels"].as_array().unwrap() {
        let actual = css_pixel(&image, &sample["at"]);
        let original: [u8; 4] = serde_json::from_value(sample["rgba"].clone())?;
        let backdrop: [u8; 4] = serde_json::from_value(sample["backdrop"]["rgba"].clone())?;
        let background = css_pixel(&image, &sample["backdrop"]["at"]);
        for channel in 0..3 {
            let alpha = (original[channel] as f32 - backdrop[channel] as f32)
                / (255. - backdrop[channel] as f32);
            let expected = background[channel] + (255. - background[channel]) * alpha;
            ensure!(
                (actual[channel] - expected).abs() <= 1.,
                "The Archive hover blend differs in channel {channel}"
            );
        }
    }
    Ok(())
}
