//! Settings ▸ File type handlers (`file-type-handlers-*.js`): one row per file extension an
//! installed plugin's MCP tool declares as a `file` entrypoint (`_meta["openai/ui"]`), with a
//! menu of `Built-in` and those plugin viewers. The choice is stored in the host setting
//! `preferred-file-viewers`, keyed like the original's `["local", "<ext>"]`.

use super::{SettingsPane, button, settings_row_content};
use crate::screens::computer_use::Load;
use crate::settings_kit::{Choice, Select, Selected};
use gpui::{
    AppContext as _, Context, Div, InteractiveElement, IntoElement,
    StatefulInteractiveElement as _, Window,
};
use serde_json::{Value, json};
use theme::Theme;

pub const VIEWERS: &str = "host.preferred-file-viewers";
const BUILT_IN: &str = "default";

/// A plugin tool that opens files, identified like the original's `yIa`:
/// `[server, tool, resourceUri, pluginId]` as JSON.
#[derive(Debug, PartialEq)]
pub struct Viewer {
    pub id: String,
    pub title: String,
    pub extensions: Vec<String>,
}

/// `Uzi`: `" .PDF"` → `"pdf"`.
fn normalize(extension: &str) -> String {
    extension.trim().trim_start_matches('.').to_lowercase()
}

