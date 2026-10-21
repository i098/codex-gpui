use super::{NAV, SettingsRoute};
use crate::screens::sidebar::MatchPattern;
use gpui::AssetSource as _;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    id: String,
    default_message: String,
}

struct Document {
    messages: Vec<Message>,
    groups: Vec<Vec<usize>>,
    lower: Vec<String>,
}

static DOCUMENTS: LazyLock<HashMap<String, Document>> = LazyLock::new(|| {
    let bytes = theme::Assets
        .load("app/settings-search.json")
        .expect("embedded settings search")
        .expect("settings search catalog");
    let source: HashMap<String, Vec<Message>> =
        serde_json::from_slice(&bytes).expect("valid settings search catalog");
    source
        .into_iter()
        .map(|(section, messages)| {
            let indexes: HashMap<_, _> = messages
                .iter()
                .enumerate()
                .map(|(index, message)| (message.id.as_str(), index))
                .collect();
            let groups = messages
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    let mut group = vec![index];
                    let mut id = message.id.as_str();
                    while let Some((prefix, _)) = id.rsplit_once('.') {
                        if let Some(&index) = indexes.get(prefix) {
                            group.push(index);
                        }
                        id = prefix;
                    }
                    group
                })
                .collect();
            let lower = messages
                .iter()
                .map(|message| message.default_message.to_lowercase())
                .collect();
            (
                section,
                Document {
                    messages,
                    groups,
                    lower,
                },
            )
        })
        .collect()
});

#[derive(Clone)]
pub(super) struct SearchResult {
    pub route: SettingsRoute,
    pub label: String,
    pub panel: &'static str,
    pub icon: &'static str,
    pub target: String,
    priority: u8,
    score: i32,
    order: usize,
}

pub(super) fn search(query: &str) -> Vec<SearchResult> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let full = MatchPattern::new(query);
    let words: Vec<_> = query
        .split_whitespace()
        .map(|word| (word.to_lowercase(), MatchPattern::new(word)))
        .collect();
    let mut results = Vec::new();
    for (order, &(panel, icon)) in NAV.iter().flat_map(|(_, rows)| rows.iter()).enumerate() {
        if panel == "Profile" {
            continue;
        }
        let Some(route) = SettingsRoute::for_section(panel) else {
            continue;
        };
        let slug = route.path.strip_prefix("/settings/").unwrap_or_default();
        let direct = full.score(panel);
        if direct > 0 {
            results.push(SearchResult {
                route,
                label: panel.into(),
                panel,
                icon,
                target: panel.into(),
                priority: 0,
                score: direct,
                order,
            });
            continue;
        }
        let Some(document) = DOCUMENTS.get(slug) else {
            continue;
        };
        let panel_matches: Vec<_> = words
            .iter()
            .map(|(_, pattern)| pattern.score(panel) > 0)
            .collect();
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for group in &document.groups {
            if words.iter().enumerate().all(|(index, (word, pattern))| {
                panel_matches[index]
                    || group.iter().any(|&entry| {
                        document.lower[entry].contains(word)
                            && pattern.score(&document.messages[entry].default_message) > 0
                    })
            }) {
                for &entry in group {
                    if seen.insert(document.messages[entry].default_message.as_str()) {
                        candidates.push(entry);
                    }
                }
            }
        }
        let terms: Vec<_> = match slug {
            "connections" => vec!["Mobile", "Phone", "Pairing", "Remote control"],
            "chronicle" => vec!["Chronicle"],
            "general-settings" => super::general::LANGUAGES
                .iter()
                .flat_map(|(_, native, english)| [*native, *english])
                .collect(),
            _ => Vec::new(),
        };
        let term_score = words
            .iter()
            .map(|(_, pattern)| {
                terms
                    .iter()
                    .map(|text| pattern.score(text))
                    .max()
                    .unwrap_or(0)
                    .max(pattern.score(panel))
            })
            .try_fold(0i32, |sum, score| {
                (score > 0).then_some(sum.saturating_add(score))
            })
            .unwrap_or(0);
        let message_score = words
            .iter()
            .map(|(word, pattern)| {
                candidates
                    .iter()
                    .filter(|&&entry| document.lower[entry].contains(word))
                    .map(|&entry| pattern.score(&document.messages[entry].default_message))
                    .max()
                    .unwrap_or(0)
                    .max(pattern.score(panel))
            })
            .try_fold(0i32, |sum, score| {
                (score > 0).then_some(sum.saturating_add(score))
            })
            .unwrap_or(0);
        if term_score == 0 && message_score == 0 {
            continue;
        }
        let term_best = terms
            .iter()
            .map(|&text| (text, best_score(text, &full, &words)))
            .reduce(|best, candidate| {
                if candidate.1 > best.1 {
                    candidate
                } else {
                    best
                }
            });
        let message_best = candidates
            .iter()
            .map(|&entry| {
                let text = document.messages[entry].default_message.as_str();
                (text, best_score(text, &full, &words))
            })
            .reduce(|best, item| if item.1 > best.1 { item } else { best });
        let use_message = term_score == 0
            || message_score > 0
                && message_best
                    .is_some_and(|(_, score)| score > term_best.map_or(0, |(_, score)| score));
        let label = if use_message {
            message_best.map(|(text, _)| text)
        } else {
            term_best.map(|(text, _)| text)
        }
        .unwrap_or(panel);
        let label = if label.contains(['<', '{']) {
            panel
        } else {
            label
        };
        results.push(SearchResult {
            route,
            label: label.into(),
            panel,
            icon,
            target: label.into(),
            priority: if use_message { 2 } else { 1 },
            score: if use_message {
                message_score
            } else {
                term_score
            },
            order,
        });
    }
    results.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.order.cmp(&b.order))
    });
    let mut seen = HashSet::new();
    results.retain(|result| seen.insert(result.route.path.clone()));
    results
}

