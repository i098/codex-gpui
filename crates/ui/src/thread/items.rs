use super::*;

/// Right-aligned user message: `bg-user-message`, 22 px squircle, at most 70% wide.
pub(super) fn user_bubble(message: &str, t: &Theme) -> Div {
    div().flex().flex_col().items_end().pb(px(30.)).child(
        div()
            .relative()
            .max_w(relative(0.7))
            .py(px(10.))
            .px(px(16.))
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(22.)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.user_message_background),
                ..Default::default()
            }))
            .child(
                div()
                    .text_size(px(14.))
                    .line_height(px(22.75))
                    .font_weight(FontWeight(430.))
                    .text_color(white())
                    .child(message.to_owned()),
            ),
    )
}

/// A row that wraps its word boxes like an inline formatting context. Chromium lets a line's
/// trailing space hang past the wrap width, and each word box carries its trailing space.
/// ponytail: a fixed 4 px allowance (one 14 px space) instead of a hanging space.
pub(super) fn wrap_row() -> Div {
    div().flex().flex_wrap().mr(px(-4.))
}

/// 14 px words of `s`, each with its trailing space, in 22.75 px line boxes.
pub(super) fn words(s: &str, weight: f32, color: Hsla, window: &Window) -> Vec<Div> {
    s.split_inclusive(' ')
        .map(|word| measured(window, word.to_owned(), 14., 22.75, weight, color, None))
        .collect()
}

/// `Worked for 17s ›` over a hairline (collapsed turn disclosure); `open`: the chevron points
/// down.
pub(super) fn disclosure(label: String, open: bool, window: &Window, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div().h(px(22.3)).pt(px(1.3)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(measured(
                        window,
                        label,
                        14.,
                        21.,
                        430.,
                        t.text_tertiary,
                        None,
                    ))
                    .child(icon(
                        if open {
                            "chevron-down"
                        } else {
                            "thread-chevron-right"
                        },
                        14.,
                        t.text_tertiary,
                    )),
            ),
        )
        .child(
            div()
                .pt(px(4.))
                .child(div().h(px(1.)).bg(over(t.border, t.surface))),
        )
}

/// `Edited math.py +4 -0` card with `Undo` and `View changes` for the file changes of turn
/// `turn_id`.
pub(super) fn diff_card(
    turn_id: &str,
    items: &[ThreadItem],
    window: &Window,
    t: &Theme,
) -> Option<Div> {
    let changes: Vec<_> = items
        .iter()
        .flat_map(|i| match i {
            ThreadItem::FileChange { changes, .. } => changes.as_slice(),
            _ => &[],
        })
        .collect();
    let first = changes.first()?;
    let (added, removed) = diff_counts(&changes);
    let mut files: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    files.dedup();
    let title = match files.len() {
        1 => format!(
            "Edited {}",
            first.path.rsplit('/').next().unwrap_or(&first.path)
        ),
        n => format!("Edited {n} files"),
    };
    let tones = tones(t);
    let card_bg = tones.card;
    // A 1 px border box (`px-2 border`): the padding stands in for the border, which the
    // decoration paints under the label.
    let button = |label: &'static str, decoration: Option<gpui::AnyElement>| {
        div()
            .relative()
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(9.))
            .children(decoration)
            .child(measured(window, label, 13., 18., 430., t.text, None))
    };
    Some(
        div()
            .relative()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(Theme::RADIUS_LG),
                corner: shapes::Corner::Squircle,
                fill: Some(card_bg),
                // `box-shadow: 0 0 0 0.5px` ring.
                shadows: vec![shadow(t.border_strong, 0., 0., 0.5, false)],
                ..Default::default()
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(12.))
                    .child(
                        div()
                            .relative()
                            .size(px(40.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(Theme::RADIUS_LG),
                                corner: shapes::Corner::Squircle,
                                // Translucent, onto the card's fill in the scroller's layer.
                                fill: Some(Hsla {
                                    a: 0.92,
                                    ..t.surface_secondary
                                }),
                                ..Default::default()
                            }))
                            .child(icon("thread-file-diff", 24., t.text_secondary)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text(title, 14., 21., 500., t.text).text_ellipsis())
                            .child(
                                div().h(px(19.5)).pt(px(3.)).child(
                                    crate::screens::diff::panel::stats(added, removed, 13., t),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(8.))
                            .child({
                                let turn_id = turn_id.to_owned();
                                button("Undo", None)
                                    .child(icon("thread-undo", 14., t.text))
                                    .id("undo-turn")
                                    .on_click(move |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(RevertTurn(turn_id.clone())),
                                            cx,
                                        )
                                    })
                            })
                            // Blink's background bleed avoidance paints the fill and the
                            // translucent border into one transparency layer (shapes' clip
                            // layer op), rasterized in the scroller's layer like the card.
                            .child(
                                button(
                                    "View changes",
                                    Some(
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .child(shapes::decoration(shapes::Decoration {
                                                radius: Corners::all(Theme::RADIUS_LG),
                                                corner: shapes::Corner::Squircle,
                                                fill: Some(tones.secondary_button),
                                                border: Some((px(1.), t.border)),
                                                ..Default::default()
                                            }))
                                            .into_any_element(),
                                    ),
                                )
                                .id("view-changes")
                                .on_click(crate::dispatch::<crate::OpenChangesPanel>),
                            ),
                    ),
            ),
    )
}

