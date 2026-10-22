//! Own one dictation session per Composer and keep its original selection.
//! The app supplies the capture and transcription service through `Dispatcher`.
//! Run `cargo test -p ui --lib composer::dictation` without microphone access.

use super::*;
use std::sync::{Arc, mpsc};

pub enum Command {
    Finish,
    Retry,
    Cancel,
}

pub enum Event {
    Recording,
    Level(f32),
    Transcribing,
    Captured(Option<String>),
    Finished(Result<String, String>),
}

/// Reports an active session's result after transcript processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Succeeded,
    Failed,
    Cancelled,
}

impl gpui::EventEmitter<Completion> for Composer {}

pub struct Session {
    pub commands: mpsc::Sender<Command>,
    pub events: async_channel::Receiver<Event>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Cancel);
    }
}

type StartSession = dyn Fn(&Window, &App) -> Result<Session, String> + Send + Sync;

pub struct Dispatcher {
    start: Arc<StartSession>,
    active: Option<(gpui::WindowId, gpui::WeakEntity<Composer>)>,
}

impl gpui::Global for Dispatcher {}

impl Dispatcher {
    pub fn new(
        start: impl Fn(&Window, &App) -> Result<Session, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            start: Arc::new(start),
            active: None,
        }
    }

    pub fn cancel(window: gpui::WindowId, cx: &mut App) {
        let target = cx.try_global::<Self>().and_then(|dispatcher| {
            dispatcher
                .active
                .as_ref()
                .filter(|(id, _)| *id == window)
                .map(|(_, target)| target.clone())
        });
        if let Some(target) = target {
            let _ = target.update(cx, |composer, cx| composer.cancel_dictation(cx));
        }
    }
}

pub(super) struct Active {
    pub(super) starting: bool,
    session: Session,
    _task: Task<()>,
    pub(super) transcribing: bool,
    selection: Range<usize>,
    pub(super) submit_when_finished: bool,
    pub(super) levels: std::collections::VecDeque<f32>,
    recorded: bool,
    pub(super) recording_url: Option<String>,
}

pub(super) struct Recovery {
    pub(super) active: Active,
    pub(super) text: Option<String>,
    pub(super) retrying: bool,
}

