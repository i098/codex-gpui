use crate::*;
use gpui::{EventEmitter, Focusable, StatefulInteractiveElement as _};
use std::fmt::Write as _;
use theme::ActiveTheme as _;

const RECAP_PROMPT: &str = "Summarize this conversation so far. Include the goal, key decisions, completed work, open questions, and next steps. Do not execute tasks or use tools. Do not replace or compact the conversation context.";

pub(crate) struct Feedback {
    reason: Entity<Composer>,
    path: Entity<Composer>,
    controls: [FocusHandle; 4],
    include_logs: bool,
    pending: bool,
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum FeedbackAction {
    Cancel,
    Save,
    Upload,
}

impl EventEmitter<FeedbackAction> for Feedback {}

impl Focusable for Feedback {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.reason.focus_handle(cx)
    }
}

impl Feedback {
    fn new(cx: &mut Context<Self>) -> Self {
        let crash = crate::crash::take();
        let reason = cx.new(|cx| match &crash {
            Some(report) => Composer::form_field(
                "Describe the problem",
                &format!("The app crashed on the last launch:\n{report}"),
                true,
                cx,
            ),
            None => Composer::form_field("Describe the problem", "", true, cx),
        });
        Self {
            reason,
            path: cx.new(|cx| {
                Composer::form_field("Absolute ZIP path on the app-server host", "", false, cx)
            }),
            controls: std::array::from_fn(|_| cx.focus_handle()),
            include_logs: crash.is_some(),
            pending: false,
            error: None,
        }
    }

    fn act(&mut self, action: FeedbackAction, cx: &mut Context<Self>) {
        if !self.pending || matches!(action, FeedbackAction::Cancel) {
            cx.emit(action);
        }
    }
}

impl Render for Feedback {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let buttons = [
            ("Cancel", FeedbackAction::Cancel),
            ("Save ZIP", FeedbackAction::Save),
            ("Send feedback", FeedbackAction::Upload),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (label, action))| {
            div()
                .id(("feedback-button", i))
                .role(gpui::Role::Button)
                .aria_label(label)
                .track_focus(&self.controls[i + 1])
                .px(px(14.))
                .py(px(8.))
                .rounded(Theme::RADIUS_FULL)
                .focus_visible(|style| style.border_2().border_color(t.focus_ring))
                .when(self.pending && i != 0, |button| button.opacity(0.5))
                .bg(if i == 2 {
                    t.button_primary_solid
                } else {
                    t.button_secondary
                })
                .text_color(if i == 2 { t.text_inverse } else { t.text })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.act(action, cx)))
        });
        div().absolute().inset_0().flex().items_center().justify_center()
            .capture_action(cx.listener(|this, _: &ui::composer::Dismiss, _, cx| {
                this.act(FeedbackAction::Cancel, cx);
                cx.stop_propagation();
            }))
            .bg(gpui::hsla(0., 0., 0., 0.4))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.act(FeedbackAction::Cancel, cx),
                    "tab" => {
                        let focus = [this.reason.focus_handle(cx), this.path.focus_handle(cx),
                            this.controls[0].clone(), this.controls[1].clone(),
                            this.controls[2].clone(), this.controls[3].clone()];
                        let current = focus.iter().position(|f| f.is_focused(window)).unwrap_or(0);
                        let next = if event.keystroke.modifiers.shift { (current + 5) % 6 } else { (current + 1) % 6 };
                        window.focus(&focus[next], cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(div().id("feedback-dialog").role(gpui::Role::Dialog).aria_label("Feedback")
                .w(px(480.)).max_w_full().p(px(24.)).flex().flex_col().gap(px(16.))
                .rounded(px(16.)).bg(t.menu_background).text_color(t.text)
                .child(ui::text("Feedback", 20., 26., 600., t.text))
                .child("Send feedback to OpenAI, or save a ZIP on the connected app-server host.")
                .child(self.reason.clone()).child(self.path.clone())
                .child(div().id("feedback-logs").role(gpui::Role::CheckBox)
                    .aria_label("Include diagnostic logs").track_focus(&self.controls[0])
                    .flex().items_center().gap(px(8.))
                    .focus_visible(|style| style.border_2().border_color(t.focus_ring))
                    .aria_toggled(if self.include_logs { gpui::Toggled::True } else { gpui::Toggled::False })
                    .child(div().size(px(14.)).rounded(px(4.)).border_1().border_color(t.border)
                        .when(self.include_logs, |square| square.bg(t.text)
                            .child(ui::text("✓", 12., 12., 600., t.menu_background))))
                    .child("Include diagnostic logs")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.pending { this.include_logs = !this.include_logs; cx.notify(); }
                    })))
                .child("Logs can contain private information. Logs are not included unless selected.")
                .children(self.error.clone().map(|message| ui::text(message, 13., 18., 400., t.text_error)))
                .when(self.pending, |d| d.child("Sending feedback…"))
                .child(div().flex().justify_end().gap(px(8.)).children(buttons)))
    }
}

