//! `terminal` parity screen: the subtract thread after a replayed turn whose `python3` check is
//! still running when the turn completes (a background terminal), with the summary panel open
//! and the terminal's tab in the side panel.

use bridge::Event;
use gpui::{
    Action, AnyView, App, AppContext as _, Context, Corners, Div, Entity, Font, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, TextRun, Window,
    div, font, prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use theme::ActiveTheme as _;

pub mod buffer;
pub mod surface;
pub use surface::{TerminalEvent, TerminalStyle, TerminalSurface};

use super::diff::panel::with_side_panel;
use super::sidebar::{HomeSidebar, home_sidebar};
use super::thread::{subtract_thread_beside_panel, view::ThreadView};
use crate::{Shell, text};

#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct OpenTerminalItem(pub String);

gpui::actions!(ui, [StopBackgroundTerminals]);

/// A bottom-panel tab's content, `tab_width` px wide, with its 14 px `×` while `selected` or
/// `hovered` (the caller's 20 px close control at `top-1.5 end-1.75` holds no glyph). The
/// original blits the selected tab's glyphs onto the translucent fill of its composited
/// `_SelectedSurface`, and every other glyph of the strip into a layer of its own.
/// `snap` moves that surface to the device grid: Chromium rounds a composited layer's origin
/// and keeps its unrounded size.
pub fn panel_tab(
    label: SharedString,
    selected: bool,
    hovered: bool,
    tab_width: f32,
    snap: f32,
    window: &Window,
    cx: &App,
) -> Div {
    let t = cx.theme();
    let closable = selected || hovered;
    let color = if selected { t.text } else { t.text_tertiary };
    let glyph = || crate::icon::icon("thread-terminal", 16., color);
    let close = || {
        crate::icon::icon("diff-tab-close", 14., t.text_tertiary)
            .absolute()
            .left(px(tab_width - 24.))
            .top(px(9.))
    };
    let width = window
        .text_system()
        .shape_line(
            label.clone(),
            px(13.),
            &[TextRun {
                len: label.len(),
                font: Font {
                    weight: FontWeight(430.),
                    ..font(crate::SANS)
                },
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        )
        .width;
    let fill = match t.appearance {
        theme::Appearance::Dark => t.ghost_hover,
        theme::Appearance::Light => t.surface_elevated_secondary,
    };
    div()
        .relative()
        .size_full()
        .flex()
        .items_center()
        .pl(px(10.))
        .pr(px(if closable { 27. } else { 7. }))
        .when(closable, |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(-1.))
                    .top_0()
                    .right(px(-1.))
                    .h(px(34.))
                    .layer()
                    .child(
                        div()
                            .absolute()
                            .left(px(1. + snap))
                            .top_0()
                            .right(px(1. - snap))
                            .h(px(32.))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(theme::Theme::RADIUS_MD),
                                corner: shapes::Corner::Squircle,
                                fill: Some(if selected { fill } else { t.ghost_hover }),
                                border: selected.then_some((px(0.5), t.border)),
                                shadows: if selected {
                                    vec![crate::composer::shadow(t.shadow_sm, 1., 2., -1., false)]
                                } else {
                                    vec![]
                                },
                                ..Default::default()
                            }))
                            .when(selected, |d| {
                                d.child(glyph().runtime().absolute().left(px(10.)).top(px(8.)))
                                    .child(close().runtime())
                            }),
                    ),
            )
        })
        .when(hovered && !selected, |d| d.child(close().own_layer(None)))
        // The selected glyph paints in the layer above; its box keeps the label's place.
        .when(selected, |d| d.child(div().flex_none().size(px(16.))))
        .when(!selected, |d| d.child(glyph().own_layer(None)))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .ml(px(4.))
                .overflow_hidden()
                .child(text(label, 13., 18.5714, 430., color))
                .when(
                    width > px(tab_width - if closable { 73. } else { 53. }),
                    |d| d.layer().child(shapes::x_fade(px(0.), px(16.))),
                ),
        )
}

