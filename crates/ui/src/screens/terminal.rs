//! `terminal` parity screen: the subtract thread after a replayed turn whose `python3` check is
//! still running when the turn completes (a background terminal), with the summary panel open
//! and the terminal's tab in the side panel.

use bridge::Event;
use gpui::{
    Action, AnyView, App, AppContext as _, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, TextRun, Window, div, font, px,
};
use serde_json::{Value, json};
use theme::ActiveTheme as _;
use unicode_width::UnicodeWidthChar;

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
fn wrap_line(mut line: &str, columns: usize) -> impl Iterator<Item = &str> {
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let mut width = 0;
        let end = line
            .char_indices()
            .find_map(|(index, ch)| {
                let cells = if ch == '\t' {
                    8 - width % 8
                } else {
                    ch.width().unwrap_or(0)
                };
                if index > 0 && width + cells > columns {
                    Some(index)
                } else {
                    width += cells;
                    None
                }
            })
            .unwrap_or(line.len());
        let (row, rest) = line.split_at(end);
        line = rest;
        done = rest.is_empty();
        Some(row)
    })
}

pub fn thumbnail(thread: &ThreadView, window: &Window, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let (_, command, output) = thread.terminal().unwrap_or(("", "", ""));
    let mut lines = std::collections::VecDeque::with_capacity(8);
    let columns = terminal_columns(window, cx);
    let raw: Vec<_> = output.lines().rev().take(8).collect();
    for line in raw
        .into_iter()
        .rev()
        .flat_map(|line| wrap_line(line, columns))
    {
        if lines.len() == 8 {
            lines.pop_front();
        }
        lines.push_back(line);
    }
    div()
        .id("background-terminal-thumbnail")
        .w(px(320.))
        .p(px(12.))
        .rounded(theme::Theme::RADIUS_MD)
        .border_1()
        .border_color(t.border)
        .bg(t.surface_elevated_secondary)
        .shadow(vec![gpui::BoxShadow {
            color: t.shadow_sm,
            offset: gpui::point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(-1.),
            inset: false,
        }])
        .overflow_hidden()
        .child(text(command.to_owned(), 13., 18., 430., t.text))
        .child(
            div().mt(px(8.)).font_family(crate::MONO).children(
                lines
                    .into_iter()
                    .map(|line| text(line.to_owned(), 12., 16.5, 400., t.text).h(px(16.5))),
            ),
        )
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
