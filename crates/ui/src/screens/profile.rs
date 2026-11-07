//! `/profile`: the original's `ProfilePage` (profile v2, `profile-page-*.js`) for the signed-in
//! user. It reads `GET /profiles/me/page` (`$9i`) and maps the body as the original's `rea`
//! does: identity, bio, the stats strip, the token activity chart, top plugins and insights.

use crate::screens::settings::button;
use crate::{EmptySidebar, Shell, focus, icon, text};
use chrono::{Datelike as _, Days, Months, NaiveDate};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, actions, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

actions!(
    profile,
    [
        /// Settings ▸ `Profile` and the command menu's `Profile` (`appRoute: "/profile"`).
        OpenProfile
    ]
);

/// One `activity_graph` bucket: `start_date` and `tokens`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bucket {
    pub date: NaiveDate,
    pub tokens: f64,
}

/// `page.stats`; the original's `summary`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub current_streak_days: Option<u64>,
    pub longest_streak_days: Option<u64>,
    pub lifetime_tokens: Option<f64>,
    pub peak_tokens: Option<f64>,
    pub longest_task_secs: Option<f64>,
}

/// `page.insights`. A `None` row value with `Some` presence shows `Not used`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Insights {
    /// `agentic.fast_mode_usage_percentage`; the outer `None` hides the row.
    pub fast_mode_percent: Option<Option<f64>>,
    /// `agentic.most_used_reasoning_effort` and its percentage; `None` hides the row.
    pub reasoning: Option<(Option<String>, Option<f64>)>,
    pub skills_explored: Option<u64>,
    pub total_skills_used: Option<u64>,
}

/// A `page.top_plugins` item with a name and a usage count.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub logo_url: Option<String>,
}

/// `page.display_settings` (`tea`): sections the owner shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sections {
    pub usage_stats: bool,
    pub activity_graph: bool,
    pub insights: bool,
    pub top_plugins: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Period {
    #[default]
    Daily,
    Weekly,
    Cumulative,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub image_url: Option<String>,
    pub bio: String,
    /// `None` when `page.stats` is missing (`hasStatsError`).
    pub summary: Option<Summary>,
    /// Daily, weekly and cumulative buckets, as the original's `activity`.
    pub activity: [Option<Vec<Bucket>>; 3],
    /// `None` when both `insights` and `top_plugins` are missing.
    pub insights: Option<Insights>,
    /// `None` when `top_plugins` is missing.
    pub plugins: Option<Vec<Plugin>>,
    pub sections: Sections,
}

impl Profile {
    /// The heading: display name, else the username before any `~suffix`, else the fallback.
    pub fn name(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.local_username())
            .unwrap_or("ChatGPT user")
    }

    pub fn local_username(&self) -> Option<&str> {
        let name = self.username.as_deref()?;
        Some(name.split_once('~').map_or(name, |(local, _)| local))
    }

    /// Up to two initials (`t9i`): the first and last word's first letters.
    pub fn initials(&self) -> String {
        let words: Vec<&str> = self.name().split_whitespace().collect();
        let first = words.first().and_then(|w| w.chars().next());
        let last = (words.len() > 1)
            .then(|| words.last().and_then(|w| w.chars().next()))
            .flatten();
        let initials: String = first.into_iter().chain(last).collect();
        if initials.is_empty() {
            "?".into()
        } else {
            initials.to_uppercase()
        }
    }
}

fn trimmed(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn buckets(value: &Value) -> Result<Option<Vec<Bucket>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let items = value.as_array().ok_or("activity buckets are not a list")?;
    items
        .iter()
        .map(|item| {
            let date = item["start_date"]
                .as_str()
                .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
                .ok_or("bucket without a start_date")?;
            Ok(Bucket {
                date,
                tokens: item["tokens"].as_f64().unwrap_or(0.),
            })
        })
        .collect::<Result<_, String>>()
        .map(Some)
}

