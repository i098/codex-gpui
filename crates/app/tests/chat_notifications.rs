#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, ensure};
use bridge::Event;
use common::{Fx, SUBTRACT};
use serde_json::json;

#[test]
fn proxy() {
    common::install_panic_hook();
    common::proxy_child();
}

fn emit(fx: &mut Fx, event: Event) {
    fx.root
        .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
    fx.cx.run_until_parked();
}

fn preference(fx: &mut Fx, key: &str, value: serde_json::Value) {
    fx.cx.update(|cx| {
        cx.global_mut::<app::host_settings::Settings>()
            .write(key, value)
            .unwrap();
    });
}

fn completion() -> Event {
    Event::Notification {
        method: "turn/completed".into(),
        params: json!({"threadId": SUBTRACT, "turn": {"id": "notify-turn", "status": "completed", "items": []}}),
    }
}

fn request(id: u64, question: bool) -> Event {
    Event::Request {
        id: json!(id),
        method: if question {
            "item/tool/requestUserInput"
        } else {
            "item/permissions/requestApproval"
        }
        .into(),
        params: if question {
            json!({"threadId": SUBTRACT, "turnId": "notify-turn", "itemId": "question",
                "questions": [{"id": "choice", "header": "Choice", "question": "Continue?", "options": [{"label": "Yes", "description": "Continue"}]}]})
        } else {
            json!({"threadId": SUBTRACT, "turnId": "notify-turn", "itemId": "permission",
                "permissions": {"network": {"enabled": true}}})
        },
    }
}

#[test]
fn ordinary_notifications_follow_focus_and_preferences_without_focusing_the_window() -> Result<()> {
    common::install_panic_hook();
    let mut fx = Fx::new()?;
    preference(&mut fx, "notifications-sound", json!("none"));
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.cx
        .update_window(fx.window, |_, window, cx| window.blur(cx))?;
    fx.cx.run_until_parked();
    ensure!(
        !fx.cx
            .update_window(fx.window, |_, window, _| window.is_window_active())?
    );

    emit(&mut fx, completion());
    fx.wait("ordinary completion notification", |fx| {
        fx.cx.shown_system_notifications().len() == 1
    })?;
    ensure!(fx.cx.shown_system_notifications()[0].body == "Your turn is complete.");
    ensure!(
        !fx.cx
            .update_window(fx.window, |_, window, _| window.is_window_active())?,
        "notification stole focus"
    );

    preference(&mut fx, "notifications-turn-mode", json!("off"));
    emit(&mut fx, completion());
    fx.advance(std::time::Duration::from_millis(100))?;
    ensure!(fx.cx.shown_system_notifications().len() == 1);

    emit(&mut fx, request(801, false));
    fx.wait("approval notification", |fx| {
        fx.cx.shown_system_notifications().len() == 2
    })?;
    emit(&mut fx, request(801, false));
    ensure!(
        fx.cx.shown_system_notifications().len() == 2,
        "duplicate approval alerted twice"
    );
    preference(&mut fx, "notifications-permissions-enabled", json!(false));
    emit(&mut fx, request(802, false));
    fx.advance(std::time::Duration::from_millis(100))?;
    ensure!(fx.cx.shown_system_notifications().len() == 2);

    emit(&mut fx, request(803, true));
    fx.wait("question notification", |fx| {
        fx.cx.shown_system_notifications().len() == 3
    })?;
    preference(&mut fx, "notifications-questions-enabled", json!(false));
    emit(&mut fx, request(804, true));
    fx.advance(std::time::Duration::from_millis(100))?;
    ensure!(fx.cx.shown_system_notifications().len() == 3);

    preference(&mut fx, "notifications-turn-mode", json!("unfocused"));
    fx.cx
        .update_window(fx.window, |_, window, _| window.activate_window())?;
    fx.cx.run_until_parked();
    emit(&mut fx, completion());
    fx.advance(std::time::Duration::from_millis(100))?;
    ensure!(fx.cx.shown_system_notifications().len() == 3);
    preference(&mut fx, "notifications-turn-mode", json!("always"));
    emit(&mut fx, completion());
    fx.wait("always completion notification", |fx| {
        fx.cx.shown_system_notifications().len() == 4
    })?;
    fx.click(common::RAIL_HOME)?;
    fx.cx
        .update_window(fx.window, |_, window, cx| window.blur(cx))?;
    let tag = fx.cx.shown_system_notifications()[0].tag.clone();
    fx.cx
        .simulate_system_notification_response(gpui::SystemNotificationResponse {
            tag,
            action_id: None,
        });
    fx.wait_thread(SUBTRACT)?;
    ensure!(fx.cx.delivered_system_notifications().is_empty());
    Ok(())
}
