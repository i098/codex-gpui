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
    let started: Bool = unsafe { msg_send![&*recorder.0, recordForDuration: LIMIT.as_secs_f64()] };
    ensure!(
        started.as_bool(),
        "Unable to start dictation: check the microphone"
    );
    let _ = events.try_send(Event::Recording);
    let start = Instant::now();
    loop {
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Finish) => break,
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
