//! Office add-in live-control rows for Computer use settings.
//! The original shows one row per installed and enabled Microsoft Office plugin on macOS and
//! Windows; every row reflects the one document-control connector's `isEnabled` (default on).

use serde_json::Value;
use ui::screens::computer_use::{OfficeApp, OfficeControl};

/// Connector the original toggles through `apps.<id>.enabled`.
pub const DOCUMENT_CONTROL_APP_ID: &str = "connector_openai_codex_document_control";

fn plugin_app(plugin: &Value) -> Option<OfficeApp> {
    let name = plugin["name"]
        .as_str()
        .or_else(|| plugin["id"].as_str()?.split('@').next())?
        .to_ascii_lowercase();
    OfficeApp::ALL
        .into_iter()
        .find(|app| name.contains(app.plugin_name()))
}

/// `marketplaces` is the `plugin/list` `marketplaces` array; `apps` the `app/list` data.
pub fn controls(marketplaces: &Value, apps: &[Value]) -> Vec<OfficeControl> {
    if !cfg!(any(target_os = "macos", target_os = "windows")) {
        return Vec::new();
    }
    controls_on_supported_platform(marketplaces, apps)
}

fn controls_on_supported_platform(marketplaces: &Value, apps: &[Value]) -> Vec<OfficeControl> {
    let enabled = apps
        .iter()
        .find(|app| app["id"] == DOCUMENT_CONTROL_APP_ID)
        .and_then(|app| app["isEnabled"].as_bool())
        .unwrap_or(true);
    let active: Vec<OfficeApp> = marketplaces
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|marketplace| marketplace["plugins"].as_array().into_iter().flatten())
        .filter(|plugin| plugin["installed"] == true && plugin["enabled"] == true)
        .filter_map(plugin_app)
        .collect();
    OfficeApp::ALL
        .into_iter()
        .filter(|app| active.contains(app))
        .map(|app| OfficeControl { app, enabled })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plugin(name: &str, installed: bool, enabled: bool) -> Value {
        json!({"id": format!("{name}@openai-bundled"), "name": name,
               "installed": installed, "enabled": enabled})
    }

    #[test]
    fn only_installed_and_enabled_plugins_get_rows_in_original_order() {
        let marketplaces = json!([{"name": "openai-bundled", "plugins": [
            plugin("microsoft-word", true, true),
            plugin("microsoft-excel", true, true),
            plugin("microsoft-powerpoint", true, false),
            plugin("chrome", true, true),
        ]}]);
        let rows = controls_on_supported_platform(&marketplaces, &[]);
        let apps: Vec<_> = rows.iter().map(|row| row.app).collect();
        assert_eq!(apps, [OfficeApp::Excel, OfficeApp::Word]);
        assert!(rows.iter().all(|row| row.enabled));
    }

    #[test]
    fn connector_flag_sets_every_row_and_defaults_on() {
        let marketplaces = json!([{"plugins": [plugin("microsoft-excel", true, true)]}]);
        let off = [json!({"id": DOCUMENT_CONTROL_APP_ID, "isEnabled": false})];
        assert!(!controls_on_supported_platform(&marketplaces, &off)[0].enabled);
        let other = [json!({"id": "other", "isEnabled": false})];
        assert!(controls_on_supported_platform(&marketplaces, &other)[0].enabled);
    }
}