/// The `/profiles/me/page` body, as the original's `rea` maps it.
pub fn parse(body: &Value) -> Result<Profile, String> {
    let details = body
        .get("profile_details")
        .filter(|v| v.is_object())
        .ok_or("missing profile_details")?;
    let page = body
        .get("page")
        .filter(|v| v.is_object())
        .ok_or("missing page")?;
    let stats = &page["stats"];
    let summary = (!stats.is_null()).then(|| Summary {
        current_streak_days: stats["current_streak_days"].as_u64(),
        longest_streak_days: stats["longest_streak_days"].as_u64(),
        lifetime_tokens: stats["agentic"]["lifetime_tokens"].as_f64(),
        peak_tokens: stats["agentic"]["peak_daily_tokens"].as_f64(),
        longest_task_secs: stats["agentic"]["longest_running_turn_sec"].as_f64(),
    });
    let insights = &page["insights"];
    let agentic = &insights["agentic"];
    let top = &page["top_plugins"];
    let plugins = top.as_array().map(|items| {
        items
            .iter()
            .filter(|item| !item["usage_count"].is_null())
            .filter_map(|item| {
                let id = item["plugin_id"].as_str().unwrap_or_default().to_owned();
                let name = trimmed(&item["plugin_name"]).or_else(|| trimmed(&item["plugin_id"]))?;
                Some(Plugin {
                    id,
                    name,
                    logo_url: trimmed(&item["logo_url"]),
                })
            })
            .take(10)
            .collect()
    });
    let graph = &page["activity_graph"];
    let settings = &page["display_settings"];
    let shown = |key: &str| settings[key].as_bool().unwrap_or(true);
    Ok(Profile {
        display_name: trimmed(&details["display_name"]),
        username: trimmed(&details["username"]),
        image_url: trimmed(&details["profile_picture_url"]),
        bio: details["description"]
            .as_str()
            .unwrap_or_default()
            .replace(['\r', '\n', '\u{2028}', '\u{2029}'], " "),
        summary,
        activity: [
            buckets(&graph["daily_usage_buckets"])?,
            buckets(&graph["weekly_usage_buckets"])?,
            buckets(&graph["cumulative_daily_usage_buckets"])?,
        ],
        insights: (!insights.is_null() || !top.is_null()).then(|| Insights {
            fast_mode_percent: agentic.get("fast_mode_usage_percentage").map(Value::as_f64),
            reasoning: agentic
                .get("most_used_reasoning_effort")
                .and_then(|effort| {
                    let percent = agentic.get("most_used_reasoning_effort_percentage")?;
                    Some((trimmed(effort), percent.as_f64()))
                }),
            skills_explored: insights["unique_skills_used"].as_u64(),
            total_skills_used: insights["total_skills_used"].as_u64(),
        }),
        plugins,
        sections: Sections {
            usage_stats: shown("show_usage_stats_section"),
            // A workspace that turns activity off hides the graph for everyone.
            activity_graph: shown("show_activity_graph_section")
                && page["workspace_display_settings"]["activity_enabled"] != false,
            insights: shown("show_insights_section"),
            top_plugins: shown("show_top_plugins_section"),
        },
    })
}

/// `Intl.NumberFormat` `notation: "compact"`, `maximumFractionDigits: 1` (en-US).
pub fn compact(value: f64) -> String {
    const UNITS: [(f64, &str); 5] = [(1., ""), (1e3, "K"), (1e6, "M"), (1e9, "B"), (1e12, "T")];
    let value = value.max(0.);
    let scaled = |i: usize| (value / UNITS[i].0 * 10.).round() / 10.;
    let mut i = UNITS
        .iter()
        .rposition(|(scale, _)| value >= *scale)
        .unwrap_or(0);
    if scaled(i) >= 1000. && i + 1 < UNITS.len() {
        i += 1;
    }
    let number = format!("{:.1}", scaled(i));
    format!("{}{}", number.trim_end_matches(".0"), UNITS[i].1)
}

