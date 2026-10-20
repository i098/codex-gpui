//! Replays the S014 grouped activity summary from refs/interactions/s014.json.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT, changed};
use serde_json::Value;
use std::fs;

const SUBTRACT_TURN: &str = "01a13de9-70bc-7de1-8ae6-bf46ce20cb99";
/// The transcript above the composer (whose model spinner animates).
const TRANSCRIPT: [f32; 4] = [340., 44., 936., 640.];

#[test]
fn proxy() {
    common::proxy_child();
}

fn rect(value: &Value) -> [f32; 4] {
    let n = |key| value[key].as_f64().unwrap() as f32;
    [n("x"), n("y"), n("w"), n("h")]
}

fn center(value: &Value) -> (f32, f32) {
    let [x, y, w, h] = rect(value);
    (x + w / 2., y + h / 2.)
}

fn texts(rows: &Value) -> Vec<String> {
    rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn grouped_activity_matches_the_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s014.json"))?)?;
    let steps = recording["steps"].as_array().context("steps")?;
    let result = &recording["result"];
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let row = fx.at("home", "Add a subtract")?;
    fx.click(row)?;
    fx.wait_thread(common::SUBTRACT)?;
    // steps[1] closes the Changes tab the original restores from a persisted atom; the port
    // keeps no tab state, so the transcript is already full width.
    assert_eq!(steps[1]["target"]["text"], "Close Changes tab");
    fx.hover((1270., 820.))?;
    let labels = |fx: &Fx| {
        fx.read(|r, cx| {
            r.thread()
                .unwrap()
                .read(cx)
                .activity_labels(SUBTRACT_TURN)
                .unwrap()
        })
    };
    let sent = fx.sent()?.len();
    let collapsed = fx.shot()?;
    // The Linux recorder wraps the user bubble onto three lines, the Mac port onto two: every
    // recorded box under it sits higher on the port by the `Worked for` offset.
    let dy = fx.at("thread", "Worked for")?.1 - center(&steps[2]["target"]["bbox"]).1;
    let at = |v: &Value| {
        let (x, y) = center(v);
        (x, y + dy)
    };
    let rect = |v: &Value| {
        let mut r = rect(v);
        r[1] += dy;
        r
    };

    assert_eq!(steps[2]["target"]["text"], "Worked for 17s");
    fx.click(at(&steps[2]["target"]["bbox"]))?;
    fx.advance(std::time::Duration::from_millis(
        steps[2]["waitMs"].as_u64().context("Worked for waitMs")?,
    ))?;
    fx.hover((1270., 820.))?;
    let opened = fx.shot()?;
    opened.save(format!(
        "{}/s014-worked-for.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    let summary = &result["expandedTurn"]["rows"][0];
    let summary_text = summary["text"].as_str().unwrap();
    assert_eq!(labels(&fx)[1..], texts(&result["expandedTurn"]));
    ensure!(
        changed(&collapsed, &opened, rect(&summary["bbox"])) > 500,
        "the summary row did not appear at the recorded bbox"
    );

    assert_eq!(steps[3]["type"], "hover");
    fx.hover(at(&steps[3]["target"]["bbox"]))?;
    let hovered = fx.shot()?;
    hovered.save(format!(
        "{}/s014-summary-hover.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    let hovered_summary = &result["hovered"]["rows"][0];
    assert_eq!(hovered_summary["leaves"][0]["color"], "rgb(255, 255, 255)");
    assert_eq!(hovered_summary["chevron"]["opacity"], "1");
    ensure!(
        changed(&opened, &hovered, rect(&summary["leaves"][0]["bbox"])) > 100,
        "the summary text did not brighten on hover"
    );
    ensure!(
        changed(&opened, &hovered, rect(&summary["chevron"]["bbox"])) > 10,
        "the chevron did not show on hover"
    );

    assert_eq!(steps[4]["target"]["text"], summary_text);
    fx.click(at(&steps[4]["target"]["bbox"]))?;
    fx.hover((1270., 820.))?;
    let expanded = fx.shot()?;
    expanded.save(format!(
        "{}/s014-group-expanded.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    assert_eq!(labels(&fx)[1..], texts(&result["expandedGroup"]));
    for row in &result["expandedGroup"]["rows"].as_array().unwrap()[1..] {
        ensure!(
            changed(&opened, &expanded, rect(&row["icon"]["bbox"])) > 10,
            "no icon at the recorded bbox of {:?}",
            row["text"]
        );
        ensure!(
            changed(&opened, &expanded, rect(&row["leaves"][0]["bbox"])) > 100,
            "no text at the recorded bbox of {:?}",
            row["text"]
        );
    }
    assert_eq!(
        result["expandedGroup"]["rows"][0]["chevron"]["rotate"],
        "90deg"
    );
    ensure!(
        changed(&opened, &expanded, rect(&summary["chevron"]["bbox"])) > 10,
        "the chevron did not turn down"
    );

    assert_eq!(steps[5]["target"]["text"], summary_text);
    fx.click(at(&steps[5]["target"]["bbox"]))?;
    fx.hover((1270., 820.))?;
    let closed = fx.shot()?;
    assert_eq!(labels(&fx)[1..], texts(&result["collapsedGroup"]));
    ensure!(
        changed(&opened, &closed, TRANSCRIPT) == 0,
        "collapsing the group did not restore the transcript"
    );
    // Expanding and collapsing send nothing; the recording's `requests` hold only the
    // original's periodic `thread/resume` retries.
    assert_eq!(
        fx.sent()?.len(),
        sent,
        "the toggles sent app-server requests"
    );
    Ok(())
}
