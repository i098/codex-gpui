//! `terminal` parity screen: the subtract thread after a replayed turn whose `python3` check is
//! still running when the turn completes (a background terminal), with the summary panel open
//! and the terminal's tab in the side panel.

use bridge::Event;
use gpui::{
    AnyView, App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Window, div, px, rgb,
};
use serde_json::{Value, json};
use theme::{ActiveTheme as _, Appearance};

use super::diff::panel::split_screen;
use super::sidebar::home_sidebar;
use super::thread::{subtract_thread_beside_panel, view::ThreadView};
use crate::text;

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
    let xterm = cx.new(|cx| {
        cx.observe(&thread, |_, _, cx| cx.notify()).detach();
        Xterm {
            thread: thread.clone(),
        }
    });
    let sidebar = home_sidebar(cx, Some(&id));
    split_screen(
        sidebar,
        thread,
        Some((xterm.into(), "thread-terminal", label.into())),
        cx,
    )
}

/// `Background terminal` tab: the first background command's output (xterm DOM renderer,
/// 12/16.5 px monospace, 12 px padding).
struct Xterm {
    thread: Entity<ThreadView>,
}

impl Render for Xterm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        // xterm theme foreground.
        let fg = match t.appearance {
            Appearance::Dark => rgb(0xdfdfdf).into(),
            Appearance::Light => t.text,
        };
        let terminals = self.thread.read(cx).background_terminals();
        let output = terminals.first().map_or("", |(_, output)| output);
        // No fill: the main panel's rounded fill shows through, so its corners stay round.
        div()
            .size_full()
            .p(px(12.))
            .pr_0()
            .font_family(crate::MONO)
            .children(
                output
                    .lines()
                    .map(|line| text(line.to_owned(), 12., 16.5, 400., fg)),
            )
    }
}

/// The notifications tools/route1/server.mjs (`replayRunningCommand`) sends for the reference:
/// the subtract session's python3 check, started in a new turn that completes while the
/// command never gets `item/completed`.
fn running_command_replay() -> Vec<Event> {
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