pub fn workspace_warning(actions: Div, narrow: bool, cx: &App) -> Div {
    let t = cx.theme();
    let mut tint: gpui::Hsla = gpui::rgb(match t.appearance {
        theme::Appearance::Dark => 0x281105,
        theme::Appearance::Light => 0xfff5f0,
    })
    .into();
    tint.a = 0.3;
    let background = shapes::decoration(shapes::Decoration {
        radius: Corners::all(theme::Theme::RADIUS_3XL),
        corner: shapes::Corner::Squircle,
        fill: Some(crate::blit(tint, t.surface)),
        shadows: vec![
            crate::composer::shadow(t.border_strong, 0., 0., 0.5, false),
            crate::composer::shadow(gpui::rgba(0x0000000d).into(), 0., 2., 0., false),
            crate::composer::shadow(gpui::rgba(0x00000005).into(), 4., 6., 0., false),
        ],
        ..Default::default()
    });
    div().p(px(8.)).flex_shrink_0().child(
        div()
            .relative()
            .py(px(16.))
            .pl(px(20.))
            .pr(px(12.))
            .flex()
            .gap(px(12.))
            .when(!narrow, |d| d.items_center())
            .child(background)
            .child(
                div()
                    .h(px(20.))
                    .pt(px(2.))
                    .flex_shrink_0()
                    .child(crate::icon::icon("terminal-warning", 18., t.text_warning)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .when(narrow, |d| d.flex_col())
                    .when(!narrow, |d| d.items_center().gap(px(32.)))
                    .child(
                        text(
                            "This terminal is outside this chat's workspaces",
                            13.,
                            21.125,
                            430.,
                            t.text,
                        )
                        .whitespace_normal()
                        .min_w_0()
                        .flex_1(),
                    )
                    .child(actions.when(narrow, |d| d.mt(px(8.)))),
            ),
    )
}

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let thread = subtract_thread_beside_panel(window, cx, true);
    thread.update(cx, |view, cx| {
        for event in running_command_replay() {
            view.apply(&event, cx);
        }
        view.summary_open = true;
    });
    let view = thread.read(cx);
    let id = view.id().to_owned();
    let label = view
        .background_terminals()
        .first()
        .map(|(command, _)| command.clone())
        .unwrap_or_default();
    let xterm = cx.new(|cx| Xterm::new(thread.clone(), cx));
    // The turn completed while the user was away: the thread is unread ("needs attention").
    let sidebar = home_sidebar(cx, Some(&id));
    if let Ok(sidebar) = sidebar.clone().downcast::<HomeSidebar>() {
        sidebar.update(cx, |sidebar, _| sidebar.unread = vec![id]);
    }
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, thread.clone().into());
        shell.can_go_back = true;
        shell.attention = true;
        shell
    });
    let panel = (xterm.into(), "thread-terminal", label.into());
    with_side_panel(shell, thread, Some(panel), cx).into()
}

pub struct Xterm {
    surface: Entity<TerminalSurface>,
    output: String,
    selected: String,
}

impl Xterm {
    pub fn new(thread: Entity<ThreadView>, cx: &mut Context<Self>) -> Self {
        thread.update(cx, |thread, cx| thread.select_terminal(None, cx));
        let mut this = Self {
            surface: cx.new(|cx| TerminalSurface::new(false, cx)),
            output: String::new(),
            selected: String::new(),
        };
        this.update(&thread, cx);
        cx.observe(&thread, |this, thread, cx| this.update(&thread, cx))
            .detach();
        this
    }

    fn update(&mut self, thread: &Entity<ThreadView>, cx: &mut Context<Self>) {
        let (id, _, output) = thread.read(cx).terminal().unwrap_or(("", "", ""));
        if id == self.selected && output == self.output {
            return;
        }
        let append = id == self.selected && output.starts_with(&self.output);
        let delta = if append {
            &output[self.output.len()..]
        } else {
            output
        };
        let normalized = delta.replace("\r\n", "\n").replace('\n', "\r\n");
        if self.selected != id {
            self.selected.clear();
            self.selected.push_str(id);
        }
        if append {
            self.output.push_str(delta);
        } else {
            self.output.clear();
            self.output.push_str(output);
        }
        self.surface.update(cx, |surface, cx| {
            if append {
                surface.append(normalized.as_bytes(), cx);
            } else {
                surface.replace(normalized.as_bytes(), cx);
            }
        });
        cx.notify();
    }
}

impl Render for Xterm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_w_0()
            .p(px(if self.output.is_empty() { 16. } else { 12. }))
            .pr_0()
            .child(self.surface.clone())
    }
}

