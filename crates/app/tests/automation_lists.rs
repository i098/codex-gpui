use app::Route;

#[test]
fn scheduled_links_decode_task_ids_without_treating_other_routes_as_tasks() {
    for url in [
        "/scheduled?automationId=weekly%20status&automationSource=local",
        "https://chatgpt.com/scheduled?automationId=weekly%20status",
        "codex-gpui://scheduled?automationId=weekly%20status",
    ] {
        assert_eq!(
            Route::from_url(url),
            Some(Route::Automation("weekly status".into()))
        );
    }
    assert_eq!(Route::from_url("/scheduled"), Some(Route::Scheduled));
    assert_eq!(
        Route::from_url("/scheduled?automationId="),
        Some(Route::Scheduled)
    );
    assert_eq!(Route::from_url("/thread/weekly"), None);
    assert_eq!(
        Route::from_url("https://example.com/scheduled?automationId=weekly"),
        None
    );
    assert_eq!(
        Route::from_url("javascript:/scheduled?automationId=weekly"),
        None
    );
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use anyhow::{Result, ensure};
    use app::host::{Host, LocalHost, Request};
    use gpui::{KeyDownEvent, KeyUpEvent, Keystroke, PlatformInput};
    use serde_json::Value;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct ControlledStore {
        local: LocalHost,
        fail: AtomicBool,
    }
    impl Host for ControlledStore {
        fn fetch(&self, request: &Request) -> Result<Value> {
            if *request == Request::post("list-automations", None) {
                ensure!(
                    !self.fail.load(Ordering::SeqCst),
                    "owned fixture read failed"
                );
            }
            self.local.fetch(request)
        }
    }

    fn visible(fx: &common::Fx) -> Vec<String> {
        fx.read(|root, cx| {
            root.scheduled_sidebar()
                .unwrap()
                .read(cx)
                .visible_tasks()
                .map(|task| task.id.clone())
                .collect()
        })
    }

    fn shot(fx: &mut common::Fx, name: &str) -> Result<()> {
        if let Some(path) = std::env::var_os("CGX_AUTOMATION_LISTS_PROOF") {
            let path = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&path)?;
            fx.reference_shot()?
                .save(path.join(format!("{name}.png")))?;
        }
        Ok(())
    }

    fn key(fx: &mut common::Fx, key: &str) -> Result<()> {
        let keystroke = Keystroke::parse(key)?;
        fx.input(PlatformInput::KeyDown(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        }))?;
        fx.input(PlatformInput::KeyUp(KeyUpEvent { keystroke }))
    }

    fn open(fx: &mut common::Fx, url: &str) -> Result<()> {
        let root = fx.root.clone();
        let handled = fx.cx.update_window(fx.window, |_, window, cx| {
            root.update(cx, |root, cx| root.open_url(url, window, cx))
        })?;
        ensure!(handled, "unhandled scheduled link");
        Ok(())
    }

    #[test]
    fn cold_selection_search_filters_and_read_failure_use_the_real_store() -> Result<()> {
        let home = tempfile::tempdir()?;
        let local = LocalHost::new(home.path());
        for (id, name, prompt, status) in [
            (
                "weekly",
                "Weekly status update",
                "Summarize this week’s progress",
                "ACTIVE",
            ),
            (
                "inbox",
                "Inbox triage",
                "Review incoming messages",
                "PAUSED",
            ),
        ] {
            local.write_automation(id, &format!("version = 1\nid = {id:?}\nkind = \"cron\"\nname = {name:?}\nprompt = {prompt:?}\nstatus = {status:?}\nrrule = \"FREQ=DAILY;BYHOUR=9\"\ncwds = [\"/tmp/project\"]\ncreated_at = 1792252800000\nupdated_at = 1792252800000\n"))?;
        }
        let host = Arc::new(ControlledStore {
            local,
            fail: AtomicBool::new(false),
        });
        let mut fx = common::Fx::new()?;
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
        open(&mut fx, "/scheduled?automationId=weekly")?;
        ensure!(fx.route() == Route::Automation("weekly".into()));
        ensure!(fx.read(|root, _| root.automations().is_none()));
        if let Some(path) = std::env::var_os("CGX_AUTOMATION_LISTS_PROOF") {
            let path = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&path)?;
            fx.cx
                .capture_screenshot(fx.window)?
                .save(path.join("loading.png"))?;
        }
        fx.wait("real task list", |fx| {
            fx.read(|root, _| matches!(root.automations(), Some(Ok(_))))
        })?;
        ensure!(visible(&fx).len() == 2);
        shot(&mut fx, "selected")?;
        if let Some(path) = std::env::var_os("CGX_AUTOMATION_LISTS_PROOF") {
            let list = fx.read(|root, _| root.automations().unwrap().as_ref().unwrap().clone());
            std::fs::write(
                std::path::PathBuf::from(path).join("list.json"),
                serde_json::to_vec(&list)?,
            )?;
        }
        fx.action(Box::new(ui::GoBack))?;
        ensure!(fx.route() == Route::Home);
        fx.action(Box::new(ui::GoForward))?;
        ensure!(fx.route() == Route::Automation("weekly".into()));

        fx.click((314., 152.))?;
        key(&mut fx, "end")?;
        shot(&mut fx, "filter-menu")?;
        key(&mut fx, "enter")?;
        ensure!(visible(&fx) == ["inbox"]);
        shot(&mut fx, "paused")?;
        fx.click((314., 152.))?;
        key(&mut fx, "home")?;
        key(&mut fx, "down")?;
        key(&mut fx, "enter")?;
        ensure!(visible(&fx) == ["weekly"]);
        fx.click((314., 152.))?;
        key(&mut fx, "home")?;
        key(&mut fx, "enter")?;
        ensure!(visible(&fx).len() == 2);

        fx.click((314., 70.))?;
        fx.type_text("incoming")?;
        ensure!(visible(&fx) == ["inbox"]);
        shot(&mut fx, "search")?;
        fx.action(Box::new(ui::SelectAll))?;
        fx.type_text("not present")?;
        ensure!(visible(&fx).is_empty());
        fx.hover((768., 6.))?;
        fx.advance(std::time::Duration::from_millis(300))?;
        shot(&mut fx, "no-match")?;
        key(&mut fx, "escape")?;
        ensure!(visible(&fx).len() == 2);
        key(&mut fx, "tab")?;
        shot(&mut fx, "focus")?;

        host.fail.store(true, Ordering::SeqCst);
        fx.action(Box::new(ui::screens::automations::ReloadTasks))?;
        fx.wait("failed list read", |fx| {
            fx.read(|root, _| matches!(root.automations(), Some(Err(_))))
        })?;
        ensure!(visible(&fx).is_empty());
        shot(&mut fx, "error")?;
        host.fail.store(false, Ordering::SeqCst);
        fx.click((100., 214.))?;
        fx.wait("retry restored list", |fx| {
            fx.read(|root, _| matches!(root.automations(), Some(Ok(_))))
        })?;
        ensure!(visible(&fx).len() == 2);
        shot(&mut fx, "retry")?;
        Ok(())
    }
}
