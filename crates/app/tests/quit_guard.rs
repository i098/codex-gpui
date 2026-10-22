#![cfg(not(target_os = "macos"))]

mod common;

use anyhow::{Result, ensure};
use app::{
    Root, State,
    host::{Host as _, LocalHost, Request},
    windows,
};
use gpui::{AppContext as _, TestAppContext, TestDispatcher};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
fn quitting_from_a_window_keeps_the_scheduled_work_confirmation() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(home.path()));
    host.fetch(&Request::post(
        "create-automation",
        Some(serde_json::json!({
            "name":"Quit guard", "prompt":"Owned fixture",
            "rrule":"FREQ=WEEKLY;BYDAY=SU;BYHOUR=23;BYMINUTE=59",
            "cwds":[home.path()]
        })),
    ))?;
    let recording = tempfile::NamedTempFile::new()?;
    let transcript = common::script(None, |_| {})?;
    std::fs::write(
        recording.path(),
        transcript
            .into_iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )?;
    let (bridge, events) = bridge::Bridge::replay(recording.path())?;
    let dispatcher = TestDispatcher::new(0);
    dispatcher.allow_parking();
    let mut cx = TestAppContext::build_with_text_system(dispatcher, None, textsys::text_system());
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        cx.set_global(common::recorded_host_settings().unwrap());
        app::init(cx);
    });
    let cwd = home.path().to_string_lossy().into_owned();
    let window = cx.update(|cx| {
        windows::open(
            windows::Spec::new(windows::Kind::Primary, "Quit guard"),
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut root = Root::new(State::default(), bridge, events, cwd, window, cx);
                    root.set_host(host, cx);
                    root
                })
            },
        )
    })?;
    let root = cx.read_window(&window, |root, _| root)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !root.read_with(&cx, |root, _| root.ready()) {
        ensure!(Instant::now() < deadline, "Root did not initialize");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(app::Quit), cx);
    })?;
    cx.run_until_parked();
    assert!(
        cx.has_pending_prompt(),
        "Quit discarded the active scheduled task"
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);

    std::fs::create_dir_all(home.path().join("sqlite"))?;
    std::fs::write(home.path().join("sqlite/codex.db"), b"not a database")?;
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(app::Quit), cx);
    })?;
    cx.run_until_parked();
    assert!(
        cx.has_pending_prompt(),
        "a task-storage failure must still offer Quit and Cancel"
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    Ok(())
}