/// `Intl.NumberFormat` default (en-US): grouped integer.
pub fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The original's `sea`: `{h}h {m}m`, `{m}m {s}s` or `{s}s`, dropping a zero second part.
pub fn duration(secs: f64) -> String {
    let n = secs.max(0.).round() as u64;
    if n >= 3600 {
        let minutes = (n as f64 / 60.).round() as u64;
        let (hours, minutes) = (minutes / 60, minutes % 60);
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    } else if n >= 60 {
        let (minutes, seconds) = (n / 60, n % 60);
        if seconds == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {seconds}s")
        }
    } else {
        format!("{n}s")
    }
}

fn days(count: u64) -> String {
    if count == 1 {
        "1 day".into()
    } else {
        format!("{count} days")
    }
}

/// The composer's reasoning effort labels (`Mo`); unknown values show as sent.
pub fn effort(value: &str) -> String {
    match value {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Light",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        "ultra" => "Ultra",
        "persistent" => "Persistent",
        other => other,
    }
    .into()
}

fn percent(value: f64) -> String {
    format!("{}%", value.round())
}

/// The chart's first week (`I9i`), its column cap (`P9i`) and its month label cap (`F9i`).
const CHART_EPOCH: (i32, u32, u32) = (2025, 7, 13);
const MAX_COLUMNS: usize = 52;
const MAX_MONTHS: u32 = 12;

fn week_start(date: NaiveDate) -> NaiveDate {
    date - Days::new(date.weekday().num_days_from_sunday().into())
}

/// Columns (`h9i`): weeks from the epoch through today's week, 1 to 52.
pub fn columns(today: NaiveDate) -> usize {
    let (y, m, d) = CHART_EPOCH;
    let epoch = NaiveDate::from_ymd_opt(y, m, d).expect("valid epoch");
    let weeks = (week_start(today) - epoch).num_days().div_euclid(7);
    (weeks + 1).clamp(1, MAX_COLUMNS as i64) as usize
}

/// The first cell's date (`v9i`): a Sunday.
fn chart_start(today: NaiveDate) -> NaiveDate {
    week_start(today) - Days::new((columns(today) as u64 - 1) * 7)
}

fn cell_date(index: usize, today: NaiveDate) -> NaiveDate {
    chart_start(today) + Days::new(index as u64)
}

/// Per-column totals (`f9i`): the sum, or the latest bucket per week for `last`.
fn weekly(buckets: &[Bucket], today: NaiveDate, last: bool) -> Vec<f64> {
    let start = chart_start(today);
    let mut totals = vec![0.; columns(today)];
    let mut sorted = buckets.to_vec();
    if last {
        sorted.sort_by_key(|b| b.date);
    }
    for bucket in sorted.iter().filter(|b| b.date <= today) {
        let offset = (week_start(bucket.date) - start).num_days();
        if offset < 0 || offset % 7 != 0 {
            continue;
        }
        if let Some(total) = totals.get_mut((offset / 7) as usize) {
            *total = if last { 0. } else { *total } + bucket.tokens.max(0.);
        }
    }
    totals
}

/// Bars (`S9i`): each column fills `ceil(v / max * 7)` cells from the bottom, at level 4.
fn bars(values: &[f64]) -> Vec<u8> {
    let max = values.iter().copied().fold(0., f64::max);
    (0..values.len() * 7)
        .map(|index| {
            let value = values[index / 7];
            let height = if value <= 0. || max <= 0. {
                0
            } else {
                ((value / max * 7.).ceil() as usize).max(1)
            };
            if height == 0 || 7 - index % 7 > height {
                0
            } else {
                4
            }
        })
        .collect()
}

