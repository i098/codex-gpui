//! PAGE-6: the Recents header's `Chat sidebar options` menu (the original's `sortMenu`)
//! sorts the chats by `Created` or `Last updated`. PAGE-4: the `Cloud` group lists the
//! account's current cloud tasks and opens one at `/remote/:taskId`.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;

#[test]
fn proxy() {
    proxy_child();
}

fn ids(fx: &Fx) -> Vec<String> {
    fx.read(|root, cx| {
        let sidebar = root.sidebar().read(cx);
        sidebar.chat_ids().map(str::to_owned).collect()
    })
}

/// Opens the options menu and picks its `row`th item by keys.
fn pick(fx: &mut Fx, row: usize) -> Result<()> {
    // The button shows only while its header is hovered.
    let header = fx
        .cx
        .debug_bounds(fx.window, "recents-header")?
        .context("Recents header")?
        .center();
    fx.hover((header.x.into(), header.y.into()))?;
    let button = fx
        .cx
        .debug_bounds(fx.window, "chat-options")?
        .context("chat options button")?
        .center();
    let at = (button.x.into(), button.y.into());
    fx.hover(at)?;
    fx.click(at)?;
    fx.capture_evidence("CGX_SORT_SHOTS", "menu")?;
    for _ in 0..=row {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[cfg(target_os = "macos")]
#[test]
fn created_sort_reorders_recents_and_last_updated_restores_it() -> Result<()> {
    // The last created chat was updated most recently, so `thread/list` puts it first.
    let lines = script(None, |line| {
        if let Some(data) = line["msg"]["result"]["data"].as_array_mut()
            && data.first().is_some_and(|t| t.get("createdAt").is_some())
        {
            data.rotate_right(1);
        }
    })?;
    let mut fx = Fx::scripted(lines)?;
    fx.wait("three chats", |fx| ids(fx).len() == 3)?;
    let updated = ids(&fx);
    pick(&mut fx, 1)?;
    let created = ids(&fx);
    ensure!(
        created == [&*updated[1], &*updated[2], &*updated[0]],
        "Created order {created:?} from {updated:?}"
    );
    ensure!(
        fx.read(|root, cx| root.sidebar().read(cx).chat_sort)
            == ui::screens::sidebar::ChatSort::CreatedAt
    );
    fx.capture_evidence("CGX_SORT_SHOTS", "created")?;
    pick(&mut fx, 0)?;
    ensure!(ids(&fx) == updated, "Last updated restores the list order");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn cloud_group_lists_current_tasks_and_opens_one() -> Result<()> {
    // The recording has no `oai-did` header, so `ReplayHost` reports no account.
    struct SignedIn(app::host::ReplayHost);
    impl app::host::Host for SignedIn {
        fn fetch(&self, request: &app::host::Request) -> Result<serde_json::Value> {
            self.0.fetch(request)
        }
        fn identity(&self) -> Option<app::host::Identity> {
            Some(app::host::Identity {
                account_id: "acct-fixture".into(),
                user_id: "user-fixture".into(),
                device_id: "device-fixture".into(),
            })
        }
    }
    let host = std::sync::Arc::new(SignedIn(app::host::ReplayHost::from_recording(
        include_str!("../../../fixtures/orbit-tasks.json"),
    )?));
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.wait("two cloud rows", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).cloud_tasks().len() == 2)
    })?;
    fx.capture_evidence("CGX_SORT_SHOTS", "cloud")?;
    let row = fx
        .cx
        .debug_bounds(fx.window, "cloud-task-task-fixture-1")?
        .context("cloud row")?
        .center();
    fx.click((row.x.into(), row.y.into()))?;
    ensure!(
        fx.route()
            == app::Route::Remote {
                task_id: "task-fixture-1".into(),
                hotkey: false
            },
        "the row opened {:?}",
        fx.route()
    );
    Ok(())
}
