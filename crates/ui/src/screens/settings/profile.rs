//! Settings ▸ Profile (`/settings/profile`): the read-only header and v2 `Insights` of
//! `GET /wham/profiles/me`. The original's edit, showcase and sharing dialogs are not here.

use crate::screens::computer_use::{Load, group};
use crate::{focus, text};
use gpui::{
    AnyElement, App, Context, Div, Global, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window,
    div, img, px,
};
use serde_json::Value;
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub image_url: Option<String>,
    pub stats: Option<Stats>,
    pub stats_error: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub current_streak_days: Option<i64>,
    pub longest_streak_days: Option<i64>,
    pub longest_running_turn_sec: Option<i64>,
    pub peak_daily_tokens: Option<i64>,
    pub lifetime_tokens: Option<i64>,
    pub total_threads: Option<i64>,
    pub total_skills_used: Option<i64>,
    pub most_used_reasoning_effort: Option<String>,
    pub fast_mode_usage_percentage: Option<f64>,
}

/// Loads (or reloads) the profile; the app installs it.
pub type Dispatch = Arc<dyn Fn(&mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub profile: Load<Arc<Profile>>,
    pub page_open: bool,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

/// `M9i`: a trimmed string, `None` when absent or blank.
fn trimmed(value: &Value) -> Option<String> {
    let s = value.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// The original's `/wham/profiles/me` mapping (`app-initial`, `e9i`).
pub fn parse(body: &Value) -> Profile {
    let profile = &body["profile"];
    let stats = &body["stats"];
    let int = |key: &str| stats[key].as_f64().map(|n| n as i64);
    Profile {
        display_name: trimmed(&profile["display_name"]),
        username: trimmed(&profile["username"]),
        image_url: trimmed(&profile["profile_picture_url"]),
        stats: stats.is_object().then(|| Stats {
            current_streak_days: int("current_streak_days"),
            longest_streak_days: int("longest_streak_days"),
            longest_running_turn_sec: int("longest_running_turn_sec"),
            peak_daily_tokens: int("peak_daily_tokens"),
            lifetime_tokens: int("lifetime_tokens"),
            total_threads: int("total_threads"),
            total_skills_used: int("total_skills_used"),
            most_used_reasoning_effort: trimmed(&stats["most_used_reasoning_effort"]),
            fast_mode_usage_percentage: stats["fast_mode_usage_percentage"].as_f64(),
        }),
        stats_error: trimmed(&body["metadata"]["stats_error"]).is_some(),
    }
}

/// `t9i`: first letters of the first and last name words, uppercased, else `?`.
pub fn initials(name: Option<&str>) -> String {
    let words: Vec<&str> = name.unwrap_or_default().split_whitespace().collect();
    let first = words.first().and_then(|w| w.chars().next());
    let last = (words.len() > 1)
        .then(|| words.last().and_then(|w| w.chars().next()))
        .flatten();
    let s: String = first
        .into_iter()
        .chain(last)
        .collect::<String>()
        .to_uppercase();
    if s.is_empty() { "?".into() } else { s }
}

/// `profile.stats.dayStreakValue`.
fn days(n: i64) -> String {
    if n == 1 {
        "1 day".into()
    } else {
        format!("{n} days")
    }
}

/// `profile.stats.duration*`: `{hours}h {minutes}m`, `{minutes}m {seconds}s`, `{seconds}s`.
fn duration(sec: i64) -> String {
    let (h, m, s) = (sec / 3600, sec % 3600 / 60, sec % 60);
    match (h, m, s) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, 0, _) => format!("{h}h"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// Compact token count (`1.2M`).
// shortcut: English compact suffixes only, matches `Intl` compact for en-US.
fn tokens(n: i64) -> String {
    let n = n as f64;
    let (value, suffix) = match n.abs() {
        v if v >= 1e9 => (n / 1e9, "B"),
        v if v >= 1e6 => (n / 1e6, "M"),
        v if v >= 1e3 => (n / 1e3, "K"),
        _ => return format!("{n}"),
    };
    let rounded = if value.abs() < 10. {
        (value * 10.).round() / 10.
    } else {
        value.round()
    };
    format!("{rounded}{suffix}")
}

/// The composer's reasoning labels (`composer.mode.local.reasoning.*.label`).
fn effort(value: &str) -> String {
    match value {
        "none" => "None".into(),
        "minimal" => "Minimal".into(),
        "low" => "Light".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra High".into(),
        "max" => "Max".into(),
        "ultra" => "Ultra".into(),
        other => other.to_owned(),
    }
}

/// The `Insights` rows with a value (labels: `profile.stats.*`, `profile.activity.features.*`).
fn stat_rows(stats: &Stats) -> Vec<(&'static str, String)> {
    [
        ("Current streak", stats.current_streak_days.map(days)),
        ("Longest streak", stats.longest_streak_days.map(days)),
        ("Longest task", stats.longest_running_turn_sec.map(duration)),
        ("Peak tokens", stats.peak_daily_tokens.map(tokens)),
        ("Lifetime tokens", stats.lifetime_tokens.map(tokens)),
        ("Total chats", stats.total_threads.map(|n| n.to_string())),
        (
            "Total skills used",
            stats.total_skills_used.map(|n| n.to_string()),
        ),
        (
            "Most used reasoning",
            stats.most_used_reasoning_effort.as_deref().map(effort),
        ),
        (
            "Fast Mode",
            stats
                .fast_mode_usage_percentage
                .map(|p| format!("{}% of runs", p.round() as i64)),
        ),
    ]
    .into_iter()
    .filter_map(|(label, value)| Some((label, value?)))
    .collect()
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("profile-settings", cx, |_, cx| {
            cx.observe_global::<State>(|_, cx| cx.notify()).detach();
            cx.default_global::<State>().page_open = true;
            cx.on_release(|_: &mut ProfilePage, cx| {
                if cx.has_global::<State>() {
                    cx.update_global::<State, _>(|state, _| state.page_open = false);
                }
            })
            .detach();
            if let Some(dispatch) = cx.global::<State>().dispatch.clone() {
                dispatch(cx);
            }
            ProfilePage {
                scroll: ScrollHandle::new(),
            }
        })
        .into_any_element()
}

struct ProfilePage {
    scroll: ScrollHandle,
}

impl ProfilePage {
    fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
            dispatch(cx);
        }
    }

    fn retry_button(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle("profile-retry", 0, window, cx);
        focus::ring(
            super::button("Retry", t.text_alpha_5, None, &t).id("profile-retry"),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.retry(cx)))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.retry(cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }
}

fn avatar(profile: &Profile, t: &Theme) -> AnyElement {
    let size = px(64.);
    match &profile.image_url {
        Some(url) => img(SharedString::from(url.clone()))
            .size(size)
            .rounded_full()
            .into_any_element(),
        None => div()
            .size(size)
            .flex_none()
            .rounded_full()
            .bg(gpui::Hsla { a: 0.1, ..t.text })
            .flex()
            .items_center()
            .justify_center()
            .child(text(
                initials(profile.display_name.as_deref()),
                22.,
                28.,
                500.,
                t.text_secondary,
            ))
            .into_any_element(),
    }
}

fn stat_row(label: &'static str, value: String, t: &Theme) -> Div {
    div()
        .min_h(px(48.))
        .px(px(16.))
        .py(px(12.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.))
        .child(text(label, 13., 18.571_428, 500., t.text))
        .child(text(value, 13., 18.571_428, 430., t.text_secondary))
}

fn message(message: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .p(px(16.))
        .child(text(message, 13., 18.571_428, 430., t.text_secondary))
}

impl Render for ProfilePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        let body = match &state.profile {
            Load::Loading => message("Loading…", &t).into_any_element(),
            Load::Failed(error) => div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(24.))
                .child(
                    message(error.clone(), &t)
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal(),
                )
                .child(self.retry_button(window, cx))
                .into_any_element(),
            Load::Ready(profile) => {
                let rows = match (&profile.stats, profile.stats_error) {
                    (Some(stats), false) => stat_rows(stats)
                        .into_iter()
                        .map(|(label, value)| stat_row(label, value, &t))
                        .collect(),
                    _ => vec![message("Activity insights unavailable", &t)],
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(40.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(avatar(profile, &t))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(text(
                                        profile
                                            .display_name
                                            .clone()
                                            .unwrap_or_else(|| "ChatGPT user".into()),
                                        20.,
                                        28.,
                                        500.,
                                        t.text,
                                    ))
                                    .children(profile.username.as_ref().map(|name| {
                                        text(format!("@{name}"), 14., 21., 430., t.text_secondary)
                                    })),
                            ),
                    )
                    .child(group("Insights", rows, None, &t))
                    .into_any_element()
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(focus::tab)
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("profile-settings-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div().pt(px(16.)).child(
                            div()
                                .w_full()
                                .max_w(px(768.))
                                .mx_auto()
                                .px(px(20.))
                                .py(px(12.))
                                .child(text("Profile", 28., 33.6, 500., t.text)),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .p(px(20.))
                            .child(body),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_trims_and_maps_fields() {
        let profile = parse(&json!({
            "profile": {"username": " ada ", "display_name": "  ", "profile_picture_url": null},
            "stats": {"current_streak_days": 3, "lifetime_tokens": 1_250_000,
                      "most_used_reasoning_effort": "xhigh", "fast_mode_usage_percentage": 12.4},
            "metadata": {"stats_error": " "}
        }));
        assert_eq!(profile.username.as_deref(), Some("ada"));
        assert_eq!(profile.display_name, None);
        assert_eq!(profile.image_url, None);
        assert!(!profile.stats_error);
        let rows = stat_rows(profile.stats.as_ref().unwrap());
        assert_eq!(
            rows,
            vec![
                ("Current streak", "3 days".to_owned()),
                ("Lifetime tokens", "1.3M".to_owned()),
                ("Most used reasoning", "Extra High".to_owned()),
                ("Fast Mode", "12% of runs".to_owned()),
            ]
        );
        assert!(parse(&json!({"metadata": {"stats_error": "boom"}})).stats_error);
        assert_eq!(parse(&json!({})).stats, None);
    }

    #[test]
    fn initials_use_first_and_last_word() {
        assert_eq!(initials(Some("ada byron lovelace")), "AL");
        assert_eq!(initials(Some(" grace ")), "G");
        assert_eq!(initials(Some("  ")), "?");
        assert_eq!(initials(None), "?");
    }

    #[test]
    fn durations_follow_the_original_formats() {
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(120), "2m");
        assert_eq!(duration(125), "2m 5s");
        assert_eq!(duration(7200), "2h");
        assert_eq!(duration(7380), "2h 3m");
    }
}