/// Daily levels (`y9i`): 1-4 by quarter of the 95th percentile of the nonzero days.
fn heat(values: &[f64]) -> Vec<u8> {
    let mut positive: Vec<f64> = values.iter().copied().filter(|v| *v > 0.).collect();
    positive.sort_by(f64::total_cmp);
    let top = if positive.is_empty() {
        0.
    } else {
        let rank = (positive.len() - 1) as f64 * 0.95;
        let low = rank.floor() as usize;
        let at = positive[low];
        at + (positive.get(low + 1).copied().unwrap_or(at) - at) * (rank - low as f64)
    };
    values
        .iter()
        .map(|&v| {
            if v <= 0. || top <= 0. {
                return 0;
            }
            match v / top {
                r if r > 0.75 => 4,
                r if r > 0.5 => 3,
                r if r > 0.25 => 2,
                _ => 1,
            }
        })
        .collect()
}

/// The chart's cell levels (`l9i`, pre-aggregated): `columns * 7` cells, column-major from
/// Sunday.
pub fn levels(buckets: &[Bucket], today: NaiveDate, period: Period) -> Vec<u8> {
    match period {
        Period::Daily => {
            let mut by_day: HashMap<NaiveDate, f64> = HashMap::new();
            for bucket in buckets.iter().filter(|b| b.date <= today) {
                *by_day.entry(bucket.date).or_default() += bucket.tokens.max(0.);
            }
            let values: Vec<f64> = (0..columns(today) * 7)
                .map(|i| by_day.get(&cell_date(i, today)).copied().unwrap_or(0.))
                .collect();
            heat(&values)
        }
        Period::Weekly => bars(&weekly(buckets, today, false)),
        Period::Cumulative => bars(&weekly(buckets, today, true)),
    }
}

/// Month labels under the chart (`_9i`): up to 12 month starts ending at today's month.
pub fn months(today: NaiveDate) -> Vec<&'static str> {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let start = chart_start(today);
    let span = (today.year() - start.year()) * 12 + today.month() as i32 - start.month() as i32;
    let count = (span + 1).clamp(1, MAX_MONTHS as i32) as u32;
    let first = today.with_day(1).expect("day 1") - Months::new(count - 1);
    (0..count)
        .map(|i| NAMES[(first + Months::new(i)).month0() as usize])
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready(Box<Profile>),
    Failed(String),
}

pub enum Event {
    /// `Retry` after a failed load.
    Refresh,
    /// The empty top plugins `Browse` link (`/skills`).
    Browse,
}

pub struct Page {
    pub load: Load,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    pub today: NaiveDate,
    pub period: Period,
    pub avatar: Option<Arc<gpui::Image>>,
    /// Loaded `logo_url` images per plugin id.
    pub logos: HashMap<String, Arc<gpui::Image>>,
}

impl EventEmitter<Event> for Page {}

/// The content column (`_content_1eppv`): `48rem` less the toolbar padding on each side.
const WIDTH: f32 = 736.;

fn heading(label: &'static str, t: &Theme) -> gpui::Div {
    text(label, 16., 24., 500., t.text)
}

/// `pm` and `_m`: a centered tertiary note, bordered for the chart and insights.
fn unavailable(label: &'static str, bordered: bool, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .px(px(16.))
        .when(bordered, |d| {
            d.min_h(px(160.))
                .rounded(px(16.))
                .border_1()
                .border_color(t.border_subtle)
        })
        .when(!bordered, |d| d.min_h(px(60.)).py(px(12.)))
        .child(text(label, 14., 20., 400., t.text_tertiary))
}

