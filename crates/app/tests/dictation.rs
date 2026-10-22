#![cfg(feature = "test-support")]

mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{CloudHost, Credentials, Host, LocalHost};
use common::Fx;
use gpui::{AppContext as _, EntityInputHandler as _, Focusable as _, px, size};
use std::{
    cell::RefCell,
    io::{BufRead as _, BufReader, Read as _, Write as _},
    net::TcpListener,
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};
use ui::composer::dictation::{Completion, Dispatcher};

fn audio() -> Vec<u8> {
    let samples = 16000_u32;
    let mut wav = Vec::with_capacity(44 + samples as usize * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&32000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    for sample in 0..samples {
        let value = ((sample as f64 * 440. * std::f64::consts::TAU / 16000.).sin() * 8000.) as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}

struct Backend {
    host: Arc<dyn Host>,
    received: mpsc::Receiver<Vec<u8>>,
    reply: mpsc::Sender<(u16, &'static str)>,
    worker: std::thread::JoinHandle<Result<()>>,
    _home: tempfile::TempDir,
}

impl Backend {
    fn new(requests: usize) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}/backend-api", listener.local_addr()?);
        let (send_request, received) = mpsc::channel();
        let (reply, replies) = mpsc::channel::<(u16, &'static str)>();
        let worker = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept()?;
                stream.set_read_timeout(Some(Duration::from_secs(20)))?;
                let mut reader = BufReader::new(stream.try_clone()?);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                ensure!(
                    line.starts_with("POST /backend-api/transcribe HTTP/1.1"),
                    "wrong transcription endpoint"
                );
                let mut length = None;
                let mut authenticated = false;
                let mut multipart = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line)?;
                    if line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = Some(value.trim().parse::<usize>()?);
                    }
                    authenticated |= lower.starts_with("authorization: bearer fixture");
                    multipart |= lower.starts_with(
                        "content-type: multipart/form-data; boundary=----codex-transcribe-",
                    );
                }
                ensure!(
                    authenticated && multipart,
                    "missing authenticated multipart transport"
                );
                let mut body = vec![0; length.context("content length")?];
                reader.read_exact(&mut body)?;
                let start = body
                    .windows(4)
                    .position(|bytes| bytes == b"\r\n\r\n")
                    .context("file part")?
                    + 4;
                let expected = audio();
                ensure!(
                    body.get(start..start + expected.len()) == Some(&expected),
                    "audio payload changed"
                );
                send_request.send(body)?;
                let (status, body) = replies.recv_timeout(Duration::from_secs(20))?;
                write!(
                    stream,
                    "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
            }
            Ok(())
        });
        let home = tempfile::tempdir()?;
        let host = Arc::new(CloudHost::new(
            LocalHost::new(home.path()),
            Box::new(|_| {
                Ok(Some(Credentials {
                    token: "fixture".into(),
                    backend_origin: None,
                    routing_override: None,
                }))
            }),
            base,
        ));
        Ok(Self {
            host,
            received,
            reply,
            worker,
            _home: home,
        })
    }

    fn install(&self, fx: &mut Fx, timeout: Duration) {
        fx.cx.update(|cx| {
            fx.root
                .update(cx, |root, cx| root.set_host(self.host.clone(), cx))
        });
        let dispatcher = app::dictation::audio_dispatcher(audio(), timeout);
        fx.cx.update(|cx| cx.set_global(dispatcher));
    }

    fn respond(&self, status: u16, body: &'static str) -> Result<()> {
        self.received.recv_timeout(Duration::from_secs(20))?;
        self.reply.send((status, body))?;
        Ok(())
    }
}

