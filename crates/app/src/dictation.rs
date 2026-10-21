use crate::host::Host;
use anyhow::Result;
use std::sync::Arc;
use ui::composer::dictation::{Dispatcher, Session};

#[cfg(any(target_os = "macos", test, feature = "test-support"))]
use {
    crate::host::{self, Request},
    anyhow::ensure,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    serde_json::json,
    std::{io::Write as _, sync::mpsc},
    ui::composer::dictation::{Command, Event},
};

#[cfg(target_os = "macos")]
mod native;

pub fn dispatcher(host: Arc<dyn Host>) -> Dispatcher {
    Dispatcher::new(move || start(host.clone()))
}

fn start(host: Arc<dyn Host>) -> Result<Session, String> {
    #[cfg(target_os = "macos")]
    return session(host, native::record);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = host;
        Err("Dictation is not supported on this platform".into())
    }
}

#[cfg(any(target_os = "macos", test, feature = "test-support"))]
fn session(
    host: Arc<dyn Host>,
    capture: impl FnOnce(&mpsc::Receiver<Command>, &async_channel::Sender<Event>) -> Result<Vec<u8>>
    + Send
    + 'static,
) -> Result<Session, String> {
    let (commands, receiver) = mpsc::channel();
    let (events, replies) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("codex-dictation".into())
        .spawn(move || {
            let result = capture(&receiver, &events).and_then(|audio| {
                ensure!(!cancelled(&receiver), "Dictation cancelled");
                let _ = events.try_send(Event::Transcribing);
                transcribe(&*host, audio)
            });
            if !cancelled(&receiver) {
                let _ = events.try_send(Event::Finished(result.map_err(|error| error.to_string())));
            }
        })
        .map_err(|_| "Unable to start dictation".to_owned())?;
    Ok(Session {
        commands,
        events: replies,
    })
}

#[cfg(any(target_os = "macos", test, feature = "test-support"))]
fn cancelled(commands: &mpsc::Receiver<Command>) -> bool {
    !matches!(commands.try_recv(), Err(mpsc::TryRecvError::Empty))
}

#[cfg(any(target_os = "macos", test, feature = "test-support"))]
fn transcribe(host: &dyn Host, audio: Vec<u8>) -> Result<String> {
    ensure!(
        audio.len() > 44 && audio.len() <= 16 * 1024 * 1024,
        "Unable to transcribe audio: invalid recording size"
    );
    ensure!(
        audio.starts_with(b"RIFF") && audio.get(8..12) == Some(b"WAVE"),
        "Unable to transcribe audio: expected WAV audio"
    );
    let boundary = format!("----codex-transcribe-{}", host::device_id());
    let mut body = Vec::with_capacity(audio.len() + 256);
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    )?;
    body.extend_from_slice(&audio);
    drop(audio);
    write!(body, "\r\n--{boundary}--\r\n")?;
    let request = Request::Http(json!({
        "url": "/transcribe", "method": "POST",
        "headers": {
            "Content-Type": format!("multipart/form-data; boundary={boundary}"),
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
        },
        "bodyBase64": STANDARD.encode(body),
    }));
    let response = host
        .fetch(&request)
        .map_err(|_| anyhow::anyhow!("Unable to transcribe audio"))?;
    let body =
        host::json_body(&response).map_err(|_| anyhow::anyhow!("Unable to transcribe audio"))?;
    body["text"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Unable to transcribe audio: missing transcript"))
}

#[cfg(any(test, feature = "test-support"))]
pub fn audio_dispatcher(host: Arc<dyn Host>, audio: Vec<u8>) -> Dispatcher {
    Dispatcher::new(move || {
        let audio = audio.clone();
        session(host.clone(), move |commands, events| {
            let _ = events.try_send(Event::Recording);
            match commands.recv()? {
                Command::Finish => Ok(audio),
                Command::Cancel => anyhow::bail!("Dictation cancelled"),
            }
        })
    })
}
