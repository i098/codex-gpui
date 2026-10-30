//! Realtime text voice session: `thread/realtime/start|appendText|stop` and the transcript the
//! server streams back in `thread/realtime/*` notifications.

use crate::*;
use ui::screens::thread::view::{VoiceCard, VoiceLine};

#[derive(Clone, PartialEq, Debug)]
enum Status {
    Starting,
    Live,
    Failed(String),
    Closed(Option<String>),
}

struct Line {
    role: String,
    text: String,
    done: bool,
}

pub(crate) struct VoiceChat {
    thread_id: String,
    status: Status,
    lines: Vec<Line>,
}

impl VoiceChat {
    fn new(thread_id: &str) -> Self {
        Self {
            thread_id: thread_id.to_owned(),
            status: Status::Starting,
            lines: Vec::new(),
        }
    }

    fn is_live(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Live)
    }

    /// Applies one `thread/realtime/*` notification of this session's thread.
    fn apply(&mut self, method: &str, params: &Value) {
        let role = params["role"].as_str().unwrap_or("assistant");
        match method {
            "thread/realtime/started" => self.status = Status::Live,
            "thread/realtime/transcript/delta" => {
                let delta = params["delta"].as_str().unwrap_or_default();
                match self.lines.last_mut() {
                    Some(line) if !line.done && line.role == role => line.text.push_str(delta),
                    _ => self.lines.push(Line {
                        role: role.to_owned(),
                        text: delta.to_owned(),
                        done: false,
                    }),
                }
            }
            "thread/realtime/transcript/done" => {
                let text = params["text"].as_str().unwrap_or_default();
                match self.lines.last_mut() {
                    Some(line) if !line.done && line.role == role => {
                        line.text = text.to_owned();
                        line.done = true;
                    }
                    _ => self.lines.push(Line {
                        role: role.to_owned(),
                        text: text.to_owned(),
                        done: true,
                    }),
                }
            }
            "thread/realtime/error" => {
                let message = params["message"].as_str().unwrap_or("Voice chat failed");
                self.status = Status::Failed(message.to_owned());
            }
            "thread/realtime/closed" => {
                // An error keeps its message; the close that follows it adds nothing.
                if !matches!(self.status, Status::Failed(_)) {
                    self.status = Status::Closed(params["reason"].as_str().map(str::to_owned));
                }
            }
            _ => {}
        }
    }

    fn card(&self) -> VoiceCard {
        let status = match &self.status {
            Status::Starting => "Starting voice chat…".to_owned(),
            Status::Live => "Voice chat".to_owned(),
            Status::Failed(message) => format!("Voice chat failed: {message}"),
            Status::Closed(Some(reason)) => format!("Voice chat ended ({reason})"),
            Status::Closed(None) => "Voice chat ended".to_owned(),
        };
        VoiceCard {
            status: status.into(),
            live: self.is_live(),
            lines: self
                .lines
                .iter()
                .map(|line| VoiceLine {
                    user: line.role == "user",
                    text: line.text.clone().into(),
                })
                .collect(),
        }
    }
}

impl Root {
    /// Pushes the session state into the open thread view when it shows the session's thread.
    pub(crate) fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.thread.clone() else {
            return;
        };
        let card = self
            .voice_chat
            .as_ref()
            .filter(|voice| voice.thread_id == view.read(cx).id())
            .map(VoiceChat::card);
        view.update(cx, |view, cx| view.set_voice(card, cx));
    }

    pub(crate) fn start_voice_chat(&mut self, cx: &mut Context<Self>) {
        let Route::Thread(id) = self.state.route.clone() else {
            self.toast(Toast::info("Open a conversation to start a voice chat"), cx);
            return;
        };
        if self.voice_chat.as_ref().is_some_and(VoiceChat::is_live) {
            return;
        }
        self.voice_chat = Some(VoiceChat::new(&id));
        self.sync_voice(cx);
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let params = json!({
                "threadId": id,
                "outputModality": "text",
                "flushTranscriptTailOnSessionEnd": true,
            });
            if let Err(error) = bridge
                .request::<_, Value>("thread/realtime/start", params)
                .await
            {
                this.update(cx, |this, cx| {
                    if let Some(voice) = this.voice_chat.as_mut().filter(|v| v.thread_id == id) {
                        voice.status = Status::Failed(format!("{error:#}"));
                    }
                    this.sync_voice(cx);
                })?;
            }
            Ok(())
        });
        log_err(task, cx);
    }

    pub(crate) fn end_voice_chat(&mut self, cx: &mut Context<Self>) {
        let Some(voice) = self.voice_chat.take() else {
            return;
        };
        self.sync_voice(cx);
        if !voice.is_live() {
            return;
        }
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |_, _| -> Result<()> {
            let params = json!({ "threadId": voice.thread_id });
            bridge
                .request::<_, Value>("thread/realtime/stop", params)
                .await?;
            Ok(())
        });
        log_err(task, cx);
    }

    /// While a session is live, typed text goes to it instead of starting a turn.
    pub(crate) fn voice_text(
        &mut self,
        thread_id: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self
            .voice_chat
            .as_ref()
            .is_some_and(|voice| voice.is_live() && voice.thread_id == thread_id)
        {
            return false;
        }
        let bridge = self.bridge.clone();
        let params = json!({ "threadId": thread_id, "role": "user", "text": text });
        let task = cx.spawn(async move |_, _| -> Result<()> {
            bridge
                .request::<_, Value>("thread/realtime/appendText", params)
                .await?;
            Ok(())
        });
        log_err(task, cx);
        true
    }

    pub(crate) fn voice_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        if !method.starts_with("thread/realtime/") {
            return;
        }
        if let Some(voice) = self
            .voice_chat
            .as_mut()
            .filter(|voice| params["threadId"] == voice.thread_id.as_str())
        {
            voice.apply(method, params);
            self.sync_voice(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_grow_one_line_per_speaker_turn_and_done_replaces_it() {
        let mut voice = VoiceChat::new("t");
        voice.apply("thread/realtime/started", &json!({}));
        voice.apply(
            "thread/realtime/transcript/delta",
            &json!({"role": "user", "delta": "he"}),
        );
        voice.apply(
            "thread/realtime/transcript/delta",
            &json!({"role": "user", "delta": "llo"}),
        );
        voice.apply(
            "thread/realtime/transcript/delta",
            &json!({"role": "assistant", "delta": "hi"}),
        );
        voice.apply(
            "thread/realtime/transcript/done",
            &json!({"role": "assistant", "text": "hi there"}),
        );
        voice.apply(
            "thread/realtime/transcript/done",
            &json!({"role": "assistant", "text": "again"}),
        );
        let lines: Vec<_> = voice
            .lines
            .iter()
            .map(|l| (l.role.as_str(), l.text.as_str(), l.done))
            .collect();
        assert_eq!(
            lines,
            [
                ("user", "hello", false),
                ("assistant", "hi there", true),
                ("assistant", "again", true)
            ]
        );
    }

    #[test]
    fn an_error_keeps_its_message_through_the_close() {
        let mut voice = VoiceChat::new("t");
        voice.apply("thread/realtime/error", &json!({"message": "boom"}));
        voice.apply("thread/realtime/closed", &json!({"reason": "error"}));
        assert_eq!(voice.status, Status::Failed("boom".into()));
        assert!(!voice.is_live());
        let mut voice = VoiceChat::new("t");
        voice.apply("thread/realtime/closed", &json!({"reason": "requested"}));
        assert_eq!(voice.status, Status::Closed(Some("requested".into())));
    }
}
