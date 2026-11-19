//! The ChatGPT composer's model list: the original's `chatgpt-models` query (`RJt`, `GET
//! /models`), the picker's version rows and the label its trigger shows.

use crate::{
    chatgpt,
    host::{Identity, Request},
};
use serde_json::Value;
use ui::composer::{ChatGptModelOption, ChatGptModelRow, ChatGptModels, ChatGptPick};

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
/// Also the `Model` slash command's model options (`options`) and `Change from` labels.
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
    let model = |slug: &str| list("models").iter().find(|m| m["slug"] == slug);
    let model_title = |slug: &str| model(slug).and_then(|m| str_of(m, "title"));
    // `BW`: an unset effort means the model's `default_thinking_effort`.
    let effort_of = |slug: &str, effort: Option<String>| {
        effort.or_else(|| model(slug).and_then(|m| str_of(m, "default_thinking_effort")))
    };
    // `eCn`: one intelligence preset as a model option.
    let preset_option = |preset: &Value, version_slugs: &[String]| -> Option<ModelOption> {
        let slug = preset_slug(preset, version_slugs)?;
        let category = categories
            .iter()
            .find(|c| c["default_model"] == slug.as_str());
        Some(ModelOption {
            effort: effort_of(&slug, str_of(preset, "thinking_effort")),
            title: str_of(preset, "title")
                .or_else(|| category.and_then(|c| str_of(c, "title")))
                .or_else(|| category.and_then(|c| str_of(c, "human_category_short_name")))
                .or_else(|| category.and_then(|c| str_of(c, "human_category_name")))
                .or_else(|| model_title(&slug)),
            description: category
                .and_then(|c| str_of(c, "short_explainer"))
                .or_else(|| model(&slug).and_then(|m| str_of(m, "description"))),
            selected_label: str_of(preset, "selected_display_title"),
            slug,
        })
    };
    // `ZSn`: one category as a model option.
    let category_option = |c: &Value| -> Option<ModelOption> {
        let slug = str_of(c, "default_model").or_else(|| {
            c["supported_models"]
                .as_array()?
                .iter()
                .find_map(|s| s.as_str().map(str::to_owned))
        })?;
        Some(ModelOption {
            effort: effort_of(&slug, None),
            title: str_of(c, "title")
                .or_else(|| str_of(c, "human_category_short_name"))
                .or_else(|| str_of(c, "human_category_name"))
                .or_else(|| model_title(&slug)),
            description: str_of(c, "short_explainer")
                .or_else(|| str_of(c, "tagline"))
                .or_else(|| model(&slug).and_then(|m| str_of(m, "description"))),
            selected_label: None,
            slug,
        })
    };
    // `rCn`'s options: the version's presets, else its categories.
    let version_options = |v: &Value| -> Vec<ModelOption> {
        let s = slugs(v);
        let version_presets = presets(v);
        if version_presets.is_empty() {
            categories
                .iter()
                .filter(|c| s.iter().any(|slug| c["default_model"] == slug.as_str()))
                .filter_map(|c| category_option(c))
                .collect()
        } else {
            version_presets
                .iter()
                .filter_map(|p| preset_option(p, &s))
                .collect()
        }
    };
    // `d`: the first version's presets, else the categories (reversed), else the models;
    // one row per slug and effort (`aCn`).
    let from_categories: Vec<ModelOption> = categories
        .iter()
        .rev()
        .filter_map(|c| category_option(c))
        .collect();
    let mut options: Vec<ModelOption> = versions
        .iter()
        .find(|v| !presets(v).is_empty())
        .map(|v| version_options(v))
        .filter(|o| !o.is_empty())
        .or_else(|| (!from_categories.is_empty()).then(|| from_categories.clone()))
        .unwrap_or_else(|| {
            list("models")
                .iter()
                .filter_map(|m| {
                    let slug = str_of(m, "slug")?;
                    Some(ModelOption {
                        effort: effort_of(&slug, None),
                        title: str_of(m, "title"),
                        description: str_of(m, "description"),
                        selected_label: None,
                        slug,
                    })
                })
                .collect()
        });
    let mut seen = std::collections::HashSet::new();
    options.retain(|o| seen.insert((o.slug.clone(), o.effort.clone())));
    // `USn` searches these and every version's options for the selection's name.
    let all_options: Vec<ModelOption> = options
        .iter()
        .cloned()
        .chain(versions.iter().flat_map(|v| version_options(v)))
        .collect();
    let pick = |slug: String, effort: Option<String>, row: Option<usize>| -> ChatGptPick {
        let effort = effort_of(&slug, effort);
        let found = all_options
            .iter()
            .find(|o| o.slug == slug && o.effort == effort)
            .or_else(|| all_options.iter().find(|o| o.slug == slug));
        let name = found
            .and_then(|o| o.selected_label.clone().or_else(|| o.title.clone()))
            .or_else(|| model_title(&slug));
        let trigger_label = match row {
            Some(i) => label_in(versions[i], &slug),
            None => model_title(&slug).unwrap_or_else(|| "Select ChatGPT model".to_owned()),
        };
        ChatGptPick {
            change_from: name
                .map(|name| format!("Change from {name}"))
                .unwrap_or_default()
                .into(),
            trigger_label: trigger_label.into(),
            slug: slug.into(),
            effort: effort.map(Into::into),
            row,
        }
    };
    let rows: Vec<ChatGptModelRow> = versions
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (slug, effort) = if Some(i) == selected {
                (default.clone(), None)
            } else {
                let s = slugs(v);
                let first = presets(v).into_iter().next();
                let slug = first
                    .as_ref()
                    .and_then(|p| preset_slug(p, &s))
                    .or_else(|| s.into_iter().next())
                    .unwrap_or_default();
                (slug, first.and_then(|p| str_of(&p, "thinking_effort")))
            };
            ChatGptModelRow {
                label: version_label(v).into(),
                pick: pick(slug, effort, Some(i)),
            }
        })
        .collect();
    // `jo(option, versionOptions[0].id)`: a picked option checks the first version.
    let first_row = (!versions.is_empty()).then_some(0);
    let options = options
        .into_iter()
        .map(|o| ChatGptModelOption {
            // `t.title ?? "Model"`.
            title: o.title.unwrap_or_else(|| "Model".to_owned()).into(),
            description: o.description.unwrap_or_default().into(),
            pick: pick(o.slug, o.effort, first_row),
        })
        .collect();
    ChatGptModels {
        current: pick(default, None, selected),
        rows,
        options,
    }
}

