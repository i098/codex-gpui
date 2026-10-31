#[cfg(target_os = "macos")]
use crate::native_menus::CheckForUpdates;
use bridge::Bridge;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use gpui::Global;
use gpui::{
    App, AppContext as _, Bounds, Context, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, actions, div, px, size,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::process::CommandExt as _;
use std::{sync::Arc, time::Duration};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use theme::ActiveTheme as _;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use ui::text;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use ui::updater::InstallUpdate;
#[cfg(target_os = "macos")]
use ui::updater::ToggleAutomaticChecks;
use ui::updater::Updates;

#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "linux")]
#[path = "updater/linux.rs"]
mod native;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod package;

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct Runtime {
    _events: Task<()>,
    #[cfg(target_os = "macos")]
    _settings: gpui::Subscription,
    backend: native::Backend,
    #[cfg(target_os = "macos")]
    installation: String,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Global for Runtime {}

/// The signed-in ChatGPT plan that the appcast URL carries.
struct Plan(String);

impl gpui::Global for Plan {}

/// The original's production appcast URL (`hn({beta, installationId, planType})`):
/// `None` on an architecture that has no build.
#[cfg(any(target_os = "macos", test))]
fn feed_url(
    base: &str,
    installation: &str,
    arch: &str,
    version: &str,
    beta: bool,
    os_version: &str,
    plan: &str,
) -> Option<String> {
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => return None,
    };
    let mut url = url::Url::parse(base).ok()?;
    url.query_pairs_mut()
        .append_pair("installation_id", installation)
        .append_pair("arch", arch)
        .append_pair("app_version", version)
        .append_pair("beta", if beta { "true" } else { "false" })
        .append_pair("os-version", os_version)
        .append_pair("plan_type", plan);
    Some(url.into())
}

/// Stores the account plan for the appcast URL.
pub fn set_plan(plan: Option<&str>, cx: &mut App) {
    let plan = plan.filter(|plan| !plan.is_empty()).unwrap_or("unknown");
    if cx.try_global::<Plan>().is_some_and(|old| old.0 == plan) {
        return;
    }
    cx.set_global(Plan(plan.to_owned()));
    #[cfg(target_os = "macos")]
    apply_preferences(cx);
}

#[cfg(target_os = "macos")]
fn host_flag(cx: &App, key: &str, default: bool) -> bool {
    cx.try_global::<crate::host_settings::Settings>()
        .and_then(|settings| settings.read_all().get(key).and_then(Value::as_bool))
        .unwrap_or(default)
}

/// The beta setting and the account plan.
#[cfg(target_os = "macos")]
fn preferences(cx: &App) -> (bool, String) {
    let plan = cx
        .try_global::<Plan>()
        .map_or("unknown", |plan| plan.0.as_str());
    (host_flag(cx, "update-beta", false), plan.to_owned())
}

/// Applies the current preferences to a running Sparkle updater.
#[cfg(target_os = "macos")]
fn apply_preferences(cx: &mut App) {
    let (beta, plan) = preferences(cx);
    if let Some(runtime) = cx.try_global::<Runtime>() {
        runtime
            .backend
            .configure(beta, &plan, &runtime.installation);
    }
}

/// A new lowercase random UUID, as the original's `randomUUID()`.
#[cfg(target_os = "macos")]
fn new_uuid() -> String {
    objc2_foundation::NSUUID::new()
        .UUIDString()
        .to_string()
        .to_lowercase()
}

/// The stored installation id, created and saved on first use.
#[cfg(target_os = "macos")]
fn installation_id(cx: &mut App) -> String {
    const KEY: &str = "update-installation-id";
    let Some(settings) = cx.try_global::<crate::host_settings::Settings>() else {
        return new_uuid();
    };
    if let Some(id) = settings.read_all().get(KEY).and_then(Value::as_str) {
        return id.to_owned();
    }
    let id = new_uuid();
    if let Err(error) = cx
        .global_mut::<crate::host_settings::Settings>()
        .write(KEY, Value::String(id.clone()))
    {
        eprintln!("codex-gpui: cannot save update installation id: {error:#}");
    }
    id
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Features {
    data: Vec<Value>,
    next_cursor: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
fn validate(feed: &str, identity: &str) -> anyhow::Result<()> {
    use anyhow::{Context as _, ensure};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let url = url::Url::parse(feed).context("invalid SUFeedURL")?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "update feed requires HTTPS"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "update feed must not contain credentials"
    );
    ensure!(
        STANDARD
            .decode(identity)
            .context("invalid SUPublicEDKey")?
            .len()
            == 32,
        "SUPublicEDKey must contain a 32-byte Ed25519 public key"
    );
    Ok(())
}

impl Features {
    fn disabled(&self) -> Option<bool> {
        self.data
            .iter()
            .find(|feature| feature["name"] == "in_app_updates")
            .map(|feature| feature["enabled"].as_bool() == Some(false))
    }
}

async fn managed(bridge: &Bridge) -> anyhow::Result<bool> {
    let mut cursor = None;
    loop {
        let page: Features = bridge
            .request(
                "experimentalFeature/list",
                json!({"cursor": cursor, "limit": 100}),
            )
            .await?;
        if let Some(disabled) = page.disabled() {
            return Ok(disabled);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(false);
        }
    }
}

pub fn init(cx: &mut App) {
    if cx.has_global::<Updates>() {
        return;
    }
    cx.set_global(Updates::default());
    #[cfg(target_os = "macos")]
    {
        cx.on_action(|_: &CheckForUpdates, cx| {
            if let Some(runtime) = cx.try_global::<Runtime>() {
                runtime.backend.check();
            }
        });
        cx.on_action(|_: &ToggleAutomaticChecks, cx| {
            if let Some(runtime) = cx.try_global::<Runtime>() {
                let on = !runtime.backend.automatic();
                runtime.backend.set_automatic(on);
                cx.global_mut::<Updates>().automatic = on;
                cx.refresh_windows();
            }
        });
        cx.on_action(|_: &InstallUpdate, cx| {
            if !cx.global::<Updates>().managed
                && let Some(runtime) = cx.try_global::<Runtime>()
            {
                runtime.backend.install();
            }
        });
    }
    #[cfg(target_os = "linux")]
    cx.on_action(|_: &InstallUpdate, cx| {
        if !cx.global::<Updates>().managed
            && let Some(runtime) = cx.try_global::<Runtime>()
        {
            runtime.backend.install();
        }
    });
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    init_restart_action(cx);
}

pub fn poll_policy(bridge: Arc<Bridge>, cx: &mut App) -> Option<Task<()>> {
    if bridge.is_acp() {
        return None;
    }
    cx.default_global::<Updates>();
    Some(cx.spawn(async move |cx| {
        let mut first = true;
        loop {
            let result = managed(&bridge).await;
            cx.update(|cx| match result {
                Ok(managed) => {
                    let old = (
                        cx.global::<Updates>().managed,
                        cx.global::<Updates>().available,
                    );
                    if first || old.0 != managed {
                        let changed = !first && old.0 != managed;
                        first = false;
                        cx.global_mut::<Updates>().managed = managed;
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        configure(managed, cx);
                        let new = (
                            cx.global::<Updates>().managed,
                            cx.global::<Updates>().available,
                        );
                        if old != new {
                            crate::native_menus::refresh_menus(cx);
                            cx.refresh_windows();
                        }
                        // The original prompts a restart when the workspace policy changes under
                        // a running app; our polled org-policy signal is `in_app_updates`.
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        if changed {
                            prompt_restart(cx);
                        }
                    }
                }
                Err(error) => eprintln!("codex-gpui: cannot read update policy: {error:#}"),
            });
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
        }
    }))
}

/// The workspace-policy restart prompt (`desktop.workspacePolicyRestart`), shown once per
/// policy flip while the app runs.
#[derive(Default)]
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct RestartPrompt(Option<WindowHandle<RestartWindow>>);
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Global for RestartPrompt {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn prompt_restart(cx: &mut App) {
    if let Some(handle) = cx.try_global::<RestartPrompt>().and_then(|state| state.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(420.), px(180.)), cx);
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Restart to apply workspace settings".into()),
                ..Default::default()
            }),
            is_resizable: false,
            ..Default::default()
        },
        |window, cx| {
            let panel = cx.new(|cx| RestartWindow {
                focus: cx.focus_handle(),
            });
            let focus = panel.read(cx).focus.clone();
            window.focus(&focus, cx);
            panel
        },
    ) {
        Ok(handle) => cx.default_global::<RestartPrompt>().0 = Some(handle),
        Err(error) => eprintln!("codex-gpui: cannot open restart prompt: {error:#}"),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct RestartWindow {
    focus: FocusHandle,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Focusable for RestartWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Render for RestartWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let button_color = if t.appearance == theme::Appearance::Dark {
            gpui::rgb(0x2d2d2d).into()
        } else {
            t.text_inverse
        };
        div()
            .id("restart-window")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .bg(t.menu_background)
            .text_color(t.text)
            .child(
                text("Restart to apply workspace settings", 20., 28., 600., t.text)
                    .whitespace_normal(),
            )
            .child(
                text(
                    "Restart Codex to apply your workspace settings. Active local chats will be interrupted.",
                    14.,
                    20.,
                    400.,
                    t.text_secondary,
                )
                .whitespace_normal(),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .id("restart-confirm")
                            .role(gpui::Role::Button)
                            .aria_label("Restart")
                            .h(px(32.))
                            .px(px(17.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(12.5))
                            .bg(t.button_primary_solid)
                            .hover(|s| s.opacity(0.85))
                            .child(text("Restart", 14., 18., 430., button_color))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(RestartApp), cx);
                                cx.stop_propagation();
                            }),
                    ),
            )
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
actions!(updater, [RestartApp]);