#[test]
fn controlled_audio_reaches_only_the_live_editor() -> Result<()> {
    let backend = Backend::new(3)?;
    let mut fx = Fx::new()?;
    backend.install(&mut fx, Duration::from_secs(60));
    let at = fx.at("home", "Dictate")?;
    fx.focus_composer()?;
    fx.type_text("α old ω")?;
    let composer = fx.composer();
    let completions = Rc::new(RefCell::new(Vec::new()));
    let observed = completions.clone();
    let _subscription = fx.cx.update(|cx| {
        cx.subscribe(&composer, move |editor, completion: &Completion, cx| {
            observed
                .borrow_mut()
                .push((*completion, editor.read(cx).field_text().to_owned()));
        })
    });
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |composer, cx| {
            composer.set_selected_text_range(2..5, window, cx);
        })
    })?;
    fx.click(at)?;
    fx.wait("recording fixture input", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.capture_evidence("CGX_DICTATION_EVIDENCE", "native-recording")?;
    fx.click(at)?;
    backend.respond(200, "{\"text\":\"hello 🦀\"}")?;
    fx.wait("transcript insertion", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_label() == "Dictate")
    })?;
    ensure!(
        fx.composer_text()? == "α hello 🦀 ω",
        "selection was not replaced"
    );
    ensure!(
        fx.composer_focused()?,
        "dictation did not return focus to the editor"
    );
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |composer, cx| {
            assert_eq!(
                composer
                    .selected_text_range(false, window, cx)
                    .unwrap()
                    .range,
                10..10
            );
        })
    })?;
    fx.keys("cmd-z")?;
    ensure!(
        fx.composer_text()? == "α old ω",
        "dictation did not form one undo step"
    );
    fx.keys("cmd-shift-z")?;
    ensure!(
        fx.composer_text()? == "α hello 🦀 ω",
        "dictation redo failed"
    );
    fx.capture_evidence("CGX_DICTATION_EVIDENCE", "native-transcript")?;
    fx.click(at)?;
    fx.wait("second fixture recording", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.click(at)?;
    backend.received.recv_timeout(Duration::from_secs(20))?;
    fx.focus_composer()?;
    fx.type_text(" safe")?;
    backend
        .reply
        .send((200, "{\"text\":\"MUST NOT INSERT\"}"))?;
    fx.wait("cancelled editor remains idle", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_label() == "Dictate")
    })?;
    ensure!(
        fx.composer_text()? == "α hello 🦀 safe ω",
        "late text changed a cancelled draft"
    );
    fx.click(at)?;
    fx.wait("recording before Escape", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.keys("escape")?;
    fx.click(at)?;
    fx.wait("error fixture recording", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.click(at)?;
    backend.respond(503, "{\"error\":\"fixture unavailable\"}")?;
    fx.wait("transcription error", |fx| {
        fx.read(|_, cx| composer.read(cx).dictation_error().is_some())
    })?;
    ensure!(
        fx.read(|_, cx| composer.read(cx).dictation_error() == Some("Unable to transcribe audio")),
        "transcription error was hidden"
    );
    fx.capture_evidence("CGX_DICTATION_EVIDENCE", "native-error")?;
    ensure!(
        fx.composer_text()? == "α hello 🦀 safe ω",
        "error changed the draft"
    );
    backend.worker.join().unwrap()?;
    fx.cx.run_until_parked();
    let completions = completions.borrow();
    assert!(matches!(
        completions.as_slice(),
        [
            (Completion::Succeeded, _),
            (Completion::Cancelled, _),
            (Completion::Cancelled, _),
            (Completion::Failed, _),
        ]
    ));
    assert_eq!(completions[0].1, "α hello 🦀 ω");
    println!(
        "PASS: controlled WAV -> authenticated HTTP -> live Composer selection; Unicode caret, undo, cancel, late reply, error. No microphone capture or permission request."
    );
    Ok(())
}

#[test]
fn closed_composer_cannot_retarget_a_late_result_to_another_window() -> Result<()> {
    use ui::composer::{
        Composer,
        dictation::{Event, Session},
    };
    let mut fx = Fx::new()?;
    let (sender, events) = async_channel::unbounded();
    let (commands, _receiver) = mpsc::channel();
    fx.cx.update(|cx| {
        cx.set_global(Dispatcher::new(move |_, _| {
            Ok(Session {
                commands: commands.clone(),
                events: events.clone(),
            })
        }))
    });
    let first = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        first.update(cx, |composer, cx| {
            composer.set_text("original", cx);
            composer.dictate(window, cx);
        })
    })?;
    let other = fx.cx.open_window(size(px(720.), px(200.)), |window, cx| {
        cx.new(|cx| {
            let mut composer = Composer::home(window, cx);
            composer.set_text("other", cx);
            composer.focus_handle(cx).focus(window, cx);
            composer
        })
    })?;
    let other_editor = fx.cx.read_window(&other, |editor, _| editor)?;
    fx.cx
        .update_window(fx.window, |_, window, _| window.remove_window())?;
    sender.try_send(Event::Finished(Ok("wrong".into())))?;
    fx.cx.run_until_parked();
    first.read_with(&fx.cx, |editor, _| {
        assert_eq!(editor.field_text(), "original")
    });
    other_editor.read_with(&fx.cx, |editor, _| assert_eq!(editor.field_text(), "other"));
    Ok(())
}

