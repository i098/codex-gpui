mod common;

use app::{Root, Route, State};
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
fn local_links_preserve_prefill_and_reject_foreign_hosts() {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    if std::env::var_os("CGX_LINK_SMOKE_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "local_links_preserve_prefill_and_reject_foreign_hosts",
                "--nocapture",
            ])
            .env("CGX_LINK_SMOKE_CHILD", "1")
            .env("HOME", home.path())
            .env("CODEX_HOME", home.path().join("codex"))
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let lines = common::script(None, |_| {}).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let (bridge, events) = bridge::Bridge::replay(file.path()).unwrap();
    let mut cx =
        HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
    cx.allow_parking();
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        cx.set_global(common::recorded_host_settings().unwrap());
        app::init(cx);
    });
    let handle = cx
        .open_window(size(px(1280.), px(832.)), |window, cx| {
            cx.new(|cx| {
                Root::new(
                    State::default(),
                    bridge,
                    events,
                    "/owned".into(),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    let root = cx.read_window(&handle, |root, _| root).unwrap();
    let home = std::env::var_os("CODEX_HOME").unwrap();
    root.update(&mut cx, |root, cx| {
        root.set_host(Arc::new(app::host::LocalHost::new(Path::new(&home))), cx)
    });
    wait(&cx, || root.read_with(&cx, |root, _| root.ready()));

    let accepted = handle
        .update(&mut cx, |root, window, cx| {
            root.receive_deep_link("codex://threads/new?prompt=Review+%CE%BB", window, cx)
        })
        .unwrap();
    assert!(accepted);
    root.read_with(&cx, |root, cx| {
        assert_eq!(root.route(), &Route::Home);
        assert_eq!(root.composer(cx).read(cx).field_text(), "Review λ");
    });

    let url = format!(
        "codex://threads/{}?prompt=Keep+this+draft",
        common::GREETING
    );
    assert!(
        handle
            .update(&mut cx, |root, window, cx| root
                .receive_deep_link(&url, window, cx))
            .unwrap()
    );
    wait(&cx, || {
        root.read_with(&cx, |root, cx| {
            root.thread()
                .is_some_and(|thread| thread.read(cx).id() == common::GREETING)
        })
    });
    root.read_with(&cx, |root, cx| {
        assert_eq!(root.route(), &Route::Thread(common::GREETING.into()));
        assert_eq!(root.composer(cx).read(cx).field_text(), "Keep this draft");
    });

    let foreign = format!("codex://threads/{}?hostId=remote", common::SUBTRACT);
    assert!(
        !handle
            .update(&mut cx, |root, window, cx| root
                .receive_deep_link(&foreign, window, cx))
            .unwrap()
    );
    assert!(
        !handle
            .update(&mut cx, |root, window, cx| root.receive_deep_link(
                "codex://threads/not-a-thread",
                window,
                cx
            ))
            .unwrap()
    );
    root.read_with(&cx, |root, cx| {
        assert_eq!(root.route(), &Route::Thread(common::GREETING.into()));
        assert_eq!(root.composer(cx).read(cx).field_text(), "Keep this draft");
    });
}

fn wait(cx: &HeadlessAppContext, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(Instant::now() < deadline, "deep-link state did not settle");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
}
