use super::*;
use base64::Engine as _;
use gpui::{Image, ImageFormat};

#[path = "items/automation.rs"]
mod automation;
use automation::automation_item;
pub(super) use automation::cache_automation_result;

pub(super) type McpImages = RefCell<HashMap<String, Vec<(usize, Arc<Image>)>>>;

/// Right-aligned user message: `bg-user-message`, 22 px squircle, at most 70% wide. A message
/// that wraps to more than 20 lines collapses to 19 with `Show more`
/// (`collapsible-user-message-content`, `collapsedLineCount` 20, ellipsis indicator).
pub(super) fn user_bubble(
    id: &SharedString,
    message: &str,
    find: Option<&FindPaint>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    const COLLAPSED_LINES: usize = 20;
    const LINE: f32 = 22.75;
    const CONTROLS_WIDTH: f32 = 187.73;
    let t = cx.theme().clone();
    let width = markdown::line_width() * 0.7 - 32.;
    let lines = if width > 0. {
        let run = TextRun {
            len: message.len(),
            font: gpui::Font {
                weight: FontWeight(430.),
                ..font(SANS)
            },
            color: white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_text(
                message.to_owned().into(),
                px(14.),
                &[run],
                Some(px(width)),
                None,
            )
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| line.wrap_boundaries.len() + 1)
                    .sum::<usize>()
            })
            .unwrap_or(0)
    } else {
        0
    };
    // The hover controls row sits 4 px below the bubble. Its time label wraps to two lines
    // (row 32 px instead of 26) once the column is narrower than the row's widest layout,
    // 187.73 px in the original's wide state.
    let line = markdown::line_width();
    let reserve = if line > 0. && line < CONTROLS_WIDTH {
        36.
    } else {
        30.
    };
    let collapsible = lines > COLLAPSED_LINES;
    let expanded =
        window.use_keyed_state(SharedString::from(format!("{id}-expanded")), cx, |_, _| {
            false
        });
    let collapsed = collapsible && !*expanded.read(cx);
    let body = div()
        .text_size(px(14.))
        .line_height(px(LINE))
        .font_weight(FontWeight(430.))
        .text_color(white())
        .when(collapsed, |d| {
            // `collapsible-user-message-content` sets `max-height: ceil(line-height * 19)`.
            d.max_h(px((LINE * (COLLAPSED_LINES - 1) as f32).ceil()))
                .line_clamp(COLLAPSED_LINES - 1)
        })
        .child(match find {
            Some(find) => find.text(message, 0).into_any_element(),
            None => message.to_owned().into_any_element(),
        });
    let toggle = collapsible.then(|| {
        let state = expanded.clone();
        div()
            .id(SharedString::from(format!("{id}-toggle")))
            .role(gpui::Role::Button)
            .aria_label(if collapsed { "Show more" } else { "Show less" })
            .mt(px(6.))
            .h(px(21.))
            .self_start()
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                state.update(cx, |open, cx| {
                    *open = !*open;
                    cx.notify();
                })
            })
            .child(text(
                if collapsed { "Show more" } else { "Show less" },
                14.,
                21.,
                430.,
                t.text_tertiary,
            ))
            .when(collapsed, |d| {
                d.child(icon("chevron-down-small", 14., t.text_tertiary))
            })
    });
    div().flex().flex_col().items_end().pb(px(reserve)).child(
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
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .w_full()
                            .child(body)
                            .when(collapsed, |d| d.child(text("…", 14., LINE, 430., white()))),
                    )
                    .children(toggle),
            ),
    )
}

/// A row that wraps its word boxes like an inline formatting context. Chromium lets a line's
/// trailing space hang past the wrap width, and each word box carries its trailing space.
/// ponytail: a fixed 4 px allowance (one 14 px space) instead of a hanging space.
pub(super) fn wrap_row() -> Div {
    div().flex().flex_wrap().mr(px(-4.))
}

