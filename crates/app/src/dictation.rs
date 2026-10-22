use crate::host::Host;
use anyhow::Result;
use gpui::{App, Window};
use std::sync::Arc;
use ui::composer::dictation::{Dispatcher, Session};

#[cfg(any(target_os = "macos", test, feature = "test-support"))]
use {
    crate::host::{self, Request},
    anyhow::ensure,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    serde_json::json,
    std::time::Duration,
    std::{io::Write as _, sync::mpsc},
    ui::composer::dictation::{Command, Event},
};

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use anyhow::Context as _;
    use objc2::{
        class, msg_send,
        rc::Retained,
        runtime::{AnyObject, Bool},
    };
    use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString, NSURL};
    use std::{
        path::Path,
        sync::Mutex,
        time::{Duration, Instant},
    };

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: &'static NSString;
    }

    static MICROPHONE: Mutex<()> = Mutex::new(());
    const LIMIT: Duration = Duration::from_secs(300);

    pub(super) fn record(
        commands: &mpsc::Receiver<Command>,
        events: &async_channel::Sender<Event>,
    ) -> Result<Vec<u8>> {
        let _lease = match MICROPHONE.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                anyhow::bail!("The microphone is already in use")
            }
        };
        permission(commands)?;
        ensure!(!cancelled(commands), "Dictation cancelled");
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("dictation.wav");
        let recorder = Recorder::new(&path)?;
        unsafe {
            let _: () = msg_send![&*recorder.0, setMeteringEnabled: Bool::YES];
        }
        let started: Bool =
            unsafe { msg_send![&*recorder.0, recordForDuration: LIMIT.as_secs_f64()] };
        ensure!(
            started.as_bool(),
            "Unable to start dictation: check the microphone"
        );
        let _ = events.try_send(Event::Recording);
        let start = Instant::now();
        loop {
            match commands.recv_timeout(Duration::from_millis(200)) {
                Ok(Command::Finish) => break,
                Ok(Command::Retry) => continue,
                Ok(Command::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Dictation cancelled")
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if start.elapsed() >= LIMIT {
                break;
            }
            let recording: Bool = unsafe { msg_send![&*recorder.0, isRecording] };
            ensure!(
                recording.as_bool(),
                "Unable to start dictation: the microphone stopped"
            );
            let power: f32 = unsafe {
                let _: () = msg_send![&*recorder.0, updateMeters];
                msg_send![&*recorder.0, averagePowerForChannel: 0_usize]
            };
            let _ = events.try_send(Event::Level(10_f32.powf(power / 20.)));
        }
        drop(recorder);
        ensure!(
            start.elapsed() >= Duration::from_millis(250),
            "Unable to transcribe audio: recording is too short"
        );
        ensure!(
            std::fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
            "Unable to transcribe audio: recording is too large"
        );
        std::fs::read(path).context("Unable to read dictation audio")
    }

    fn permission(commands: &mpsc::Receiver<Command>) -> Result<()> {
        let status: isize = unsafe {
            msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeAudio]
        };
        match status {
            3 => Ok(()),
            0 => {
                let (sender, receiver) = mpsc::channel();
                let block = block2::RcBlock::new(move |granted: Bool| {
                    let _ = sender.send(granted.as_bool());
                });
                unsafe {
                    let _: () = msg_send![class!(AVCaptureDevice), requestAccessForMediaType: AVMediaTypeAudio, completionHandler: &*block];
                }
                let start = Instant::now();
                loop {
                    ensure!(!cancelled(commands), "Dictation cancelled");
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(true) => return Ok(()),
                        Ok(false) => anyhow::bail!("Enable mic access to dictate"),
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            anyhow::bail!("Unable to check microphone permission")
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    ensure!(
                        start.elapsed() < Duration::from_secs(120),
                        "Microphone permission request timed out"
                    );
                }
            }
            _ => anyhow::bail!("Enable mic access to dictate"),
        }
    }

    struct Recorder(Retained<AnyObject>);

    impl Recorder {
        fn new(path: &Path) -> Result<Self> {
            let url = NSURL::fileURLWithPath(&NSString::from_str(
                path.to_str().context("Invalid recording path")?,
            ));
            let keys = [
                "AVFormatIDKey",
                "AVSampleRateKey",
                "AVNumberOfChannelsKey",
                "AVLinearPCMBitDepthKey",
                "AVLinearPCMIsBigEndianKey",
                "AVLinearPCMIsFloatKey",
            ]
            .map(NSString::from_str);
            let values = [1819304813., 16000., 1., 16., 0., 0.].map(NSNumber::new_f64);
            let settings = NSDictionary::from_slices(
                &keys.each_ref().map(|key| &**key),
                &values.each_ref().map(|value| &**value),
            );
            let mut error: Option<Retained<NSError>> = None;
            let allocated: *mut AnyObject = unsafe { msg_send![class!(AVAudioRecorder), alloc] };
            let recorder: *mut AnyObject = unsafe {
                msg_send![allocated, initWithURL: &*url, settings: &*settings, error: &mut error]
            };
            Ok(Self(unsafe { Retained::from_raw(recorder) }.context(
                "Unable to start dictation: no available microphone",
            )?))
        }
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![&*self.0, stop];
            }
        }
    }
}

