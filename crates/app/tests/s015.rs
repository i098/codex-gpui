//! Replays S015 (TH-097 command row expand, TH-122 approval keys) from the original renderer
//! recording `refs/interactions/s015.json`.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT, SUBTRACT, changed};
use image::RgbaImage;
use serde_json::{Value, json};
use std::{fs, time::Duration};

const RAN: &str = "Ran python3 -c 'from src.math import subtract; print(subtract(5, 3))'";
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

/// The CSS x just past the last ink of `shot` in the CSS-px band `[x, y, w, h]`: the band's
/// right end is background.
fn ink_end(shot: &RgbaImage, [x, y, w, h]: [f32; 4]) -> f32 {
    let (x0, x1) = ((x * 2.) as u32, ((x + w) * 2.) as u32);
    let (y0, y1) = ((y * 2.) as u32, ((y + h) * 2.) as u32);
    let bg = shot.get_pixel(x1 - 1, y0);
    (x0..x1)
        .rev()
        .find(|&px| (y0..y1).any(|py| shot.get_pixel(px, py) != bg))
        .map_or(x, |px| (px + 1) as f32 / 2.)
}

fn recording() -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s015.json"
    ))?)?)
}

/// Opens the subtract chat and its `Worked for` group, as the recording's first steps do.
fn open_group(fx: &mut Fx, steps: &[Value]) -> Result<f32> {
    let row = fx.at("home", "Add a subtract")?;
    fx.click(row)?;
    fx.wait_thread(SUBTRACT)?;
    // steps[1] closes the Changes tab the original restores from a persisted atom; the port
    // keeps no tab state, so the transcript is already full width.
    assert_eq!(steps[1]["target"]["text"], "Close Changes tab");
    fx.hover((1270., 820.))?;
    // The Linux recorder wraps the user bubble onto three lines, the Mac port onto two: every
    // recorded box under it sits higher on the port by the `Worked for` offset.
    let dy = fx.at("thread", "Worked for")?.1 - center(&steps[2]["target"]["bbox"]).1;
    for step in &steps[2..4] {
        assert_eq!(step["type"], "click");
        let (x, y) = center(&step["target"]["bbox"]);
        fx.click((x, y + dy))?;
        // The `Worked for` disclosure animates open (motion::RELAXED).
        fx.advance(Duration::from_millis(400))?;
    }
    fx.hover((1270., 820.))?;
    Ok(dy)
}

#[test]
fn command_row_expands_the_shell_card() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording = recording()?;
    let steps = recording["steps"].as_array().context("steps")?;
    let result = &recording["result"];
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let dy = open_group(&mut fx, steps)?;
    let at = |v: &Value| {
        let (x, y) = center(v);
        (x, y + dy)
    };
    let rect = |v: &Value| {
        let mut r = rect(v);
        r[1] += dy;
        r
    };
    let sent = fx.sent()?.len();
    let rest = fx.shot()?;
    let labels = |fx: &Fx| {
        fx.read(|r, cx| {
            r.thread()
                .unwrap()
                .read(cx)
                .activity_labels("01a13de9-70bc-7de1-8ae6-bf46ce20cb99")
                .unwrap()
        })
    };
    ensure!(labels(&fx).contains(&RAN.to_owned()), "{:?}", labels(&fx));

    // Hover: the label turns white, the chevron shows.
    assert_eq!(steps[4]["type"], "hover");
    assert_eq!(steps[4]["target"]["text"], RAN);
    let label = &result["ranHover"]["label"];
    assert_eq!(label["computed"]["color"], "rgb(255, 255, 255)");
    assert_eq!(result["ranRest"]["chevron"]["computed"]["opacity"], "0");
    fx.hover(at(&steps[4]["target"]["bbox"]))?;
    let hovered = fx.shot()?;
    hovered.save(format!(
        "{}/s015-ran-hover.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    ensure!(
        changed(&rest, &hovered, rect(&label["bbox"])) > 100,
        "the Ran label did not brighten on hover"
    );
    // The chevron follows the label's end, which the Mac's font metrics set: a 14 px box
    // 4 px past the label's ink, at the recorded y.
    let mut chevron = rect(&result["ranHover"]["chevron"]["bbox"]);
    let mut band = rect(&label["bbox"]);
    band[2] += 40.;
    chevron[0] = ink_end(&rest, band) + 4.;
    ensure!(
        changed(&rest, &hovered, chevron) > 10,
        "the chevron did not show on hover at {chevron:?}"
    );

    // Click: the shell card opens under the row, the chevron points down.
    assert_eq!(steps[5]["type"], "click");
    fx.click(at(&steps[5]["target"]["bbox"]))?;
    fx.hover((1270., 820.))?;
    let opened = fx.shot()?;
    opened.save(format!("{}/s015-ran-open.png", env!("CARGO_TARGET_TMPDIR")))?;
    let open = &result["ranOpen"];
    assert_eq!(open["header"]["expanded"], "true");
    assert_eq!(open["chevron"]["computed"]["rotate"], "90deg");
    ensure!(
        changed(&rest, &opened, chevron) > 10,
        "the chevron did not turn down"
    );
    let card = &open["card"];
    assert_eq!(
        card["computed"]["backgroundColor"],
        "rgba(255, 255, 255, 0.08)"
    );
    assert_eq!(card["computed"]["borderRadius"], "12.5px");
    ensure!(
        changed(&rest, &opened, rect(&card["bbox"])) > 2000,
        "no card at the recorded bbox"
    );
    let texts: Vec<&str> = card["texts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        texts,
        [
            "Shell",
            "$",
            "python3 -c 'from src.math import subtract; print(subtract(5, 3))'",
            "2"
        ]
    );
    for text in card["texts"].as_array().unwrap() {
        ensure!(
            changed(&rest, &opened, rect(&text["bbox"])) > 20,
            "no text at the recorded bbox of {:?}",
            text["text"]
        );
    }
    let footer = &card["footer"];
    assert_eq!(footer["text"], "Success");
    ensure!(
        changed(&rest, &opened, rect(&footer["textBbox"])) > 100,
        "no `Success` at the recorded bbox"
    );
    ensure!(
        changed(&rest, &opened, rect(&footer["icon"]["bbox"])) > 10,
        "no check icon at the recorded bbox"
    );
    // The card is not an activity row: the labels are unchanged.
    ensure!(labels(&fx).contains(&RAN.to_owned()), "{:?}", labels(&fx));

    // Click again: the card folds, the transcript is as before.
    assert_eq!(steps[6]["type"], "click");
    fx.click(at(&steps[6]["target"]["bbox"]))?;
    fx.hover((1270., 820.))?;
    let closed = fx.shot()?;
    assert!(result["ranClosed"]["card"].is_null());
    ensure!(
        changed(&rest, &closed, TRANSCRIPT) == 0,
        "folding the card did not restore the transcript"
    );
    // The toggles send nothing; the recording's `expandRequests` hold only the original's
    // periodic `thread/resume` retries.
    assert_eq!(
        fx.sent()?.len(),
        sent,
        "the toggles sent app-server requests"
    );
    Ok(())
}