/// `Worked for 17s ›` over a hairline (turn disclosure); `angle` turns the chevron clockwise
/// in degrees (90 = open). At rest it is the fitted icon; while it turns, the svg path is
/// rotated about its viewBox centre and rasterized like any icon (no clipping: the path stays
/// 5.5 px from the centre). The sprite fallback (Linux) rotates the mask instead.
/// `None` draws the running status without a disclosure control.
pub(super) fn disclosure(label: String, angle: Option<f32>, window: &Window, t: &Theme) -> Div {
    let color = t.text_tertiary;
    let chevron = angle.map(|angle| {
        if angle == 0. {
            icon("thread-chevron-right", 14., color).into_any_element()
        } else {
            canvas(
                |_, _, _| {},
                move |bounds, (), window, cx| {
                    let path = "icons/thread-chevron-right.svg";
                    let Ok(Some(src)) = cx.asset_source().load(path) else {
                        return;
                    };
                    // ponytail: one parsed icon per 0.1 deg (at most 900), keyed by the angle.
                    let angle = (angle * 10.).round() / 10.;
                    let (sin, cos) = angle.to_radians().sin_cos();
                    let rotated = String::from_utf8_lossy(&src)
                        .replacen(
                            "><path",
                            &format!(
                                "><g transform=\"matrix({cos} {sin} {} {cos} {} {})\"><path",
                                -sin,
                                10. - 10. * cos + 10. * sin,
                                10. - 10. * sin - 10. * cos
                            ),
                            1,
                        )
                        .replacen("</svg>", "</g></svg>", 1);
                    let key = format!("{path}@{angle:.1}");
                    if shapes::paint_icon(
                        rotated.as_bytes(),
                        &key,
                        bounds,
                        false,
                        color,
                        None,
                        window,
                        cx,
                    ) {
                        return;
                    }
                    let scale = window.scale_factor();
                    let center = bounds.center();
                    let matrix = TransformationMatrix::unit()
                        .translate(center.scale(scale))
                        .rotate(radians(angle.to_radians()))
                        .translate(center.scale(-scale));
                    window
                        .paint_svg(bounds, path.into(), None, matrix, color, cx)
                        .ok();
                },
            )
            .flex_none()
            .w(px(14.))
            .h(px(14.))
            .into_any_element()
        }
    });
    // In a column narrower than the label and chevron the chevron wraps under the label (the
    // button's `flex-wrap`, `gap-1` both ways) and the label overflows the column. Beside the
    // label the row sits 1.3 px below its line box's top (the button's inline baseline).
    let line = markdown::line_width();
    let wraps = line > 0. && f32::from(width(window, &label, 14., 430., None)) + 4. + 14. > line;
    div()
        .flex()
        .flex_col()
        .child(
            div().when(!wraps, |d| d.min_h(px(22.3)).pt(px(1.3))).child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
                    .child(measured(window, label, 14., 21., 430., color, None))
                    .children(chevron),
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
    focus: &TurnFocus,
    window: &Window,
    t: &Theme,
    cx: &App,
) -> Option<Div> {
    let changes: Vec<_> = items
        .iter()
        .flat_map(|i| match i {
            ThreadItem::FileChange { changes, .. } => changes.as_slice(),
            _ => &[],
        })
        .collect();
    let first = changes.first()?;
    let review_path = first.path.clone();
    let (added, removed) = diff_counts(changes.iter().copied());
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
    // `focus-visible:ring-2 ring-offset-0` on the `rounded-button-toolbar` (12.5 px) button.
    let button = |label: &'static str,
                  id: &'static str,
                  focus: &gpui::Entity<crate::focus::ControlFocus>,
                  decoration: Option<gpui::AnyElement>| {
        crate::focus::shadow_ring(
            div()
                .id(id)
                .relative()
                .h(px(28.))
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(9.))
                .children(decoration)
                .child(measured(window, label, 13., 18., 430., t.text, None)),
            focus,
            Theme::RADIUS_LG,
            px(2.),
            cx,
        )
    };
    let engaged = focus.engaged;
    let card_group = SharedString::from(format!("diff-card-{turn_id}"));
    Some(
        div()
            .relative()
            .group(card_group.clone())
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
                    // The card clips its buttons in a narrow column.
                    .overflow_hidden()
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
                            // `turn-diff-default-subtitle` / `turn-diff-hover-subtitle`: hover or
                            // focus within the card swaps the counts for `View changes`.
                            .child({
                                let default = div().h(px(19.5)).pt(px(3.)).overflow_hidden().child(
                                    crate::screens::diff::panel::stats(added, removed, 13., t),
                                );
                                let hover = div()
                                    .h(px(19.5))
                                    .overflow_hidden()
                                    .items_center()
                                    .gap(px(4.))
                                    .child(measured(
                                        window,
                                        "View changes",
                                        13.,
                                        19.5,
                                        430.,
                                        t.text_secondary,
                                        None,
                                    ))
                                    .child(icon("thread-chevron-right", 14., t.text_secondary));
                                // Both lines occupy the same box; only paint swaps.
                                let default = if engaged {
                                    default.invisible()
                                } else {
                                    default.group_hover(card_group.clone(), |s| s.invisible())
                                };
                                let hover = hover.absolute().top_0().left_0().w_full().flex();
                                let hover = if engaged {
                                    hover
                                } else {
                                    hover
                                        .invisible()
                                        .group_hover(card_group.clone(), |s| s.visible())
                                };
                                div().relative().child(default).child(hover)
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(8.))
                            .child({
                                let turn_id = turn_id.to_owned();
                                let action = PatchTurn {
                                    thread_id: focus.thread_id.clone(),
                                    turn_id,
                                };
                                let pending = focus.patch_pending;
                                button(
                                    if focus.reverted { "Reapply" } else { "Undo" },
                                    "undo-turn",
                                    &focus.undo,
                                    None,
                                )
                                .child(if focus.reverted {
                                    icon("thread-undo", 14., t.text).flip_x()
                                } else {
                                    icon("thread-undo", 14., t.text)
                                })
                                .when(pending, |button| button.opacity(0.5))
                                .on_click({
                                    let thread_id = focus.thread_id.clone();
                                    move |_, window, cx| {
                                        if pending {
                                            return;
                                        }
                                        window.dispatch_action(
                                            Box::new(ThreadPressed(thread_id.clone())),
                                            cx,
                                        );
                                        window.dispatch_action(Box::new(action.clone()), cx)
                                    }
                                })
                            })
                            // Blink's background bleed avoidance paints the fill and the
                            // translucent border into one transparency layer (shapes' clip
                            // layer op), rasterized in the scroller's layer like the card.
                            .child(
                                button(
                                    "View changes",
                                    "view-changes",
                                    &focus.view,
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
                                .on_click({
                                    let thread_id = focus.thread_id.clone();
                                    move |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(ThreadPressed(thread_id.clone())),
                                            cx,
                                        );
                                        window.dispatch_action(
                                            Box::new(crate::ViewChanges(review_path.clone())),
                                            cx,
                                        )
                                    }
                                }),
                            ),
                    ),
            ),
    )
}

