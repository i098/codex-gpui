//! Side panel of a thread route: the split layout and titlebar tab strip that refs/diff and
//! refs/terminal share, and the Changes panel with the unified diff of the last turn's file
//! changes. All numbers are the spec's CSS px.

use crate::{
    CloseSidePanel, MONO, SANS, Shell, composer::shadow, dispatch, fixture, icon::icon,
    layout_color, over, screens::thread::view::ThreadView, text,
};
use bridge::types::{ThreadItem, ThreadReadResponse, ThreadStartResponse, Turn};
use gpui::{
    AnyView, App, AppContext as _, Context, Corners, Div, Entity, Font, FontFeatures, FontWeight,
    HighlightStyle, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Render, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun,
    Window, black, div, font, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
    rgb, white,
};
use shapes::Decoration;
use std::{ops::Range, sync::Arc};
use theme::{ActiveTheme as _, Appearance, Theme};

/// Panel width the capture had stored (`app-shell:right-panel-width:v3`) before clamping.
const STORED_WIDTH: f32 = 664.17;
/// Room the chat keeps beside the panel (`unified` layout), also the panel's minimum.
const MIN_CHAT: f32 = 320.;
/// `--diffs-font-size`, `--diffs-line-height`.
const CODE_SIZE: f32 = 12.;
const CODE_LINE: f32 = 21.6;
/// Menlo advance at 12 px (0.602em); `--diffs-min-number-column-width` is 4ch.
const CH: f32 = 7.224;

/// Panel width for the current window, as the original clamps its stored width.
/// ponytail: assumes the sidebar is open; a collapsed sidebar only leaves the chat wider.
pub fn panel_width(window: &Window) -> Pixels {
    // Main panel inner width: window minus sidebar, the main panel's 0.5 px hairline and 4 px
    // inset.
    let main = window.viewport_size().width - Theme::SIDEBAR_WIDTH - px(4.5);
    px(STORED_WIDTH).min((main - px(MIN_CHAT)).max(px(MIN_CHAT)))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Context,
    Added,
    Removed,
}

struct Line {
    kind: Kind,
    /// New-file number; old-file number for removed lines.
    number: u32,
    text: SharedString,
}

struct Hunk {
    /// `N unmodified lines` bar for the old-file lines the diff skips before this hunk.
    separator: Option<SharedString>,
    lines: Vec<Line>,
}

pub struct FileDiff {
    /// Workspace-relative directory part (tertiary) and file name of the header path.
    dir: SharedString,
    name: SharedString,
    added: usize,
    removed: usize,
    hunks: Vec<Hunk>,
    pub open: bool,
}

pub struct DiffPanel {
    pub files: Vec<FileDiff>,
}

impl DiffPanel {
    /// The subtract session as the reference shows it.
    pub fn subtract() -> Self {
        let start: ThreadStartResponse =
            fixture::response("sessions/subtract.jsonl", "thread/start", 0);
        let read: ThreadReadResponse = fixture::response("readback.jsonl", "thread/read", 1);
        Self::last_turn(&start.cwd, &read.thread.cwd, &read.thread.turns)
    }

    /// "Last Turn" source: every file change of the last of `turns`, a thread in `cwd`. Paths
    /// show relative to `workspace_root`; the server reports the thread cwd canonicalized
    /// (`/private/tmp/...`), which is why the reference shows `../../../private/tmp/...`.
    pub fn last_turn(workspace_root: &str, cwd: &str, turns: &[Turn]) -> Self {
        let mut files: Vec<FileDiff> = Vec::new();
        let changes = turns
            .last()
            .into_iter()
            .flat_map(|turn| &turn.items)
            .filter_map(|item| match item {
                ThreadItem::FileChange { changes, .. } => Some(changes),
                _ => None,
            })
            .flatten();
        for change in changes {
            let path = match change.path.strip_prefix(workspace_root) {
                Some(rel) => format!("{cwd}/{}", rel.trim_start_matches('/')),
                None => change.path.clone(),
            };
            let shown = relative(workspace_root, &path);
            let split = shown.rfind('/').map_or(0, |i| i + 1);
            let (dir, name) = shown.split_at(split);
            let hunks = parse(&change.diff);
            let count = |k| {
                hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .filter(|l| l.kind == k)
                    .count()
            };
            let (added, removed) = (count(Kind::Added), count(Kind::Removed));
            match files.iter_mut().find(|f| f.dir == dir && f.name == name) {
                Some(f) => {
                    f.added += added;
                    f.removed += removed;
                    f.hunks.extend(hunks);
                }
                None => files.push(FileDiff {
                    dir: dir.to_string().into(),
                    name: name.to_string().into(),
                    added,
                    removed,
                    hunks,
                    open: true,
                }),
            }
        }
        Self { files }
    }