fn terminal_columns(window: &Window, cx: &App) -> usize {
    let cell = window
        .text_system()
        .shape_line(
            "M".into(),
            px(12.),
            &[TextRun {
                len: 1,
                font: font(crate::MONO),
                color: cx.theme().text,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        )
        .width;
    ((super::diff::panel::panel_width(window, cx) - px(26.)) / cell)
        .floor()
        .max(1.) as usize
}

pub fn thumbnail(thread: &ThreadView, window: &Window, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let (_, command, output) = thread.terminal().unwrap_or(("", "", ""));
    let mut buffer = buffer::Buffer::new(8, terminal_columns(window, cx).min(120) as u16);
    buffer.append(
        output
            .replace("\r\n", "\n")
            .replace('\n', "\r\n")
            .as_bytes(),
    );
    let mut end = (buffer.cursor.0 + 1).min(buffer.rows.len());
    let start = end.saturating_sub(8);
    while end > start
        && buffer.rows[end - 1]
            .cells
            .iter()
            .all(|cell| !cell.has_contents() || cell.contents().chars().all(char::is_whitespace))
    {
        end -= 1;
    }
    let rows = &buffer.rows[start..end];
    let style = TerminalStyle {
        size: 12.,
        ..cx.try_global::<TerminalStyle>()
            .cloned()
            .unwrap_or_default()
    };
    let radius = theme::Theme::RADIUS_XL;
    let width = f32::from(window.viewport_size().width).min(336.) - 16.;
    let shaped = window.text_system().shape_line(
        command.to_owned().into(),
        px(13.),
        &[TextRun {
            len: command.len(),
            font: Font {
                weight: FontWeight(500.),
                ..font(crate::SANS)
            },
            color: t.text,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let wrap_at_space = |start| {
        command[start..]
            .match_indices(char::is_whitespace)
            .take_while(|(i, _)| {
                shaped.x_for_index(start + i) - shaped.x_for_index(start) <= px(width - 16.)
            })
            .last()
            .map(|(i, _)| start + i)
    };
    let split = (shaped.width > px(width - 16.))
        .then(|| wrap_at_space(0))
        .flatten();
    let title = if let Some(split) = split {
        let remaining = command[split..].trim_start();
        let start = command.len() - remaining.len();
        let second = if shaped.width - shaped.x_for_index(start) > px(width - 16.)
            && let Some(end) = wrap_at_space(start)
        {
            format!("{}…", &command[start..end])
        } else {
            remaining.to_owned()
        };
        div()
            .child(text(command[..split].to_owned(), 13., 20., 500., t.text))
            .child(text(second, 13., 20., 500., t.text).text_ellipsis())
    } else {
        text(command.to_owned(), 13., 20., 500., t.text)
            .whitespace_normal()
            .line_clamp(2)
            .text_ellipsis()
    };
    let content = div()
        .flex()
        .flex_col()
        .px(px(8.))
        .py(px(12.))
        .gap(px(12.))
        .child(title)
        .child(div().p(px(4.)).font_family(style.font.clone()).children(
            rows.iter().enumerate().map(|(index, row)| {
                surface::row_element(
                    row,
                    index,
                    &style,
                    &surface::RowDecorations::default(),
                    gpui::size(px(style.size * 0.6025), px(16.)),
                    t.text,
                    cx,
                )
            }),
        ));
    let surface = div()
        .id("background-terminal-thumbnail")
        .relative()
        .w(px(width))
        .child(shapes::decoration(shapes::Decoration {
            radius: Corners::all(radius),
            corner: shapes::Corner::Squircle,
            fill: Some(t.surface_elevated_secondary),
            shadows: vec![
                crate::composer::shadow(t.border, 0., 0., 0.5, false),
                crate::composer::shadow(t.border, 0., 0., 0.5, false),
                crate::composer::shadow(gpui::rgba(0x0000001f).into(), 8., 16., -4., false),
            ],
            ..Default::default()
        }))
        .child(crate::rounded_clip(
            radius,
            shapes::Corner::Squircle,
            content,
        ));
    div()
        .ml(px(-20.))
        .mt(px(-12.))
        .pl(px(20.))
        .pt(px(12.))
        .layer()
        .child(surface)
}

/// The notifications tools/route1/server.mjs (`replayRunningCommand`) sends for the reference:
/// the subtract session's python3 check, started in a new turn that completes while the
/// command never gets `item/completed`.
pub fn running_command_replay() -> Vec<Event> {
    let msgs: Vec<Value> = include_str!("../../../../fixtures/sessions/subtract.jsonl")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).expect("fixture line is JSON")["msg"].take())
        .collect();
    let all = |method: &'static str| {
        msgs.iter()
            .filter(move |m| m["method"] == method)
            .map(|m| m["params"].clone())
    };
    let is_check = |p: &Value| {
        p["item"]["type"] == "commandExecution"
            && p["item"]["command"]
                .as_str()
                .is_some_and(|c| c.contains("python3"))
    };
    let turn_id = "01a13de9-70bc-7de1-8ae6-000000000001";
    let mut command = all("item/started").find(is_check).expect("python3 check");
    let item_id = format!(
        "{}-running",
        command["item"]["id"].as_str().unwrap_or_default()
    );
    command["turnId"] = turn_id.into();
    command["item"]["id"] = item_id.clone().into();
    let output = all("item/completed")
        .find(is_check)
        .expect("python3 output")["item"]["aggregatedOutput"]
        .take();
    let mut started = all("turn/started").next().expect("turn/started");
    started["turn"]["id"] = turn_id.into();
    let mut completed = all("turn/completed").next().expect("turn/completed");
    completed["turn"]["id"] = turn_id.into();
    completed["turn"]["items"] = json!([command["item"]]);
    let delta = json!({
        "threadId": command["threadId"],
        "turnId": turn_id,
        "itemId": item_id,
        "delta": output,
    });
    [
        ("turn/started", started),
        ("item/started", command),
        ("item/commandExecution/outputDelta", delta),
        ("turn/completed", completed),
    ]
    .map(|(method, params)| Event::Notification {
        method: method.into(),
        params,
    })
    .into()
}
