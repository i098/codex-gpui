use anyhow::Result;
use app::host::{CloudHost, Host, LocalHost, Request, voice::Settings};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::{Path, PathBuf},
    time::Duration,
};
use ui::screens::settings::voice::{Access, Policy, Recording};

fn recording(home: &Path, id: &str, status: &str, created: i64) -> Result<PathBuf> {
    let path = home
        .join("dictation-history")
        .join(format!("{:x}", Sha256::digest(id.as_bytes())));
    fs::create_dir_all(&path)?;
    fs::write(
        path.join("metadata.json"),
        serde_json::to_vec(
            &json!({"id":id,"createdAtMs":created,"mimeType":"audio/webm","sizeBytes":999,"status":status,"text":"","surface":"global","futureField":{"keep":true}}),
        )?,
    )?;
    Ok(path)
}

#[test]
fn preferences_reject_invalid_values_without_losing_confirmed_data() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("preferences.json");
    let settings = Settings::new(home.path().into(), path.clone());
    settings.write("dictationSoundsEnabled", json!(false))?;
    settings.write(
        "dictationDictionary",
        json!([" Jane Doe ", " ", "useCartState"]),
    )?;
    let before = fs::read(&path)?;
    assert!(
        settings
            .write("dictationSoundsEnabled", json!("yes"))
            .is_err()
    );
    assert!(
        settings
            .write("microphoneInputDeviceId", json!(""))
            .is_err()
    );
    assert!(settings.write("unknown", json!(true)).is_err());
    assert_eq!(fs::read(&path)?, before);
    assert_eq!(
        settings.values()?["dictationDictionary"],
        json!(["Jane Doe", "useCartState"])
    );
    Ok(())
}

#[test]
fn recordings_preserve_audio_order_and_never_delete_active_audio() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = Settings::new(home.path().into(), home.path().join("preferences.json"));
    let older = recording(home.path(), "older", "completed", 1)?;
    let active = recording(home.path(), "active", "recording", 2)?;
    fs::write(older.join("0000000002.chunk"), [0, 255, 2])?;
    fs::write(older.join("0000000001.chunk"), [1, 254])?;
    fs::write(older.join("ignored.chunk"), [9, 9])?;
    let rows = settings.recordings()?;
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["active", "older"]
    );
    assert_eq!(rows[1].size_bytes, 5);
    assert_eq!(settings.audio("older")?.1, [1, 254, 0, 255, 2]);
    assert!(settings.delete("active").is_err());
    assert!(active.exists());
    settings.set_transcript("older", "  actual transcript  ")?;
    let value: Value = serde_json::from_slice(&fs::read(older.join("metadata.json"))?)?;
    assert_eq!(value["text"], "actual transcript");
    assert_eq!(value["futureField"], json!({"keep":true}));
    assert!(settings.set_transcript("older", " ").is_err());
    settings.delete("older")?;
    assert!(!older.exists());
    Ok(())
}

#[test]
fn downloads_do_not_overwrite_existing_files() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = Settings::new(home.path().into(), home.path().join("preferences.json"));
    let path = recording(home.path(), "saved", "completed", 0)?;
    fs::write(path.join("0000000000.chunk"), [0, 128, 255])?;
    let downloads = home.path().join("downloads");
    let first = settings.download("saved", &downloads)?;
    let second = settings.download("saved", &downloads)?;
    assert_ne!(first, second);
    assert_eq!(fs::read(first)?, [0, 128, 255]);
    assert_eq!(fs::read(second)?, [0, 128, 255]);
    Ok(())
}

#[cfg(unix)]
#[test]
fn recording_links_cannot_escape_the_owned_store() -> Result<()> {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let settings = Settings::new(home.path().into(), home.path().join("preferences.json"));
    let root = home.path().join("dictation-history");
    fs::create_dir(&root)?;
    symlink(
        outside.path(),
        root.join(format!("{:x}", Sha256::digest(b"escape"))),
    )?;
    assert!(settings.audio("escape").is_err());
    assert!(settings.delete("escape").is_err());
    assert!(outside.path().exists());
    Ok(())
}

#[test]
fn missing_or_disabled_gates_never_enable_voice_or_screen_context() {
    let capabilities =
        json!({"voice.realtime":{"isCapable":true},"dictation.global":{"isCapable":false}});
    let missing = Policy::from_original(&Value::Null, &capabilities, "macOS", true);
    assert!(!missing.realtime_visible);
    assert_eq!(missing.realtime, Access::Denied);
    assert!(!missing.screen_context);
    let enabled = Policy::from_original(&json!({"2380644311":true}), &capabilities, "macOS", true);
    assert_eq!(enabled.realtime, Access::Allowed);
    assert!(enabled.screen_context);
    let killed = Policy::from_original(
        &json!({"2380644311":true,"1697652030":true}),
        &capabilities,
        "macOS",
        true,
    );
    assert_eq!(killed.realtime, Access::Denied);
    assert!(!killed.screen_context);
    let loading = Policy::from_original(
        &json!({"2380644311":true}),
        &json!({"voice.realtime":{"isLoading":true}}),
        "macOS",
        true,
    );
    assert_eq!(loading.realtime, Access::Loading);
    assert!(!loading.screen_context);
}

#[test]
fn retry_requires_saved_audio_without_a_transcript() {
    let mut item = Recording {
        id: "r".into(),
        created_at_ms: 0,
        mime_type: "audio/webm".into(),
        size_bytes: 3,
        status: "interrupted".into(),
        text: String::new(),
        language: None,
    };
    assert!(item.can_retry(true));
    assert!(!item.can_retry(false));
    item.status = "recording".into();
    assert!(!item.can_retry(true));
    item.status = "completed".into();
    item.text = "text".into();
    assert!(!item.can_retry(true));
    item.text.clear();
    item.size_bytes = 0;
    assert!(!item.can_retry(true));
}