impl Page {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            load: Load::Loading,
            generation: 0,
            today,
            period: Period::Daily,
            avatar: None,
            logos: HashMap::new(),
        }
    }

    fn pressable(
        &self,
        id: &'static str,
        label: &'static str,
        color: Hsla,
        on_press: impl Fn(&mut Self, &mut Context<Self>) + Clone + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let handle = focus::handle(id, 0, window, cx);
        let press = on_press.clone();
        focus::ring(
            div()
                .id(id)
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label)
                .child(text(label, 14., 20., 400., color)),
            &handle,
            px(4.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| on_press(this, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                press(this, cx);
                cx.stop_propagation();
            }
        }))
    }

    fn avatar(&self, profile: &Profile, t: &Theme) -> gpui::Div {
        let disc = div()
            .size(px(112.))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.primary_soft_alpha);
        match &self.avatar {
            Some(image) => disc.child(gpui::img(image.clone()).size_full()),
            None => disc.child(text(profile.initials(), 40., 48., 500., t.text_secondary)),
        }
    }

    /// `Aa` in `inline-fields` layout: avatar, name and `@username`.
    fn identity(&self, profile: &Profile, t: &Theme) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(div().mb(px(8.)).child(self.avatar(profile, t)))
            .child(
                div()
                    .min_h(px(36.))
                    .max_w_full()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(text(profile.name().to_owned(), 24., 28.8, 500., t.text)),
            )
            .when_some(profile.local_username(), |d, name| {
                d.child(
                    div()
                        .mt(px(4.))
                        .min_h(px(28.))
                        .flex()
                        .items_center()
                        .child(text(format!("@{name}"), 16., 24., 400., t.text_tertiary)),
                )
            })
            .when(!profile.bio.trim().is_empty(), |d| {
                d.child(
                    div()
                        .mt(px(24.))
                        .max_w(px(420.))
                        .px(px(10.))
                        .py(px(5.))
                        .text_center()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .text_color(t.text_secondary)
                        .child(profile.bio.clone()),
                )
            })
    }

    /// The stats strip (`T`, `ud` cells, `D` dividers).
    fn stats(&self, profile: &Profile, t: &Theme) -> AnyElement {
        let Some(summary) = &profile.summary else {
            return div()
                .rounded(px(16.))
                .border_1()
                .border_color(t.border)
                .child(unavailable("Profile stats unavailable", false, t))
                .into_any_element();
        };
        let dash = || "—".to_owned();
        let mut cells = vec![
            (
                "Lifetime tokens",
                summary.lifetime_tokens.map_or_else(dash, compact),
            ),
            (
                "Peak tokens",
                summary.peak_tokens.map_or_else(dash, compact),
            ),
        ];
        if let Some(secs) = summary.longest_task_secs {
            cells.push(("Longest task", duration(secs)));
        }
        cells.push((
            "Longest streak",
            summary.longest_streak_days.map_or_else(dash, days),
        ));
        cells.push((
            "Current streak",
            summary.current_streak_days.map_or_else(dash, days),
        ));
        // shortcut: one row as at the `@2xl` (672 px) container width the 736 px column has;
        // split into two rows if the page gets a narrower layout.
        let mut row = div().w_full().flex();
        for (i, (label, value)) in cells.into_iter().enumerate() {
            if i > 0 {
                row = row.child(div().my(px(12.)).w(px(1.)).flex_none().bg(t.border));
            }
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(1.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .px(px(12.))
                    .py(px(10.))
                    .child(text(value, 14., 20., 400., t.text))
                    .child(text(label, 14., 20., 400., t.text_secondary)),
            );
        }
        div()
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .child(row)
            .into_any_element()
    }

    /// `Token activity` (`O` section around `P`).
    fn activity(
        &self,
        profile: &Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let tabs = [
            ("profile-period-daily", "Daily", Period::Daily),
            ("profile-period-weekly", "Weekly", Period::Weekly),
            (
                "profile-period-cumulative",
                "Cumulative",
                Period::Cumulative,
            ),
        ];
        let mut controls = div().flex().items_center().gap(px(12.));
        for (id, label, period) in tabs {
            let color = if self.period == period {
                t.text
            } else {
                t.text_secondary
            };
            controls = controls.child(self.pressable(
                id,
                label,
                color,
                move |this, cx| {
                    this.period = period;
                    cx.notify();
                },
                window,
                cx,
            ));
        }
        let index = match self.period {
            Period::Daily => 0,
            Period::Weekly => 1,
            Period::Cumulative => 2,
        };
        let chart = match &profile.activity[index] {
            Some(buckets) => self.chart(buckets, &t).into_any_element(),
            None => unavailable("Token usage unavailable", true, &t).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(heading("Token activity", &t))
                    .child(controls),
            )
            .child(chart)
    }

    /// The grid (`P`): 7 rows by week columns, with the month labels below.
    fn chart(&self, buckets: &[Bucket], t: &Theme) -> impl IntoElement {
        const GAP: f32 = 3.;
        let today = self.today;
        let columns = columns(today);
        let cell = (WIDTH - GAP * (columns as f32 - 1.)) / columns as f32;
        let levels = levels(buckets, today, self.period);
        let palette = match self.period {
            Period::Daily => [
                t.chart_empty,
                t.chart_sequential_1,
                t.chart_sequential_2,
                t.chart_sequential_3,
                t.chart_sequential_4,
            ],
            _ => [
                t.border_subtle,
                t.chart_sequential_1,
                t.chart_sequential_2,
                t.chart_sequential_3,
                t.chart_sequential_3,
            ],
        };
        let grid = div().flex().gap(px(GAP)).children((0..columns).map(|c| {
            div()
                .flex_none()
                .w(px(cell))
                .flex()
                .flex_col()
                .gap(px(GAP))
                .children((0..7).map(|r| {
                    let index = c * 7 + r;
                    let hidden = self.period == Period::Daily && cell_date(index, today) > today;
                    div()
                        .size(px(cell))
                        .rounded(px(4.))
                        .when(!hidden, |d| d.bg(palette[levels[index] as usize]))
                }))
        }));
        div()
            .id("profile-chart")
            .flex()
            .flex_col()
            .gap(px(8.))
            .role(gpui::Role::Image)
            .aria_label("Codex token activity")
            .child(grid)
            .child(
                div().flex().items_center().justify_between().children(
                    months(today)
                        .into_iter()
                        .map(|m| text(m, 12., 16., 400., t.text_tertiary)),
                ),
            )
    }

    /// `Top plugins` then `Insights` (`k` in the `stacked` layout).
    fn insights(
        &self,
        profile: &Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = cx.theme().clone();
        let sections = profile.sections;
        let Some(insights) = &profile.insights else {
            if !(sections.insights || sections.top_plugins) {
                return Vec::new();
            }
            let (title, note) = if sections.insights {
                ("Insights", "Activity insights unavailable")
            } else {
                ("Top plugins", "Plugin activity unavailable")
            };
            return vec![
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(heading(title, &t))
                    .child(unavailable(note, true, &t))
                    .into_any_element(),
            ];
        };
        let mut out = Vec::new();
        if sections.top_plugins
            && let Some(plugins) = &profile.plugins
        {
            let body = if plugins.is_empty() {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .px(px(16.))
                    .py(px(24.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(t.border_subtle)
                    .child(icon("plugins", 12., t.text_tertiary))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(text(
                                "No plugins used yet ·",
                                14.,
                                20.,
                                400.,
                                t.text_tertiary,
                            ))
                            .child(self.pressable(
                                "profile-plugins-browse",
                                "Browse",
                                t.text_tertiary,
                                |_, cx| cx.emit(Event::Browse),
                                window,
                                cx,
                            )),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(16.))
                    .gap_y(px(32.))
                    .pb(px(24.))
                    .children(plugins.iter().map(|plugin| self.plugin(plugin, &t)))
                    .into_any_element()
            };
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(heading("Top plugins", &t))
                    .child(body)
                    .into_any_element(),
            );
        }
        if sections.insights {
            let not_used = || "Not used".to_owned();
            let mut rows: Vec<(&str, String, bool)> = Vec::new();
            if let Some(fast) = insights.fast_mode_percent {
                let used = fast.filter(|p| *p != 0.);
                rows.push((
                    "Fast Mode",
                    used.map_or_else(not_used, |p| format!("{} of runs", percent(p))),
                    used.is_none(),
                ));
            }
            if let Some((effort_value, share)) = &insights.reasoning {
                let used = effort_value.as_deref().zip(share.filter(|p| *p != 0.));
                rows.push((
                    "Most used reasoning",
                    used.map_or_else(not_used, |(e, p)| format!("{} · {}", effort(e), percent(p))),
                    used.is_none(),
                ));
            }
            for (label, count) in [
                ("Skills explored", insights.skills_explored),
                ("Total skills used", insights.total_skills_used),
            ] {
                if let Some(count) = count {
                    let value = if count == 0 {
                        "None".to_owned()
                    } else {
                        grouped(count)
                    };
                    rows.push((label, value, count == 0));
                }
            }
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(heading("Insights", &t))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .children(rows.into_iter().map(|(label, value, empty)| {
                                div()
                                    .h(px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(12.))
                                    .child(text(label, 14., 20., 400., t.text_secondary))
                                    .child(text(
                                        value,
                                        14.,
                                        20.,
                                        400.,
                                        if empty { t.text_tertiary } else { t.text },
                                    ))
                            })),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    /// A 56 px plugin tile with its 40 px logo, else the plugin glyph.
    fn plugin(&self, plugin: &Plugin, t: &Theme) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("profile-plugin-{}", plugin.id)))
            .size(px(56.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .role(gpui::Role::Image)
            .aria_label(SharedString::from(plugin.name.clone()))
            .child(match self.logos.get(&plugin.id) {
                Some(logo) => gpui::img(logo.clone()).size(px(40.)).into_any_element(),
                None => icon("plugins", 24., t.icon_secondary).into_any_element(),
            })
    }

    fn failed(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let handle = focus::handle("profile-retry", 0, window, cx);
        div()
            .id("profile-error")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(16.))
            .role(gpui::Role::Alert)
            .child(icon("toast-error", 16., t.text_tertiary))
            .child(text(
                "We’re having trouble loading your profile. Please try again later.",
                14.,
                20.,
                400.,
                t.text_tertiary,
            ))
            .child(
                focus::ring(
                    button("Retry", gpui::transparent_black(), None, &t)
                        .id("profile-retry")
                        .cursor_pointer()
                        .role(gpui::Role::Button)
                        .aria_label("Retry"),
                    &handle,
                    px(12.5),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Refresh)))
                .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.emit(Event::Refresh);
                        cx.stop_propagation();
                    }
                })),
            )
    }

    /// `lm`: the avatar and name placeholders while the page loads.
    fn loading(&self, t: &Theme) -> impl IntoElement {
        let block = |w: f32, h: f32, r: f32| {
            div()
                .w(px(w))
                .h(px(h))
                .rounded(px(r))
                .bg(t.primary_soft_alpha)
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(block(112., 112., 56.))
            .child(block(224., 32., 8.))
            .child(block(128., 20., 8.))
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut column = div().w(px(WIDTH)).flex_none().flex().flex_col();
        match self.load.clone() {
            Load::Loading => column = column.child(self.loading(&t)),
            Load::Failed(_) => column = column.child(self.failed(window, cx)),
            Load::Ready(profile) => {
                // Each section sits in the original's `pt-10` presence wrapper.
                let section = |child: AnyElement| div().pt(px(40.)).child(child);
                column = column.child(self.identity(&profile, &t));
                if profile.sections.usage_stats {
                    column = column.child(section(self.stats(&profile, &t)));
                }
                if profile.sections.activity_graph {
                    column = column.child(section(
                        self.activity(&profile, window, cx).into_any_element(),
                    ));
                }
                for child in self.insights(&profile, window, cx) {
                    column = column.child(section(child));
                }
            }
        }
        div()
            .id("profile-page")
            .size_full()
            .overflow_y_scroll()
            .bg(t.surface)
            .flex()
            .justify_center()
            // `_page_1eppv` padding plus `_content_1eppv`'s `3rem 2rem` block padding.
            .pt(px(20. + 16. + 48.))
            .pb(px(20. + 32.))
            .px(px(20.))
            .child(column)
    }
}