#[test]
fn each_window_uses_its_own_host_without_cancelling_another_window() -> Result<()> {
    let first_backend = Backend::new(1)?;
    let second_backend = Backend::new(1)?;
    let mut fx = Fx::new()?;
    first_backend.install(&mut fx, Duration::from_secs(60));
    let first = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        first.update(cx, |editor, cx| {
            editor.set_text("first ", cx);
            editor.dictate(window, cx);
        })
    })?;
    fx.wait("first recording", |fx| {
        fx.read(|_, cx| first.read(cx).dictation_label() == "Stop dictation")
    })?;

    let mut recording = tempfile::NamedTempFile::new()?;
    for entry in common::script(None, |_| {})? {
        writeln!(recording, "{entry}")?;
    }
    let (bridge, events) = bridge::Bridge::replay(recording.path())?;
    let second_window = fx.cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        cx.new(|cx| {
            app::Root::new(
                app::State::default(),
                bridge,
                events,
                common::CWD.into(),
                window,
                cx,
            )
        })
    })?;
    let second_root = fx.cx.read_window(&second_window, |root, _| root)?;
    let second = fx.cx.update(|cx| {
        second_root.update(cx, |root, cx| {
            root.set_host(second_backend.host.clone(), cx);
            root.composer(cx)
        })
    });
    ensure!(fx.read(|_, cx| first.read(cx).has_active_dictation()));
    fx.cx.update_window(fx.window, |_, window, cx| {
        first.update(cx, |editor, cx| editor.finish_dictation(window, cx))
    })?;
    first_backend.respond(200, "{\"text\":\"account one\"}")?;
    fx.wait("first host transcript", |fx| {
        fx.read(|_, cx| first.read(cx).field_text() == "first account one")
    })?;

    fx.cx.update_window(second_window.into(), |_, window, cx| {
        second.update(cx, |editor, cx| {
            editor.set_text("second ", cx);
            editor.dictate(window, cx);
        })
    })?;
    fx.wait("second recording", |fx| {
        fx.read(|_, cx| second.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.cx.update_window(second_window.into(), |_, window, cx| {
        second.update(cx, |editor, cx| editor.finish_dictation(window, cx))
    })?;
    second_backend.respond(200, "{\"text\":\"account two\"}")?;
    fx.wait("second host transcript", |fx| {
        fx.read(|_, cx| second.read(cx).field_text() == "second account two")
    })?;
    first_backend.worker.join().unwrap()?;
    second_backend.worker.join().unwrap()?;
    Ok(())
}

#[test]
fn a_stalled_transcription_finishes_with_an_error() -> Result<()> {
    let backend = Backend::new(1)?;
    let mut fx = Fx::new()?;
    backend.install(&mut fx, Duration::from_millis(50));
    let editor = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_text("keep", cx);
            editor.dictate(window, cx);
            editor.finish_dictation(window, cx);
        })
    })?;
    backend.received.recv_timeout(Duration::from_secs(20))?;
    fx.wait("stalled transcription deadline", |fx| {
        fx.read(|_, cx| editor.read(cx).dictation_error().is_some())
    })?;
    ensure!(fx.read(|_, cx| {
        let editor = editor.read(cx);
        !editor.has_active_dictation()
            && editor.field_text() == "keep"
            && editor.dictation_error() == Some("Unable to transcribe audio")
    }));
    backend.reply.send((200, "{\"text\":\"late\"}"))?;
    let _ = backend.worker.join().unwrap();
    Ok(())
}