/// Added and removed line counts of `changes`' unified diffs.
fn diff_counts<'a>(changes: impl Iterator<Item = &'a FileUpdateChange> + Clone) -> (usize, usize) {
    let lines = |prefix: char| {
        changes
            .clone()
            .flat_map(|c| c.diff.lines())
            .filter(|l| l.starts_with(prefix) && !l.starts_with("+++") && !l.starts_with("---"))
            .count()
    };
    (lines('+'), lines('-'))
}

/// One entry of a turn's activity column (`group/agent-activity`), in order.
pub(super) enum Activity<'a> {
    /// An agent message, by its item id and text.
    Markdown(&'a str, &'a str),
    /// A command still running, by its command line.
    Running(&'a str),
    Mcp(&'a ThreadItem),
    /// A reasoning item seen live: `Thought for 3s` once completed, its summary under it while
    /// open (keyed `reasoning-<id>`); `Thinking` while it streams, with the streamed text
    /// under it.
    Reasoning {
        key: String,
        label: String,
        text: String,
        live: bool,
    },
    /// A `contextCompaction` item: `Compacting context`, `Context compacted`, …
    Compaction(&'static str),
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
    /// A `Ran` row's shell card, shown under it while open under `shell.key`.
    shell: Option<Shell>,
}

/// The shell card of a command (`ShellOutputBlock`): `$ command`, its output, exit footer.
pub(super) struct Shell {
    key: String,
    command: String,
    output: String,
    /// `Success`, `Exit code 1`; empty while the command runs.
    footer: String,
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
            Activity::Markdown(_, text) => vec![(*text).to_owned()],
            Activity::Running(command) => vec![(*command).to_owned()],
            Activity::Mcp(item) => vec![mcp_label(item)],
            Activity::Reasoning {
                key,
                label,
                text,
                live,
            } => std::iter::once(label.clone())
                .chain((*live || open.contains(key)).then(|| text.clone()))
                .filter(|s| !s.is_empty())
                .collect(),
            Activity::Compaction(label) => vec![(*label).to_owned()],
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
pub(super) fn activities<'a>(
    items: &'a [ThreadItem],
    running: bool,
    view: &ThreadView,
    visibility: super::ReasoningVisibility,
    now: Instant,
) -> Vec<Activity<'a>> {
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
            ThreadItem::Reasoning {
                id,
                summary,
                content,
            } if !visibility.hide => {
                let text = visibility.text(summary, content);
                let (label, text, live) = if view.thinking(id) {
                    let body = super::context::without_heading(&text).to_owned();
                    ("Thinking".to_owned(), body, true)
                } else if let Some(secs) = view.reasoning_secs.get(id)
                    && !text.is_empty()
                {
                    (format!("Thought for {}", duration(*secs)), text, false)
                } else {
                    continue;
                };
                flush(&mut out, &mut group);
                out.push(Activity::Reasoning {
                    key: format!("reasoning-{id}"),
                    label,
                    text,
                    live,
                });
            }
            ThreadItem::ContextCompaction { id } => {
                flush(&mut out, &mut group);
                out.push(Activity::Compaction(
                    view.compaction_label(id, running, now),
                ));
            }
            ThreadItem::McpToolCall { .. } if automation_item(item).is_some() => {}
            ThreadItem::McpToolCall { .. } => {
                flush(&mut out, &mut group);
                out.push(Activity::Mcp(item));
            }
            ThreadItem::CommandExecution { .. } | ThreadItem::FileChange { .. } => {
                group.push(item);
            }
            ThreadItem::AgentMessage { id, text, .. } | ThreadItem::Plan { id, text, .. }
                if !text.is_empty() =>
            {
                flush(&mut out, &mut group);
                out.push(Activity::Markdown(id, text));
            }
            _ => {}
        }
    }
    flush(&mut out, &mut group);
    out
}

