mod common;

use app::{Root, State};
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use std::{
    os::unix::fs::PermissionsExt as _,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use ui::screens::recovery::{Retry, State as RecoveryState};

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn reconnect_reloads_the_server_without_discarding_the_draft() {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    if std::env::var_os("CGX_RECOVERY_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let script = home.path().join("readback.jsonl");
        let log = home.path().join("requests.jsonl");
        let wrapper = home.path().join("codex-fixture");
        std::fs::write(&script, transcript()).unwrap();
        std::fs::write(&log, "").unwrap();
        let executable = std::env::current_exe().unwrap();
        std::fs::write(&wrapper, format!(
            "#!/bin/sh\nexport CGX_PROXY_SCRIPT={}\nexport CGX_PROXY_LOG={}\nexec {} --exact proxy --nocapture --quiet\n",
            quote(&script), quote(&log), quote(&executable),
        )).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let workspace = home.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let status = std::process::Command::new(&executable)
            .args([
                "--exact",
                "reconnect_reloads_the_server_without_discarding_the_draft",
                "--nocapture",
            ])
            .env("CGX_RECOVERY_CHILD", "1")
            .env("CGX_RECOVERY_SCRIPT", &script)
            .env("CGX_RECOVERY_CWD", &workspace)
            .env("CODEX_BIN", &wrapper)
            .env("HOME", home.path())
            .env("CODEX_HOME", home.path().join("codex"))
            .env_remove("CODEX_GPUI_ACP")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let (bridge, _) =
        bridge::Bridge::replay(Path::new(&std::env::var_os("CGX_RECOVERY_SCRIPT").unwrap()))
            .unwrap();
    let (alive, events) = async_channel::unbounded();
    let mut cx =
        HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
    cx.allow_parking();
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        cx.set_global(common::recorded_host_settings().unwrap());
        app::init(cx);
    });
    let cwd = std::env::var("CGX_RECOVERY_CWD").unwrap();
    let window = cx
        .open_window(size(px(1280.), px(832.)), |window, cx| {
            cx.new(|cx| Root::new(State::default(), bridge, events, cwd, window, cx))
        })
        .unwrap();
    let root = cx.read_window(&window, |root, _| root).unwrap();
    let home = std::env::var_os("CODEX_HOME").unwrap();
    root.update(&mut cx, |root, cx| {
        root.set_host(Arc::new(app::host::LocalHost::new(Path::new(&home))), cx)
    });
    wait(&cx, || root.read_with(&cx, |root, _| root.ready()));
    root.update(&mut cx, |root, cx| {
        root.composer(cx).update(cx, |composer, cx| {
            composer.set_text("Keep this unsent draft", cx)
        })
    });

    drop(alive);
    wait(&cx, || {
        root.read_with(&cx, |root, cx| {
            matches!(root.recovery_state(cx), Some(RecoveryState::Crashed { .. }))
        })
    });
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(Retry), cx)
    })
    .unwrap();
    wait(&cx, || root.read_with(&cx, |root, _| root.ready()));
    root.read_with(&cx, |root, cx| {
        assert!(root.recovery_state(cx).is_none());
        assert_eq!(
            root.composer(cx).read(cx).field_text(),
            "Keep this unsent draft"
        );
    });
}

#[test]
fn failed_thread_read_returns_home_on_retry() -> anyhow::Result<()> {
    let mut fx = common::Fx::with("dark", (1280., 832.), None, |line| {
        if line["msg"]["result"]["thread"]["id"] == common::GREETING {
            line["msg"]["result"]["thread"]["turns"] = serde_json::json!(false);
        }
    })?;
    fx.action(Box::new(app::GoToChat(0)))?;
    fx.wait("route error", |fx| {
        fx.read(|root, cx| matches!(root.recovery_state(cx), Some(RecoveryState::RouteError(_))))
    })?;
    fx.capture_evidence("CGX_RECOVERY_PROOF", "route-error")?;
    fx.action(Box::new(Retry))?;
    assert_eq!(fx.route(), app::Route::Home);
    assert!(fx.read(|root, cx| root.recovery_state(cx).is_none()));
    Ok(())
}

fn transcript() -> String {
    common::script(None, |_| {})
        .unwrap()
        .iter()
        .map(|line| format!("{line}\n"))
        .collect()
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
}

fn wait(cx: &HeadlessAppContext, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(Instant::now() < deadline, "recovery state did not settle");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
}