/// The recording's three approval requests, each after the answer to the one before, with
/// the replay's answers to the sends their keys cause.
fn inject_approvals(recording: &Value) -> Result<Vec<Value>> {
    let mut lines = common::script(None, |_| {})?;
    let read = lines
        .iter()
        .position(|v| v["dir"] == "recv" && v["msg"]["result"]["thread"]["id"] == SUBTRACT)
        .context("no thread/read reply")?;
    let requests = recording["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["type"] == "bridge")
        .map(|s| &s["message"]["request"]);
    let mut inserted = Vec::new();
    for (request, case) in requests.zip(recording["result"]["approvals"].as_array().unwrap()) {
        let reply = &case["afterKey"]["responses"][0]["response"];
        assert_eq!(reply["id"], request["id"]);
        inserted.push(json!({ "dir": "recv", "msg": request }));
        inserted.push(json!({ "dir": "send", "msg": reply }));
    }
    lines.splice(read + 1..read + 1, inserted);
    lines.extend(common::exchange("thread/resume", json!({})));
    lines.extend(common::exchange("turn/start", json!({})));
    Ok(lines)
}

#[test]
fn approval_keys_answer_the_request() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording = recording()?;
    let cases = recording["result"]["approvals"].as_array().unwrap();
    let mut fx = Fx::captured("dark", (1280., 832.), inject_approvals(&recording)?)?;
    let row = fx.at("home", "Add a subtract")?;
    fx.click(row)?;
    fx.wait_thread(SUBTRACT)?;
    let composer = fx.composer();
    let pending = |fx: &Fx| fx.read(|_, cx| composer.read(cx).approval().is_some());
    // The replay emits each request as soon as the one before is answered: the card of a
    // case shows until its key, and the last one leaves no card.
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let key = case["key"].as_str().unwrap();
        let draft = case["draft"].as_str().unwrap();
        fx.wait(&format!("the approval card of {id}"), |fx| pending(fx))?;
        ensure!(fx.composer_focused()?, "the composer lost focus");
        if !draft.is_empty() {
            fx.type_text(draft)?;
        }
        fx.keys(&key.to_lowercase())?;
        let decision = &case["afterKey"]["responses"][0]["response"]["result"];
        let reply = json!({ "reply": id, "result": decision });
        fx.wait(&format!("the answer to {id}"), |fx| {
            fx.sent().unwrap().contains(&reply)
        })?;
        assert_eq!(fx.composer_text()?, case["afterKey"]["composerText"]);
        if !draft.is_empty() {
            // Enter also sends the draft (the original asked `thread/resume` for it).
            fx.wait("the draft's send", |fx| {
                fx.sent()
                    .unwrap()
                    .iter()
                    .any(|m| m["method"] == "thread/resume" && m["params"]["threadId"] == SUBTRACT)
            })?;
        }
    }
    ensure!(!pending(&fx), "a card stayed after the last answer");
    Ok(())
}
