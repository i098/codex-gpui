use anyhow::{Context as _, Result, ensure};
use std::{
    fs::File,
    io::{Seek as _, SeekFrom, Write as _},
    path::Path,
};

const SAMPLE_RATE: u32 = 44_100;
const MAX_AUDIO_BYTES: u32 = 128 * 1024 * 1024 - 44;

fn wav_header(bytes: u32) -> [u8; 44] {
    let mut header = [0; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&1u16.to_le_bytes());
    header[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&bytes.to_le_bytes());
    header
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::{class, msg_send, runtime::Bool};
    use objc2_foundation::NSString;
    use std::{
        ffi::{CString, c_char, c_void},
        ptr,
        sync::{
            Mutex, PoisonError,
            atomic::{AtomicU32, Ordering},
        },
    };

    type Queue = *mut c_void;
    #[repr(C)]
    struct Format {
        sample_rate: f64,
        format_id: u32,
        flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels: u32,
        bits_per_channel: u32,
        reserved: u32,
    }
    #[repr(C)]
    struct Buffer {
        capacity: u32,
        data: *mut c_void,
        bytes: u32,
        user_data: *mut c_void,
        packet_capacity: u32,
        packets: *mut c_void,
        packet_count: u32,
    }
    type InputCallback =
        unsafe extern "C" fn(*mut c_void, Queue, *mut Buffer, *const c_void, u32, *const c_void);

    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioQueueNewInput(
            format: *const Format,
            callback: InputCallback,
            context: *mut c_void,
            run_loop: *const c_void,
            mode: *const c_void,
            flags: u32,
            queue: *mut Queue,
        ) -> i32;
        fn AudioQueueAllocateBuffer(queue: Queue, bytes: u32, buffer: *mut *mut Buffer) -> i32;
        fn AudioQueueEnqueueBuffer(
            queue: Queue,
            buffer: *mut Buffer,
            packets: u32,
            descriptions: *const c_void,
        ) -> i32;
        fn AudioQueueSetProperty(
            queue: Queue,
            property: u32,
            data: *const c_void,
            bytes: u32,
        ) -> i32;
        fn AudioQueueStart(queue: Queue, at: *const c_void) -> i32;
        fn AudioQueueStop(queue: Queue, immediate: u8) -> i32;
        fn AudioQueueDispose(queue: Queue, immediate: u8) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }
    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    pub async fn request_access() -> Result<()> {
        let audio = NSString::from_str("soun");
        let authorization: isize =
            unsafe { msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: &*audio] };
        if authorization == 3 {
            return Ok(());
        }
        ensure!(
            authorization == 0,
            "Microphone access is disabled. Allow Codex in System Settings."
        );
        let (sender, receiver) = async_channel::bounded(1);
        let completion = block2::RcBlock::new(move |allowed: Bool| {
            let _ = sender.try_send(allowed.as_bool());
        });
        unsafe {
            let _: () = msg_send![class!(AVCaptureDevice), requestAccessForMediaType: &*audio, completionHandler: &*completion];
        }
        ensure!(receiver.recv().await?, "Microphone access was not allowed");
        Ok(())
    }

    struct Writer {
        file: File,
        bytes: u32,
        error: Option<String>,
    }
    struct Capture {
        writer: Mutex<Writer>,
        peak: AtomicU32,
    }
    pub struct Recorder {
        queue: Queue,
        capture: Option<Box<Capture>>,
    }

    fn check(status: i32, operation: &str) -> Result<()> {
        ensure!(status == 0, "{operation} failed ({status})");
        Ok(())
    }

    impl Recorder {
        pub fn start(path: &Path, device: Option<&str>) -> Result<Self> {
            use std::os::unix::fs::OpenOptionsExt as _;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(&wav_header(0))?;
            let mut recorder = Self {
                queue: ptr::null_mut(),
                capture: Some(Box::new(Capture {
                    writer: Mutex::new(Writer {
                        file,
                        bytes: 0,
                        error: None,
                    }),
                    peak: AtomicU32::new(0),
                })),
            };
            let format = Format {
                sample_rate: SAMPLE_RATE as f64,
                format_id: u32::from_be_bytes(*b"lpcm"),
                flags: 12,
                bytes_per_packet: 2,
                frames_per_packet: 1,
                bytes_per_frame: 2,
                channels: 1,
                bits_per_channel: 16,
                reserved: 0,
            };
            let capture = recorder
                .capture
                .as_mut()
                .context("Audio capture is unavailable")?;
            check(
                unsafe {
                    AudioQueueNewInput(
                        &format,
                        record,
                        (&mut **capture as *mut Capture).cast(),
                        ptr::null(),
                        ptr::null(),
                        0,
                        &mut recorder.queue,
                    )
                },
                "Create audio input",
            )?;
            if let Some(device) = device {
                let name = CString::new(device).context("Invalid microphone identifier")?;
                let device =
                    unsafe { CFStringCreateWithCString(ptr::null(), name.as_ptr(), 0x0800_0100) };
                ensure!(!device.is_null(), "Invalid microphone identifier");
                let status = unsafe {
                    AudioQueueSetProperty(
                        recorder.queue,
                        u32::from_be_bytes(*b"aqcd"),
                        (&device as *const *const c_void).cast(),
                        std::mem::size_of_val(&device) as u32,
                    )
                };
                unsafe {
                    CFRelease(device);
                }
                check(status, "Select microphone")?;
            }
            for _ in 0..3 {
                let mut buffer = ptr::null_mut();
                check(
                    unsafe {
                        AudioQueueAllocateBuffer(recorder.queue, SAMPLE_RATE / 5, &mut buffer)
                    },
                    "Allocate recording buffer",
                )?;
                check(
                    unsafe { AudioQueueEnqueueBuffer(recorder.queue, buffer, 0, ptr::null()) },
                    "Queue recording buffer",
                )?;
            }
            check(
                unsafe { AudioQueueStart(recorder.queue, ptr::null()) },
                "Start recording",
            )?;
            Ok(recorder)
        }

        pub fn level(&self) -> f32 {
            self.capture.as_ref().map_or(0., |capture| {
                f32::from_bits(capture.peak.load(Ordering::Relaxed))
            })
        }

        pub fn error(&self) -> Option<String> {
            self.capture.as_ref().and_then(|capture| {
                capture
                    .writer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .error
                    .clone()
            })
        }

        pub fn stop(mut self) -> Result<u32> {
            let closed = self.close_queue();
            let capture = self
                .capture
                .take()
                .context("Audio capture is unavailable")?;
            let mut writer = capture
                .writer
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner);
            writer.file.seek(SeekFrom::Start(0))?;
            let bytes = writer.bytes;
            writer.file.write_all(&wav_header(bytes))?;
            writer.file.sync_all()?;
            closed?;
            ensure!(
                writer.error.is_none(),
                "{}",
                writer.error.unwrap_or_default()
            );
            Ok(bytes)
        }

        fn close_queue(&mut self) -> Result<()> {
            if self.queue.is_null() {
                return Ok(());
            }
            let queue = std::mem::replace(&mut self.queue, ptr::null_mut());
            let stopped = unsafe { AudioQueueStop(queue, 1) };
            let disposed = unsafe { AudioQueueDispose(queue, 1) };
            // Disposal ends callbacks before returning, including asynchronous queue mode.
            check(disposed, "Release recording input")?;
            check(stopped, "Stop recording")
        }
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            let _ = self.close_queue();
        }
    }

    unsafe extern "C" fn record(
        context: *mut c_void,
        queue: Queue,
        buffer: *mut Buffer,
        _: *const c_void,
        _: u32,
        _: *const c_void,
    ) {
        if context.is_null() || buffer.is_null() {
            return;
        }
        // Core Audio owns the buffer until this callback returns; Capture outlives the queue.
        let capture = unsafe { &*context.cast::<Capture>() };
        let buffer_data = unsafe { &*buffer };
        if buffer_data.data.is_null() {
            return;
        }
        let size = buffer_data.bytes.min(buffer_data.capacity) as usize;
        let audio = unsafe { std::slice::from_raw_parts(buffer_data.data.cast::<u8>(), size) };
        let peak = audio
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| i16::from_le_bytes(*sample).unsigned_abs())
            .max()
            .unwrap_or(0);
        capture
            .peak
            .store((peak as f32 / 32768.).to_bits(), Ordering::Relaxed);
        let mut writer = capture
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if writer.error.is_some() {
            return;
        }
        if size as u32 > MAX_AUDIO_BYTES.saturating_sub(writer.bytes) {
            writer.error = Some("Recording reached the 128 MiB limit".into());
            return;
        }
        match writer.file.write_all(audio) {
            Ok(()) => writer.bytes += size as u32,
            Err(error) => {
                writer.error = Some(error.to_string());
                return;
            }
        }
        if unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, ptr::null()) } != 0 {
            writer.error = Some("Could not queue the next recording buffer".into());
        }
    }
}

#[cfg(target_os = "macos")]
pub use native::{Recorder, request_access};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_header_accounts_for_payload_and_sample_rate() {
        let header = wav_header(SAMPLE_RATE * 2);
        assert_eq!(
            u32::from_le_bytes(header[4..8].try_into().unwrap()),
            SAMPLE_RATE * 2 + 36
        );
        assert_eq!(
            u32::from_le_bytes(header[28..32].try_into().unwrap()),
            SAMPLE_RATE * 2
        );
        assert_eq!(
            u32::from_le_bytes(header[40..44].try_into().unwrap()),
            SAMPLE_RATE * 2
        );
        assert_eq!(&header[8..12], b"WAVE");
    }
}