    fn header(&self, t: &Theme, wide: bool) -> Div {
        let added = self.files.iter().map(|f| f.added).sum();
        let removed = self.files.iter().map(|f| f.removed).sum();
        let tool = |name| round_icon(name, 28., t);
        // A button with its own `backdrop-filter: blur(24px)`, nested in the capsule's.
        let lens = || {
            shapes::backdrop_blur(
                px(24.),
                shapes::Fill::Color,
                Decoration {
                    radius: Corners::all(px(14.)),
                    ..Default::default()
                },
            )
        };
        div()
            .h(px(48.))
            .flex_none()
            .p(px(8.))
            .flex()
            .justify_between()
            .gap(px(8.))
            .child(
                capsule(t)
                    .child(
                        div()
                            .relative()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(12.))
                            .pr(px(6.))
                            .child(lens())
                            .child(text("Last Turn", 13., 20., 430., t.text))
                            .child(icon("diff-chevron-down", 12., t.text)),
                    )
                    .child(stats(added, removed, 14., t).mr(px(4.))),
            )
            .child(
                capsule(t)
                    .child(tool("diff-more"))
                    .child(tool("diff-jump-to-file"))
                    // `@container review-header (min-width: 625px)`
                    .when(wide, |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(tool("diff-refresh"))
                                .child(tool("diff-word-wrap"))
                                .child(tool("diff-load-full-files"))
                                .child(tool("diff-word-diffs")),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(lens())
                            .child(icon("diff-show-files", 16., t.text_tertiary)),
                    ),
            )
    }
}

impl Render for DiffPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let wide = panel_width(window) - px(1.) >= px(625.);
        let files = self.files.iter().enumerate().map(|(i, file)| {
            div()
                .flex()
                .flex_col()
                .child(
                    file_header(i, file, &t).on_click(cx.listener(move |this, _, _, cx| {
                        this.files[i].open ^= true;
                        cx.notify();
                    })),
                )
                .when(file.open, |d| {
                    d.children(file.hunks.iter().map(|h| hunk(h, &t)))
                })
        });
        // No fill: the main panel's rounded fill (a `shapes` decoration the capsules' backdrop
        // blur reads) shows through, so its corners stay round. The capsules' shadows reach into
        // the body's 1 px top border (translucent) but not into the opaque scroller under it,
        // which Chromium composites in its own layer above them: clip them there.
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .overflow_hidden()
                    .child(self.header(&t, wide))
                    // Skia blends it over the shadows like Chromium (a GPUI quad rounds apart).
                    .child(
                        div()
                            .relative()
                            .h(px(1.))
                            .child(shapes::decoration(Decoration {
                                fill: Some(t.border),
                                ..Default::default()
                            })),
                    ),
            )
            .child(
                div()
                    .id("diff-files")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .pb(px(12.))
                    .children(files),
            )
    }
}

