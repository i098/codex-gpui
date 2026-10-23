//! Row menu `Share` (AM-024): the task's link reaches the clipboard and the toast confirms it.
use anyhow::{Result, ensure};
use app::Root;

#[test]
fn link_is_the_originals_scheduled_url_with_the_id_encoded() -> Result<()> {
    ensure!(
        Root::automation_link("0197-abc")
            == "https://chatgpt.com/scheduled?automationId=0197-abc&automationSource=local"
    );
    let odd = Root::automation_link("a b&c=d/é");
    ensure!(
        odd == "https://chatgpt.com/scheduled?automationId=a+b%26c%3Dd%2F%C3%A9&automationSource=local",
        "{odd}"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

/// The headless platform keeps its clipboard in memory: the user's pasteboard is untouched.
#[cfg(target_os = "macos")]
#[test]
fn share_copies_the_task_link_and_confirms_once() -> Result<()> {
    use app::host::{Host, LocalHost, Request, json_body};
    use common::{Fx, RAIL_SCHEDULED};
    use serde_json::json;
    use std::sync::Arc;
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let params = json!({"name": "Review", "prompt": "Review local changes",
        "rrule": "FREQ=DAILY;BYHOUR=9;BYMINUTE=0", "cwds": [home.path()]});
    let created = json_body(&host.fetch(&Request::post("automation-create", Some(params)))?)?;
    let id = created["id"].as_str().expect("created task ID").to_owned();
    let mut fx = Fx::new()?;
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root, cx| root.set_host(Arc::new(host), cx))
    });
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("saved task listed", |fx| {
        fx.read(|r, _| r.automations().is_some_and(Result::is_ok))
    })?;
    ensure!(fx.cx.update(|cx| cx.read_from_clipboard()).is_none());

    let shot = |fx: &mut Fx, name: &str| -> Result<()> {
        if let Ok(output) = std::env::var("CGX_SHARE_SHOTS") {
            std::fs::create_dir_all(&output)?;
            fx.shot()?.save(format!("{output}/{name}.png"))?;
        }
        Ok(())
    };
    fx.hover(ROW)?;
    shot(&mut fx, "1-hover")?;
    fx.click(ACTIONS)?;
    fx.advance(std::time::Duration::from_millis(300))?;
    shot(&mut fx, "2-menu")?;
    fx.click(SHARE)?;
    shot(&mut fx, "3-shared")?;

    let link = format!("https://chatgpt.com/scheduled?automationId={id}&automationSource=local");
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    ensure!(copied.as_deref() == Some(link.as_str()), "{copied:?}");
    ensure!(fx.read(|r, cx| r.toasts(cx)) == ["Link copied"]);
    ensure!(
        matches!(fx.route(), app::Route::Scheduled),
        "Share must not open the task"
    );
    ensure!(
        fx.read(|root, cx| root
            .scheduled_sidebar()
            .is_some_and(|sidebar| sidebar.read(cx).open_menu().is_none())),
        "Share must close the menu"
    );
    for keys in ["end enter", "home down down space", "home up enter"] {
        fx.cx.update(|cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("not a task link".into()));
        });
        for key in std::iter::once("enter").chain(keys.split_whitespace()) {
            let keystroke = gpui::Keystroke::parse(key)?;
            fx.input(gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }))?;
            fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }))?;
        }
        let copied = fx
            .cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
        ensure!(
            copied.as_deref() == Some(link.as_str()),
            "{keys}: {copied:?}"
        );
        ensure!(
            matches!(fx.route(), app::Route::Scheduled),
            "keyboard Share must not open the task"
        );
    }
    Ok(())
}

/// Row, `Scheduled task actions` button and `Share` item of the first task (1280x832).
#[cfg(target_os = "macos")]
const ROW: (f32, f32) = (150., 190.);
#[cfg(target_os = "macos")]
const ACTIONS: (f32, f32) = (302., 190.);
#[cfg(target_os = "macos")]
const SHARE: (f32, f32) = (230., 283.);
