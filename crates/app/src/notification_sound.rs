#[cfg(not(target_os = "macos"))]
use anyhow::bail;
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use symphonia::core::{
    audio::SampleBuffer, errors::Error, formats::FormatOptions, io::MediaSourceStream, probe::Hint,
};

const MAX_INPUT: u64 = 10 * 1024 * 1024;
const BUILTINS: &[&str] = &[
    "default", "classic", "none", "Basso", "Glass", "Ping", "Pop", "Purr", "Tink",
];

pub(super) fn valid(value: &Value) -> bool {
    if let Some(value) = value.as_str() {
        return BUILTINS.contains(&value);
    }
    let Some(name) = value["fileName"].as_str() else {
        return false;
    };
    let Some(hash_ext) = name.strip_prefix("codex-custom-") else {
        return false;
    };
    let Some((hash, ext)) = hash_ext.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && ["wav", "aiff", "caf"].contains(&ext)
        && value["name"].as_str().is_some_and(|name| !name.is_empty())
}

fn sound_directory() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/codex-gpui/notification-sounds"))
}

pub(super) fn import(path: &Path) -> Result<Value> {
    let directory = sound_directory()?;
    import_into(path, &directory)
}

pub(super) fn import_into(path: &Path, directory: &Path) -> Result<Value> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    ensure!(
        ["mp3", "wav", "flac", "ogg"].contains(&extension.as_str()),
        "unsupported notification sound format"
    );
    let file = fs::File::open(path)?;
    ensure!(file.metadata()?.is_file(), "sound is not a regular file");
    let mut bytes = Vec::new();
    file.take(MAX_INPUT + 1).read_to_end(&mut bytes)?;
    ensure!(
        (bytes.len() as u64) < MAX_INPUT,
        "notification sound must be under 10 MB"
    );
    let wave = decode(bytes, &extension)?;
    let file_name = format!("codex-custom-{:x}.wav", Sha256::digest(&wave));
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&wave)?;
    temporary.persist(directory.join(&file_name))?;
    Ok(
        json!({"fileName": file_name, "name": path.file_name().context("sound has no name")?.to_string_lossy()}),
    )
}

fn decode(bytes: Vec<u8>, extension: &str) -> Result<Vec<u8>> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &Default::default(),
        )?
        .format;
    let track = format.default_track().context("sound has no audio track")?;
    let track_id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .context("sound has no sample rate")?;
    let channels = track
        .codec_params
        .channels
        .context("sound has no channel layout")?
        .count();
    ensure!(
        (1..=192_000).contains(&rate) && (1..=8).contains(&channels),
        "unsupported sound layout"
    );
    let maximum_samples = u64::from(rate) * channels as u64 * 30;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &Default::default())?;
    let mut wave = vec![0; 44];
    let mut sample_buffer = None;
    let mut capacity = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        ensure!(
            decoded.spec().rate == rate && decoded.spec().channels.count() == channels,
            "sound changes format"
        );
        ensure!(
            (wave.len() - 44) as u64 / 2 + ((decoded.frames() * channels) as u64) < maximum_samples,
            "notification sound must be under 30 seconds"
        );
        if decoded.capacity() > capacity {
            capacity = decoded.capacity();
            sample_buffer = Some(SampleBuffer::<i16>::new(capacity as u64, *decoded.spec()));
        }
        let samples = sample_buffer.as_mut().context("sound has no samples")?;
        samples.copy_interleaved_ref(decoded);
        wave.extend(
            samples
                .samples()
                .iter()
                .flat_map(|sample| sample.to_le_bytes()),
        );
    }
    ensure!(wave.len() > 44, "sound is empty");
    let data_length = u32::try_from(wave.len() - 44)?;
    let channels = channels as u16;
    let mut header = Cursor::new(&mut wave[..44]);
    header.write_all(b"RIFF")?;
    header.write_all(&(data_length + 36).to_le_bytes())?;
    header.write_all(b"WAVEfmt ")?;
    header.write_all(&16u32.to_le_bytes())?;
    header.write_all(&1u16.to_le_bytes())?;
    header.write_all(&channels.to_le_bytes())?;
    header.write_all(&rate.to_le_bytes())?;
    header.write_all(&(rate * u32::from(channels) * 2).to_le_bytes())?;
    header.write_all(&(channels * 2).to_le_bytes())?;
    header.write_all(&16u16.to_le_bytes())?;
    header.write_all(b"data")?;
    header.write_all(&data_length.to_le_bytes())?;
    Ok(wave)
}