/// `/profile`: a full-page route with the sidebar closed.
pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn compact_matches_intl() {
        for (value, want) in [
            (0., "0"),
            (999., "999"),
            (1000., "1K"),
            (1250., "1.3K"),
            (12_345., "12.3K"),
            (999_960., "1M"),
            (5_400_000_000., "5.4B"),
        ] {
            assert_eq!(compact(value), want, "{value}");
        }
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn duration_drops_zero_parts() {
        assert_eq!(duration(42.), "42s");
        assert_eq!(duration(120.), "2m");
        assert_eq!(duration(125.), "2m 5s");
        assert_eq!(duration(3600.), "1h");
        assert_eq!(duration(5430.), "1h 31m");
    }

    #[test]
    fn missing_stats_and_insights_show_unavailable_states() {
        let profile = parse(&json!({
            "profile_details": {"username": "ada~x1", "display_name": " "},
            "page": {"stats": null, "workspace_display_settings": {"activity_enabled": false}},
        }))
        .unwrap();
        assert_eq!(profile.summary, None);
        assert_eq!(profile.insights, None);
        assert_eq!(profile.name(), "ada");
        assert_eq!(profile.initials(), "A");
        assert!(!profile.sections.activity_graph);
        assert!(parse(&json!({"page": {}})).is_err());
    }

    #[test]
    fn chart_spans_weeks_since_the_epoch_up_to_a_year() {
        assert_eq!(columns(date("2025-07-13")), 1);
        assert_eq!(columns(date("2025-07-26")), 2);
        assert_eq!(columns(date("2026-11-07")), 52);
        assert_eq!(chart_start(date("2025-07-30")), date("2025-07-13"));
        assert_eq!(months(date("2025-07-30")), ["Jul"]);
        assert_eq!(months(date("2026-11-07")).len(), 12);
        assert_eq!(months(date("2026-11-07")).last(), Some(&"Nov"));
    }

    #[test]
    fn daily_levels_scale_to_the_95th_percentile() {
        let today = date("2025-07-19");
        let buckets: Vec<Bucket> = [(13, 100.), (14, 50.), (15, 26.), (16, 10.), (20, 999.)]
            .iter()
            .map(|&(day, tokens)| Bucket {
                date: NaiveDate::from_ymd_opt(2025, 7, day).unwrap(),
                tokens,
            })
            .collect();
        // Nonzero days 10, 26, 50, 100 (the 20th is after today): p95 = 50 + 50 * 0.85 = 92.5.
        assert_eq!(
            levels(&buckets, today, Period::Daily),
            [4, 3, 2, 1, 0, 0, 0]
        );
    }

    #[test]
    fn weekly_and_cumulative_draw_bottom_up_bars() {
        let today = date("2025-07-26");
        let buckets = [
            Bucket {
                date: date("2025-07-13"),
                tokens: 10.,
            },
            Bucket {
                date: date("2025-07-20"),
                tokens: 40.,
            },
        ];
        let weekly = levels(&buckets, today, Period::Weekly);
        // Week 1: ceil(10 / 40 * 7) = 2 cells at the bottom; week 2: the full column.
        assert_eq!(weekly[..7], [0, 0, 0, 0, 0, 4, 4]);
        assert_eq!(weekly[7..], [4; 7]);
        // Cumulative keeps each week's latest bucket, not the sum.
        let cumulative = [
            Bucket {
                date: date("2025-07-21"),
                tokens: 50.,
            },
            Bucket {
                date: date("2025-07-20"),
                tokens: 5.,
            },
        ];
        assert_eq!(levels(&cumulative, today, Period::Cumulative)[7..], [4; 7]);
        assert_eq!(levels(&[], today, Period::Weekly), [0; 14]);
    }
}