/// `Restart`: the original `app.relaunch()` + quit; here one exec in place.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn relaunch() -> std::io::Error {
    let exe = std::env::current_exe().expect("running binary path");
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .exec()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn init_restart_action(cx: &mut App) {
    cx.on_action(|_: &RestartApp, _| {
        eprintln!("codex-gpui: restarting for the workspace policy");
        let error = relaunch();
        eprintln!("codex-gpui: cannot relaunch: {error:#}");
    });
}

#[cfg(target_os = "macos")]
fn configure(managed: bool, cx: &mut App) {
    if let Some(runtime) = cx.try_global::<Runtime>() {
        runtime.backend.set_managed(managed);
        cx.global_mut::<Updates>().available = !managed;
        if managed {
            cx.global_mut::<Updates>().status = ui::updater::Status::Idle;
        }
        return;
    }
    if !managed {
        let installation = installation_id(cx);
        let (beta, plan) = preferences(cx);
        match native::start(beta, &plan, &installation) {
            Ok(Some((backend, events))) => {
                cx.global_mut::<Updates>().available = true;
                backend.check_at_start();
                cx.global_mut::<Updates>().automatic = backend.automatic();
                let events = cx.spawn(async move |cx| {
                    while let Ok(status) = events.recv().await {
                        cx.update(|cx| {
                            if !cx.global::<Updates>().managed {
                                cx.global_mut::<Updates>().status = status;
                                cx.refresh_windows();
                            }
                        });
                    }
                });
                let settings =
                    cx.observe_global::<crate::host_settings::Settings>(apply_preferences);
                cx.set_global(Runtime {
                    _events: events,
                    _settings: settings,
                    backend,
                    installation,
                });
            }
            Ok(None) => {}
            Err(error) => eprintln!("codex-gpui: cannot start updater: {error:#}"),
        }
    }
}

