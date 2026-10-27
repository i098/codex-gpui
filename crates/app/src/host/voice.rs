use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};
use ui::screens::settings::voice::{Microphone, Recording};

const MAX_AUDIO_BYTES: u64 = 128 * 1024 * 1024;

pub struct Settings {
    home: PathBuf,
    preferences: PathBuf,
}

impl Settings {
    pub fn new(home: PathBuf, preferences: PathBuf) -> Self {
        Self { home, preferences }
    }

    pub fn values(&self) -> Result<Value> {
        Ok(Value::Object(
            crate::host_settings::Settings::load(self.preferences.clone())?
                .read_all()
                .clone(),
        ))
    }

    pub fn normalize(key: &str, value: Value) -> Result<Value> {
        match key {
            "realtimeVoiceScreenContextEnabled" | "dictationSoundsEnabled" => {
                ensure!(value.is_boolean(), "Invalid Voice preference")
            }
            "microphoneInputDeviceId" => ensure!(
                value.is_null() || value.as_str().is_some_and(|v| !v.is_empty()),
                "Invalid microphone identifier"
            ),
            "dictationDictionary" => ensure!(
                value
                    .as_array()
                    .is_some_and(|words| words.iter().all(Value::is_string)),
                "Invalid dictionary entries"
            ),
            _ => bail!("Unknown Voice preference"),
        }
        let value = if key == "dictationDictionary" {
            json!(
                value
                    .as_array()
                    .context("Invalid dictionary entries")?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            )
        } else {
            value
        };
        Ok(value)
    }

    pub fn write(&self, key: &str, value: Value) -> Result<()> {
        let value = Self::normalize(key, value)?;
        crate::host_settings::Settings::load(self.preferences.clone())?.write(key, value)
    }

    pub fn begin_capture(&self, recording: &Recording) -> Result<PathBuf> {
        use std::os::unix::fs::PermissionsExt as _;
        Self::validate_recording(recording)?;
        ensure!(
            recording.status == "recording",
            "A new capture must be recording"
        );
        ensure!(!recording.id.is_empty(), "Invalid recording identifier");
        let root = self.root();
        fs::create_dir_all(&root)?;
        ensure!(
            !fs::symlink_metadata(&root)?.file_type().is_symlink(),
            "Invalid recording store"
        );
        ensure!(
            root.canonicalize()?.starts_with(self.home.canonicalize()?),
            "Recording store escapes CODEX_HOME"
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let directory = root.join(format!("{:x}", Sha256::digest(recording.id.as_bytes())));
        fs::create_dir(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        Self::write_metadata(&directory, &serde_json::to_value(recording)?)?;
        Ok(directory.join("0000000000.chunk"))
    }

    pub fn finish_capture(&self, id: &str, status: &str) -> Result<()> {
        ensure!(
            matches!(status, "completed" | "cancelled" | "interrupted"),
            "Invalid final recording status"
        );
        let (directory, mut value, _) = self.metadata(id)?;
        value["status"] = status.into();
        value["sizeBytes"] = self
            .chunks(&directory)?
            .iter()
            .try_fold(0u64, |size, path| {
                Ok::<_, anyhow::Error>(size + fs::metadata(path)?.len())
            })?
            .into();
        Self::write_metadata(&directory, &value)
    }

    fn write_metadata(directory: &Path, value: &Value) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.as_file().sync_all()?;
        file.persist(directory.join("metadata.json"))?;
        Ok(())
    }

    fn root(&self) -> PathBuf {
        self.home.join("dictation-history")
    }

    fn directory(&self, id: &str) -> Result<PathBuf> {
        ensure!(!id.is_empty(), "Invalid recording identifier");
        let root = self.root();
        let path = root.join(format!("{:x}", Sha256::digest(id.as_bytes())));
        let metadata = fs::symlink_metadata(&path).context("Dictation recording was not found")?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Invalid recording directory"
        );
        ensure!(
            path.canonicalize()?.starts_with(self.home.canonicalize()?),
            "Recording directory escapes CODEX_HOME"
        );
        Ok(path)
    }