/// The summary and rows of one group. Summary parts come in the original's order (loaded
/// tools, file changes, exploration, commands), the first one capitalised; the group's icon is
/// the first part's.
fn group_entry<'a>(items: &[&'a ThreadItem]) -> Option<Activity<'a>> {
    let key = item_id(items.first()?)?;
    let (mut files, mut reads, mut commands) = (0, 0, 0);
    // Distinct skill definition files read (`loadedToolPaths`).
    let mut tools = HashSet::new();
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        match item {
            ThreadItem::CommandExecution {
                id,
                command,
                cwd,
                command_actions,
                status,
                aggregated_output,
                exit_code,
                ..
            } => {
                let skills: Vec<String> = command_actions
                    .iter()
                    .filter(|a| a.kind == "read")
                    .filter_map(|a| a.path.as_deref().or(a.name.as_deref()))
                    .map(|p| match p.starts_with('/') {
                        true => p.to_owned(),
                        false => format!("{cwd}/{p}"),
                    })
                    .filter(|p| skill_name(p, cwd).is_some())
                    .collect();
                // The server's parse: all `read` / `search` / `list_files` is exploration.
                let exploration = !command_actions.is_empty()
                    && command_actions
                        .iter()
                        .all(|a| matches!(a.kind.as_str(), "read" | "search" | "list_files"));
                if !skills.is_empty() && skills.len() == command_actions.len() {
                    tools.extend(skills);
                } else if exploration {
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
                    [] => shell_command(command, command_actions).to_owned(),
                    actions => actions
                        .iter()
                        .map(|a| a.command.as_str())
                        .collect::<Vec<_>>()
                        .join("; "),
                };
                // A skill's `SKILL.md` read names the skill (`toolSummaryForCmd.readSkillDefinition`).
                let skill = match command_actions.as_slice() {
                    [a] if a.kind == "read" => a
                        .path
                        .as_deref()
                        .or(a.name.as_deref())
                        .and_then(|p| skill_name(p, cwd)),
                    _ => None,
                };
                rows.push(if let Some(skill) = skill {
                    GroupRow {
                        icon: "thread-wrench",
                        verb: format!("Read {skill} skill"),
                        file: None,
                        stats: None,
                        shell: None,
                    }
                } else if names.is_empty() {
                    // ponytail: `Stopped` (an interrupted turn) is not told apart from a failure.
                    let footer = match (status.as_str(), exit_code) {
                        ("inProgress", _) => String::new(),
                        (_, Some(0)) => "Success".into(),
                        (_, Some(code)) => format!("Exit code {code}"),
                        (_, None) => "Exit code unknown".into(),
                    };
                    GroupRow {
                        icon: "thread-terminal",
                        verb: format!("Ran {parsed}"),
                        file: None,
                        stats: None,
                        shell: Some(Shell {
                            key: format!("shell-{id}"),
                            command: shell_command(command, command_actions).to_owned(),
                            output: aggregated_output
                                .as_deref()
                                .unwrap_or_default()
                                .trim_end()
                                .to_owned(),
                            footer,
                        }),
                    }
                } else {
                    GroupRow {
                        icon: "thread-read",
                        verb: "Read".into(),
                        file: Some(names.join(", ")),
                        stats: None,
                        shell: None,
                    }
                });
            }
            ThreadItem::FileChange { changes, .. } => {
                files += changes.len();
                let Some(first) = changes.first() else {
                    continue;
                };
                let name = first.path.rsplit('/').next().unwrap_or(&first.path);
                rows.push(GroupRow {
                    icon: "pencil",
                    verb: "Edited".into(),
                    file: Some(name.to_owned()),
                    stats: Some(diff_counts(changes.iter())),
                    shell: None,
                });
            }
            _ => {}
        }
    }
    let count = |n: usize, one, many| (n > 0).then_some(if n == 1 { one } else { many });
    let parts: Vec<(&str, &'static str)> = [
        (
            count(tools.len(), "loaded a tool", "loaded tools"),
            "thread-wrench",
        ),
        (count(files, "edited a file", "edited files"), "pencil"),
        (count(reads, "read files", "read files"), "thread-read"),
        (
            count(commands, "ran a command", "ran commands"),
            "thread-terminal",
        ),
    ]
    .into_iter()
    .filter_map(|(part, icon)| Some((part?, icon)))
    .collect();
    let icon = parts.first()?.1;
    let mut summary = parts.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
    summary[..1].make_ascii_uppercase();
    Some(Activity::Group {
        key,
        summary,
        icon,
        rows,
    })
}