fn best_score(text: &str, full: &MatchPattern, words: &[(String, MatchPattern)]) -> i32 {
    full.score(text).max(
        words
            .iter()
            .map(|(_, pattern)| pattern.score(text))
            .max()
            .unwrap_or(0),
    )
}

impl super::SettingsNav {
    pub(super) fn refresh_search(&mut self, query: &str) {
        if self.query_key != query {
            self.query_key = query.to_owned();
            self.results = search(query);
            self.highlighted = None;
        }
    }

    pub(super) fn activate_search(
        &mut self,
        index: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(result) = self.results.get(index).cloned() else {
            return;
        };
        self.page
            .update(cx, |page, cx| page.request_search(result, window, cx));
    }

    pub(super) fn search_row(
        &self,
        index: usize,
        result: &SearchResult,
        t: &theme::Theme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
            div, prelude::FluentBuilder as _, px,
        };
        let picked = result.clone();
        let fill = super::over(t.list_hover, super::sidebar_color(t));
        let label = if result.label == result.panel {
            result.label.clone()
        } else {
            format!("{}, {}", result.label, result.panel)
        };
        let row = div()
            .id(("settings-search-result", index))
            .role(gpui::Role::Button)
            .aria_label(label)
            .focusable()
            .tab_index(0)
            .px(px(8.))
            .py(px(4.5))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(12.5))
            .when(self.highlighted == Some(index), |d| d.bg(fill))
            .hover(move |style| style.bg(fill))
            .child(super::icon(result.icon, 16., t.text))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(super::text(result.label.clone(), 14., 21., 400., t.text))
                    .when(result.label != result.panel, |d| {
                        d.child(super::text(result.panel, 12., 16., 430., t.text_secondary))
                    }),
            )
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.highlighted != Some(index) {
                    this.highlighted = Some(index);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.page.update(cx, |page, cx| {
                    page.request_search(picked.clone(), window, cx)
                });
            }));
        div().px(px(8.)).child(row.into_any_element())
    }
}

pub(super) struct Flash {
    serial: u64,
    text: String,
    started: Option<std::time::Instant>,
    group_scrolled: bool,
    pub(super) row_bounds: Option<gpui::Bounds<gpui::Pixels>>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum TargetLevel {
    Group,
    Row { title: bool, detail: bool },
}
impl super::SettingsPane {
    pub(super) fn sync_search_flash(&mut self, cx: &mut gpui::Context<Self>) {
        let target = self.page.read(cx).search_target.as_ref();
        let mut flash = self.search_flash.borrow_mut();
        match target {
            Some((serial, text)) if flash.as_ref().is_none_or(|flash| flash.serial != *serial) => {
                *flash = Some(Flash {
                    serial: *serial,
                    text: text.clone(),
                    started: None,
                    group_scrolled: false,
                    row_bounds: None,
                });
            }
            None => *flash = None,
            _ => {}
        }
    }

    pub(super) fn target_row_index(&self, rows: &[super::SettingsRow]) -> Option<usize> {
        let flash = self.search_flash.borrow();
        let target = &flash.as_ref()?.text;
        rows.iter().position(|row| {
            row.title == target || row.detail.is_some_and(|detail| detail.contains(target))
        })
    }