    fn metadata(&self, id: &str) -> Result<(PathBuf, Value, Recording)> {
        let path = self.directory(id)?;
        let metadata_path = path.join("metadata.json");
        ensure!(
            !fs::symlink_metadata(&metadata_path)?
                .file_type()
                .is_symlink(),
            "Invalid recording metadata"
        );
        let value: Value = serde_json::from_slice(&fs::read(metadata_path)?)?;
        let item: Recording = serde_json::from_value(value.clone())?;
        Self::validate_recording(&item)?;
        ensure!(
            item.id == id,
            "Recording identifier does not match its directory"
        );
        Ok((path, value, item))
    }

    fn validate_recording(item: &Recording) -> Result<()> {
        ensure!(
            matches!(
                item.status.as_str(),
                "recording" | "completed" | "cancelled" | "interrupted"
            ),
            "Invalid recording status"
        );
        ensure!(!item.mime_type.is_empty(), "Invalid recording content type");
        Ok(())
    }

    fn legacy(&self) -> Result<Vec<Recording>> {
        let path = self.home.join("transcription-history.jsonl");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Invalid legacy recording store"
        );
        let raw = fs::read_to_string(&path)?;
        let mut items: Vec<_> = raw
            .lines()
            .filter_map(|line| {
                let value: Value = serde_json::from_str(line).ok()?;
                Some(Recording {
                    id: value["id"].as_str()?.to_owned(),
                    created_at_ms: value["createdAtMs"].as_i64()?,
                    text: value["text"].as_str()?.to_owned(),
                    mime_type: "audio/webm".into(),
                    size_bytes: 0,
                    status: "completed".into(),
                    language: None,
                })
            })
            .rev()
            .take(20)
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.created_at_ms));
        Ok(items)
    }

    fn delete_legacy(&self, id: &str) -> Result<()> {
        let path = self.home.join("transcription-history.jsonl");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Invalid legacy recording store"
        );
        // ponytail: coordinates native writers only; use separate CODEX_HOME while the original app writes legacy history.
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.home.join(".dictation-history.lock"))?;
        lock.lock()?;
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Invalid legacy recording store"
        );
        let raw = fs::read_to_string(&path)?;
        let mut output = String::new();
        for line in raw.lines() {
            if serde_json::from_str::<Value>(line).is_ok_and(|value| value["id"] == id) {
                continue;
            }
            output.push_str(line);
            output.push('\n');
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.home)?;
        file.write_all(output.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }

    pub fn recordings(&self) -> Result<Vec<Recording>> {
        let root = self.root();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return self.legacy(),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            root.canonicalize()?.starts_with(self.home.canonicalize()?),
            "Recording directory escapes CODEX_HOME"
        );
        let mut items = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("metadata.json");
            if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                continue;
            }
            let read = (|| -> Result<Recording> {
                let mut item: Recording = serde_json::from_slice(&fs::read(path)?)?;
                Self::validate_recording(&item)?;
                let directory = self.directory(&item.id)?;
                ensure!(
                    directory == entry.path(),
                    "Recording identifier does not match its directory"
                );
                item.size_bytes = self
                    .chunks(&directory)?
                    .iter()
                    .try_fold(0u64, |total, path| {
                        Ok::<_, anyhow::Error>(total + fs::metadata(path)?.len())
                    })?;
                Ok(item)
            })();
            match read {
                Ok(item) => items.push(item),
                Err(error) => eprintln!("Could not read a dictation recording: {error}"),
            }
        }
        items.extend(
            self.legacy()?
                .into_iter()
                .filter(|legacy| !items.iter().any(|item| item.id == legacy.id))
                .collect::<Vec<_>>(),
        );
        items.sort_by_key(|item| std::cmp::Reverse(item.created_at_ms));
        Ok(items)
    }

    fn chunks(&self, directory: &Path) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let valid = name
                .strip_suffix(".chunk")
                .is_some_and(|n| n.len() == 10 && n.bytes().all(|b| b.is_ascii_digit()));
            if valid {
                ensure!(entry.file_type()?.is_file(), "Invalid audio chunk");
                paths.push(entry.path());
            }
        }
        paths.sort();
        Ok(paths)
    }

    pub fn audio(&self, id: &str) -> Result<(Recording, Vec<u8>)> {
        let (directory, _, item) = self.metadata(id)?;
        let paths = self.chunks(&directory)?;
        let total = paths.iter().try_fold(0u64, |total, path| -> Result<u64> {
            total
                .checked_add(fs::metadata(path)?.len())
                .context("Recording size overflow")
        })?;
        ensure!(
            total <= MAX_AUDIO_BYTES,
            "Saved recording exceeds the 128 MiB retry limit"
        );
        let mut audio = Vec::new();
        audio.try_reserve_exact(usize::try_from(total)?)?;
        for path in paths {
            fs::File::open(path)?
                .take(MAX_AUDIO_BYTES + 1 - audio.len() as u64)
                .read_to_end(&mut audio)?;
            ensure!(
                audio.len() as u64 <= MAX_AUDIO_BYTES,
                "Saved recording exceeds the 128 MiB retry limit"
            );
        }
        ensure!(!audio.is_empty(), "Dictation recording has no audio");
        Ok((item, audio))
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self
            .root()
            .join(format!("{:x}", Sha256::digest(id.as_bytes())));
        if matches!(fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return self.delete_legacy(id);
        }
        let (directory, _, item) = self.metadata(id)?;
        ensure!(
            item.status != "recording",
            "Cannot delete an active dictation recording"
        );
        fs::remove_dir_all(directory)?;
        self.delete_legacy(id)
    }

    pub fn set_transcript(&self, id: &str, text: &str) -> Result<()> {
        let (directory, mut value, _) = self.metadata(id)?;
        ensure!(
            !text.trim().is_empty(),
            "Dictation recording returned an empty transcript"
        );
        value["text"] = text.trim().into();
        Self::write_metadata(&directory, &value)
    }

    pub fn download(&self, id: &str, directory: &Path) -> Result<PathBuf> {
        let (saved, _, item) = self.metadata(id)?;
        let chunks = self.chunks(&saved)?;
        ensure!(!chunks.is_empty(), "Dictation recording has no audio");
        let mime = item
            .mime_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let ext = match mime.as_str() {
            "audio/mp4" => "m4a",
            "audio/ogg" => "ogg",
            "audio/mpeg" => "mp3",
            "audio/aac" => "aac",
            "audio/wav" | "audio/wave" | "audio/x-wav" => "wav",
            _ => "webm",
        };
        let time = chrono::DateTime::from_timestamp_millis(item.created_at_ms)
            .context("Invalid recording timestamp")?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .replace(':', "-");
        fs::create_dir_all(directory)?;
        for index in 0u32.. {
            let suffix = if index == 0 {
                String::new()
            } else {
                format!(" ({index})")
            };
            let path = directory.join(format!("Codex dictation {time}{suffix}.{ext}"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let result = (|| -> std::io::Result<()> {
                        for path in &chunks {
                            std::io::copy(&mut fs::File::open(path)?, &mut file)?;
                        }
                        file.sync_all()
                    })();
                    if let Err(error) = result {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error.into());
                    }
                    return Ok(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("No download filename is available")
    }
}

#[cfg(not(target_os = "macos"))]
pub fn microphones() -> Result<Vec<Microphone>> {
    bail!("Microphone selection is not available on this device")
}

#[cfg(target_os = "macos")]
pub fn microphones() -> Result<Vec<Microphone>> {
    core_audio::microphones()
}

#[cfg(target_os = "macos")]
mod core_audio {
    use super::*;
    use std::{
        ffi::{c_char, c_void},
        mem::{self, size_of},
    };
    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }
    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetLength(value: *const c_void) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            value: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> bool;
        fn CFRelease(value: *const c_void);
    }
    fn address(selector: &[u8; 4], input: bool) -> Address {
        Address {
            selector: u32::from_be_bytes(*selector),
            scope: u32::from_be_bytes(if input { *b"inpt" } else { *b"glob" }),
            element: 0,
        }
    }
    fn bytes(object: u32, property: &Address) -> Result<Vec<u8>> {
        let mut size = 0;
        ensure!(
            unsafe {
                AudioObjectGetPropertyDataSize(object, property, 0, std::ptr::null(), &mut size)
            } == 0,
            "Could not read audio device properties"
        );
        let mut bytes = vec![0u8; size as usize];
        ensure!(
            unsafe {
                AudioObjectGetPropertyData(
                    object,
                    property,
                    0,
                    std::ptr::null(),
                    &mut size,
                    bytes.as_mut_ptr().cast(),
                )
            } == 0,
            "Could not read audio device properties"
        );
        bytes.truncate(size as usize);
        Ok(bytes)
    }
    fn string(object: u32, selector: &[u8; 4]) -> Result<String> {
        let data = bytes(object, &address(selector, false))?;
        ensure!(
            data.len() == size_of::<*const c_void>(),
            "Invalid audio device string"
        );
        let value = unsafe { std::ptr::read_unaligned(data.as_ptr().cast::<*const c_void>()) };
        ensure!(!value.is_null(), "Missing audio device string");
        let size =
            unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), 0x08000100) } + 1;
        let mut buffer = vec![0u8; size as usize];
        let ok = unsafe { CFStringGetCString(value, buffer.as_mut_ptr().cast(), size, 0x08000100) };
        unsafe { CFRelease(value) };
        ensure!(ok, "Invalid audio device name");
        let length = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        Ok(String::from_utf8(buffer[..length].to_vec())?)
    }
    pub fn microphones() -> Result<Vec<Microphone>> {
        let devices = bytes(1, &address(b"dev#", false))?;
        ensure!(
            devices.len() % mem::size_of::<u32>() == 0,
            "Invalid audio device list"
        );
        let mut result = Vec::new();
        for device in devices.as_chunks::<4>().0 {
            let device = u32::from_ne_bytes(*device);
            if bytes(device, &address(b"stm#", true))?.is_empty() {
                continue;
            }
            let device_id = string(device, b"uid ")?;
            let label = string(device, b"lnam")?;
            if !device_id.is_empty() {
                result.push(Microphone { device_id, label });
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    #[test]
    fn completed_capture_can_retry_until_a_transcript_is_saved() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let settings = Settings::new(
            directory.path().to_owned(),
            directory.path().join("preferences.json"),
        );
        let recording = Recording {
            id: "owned-capture".into(),
            created_at_ms: 1,
            mime_type: "audio/wav".into(),
            size_bytes: 0,
            status: "recording".into(),
            text: String::new(),
            language: None,
        };
        let chunk = settings.begin_capture(&recording)?;
        fs::write(chunk, b"RIFF\\0\\0\\0\\0WAVEfmt ")?;
        assert!(!settings.recordings()?[0].can_retry(true));
        settings.finish_capture(&recording.id, "completed")?;
        assert!(settings.recordings()?[0].can_retry(true));
        settings.set_transcript(&recording.id, "captured text")?;
        let saved = settings.recordings()?.remove(0);
        assert_eq!(saved.text, "captured text");
        assert!(!saved.can_retry(true));
        assert!(settings.begin_capture(&recording).is_err());
        assert_eq!(settings.recordings()?[0].text, "captured text");
        Ok(())
    }
}