/// Titlebar row over a side panel (`Task tabs`): the panel's one selected tab (`tab_icon`,
/// `label`), `New tab`, and the `Enter full view` / `Hide tabs` buttons. The caller places it
/// from the panel's left edge to the window's right edge, 44 px tall.
pub fn tab_strip(t: &Theme, tab_icon: &str, label: SharedString, window: &Window) -> Div {
    /// Label box width and its `text-fade-truncate` distance.
    const LABEL_WIDTH: f32 = 181.;
    const FADE: f32 = 16.;
    let layout = layout_color(t);
    let fill = match t.appearance {
        Appearance::Dark => t.ghost_hover,
        Appearance::Light => t.surface_elevated_secondary,
    };
    let tab_bg = over(fill, layout);
    // The mask fades the label's last 16 px; painting the tab fill over them as a gradient
    // gives the same pixels. Only drawn when glyphs reach it: GPUI dithers gradients.
    let run = TextRun {
        len: label.len(),
        font: Font {
            weight: FontWeight(430.),
            ..font(SANS)
        },
        color: t.text,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = window
        .text_system()
        .shape_line(label.clone(), px(13.), &[run], None)
        .width;
    let fade = (width > px(LABEL_WIDTH - FADE)).then(|| {
        linear_gradient(
            90.,
            linear_color_stop(tab_bg.opacity(0.), 0.),
            linear_color_stop(tab_bg, 1.),
        )
    });
    let tab = div()
        .relative()
        .w(px(238.))
        .h(px(32.))
        .flex()
        .items_center()
        .pl(px(10.))
        .pr(px(7.))
        // `_SelectedSurface`: its own squircle layer under the tab button, with a 0.5 px
        // `border-hairline`. Chromium composites it (its rect: the ink of `0 1px 2px -1px`), so
        // the shadow lands in the layer's transparent tiles and blends onto the titlebar once.
        .child(
            div()
                .absolute()
                .left(px(-1.))
                .top_0()
                .w(px(240.))
                .h(px(34.))
                .layer()
                .child(
                    div()
                        .absolute()
                        .left(px(1.))
                        .top_0()
                        .w(px(238.))
                        .h(px(32.))
                        .child(shapes::decoration(Decoration {
                            radius: Corners::all(Theme::RADIUS_MD),
                            corner: shapes::Corner::Squircle,
                            fill: Some(fill),
                            border: Some((px(0.5), t.border)),
                            shadows: vec![shadow(t.shadow_sm, 1., 2., -1., false)],
                            ..Default::default()
                        })),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .pr(px(20.))
                .child(icon(tab_icon, 16., t.text))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .ml(px(-4.))
                        .overflow_hidden()
                        .child(text(label, 13., 18.5714, 430., t.text))
                        .when_some(fade, |d, fade| {
                            d.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .w(px(FADE))
                                    .h_full()
                                    .bg(fade),
                            )
                        }),
                ),
        )
        .child(
            div()
                .id("side-panel-close-tab")
                .absolute()
                .top(px(6.))
                .right(px(7.))
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(dispatch::<CloseSidePanel>)
                .child(icon("diff-tab-close", 14., t.text_tertiary)),
        );
    // 28 px `_Button`: 12.5 px `--codex-corner-shape` squircle, `border border-transparent`
    // (Blink paints its fill in a clip layer).
    let button = |name: &str, color: Hsla, fill: Option<Hsla>| {
        div()
            .relative()
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .when_some(fill, |d, fill| {
                d.child(shapes::decoration(Decoration {
                    radius: Corners::all(px(12.5)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(fill),
                    border: Some((px(1.), gpui::transparent_black())),
                    ..Default::default()
                }))
            })
            .child(icon(name, 16., color))
    };
    div()
        .relative()
        .h(Theme::TITLEBAR_HEIGHT)
        .flex()
        .items_center()
        .pl(px(8.))
        // `border-b border-transparent`: the content box is 43 px, so the tab sits at y 5.5.
        .pb(px(1.))
        .child(
            div()
                .absolute()
                .left_0()
                .top(px(12.))
                .w(px(1.))
                .h(px(20.))
                .bg(over(t.border, layout)),
        )
        .child(div().w(px(240.)).pr(px(2.)).child(tab))
        .child(
            div()
                .pl(px(6.))
                .child(button("diff-new-tab", t.text_tertiary, None)),
        )
        .child(
            div()
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .flex()
                .gap(px(6.))
                .child(button("diff-full-view", t.text_tertiary, None))
                // `bg-text/5` (dark: a float color, see `Theme::text_alpha_5`).
                .child(button("diff-hide-tabs", t.text, Some(t.text_alpha_5))),
        )
}

/// Main panel of a thread with a side panel open: the thread, then the panel behind its 1 px
/// `border-l`, ending at the main panel's right edge.
struct SplitMain {
    thread: AnyView,
    panel: AnyView,
}

impl Render for SplitMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .size_full()
            .flex()
            .child(div().flex_1().min_w_0().child(self.thread.clone()))
            .child(
                div()
                    .w(panel_width(window))
                    .flex_none()
                    .border_l_1()
                    .border_color(over(t.border, t.surface))
                    .child(self.panel.clone()),
            )
    }
}

