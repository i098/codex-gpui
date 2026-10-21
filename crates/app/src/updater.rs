use bridge::Bridge;
use gpui::{App, Task};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use ui::updater::Updates;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Features {
    data: Vec<Value>,
    next_cursor: Option<String>,
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

pub fn poll_policy(bridge: Arc<Bridge>, cx: &mut App) -> Option<Task<()>> {
    if bridge.is_acp() {
        return None;
    }
    cx.default_global::<Updates>();
    Some(cx.spawn(async move |cx| {
        loop {
            let result = managed(&bridge).await;
            cx.update(|cx| match result {
                Ok(managed) => {
                    if cx.global::<Updates>().managed != managed {
                        cx.global_mut::<Updates>().managed = managed;
                        cx.refresh_windows();
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

#[cfg(test)]
mod tests {
    use super::{Features, managed};
    use serde_json::json;

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