/// The display name of the skill whose `SKILL.md` `path` (relative to `cwd`) reads, as the
/// original's skill path parse tells it: `.codex|.agents/skills/[_import|.system/]<id>/SKILL.md`,
/// or under `plugins/` either `[cache/<marketplace>/]<plugin>/[<version>/]…/skills/<id>/SKILL.md`
/// or a plugin's own `SKILL.md` (named by the plugin). Segments match case-insensitively.
fn skill_name(path: &str, cwd: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    let path = match path.starts_with('/') {
        true => path,
        false => format!("{cwd}/{path}"),
    };
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let is = |i: usize, s: &str| segs.get(i).is_some_and(|x| x.eq_ignore_ascii_case(s));
    let definition = |rest: &[&str]| matches!(rest, [f] if f.eq_ignore_ascii_case("skill.md"));
    for i in 0..segs.len() {
        if (is(i, ".codex") || is(i, ".agents")) && is(i + 1, "skills") {
            let id = i + 2 + usize::from(is(i + 2, "_import") || is(i + 2, ".system"));
            if id < segs.len() {
                return definition(&segs[id + 1..]).then(|| humanize(segs[id]));
            }
        }
    }
    for n in 0..segs.len() {
        if !is(n, "plugins") {
            continue;
        }
        let cache = is(n + 1, "cache");
        let plugin = n + if cache { 3 } else { 1 };
        if plugin >= segs.len() {
            continue;
        }
        let root = plugin + usize::from(cache);
        match (root + 1..segs.len()).find(|&t| is(t, "skills")) {
            Some(s) if s + 1 < segs.len() => {
                return definition(&segs[s + 2..]).then(|| humanize(segs[s + 1]));
            }
            Some(_) => {}
            None if definition(segs.get(root + 1..).unwrap_or_default()) => {
                return Some(humanize(segs[plugin]));
            }
            None => {}
        }
    }
    None
}