/// The shell with the thread header and, with a side panel, the panel's tab strip laid over
/// its titlebar.
pub struct SplitScreen {
    shell: Entity<Shell>,
    thread: Entity<ThreadView>,
    tab: Option<(&'static str, SharedString)>,
}

impl Render for SplitScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The panel ends at the main panel's right edge, 4 px
        // (`--app-shell-page-inline-end-inset`) from the window's.
        let left = self
            .tab
            .is_some()
            .then(|| window.viewport_size().width - px(4.) - panel_width(window));
        // ponytail: like refs/thread, the header only follows the open-sidebar titlebar layout.
        let open = self.shell.read(cx).sidebar_open;
        div()
            .size_full()
            .relative()
            .font_family(SANS)
            .child(self.shell.clone())
            .when(open, |d| {
                d.child(
                    self.thread
                        .update(cx, |view, cx| view.titlebar_header(cx))
                        .when_some(left, |h, left| h.w(left - Theme::SIDEBAR_WIDTH)),
                )
            })
            // Without a panel the shell's `New tab` group follows the header behind a divider.
            .when(open && left.is_none(), |d| {
                let t = cx.theme();
                d.child(
                    div()
                        .absolute()
                        .top(px(14.))
                        .right(px(42.))
                        .w(px(1.))
                        .h(px(16.))
                        .bg(over(t.border, layout_color(t))),
                )
            })
            .when_some(self.tab.clone().zip(left), |d, ((icon, label), left)| {
                d.child(
                    tab_strip(cx.theme(), icon, label, window)
                        .absolute()
                        .top_0()
                        .left(left)
                        .right_0(),
                )
            })
    }
}

/// A thread route (refs/thread): `thread` beside the sidebar; with `panel` (refs/diff,
/// refs/terminal) the panel view on the right under a titlebar tab `(view, icon, label)`.
pub fn split_screen(
    sidebar: AnyView,
    thread: Entity<ThreadView>,
    panel: Option<(AnyView, &'static str, SharedString)>,
    cx: &mut App,
) -> AnyView {
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, thread.clone().into());
        shell.can_go_back = true;
        shell
    });
    with_side_panel(shell, thread, panel, cx).into()
}

