//! PAGE-6: the Recents header's `Chat sidebar options` menu (`refs/chat-options*`): sort by
//! `Last updated` or `Manual order`, organize `By project` or `In one list`, `Show` ▸
//! `Projects`. PAGE-4: the `Cloud` group lists the account's current cloud tasks and opens
//! one at `/remote/:taskId`.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use ui::screens::sidebar::{ChatSort, Organize};

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

/// Opens the options menu, then presses `keys` in it.
fn open_with(fx: &mut Fx, keys: &[&str]) -> Result<()> {
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
    fx.click((button.x.into(), button.y.into()))?;
    keys.iter().try_for_each(|key| fx.keys(key))
}

/// The menu, then each submenu opened by keys, as `tools/route1/capture.mjs` opens them.
#[cfg(target_os = "macos")]
#[test]
fn chat_options_menu_matches_the_original() -> Result<()> {
    let mut notes = Vec::new();
    for (screen, keys) in [
        ("chat-options", &[][..]),
        ("chat-options-organize", &["down", "right"][..]),
        ("chat-options-sort", &["down", "down", "right"][..]),
    ] {
        let mut fx = Fx::open("dark", (1280., 832.), script(None, |_| {})?)?;
        fx.wait("three chats", |fx| ids(fx).len() == 3)?;
        open_with(&mut fx, keys)?;
        fx.capture_evidence("CGX_SORT_SHOTS", screen)?;
        notes.push(fx.parity(screen));
    }
    let failed = notes.iter().any(Result::is_err);
    let notes: Vec<String> = notes
        .into_iter()
        .map(|n| n.unwrap_or_else(|e| format!("{e:#}")))
        .collect();
    println!("{}", notes.join("\n"));
    ensure!(!failed, "{}", notes.join("\n"));
    Ok(())
}

#[test]
fn manual_order_keeps_the_order_and_last_updated_restores_it() -> Result<()> {
    let mut fx = Fx::open("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.wait("three chats", |fx| ids(fx).len() == 3)?;
    let updated = ids(&fx);
    open_with(&mut fx, &["down", "down", "right", "down", "enter"])?;
    let options = fx.read(|root, cx| root.sidebar().read(cx).chat_options.clone());
    ensure!(options.sort == ChatSort::Manual, "{options:?}");
    ensure!(options.manual_order == updated, "{options:?}");
    // A saved manual order applies to the next list; a chat it does not know goes on top.
    let reversed: Vec<String> = updated.iter().rev().take(2).cloned().collect();
    fx.root.update(&mut fx.cx, |root, cx| {
        root.sidebar().update(cx, |s, cx| {
            s.chat_options.manual_order = reversed.clone();
            cx.emit(ui::screens::sidebar::ChatOrderChanged);
        })
    });
    fx.cx.run_until_parked();
    let manual = ids(&fx);
    ensure!(
        manual == [&*updated[0], &*updated[2], &*updated[1]],
        "manual {manual:?} from {updated:?}"
    );
    open_with(&mut fx, &["down", "down", "right", "enter"])?;
    ensure!(ids(&fx) == updated, "Last updated restores the list order");
    Ok(())
}

#[test]
fn one_list_hides_projects_and_show_projects_toggles() -> Result<()> {
    let mut fx = Fx::open("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.wait("three chats", |fx| ids(fx).len() == 3)?;
    let options = |fx: &Fx| fx.read(|root, cx| root.sidebar().read(cx).chat_options.clone());
    // `Projects` is the third row while organized by project.
    open_with(&mut fx, &["down", "down", "down", "enter"])?;
    ensure!(!options(&fx).show_projects, "{:?}", options(&fx));
    open_with(&mut fx, &["down", "right", "down", "enter"])?;
    ensure!(
        options(&fx).organize == Organize::List,
        "{:?}",
        options(&fx)
    );
    ensure!(
        fx.cx.debug_bounds(fx.window, "projects-header")?.is_none(),
        "In one list still shows the Projects section"
    );
    Ok(())
}

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
