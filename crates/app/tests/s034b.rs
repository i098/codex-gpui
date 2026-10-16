//! ST-178 `Continue to sign in` (ChatGPT), replayed from `refs/interactions/s034b.json`: the
//! signed-out port sends the original's `account/login/start`, opens the answered `authUrl`,
//! shows `Continue signing in with your browser` with `Cancel sign-in` at the recorded boxes,
//! and enters the app on `account/login/completed` success.

#[cfg(target_os = "macos")]
mod common;

#[test]
#[cfg(target_os = "macos")]
fn proxy() {
    common::proxy_child();
}

#[test]
#[cfg(target_os = "macos")]
fn recorded_chatgpt_sign_in() -> anyhow::Result<()> {
    use anyhow::{Context as _, ensure};
    use app::Route;
    use common::*;
    use image::RgbaImage;
    use serde_json::{Value, json};

    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s034b.json"))?;
    let login = &rec["result"]["login"];
    // The signed-out app-server of the recording: no account, and the mock's login answer.
    let mut lines = script(None, |v| {
        if v["dir"] == "recv" && v["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            v["msg"]["result"] = json!({ "account": null, "requiresOpenaiAuth": true });
        }
    })?;
    lines.extend(exchange(
        "account/login/start",
        json!({
            "type": "chatgpt",
            "loginId": login["loginStart"]["loginId"],
            "authUrl": login["loginStart"]["authUrl"],
        }),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    ensure!(fx.read(|r, _| r.signed_out()), "no sign-in view");
    let before = fx.sent()?.len();
    let rest = fx.shot()?;

    let bbox = |b: &Value| ["x", "y", "w", "h"].map(|k| b[k].as_f64().unwrap() as f32);
    // Device pixels (the window is 2x) inside the CSS-px box that are not the page surface.
    let ink = |shot: &RgbaImage, [x, y, w, h]: [f32; 4]| -> usize {
        let surface = shot.get_pixel(2, 2).0;
        let (x0, y0) = ((x * 2.) as u32, (y * 2.) as u32);
        let (x1, y1) = (((x + w) * 2.) as u32, ((y + h) * 2.) as u32);
        (y0..y1)
            .flat_map(|py| (x0..x1).map(move |px| (px, py)))
            .filter(|&(px, py)| {
                let p = shot.get_pixel(px, py).0;
                (0..3).any(|i| p[i].abs_diff(surface[i]) > 8)
            })
            .count()
    };
    let steps = rec["steps"].as_array().context("steps")?;
    let (click, updated, completed) = (&steps[0], &steps[1], &steps[2]);
    assert_eq!(click["target"]["text"], "Continue to sign in");
    let button = bbox(&login["rest"]["continue"]["bbox"]);
    ensure!(
        ink(&rest, button) > 10_000,
        "no `Continue to sign in` button at the recorded box"
    );

    let [x, y, w, h] = bbox(&click["target"]["bbox"]);
    fx.click((x + w / 2., y + h / 2.))?;
    let mut opened = None;
    fx.wait("account/login/start answered", |fx| {
        opened = fx.cx.take_opened_url();
        opened.is_some()
    })?;
    // The original appends its app version and statsig stable ids to the server's authUrl.
    let strip = |url: &str| {
        url.split('&')
            .filter(|p| {
                !p.starts_with("codex_app_version=")
                    && !p.starts_with("source_surface_stable_id=")
                    && !p.starts_with("codex_origin_stable_id=")
            })
            .collect::<Vec<_>>()
            .join("&")
    };
    let opened = opened.context("no browser URL opened")?;
    assert_eq!(
        strip(&opened),
        strip(login["openExternal"]["url"].as_str().unwrap()),
        "opened URL"
    );
    assert_eq!(
        login["openExternal"]["openTarget"], "external-browser",
        "the original opens it in the external browser"
    );
    let sent = fx.sent()?.split_off(before);
    let want: Vec<Value> = login["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| json!({ "method": r["method"], "params": r["params"] }))
        .collect();
    let got: Vec<Value> = sent
        .iter()
        .filter(|m| {
            m["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("account/login/"))
        })
        .map(|m| json!({ "method": m["method"], "params": m["params"] }))
        .collect();
    assert_eq!(got, want, "account/login requests");
    ensure!(
        fx.read(|r, _| r.signed_out()),
        "left the sign-in view early"
    );

    // Pending view: the message and the button where the original draws them, nothing else
    // in the column (the `Continue to sign in` button is gone).
    let pending = fx.shot()?;
    pending.save(format!("{}/s034b-pending.png", env!("CARGO_TARGET_TMPDIR")))?;
    let title = bbox(&login["pending"]["title"]["bbox"]);
    let cancel = bbox(&login["pending"]["cancel"]["bbox"]);
    ensure!(ink(&pending, title) > 200, "no message at the recorded box");
    ensure!(
        ink(&pending, cancel) > 1_000,
        "no `Cancel sign-in` at the recorded box"
    );
    let column = |y0: f32, y1: f32| [470., y0, 340., y1 - y0];
    for (what, band) in [
        ("above the message", column(title[1] - 12., title[1] - 2.)),
        (
            "between message and button",
            column(title[1] + title[3] + 2., cancel[1] - 2.),
        ),
        (
            "below the button",
            column(cancel[1] + cancel[3] + 2., cancel[1] + cancel[3] + 40.),
        ),
    ] {
        ensure!(
            ink(&pending, band) == 0,
            "pending view draws {what}: {band:?}"
        );
    }

    for step in [updated, completed] {
        let event = bridge::Event::Notification {
            method: step["method"].as_str().unwrap().into(),
            params: step["params"].clone(),
        };
        fx.root
            .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
        fx.cx.run_until_parked();
    }
    ensure!(
        !fx.read(|r, _| r.signed_out()),
        "login/completed did not enter the app"
    );
    assert_eq!(fx.route(), Route::Home);
    assert_eq!(rec["result"]["state"]["route"], "home");
    let note = fx.parity("home")?;
    ensure!(note.contains("PASS"), "home after sign-in: {note}");
    println!("S034b: login/start, opened URL, pending view and home after completion {note}");
    Ok(())
}
