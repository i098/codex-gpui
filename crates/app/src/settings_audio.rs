use crate::{Root, SettingChanged};
use gpui::Context;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

pub(super) const SETTING: &str = "host.toys-audio-visualizer-enabled";

pub(super) struct Capture {
    child: Option<Child>,
    pub ready: bool,
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    Ready,
    Levels([f32; 24]),
    Failed(bool),
}

fn decode(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value["type"].as_str()? {
        "ready"
            if value["sampleRate"]
                .as_f64()
                .is_some_and(|rate| rate.is_finite() && rate > 0.) =>
        {
            Some(Event::Ready)
        }
        "levels" => {
            let values = value["levels"].as_array()?;
            if values.len() != 24 {
                return None;
            }
            let mut levels = [0.; 24];
            for (level, value) in levels.iter_mut().zip(values) {
                let value = value.as_f64()?;
                if !value.is_finite() || !(0. ..=1.).contains(&value) {
                    return None;
                }
                *level = value as f32;
            }
            Some(Event::Levels(levels))
        }
        "error" => Some(Event::Failed(value["operation"] == "permission")),
        _ => None,
    }
}

fn helper() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let bundled = std::env::current_exe()
        .ok()?
        .parent()?
        .parent()?
        .join("Resources/native/system-audio-spectrum");
    if bundled.is_file() {
        return Some(bundled);
    }
    let original =
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/native/system-audio-spectrum");
    original.is_file().then_some(original)
}

pub(super) fn available() -> bool {
    if helper().is_none() {
        return false;
    }
    static SUPPORTED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        let Ok(output) = Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
        else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        let version = String::from_utf8_lossy(&output.stdout);
        let mut parts = version.trim().split('.');
        let major = parts
            .next()
            .and_then(|part| part.parse::<u32>().ok())
            .unwrap_or(0);
        let minor = parts
            .next()
            .and_then(|part| part.parse::<u32>().ok())
            .unwrap_or(0);
        major > 14 || major == 14 && minor >= 2
    });
    *SUPPORTED
}

fn start() -> anyhow::Result<(Capture, async_channel::Receiver<Event>)> {
    use anyhow::Context as _;
    anyhow::ensure!(available(), "System audio capture is unavailable");
    let mut child = Command::new(helper().context("System audio capture is unavailable")?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let output = child.stdout.take().context("Audio helper has no output")?;
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(count) if count > 0 && count <= 4096 => {}
                _ => break,
            }
            let event = decode(&line).unwrap_or(Event::Failed(false));
            let terminal = matches!(event, Event::Failed(_));
            match event {
                Event::Levels(_) => {
                    let _ = sender.try_send(event);
                }
                _ => {
                    if sender.send_blocking(event).is_err() {
                        return;
                    }
                }
            }
            if terminal {
                return;
            }
        }
        let _ = sender.send_blocking(Event::Failed(false));
    });
    Ok((
        Capture {
            child: Some(child),
            ready: false,
        },
        receiver,
    ))
}

impl Root {
    pub(super) fn start_settings_audio(&mut self, cx: &mut Context<Self>) {
        self.audio_generation = self.audio_generation.wrapping_add(1);
        let generation = self.audio_generation;
        if let Some(page) = &self.settings_page {
            page.update(cx, |page, cx| page.audio_pending(cx));
        }
        let (capture, events) = match start() {
            Ok(started) => started,
            Err(_) => {
                self.settings_audio_failed(false, cx);
                return;
            }
        };
        self.audio_capture = Some(capture);
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let active = this
                    .update(cx, |this, cx| {
                        if this.audio_generation != generation {
                            return false;
                        }
                        match event {
                            Event::Ready => {
                                if let Some(capture) = &mut this.audio_capture {
                                    capture.ready = true;
                                }
                                this.write_setting(
                                    &SettingChanged {
                                        path: SETTING,
                                        value: json!(true),
                                    },
                                    cx,
                                );
                                this.audio_capture = None;
                                this.audio_generation = this.audio_generation.wrapping_add(1);
                                return false;
                            }
                            Event::Levels(_) => {}
                            Event::Failed(permission) => {
                                this.settings_audio_failed(permission, cx);
                                return false;
                            }
                        }
                        true
                    })
                    .unwrap_or(false);
                if !active {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.audio_generation == generation
                    && this
                        .audio_capture
                        .as_ref()
                        .is_some_and(|capture| !capture.ready)
                {
                    this.settings_audio_failed(false, cx);
                }
            });
        })
        .detach();
    }

    fn settings_audio_failed(&mut self, permission: bool, cx: &mut Context<Self>) {
        self.audio_generation = self.audio_generation.wrapping_add(1);
        self.audio_capture = None;
        if let Some(page) = &self.settings_page {
            page.update(cx, |page, cx| {
                page.finish_write(SETTING, json!(false), true, cx)
            });
        }
        let toast = ui::toast::Toast::info(if permission {
            "Allow system audio recording in System Settings to use the visualizer"
        } else {
            "Couldn’t start the visualizer"
        });
        self.toast(if permission {
            toast.action("Open Settings", |_, cx| cx.open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"))
        } else { toast }, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_protocol_rejects_invalid_frames_and_identifies_permission_denial() {
        assert_eq!(
            decode(r#"{"type":"ready","sampleRate":48000}"#),
            Some(Event::Ready)
        );
        assert_eq!(
            decode(r#"{"type":"error","operation":"permission"}"#),
            Some(Event::Failed(true))
        );
        assert_eq!(decode(r#"{"type":"levels","levels":[1]}"#), None);
        assert_eq!(
            decode(&json!({"type":"levels","levels":vec![2.;24]}).to_string()),
            None
        );
        assert_eq!(
            decode(&json!({"type":"levels","levels":vec![0.5;24]}).to_string()),
            Some(Event::Levels([0.5; 24]))
        );
    }
}
