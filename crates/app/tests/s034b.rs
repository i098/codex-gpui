//! Recorded browser login, copied-link fallback, and completion.

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
    use serde_json::{Value, json};
    use ui::screens::signin::{Options, policy::Policy};
    use url::Url;

    common::install_panic_hook();
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s034b.json"))?;
    let login = &rec["result"]["login"];
    let mut lines = script(None, |_| {})?;
    let config = ui::fixture::response("readback.jsonl", "config/read", 0);
    let account = ui::fixture::response("readback.jsonl", "account/read", 0);
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            line["msg"]["result"] = json!({"account":null,"requiresOpenaiAuth":true});
        }
    }
    lines.extend(exchange(
        "configRequirements/read",
        json!({"requirements":null}),
    ));
    lines.extend(exchange("config/read", config));
    lines.extend(exchange(
        "account/login/start",
        json!({
            "type": "chatgpt",
            "loginId": login["loginStart"]["loginId"],
            "authUrl": login["loginStart"]["authUrl"]
        }),
    ));
    lines.extend(exchange("account/read", account));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.wait("sign-in requirements fallback", |fx| {
        fx.read(|root, cx| {
            root.login().is_some_and(|login| {
                matches!(login.read(cx).policy(), Policy::Ready { chatgpt: true, .. })
            })
        })
    })?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.login().unwrap().update(cx, |login, cx| {
            login.set_options(
                Options {
                    desktop_auth: true,
                    browser_help: true,
                    ..Options::default()
                },
                cx,
            );
        });
    });
    let before = fx.sent()?.len();
    fx.keys("tab tab enter")?;
    let mut opened = None;
    fx.wait("browser handoff", |fx| {
        opened = fx.cx.take_opened_url();
        opened.is_some()
    })?;
    let opened = opened.context("No browser URL opened")?;
    let desktop = Url::parse(&opened)?;
    ensure!(desktop.path() == "/codex/desktop-auth");
    let authorize = desktop
        .query_pairs()
        .find(|(key, _)| key == "authorize_url")
        .map(|(_, value)| value.into_owned())
        .context("No nested authorization URL")?;
    let authorize = Url::parse(&authorize)?;
    let recorded = Url::parse(
        login["loginStart"]["authUrl"]
            .as_str()
            .context("Recorded authorization URL")?,
    )?;
    let state = |url: &Url| {
        url.query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
    };
    ensure!(authorize.origin() == recorded.origin() && state(&authorize) == state(&recorded));

    fx.cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("clipboard-marker".into()))
    });
    fx.keys("tab tab enter")?;
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|value| value.text()))
            == Some(opened)
    );
    let sent = fx.sent()?;
    let starts: Vec<_> = sent[before..]
        .iter()
        .filter(|message| message["method"] == "account/login/start")
        .collect();
    ensure!(starts.len() == 1 && starts[0]["params"]["type"] == "chatgpt");
    ensure!(fx.read(|root, _| root.signed_out()));

    let steps = rec["steps"].as_array().context("Recorded steps")?;
    for step in [&steps[1], &steps[2]] {
        let event = bridge::Event::Notification {
            method: step["method"]
                .as_str()
                .context("Recorded event method")?
                .into(),
            params: step["params"].clone(),
        };
        fx.root
            .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
        fx.cx.run_until_parked();
        if step["method"] == "account/updated" {
            ensure!(fx.read(|root, _| root.signed_out()));
        }
    }
    ensure!(!fx.read(|root, _| root.signed_out()));
    ensure!(fx.route() == Route::Home);
    Ok(())
}
