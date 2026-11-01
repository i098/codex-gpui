mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::{Fx, SUBTRACT};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

/// A finished turn's plan item opens from Thread summary ▸ `Plan` in the `Plan` tab.
#[test]
fn summary_plan_row_opens_plan_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn = fx
        .read(|r, cx| Some(r.thread()?.read(cx).turns().last()?.id.clone()))
        .context("subtract turn")?;
    let plan = "# Ship subtract\n\n- Add `subtract`\n- Test it";
    fx.root.update(&mut fx.cx, |r, cx| {
        r.on_event(
            bridge::Event::Notification {
                method: "item/completed".into(),
                params: json!({"threadId": SUBTRACT, "turnId": turn,
                    "item": {"type": "plan", "id": "plan-1", "text": plan}}),
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
    let shots = std::env::var("CGX_PLAN_SHOTS").ok();
    if let Some(out) = &shots {
        std::fs::create_dir_all(out)?;
    }
    // `Toggle summary`, 63 px from the right edge, then the `Plan` row.
    let w = fx.size.0;
    fx.click((w - 63., 22.))?;
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/1-summary.png"))?;
    }
    fx.action(Box::new(ui::OpenPlanPanel))?;
    ensure!(
        fx.read(|r, _| r.side_panel()) == Some(SidePanel::Plan),
        "side panel {:?}",
        fx.read(|r, _| r.side_panel())
    );
    let title = fx.read(|r, cx| Some(r.plan()?.read(cx).title()));
    ensure!(title.as_deref() == Some("Ship subtract"), "{title:?}");
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/2-plan-tab.png"))?;
    }
    Ok(())
}
