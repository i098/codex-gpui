//! The ChatGPT composer's model list: the original's `chatgpt-models` query (`RJt`, `GET
//! /models`), the picker's version rows and the label its trigger shows.

use crate::{
    chatgpt,
    host::{Identity, Request},
};
use serde_json::Value;
use ui::composer::{ChatGptModelRow, ChatGptModels};

pub(crate) fn request(identity: &Identity) -> Request {
    chatgpt::web_request(
        identity,
        "GET",
        "/models?iim=false&include_icons=false&is_gizmo=false",
        None,
    )
}

/// The picker's version rows (`modelListConfig.options`, `rCn`) and the trigger label, as the
/// original's `pt` in `U3` computes it for a Chat (non-`tpp`) conversation: with several
/// intelligence presets the selected preset's title, else the version's label. Without a
/// version holding the default model, the default model's title, else `Select ChatGPT model`.
// shortcut: no display-title overrides, limits or lane matching (`mOt`); a picked version takes
// its first preset. #1543/#1544 add internal models and limits.
pub(crate) fn picker(body: &Value) -> ChatGptModels {
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
    let version_label = |v: &Value| -> String {
        [
            "display_text_for_intelligence",
            "display_text",
            "display_text_full",
            "id",
        ]
        .iter()
        .find_map(|key| str_of(v, key))
        .unwrap_or_default()
    };
    // `pt`: several presets show the selected preset's title, else the version's label.
    let label_in = |v: &Value, slug: &str| -> String {
        let (version_slugs, version_presets) = (slugs(v), presets(v));
        if version_presets.len() > 1
            && let Some(preset) = version_presets
                .iter()
                .find(|p| preset_slug(p, &version_slugs).as_deref() == Some(slug))
        {
            let category = categories.iter().find(|c| c["default_model"] == slug);
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
        version_label(v)
    };
    let selected = versions.iter().position(|v| {
        let s = slugs(v);
        s.contains(&default)
            || presets(v)
                .iter()
                .any(|p| preset_slug(p, &s).as_deref() == Some(&default))
    });
    let rows: Vec<ChatGptModelRow> = versions
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let slug = if Some(i) == selected {
                default.clone()
            } else {
                let s = slugs(v);
                presets(v)
                    .first()
                    .and_then(|p| preset_slug(p, &s))
                    .or_else(|| s.into_iter().next())
                    .unwrap_or_default()
            };
            ChatGptModelRow {
                label: version_label(v).into(),
                trigger_label: label_in(v, &slug).into(),
            }
        })
        .collect();
    let label = match selected {
        Some(i) => rows[i].trigger_label.clone(),
        None => list("models")
            .iter()
            .find(|m| m["slug"] == default.as_str())
            .and_then(|m| str_of(m, "title"))
            .unwrap_or_else(|| "Select ChatGPT model".to_owned())
            .into(),
    };
    ChatGptModels {
        label,
        rows,
        selected,
    }
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
        let picker = picker(&body);
        assert_eq!(picker.label.as_ref(), "6");
        let rows: Vec<_> = picker.rows.iter().map(|r| r.label.as_ref()).collect();
        assert_eq!((rows, picker.selected), (vec!["6"], Some(0)));
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
        let picker = picker(&body);
        assert_eq!(picker.label.as_ref(), "6 Thinking");
        assert_eq!(picker.selected, Some(1));
        // A picked version shows its first preset's label.
        let row = &picker.rows[0];
        assert_eq!((row.label.as_ref(), row.trigger_label.as_ref()), ("5", "5"));
    }

    #[test]
    fn no_matching_version_falls_back_to_the_model_title() {
        let body = json!({
            "default_model_slug": "research",
            "models": [{"slug": "research", "title": "Deep Research"}],
            "versions": [{"id": "6", "slugs": ["gpt-6"]}],
        });
        assert_eq!(picker(&body).label.as_ref(), "Deep Research");
        assert_eq!(picker(&body).selected, None);
        assert_eq!(picker(&json!({})).label.as_ref(), "Select ChatGPT model");
    }
}