impl Root {
    pub(crate) fn open_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_dialog() {
            return;
        }
        if self.bridge.is_acp() {
            self.toast(
                Toast::danger("Feedback requires a Codex app-server connection"),
                cx,
            );
            return;
        }
        let thread = match &self.state.route {
            Route::Thread(id) => Some(id.clone()),
            _ => None,
        };
        let dialog = cx.new(Feedback::new);
        cx.subscribe_in(&dialog, window, move |this, dialog, action, window, cx| {
            if matches!(action, FeedbackAction::Cancel) {
                this.feedback = None;
                window.focus(&this.composer(cx).focus_handle(cx), cx);
                cx.notify();
                return;
            }
            let state = dialog.read(cx);
            let mut params = json!({"classification": "bug", "reason": state.reason.read(cx).field_text(),
                "includeLogs": state.include_logs, "threadId": thread});
            let method = if matches!(action, FeedbackAction::Save) {
                let path = state.path.read(cx).field_text().to_owned();
                if !std::path::Path::new(&path).is_absolute() || !path.ends_with(".zip") {
                    dialog.update(cx, |dialog, cx| { dialog.error = Some("Enter an absolute, unused .zip path on the app-server host".into()); cx.notify(); });
                    return;
                }
                params["savePath"] = path.into();
                "feedback/save"
            } else { "feedback/upload" };
            dialog.update(cx, |dialog, cx| { dialog.pending = true; dialog.error = None; cx.notify(); });
            let dialog = dialog.clone();
            let bridge = this.bridge.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = bridge.request::<_, Value>(method, params).await;
                this.update_in(cx, |this, window, cx| {
                    if this.feedback.as_ref() != Some(&dialog) { return; }
                    match result {
                    Ok(result) => {
                        this.feedback = None;
                        let message = if method == "feedback/save" {
                            format!("Feedback saved on app-server host: {}", result["savedPath"].as_str().unwrap_or_default())
                        } else { "Feedback sent".into() };
                        this.toast(Toast::success(message), cx);
                        window.focus(&this.composer(cx).focus_handle(cx), cx);
                    }
                    Err(error) => dialog.update(cx, |dialog, cx| {
                        dialog.pending = false;
                        dialog.error = Some(error.to_string());
                        cx.notify();
                    }),
                    }
                }).ok();
            }).detach();
        }).detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.feedback = Some(dialog);
        cx.notify();
    }

    pub(crate) fn recap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self
            .thread
            .as_ref()
            .filter(|_| matches!(self.state.route, Route::Thread(_)))
        else {
            self.toast(Toast::danger("Open a chat before requesting a recap"), cx);
            return;
        };
        let view = view.read(cx);
        if view
            .turns()
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress)
        {
            self.toast(
                Toast::danger("Wait for the current turn before requesting a recap"),
                cx,
            );
            return;
        }
        let id = view.id().to_owned();
        let resume = !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = async {
                resume_if(resume, &bridge, &this, &id, cx).await?;
                bridge
                    .request::<_, Value>(
                        "turn/start",
                        json!({"threadId": id,
                    "input": [{"type": "text", "text": RECAP_PROMPT, "text_elements": []}]}),
                    )
                    .await
            }
            .await;
            if result.is_err() {
                this.update(cx, |this, cx| {
                    this.toast(Toast::danger("Failed to recap chat"), cx)
                })?;
            }
            result.map(drop)
        });
        log_err(task, cx);
    }

    pub(crate) fn export_transcript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Route::Thread(id) = self.state.route.clone() else {
            self.toast(
                Toast::danger("Open a chat before exporting a transcript"),
                cx,
            );
            return;
        };
        let path = cx.prompt_for_new_path(&std::env::temp_dir(), Some("conversation.md"));
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = async {
                let Some(path) = path.await?? else {
                    return Ok(false);
                };
                let response: Value = bridge
                    .request("thread/read", json!({"threadId": id, "includeTurns": true}))
                    .await?;
                cx.background_spawn(async move {
                    use std::io::Write as _;
                    let markdown = transcript_markdown(&response["thread"])?;
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)?;
                    file.write_all(markdown.as_bytes())?;
                    Ok::<_, anyhow::Error>(())
                })
                .await?;
                Ok::<_, anyhow::Error>(true)
            }
            .await;
            this.update(cx, |this, cx| match &result {
                Ok(true) => this.toast(Toast::success("Transcript exported"), cx),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
                {
                    this.toast(
                        Toast::danger("Failed to export transcript; choose an unused file path"),
                        cx,
                    )
                }
                Err(error) => this.toast(
                    Toast::danger(format!("Failed to export transcript: {error}")),
                    cx,
                ),
                Ok(false) => {}
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }
}