#[test]
fn binary_http_bodies_reach_the_server_without_utf8_conversion() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> Result<Vec<u8>> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut received = Vec::new();
        let mut byte = [0];
        while !received.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            received.push(byte[0]);
        }
        let headers = String::from_utf8(received)?;
        let length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()?;
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")?;
        Ok(body)
    });
    let home = tempfile::tempdir()?;
    let host = CloudHost::new(
        LocalHost::new(home.path()),
        Box::new(|_| Ok(None)),
        format!("http://{address}"),
    );
    let audio = [0u8, 128, 255, 13, 10];
    let reply=host.fetch(&Request::Http(json!({"url":"/upload","method":"POST","headers":{"Content-Type":"application/octet-stream"},"bodyBase64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, audio)})))?;
    assert_eq!(app::host::json_body(&reply)?["ok"], true);
    assert_eq!(server.join().unwrap()?, audio);
    Ok(())
}

#[test]
fn legacy_transcripts_stay_visible_and_delete_by_identity() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = Settings::new(home.path().into(), home.path().join("preferences.json"));
    let path = home.path().join("transcription-history.jsonl");
    fs::write(
        &path,
        "{\"id\":\"old\",\"createdAtMs\":1,\"text\":\"old transcript\"}\n{\"id\":\"new\",\"createdAtMs\":2,\"text\":\"new transcript\"}\n",
    )?;
    let rows = settings.recordings()?;
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["new", "old"]
    );
    assert_eq!(rows[0].text, "new transcript");
    assert!(!rows[0].can_retry(true));
    settings.delete("old")?;
    assert_eq!(
        settings
            .recordings()?
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );
    assert!(path.exists());
    Ok(())
}

#[test]
fn oversized_recordings_fail_before_audio_allocation() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = Settings::new(home.path().into(), home.path().join("preferences.json"));
    let path = recording(home.path(), "large", "completed", 0)?;
    fs::File::create(path.join("0000000000.chunk"))?.set_len(129 * 1024 * 1024)?;
    assert!(
        settings
            .audio("large")
            .unwrap_err()
            .to_string()
            .contains("128 MiB")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn legacy_symlinks_are_rejected_before_read_or_delete() -> Result<()> {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir()?;
    let outside = tempfile::NamedTempFile::new()?;
    let original = b"{\"id\":\"private\",\"createdAtMs\":1,\"text\":\"outside\"}\n";
    fs::write(outside.path(), original)?;
    symlink(
        outside.path(),
        home.path().join("transcription-history.jsonl"),
    )?;
    let settings = Settings::new(home.path().into(), home.path().join("preferences.json"));
    assert!(settings.recordings().is_err());
    assert!(settings.delete("private").is_err());
    assert_eq!(fs::read(outside.path())?, original);
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn voice_toggle_persists_only_after_the_native_click() -> Result<()> {
    use gpui::{AppContext as _, BorrowAppContext as _, HeadlessAppContext, Render, px, size};
    use std::sync::Arc;
    use ui::screens::{
        computer_use::Load,
        settings::voice::{VoiceAction, VoiceData, VoiceState},
    };
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    struct Surface;
    impl Render for Surface {
        fn render(
            &mut self,
            window: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            ui::screens::settings::voice::render(window, cx)
        }
    }
    let directory = tempfile::tempdir()?;
    let preferences = directory.path().join("preferences.json");
    let store = Arc::new(Settings::new(directory.path().into(), preferences.clone()));
    let writer = store.clone();
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        let mut data = VoiceData::default();
        data.policy.dictation = true;
        data.policy.microphone_selection = true;
        data.values = json!({"dictationSoundsEnabled": true});
        data.language = Load::Ready("auto".into());
        data.microphones = Load::Ready(Vec::new());
        data.recordings = Load::Ready(Vec::new());
        cx.set_global(VoiceState {
            data: Arc::new(data),
            dispatch: Some(Arc::new(move |action, cx| {
                if let VoiceAction::SetPreference(key, value) = action {
                    writer.write(key, value).unwrap();
                    let values = writer.values().unwrap();
                    cx.update_global::<VoiceState, _>(|state, _| {
                        Arc::make_mut(&mut state.data).values = values
                    });
                }
            })),
            ..Default::default()
        });
    });
    let page = cx.update(|cx| cx.new(|_| Surface));
    let window = cx.open_window(size(px(840.), px(650.)), |_, _| page.clone())?;
    cx.run_until_parked();
    assert!(!preferences.exists());
    if let Some(path) = std::env::var_os("CGX_VOICE_SHOT") {
        cx.capture_screenshot(window.into())?
            .save(PathBuf::from(path).with_extension("before.png"))?;
    }
    cx.update_window(window.into(), |_, window, cx| {
        let position = gpui::point(px(750.), px(396.));
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }),
            cx,
        );
    })?;
    cx.run_until_parked();
    assert_eq!(store.values()?["dictationSoundsEnabled"], false);
    cx.advance_clock(Duration::from_millis(200));
    cx.test_window(window.into())
        .simulate_frame_request(gpui::RequestFrameOptions {
            require_presentation: true,
            ..Default::default()
        });
    cx.run_until_parked();
    if let Some(path) = std::env::var_os("CGX_VOICE_SHOT") {
        cx.capture_screenshot(window.into())?.save(path)?;
    }
    Ok(())
}
