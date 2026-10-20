use anyhow::Result;
#[cfg(not(target_os = "macos"))]
use app::host::computer_use_native::NativeComputerUse;
use app::host::computer_use_native::{Appshot, LockedUsePolicy, LockedUseState, capture_update};
use serde_json::json;
use std::fs;

#[test]
fn locked_use_requires_every_account_and_feature_gate() {
    let allowed = LockedUsePolicy {
        locked_use_enabled: true,
        slingshot_enabled: true,
        ..LockedUsePolicy::default()
    };
    assert!(allowed.allowed());
    let mut cases = vec![allowed.clone(); 5];
    cases[0].locked_use_enabled = false;
    cases[1].slingshot_enabled = false;
    cases[2].remote_control_available = Some(false);
    cases[3].remote_control_access_required = true;
    cases[4].allow_locked_computer_use = Some(false);
    for policy in cases {
        assert!(!policy.allowed());
        assert!(
            !LockedUseState {
                enabled: Some(true),
                ..Default::default()
            }
            .visible(&policy)
        );
    }
    assert!(!LockedUseState::default().visible(&allowed));
    assert!(
        LockedUseState {
            enabled: Some(false),
            ..Default::default()
        }
        .visible(&allowed)
    );
}

#[test]
fn captures_do_not_accept_another_request_or_updates_after_completion() {
    let mut capture = Appshot::new("capture-a".into());
    let update = json!({"type":"axText", "text":"owned content"});
    assert!(capture.update("capture-b", &update).is_err());
    assert_eq!(capture.ax_text, None);
    capture.update("capture-a", &update).unwrap();
    capture
        .update("capture-a", &json!({"type":"completed"}))
        .unwrap();
    assert!(
        capture
            .update("capture-a", &json!({"type":"axText", "text":"stale"}))
            .is_err()
    );
    assert_eq!(capture.ax_text.as_deref(), Some("owned content"));
}

#[test]
fn cancelled_and_failed_captures_are_terminal() {
    let mut capture = Appshot::new("capture".into());
    capture
        .update("capture", &json!({"type":"failed", "failureReason":" "}))
        .unwrap();
    assert_eq!(
        capture.failure_reason.as_deref(),
        Some("native_service_failed")
    );
    assert!(capture.completed);
    let mut cancelled = Appshot::new("cancelled".into());
    cancelled.cancel();
    assert!(
        cancelled
            .update("cancelled", &json!({"type":"completed"}))
            .is_err()
    );
}

#[test]
fn capture_images_stay_inside_the_helper_directory() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("com.openai.sky.CUAService");
    fs::create_dir(&root)?;
    let image = root.join("capture.png");
    fs::write(&image, b"png payload")?;
    let update = capture_update(
        &root,
        json!({"type":"screenshot", "screenshotURL":url::Url::from_file_path(&image).unwrap().to_string()}),
    )?;
    assert_eq!(
        update["screenshotPath"],
        image.canonicalize()?.to_str().unwrap()
    );
    assert_eq!(
        update["screenshotDataURL"],
        "data:image/png;base64,cG5nIHBheWxvYWQ="
    );
    let outside = directory.path().join("secret.png");
    fs::write(&outside, b"private")?;
    for source in [
        outside.to_str().unwrap(),
        "https://example.com/image.png",
        "relative.png",
    ] {
        assert!(
            capture_update(&root, json!({"type":"screenshot", "screenshotURL":source})).is_err()
        );
    }
    #[cfg(unix)]
    {
        let link = root.join("link.png");
        std::os::unix::fs::symlink(outside, &link)?;
        assert!(capture_update(&root, json!({"type":"screenshot", "screenshotURL":link})).is_err());
    }
    Ok(())
}

#[test]
fn oversized_or_unsupported_capture_files_do_not_reach_the_ui() -> Result<()> {
    let root = tempfile::tempdir()?;
    let huge = root.path().join("huge.png");
    fs::File::create(&huge)?.set_len(25 * 1024 * 1024 + 1)?;
    assert!(
        capture_update(
            root.path(),
            json!({"type":"screenshot", "screenshotURL":huge})
        )
        .is_err()
    );
    let text = root.path().join("text.txt");
    fs::write(&text, "secret")?;
    assert!(
        capture_update(
            root.path(),
            json!({"type":"completed", "transitionSnapshotURL":text})
        )
        .is_err()
    );
    assert!(capture_update(root.path(), json!({"type":"new-protocol-kind"})).is_err());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_hosts_never_start_or_install_a_helper() -> Result<()> {
    let home = tempfile::tempdir()?;
    let native = NativeComputerUse::new(home.path());
    assert!(native.install_helper(home.path()).is_err());
    assert!(native.enable().is_err());
    assert!(native.request_permissions("fixture.app").is_err());
    assert!(
        native
            .set_locked_use_enabled(true, &LockedUsePolicy::default())
            .is_err()
    );
    assert!(native.locked_use_state()?.enabled.is_none());
    assert!(!home.path().join("computer-use").exists());
    Ok(())
}