/// The file viewers in the `{"data": [server]}` that `mcpServerStatus/list` paging gives:
/// tools with a `ui://` resource and at least one `file` entrypoint that lists extensions.
pub fn viewers(servers: &Value) -> Vec<Viewer> {
    let mut viewers = Vec::new();
    for server in servers["data"].as_array().into_iter().flatten() {
        let Some(name) = server["name"].as_str() else {
            continue;
        };
        for (tool_name, tool) in server["tools"].as_object().into_iter().flatten() {
            let meta = &tool["_meta"];
            let resource = meta["ui"]["resourceUri"]
                .as_str()
                .or_else(|| meta["ui/resourceUri"].as_str())
                .or_else(|| meta["openai/outputTemplate"].as_str());
            let Some(resource) = resource.filter(|uri| uri.starts_with("ui://")) else {
                continue;
            };
            let extensions: Vec<String> = meta["openai/ui"]["entrypoints"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["type"] == "file")
                .flat_map(|entry| entry["extensions"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .map(normalize)
                .filter(|extension| !extension.is_empty())
                .collect();
            if extensions.is_empty() {
                continue;
            }
            let tool_name = tool["name"].as_str().unwrap_or(tool_name);
            let title = tool["title"]
                .as_str()
                .or_else(|| tool["annotations"]["title"].as_str())
                .unwrap_or(tool_name);
            viewers.push(Viewer {
                id: json!([name, tool_name, resource, server["pluginId"]]).to_string(),
                title: title.to_owned(),
                extensions,
            });
        }
    }
    viewers
}

/// The extensions with a viewer, sorted and without duplicates.
fn extensions(viewers: &[Viewer]) -> Vec<String> {
    let mut extensions: Vec<String> = viewers
        .iter()
        .flat_map(|viewer| viewer.extensions.iter().cloned())
        .collect();
    extensions.sort();
    extensions.dedup();
    extensions
}

/// The preference key of `extension` on the local host.
fn key(extension: &str) -> String {
    json!(["local", extension]).to_string()
}

impl SettingsPane {
    pub(super) fn file_type_handlers_page(
        &mut self,
        t: &Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let integrations = self.page.read(cx).integrations.read(cx);
        let (servers, failed) = match &integrations.servers {
            Load::Ready(servers) => {
                let failed = servers["data"]
                    .as_array()
                    .is_some_and(|all| all.iter().any(|s| !s["toolsError"].is_null()));
                ((**servers).clone(), failed)
            }
            Load::Failed(_) => (Value::Null, true),
            Load::Loading => (Value::Null, false),
        };
        let loading = matches!(integrations.servers, Load::Loading);
        let dispatch = integrations.dispatch.clone();
        let viewers = viewers(&servers);
        let mut rows: Vec<Div> = extensions(&viewers)
            .into_iter()
            .map(|extension| self.handler_row(&extension, &viewers, t, cx))
            .collect();
        if failed {
            let retry = button("Retry", t.text_alpha_5, None, t)
                .id("retry-file-type-handlers")
                .role(gpui::Role::Button)
                .aria_label("Retry")
                .on_click(move |_, _, cx| {
                    if let Some(dispatch) = &dispatch {
                        dispatch(super::integrations::Action::Refresh, cx);
                    }
                })
                .into_any_element();
            rows.push(settings_row_content(
                "Couldn’t load file handlers",
                None,
                retry,
                t,
            ));
        } else if rows.is_empty() {
            let message = if loading {
                "Loading file handlers"
            } else {
                "No installed plugins support file types"
            };
            rows.push(settings_row_content(
                message,
                None,
                gpui::div().into_any_element(),
                t,
            ));
        }
        vec![(
            "Select which plugins should open specific types of files",
            rows,
        )]
    }

    fn handler_row(
        &mut self,
        extension: &str,
        viewers: &[Viewer],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut handlers: Vec<&Viewer> = viewers
            .iter()
            .filter(|viewer| viewer.extensions.iter().any(|e| e == extension))
            .collect();
        handlers.sort_by(|a, b| a.title.cmp(&b.title));
        let key = key(extension);
        let saved = self.page.read(cx).get(VIEWERS)[&key]
            .as_str()
            .unwrap_or(BUILT_IN);
        // A saved viewer that is no longer installed falls back to the built-in viewer.
        let current = if handlers.iter().any(|viewer| viewer.id == saved) {
            saved.to_owned()
        } else {
            BUILT_IN.to_owned()
        };
        let choices = std::iter::once(Choice {
            value: BUILT_IN.into(),
            label: "Built-in".into(),
            checked: false,
            disabled: false,
        })
        .chain(handlers.iter().map(|viewer| Choice {
            value: viewer.id.clone().into(),
            label: viewer.title.clone().into(),
            checked: false,
            disabled: false,
        }))
        .collect();
        let page = self.page.clone();
        let select = self
            .file_selects
            .entry(key.clone())
            .or_insert_with(|| {
                let select = cx.new(|cx| Select::new(current.clone(), Vec::new(), None, cx));
                cx.subscribe(&select, move |_, _, Selected(selected), cx| {
                    page.update(cx, |page, cx| {
                        let mut all = page.get(VIEWERS).clone();
                        if !all.is_object() {
                            all = json!({});
                        }
                        all[&key] = json!(selected.as_ref());
                        page.set(VIEWERS, all, cx);
                    });
                })
                .detach();
                select
            })
            .clone();
        select.update(cx, |select, cx| {
            select.set_choices(choices, cx);
            select.set_value(current, cx);
        });
        settings_row_content(format!(".{extension}"), None, select.into_any_element(), t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, tools: Value) -> Value {
        json!({"name": name, "pluginId": "p@m", "tools": tools})
    }

    #[test]
    fn only_ui_tools_with_file_entrypoints_are_viewers() {
        let servers = json!({"data": [server(
            "docs",
            json!({
                "open": {"name": "open", "title": "Docs", "_meta": {
                    "ui": {"resourceUri": "ui://docs/app"},
                    "openai/ui": {"entrypoints": [
                        {"type": "global"},
                        {"type": "file", "extensions": [" .PDF", "docx", "."]}
                    ]}
                }},
                "web": {"name": "web", "_meta": {
                    "ui": {"resourceUri": "https://docs/app"},
                    "openai/ui": {"entrypoints": [{"type": "file", "extensions": ["pdf"]}]}
                }},
                "global": {"name": "global", "_meta": {
                    "ui": {"resourceUri": "ui://docs/global"},
                    "openai/ui": {"entrypoints": [{"type": "global"}]}
                }}
            })
        )]});
        assert_eq!(
            viewers(&servers),
            vec![Viewer {
                id: r#"["docs","open","ui://docs/app","p@m"]"#.into(),
                title: "Docs".into(),
                extensions: vec!["pdf".into(), "docx".into()],
            }]
        );
    }

    #[test]
    fn extensions_are_sorted_once() {
        let viewer = |extensions: &[&str]| Viewer {
            id: String::new(),
            title: String::new(),
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
        };
        assert_eq!(
            extensions(&[viewer(&["pdf", "csv"]), viewer(&["csv"])]),
            ["csv", "pdf"]
        );
    }
}