/// A skill or plugin id in title case, as the original's humanizer (`gh-fix_ci:openai` →
/// `GH Fix CI: OpenAI`).
fn humanize(id: &str) -> String {
    const ACRONYMS: [&str; 16] = [
        "GH", "IA", "MCP", "API", "CI", "CLI", "LLM", "PDF", "PR", "RCS", "UI", "URL", "SQL", "TW",
        "GPU", "CPU",
    ];
    const BRANDS: [(&str, &str); 10] = [
        ("openai", "OpenAI"),
        ("openaideveloperdocs", "OpenAI Developer Docs"),
        ("openapi", "OpenAPI"),
        ("github", "GitHub"),
        ("imessage", "iMessage"),
        ("pagerduty", "PagerDuty"),
        ("datadog", "DataDog"),
        ("sharepoint", "SharePoint"),
        ("sqlite", "SQLite"),
        ("fastapi", "FastAPI"),
    ];
    let word = |i: usize, w: &str| {
        let upper = w.to_uppercase();
        if ACRONYMS.contains(&upper.as_str()) {
            return upper;
        }
        if let Some(stem) = upper.strip_suffix('S')
            && ACRONYMS.contains(&stem)
        {
            return format!("{stem}s");
        }
        let lower = w.to_lowercase();
        if let Some((_, brand)) = BRANDS.iter().find(|(id, _)| *id == lower) {
            return (*brand).to_owned();
        }
        if i > 0 && ["and", "or", "to", "up", "with"].contains(&lower.as_str()) {
            return lower;
        }
        let mut chars = lower.chars();
        chars
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
    };
    id.split(':')
        .map(|part| {
            part.split(|c: char| c == '-' || c == '_' || c.is_whitespace())
                .filter(|w| !w.is_empty())
                .enumerate()
                .map(|(i, w)| word(i, w))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(": ")
}

/// A turn's items without a final answer: agent messages, running command rows and activity
/// groups. A group is one 21 px summary row (`text/60`, white on hover, with a 14 px chevron
/// that shows on hover and points down while open); open, its rows follow at 4 px gaps.
/// Clicking the summary toggles `open` under the group's key.
pub(super) fn activity(
    items: &[ThreadItem],
    running: bool,
    view: &ThreadView,
    window: &Window,
    t: &Theme,
    cx: &Context<ThreadView>,
) -> Div {
    let open = &view.expanded;
    let fades = &view.stream_fades;
    let images = &view.mcp_images;
    let text60 = t.text_alpha_60;
    // Exact-width 14 px text, styled before it is boxed (see [`measured`]). The box rounds the
    // width up, or its `truncate` cuts a run that fits (`Read math.…`).
    let label = |s: String, color: Hsla| {
        let w = fit_width(window, &s, 14., 430., None);
        (text(s, 14., 21., 430., color), w)
    };
    // The label is exactly as wide as its text, and shrinks with an ellipsis (`truncate`) in a
    // column too narrow for it.
    let boxed = |label: gpui::AnyElement, w: Pixels| {
        div()
            .w(w)
            .min_w_0()
            .flex_shrink(1.)
            .overflow_hidden()
            .text_ellipsis()
            .child(label)
    };
    let plain = |(label, w): (Div, Pixels)| boxed(label.into_any_element(), w);
    // A 21 px line: a 16 px icon, then its words.
    let row = |icon_name: &'static str, words: Div| {
        div()
            .h(px(21.))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(icon(icon_name, 16., text60))
            .child(words)
    };
    // A row that opens `key`: its label whitens on hover, its 14 px chevron shows on hover and
    // points down while open.
    let disclosure = |key: String, icon_name: &'static str, (label, w): (Div, Pixels), is_open| {
        let group = SharedString::from(format!("activity-group-{key}"));
        // Group hover recolours text only on a stateful element: the run colour is fixed at
        // layout, before this frame's group hitbox exists.
        let label = label
            .id(SharedString::from(format!("activity-summary-{key}")))
            .group_hover(group.clone(), |s| s.text_color(t.text));
        let chevron = if is_open {
            "chevron-down"
        } else {
            "thread-chevron-right"
        };
        div()
            .id(group.clone())
            .group(group.clone())
            .min_w_0()
            .max_w_full()
            .self_start()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(row(icon_name, boxed(label.into_any_element(), w)))
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
            }))
    };
    let mut col = div().flex().flex_col().gap(px(16.));
    let now = cx.background_executor().now();
    for entry in activities(
        items,
        running,
        view,
        super::ReasoningVisibility::get(cx),
        now,
    ) {
        col = col.child(match entry {
            Activity::Markdown(id, text) => {
                let segments = fades.get(id).map(Vec::as_slice).unwrap_or_default();
                markdown(text, window, t, segments)
            }
            Activity::Running(command) => {
                row("thread-terminal", plain(label(command.to_owned(), text60)))
            }
            Activity::Reasoning {
                key: _,
                label: words,
                text,
                live: true,
            } => div()
                .w_full()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(4.))
                .child(row("thread-summary", plain(label(words, text60))))
                .when(!text.is_empty(), |d| d.child(text60_block(text, t))),
            Activity::Reasoning {
                key,
                label: words,
                text,
                ..
            } => {
                let is_open = open.contains(&key);
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(4.))
                    .child(disclosure(
                        key,
                        "thread-summary",
                        label(words, text60),
                        is_open,
                    ))
                    .when(is_open, |d| d.child(text60_block(text, t)))
            }
            Activity::Compaction(words) => row(
                "thread-compaction",
                plain(label(words.to_owned(), text60)),
            ),
            Activity::Mcp(item) => {
                let ThreadItem::McpToolCall {
                    id,
                    status,
                    result,
                    error,
                    progress,
                    ..
                } = item
                else {
                    unreachable!();
                };
                let key = format!("mcp-{id}");
                let is_open = open.contains(&key);
                let color = if status == "failed" || error.is_some() {
                    t.text
                } else {
                    text60
                };
                let mut content = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(disclosure(
                        key,
                        "thread-mcp",
                        label(mcp_label(item), color),
                        is_open,
                    ));
                if is_open {
                    for update in progress {
                        if let Some(message) = update["message"].as_str() {
                            content =
                                content.child(text(message.to_owned(), 14., 21., 430., text60));
                        }
                    }
                    if let Some(result) = result {
                        if let Some(blocks) = result["content"].as_array() {
                            let mut output = div().flex().flex_col().gap(px(2.));
                            for (index, block) in blocks.iter().enumerate() {
                                if let Some(text) = block["text"].as_str() {
                                    output = output.child(mcp_output(
                                        format!("{id}-text-{index}"),
                                        "plaintext",
                                        text,
                                        t,
                                    ));
                                } else if block["type"] == "image" {
                                    let mut images = images.borrow_mut();
                                    let decoded = images.entry(id.clone()).or_insert_with(|| {
                                        blocks
                                            .iter()
                                            .enumerate()
                                            .filter_map(|(index, block)| {
                                                let format = ImageFormat::from_mime_type(
                                                    block["mimeType"].as_str()?,
                                                )?;
                                                let data =
                                                    base64::engine::general_purpose::STANDARD
                                                        .decode(block["data"].as_str()?)
                                                        .ok()?;
                                                Some((
                                                    index,
                                                    Arc::new(Image::from_bytes(format, data)),
                                                ))
                                            })
                                            .collect()
                                    });
                                    if let Some((_, image)) =
                                        decoded.iter().find(|(at, _)| *at == index)
                                    {
                                        let context = crate::image_context::ImageContext {
                                            image: image.clone(),
                                            path: None,
                                        };
                                        output = output.child(
                                            gpui::img(image.clone()).max_w_full()
                                                .on_mouse_down(gpui::MouseButton::Right, cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                                                    this.menu = Some(context.menu(event.position, window, cx));
                                                    this.context_image = Some(context.clone());
                                                    cx.stop_propagation();
                                                    cx.notify();
                                                })),
                                        );
                                    } else {
                                        output = output.child(text(
                                            "Could not display tool image",
                                            14.,
                                            21.,
                                            430.,
                                            t.text,
                                        ));
                                    }
                                }
                            }
                            content = content.child(output);
                        }
                        if let Some(structured) = result.get("structuredContent") {
                            content = content.child(mcp_output(
                                format!("{id}-structured"),
                                "json",
                                &format!("{structured:#}"),
                                t,
                            ));
                        }
                    }
                    if let Some(error) = error {
                        content = content.child(mcp_output(
                            format!("{id}-error"),
                            "plaintext",
                            error["message"].as_str().unwrap_or("Tool call failed"),
                            t,
                        ));
                    }
                }
                content
            }
            Activity::Group {
                key,
                summary,
                icon: icon_name,
                rows,
            } => {
                let is_open = open.contains(key);
                let header = disclosure(key.to_owned(), icon_name, label(summary, text60), is_open);
                let mut col = div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(4.))
                    .child(header);
                if is_open {
                    col = col.children(rows.into_iter().map(|r| {
                        if let Some(shell) = r.shell {
                            let is_open = open.contains(&shell.key);
                            let key = shell.key.clone();
                            return div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .items_start()
                                .child(disclosure(key, r.icon, label(r.verb, text60), is_open))
                                .when(is_open, |d| d.child(shell_card(&shell, t)));
                        }
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

fn text60_block(body: String, t: &Theme) -> Div {
    div()
        .w_full()
        .child(text(body, 14., 21., 430., t.text_alpha_60))
}

fn mcp_output(id: String, language: &'static str, value: &str, t: &Theme) -> Div {
    let color = t.text_alpha_60.opacity(5. / 6.);
    div()
        .relative()
        .w_full()
        .p(px(1.))
        .child(shapes::decoration(shapes::Decoration {
            radius: Corners::all(px(12.5)),
            corner: shapes::Corner::Squircle,
            fill: Some(t.ghost_hover),
            border: Some((px(1.), t.border_strong)),
            ..Default::default()
        }))
        .child(
            div()
                .px(px(8.))
                .py(px(4.))
                .layer()
                .child(text(language, 13., 18.570_313, 430., color)),
        )
        .child(
            div()
                .id(SharedString::from(id))
                .max_h(px(192.))
                .overflow_y_scroll()
                .layer()
                .p(px(8.))
                .child(
                    text(value.to_owned(), 14., 22.75, 430., color)
                        .font_family(MONO)
                        .whitespace_normal(),
                ),
        )
}

fn mcp_label(item: &ThreadItem) -> String {
    let ThreadItem::McpToolCall {
        server,
        tool,
        arguments,
        ..
    } = item
    else {
        unreachable!();
    };
    if tool == "js"
        && let Some(title) = arguments["title"].as_str()
    {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if !title.is_empty() {
            return if title.chars().count() > 80 {
                format!("{}…", title.chars().take(79).collect::<String>().trim_end())
            } else {
                title
            };
        }
    }
    format!("{server}.{tool}")
}

/// The card under an open `Ran` row, 8 px below it and 4 px above the next row, 10 px short
/// of the column (the original's row list keeps that for its scrollbar): a 27 px `Shell`
/// header, `$ command`, the output (144 px max; ponytail: scrolls from the top, the original
/// sticks to the bottom) and a 27 px footer with `Success` or `Exit code N` at the right.
fn shell_card(shell: &Shell, t: &Theme) -> Div {
    let description = t.text.opacity(0.7);
    let mono =
        |s: String, weight: f32, color: Hsla| text(s, 13., 19.5, weight, color).font_family(MONO);
    let footer = div()
        .h(px(27.))
        .pt(px(2.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(4.))
        .when(shell.footer == "Success", |d| {
            d.child(icon("thread-check", 12., t.text_tertiary))
        })
        .when(!shell.footer.is_empty(), |d| {
            d.child(text(shell.footer.clone(), 14., 21., 430., t.text_tertiary))
        });
    div().w_full().pt(px(8.)).pb(px(4.)).pr(px(10.)).child(
        div()
            .w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(Theme::RADIUS_LG)
            .border_1()
            .border_color(t.border_strong)
            .bg(t.ghost_hover)
            .child(
                div()
                    .h(px(27.))
                    .px(px(8.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Shell", 13., 18.571_428, 430., description))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(copy_control(
                                format!("{}-copy-command", shell.key),
                                "Copy command",
                                &shell.command,
                                t,
                            ))
                            .child(copy_control(
                                format!("{}-copy-output", shell.key),
                                "Copy output",
                                &shell.output,
                                t,
                            )),
                    ),
            )
            .child(
                div()
                    .pt(px(8.))
                    .pl(px(8.))
                    .pr(px(24.))
                    .flex()
                    .gap(px(8.))
                    .child(mono("$".into(), 430., t.text_tertiary))
                    .child(mono(shell.command.clone(), 430., description)),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{}-output", shell.key)))
                    .max_h(px(144.))
                    .overflow_y_scroll()
                    .child(div().p(px(8.)).child(mono(
                        if shell.output.is_empty() {
                            "No output".into()
                        } else {
                            shell.output.clone()
                        },
                        500.,
                        description,
                    ))),
            )
            .child(footer),
    )
}

/// A 20 px copy button of the shell card header: puts `body` on the clipboard.
fn copy_control(id: String, label: &'static str, body: &str, t: &Theme) -> impl IntoElement {
    let body = body.to_owned();
    tooltip(
        div()
            .id(SharedString::from(id))
            .role(gpui::Role::Button)
            .aria_label(label)
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(Theme::RADIUS_MD)
            .hover(|s| s.bg(t.ghost_hover))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(body.clone()))
            })
            .child(icon("thread-copy", 14., t.text_tertiary)),
        label,
        None,
        Side::Bottom,
    )
}

/// `+4`, `-0` in one colour: 13 px tabular figures, tracked like the diff header's.
fn stats(added: usize, removed: usize, color: Hsla) -> [Div; 2] {
    let mut features = vec![("tnum".into(), 1), ("cv01".into(), 1), ("cv02".into(), 1)];
    features.extend(textsys::letter_spacing(-0.025 * 13.).0.iter().cloned());
    let features = gpui::FontFeatures(Arc::new(features));
    [format!("+{added}"), format!("-{removed}")]
        .map(|s| text(s, 13., 13., 430., color).font_features(features.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_name_follows_the_original_parse() {
        for (path, name) in [
            (
                "/Users/a/.codex/skills/operating-rules/SKILL.md",
                "Operating Rules",
            ),
            ("/Users/a/.codex/skills/.system/plan/skill.md", "Plan"),
            (
                "/Users/a/.codex/skills/_import/pdf_tools-and-apis/SKILL.md",
                "PDF Tools and APIs",
            ),
            ("/repo/.agents/skills/lint/SKILL.md", "Lint"),
            (".agents/skills/to-do/SKILL.md", "To Do"),
            (
                "/Users/a/.codex/plugins/cache/mkt/github/1.0/skills/gh-fix-ci/SKILL.md",
                "GH Fix CI",
            ),
            ("/Users/a/.codex/plugins/github/SKILL.md", "GitHub"),
        ] {
            assert_eq!(skill_name(path, "/repo").as_deref(), Some(name), "{path}");
        }
        for path in [
            "/Users/a/.codex/skills/operating-rules/scripts/run.md",
            "/Users/a/.codex/skills/SKILL.md",
            "/repo/skills/lint/SKILL.md",
            "/repo/README.md",
        ] {
            assert_eq!(skill_name(path, "/repo"), None, "{path}");
        }
    }

    #[test]
    fn skill_reads_summarise_as_loaded_tools() {
        let read = |id: &str, path: &str| -> ThreadItem {
            serde_json::from_value(serde_json::json!({
                "type": "commandExecution", "id": id, "command": format!("cat {path}"),
                "cwd": "/Users/a/proj", "status": "completed", "exitCode": 0,
                "commandActions": [{"type": "read", "command": format!("cat {path}"),
                    "name": "SKILL.md", "path": path}],
            }))
            .unwrap()
        };
        let summary = |items: &[ThreadItem]| match group_entry(&items.iter().collect::<Vec<_>>()) {
            Some(Activity::Group { summary, icon, .. }) => (summary, icon),
            _ => unreachable!(),
        };
        let one = read("a", "/Users/a/.codex/skills/rules/SKILL.md");
        assert_eq!(
            summary(std::slice::from_ref(&one)),
            ("Loaded a tool".into(), "thread-wrench")
        );
        let Some(Activity::Group { rows, .. }) = group_entry(&[&one]) else {
            unreachable!()
        };
        assert_eq!(
            (rows[0].icon, rows[0].verb.as_str(), &rows[0].file),
            ("thread-wrench", "Read Rules skill", &None)
        );
        let items = [
            one.clone(),
            read("b", "/Users/a/.codex/skills/talk/SKILL.md"),
            one,
            read("c", "src/main.rs"),
        ];
        assert_eq!(
            summary(&items),
            ("Loaded tools, read files".into(), "thread-wrench")
        );
    }
}
