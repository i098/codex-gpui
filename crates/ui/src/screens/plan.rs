//! `Plan` side panel tab (`plan-side-panel` / `editable-plan-tab`): the thread's latest
//! finished plan as an editable document, saved like the original to
//! `$CODEX_HOME/plans/<thread>/<plan item>/PLAN.md`.

use bridge::types::{ThreadItem, Turn, TurnStatus};
use gpui::{
    App, Context, Entity, Focusable as _, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::*, px,
};
use std::{future::Future, pin::Pin, rc::Rc};
use theme::{ActiveTheme as _, Theme};

use crate::composer::Composer;
use crate::screens::thread::view::ThreadView;
use crate::text;

pub type Reply<T> = Pin<Box<dyn Future<Output = anyhow::Result<T>>>>;

/// What the app provides to the plan tab. Implementations resolve the live server
/// connection when called.
pub trait PlanPort {
    /// The plan's `PLAN.md` path and, when one was saved, its text.
    fn load(&self, thread_id: &str, key: &str, cx: &App) -> Reply<(String, Option<String>)>;
    fn save(&self, path: &str, content: &str, cx: &App) -> Reply<()>;
}

/// The app's [`PlanPort`]; without one the tab edits but cannot save.
#[derive(Clone)]
pub struct PlanService(pub Rc<dyn PlanPort>);

impl gpui::Global for PlanService {}

/// The newest plan item of a finished turn: `(item id, text)`, as the original's
/// `latestPlan` skips the running turn.
pub fn latest_plan(turns: &[Turn]) -> Option<(&str, &str)> {
    turns
        .iter()
        .rev()
        .filter(|turn| turn.status != TurnStatus::InProgress)
        .find_map(|turn| {
            turn.items.iter().rev().find_map(|item| match item {
                ThreadItem::Plan { id, text } if !text.trim().is_empty() => {
                    Some((id.as_str(), text.as_str()))
                }
                _ => None,
            })
        })
}

/// The plan's first heading, else `Plan`.
pub fn plan_title(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim())
        .filter(|title| !title.is_empty())
        .unwrap_or("Plan")
        .to_owned()
}

pub struct PlanView {
    /// The plan item's id: the tab shows this plan until a newer one replaces it.
    key: String,
    title: SharedString,
    /// `PLAN.md`, once the server's `codexHome` is known.
    path: Option<String>,
    editor: Entity<Composer>,
    saved: String,
    saving: bool,
    error: Option<&'static str>,
}

impl PlanView {
    pub fn new(
        thread: &Entity<ThreadView>,
        key: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| Composer::settings_field("Plan", text, true, cx));
        editor.update(cx, |editor, cx| editor.set_document_style(true, cx));
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        window.focus(&editor.focus_handle(cx), cx);
        let thread_id = thread.read(cx).id().to_owned();
        if let Some(port) = cx
            .try_global::<PlanService>()
            .map(|service| service.0.clone())
        {
            let load = port.load(&thread_id, key, cx);
            cx.spawn(async move |this, cx| {
                let Ok((path, file)) = load.await else {
                    return;
                };
                this.update(cx, |this, cx| {
                    // A saved edit wins over the item's text, as reopening the original's
                    // `PLAN.md` does.
                    if let Some(file) = file {
                        this.editor
                            .update(cx, |editor, cx| editor.set_text(&file, cx));
                        this.saved = file;
                    }
                    this.path = Some(path);
                    cx.notify();
                })
            })
            .detach();
        }
        Self {
            key: key.to_owned(),
            title: plan_title(text).into(),
            path: None,
            editor,
            saved: text.to_owned(),
            saving: false,
            error: None,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn dirty(&self, cx: &App) -> bool {
        self.editor.read(cx).field_text() != self.saved
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (Some(path), Some(port)) = (
            self.path.clone(),
            cx.try_global::<PlanService>()
                .map(|service| service.0.clone()),
        ) else {
            return;
        };
        if self.saving || !self.dirty(cx) {
            return;
        }
        let content = self.editor.read(cx).field_text().to_owned();
        let write = port.save(&path, &content, cx);
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let written = write.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match written {
                    Ok(_) => this.saved = content,
                    Err(_) => this.error = Some("Failed to save plan"),
                }
                cx.notify();
            })
        })
        .detach();
    }
}

impl Render for PlanView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let dirty = self.dirty(cx);
        let ready = dirty && !self.saving && self.path.is_some();
        let label = if self.saving {
            "Saving…"
        } else if dirty {
            "Save"
        } else {
            "Saved"
        };
        div()
            .id("plan-side-panel")
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().flex_1().min_w_0().child(text(
                        self.title.clone(),
                        14.,
                        20.,
                        600.,
                        t.text,
                    )))
                    .children(
                        self.error
                            .map(|error| text(error, 12., 16., 400., t.text_error)),
                    )
                    .child(
                        div()
                            .id("plan-save")
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(Theme::RADIUS_SM)
                            .bg(t.control_background)
                            .opacity(if ready { 1. } else { 0.5 })
                            .aria_label("Save plan")
                            .when(ready, |d| {
                                d.cursor_pointer()
                                    .hover(|s| s.bg(t.ghost_hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                            })
                            .child(text(label, 13., 18., 500., t.text)),
                    ),
            )
            .child(
                div()
                    .id("plan-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .py(px(12.))
                    .child(self.editor.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turns(value: Value) -> Vec<Turn> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn latest_plan_skips_running_turn_and_blank_plans() {
        let turns = turns(json!([
            {"id": "t1", "status": "completed", "items": [
                {"type": "plan", "id": "p1", "text": "# Old\n- a"},
                {"type": "plan", "id": "p2", "text": "# Ship it\n- b"},
                {"type": "plan", "id": "p3", "text": "  "}
            ]},
            {"id": "t2", "status": "inProgress", "items": [
                {"type": "plan", "id": "p4", "text": "# Draft"}
            ]}
        ]));
        assert_eq!(latest_plan(&turns), Some(("p2", "# Ship it\n- b")));
        assert_eq!(latest_plan(&turns[1..]), None);
    }

    #[test]
    fn title_is_first_heading() {
        assert_eq!(plan_title("intro\n## Fix the bug \n# Later"), "Fix the bug");
        assert_eq!(plan_title("- step"), "Plan");
        assert_eq!(plan_title("#\n"), "Plan");
    }
}