pub fn dispatcher() -> Dispatcher {
    Dispatcher::new(|window, cx| start(window_host(window, cx)?))
}

fn window_host(window: &Window, cx: &App) -> Result<Arc<dyn Host>, String> {
    window
        .root::<crate::Root>()
        .flatten()
        .map(|root| root.read(cx).host.clone())
        .ok_or_else(|| "Dictation is not available in this window".into())
}

fn start(host: Arc<dyn Host>) -> Result<Session, String> {
    #[cfg(target_os = "macos")]
    return session(host, native::record, Duration::from_secs(60));
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
    timeout: Duration,
) -> Result<Session, String> {
    let (commands, receiver) = mpsc::channel();
    let (events, replies) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("codex-dictation".into())
        .spawn(move || {
            let audio = match capture(&receiver, &events) {
                Ok(audio) => audio,
                Err(error) => {
                    if !cancelled(&receiver) {
                        let _ = events.try_send(Event::Finished(Err(error.to_string())));
                    }
                    return;
                }
            };
            if cancelled(&receiver) {
                return;
            }
            let path = host
                .save_dictation(&audio)
                .ok()
                .and_then(|path| url::Url::from_file_path(path).ok())
                .map(String::from);
            if events.try_send(Event::Captured(path)).is_err() {
                return;
            }
            loop {
                let _ = events.try_send(Event::Transcribing);
                let result = transcribe(&*host, &audio, timeout).map_err(|error| error.to_string());
                let succeeded = result.is_ok();
                if cancelled(&receiver) {
                    return;
                }
                if events.try_send(Event::Finished(result)).is_err() || succeeded {
                    return;
                }
                match receiver.recv() {
                    Ok(Command::Retry) => {}
                    _ => return,
                }
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
fn transcribe(host: &dyn Host, audio: &[u8], timeout: Duration) -> Result<String> {
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
    body.extend_from_slice(audio);
    write!(body, "\r\n--{boundary}--\r\n")?;
    let request = Request::Http(json!({
        "url": "/transcribe", "method": "POST",
        "timeoutMs": timeout.as_millis(),
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
pub fn audio_dispatcher(audio: Vec<u8>, timeout: Duration) -> Dispatcher {
    Dispatcher::new(move |window, cx| {
        let audio = audio.clone();
        session(
            window_host(window, cx)?,
            move |commands, events| {
                let _ = events.try_send(Event::Recording);
                let samples = audio.get(44..).unwrap_or_default();
                let count = samples.len() / 2;
                let rms = (samples
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|sample| {
                        let value = i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.;
                        value * value
                    })
                    .sum::<f32>()
                    / count.max(1) as f32)
                    .sqrt();
                loop {
                    match commands.recv_timeout(Duration::from_millis(200)) {
                        Ok(Command::Finish) => break Ok(audio),
                        Ok(Command::Retry) => continue,
                        Ok(Command::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            anyhow::bail!("Dictation cancelled")
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let _ = events.try_send(Event::Level(rms));
                        }
                    }
                }
            },
            timeout,
        )
    })
}