/// `shell`'s thread route: `thread` in its main panel and, with `panel`, the panel on the
/// right under its titlebar tab `(view, icon, label)`.
pub fn with_side_panel(
    shell: Entity<Shell>,
    thread: Entity<ThreadView>,
    panel: Option<(AnyView, &'static str, SharedString)>,
    cx: &mut App,
) -> Entity<SplitScreen> {
    let main = match &panel {
        Some((panel, ..)) => cx
            .new(|_| SplitMain {
                thread: thread.clone().into(),
                panel: panel.clone(),
            })
            .into(),
        None => thread.clone().into(),
    };
    shell.update(cx, |shell, cx| {
        // A panel's tab strip brings its own `New tab`.
        shell.new_tab = panel.is_none();
        // The thread's top fade and composer band reach the main surface's right corners.
        shell.main_backgrounds = 1;
        shell.set_main(main, cx);
    });
    let tab = panel.map(|(_, icon, label)| (icon, label));
    cx.new(|cx: &mut Context<SplitScreen>| {
        cx.observe(&thread, |_, _, cx| cx.notify()).detach();
        SplitScreen { shell, thread, tab }
    })
}

/// A `rounded-full` `size` px box around a 16 px tertiary icon.
fn round_icon(name: &str, size: f32, t: &Theme) -> Div {
    div()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .child(icon(name, 16., t.text_tertiary))
}

/// `_capsule _controlSurface`: 32 px pill holding the header controls.
fn capsule(t: &Theme) -> Div {
    let shadows = match t.appearance {
        Appearance::Dark => vec![
            shadow(t.border, 0., 0., 0.5, false),
            shadow(t.border, 0., 0., 0.5, false),
            shadow(black().opacity(0.12), 8., 16., -4., false),
        ],
        Appearance::Light => vec![
            shadow(t.text.opacity(0.047), 0., 0., 1., false),
            shadow(black().opacity(0.05), 4., 16., 0., false),
        ],
    };
    // `backdrop-filter: blur(24px)` under a one-color `linear-gradient` fill.
    div()
        .relative()
        .h(px(32.))
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .child(shapes::backdrop_blur(
            px(24.),
            shapes::Fill::Gradient,
            Decoration {
                // `rounded-full`, constrained to the 32 px box's half height.
                radius: Corners::all(px(16.)),
                fill: Some(t.control_background),
                shadows,
                ..Default::default()
            },
        ))
}

/// `+N -M` in git colors, `size` px with `tracking-tight` (-0.025em letter spacing),
/// `tabular-nums` and `disambiguated-digits` (`cv01`, `cv02`).
pub fn stats(added: usize, removed: usize, size: f32, t: &Theme) -> Div {
    let mut features = vec![("tnum".into(), 1), ("cv01".into(), 1), ("cv02".into(), 1)];
    features.extend(textsys::letter_spacing(-0.025 * size).0.iter().cloned());
    let features = FontFeatures(Arc::new(features));
    let tracked =
        |s: String, color: Hsla| text(s, size, size, 430., color).font_features(features.clone());
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(tracked(format!("+{added}"), t.git_added))
        .child(tracked(format!("-{removed}"), t.git_deleted))
}

fn file_header(i: usize, file: &FileDiff, t: &Theme) -> Stateful<Div> {
    let small = |name| round_icon(name, 20., t);
    let python = match t.appearance {
        Appearance::Dark => rgb(0x69b1ff),
        Appearance::Light => rgb(0x1a85d4),
    };
    div()
        .id(("diff-file", i))
        .flex_none()
        .h(px(38.))
        .mb(px(1.))
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(12.))
        .pr(px(8.))
        .border_b_1()
        .border_color(over(t.border, t.surface))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .pl(px(4.))
                .gap(px(8.))
                .when(file.name.ends_with(".py"), |d| {
                    // `#file-tree-builtin-python` sprite.
                    d.child(icon("diff-file-python", 16., python.into()))
                })
                .child(
                    div()
                        .flex()
                        .overflow_hidden()
                        .child(text(file.dir.clone(), 14., 21., 430., t.text_tertiary))
                        .child(text(file.name.clone(), 14., 21., 430., t.text)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(stats(file.added, file.removed, 14., t).mx(px(4.)))
                .child(small("diff-open-in"))
                .child(small("diff-more")),
        )
}

/// `--color-codex-syntax-*` of the codex themes.
struct Syntax {
    keyword: Hsla,
    name: Hsla,
    string: Hsla,
    comment: Hsla,
    literal: Hsla,
}

impl Syntax {
    fn of(t: &Theme) -> Self {
        let c = |dark: u32, light: u32| -> Hsla {
            rgb(if t.appearance == Appearance::Dark {
                dark
            } else {
                light
            })
            .into()
        };
        Self {
            keyword: c(0xf8a6c8, 0xab4f7a),
            name: c(0x63a8f8, 0x1f4e94),
            string: c(0x83d197, 0x3a843f),
            comment: c(0xb9b9b9, 0x4f4f4f),
            literal: c(0xf1a275, 0xac4f23),
        }
    }
}

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

/// Python token colors for one line: keywords, `def`/`class` names, strings, comments, numbers.
/// ponytail: a line tokenizer, so strings spanning lines color only their first line.
fn highlight(line: &str, s: &Syntax) -> Vec<(Range<usize>, HighlightStyle)> {
    let b = line.as_bytes();
    let (mut i, mut prev, mut out) = (0, "", Vec::new());
    while i < b.len() {
        let start = i;
        let color = match b[i] {
            b'#' => {
                i = b.len();
                Some(s.comment)
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                Some(s.string)
            }
            c if c.is_ascii_digit() => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                    i += 1;
                }
                Some(s.literal)
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &line[start..i];
                let color = if KEYWORDS.contains(&word) {
                    Some(s.keyword)
                } else if matches!(word, "True" | "False" | "None") {
                    Some(s.literal)
                } else if matches!(prev, "def" | "class") {
                    Some(s.name)
                } else {
                    None
                };
                prev = word;
                color
            }
            _ => {
                i += 1;
                None
            }
        };
        if let Some(color) = color {
            let style = HighlightStyle {
                color: Some(color),
                ..Default::default()
            };
            out.push((start..i, style));
        }
    }
    out
}

/// `color-mix(in srgb, a (1 - w), b w)`.
fn mix(a: Hsla, b: Hsla, w: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let m = |x: f32, y: f32| x + (y - x) * w;
    Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: 1.,
    }
    .into()
}