/// Added and removed line counts of `changes`' unified diffs.
fn diff_counts(changes: &[&FileUpdateChange]) -> (usize, usize) {
    let lines = |prefix: char| {
        changes
            .iter()
            .flat_map(|c| c.diff.lines())
            .filter(|l| l.starts_with(prefix) && !l.starts_with("+++") && !l.starts_with("---"))
            .count()
    };
    (lines('+'), lines('-'))
}

/// One entry of a turn's activity column (`group/agent-activity`), in order.
pub(super) enum Activity<'a> {
    Markdown(&'a str),
    /// A command still running, by its command line.
    Running(&'a str),
    /// Consecutive finished commands and file changes behind one summary
    /// (`agent-activity-group`, keyed by its first item): `Edited a file, read files, ran
    /// commands`.
    Group {
        key: &'a str,
        summary: String,
        icon: &'static str,
        rows: Vec<GroupRow>,
    },
}

/// A row of an open group: `Ran <command>`, `Read <file>`, `Edited <file> +4 -0`.
pub(super) struct GroupRow {
    icon: &'static str,
    verb: String,
    file: Option<String>,
    stats: Option<(usize, usize)>,
}

impl GroupRow {
    /// The row's text, as the original's `innerText`.
    fn label(&self) -> String {
        let mut s = self.verb.clone();
        if let Some(file) = &self.file {
            s.push(' ');
            s.push_str(file);
        }
        if let Some((added, removed)) = self.stats {
            s.push_str(&format!(" +{added} -{removed}"));
        }
        s
    }
}

impl Activity<'_> {
    /// The texts drawn for this entry: a group's summary, then its rows when `open`.
    pub(super) fn labels(&self, open: &HashSet<String>) -> Vec<String> {
        match self {
            Activity::Markdown(text) => vec![(*text).to_owned()],
            Activity::Running(command) => vec![(*command).to_owned()],
            Activity::Group {
                key, summary, rows, ..
            } => std::iter::once(summary.clone())
                .chain(
                    rows.iter()
                        .filter(|_| open.contains(*key))
                        .map(GroupRow::label),
                )
                .collect(),
        }
    }
}

