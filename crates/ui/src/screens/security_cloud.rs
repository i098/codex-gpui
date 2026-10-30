//! Explore ▸ `Security Cloud`: the original opens the `codex_security_cloud` MCP app
//! (`ui://defense-factory/workflows.html`). This page shows the same reads natively: the
//! Findings, Repositories and Scans tabs over the first page of each list. Display only:
//! scans, triage, filters beyond search and monitoring changes stay in the original.

use crate::composer::Composer;
use crate::screens::thread::view::Clock;
use crate::{EmptySidebar, Shell, focus, icon, text};
use bridge::security_cloud::{Finding, Repository, Scan, Snapshot, repository_label};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tab {
    #[default]
    Findings,
    Repositories,
    Scans,
}

/// Reload the snapshot; `all` drops the open-status filter (`View all findings`).
pub struct Refresh {
    pub all: bool,
}

pub struct Page {
    pub tab: Tab,
    /// `None` while loading.
    pub snapshot: Option<Snapshot>,
    /// All statuses instead of `Open findings`.
    pub all: bool,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    search: [Entity<Composer>; 3],
    clock: Clock,
}

impl EventEmitter<Refresh> for Page {}

impl Page {
    pub fn new(clock: Clock, cx: &mut Context<Self>) -> Self {
        let search =
            ["Search findings", "Search repositories", "Search scans"].map(|placeholder| {
                let field = cx.new(|cx| Composer::inline_field(placeholder, "", cx));
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            });
        Self {
            tab: Tab::Findings,
            snapshot: None,
            all: false,
            generation: 0,
            search,
            clock,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search[self.tab as usize]
            .read(cx)
            .field_text()
            .trim()
            .to_lowercase()
    }

    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut tabs = div().flex().items_center().gap(px(4.));
        for (label, tab) in [
            ("Findings", Tab::Findings),
            ("Repositories", Tab::Repositories),
            ("Scans", Tab::Scans),
        ] {
            let selected = self.tab == tab;
            let handle = focus::handle(format!("security-cloud-tab-{label}"), 0, window, cx);
            tabs = tabs.child(
                focus::ring(
                    div()
                        .id(SharedString::from(format!("security-cloud-tab-{label}")))
                        .role(gpui::Role::Tab)
                        .aria_label(label)
                        .aria_selected(selected)
                        .h(px(36.))
                        .px(px(13.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .when(selected, |d| d.bg(t.text_alpha_5))
                        .hover(|s| s.bg(t.text_alpha_5))
                        .child(text(
                            label,
                            14.,
                            20.,
                            if selected { 500. } else { 430. },
                            if selected { t.text } else { t.text_secondary },
                        )),
                    &handle,
                    px(18.),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    cx.notify();
                }))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.tab = tab;
                            cx.notify();
                            cx.stop_propagation();
                        }
                    },
                )),
            );
        }
        let credits = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.free_credits)
            .map(|remaining| {
                div()
                    .h(px(34.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_full()
                    .border_1()
                    .border_color(t.border)
                    .bg(t.background_surface_under)
                    .child(icon("diff-info", 18., t.text))
                    .child(text(
                        format!("{} free credits", usd(remaining)),
                        14.,
                        20.,
                        400.,
                        t.text,
                    ))
            });
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.))
            .child(tabs)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children(credits)
                    .child(self.refresh_button(window, cx)),
            )
    }

    fn refresh_button(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let handle = focus::handle("security-cloud-refresh", 0, window, cx);
        let all = self.all;
        focus::ring(
            div()
                .id("security-cloud-refresh")
                .role(gpui::Role::Button)
                .aria_label("Refresh")
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .border_1()
                .border_color(t.border)
                .hover(|s| s.bg(t.text_alpha_5))
                .child(icon("diff-refresh", 16., t.text)),
            &handle,
            px(10.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(Refresh { all })))
        .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.emit(Refresh { all });
                cx.stop_propagation();
            }
        }))
    }

    fn search(&self, t: &Theme) -> Div {
        div()
            .h(px(34.))
            .px(px(11.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(t.border_strong)
            .child(icon("search", 16., t.text_secondary))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.search[self.tab as usize].clone()),
            )
    }

    fn columns(names: &[(&'static str, f32)], t: &Theme) -> Div {
        let mut row = div()
            .h(px(40.))
            .px(px(24.))
            .flex()
            .items_center()
            .border_t_1()
            .border_b_1()
            .border_color(t.border);
        for &(name, width) in names {
            let cell = text(name, 12., 16., 430., t.text_secondary);
            row = row.child(if width > 0. {
                div().w(px(width)).child(cell)
            } else {
                div().flex_1().min_w_0().child(cell)
            });
        }
        row
    }

    fn state(message: impl Into<SharedString>, t: &Theme) -> Div {
        div()
            .h(px(160.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .child(text(message, 14., 20., 430., t.text_secondary))
    }

    fn findings(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let query = self.query(cx);
        let snapshot = self.snapshot.as_ref();
        let chart = match snapshot.map(|snapshot| &snapshot.summary) {
            None => "Loading chart…".to_owned(),
            Some(Err(_)) => "Chart could not be loaded.".to_owned(),
            Some(Ok(0)) => "No findings match your current filters.".to_owned(),
            Some(Ok(total)) => plural(*total, "finding"),
        };
        let mut page = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(168.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text(chart, 14., 20., 430., t.text_secondary)),
            )
            .child(Self::columns(
                &[
                    ("Severity  ↓", 95.),
                    ("Finding", 0.),
                    ("Status", 90.),
                    ("Discovered", 70.),
                ],
                &t,
            ));
        let (rows, total) = match snapshot.map(|snapshot| &snapshot.findings) {
            None => {
                return page.child(Self::state("Loading findings…", &t));
            }
            Some(Err(_)) => {
                return page.child(Self::state("Findings could not be loaded.", &t));
            }
            Some(Ok(findings)) => (
                findings
                    .items
                    .iter()
                    .filter(|finding| matches(&query, &[&finding.title, &finding.repo_url]))
                    .collect::<Vec<_>>(),
                findings.total,
            ),
        };
        if rows.is_empty() {
            let empty = if self.all {
                "No findings."
            } else {
                "No open findings."
            };
            let mut state = Self::state(empty, &t);
            if !self.all {
                state = state.child(self.view_all(window, cx));
            }
            page = page.child(state);
        } else {
            page = page.children(rows.iter().map(|finding| self.finding_row(finding, &t)));
        }
        let shown = rows.len() as u64;
        page.child(
            div()
                .h(px(58.))
                .px(px(24.))
                .flex()
                .items_center()
                .border_t_1()
                .border_b_1()
                .border_color(t.border)
                .child(text(
                    format!("{}–{shown} of {total} findings", u64::from(shown > 0)),
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                )),
        )
    }

    fn view_all(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let handle = focus::handle("security-cloud-view-all", 0, window, cx);
        focus::ring(
            div()
                .id("security-cloud-view-all")
                .role(gpui::Role::Button)
                .aria_label("View all findings")
                .h(px(34.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .bg(t.text_alpha_5)
                .hover(|s| s.bg(t.text_alpha_10))
                .child(text("View all findings", 14., 20., 430., t.text)),
            &handle,
            px(8.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(|_, _, _, cx| cx.emit(Refresh { all: true })))
        .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.emit(Refresh { all: true });
                cx.stop_propagation();
            }
        }))
    }

    fn finding_row(&self, finding: &Finding, t: &Theme) -> Div {
        let severity = finding
            .criticality
            .as_deref()
            .map_or("—", criticality_label);
        let severity_color = match finding.criticality.as_deref() {
            Some("critical") => t.text_error,
            Some("high") => t.text_warning,
            _ => t.text_secondary,
        };
        div()
            .min_h(px(56.))
            .px(px(24.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .w(px(95.))
                    .child(text(severity, 12., 16., 500., severity_color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(finding.title.clone(), 14., 20., 500., t.text).truncate())
                    .child(text(
                        repository_label(&finding.repo_url).to_owned(),
                        12.,
                        16.,
                        430.,
                        t.text_secondary,
                    )),
            )
            .child(div().w(px(90.)).child(text(
                status_label(&finding.status),
                12.,
                16.,
                500.,
                t.text,
            )))
            .child(
                div().w(px(70.)).child(text(
                    finding
                        .created_at
                        .as_deref()
                        .map_or_else(|| "—".to_owned(), |date| relative(date, self.clock)),
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                )),
            )
    }

    fn repositories(&self, cx: &App) -> Div {
        let t = cx.theme();
        let query = self.query(cx);
        let page = div().flex().flex_col().child(Self::columns(
            &[("Repository", 0.), ("Monitoring", 176.), ("Owner", 212.)],
            t,
        ));
        let rows: Vec<&Repository> = match self.snapshot.as_ref().map(|s| &s.repositories) {
            None => return page.child(Self::state("Loading repositories…", t)),
            Some(Err(_)) => return page.child(Self::state("Repositories couldn’t be loaded.", t)),
            Some(Ok(list)) => list
                .iter()
                .filter(|repo| matches(&query, &[&repo.name]))
                .collect(),
        };
        if rows.is_empty() {
            return page.child(Self::state("No repositories match this search", t));
        }
        page.children(rows.into_iter().map(|repo| {
            div()
                .h(px(48.))
                .px(px(24.))
                .flex()
                .items_center()
                .border_b_1()
                .border_color(t.border)
                .child(div().flex_1().min_w_0().child(text(
                    repo.name.clone(),
                    14.,
                    20.,
                    500.,
                    t.text,
                )))
                .child(
                    div()
                        .w(px(176.))
                        .child(text("—", 14., 20., 430., t.text_secondary)),
                )
                .child(div().w(px(212.)).child(text(
                    repo.owner.clone(),
                    14.,
                    20.,
                    430.,
                    t.text_secondary,
                )))
        }))
    }

    fn scans(&self, cx: &App) -> Div {
        let t = cx.theme();
        let query = self.query(cx);
        let page = div().flex().flex_col().child(Self::columns(
            &[("Repository", 0.), ("Status", 160.), ("Started", 120.)],
            t,
        ));
        let rows: Vec<&Scan> = match self.snapshot.as_ref().map(|s| &s.scans) {
            None => return page.child(Self::state("Loading scans…", t)),
            Some(Err(_)) => return page.child(Self::state("Scan history could not be loaded.", t)),
            Some(Ok(list)) => list
                .iter()
                .filter(|scan| matches(&query, &[scan_repository(scan), &scan.status]))
                .collect(),
        };
        if rows.is_empty() {
            return page.child(Self::state("No scans to show.", t));
        }
        page.children(rows.into_iter().map(|scan| {
            div()
                .h(px(48.))
                .px(px(24.))
                .flex()
                .items_center()
                .border_b_1()
                .border_color(t.border)
                .child(div().flex_1().min_w_0().child(text(
                    scan_repository(scan).to_owned(),
                    14.,
                    20.,
                    500.,
                    t.text,
                )))
                .child(div().w(px(160.)).child(text(
                    scan_status_label(&scan.status),
                    14.,
                    20.,
                    430.,
                    t.text,
                )))
                .child(
                    div().w(px(120.)).child(text(
                        scan.created_at
                            .as_deref()
                            .map_or_else(|| "—".to_owned(), |date| relative(date, self.clock)),
                        14.,
                        20.,
                        430.,
                        t.text_secondary,
                    )),
                )
        }))
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body = match self.tab {
            Tab::Findings => self.findings(window, cx),
            Tab::Repositories => self.repositories(cx),
            Tab::Scans => self.scans(cx),
        };
        div()
            .id("security-cloud-page")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.surface)
            .child(
                div()
                    .px(px(24.))
                    .pt(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .pb(px(16.))
                    .child(self.header(window, cx))
                    .child(self.search(&t)),
            )
            .child(
                div()
                    .id("security-cloud-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

fn matches(query: &str, fields: &[&str]) -> bool {
    query.is_empty()
        || fields
            .iter()
            .any(|field| field.to_lowercase().contains(query))
}

fn scan_repository(scan: &Scan) -> &str {
    scan.repo_url
        .as_deref()
        .map_or(scan.repo_id.as_str(), repository_label)
}

/// `{count, plural, one {# finding} other {# findings}}`.
fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// USD with no forced decimals, as `formatNumber(…, {style: "currency", maximumFractionDigits: 2})`.
fn usd(amount: f64) -> String {
    let cents = (amount * 100.).round() as u64;
    if cents % 100 == 0 {
        format!("${}", cents / 100)
    } else {
        format!("${}.{:02}", cents / 100, cents % 100)
            .trim_end_matches('0')
            .to_owned()
    }
}

/// `CRITICALITY_MESSAGES`.
fn criticality_label(level: &str) -> &str {
    match level {
        "critical" => "Critical",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        "informational" => "Informational",
        _ => "—",
    }
}

/// `STATUS_MESSAGES`.
fn status_label(status: &str) -> &str {
    match status {
        "new" => "New",
        "triaged" => "Triaged",
        "in_progress" => "In Progress",
        "fixed" => "Fixed",
        "wontfix" => "Won't fix",
        "duplicate" => "Duplicate",
        "false_positive" => "False positive",
        other => other,
    }
}

/// The scans table's status labels (`scans-messages.ts`).
fn scan_status_label(status: &str) -> &str {
    match status {
        "queued" => "Queued",
        "running" => "Running",
        "preparing" => "Preparing",
        "validating_output" => "Validating output",
        "cancel_requested" => "Cancel requested",
        "completed" | "finished" => "Completed",
        "failed" => "Failed",
        "canceled" | "cancelled" => "Canceled",
        other => other,
    }
}

/// `FormattedRelativeTime` with `numeric: "auto"` in the largest whole unit.
fn relative(date: &str, clock: Clock) -> String {
    let Ok(date) = chrono::DateTime::parse_from_rfc3339(date) else {
        return "—".into();
    };
    let seconds = (clock.now() - date.timestamp()).max(0);
    let (value, unit) = [
        (31_536_000, "year"),
        (2_592_000, "month"),
        (604_800, "week"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
    ]
    .into_iter()
    .find(|(size, _)| seconds >= *size)
    .map_or((0, "now"), |(size, unit)| (seconds / size, unit));
    match (value, unit) {
        (_, "now") => "now".into(),
        (1, "day") => "yesterday".into(),
        (1, "hour" | "minute") => format!("1 {unit} ago"),
        (1, unit) => format!("last {unit}"),
        (value, unit) => format!("{value} {unit}s ago"),
    }
}

/// Explore ▸ `Security Cloud`: sidebar closed, no rail item current.
pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times_use_the_largest_whole_unit() {
        // 2026-10-16T20:00:00Z.
        let clock = Clock {
            utc_offset: 0,
            pinned_now: Some(1_792_180_800),
            elapsed_since: None,
        };
        assert_eq!(relative("2026-10-16T19:59:30Z", clock), "now");
        assert_eq!(relative("2026-10-16T17:00:00Z", clock), "3 hours ago");
        assert_eq!(relative("2026-10-15T19:00:00Z", clock), "yesterday");
        assert_eq!(relative("2026-10-07T20:00:00Z", clock), "last week");
        assert_eq!(relative("x", clock), "—");
    }

    #[test]
    fn usd_drops_zero_cents() {
        assert_eq!(usd(500.), "$500");
        assert_eq!(usd(12.5), "$12.5");
        assert_eq!(usd(0.25), "$0.25");
    }
}