    pub(super) fn row_target(&self, row: &super::SettingsRow) -> TargetLevel {
        TargetLevel::Row {
            title: self
                .search_flash
                .borrow()
                .as_ref()
                .is_some_and(|flash| flash.text == row.title),
            detail: row.detail.is_some(),
        }
    }

    pub(super) fn track_search_layout(
        &self,
        content: gpui::Div,
        index: Option<usize>,
        level: TargetLevel,
    ) -> gpui::Div {
        use gpui::prelude::FluentBuilder as _;
        content.when_some(index, |content, index| {
            let flash = self.search_flash.clone();
            let scroll = self.scroll.clone();
            // Child bounds already include the window's current scroll translation.
            content.on_children_prepainted(move |bounds, window, cx| {
                let Some(mut target) = bounds.get(index).copied() else {
                    return;
                };
                let mut flash = flash.borrow_mut();
                let Some(flash) = flash.as_mut() else {
                    return;
                };
                let row = matches!(level, TargetLevel::Row { .. });
                if row {
                    flash.row_bounds = Some(target);
                }
                if flash.started.is_some()
                    || !row && (flash.group_scrolled || flash.row_bounds.is_some())
                {
                    return;
                }
                let viewport = scroll.bounds();
                if viewport.size.height <= gpui::px(0.) || target.size.height <= gpui::px(0.) {
                    return;
                }
                if let TargetLevel::Row {
                    title,
                    detail: true,
                } = level
                {
                    target.origin.y += gpui::px(super::ROW_PADDING);
                    if title {
                        target.size.height = gpui::px(super::ROW_LABEL_LINE_HEIGHT);
                    } else {
                        target.origin.y +=
                            gpui::px(super::ROW_LABEL_LINE_HEIGHT + super::ROW_TEXT_GAP);
                        target.size.height -= gpui::px(
                            2. * super::ROW_PADDING
                                + super::ROW_LABEL_LINE_HEIGHT
                                + super::ROW_TEXT_GAP,
                        );
                    }
                }
                let offset = (scroll.offset().y + viewport.center().y - target.center().y)
                    .clamp(-scroll.max_offset().y, gpui::px(0.));
                if !row {
                    flash.group_scrolled = true;
                }
                if (scroll.offset().y - offset).abs() > gpui::px(0.5 / window.scale_factor()) {
                    let scroll = scroll.clone();
                    window.on_next_frame(move |window, _| {
                        scroll.set_offset(gpui::point(scroll.offset().x, offset));
                        window.refresh();
                    });
                } else if row {
                    flash.started = Some(cx.background_executor().now());
                }
            })
        })
    }

    pub(super) fn highlight_row(&self, row: super::SettingsRow, t: &theme::Theme) -> gpui::Div {
        use gpui::{ParentElement, Styled, canvas, div};
        let paint_flash = self.search_flash.clone();
        let title = row.title;
        let detail = row.detail;
        let matches =
            move |text: &str| title == text || detail.is_some_and(|detail| detail.contains(text));
        let color = t.ghost_hover;
        let highlight = canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let flash = paint_flash.borrow();
                let Some(flash) = flash.as_ref().filter(|flash| matches(&flash.text)) else {
                    return;
                };
                let Some(started) = flash.started else {
                    return;
                };
                let elapsed = cx
                    .background_executor()
                    .now()
                    .saturating_duration_since(started);
                if elapsed.as_millis() >= 450 {
                    return;
                }
                let alpha = if crate::motion::reduced(cx) {
                    1.
                } else {
                    1. - elapsed.as_secs_f32() / 0.45
                };
                shapes::paint(
                    &shapes::Decoration {
                        fill: Some(color.opacity(alpha)),
                        ..Default::default()
                    },
                    None,
                    bounds,
                    window,
                    cx,
                );
                window.request_animation_frame();
            },
        )
        .absolute()
        .size_full();
        div().relative().child(highlight).child(row.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_queries_find_destinations_outside_the_navigation_labels() {
        let speed = search("Speed");
        assert_eq!(speed[0].label, "Speed");
        assert_eq!(speed[0].route.path, "/settings/general-settings");
        let theme = search("Theme");
        assert!(theme.iter().any(|result| result.panel == "Appearance"));
        assert!(search("unmatched-token-937204").is_empty());
        assert!(search("   ").is_empty());
    }
}