fn transcript_markdown(thread: &Value) -> Result<String> {
    let turns = thread["turns"]
        .as_array()
        .context("thread/read omitted turns")?;
    let title = thread["name"]
        .as_str()
        .unwrap_or("Conversation")
        .replace(['\n', '\r'], " ");
    let mut output = format!("# {title}\n\n");
    for turn in turns {
        for item in turn["items"].as_array().context("turn omitted items")? {
            match item["type"].as_str() {
                Some("userMessage") => {
                    output.push_str("## User\n\n");
                    for part in item["content"]
                        .as_array()
                        .context("user message omitted content")?
                    {
                        if part["type"] == "text" {
                            writeln!(
                                output,
                                "{}\n",
                                part["text"].as_str().context("text omitted content")?
                            )?;
                        } else {
                            json_block(&mut output, part)?;
                        }
                    }
                }
                Some("agentMessage") => {
                    writeln!(
                        output,
                        "## Assistant\n\n{}\n",
                        item["text"]
                            .as_str()
                            .context("agent message omitted text")?
                    )?;
                }
                _ => {
                    output.push_str("## Activity\n\n");
                    json_block(&mut output, item)?;
                }
            }
        }
        if !turn["error"].is_null() {
            json_block(&mut output, &turn["error"])?;
        }
    }
    Ok(output)
}

fn json_block(output: &mut String, value: &Value) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    let fence = "`".repeat(
        json.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    writeln!(output, "{fence}json\n{json}\n{fence}\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_preserves_messages_attachments_tools_and_errors() {
        let thread = json!({"name": "A\nTitle", "turns": [{"items": [
            {"type": "userMessage", "content": [{"type":"text", "text":"Keep **Markdown**"}, {"type":"localImage", "path":"/tmp/photo.png"}]},
            {"type":"agentMessage", "text":"Answer\n```rust\n42\n```"},
            {"type":"futureTool", "output":"```` do not close this fence"}
        ], "error": {"message":"interrupted"}}]});
        let text = transcript_markdown(&thread).unwrap();
        assert!(text.starts_with("# A Title\n\n## User\n\nKeep **Markdown**\n"));
        assert!(text.contains("/tmp/photo.png"));
        assert!(text.contains("## Assistant\n\nAnswer\n```rust\n42\n```"));
        assert!(text.contains("`````json\n"));
        assert!(text.contains("futureTool"));
        assert!(text.contains("interrupted"));
        assert!(transcript_markdown(&json!({"id":"missing-turns"})).is_err());
    }
}