pub(super) enum Preview {
    Silent,
    Named(&'static str),
    Data(std::borrow::Cow<'static, [u8]>),
}

pub(super) fn prepare_preview(value: &Value) -> Result<Preview> {
    ensure!(valid(value), "invalid notification sound");
    match value.as_str() {
        Some("none") => Ok(Preview::Silent),
        Some("default") => Ok(Preview::Data(std::borrow::Cow::Borrowed(include_bytes!(
            "../../../assets/sounds/codex-notification.wav"
        )))),
        Some("classic") => Ok(Preview::Data(std::borrow::Cow::Borrowed(include_bytes!(
            "../../../assets/sounds/codex-classic.wav"
        )))),
        Some(value) => Ok(Preview::Named(
            BUILTINS
                .iter()
                .copied()
                .find(|name| *name == value)
                .context("unknown sound")?,
        )),
        None => {
            let path = sound_directory()?.join(
                value["fileName"]
                    .as_str()
                    .context("missing imported sound name")?,
            );
            let file = fs::File::open(path)?;
            let maximum = 192_000 * 8 * 2 * 30 + 44;
            ensure!(
                file.metadata()?.len() <= maximum,
                "imported sound is too large"
            );
            let mut bytes = Vec::new();
            file.take(maximum + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= maximum, "imported sound is too large");
            Ok(Preview::Data(std::borrow::Cow::Owned(bytes)))
        }
    }
}

#[cfg(target_os = "macos")]
thread_local! {
    static CURRENT_SOUND: std::cell::RefCell<Option<objc2::rc::Retained<objc2_app_kit::NSSound>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn stop_preview() {
    #[cfg(target_os = "macos")]
    CURRENT_SOUND.with_borrow_mut(|current| {
        if let Some(sound) = current.take() {
            sound.stop();
        }
    });
}

#[cfg(target_os = "macos")]
pub(super) fn play_preview(preview: Preview) -> Result<()> {
    use objc2::AnyThread as _;
    use objc2_app_kit::NSSound;
    use objc2_foundation::{NSData, NSString};
    stop_preview();
    let sound = match preview {
        Preview::Silent => return Ok(()),
        Preview::Named(name) => NSSound::soundNamed(&NSString::from_str(name)),
        Preview::Data(data) => NSSound::initWithData(NSSound::alloc(), &NSData::with_bytes(&data)),
    }
    .context("could not decode notification sound")?;
    ensure!(sound.play(), "could not play notification sound");
    CURRENT_SOUND.with_borrow_mut(|current| *current = Some(sound));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn play_preview(preview: Preview) -> Result<()> {
    match preview {
        Preview::Silent => Ok(()),
        Preview::Named(name) => {
            bail!("cannot play {name}: notification sound playback is available on macOS")
        }
        Preview::Data(data) => bail!(
            "cannot play {} audio bytes: notification sound playback is available on macOS",
            data.len()
        ),
    }
}

#[cfg(target_os = "macos")]
pub(super) fn open_settings() -> Result<()> {
    let identifier = objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .context("notification settings require an app bundle")?;
    let status = std::process::Command::new("/usr/bin/open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={identifier}"
        ))
        .status()?;
    ensure!(status.success(), "could not open notification settings");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn open_settings() -> Result<()> {
    bail!("notification settings are available on macOS")
}

#[cfg(target_os = "macos")]
pub(super) fn permission() -> Result<bool> {
    use block2::RcBlock;
    use objc2_foundation::NSBundle;
    use objc2_user_notifications::{
        UNAuthorizationStatus, UNNotificationSetting, UNNotificationSettings,
        UNUserNotificationCenter,
    };
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return Ok(false);
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let callback = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
        // The framework keeps its settings object alive for this callback.
        let settings = unsafe { settings.as_ref() };
        let allowed = matches!(
            settings.authorizationStatus(),
            UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional
        ) && settings.alertSetting() == UNNotificationSetting::Enabled;
        let _ = sender.try_send(allowed);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&callback);
    Ok(receiver.recv_timeout(std::time::Duration::from_secs(5))?)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn permission() -> Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn native_preview_stops_when_none_is_selected() -> Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        play_preview(prepare_preview(&json!("default"))?)?;
        let playing = CURRENT_SOUND
            .with_borrow(|sound| sound.clone())
            .context("preview did not start")?;
        assert!(playing.isPlaying());
        play_preview(prepare_preview(&json!("none"))?)?;
        assert!(!playing.isPlaying());
        Ok(())
    }

    #[test]
    fn rejects_thirty_seconds_but_accepts_shorter_audio() {
        let wave = |seconds: u32| {
            let length = seconds * 10 * 2;
            let mut bytes = Vec::from(&b"RIFF"[..]);
            bytes.extend((length + 36).to_le_bytes());
            bytes.extend(b"WAVEfmt ");
            bytes.extend(16u32.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(10u32.to_le_bytes());
            bytes.extend(20u32.to_le_bytes());
            bytes.extend(2u16.to_le_bytes());
            bytes.extend(16u16.to_le_bytes());
            bytes.extend(b"data");
            bytes.extend(length.to_le_bytes());
            bytes.resize(44 + length as usize, 0);
            bytes
        };
        assert!(decode(wave(29), "wav").is_ok());
        assert!(decode(wave(30), "wav").is_err());
    }

    #[test]
    fn imports_audio_without_retaining_the_source_path() -> Result<()> {
        let input = tempfile::tempdir()?;
        let output = tempfile::tempdir()?;
        let source = input.path().join("bell.wav");
        fs::write(
            &source,
            include_bytes!("../../../assets/sounds/codex-notification.wav"),
        )?;
        let value = import_into(&source, output.path())?;
        assert!(valid(&value));
        fs::remove_file(source)?;
        let saved = fs::read(output.path().join(value["fileName"].as_str().unwrap()))?;
        assert_eq!(&saved[..4], b"RIFF");
        assert_eq!(value["name"], "bell.wav");
        assert!(!valid(
            &json!({"fileName": "../../outside.wav", "name": "bad"})
        ));
        Ok(())
    }

    #[test]
    fn rejects_oversize_and_invalid_audio() -> Result<()> {
        let input = tempfile::tempdir()?;
        let source = input.path().join("invalid.wav");
        fs::write(&source, b"not audio")?;
        assert!(import_into(&source, input.path()).is_err());
        fs::File::create(&source)?.set_len(MAX_INPUT)?;
        assert!(import_into(&source, input.path()).is_err());
        assert!(!valid(
            &json!({"fileName": format!("codex-custom-{}.wav", "0".repeat(64)), "name": ""})
        ));
        Ok(())
    }
}