fn hunk(h: &Hunk, t: &Theme) -> Div {
    let syntax = Syntax::of(t);
    let digits = h
        .lines
        .iter()
        .map(|l| l.number)
        .max()
        .unwrap_or(0)
        .to_string()
        .len();
    let gutter = px(CH * digits.max(4) as f32 + 16.);
    let (base, mixer) = match t.appearance {
        Appearance::Dark => (black(), 0.15),
        Appearance::Light => (white(), 0.09),
    };
    let row = |number: Option<u32>, number_bg: Hsla, bg: Hsla, content: StyledText| {
        div()
            .h(px(CODE_LINE))
            .flex()
            .child(
                div()
                    .w(gutter)
                    .flex_none()
                    .h_full()
                    .flex()
                    .justify_end()
                    .items_center()
                    .pr(px(8.))
                    .bg(number_bg)
                    .text_color(t.text_tertiary)
                    .when_some(number, |d, n| d.child(n.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .pl(px(8.))
                    .bg(bg)
                    .whitespace_nowrap()
                    .child(content),
            )
    };
    let separator = mix(t.surface, t.info_solid, 0.06);
    div()
        .flex()
        .flex_col()
        .font_family(MONO)
        .text_size(px(CODE_SIZE))
        .line_height(px(CODE_LINE))
        .text_color(t.text)
        .when_some(h.separator.clone(), |d, label| {
            let len = label.len();
            d.child(row(
                None,
                separator,
                separator,
                StyledText::new(label).with_highlights([(
                    0..len,
                    HighlightStyle {
                        color: Some(t.text_tertiary),
                        ..Default::default()
                    },
                )]),
            ))
        })
        .children(h.lines.iter().map(|l| {
            let (number_bg, bg) = match l.kind {
                Kind::Context => (t.surface, t.surface),
                Kind::Added => (mix(base, t.git_added, mixer), over(t.diff_added, t.surface)),
                Kind::Removed => (
                    mix(base, t.git_deleted, mixer),
                    over(t.diff_deleted, t.surface),
                ),
            };
            let content =
                StyledText::new(l.text.clone()).with_highlights(highlight(&l.text, &syntax));
            row(Some(l.number), number_bg, bg, content)
        }))
}

/// Hunks of a unified diff; file headers (`diff --git`, `---`, `+++`) before the first `@@`
/// are skipped.
fn parse(diff: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    // `old` starts at 1 so the lines before the first hunk count as skipped.
    let (mut old, mut new): (u32, u32) = (1, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let start = |s: &str| {
                s.split(',')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0)
            };
            let mut ranges = rest.split(' ');
            let old_start: u32 = start(ranges.next().unwrap_or(""));
            let skipped = old_start.saturating_sub(old);
            old = old_start;
            new = start(ranges.next().unwrap_or("").trim_start_matches('+'));
            let plural = if skipped == 1 { "" } else { "s" };
            hunks.push(Hunk {
                separator: (skipped > 0)
                    .then(|| format!("{skipped} unmodified line{plural}").into()),
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            continue;
        };
        let (kind, number) = match line.as_bytes().first() {
            Some(b'+') => (Kind::Added, new),
            Some(b'-') => (Kind::Removed, old),
            Some(b' ') | None => (Kind::Context, new),
            _ => continue, // `\ No newline at end of file`
        };
        if kind != Kind::Removed {
            new += 1;
        }
        if kind != Kind::Added {
            old += 1;
        }
        hunk.lines.push(Line {
            kind,
            number,
            text: line.get(1..).unwrap_or("").to_string().into(),
        });
    }
    hunks
}

/// `path` relative to directory `base`, both absolute (`path.relative` of the original).
fn relative(base: &str, path: &str) -> String {
    let base: Vec<_> = base.split('/').filter(|s| !s.is_empty()).collect();
    let path: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();
    let common = base.iter().zip(&path).take_while(|(a, b)| a == b).count();
    let up = std::iter::repeat_n("..", base.len() - common);
    up.chain(path[common..].iter().copied())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_fixture() {
        let panel = DiffPanel::subtract();
        let [file] = &panel.files[..] else {
            panic!("one file")
        };
        assert_eq!(file.dir, "../../../private/tmp/cgx-mac/proj/src/");
        assert_eq!(file.name, "math.py");
        assert_eq!((file.added, file.removed), (4, 0));
        assert_eq!(
            file.hunks[0].separator.as_deref(),
            Some("1 unmodified line")
        );
        let lines = &file.hunks[0].lines;
        assert_eq!((lines[0].kind, lines[0].number), (Kind::Context, 2));
        assert_eq!((lines[4].kind, lines[4].number), (Kind::Added, 6));
    }

    #[test]
    fn python_tokens() {
        let s = Syntax::of(&Theme::dark());
        let spans: Vec<_> = highlight("def subtract(a, b): # x", &s)
            .into_iter()
            .map(|(r, h)| (r, h.color))
            .collect();
        assert_eq!(
            spans,
            [
                (0..3, Some(s.keyword)),
                (4..12, Some(s.name)),
                (20..23, Some(s.comment))
            ]
        );
    }
}
