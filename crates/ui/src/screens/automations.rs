//! `automations` parity screen: rail `Scheduled` (`/automations`). Sidebar `ScheduledSidebar`
//! and the page's empty state with suggested tasks, from `refs/automations/*.spec.json`.
//! Host data (`list-automations`, `/suggested_automations`) comes from the recorded answers
//! in `tools/route1/oracle.json`.

use crate::{Shell, icon, icon_button, text};
use gpui::{
    Action, AnyView, App, AppContext as _, Bounds, Context, Hsla, InteractiveElement, IntoElement,
    ParentElement, PathBuilder, Pixels, Render, SharedString, StatefulInteractiveElement, Styled,
    Window, actions, canvas, div, img, point, px,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2},
    fs,
    path::Path,
};
use theme::{ActiveTheme as _, Theme};

actions!(
    automations,
    [
        /// Sidebar `New task`: opens the create flow.
        NewTask,
        /// Sidebar `Search scheduled tasks`.
        SearchTasks,
        /// Sidebar `Filter tasks` (ownership `Personal` / `Team`).
        FilterTasks,
    ]
);

/// A suggested-task card was clicked: start a task from its prompt.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = automations, no_json)]
pub struct StartSuggestedTask {
    pub prompt: SharedString,
}

/// One `/suggested_automations` entry (or a built-in landing example).
#[derive(Clone, Deserialize)]
struct Suggestion {
    title: SharedString,
    description: SharedString,
    user_message: SharedString,
    emoji: SharedString,
    icon_url: Option<String>,
}

/// The recorded answer of the first `tools/route1/oracle.json` entry whose key contains
/// `key`: a host fetch body (`bodyJsonString`) or a ChatGPT backend body (`__response.body`).
fn oracle(key: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/route1/oracle.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let all: serde_json::Map<String, Value> = serde_json::from_str(&text).expect("oracle JSON");
    let entry = all
        .iter()
        .find(|(k, _)| k.contains(key))
        .unwrap_or_else(|| panic!("oracle.json: no entry for {key}"))
        .1;
    let body = entry["bodyJsonString"]
        .as_str()
        .or_else(|| entry["value"]["response"]["__response"]["body"].as_str())
        .unwrap_or_else(|| panic!("oracle.json: {key} has no body"));
    serde_json::from_str(body).expect("oracle body JSON")
}

/// `scheduled.landing.example.*` in bundle order: the page fills up to 6 cards with these
/// after the server's suggestions. ponytail: the original shuffles them per mount
/// (`Math.random`); the port keeps bundle order.
fn landing_examples() -> Vec<Suggestion> {
    [
        (
            "🥗",
            "Weekly meal plan",
            "Plan meals for the week and make a grocery list.",
            "Help me set up a weekly meal plan with a grocery list. Ask about my dietary preferences, household size, and when I’d like to receive it before scheduling.",
        ),
        (
            "💰",
            "Weekly finances update",
            "Review my recent spending, and anything that needs attention.",
            "Help me set up a weekly review of my recent spending, recurring charges, and anything that needs attention. Ask which financial information I want to use and what day and time works for me before scheduling.",
        ),
        (
            "😴",
            "Sleep coaching",
            "Track my sleep quality and help me improve my habits.",
            "Help me set up a regular sleep check-in. Ask about my sleep goals, the information I’m comfortable sharing, and when I’d like to check in before scheduling.",
        ),
        (
            "🛩",
            "Find flights for a trip",
            "Find the best deal on flights for an upcoming trip.",
            "Help me set up a flight price watch for an upcoming trip. Ask where I’m flying from and to, my dates, budget, and how often to check. Notify me when there’s a good match.",
        ),
        (
            "✉️",
            "Email monitor",
            "Scan my emails and let me know anything that needs my attention.",
            "Help me set up an email monitor that flags messages needing my attention. Ask which inbox or senders to watch, what counts as important, and how often to check before scheduling. Help me connect email if needed.",
        ),
    ]
    .into_iter()
    .map(|(emoji, title, description, prompt)| Suggestion {
        title: title.into(),
        description: description.into(),
        user_message: prompt.into(),
        emoji: emoji.into(),
        icon_url: None,
    })
    .collect()
}