/// One model option before it becomes a [`ChatGptModelOption`]; `selected_label` names a
/// selection of it in `Change from {model}`.
#[derive(Clone)]
struct ModelOption {
    slug: String,
    effort: Option<String>,
    title: Option<String>,
    description: Option<String>,
    selected_label: Option<String>,
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
                 "human_category_name": "Instant", "short_explainer": "Decides how long to think"},
            ],
            "versions": [{
                "id": "6", "display_text": "Latest • 6", "display_text_for_intelligence": "6",
                "slugs": ["gpt-6", "gpt-6-t-mini"],
                "intelligence_presets": [{"title": "Instant", "selected_display_title": "Instant",
                    "model_slug": "gpt-6", "lane": "instant"}],
            }],
        });
        let picker = picker(&body);
        assert_eq!(picker.current.trigger_label.as_ref(), "6");
        let rows: Vec<_> = picker.rows.iter().map(|r| r.label.as_ref()).collect();
        assert_eq!((rows, picker.current.row), (vec!["6"], Some(0)));
        // The `Model` slash command: the preset as its one option, named by its display title.
        assert_eq!(picker.current.change_from.as_ref(), "Change from Instant");
        let options: Vec<_> = picker
            .options
            .iter()
            .map(|o| {
                (
                    o.title.as_ref(),
                    o.description.as_ref(),
                    o.pick.slug.as_ref(),
                )
            })
            .collect();
        assert_eq!(
            options,
            vec![("Instant", "Decides how long to think", "gpt-6")]
        );
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
        assert_eq!(picker.current.trigger_label.as_ref(), "6 Thinking");
        assert_eq!(picker.current.row, Some(1));
        assert_eq!(
            picker.current.change_from.as_ref(),
            "Change from 6 Thinking"
        );
        // A picked version shows its first preset's label.
        let row = &picker.rows[0];
        assert_eq!(
            (row.label.as_ref(), row.pick.trigger_label.as_ref()),
            ("5", "5")
        );
        // The options are the first version with presets; a pick checks the first version.
        let options: Vec<_> = picker.options.iter().map(|o| o.title.as_ref()).collect();
        assert_eq!(options, vec!["Instant", "Thinking"]);
        assert_eq!(
            picker.options[0].pick.change_from.as_ref(),
            "Change from Instant"
        );
        assert_eq!(picker.options[1].pick.row, Some(0));
    }

    #[test]
    fn no_matching_version_falls_back_to_the_model_title() {
        let body = json!({
            "default_model_slug": "research",
            "models": [{"slug": "research", "title": "Deep Research"}],
            "versions": [{"id": "6", "slugs": ["gpt-6"]}],
        });
        assert_eq!(
            picker(&body).current.trigger_label.as_ref(),
            "Deep Research"
        );
        assert_eq!(picker(&body).current.row, None);
        assert_eq!(
            picker(&body).current.change_from.as_ref(),
            "Change from Deep Research"
        );
        assert_eq!(
            picker(&json!({})).current.trigger_label.as_ref(),
            "Select ChatGPT model"
        );
    }
}
