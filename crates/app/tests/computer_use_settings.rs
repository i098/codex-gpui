use anyhow::Result;
use app::host::computer_use::{ComputerUseSettings, NativePermissions, SoundMode};
use app::host::{Host, LocalHost, Request};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const STORE: &str = "Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/Library/Application Support/Software/ComputerUseAppApprovals.json";

fn service(home: &Path) -> ComputerUseSettings {
    ComputerUseSettings::new(home, home.join(".codex"))
}

fn write_store(home: &Path, contents: &[u8]) -> Result<()> {
    let path = home.join(STORE);
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, contents)?;
    Ok(())
}

#[test]
fn absent_store_is_unavailable_and_revoke_does_not_create_it() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = service(home.path());
    assert_eq!(settings.get_app_approvals()?, None);
    assert_eq!(settings.remove_app_approval("com.apple.TextEdit")?, None);
    assert!(!home.path().join(STORE).exists());
    Ok(())
}

#[test]
fn revoke_preserves_unrelated_grants_and_unknown_store_fields() -> Result<()> {
    let home = tempfile::tempdir()?;
    write_store(
        home.path(),
        &serde_json::to_vec(&json!({
            "approvedBundleIdentifiers": [" com.apple.TextEdit ", "com.apple.TextEdit", "", "com.apple.Calculator"],
            "futurePolicy": {"revision": 2}
        }))?,
    )?;
    let host = LocalHost::new(home.path().join(".codex"));
    assert!(host.computer_use_settings().is_err());
    let host = host.with_computer_use_settings(service(home.path()));
    let settings = host.computer_use_settings()?;
    assert_eq!(
        settings
            .get_app_approvals()?
            .unwrap()
            .approved_bundle_identifiers,
        ["com.apple.TextEdit", "com.apple.Calculator"]
    );
    assert_eq!(
        settings
            .remove_app_approval(" com.apple.TextEdit ")?
            .unwrap()
            .approved_bundle_identifiers,
        ["com.apple.Calculator"]
    );
    let saved: Value = serde_json::from_slice(&fs::read(home.path().join(STORE))?)?;
    assert_eq!(
        saved,
        json!({
            "approvedBundleIdentifiers": ["com.apple.Calculator"],
            "futurePolicy": {"revision": 2}
        })
    );
    assert!(settings.remove_app_approval(" ").is_err());
    assert!(
        host.fetch(&Request::post(
            "computerUseSettings.removeAppApproval",
            Some(json!({"bundleId":"com.apple.Calculator"}))
        ))
        .is_err()
    );
    assert_eq!(
        settings
            .get_app_approvals()?
            .unwrap()
            .approved_bundle_identifiers,
        ["com.apple.Calculator"]
    );
    Ok(())
}

#[test]
fn corrupt_stores_fail_without_destroying_the_original_bytes() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = service(home.path());
    for invalid in [
        b"not json".as_slice(),
        b"[]",
        b"{}",
        br#"{"approvedBundleIdentifiers":[12]}"#,
    ] {
        write_store(home.path(), invalid)?;
        assert!(settings.get_app_approvals().is_err());
        assert!(settings.remove_app_approval("com.apple.TextEdit").is_err());
        assert_eq!(fs::read(home.path().join(STORE))?, invalid);
    }
    Ok(())
}

#[test]
fn readiness_does_not_confuse_registration_with_native_permissions() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = service(home.path());
    let empty = settings.readiness()?;
    assert!(!empty.relay_installed);
    assert!(!empty.provider_registered);
    let codex = home.path().join(".codex");
    fs::create_dir_all(codex.join("computer-use/lcu/current/adapters"))?;
    fs::write(
        codex.join("computer-use/lcu/current/adapters/codex.mjs"),
        "",
    )?;
    fs::write(
        codex.join("config.toml"),
        "[mcp_servers.lcu]\ncommand = 'node'\n",
    )?;
    let installed = settings.readiness()?;
    assert!(installed.relay_installed);
    assert!(installed.provider_registered);
    assert_eq!(
        installed.native_permissions,
        if cfg!(target_os = "macos") {
            NativePermissions::Unknown
        } else {
            NativePermissions::Unsupported
        }
    );
    fs::write(
        codex.join("config.toml"),
        "[mcp_servers.lcu]\nenabled = false\n",
    )?;
    assert!(!settings.readiness()?.provider_registered);
    fs::write(codex.join("config.toml"), "[")?;
    assert!(settings.readiness().is_err());
    Ok(())
}

#[test]
fn unknown_sound_modes_are_rejected() {
    assert!(serde_json::from_value::<SoundMode>(json!("always")).is_err());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_sound_write_never_claims_success() -> Result<()> {
    let home = tempfile::tempdir()?;
    let settings = service(home.path());
    assert_eq!(settings.get_sound_mode()?, None);
    assert!(settings.set_sound_mode(SoundMode::Off).is_err());
    assert!(!home.path().join("Library/Preferences").exists());
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn sound_modes_round_trip_in_an_owned_native_preferences_domain() -> Result<()> {
    let home = tempfile::tempdir()?;
    fs::create_dir_all(home.path().join("Library/Preferences"))?;
    let settings = service(home.path());
    assert_eq!(settings.get_sound_mode()?, None);
    for mode in [
        SoundMode::ForegroundClicks,
        SoundMode::ForegroundAndBackgroundClicks,
        SoundMode::Off,
    ] {
        assert_eq!(settings.set_sound_mode(mode)?, mode);
        assert_eq!(settings.get_sound_mode()?, Some(mode));
    }
    Ok(())
}