/// Server suggestions, then landing examples whose title is not already shown, at most 6.
fn cards(mut all: Vec<Suggestion>) -> Vec<Suggestion> {
    let key = |s: &Suggestion| s.title.trim().to_lowercase();
    let taken: Vec<String> = all.iter().map(key).collect();
    all.extend(
        landing_examples()
            .into_iter()
            .filter(|s| !taken.contains(&key(s))),
    );
    all.truncate(6);
    all
}

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let server = serde_json::from_value(oracle("\"url\":\"/suggested_automations\""))
        .expect("suggested_automations shape");
    let sidebar = cx.new(|_| ScheduledSidebar).into();
    let main = cx
        .new(|_| AutomationsPage {
            cards: cards(server),
        })
        .into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, main);
        shell.selected_rail = Some("scheduled");
        shell.new_tab = false;
        shell.can_go_back = true;
        shell
    })
    .into()
}

/// `border: <width> dashed` on a rounded box, laid out as Chromium paints it
/// (`BoxBorderPainter::DrawCurvedDashedDottedBoxSide`): one closed dash pattern along the
/// border's centerline, starting at the end of the top-left corner and running clockwise;
/// dash and base gap are 3x / 2x the device border width (2x / 1x from 3 device px), and the
/// gap is stretched so whole periods fit the truncated device path length
/// (`SelectBestDashGap`). The dashes are one stroked svg path rasterized by Chromium's Skia
/// (`shapes::paint_icon`), a GPUI path where that is not available. GPUI's own
/// `border_dashed` uses a fixed 2:1 pattern.
fn dashed_border(width: Pixels, radius: Pixels, color: Hsla) -> impl IntoElement {
    let dashes = canvas(
        |_, _, _| {},
        move |b: Bounds<Pixels>, _, window, cx| {
            let (bw, r) = (f32::from(width), f32::from(radius));
            let scale = window.scale_factor();
            let rc = r - bw / 2.;
            let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
            // Centerline box, local CSS px.
            let (l, t, rt, bt) = (bw / 2., bw / 2., w - bw / 2., h - bw / 2.);
            // The corners' length as Skia measures the conics (chords), not the true arc.
            let chords = quarter_chords(rc * scale);
            let arc = chords.last().map_or(0., |c| c.1) / scale;
            let (sw, sh) = (rt - l - 2. * rc, bt - t - 2. * rc);
            let thick = bw * scale;
            let (dash_ratio, gap_ratio) = if thick >= 3. { (2., 1.) } else { (3., 2.) };
            let (dash, gap) = (thick * dash_ratio, thick * gap_ratio);
            let len = ((2. * (sw + sh) + 4. * arc) * scale).trunc();
            let n = (len / (dash + gap)).floor();
            let (gap_lo, gap_hi) = ((len - n * dash) / n, (len - (n + 1.) * dash) / (n + 1.));
            let gap = if gap_hi <= 0. || (gap_lo - gap).abs() < (gap_hi - gap).abs() {
                gap_lo
            } else {
                gap_hi
            };
            let (dash, period) = (dash / scale, (dash + gap) / scale);
            // Clockwise segments from the top edge: (start offset, length, kind).
            let segs = [
                (0., sw, 0),
                (sw, arc, 1),
                (sw + arc, sh, 2),
                (sw + arc + sh, arc, 3),
                (sw + 2. * arc + sh, sw, 4),
                (2. * sw + 2. * arc + sh, arc, 5),
                (2. * sw + 3. * arc + sh, sh, 6),
                (2. * sw + 3. * arc + 2. * sh, arc, 7),
            ];
            // Each dash piece on the centerline: (is a quarter-circle arc, start, end).
            let mut pieces = Vec::new();
            let mut start = 0.;
            while start < len / scale {
                let end = start + dash;
                for &(off, seg_len, kind) in &segs {
                    let (s0, s1) = ((start - off).max(0.), (end - off).min(seg_len));
                    if s0 >= s1 {
                        continue;
                    }
                    pieces.push(match kind {
                        0 => (false, (l + rc + s0, t), (l + rc + s1, t)),
                        2 => (false, (rt, t + rc + s0), (rt, t + rc + s1)),
                        4 => (false, (rt - rc - s0, bt), (rt - rc - s1, bt)),
                        6 => (false, (l, bt - rc - s0), (l, bt - rc - s1)),
                        _ => {
                            let (cx, cy, a0) = match kind {
                                1 => (rt - rc, t + rc, -FRAC_PI_2),
                                3 => (rt - rc, bt - rc, 0.),
                                5 => (l + rc, bt - rc, FRAC_PI_2),
                                _ => (l + rc, t + rc, 2. * FRAC_PI_2),
                            };
                            let at = |s: f32| {
                                let a = a0 + quarter_angle(&chords, s * scale);
                                (cx + rc * a.cos(), cy + rc * a.sin())
                            };
                            (true, at(s0), at(s1))
                        }
                    });
                }
                start += period;
            }
            let d: String = pieces
                .iter()
                .map(|&(arc, (x0, y0), (x1, y1))| match arc {
                    true => format!("M{x0} {y0}A{rc} {rc} 0 0 1 {x1} {y1}"),
                    false => format!("M{x0} {y0}L{x1} {y1}"),
                })
                .collect();
            let svg = format!(
                r#"<svg viewBox="0 0 {w} {h}" fill="none"><path d="{d}" stroke="black" stroke-width="{bw}"/></svg>"#
            );
            let key = format!("dashed-border-{w}x{h}-{bw}-{r}");
            if shapes::paint_icon(svg.as_bytes(), &key, b, false, color, None, window, cx) {
                return;
            }
            let at = |(x, y): (f32, f32)| b.origin + point(px(x), px(y));
            let mut path = PathBuilder::stroke(width);
            for &(arc, p0, p1) in &pieces {
                path.move_to(at(p0));
                match arc {
                    true => path.arc_to(point(px(rc), px(rc)), px(0.), false, true, at(p1)),
                    false => path.line_to(at(p1)),
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size_full();
    // Blink paints the translucent sides as an opacity group: a layer composited onto the card.
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .layer()
        .child(dashes)
}

/// A rounded corner's conic (`SkPath::addRRect`, radius `r` device px) as `SkContourMeasure`
/// splits it: halved in t while the curve's midpoint is more than 0.5 px (larger axis) from
/// the chord's. Returns `(t, length so far)` at each chord end.
fn quarter_chords(r: f32) -> Vec<(f32, f32)> {
    fn split(r: f32, t0: f32, t1: f32, out: &mut Vec<(f32, f32)>) {
        let (p0, pm, p1) = (conic(r, t0), conic(r, (t0 + t1) / 2.), conic(r, t1));
        let (dx, dy) = (pm.0 - (p0.0 + p1.0) / 2., pm.1 - (p0.1 + p1.1) / 2.);
        if dx.abs().max(dy.abs()) > 0.5 {
            split(r, t0, (t0 + t1) / 2., out);
            split(r, (t0 + t1) / 2., t1, out);
        } else {
            let len = out.last().map_or(0., |c| c.1) + (p1.0 - p0.0).hypot(p1.1 - p0.1);
            out.push((t1, len));
        }
    }
    let mut out = vec![(0., 0.)];
    split(r, 0., 1., &mut out);
    out
}

/// Point `t` of the quarter circle conic (r, 0) → (0, r), weight √½.
fn conic(r: f32, t: f32) -> (f32, f32) {
    let (u, k) = (1. - t, 2. * FRAC_1_SQRT_2 * t * (1. - t));
    let d = u * u + k + t * t;
    ((u * u + k) * r / d, (k + t * t) * r / d)
}

/// The angle into the corner at measured length `s` (device px): Skia's dash maps length to t
/// linearly within each chord.
fn quarter_angle(chords: &[(f32, f32)], s: f32) -> f32 {
    let i = chords
        .partition_point(|c| c.1 < s)
        .clamp(1, chords.len() - 1);
    let ((t0, c0), (t1, c1)) = (chords[i - 1], chords[i]);
    let (x, y) = conic(1., t0 + (t1 - t0) * ((s - c0) / (c1 - c0)).clamp(0., 1.));
    y.atan2(x)
}

/// `ScheduledSidebar`: header `Scheduled` + search, `New task`, section `Upcoming`.
pub struct ScheduledSidebar;

impl Render for ScheduledSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let hover_bg = t.list_hover;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .child(
                        div()
                            .h(px(52.))
                            .ml(px(8.))
                            .pr(px(4.))
                            .flex()
                            .items_center()
                            .child(div().flex_1().font_family("OpenAI Sans").child(text(
                                "Scheduled",
                                18.,
                                24.,
                                600.,
                                t.text_default_nav,
                            )))
                            .child(
                                icon_button(
                                    "search-tasks",
                                    icon("search", 16., t.text_tertiary),
                                    t,
                                )
                                .on_click(crate::dispatch::<SearchTasks>),
                            ),
                    )
                    .child(
                        div()
                            .id("new-task")
                            .mb(px(8.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .rounded(px(12.5))
                            .hover(move |s| s.bg(hover_bg))
                            .on_click(crate::dispatch::<NewTask>)
                            .child(icon("automations-plus", 16., t.text_default_nav))
                            .child(text("New task", 14., 21., 430., t.text_default_nav)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .px(px(8.))
                    .pb(px(8.))
                    .child(
                        div()
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(8.))
                            .pr(px(6.))
                            .child(text("Upcoming", 14., 21., 500., t.text_tertiary))
                            .child(
                                icon_button(
                                    "filter-tasks",
                                    icon("automations-filter", 16., t.text_tertiary),
                                    t,
                                )
                                .on_click(crate::dispatch::<FilterTasks>),
                            ),
                    )
                    .child(div().mt(px(1.)).px(px(8.)).py(px(4.)).child(text(
                        "No scheduled tasks",
                        13.,
                        18.571_428,
                        430.,
                        t.text_tertiary,
                    ))),
            )
    }
}

/// Main column of the empty page: hero and the suggested-task grid.
pub struct AutomationsPage {
    cards: Vec<Suggestion>,
}

impl AutomationsPage {
    fn card(&self, i: usize, s: &Suggestion, cx: &App) -> impl IntoElement {
        let t: &Theme = cx.theme();
        let hover_bg = t.ghost_hover;
        // `icon_url` is shown from its bundled copy under assets/ui/automations, its paths
        // rasterized by Chromium's Skia like Blink draws an svg `<img>`. Elsewhere GPUI's own
        // image raster (the copies declare 40x40, the 20 px slot at 2x: GPUI rasterizes svg
        // images at their intrinsic size).
        let glyph = match &s.icon_url {
            Some(url) => {
                let path = format!("ui/automations/{}", url.rsplit('/').next().unwrap_or(url));
                if cfg!(target_os = "macos") {
                    canvas(
                        |_, _, _| {},
                        move |b, (), window, cx| {
                            if let Ok(Some(src)) = cx.asset_source().load(&path) {
                                let white = gpui::white();
                                shapes::paint_icon(&src, &path, b, false, white, None, window, cx);
                            }
                        },
                    )
                    .size(px(20.))
                    .into_any_element()
                } else {
                    img(path).size(px(20.)).into_any_element()
                }
            }
            None => text(s.emoji.clone(), 28., 28., 430., t.text).into_any_element(),
        };
        let prompt = s.user_message.clone();
        div()
            .id(("suggestion", i))
            .flex_1()
            .min_w_0()
            .min_h(px(108.))
            .flex()
            .items_center()
            .gap(px(16.))
            .relative()
            // 20 px padding inside a 1 px dashed border.
            .p(px(21.))
            .rounded(px(24.))
            .child(dashed_border(px(1.), px(24.), t.border))
            .hover(move |st| st.bg(hover_bg))
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(StartSuggestedTask {
                        prompt: prompt.clone(),
                    }),
                    cx,
                )
            })
            .child(
                div()
                    .size(px(24.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        text(s.title.clone(), 16., 26., 400., t.text)
                            .font_features(textsys::letter_spacing(-0.32))
                            .truncate(),
                    )
                    .child(
                        text(s.description.clone(), 14., 20., 400., t.text_tertiary)
                            .whitespace_normal()
                            .line_clamp(2)
                            .text_ellipsis(),
                    ),
            )
    }
}

impl Render for AutomationsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let rows = self.cards.chunks(2).enumerate().map(|(r, pair)| {
            div().flex().gap(px(20.)).children(
                pair.iter()
                    .enumerate()
                    .map(|(c, s)| self.card(r * 2 + c, s, cx)),
            )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            // Page header row (`--height-toolbar`), empty in this state.
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("automations-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .min_h_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .pt(px(24.))
                            .px(px(12.))
                            .pb(px(44.))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(738.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(44.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap(px(12.))
                                            .child(
                                                text("🕗", 48., 48., 400., t.text)
                                                    .font_features(textsys::letter_spacing(-1.2)),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .items_center()
                                                    .gap(px(2.))
                                                    .child(
                                                        text("Schedule a task", 18., 26., 400., t.text)
                                                            .font_features(textsys::letter_spacing(-0.45)),
                                                    )
                                                    .child(
                                                        text(
                                                            "ChatGPT can take care of ongoing tasks so you don’t have to.",
                                                            14.,
                                                            20.,
                                                            400.,
                                                            t.text_tertiary,
                                                        )
                                                        .font_features(textsys::letter_spacing(-0.14)),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .gap(px(20.))
                                            .children(rows),
                                    ),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cards_fill_six_from_oracle() {
        let server = serde_json::from_value(super::oracle("\"url\":\"/suggested_automations\""))
            .expect("shape");
        let titles: Vec<_> = super::cards(server).into_iter().map(|s| s.title).collect();
        assert_eq!(titles.len(), 6);
        assert_eq!(titles[0], "AP Lang practice");
        assert_eq!(titles[3], "Weekly meal plan");
    }
}