/// The activity entries of a turn's `items`. A running turn names its running commands.
pub(super) fn activities<'a>(items: &'a [ThreadItem], running: bool) -> Vec<Activity<'a>> {
    let mut out: Vec<Activity<'a>> = Vec::new();
    let mut group: Vec<&'a ThreadItem> = Vec::new();
    let flush = |out: &mut Vec<Activity<'a>>, group: &mut Vec<&'a ThreadItem>| {
        out.extend(group_entry(&std::mem::take(group)));
    };
    for item in items {
        match item {
            ThreadItem::CommandExecution {
                command, status, ..
            } if running && status == "inProgress" => {
                flush(&mut out, &mut group);
                out.push(Activity::Running(command));
            }
            ThreadItem::CommandExecution { .. } | ThreadItem::FileChange { .. } => {
                group.push(item);
            }
            ThreadItem::AgentMessage { text, .. } if !text.is_empty() => {
                flush(&mut out, &mut group);
                out.push(Activity::Markdown(text));
            }
            _ => {}
        }
    }
    flush(&mut out, &mut group);
    out
}

/// The summary and rows of one group. Summary parts come in the original's order (file
/// changes, exploration, commands), the first one capitalised; the group's icon is the first
/// part's.
fn group_entry<'a>(items: &[&'a ThreadItem]) -> Option<Activity<'a>> {
    let key = item_id(items.first()?)?;
    let (mut files, mut reads, mut commands) = (0, 0, 0);
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        match item {
            ThreadItem::CommandExecution {
                command,
                command_actions,
                ..
            } => {
                // The server's parse: all `read` / `search` / `list_files` is exploration.
                let exploration = !command_actions.is_empty()
                    && command_actions
                        .iter()
                        .all(|a| matches!(a.kind.as_str(), "read" | "search" | "list_files"));
                if exploration {
                    reads += 1;
                } else {
                    commands += 1;
                }
                let names: Vec<&str> = command_actions
                    .iter()
                    .filter(|a| a.kind == "read")
                    .filter_map(|a| a.name.as_deref())
                    .collect();
                // The row names the server's parse of the command (no shell quoting), the raw
                // command line without one. ponytail: `search` and `list_files` rows also read
                // as `Ran <command>`; the original words them per action (`Searched for …`).
                let parsed = match command_actions.as_slice() {
                    [] => shell_command(command),
                    actions => actions
                        .iter()
                        .map(|a| a.command.as_str())
                        .collect::<Vec<_>>()
                        .join("; "),
                };
                rows.push(if names.is_empty() {
                    GroupRow {
                        icon: "thread-terminal",
                        verb: format!("Ran {parsed}"),
                        file: None,
                        stats: None,
                    }
                } else {
                    GroupRow {
                        icon: "thread-read",
                        verb: "Read".into(),
                        file: Some(names.join(", ")),
                        stats: None,
                    }
                });
            }
            ThreadItem::FileChange { changes, .. } => {
                files += changes.len();
                let changes: Vec<&FileUpdateChange> = changes.iter().collect();
                let Some(first) = changes.first() else {
                    continue;
                };
                let name = first.path.rsplit('/').next().unwrap_or(&first.path);
                rows.push(GroupRow {
                    icon: "pencil",
                    verb: "Edited".into(),
                    file: Some(name.to_owned()),
                    stats: Some(diff_counts(&changes)),
                });
            }
            _ => {}
        }
    }
    let count = |n: usize, one: &str, many: &str| match n {
        0 => None,
        1 => Some(one.to_owned()),
        _ => Some(many.to_owned()),
    };
    let parts: Vec<(String, &'static str)> = [
        (count(files, "Edited a file", "Edited files"), "pencil"),
        (count(reads, "Read files", "Read files"), "thread-read"),
        (
            count(commands, "Ran a command", "Ran commands"),
            "thread-terminal",
        ),
    ]
    .into_iter()
    .filter_map(|(part, icon)| Some((part?, icon)))
    .collect();
    let icon = parts.first()?.1;
    let summary = parts
        .iter()
        .enumerate()
        .map(|(i, (part, _))| match i {
            0 => part.clone(),
            _ => part[..1].to_lowercase() + &part[1..],
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(Activity::Group {
        key,
        summary,
        icon,
        rows,
    })
}

/// A turn's items without a final answer: agent messages, running command rows and activity
/// groups. A group is one 21 px summary row (`text/60`, white on hover, with a 14 px chevron
/// that shows on hover and points down while open); open, its rows follow at 4 px gaps.
/// Clicking the summary toggles `open` under the group's key.
pub(super) fn activity(
    items: &[ThreadItem],
    running: bool,
    open: &HashSet<String>,
    window: &Window,
    t: &Theme,
    cx: &Context<ThreadView>,
) -> Div {
    let text60 = t.text_alpha_60;
    // Exact-width 14 px text, styled before it is boxed (see [`measured`]).
    let label = |s: String, color: Hsla| {
        let w = width(window, &s, 14., 430., None);
        (text(s, 14., 21., 430., color), w)
    };
    let boxed = |label: gpui::AnyElement, w: Pixels| div().flex_none().w(w).child(label);
    let plain = |(label, w): (Div, Pixels)| boxed(label.into_any_element(), w);
    // A 21 px line: a 16 px icon, then its words.
    let row = |icon_name: &'static str, words: Div| {
        div()
            .h(px(21.))
            .flex()
            .items_center()
            .gap(px(6.))
            .child(icon(icon_name, 16., text60))
            .child(words)
    };
    let mut col = div().flex().flex_col().gap(px(16.));
    for entry in activities(items, running) {
        col = col.child(match entry {
            Activity::Markdown(text) => markdown(text, window, t),
            Activity::Running(command) => {
                row("thread-terminal", plain(label(command.to_owned(), text60)))
            }
            Activity::Group {
                key,
                summary,
                icon: icon_name,
                rows,
            } => {
                let is_open = open.contains(key);
                let group = SharedString::from(format!("activity-group-{key}"));
                let key = key.to_owned();
                let (summary, w) = label(summary, text60);
                // Group hover recolours text only on a stateful element: the run colour is
                // fixed at layout, before this frame's group hitbox exists.
                let summary = summary
                    .id(SharedString::from(format!("activity-summary-{key}")))
                    .group_hover(group.clone(), |s| s.text_color(t.text));
                let chevron = if is_open {
                    "chevron-down"
                } else {
                    "thread-chevron-right"
                };
                let header = div()
                    .id(group.clone())
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(row(icon_name, boxed(summary.into_any_element(), w)))
                    .child(
                        // `Icon` folds its opacity into the color, so the hover sets the color.
                        icon(chevron, 14., text60.opacity(if is_open { 1. } else { 0. }))
                            .group_hover(group, move |s| s.text_color(text60)),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if !view.expanded.remove(&key) {
                            view.expanded.insert(key.clone());
                        }
                        cx.notify();
                    }));
                let mut col = div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(4.))
                    .child(header);
                if is_open {
                    col = col.children(rows.into_iter().map(|r| {
                        let mut words = div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(plain(label(r.verb, text60)));
                        if let Some(file) = r.file {
                            // A file link: `text/30` after `Read`, `text-tertiary` after
                            // `Edited`. ponytail: solid underline, GPUI has no dotted one.
                            let color = match r.stats {
                                Some(_) => t.text_tertiary,
                                None => t.text.opacity(0.3),
                            };
                            let (link, w) = label(file, color);
                            words = words
                                .child(plain((link.underline().text_decoration_color(color), w)));
                        }
                        if let Some((added, removed)) = r.stats {
                            words = words.children(stats(added, removed, text60));
                        }
                        row(r.icon, words)
                    }));
                }
                col
            }
        });
    }
    col
}

/// `+4`, `-0` in one colour: 13 px tabular figures, tracked like the diff header's.
fn stats(added: usize, removed: usize, color: Hsla) -> [Div; 2] {
    let mut features = vec![("tnum".into(), 1), ("cv01".into(), 1), ("cv02".into(), 1)];
    features.extend(textsys::letter_spacing(-0.025 * 13.).0.iter().cloned());
    let features = gpui::FontFeatures(Arc::new(features));
    [format!("+{added}"), format!("-{removed}")]
        .map(|s| text(s, 13., 13., 430., color).font_features(features.clone()))
}
