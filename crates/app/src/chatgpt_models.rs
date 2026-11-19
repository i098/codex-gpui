//! The ChatGPT composer's model list: the original's `chatgpt-models` query (`RJt`, `GET
//! /models`) and the label its picker trigger shows for the default selection.

use crate::{
    chatgpt,
    host::{Identity, Request},
};
use serde_json::Value;

pub(crate) fn request(identity: &Identity) -> Request {
    chatgpt::web_request(
        identity,
        "GET",
        "/models?iim=false&include_icons=false&is_gizmo=false",
        None,
    )
}

/// The trigger label of the default selection, as the original's `pt` in `U3` computes it
/// for a Chat (non-`tpp`) conversation: with several intelligence presets the default
/// preset's title, else the version's label (`rCn`). Without a matching version, the
/// default model's title, else `Select ChatGPT model`.
// shortcut: default selection only, no display-title overrides; the picker menu (#1542) adds the rest.
pub(crate) fn trigger_label(body: &Value) -> String {
    let list = |key: &str| body[key].as_array().map(Vec::as_slice).unwrap_or_default();
    let str_of = |v: &Value, key: &str| v[key].as_str().map(str::to_owned);
    let categories: Vec<&Value> = list("categories")
        .iter()
        .filter(|c| c["disabled_by_admin"] != true)
        .collect();
    let versions: Vec<&Value> = list("versions")
        .iter()
        .filter(|v| v["disabled"] != true)
        .collect();
    let slugs = |v: &Value| -> Vec<String> {
        v["slugs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    let default = str_of(body, "default_model_slug")
        .or_else(|| versions.first().and_then(|v| slugs(v).into_iter().next()))
        .unwrap_or_else(|| "auto".to_owned());
    // `eCn`: an `instant` preset runs the version's `auto`-lane category default.
    let preset_slug = |preset: &Value, version_slugs: &[String]| -> Option<String> {
        let auto = (preset["lane"] == "instant")
            .then(|| {
                categories.iter().find_map(|c| {
                    let model = c["default_model"].as_str()?;
                    (c["model_lane"] == "auto" && version_slugs.iter().any(|s| s == model))
                        .then(|| model.to_owned())
                })
            })
            .flatten();
        auto.or_else(|| str_of(preset, "model_slug"))
    };
    let presets = |v: &Value| -> Vec<Value> {
        v["intelligence_presets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| !p.is_null())
            .cloned()
            .collect()
    };
    let version = versions.iter().find(|v| {
        let s = slugs(v);
        s.contains(&default)
            || presets(v)
                .iter()
                .any(|p| preset_slug(p, &s).as_deref() == Some(&default))
    });
    let Some(version) = version else {
        return list("models")
            .iter()
            .find(|m| m["slug"] == default.as_str())
            .and_then(|m| str_of(m, "title"))
            .unwrap_or_else(|| "Select ChatGPT model".to_owned());
    };
    let version_slugs = slugs(version);
    let version_presets = presets(version);
    if version_presets.len() > 1
        && let Some(preset) = version_presets
            .iter()
            .find(|p| preset_slug(p, &version_slugs).as_deref() == Some(&default))
    {
        let category = categories
            .iter()
            .find(|c| c["default_model"] == default.as_str());
        let title = str_of(preset, "title")
            .or_else(|| category.and_then(|c| str_of(c, "human_category_short_name")))
            .or_else(|| category.and_then(|c| str_of(c, "human_category_name")));
        let label = if preset["selected_display_version"].is_null() {
            title
        } else {
            str_of(preset, "selected_display_title").or(title)
        };
        if let Some(label) = label {
            return label;
        }
    }
    [
        "display_text_for_intelligence",
        "display_text",
        "display_text_full",
        "id",
    ]
    .iter()
    .find_map(|key| str_of(version, key))
    .unwrap_or_else(|| "Select ChatGPT model".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_preset_shows_the_version_label() {
        // The recorded Go account's `/models` (trimmed).
        let body = json!({
            "default_model_slug": "gpt-6",
            "models": [{"slug": "gpt-6", "title": "GPT-6"}],
            "categories": [
                {"category": "gpt_5_auto", "model_lane": "auto", "default_model": "gpt-6",
                 "human_category_name": "Instant"},
            ],
            "versions": [{
                "id": "6", "display_text": "Latest • 6", "display_text_for_intelligence": "6",
                "slugs": ["gpt-6", "gpt-6-t-mini"],
                "intelligence_presets": [{"title": "Instant", "selected_display_title": "Instant",
                    "model_slug": "gpt-6", "lane": "instant"}],
            }],
        });
        assert_eq!(trigger_label(&body), "6");
    }

    #[test]
    fn several_presets_show_the_default_preset() {
        let body = json!({
            "default_model_slug": "gpt-6-thinking",
            "categories": [],
            "versions": [
                {"id": "5", "display_text_for_intelligence": "5", "slugs": ["gpt-5"]},
                {"id": "6", "display_text_for_intelligence": "6",
                 "slugs": ["gpt-6", "gpt-6-thinking"],
                 "intelligence_presets": [
                    {"title": "Instant", "model_slug": "gpt-6", "lane": "instant"},
                    {"title": "Thinking", "selected_display_title": "6 Thinking",
                     "selected_display_version": "6", "model_slug": "gpt-6-thinking",
                     "lane": "thinking"},
                 ]},
            ],
        });
        assert_eq!(trigger_label(&body), "6 Thinking");
    }

    #[test]
    fn no_matching_version_falls_back_to_the_model_title() {
        let body = json!({
            "default_model_slug": "research",
            "models": [{"slug": "research", "title": "Deep Research"}],
            "versions": [{"id": "6", "slugs": ["gpt-6"]}],
        });
        assert_eq!(trigger_label(&body), "Deep Research");
        assert_eq!(trigger_label(&json!({})), "Select ChatGPT model");
    }
}
