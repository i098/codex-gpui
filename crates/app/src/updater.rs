#[cfg(target_os = "macos")]
use crate::native_menus::CheckForUpdates;
use bridge::Bridge;
#[cfg(target_os = "macos")]
use gpui::Global;
use gpui::{App, Task};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use ui::updater::Updates;
#[cfg(target_os = "macos")]
use ui::updater::{InstallUpdate, ToggleAutomaticChecks};

#[cfg(target_os = "macos")]
mod native;

#[cfg(target_os = "macos")]
struct Runtime {
    _events: Task<()>,
    backend: native::Backend,
}

#[cfg(target_os = "macos")]
impl Global for Runtime {}

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
                        first = false;
                        cx.global_mut::<Updates>().managed = managed;
                        #[cfg(target_os = "macos")]
                        configure(managed, cx);
                        let new = (
                            cx.global::<Updates>().managed,
                            cx.global::<Updates>().available,
                        );
                        if old != new {
                            crate::native_menus::refresh_menus(cx);
                            cx.refresh_windows();
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
        match native::start() {
            Ok(Some((backend, events))) => {
                cx.global_mut::<Updates>().available = true;
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
    use super::{Features, managed, validate};
    use serde_json::json;

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