impl Composer {
    pub fn dictate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .dictation
            .as_ref()
            .is_some_and(|active| active.starting)
        {
            self.cancel_dictation(cx);
            return;
        }
        if self.dictation.is_some() {
            self.finish_dictation(window, cx);
            return;
        }
        if self.secret || self.edit_pending || !self.dictation_target_current(cx) {
            return;
        }
        let Some(dispatcher) = cx.try_global::<Dispatcher>() else {
            self.dictation_error = Some("Dictation is not supported on this platform".into());
            cx.notify();
            return;
        };
        if dispatcher.active.as_ref().is_some_and(|(_, target)| {
            target.entity_id() != cx.entity_id()
                && target
                    .upgrade()
                    .is_some_and(|target| target.read(cx).dictation.is_some())
        }) {
            self.dictation_error = Some("The microphone is already in use".into());
            cx.notify();
            return;
        }
        let start = dispatcher.start.clone();
        let session = match start(window, cx) {
            Ok(session) => session,
            Err(error) => {
                self.dictation_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.dictation_error = None;
        self.dictation_recovery = None;
        self.marked_range = None;
        self.focus_handle.focus(window, cx);
        let target = cx.weak_entity();
        cx.global_mut::<Dispatcher>().active = Some((window.window_handle().window_id(), target));
        let events = session.events.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let finished = matches!(event, Event::Finished(Ok(_)));
                if this
                    .update_in(cx, |this, window, cx| {
                        this.dictation_event(event, window, cx)
                    })
                    .is_err()
                    || finished
                {
                    return;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if this.dictation.is_some() {
                    this.dictation_event(
                        Event::Finished(Err("Unable to transcribe audio".into())),
                        window,
                        cx,
                    );
                }
            });
        });
        self.dictation = Some(Active {
            starting: true,
            session,
            _task: task,
            transcribing: false,
            selection: self.selected_range.clone(),
            submit_when_finished: false,
            levels: std::collections::VecDeque::new(),
            recorded: false,
            recording_url: None,
        });
        cx.notify();
    }

    /// Requests transcription while retaining the active session and its selection.
    pub fn finish_dictation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = &mut self.dictation else {
            return;
        };
        if active.transcribing {
            return;
        }
        active.transcribing = true;
        if active.session.commands.send(Command::Finish).is_err() {
            self.dictation_event(
                Event::Finished(Err("Unable to transcribe audio".into())),
                window,
                cx,
            );
        } else {
            cx.notify();
        }
    }

    pub(super) fn transcribe_and_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = &mut self.dictation {
            active.submit_when_finished = true;
            self.finish_dictation(window, cx);
        }
    }

    pub fn has_active_dictation(&self) -> bool {
        self.dictation.is_some()
    }

    pub fn cancel_dictation(&mut self, cx: &mut Context<Self>) {
        let cancelled = self.dictation.take().is_some();
        self.dictation_recovery = None;
        for field in &self.request_draft.fields {
            field.update(cx, |field, cx| field.cancel_dictation(cx));
        }
        if cancelled {
            cx.emit(Completion::Cancelled);
        }
        cx.notify();
    }

    fn dictation_target_current(&self, cx: &App) -> bool {
        self.request_parent
            .as_ref()
            .is_none_or(|(parent, identity, index)| {
                parent.upgrade().is_some_and(|parent| {
                    let parent = parent.read(cx);
                    parent
                        .approval
                        .as_ref()
                        .is_some_and(|approval| approval.identity == *identity)
                        && parent.request_draft.index == *index
                })
            })
    }

    fn dictation_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dictation_target_current(cx) {
            self.cancel_dictation(cx);
            return;
        }
        match event {
            Event::Recording => {
                if let Some(active) = &mut self.dictation {
                    active.starting = false;
                }
            }
            Event::Level(rms) => {
                if let Some(active) = &mut self.dictation {
                    if active.levels.len() == 512 {
                        active.levels.pop_front();
                    }
                    active
                        .levels
                        .push_back(((rms - 0.006).max(0.) / 0.154).min(1.).powf(0.6));
                }
            }
            Event::Transcribing => {
                if let Some(recovery) = &mut self.dictation_recovery {
                    recovery.retrying = true;
                }
                if let Some(active) = &mut self.dictation {
                    active.transcribing = true;
                }
            }
            Event::Captured(path) => {
                if let Some(active) = &mut self.dictation {
                    active.recorded = true;
                    active.recording_url = path;
                }
            }
            Event::Finished(result) => self.finish_dictation_event(result, window, cx),
        }
        cx.notify();
    }

    fn finish_dictation_event(
        &mut self,
        result: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(recovery) = &mut self.dictation_recovery {
            recovery.retrying = false;
            match result {
                Ok(text) => recovery.text = Some(text),
                Err(error) => self.dictation_error = Some(error),
            }
            return;
        }
        let Some(active) = self.dictation.take() else {
            return;
        };
        let submit = active.submit_when_finished;
        let completion = match result {
            Ok(text) => {
                if !text.trim().is_empty() {
                    self.selected_range = active.selection;
                    self.history.last = None;
                    self.replace_text_in_range(None, text.trim(), window, cx);
                    self.history.last = None;
                    self.focus_handle.focus(window, cx);
                }
                Completion::Succeeded
            }
            Err(error) => {
                self.dictation_error = Some(error);
                if active.recorded {
                    self.dictation_recovery = Some(Recovery {
                        active,
                        text: None,
                        retrying: false,
                    });
                }
                Completion::Failed
            }
        };
        cx.emit(completion);
        if completion == Completion::Succeeded && submit {
            self.submit(&Submit, window, cx);
        }
    }

    pub fn dictation_label(&self) -> &'static str {
        match &self.dictation {
            Some(active) if active.transcribing => "Transcribing",
            Some(active) if active.starting => "Starting dictation",
            Some(_) => "Stop dictation",
            None => "Dictate",
        }
    }

    pub fn dictation_error(&self) -> Option<&str> {
        self.dictation_error.as_deref()
    }

    pub(super) fn retry_dictation(&mut self, cx: &mut Context<Self>) {
        if let Some(recovery) = &mut self.dictation_recovery {
            if recovery.retrying {
                return;
            }
            if recovery
                .active
                .session
                .commands
                .send(Command::Retry)
                .is_ok()
            {
                recovery.retrying = true;
            } else {
                self.dictation_error = Some("Unable to transcribe audio".into());
            }
            cx.notify();
        }
    }

    pub(super) fn view_recording(&self, cx: &mut App) {
        if let Some(url) = self
            .dictation_recovery
            .as_ref()
            .and_then(|recovery| recovery.active.recording_url.as_ref())
        {
            cx.open_url(url);
        }
    }

    pub(super) fn insert_recovered_dictation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = self
            .dictation_recovery
            .as_ref()
            .and_then(|recovery| recovery.text.clone())
        {
            self.selected_range = self.content.len()..self.content.len();
            let separator =
                if self.content.is_empty() || self.content.ends_with(char::is_whitespace) {
                    ""
                } else {
                    " "
                };
            self.insert(&format!("{separator}{text}"), window, cx);
            self.dictation_recovery = None;
            self.dictation_error = None;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    fn install(cx: &mut App) -> mpsc::Receiver<Command> {
        cx.set_global(Theme::dark());
        let (commands, receiver) = mpsc::channel();
        let (_events, events) = async_channel::unbounded();
        cx.set_global(Dispatcher::new(move |_, _| {
            Ok(Session {
                commands: commands.clone(),
                events: events.clone(),
            })
        }));
        receiver
    }

    #[test]
    fn recovery_retries_retained_audio_and_inserts_only_on_request() {
        let mut cx = gpui::TestAppContext::single();
        let commands = cx.update(install);
        let window = cx.add_window(Composer::home);
        window
            .update(&mut cx, |editor, window, cx| {
                editor.set_text("keep", cx);
                editor.dictate(window, cx);
                editor.dictation_event(
                    Event::Captured(Some("file:///recording.wav".into())),
                    window,
                    cx,
                );
                editor.dictation_event(Event::Finished(Err("offline".into())), window, cx);
                assert!(!editor.has_active_dictation());
                assert_eq!(editor.field_text(), "keep");
                editor.retry_dictation(cx);
                assert!(matches!(commands.try_recv(), Ok(Command::Retry)));
                editor.dictation_event(Event::Finished(Ok("recovered".into())), window, cx);
                assert_eq!(editor.field_text(), "keep");
                assert_eq!(
                    editor.dictation_recovery.as_ref().unwrap().text.as_deref(),
                    Some("recovered")
                );
                editor.insert_recovered_dictation(window, cx);
                assert_eq!(editor.field_text(), "keep recovered");
                assert!(editor.dictation_recovery.is_none());
            })
            .unwrap();
    }

    #[test]
    fn a_new_dictation_replaces_failed_recovery_and_inserts_its_transcript() {
        let mut cx = gpui::TestAppContext::single();
        cx.update(install);
        let window = cx.add_window(Composer::home);
        window
            .update(&mut cx, |editor, window, cx| {
                editor.set_text("keep ", cx);
                editor.dictate(window, cx);
                editor.dictation_event(Event::Captured(None), window, cx);
                editor.dictation_event(Event::Finished(Err("offline".into())), window, cx);
                assert!(editor.dictation_recovery.is_some());
                editor.dictate(window, cx);
                assert!(editor.dictation_recovery.is_none());
                editor.dictation_event(Event::Finished(Ok("new transcript".into())), window, cx);
                assert_eq!(editor.field_text(), "keep new transcript");
                assert!(!editor.has_active_dictation());
            })
            .unwrap();
    }

    #[test]
    fn closing_a_composer_cancels_its_capture_without_retaining_the_editor() {
        let mut cx = gpui::TestAppContext::single();
        let receiver = cx.update(install);
        let window = cx.add_window(Composer::home);
        let weak = cx
            .update_window(window.into(), |_, window, cx| {
                let editor = cx.new(|cx| Composer::home(window, cx));
                let weak = editor.downgrade();
                editor.update(cx, |editor, cx| editor.dictate(window, cx));
                drop(editor);
                weak
            })
            .unwrap();
        cx.run_until_parked();
        assert!(weak.upgrade().is_none());
        assert!(matches!(receiver.try_recv(), Ok(Command::Cancel)));
    }

    #[test]
    fn replacing_a_request_cancels_its_field_and_rejects_late_text() {
        let mut cx = gpui::TestAppContext::single();
        cx.update(install);
        let window = cx.add_window(Composer::home);
        cx.update_window(window.into(), |view, window, cx| {
            let parent = view.downcast::<Composer>().unwrap();
            let params = json!({"threadId":"thread", "turnId":"turn", "itemId":"item",
                "questions":[{"id":"answer","question":"Answer","isOther":true,"options":[]}]});
            let editor = parent.update(cx, |parent, cx| {
                parent.set_approval(
                    Approval::form(json!(1), "item/tool/requestUserInput", &params),
                    cx,
                );
                parent.request_draft.fields[0].clone()
            });
            editor.update(cx, |editor, cx| {
                editor.set_text("kept", cx);
                editor.dictate(window, cx);
            });
            parent.update(cx, |parent, cx| {
                parent.set_approval(
                    Approval::form(json!(2), "item/tool/requestUserInput", &params),
                    cx,
                )
            });
            editor.update(cx, |editor, cx| {
                editor.dictation_event(Event::Finished(Ok("late".into())), window, cx);
                assert_eq!(editor.field_text(), "kept");
                assert!(editor.dictation.is_none());
            });
            parent.read_with(cx, |parent, cx| {
                assert_eq!(parent.request_draft.fields[0].read(cx).field_text(), "")
            });
        })
        .unwrap();
    }
    #[test]
    fn stopping_during_startup_keeps_the_draft_without_an_error() {
        let mut cx = gpui::TestAppContext::single();
        let receiver = cx.update(install);
        let window = cx.add_window(Composer::home);
        window
            .update(&mut cx, |composer, window, cx| {
                composer.set_text("keep", cx);
                composer.dictate(window, cx);
                composer.dictate(window, cx);
                composer.dictation_event(Event::Finished(Ok("late".into())), window, cx);
                assert_eq!(composer.field_text(), "keep");
                assert_eq!(composer.dictation_label(), "Dictate");
                assert_eq!(composer.dictation_error(), None);
            })
            .unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Command::Cancel)));
    }

    #[test]
    fn changing_a_draft_cancels_transcription_and_clears_its_error() {
        let mut cx = gpui::TestAppContext::single();
        cx.update(install);
        let window = cx.add_window(Composer::home);
        window
            .update(&mut cx, |composer, window, cx| {
                composer.set_text("replace this longer draft", cx);
                composer.dictate(window, cx);
                composer.dictation_error = Some("old error".into());
                composer.splice(0..composer.content.len(), "new");
                composer.dictation_event(Event::Finished(Ok("late".into())), window, cx);
                assert_eq!(composer.field_text(), "new");
                assert_eq!(composer.dictation_error(), None);
            })
            .unwrap();
    }

    #[test]
    fn a_closed_event_stream_clears_the_session_and_preserves_the_draft() {
        let mut cx = gpui::TestAppContext::single();
        let receiver = cx.update(install);
        let window = cx.add_window(Composer::home);
        window
            .update(&mut cx, |composer, window, cx| {
                composer.set_text("keep", cx);
                composer.dictate(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |composer, _, _| {
                assert!(!composer.has_active_dictation());
                assert_eq!(composer.field_text(), "keep");
                assert_eq!(
                    composer.dictation_error(),
                    Some("Unable to transcribe audio")
                );
            })
            .unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Command::Cancel)));
    }

    #[test]
    fn completion_observers_see_inserted_text_and_terminal_outcomes() {
        use std::{cell::RefCell, rc::Rc};
        let mut cx = gpui::TestAppContext::single();
        let commands = cx.update(install);
        let window = cx.add_window(Composer::home);
        let editor = window.update(&mut cx, |_, _, cx| cx.entity()).unwrap();
        let results = Rc::new(RefCell::new(Vec::new()));
        let observed = results.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&editor, move |editor, completion: &Completion, cx| {
                observed
                    .borrow_mut()
                    .push((*completion, editor.read(cx).field_text().to_owned()));
            })
        });
        window
            .update(&mut cx, |editor, window, cx| {
                editor.set_text("before ", cx);
                editor.dictate(window, cx);
                editor.finish_dictation(window, cx);
                editor.dictation_event(Event::Finished(Ok("after".into())), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(matches!(commands.try_recv(), Ok(Command::Finish)));
        window
            .update(&mut cx, |editor, window, cx| {
                editor.dictate(window, cx);
                editor.insert(" edited", window, cx);
                editor.dictation_event(Event::Finished(Ok("late".into())), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |editor, window, cx| {
                editor.dictate(window, cx);
                editor.dictation_event(Event::Finished(Err("failure".into())), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            *results.borrow(),
            vec![
                (Completion::Succeeded, "before after".to_owned()),
                (Completion::Cancelled, "before after edited".to_owned()),
                (Completion::Failed, "before after edited".to_owned()),
            ]
        );
    }
}
