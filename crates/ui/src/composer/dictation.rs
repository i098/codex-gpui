use super::*;
use std::sync::{Arc, mpsc};

pub enum Command {
    Finish,
    Cancel,
}

pub enum Event {
    Recording,
    Transcribing,
    Finished(Result<String, String>),
}

pub struct Session {
    pub commands: mpsc::Sender<Command>,
    pub events: async_channel::Receiver<Event>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Cancel);
    }
}

pub struct Dispatcher {
    start: Arc<dyn Fn() -> Result<Session, String> + Send + Sync>,
    active: Option<(gpui::WindowId, gpui::WeakEntity<Composer>)>,
}

impl gpui::Global for Dispatcher {}

impl Dispatcher {
    pub fn new(start: impl Fn() -> Result<Session, String> + Send + Sync + 'static) -> Self {
        Self {
            start: Arc::new(start),
            active: None,
        }
    }

    pub fn install(self, cx: &mut App) {
        let target = cx
            .try_global::<Self>()
            .and_then(|dispatcher| dispatcher.active.clone());
        if let Some((_, target)) = target {
            let _ = target.update(cx, |composer, cx| composer.cancel_dictation(cx));
        }
        cx.set_global(self);
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
    starting: bool,
    session: Session,
    _task: Task<()>,
    transcribing: bool,
    selection: Range<usize>,
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
        if let Some(active) = &mut self.dictation {
            if !active.transcribing {
                active.transcribing = true;
                let _ = active.session.commands.send(Command::Finish);
                cx.notify();
            }
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
        let session = match start() {
            Ok(session) => session,
            Err(error) => {
                self.dictation_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.dictation_error = None;
        self.marked_range = None;
        self.focus_handle.focus(window, cx);
        let target = cx.weak_entity();
        cx.global_mut::<Dispatcher>().active = Some((window.window_handle().window_id(), target));
        let events = session.events.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let finished = matches!(event, Event::Finished(_));
                if this
                    .update_in(cx, |this, window, cx| {
                        this.dictation_event(event, window, cx)
                    })
                    .is_err()
                    || finished
                {
                    break;
                }
            }
        });
        self.dictation = Some(Active {
            starting: true,
            session,
            _task: task,
            transcribing: false,
            selection: self.selected_range.clone(),
        });
        cx.notify();
    }

    pub fn cancel_dictation(&mut self, cx: &mut Context<Self>) {
        self.dictation = None;
        for field in &self.request_draft.fields {
            field.update(cx, |field, cx| field.cancel_dictation(cx));
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
            Event::Transcribing => {
                if let Some(active) = &mut self.dictation {
                    active.transcribing = true;
                }
            }
            Event::Finished(result) => {
                let Some(active) = self.dictation.take() else {
                    return;
                };
                match result {
                    Ok(text) if !text.trim().is_empty() => {
                        self.selected_range = active.selection;
                        self.history.last = None;
                        self.replace_text_in_range(None, text.trim(), window, cx);
                        self.history.last = None;
                        self.focus_handle.focus(window, cx);
                    }
                    Ok(_) => {}
                    Err(error) => self.dictation_error = Some(error),
                }
            }
        }
        cx.notify();
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
    pub(super) fn dictation_button(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = self.dictation_label();
        pill("dictate", t)
            .justify_center()
            .child(if self.dictation.is_some() {
                div()
                    .size(px(8.8))
                    .rounded(px(1.))
                    .bg(t.text)
                    .into_any_element()
            } else {
                icon("mic", 16., t.text)
                    .when(self.docked, |i| i.own_layer(None))
                    .into_any_element()
            })
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(label == "Transcribing")
            .tab_index(0)
            .focus_visible(menu::trigger_focus)
            .size(px(28.))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.dictate(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.dictate(window, cx)))
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
        cx.set_global(Dispatcher::new(move || {
            Ok(Session {
                commands: commands.clone(),
                events: events.clone(),
            })
        }));
        receiver
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
}
