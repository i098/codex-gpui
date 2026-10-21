mod common;

use anyhow::Result;
use bridge::{Event, types::ThreadItem};
use common::Fx;
use gpui::{
    AppContext as _, HeadlessAppContext, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput,
    point, px, size,
};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::plan_summary::{OpenPlan, Plan, PlanSummary};

fn notify(fx: &mut Fx, method: &str, params: Value) {
    fx.cx.update(|cx| {
        fx.root.update(cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: method.into(),
                    params,
                },
                cx,
            )
        })
    });
}

#[test]
fn plan_stream_stays_scoped_and_opens_only_after_completion() -> Result<()> {
    let mut fx = Fx::with("dark", (1280., 832.), None, |line| {
        if let Some(thread) = line["msg"]["result"].get_mut("thread") {
            thread["turns"] = json!([]);
        }
    })?;
    fx.action(Box::new(app::GoToRecentChat(0)))?;
    fx.wait_thread(common::GREETING)?;
    notify(
        &mut fx,
        "turn/started",
        json!({"threadId": common::GREETING,
        "turn": {"id":"plan-turn", "status":"inProgress", "items":[]}}),
    );
    notify(
        &mut fx,
        "item/started",
        json!({"threadId":common::GREETING, "turnId":"plan-turn",
        "item":{"type":"plan", "id":"plan-item", "text":"# Plan\n"}}),
    );
    let plan = Plan {
        conversation_id: common::GREETING.into(),
        host_id: "local".into(),
        item_id: "plan-item".into(),
        content: "untrusted action payload".into(),
    };
    fx.action(Box::new(OpenPlan(plan.clone())))?;
    assert_eq!(fx.read(|root, _| root.side_panel()), None);
    notify(
        &mut fx,
        "item/plan/delta",
        json!({"threadId":common::GREETING, "turnId":"other-turn",
        "itemId":"plan-item", "delta":"wrong turn"}),
    );
    notify(
        &mut fx,
        "item/plan/delta",
        json!({"threadId":"other-thread", "turnId":"plan-turn",
        "itemId":"plan-item", "delta":"wrong thread"}),
    );
    notify(
        &mut fx,
        "item/plan/delta",
        json!({"threadId":common::GREETING, "turnId":"plan-turn",
        "itemId":"plan-item", "delta":"Keep the café open.\n"}),
    );
    assert_eq!(
        fx.read(|root, cx| {
            let thread = root.thread().unwrap().read(cx);
            match &thread.turns()[0].items[0] {
                ThreadItem::Plan { text, .. } => text.clone(),
                _ => unreachable!(),
            }
        }),
        "# Plan\nKeep the café open.\n"
    );
    notify(
        &mut fx,
        "item/completed",
        json!({"threadId":common::GREETING, "turnId":"plan-turn",
        "item":{"type":"plan", "id":"plan-item", "text":"# Plan\nKeep the café open.\n"}}),
    );
    fx.action(Box::new(OpenPlan(plan.clone())))?;
    assert_eq!(fx.read(|root, _| root.side_panel()), None);
    notify(
        &mut fx,
        "item/started",
        json!({"threadId":common::GREETING, "turnId":"plan-turn",
        "item":{"type":"agentMessage", "id":"next-item", "text":""}}),
    );
    fx.action(Box::new(OpenPlan(plan.clone())))?;
    assert_eq!(
        fx.read(|root, _| root.side_panel()),
        Some(app::SidePanel::Plan)
    );
    assert!(fx.read(|root, cx| root.thread().unwrap().read(cx).plan_completed("plan-item")));
    fx.action(Box::new(OpenPlan(plan)))?;
    assert_eq!(fx.read(|root, _| root.side_panel()), None);
    Ok(())
}

#[test]
fn standalone_plan_collapses_and_expands_from_its_controls() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let mut cx =
        HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
    });
    let window = cx.open_window(size(px(600.), px(500.)), |_, cx| {
        cx.new(|_| {
            PlanSummary::page(Plan {
                conversation_id: "unit-test".into(),
                host_id: "local".into(),
                item_id: "plan".into(),
                content: "# Plan\n\nOne step.\n".repeat(20),
            })
        })
    })?;
    cx.run_until_parked();
    for expected in [true, false] {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(554.), px(40.)),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: point(px(554.), px(40.)),
                    modifiers: Default::default(),
                    click_count: 1,
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
        cx.update(|cx| window.update(cx, |page, _, _| assert_eq!(page.collapsed(), expected)))?;
    }
    Ok(())
}

#[gpui::test]
async fn download_preserves_markdown_bytes_and_cancel_leaves_files_alone(
    cx: &mut gpui::TestAppContext,
) {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PLAN.md");
    let content = "# Plan\r\n\r\n- Keep the café open.\n";
    let task = cx.update(|cx| ui::screens::plan_summary::download(content.into(), cx));
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    task.await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), content.as_bytes());
    let task = cx.update(|cx| ui::screens::plan_summary::download("not saved".into(), cx));
    cx.simulate_new_path_selection(|_| None);
    task.await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), content.as_bytes());
}