#[cfg(target_os = "linux")]
fn configure(managed: bool, cx: &mut App) {
    if let Some(runtime) = cx.try_global::<Runtime>() {
        runtime.backend.set_managed(managed);
        cx.global_mut::<Updates>().available = !managed;
        if managed {
            cx.global_mut::<Updates>().status = ui::updater::Status::Idle;
        }
        return;
    }
    if !managed {
        match native::start() {
            Ok(Some((backend, events))) => {
                cx.global_mut::<Updates>().available = true;
                let events = cx.spawn(async move |cx| {
                    while let Ok(status) = events.recv().await {
                        cx.update(|cx| {
                            if !cx.global::<Updates>().managed {
                                cx.global_mut::<Updates>().status = status;
                                cx.refresh_windows();
                            }
                        });
                    }
                });
                cx.set_global(Runtime {
                    _events: events,
                    backend,
                });
            }
            Ok(None) => {}
            Err(error) => eprintln!("codex-gpui: cannot start updater: {error:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Features, feed_url, managed, validate};
    use serde_json::json;

    #[test]
    fn feed_url_carries_the_original_appcast_parameters() {
        let url = feed_url(
            "https://updates.example/appcast.xml?channel=a",
            "id 1",
            "aarch64",
            "26.1.0",
            true,
            "15.1.0",
            "plus",
        );
        assert_eq!(
            url.as_deref(),
            Some(
                "https://updates.example/appcast.xml?channel=a&installation_id=id+1&arch=arm64\
                 &app_version=26.1.0&beta=true&os-version=15.1.0&plan_type=plus"
            )
        );
        let intel = feed_url(
            "https://u.example/f",
            "i",
            "x86_64",
            "1.0.0",
            false,
            "14",
            "free",
        );
        assert!(intel.is_some_and(|url| url.contains("arch=x64") && url.contains("beta=false")));
        assert_eq!(
            feed_url("https://u.example/f", "i", "riscv64", "1", false, "1", "x"),
            None
        );
    }

    #[test]
    fn updater_rejects_untrusted_feed_configuration() {
        let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        assert!(validate("https://updates.example/appcast.xml", key).is_ok());
        assert!(validate("http://127.0.0.1:1234/appcast.xml", key).is_ok());
        for feed in [
            "http://updates.example/appcast.xml",
            "file:///tmp/update.xml",
            "https://user:pass@updates.example/feed",
        ] {
            assert!(validate(feed, key).is_err());
        }
        for identity in ["", "bad-key", "AA=="] {
            assert!(validate("https://updates.example/feed", identity).is_err());
        }
    }

    #[test]
    fn managed_policy_uses_only_an_explicit_disabled_flag() {
        for enabled in [true, false] {
            let page: Features = serde_json::from_value(json!({
                "data": [{"name": "in_app_updates", "enabled": enabled}], "nextCursor": null
            }))
            .unwrap();
            assert_eq!(page.disabled(), Some(!enabled));
        }
        let page: Features = serde_json::from_value(json!({
            "data": [{"name": "other", "enabled": false}], "nextCursor": "second"
        }))
        .unwrap();
        assert_eq!(page.disabled(), None);
        assert_eq!(page.next_cursor.as_deref(), Some("second"));
        let missing: Features = serde_json::from_value(json!({
            "data": [{"name": "in_app_updates"}], "nextCursor": null
        }))
        .unwrap();
        assert_eq!(missing.disabled(), Some(false));
    }

    #[test]
    fn policy_on_a_later_page_is_read() -> anyhow::Result<()> {
        use std::io::Write as _;

        for (last_page, expected) in [
            (json!([{"name": "in_app_updates", "enabled": false}]), true),
            (json!([]), false),
        ] {
            let mut fixture = tempfile::NamedTempFile::new()?;
            for record in [
                json!({"dir":"send","msg":{"id":1,"method":"experimentalFeature/list","params":{"cursor":null,"limit":100}}}),
                json!({"dir":"recv","msg":{"id":1,"result":{"data":[{"name":"future_feature"}],"nextCursor":"page2"}}}),
                json!({"dir":"send","msg":{"id":2,"method":"experimentalFeature/list","params":{"cursor":"page2","limit":100}}}),
                json!({"dir":"recv","msg":{"id":2,"result":{"data":last_page,"nextCursor":null}}}),
            ] {
                writeln!(fixture, "{record}")?;
            }
            let (bridge, _) = bridge::Bridge::replay(fixture.path())?;
            assert_eq!(pollster::block_on(managed(&bridge))?, expected);
        }
        Ok(())
    }
}
