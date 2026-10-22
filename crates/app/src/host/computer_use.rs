//! The original `computerUseSettings` service, called only by trusted native UI code.
//! No method is exposed through model tools or the generic host fetch dispatcher.
//!
//! Codex 26.1007.30912 stores helper grants in `ComputerUseAppApprovals.json`.
//! Sound uses the `com.openai.sky.CUAService` preference domain.
//! A supplied fixture home keeps both stores separate from the user's stores.
//! App rows use the original bundle-identifier fallback when native metadata is unavailable.

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

const APPROVALS: &str = "Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/Library/Application Support/Software/ComputerUseAppApprovals.json";
const SOUND_DOMAIN: &str = "Library/Preferences/com.openai.sky.CUAService";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppApproval {
    pub bundle_identifier: String,
    pub display_name: String,
    #[serde(rename = "iconDataURL")]
    pub icon_data_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppApprovals {
    pub approved_apps: Vec<AppApproval>,
    pub approved_bundle_identifiers: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SoundMode {
    ForegroundClicks,
    ForegroundAndBackgroundClicks,
    Off,
}

impl SoundMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ForegroundClicks => "foregroundClicks",
            Self::ForegroundAndBackgroundClicks => "foregroundAndBackgroundClicks",
            Self::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativePermissions {
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub supported: bool,
    pub relay_installed: bool,
    pub provider_registered: bool,
    pub native_permissions: NativePermissions,
}

pub struct ComputerUseSettings {
    home: PathBuf,
    codex_home: PathBuf,
}

impl ComputerUseSettings {
    /// Supply an owned home directory for fixtures; production uses the account's real home.
    pub fn new(home: impl Into<PathBuf>, codex_home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            codex_home: codex_home.into(),
        }
    }

    /// Implements `getAppApprovals`. An absent helper store returns `None`, not an empty grant list.
    pub fn get_app_approvals(&self) -> Result<Option<AppApprovals>> {
        let Some((_, document)) = self.read_approvals()? else {
            return Ok(None);
        };
        approvals(&document).map(Some)
    }

    /// Implements `removeAppApproval(bundleId)` for a confirmed native Settings action.
    /// This removes grants only. New grants still require the original helper approval flow.
    pub fn remove_app_approval(&self, bundle_id: &str) -> Result<Option<AppApprovals>> {
        let bundle_id = bundle_id.trim();
        ensure!(!bundle_id.is_empty(), "missing app bundle identifier");
        let path = self.home.join(APPROVALS);
        if !path.try_exists()? {
            return Ok(None);
        }
        let parent = path.parent().context("app approval store has no parent")?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(parent.join(".codex-gpui-app-approvals.lock"))?;
        lock.lock()?;
        let Some((before, mut document)) = self.read_approvals()? else {
            return Ok(None);
        };
        let mut current = approvals(&document)?;
        let count = current.approved_bundle_identifiers.len();
        current
            .approved_bundle_identifiers
            .retain(|id| id != bundle_id);
        if count == current.approved_bundle_identifiers.len() {
            return Ok(Some(current));
        }
        document["approvedBundleIdentifiers"] =
            serde_json::to_value(&current.approved_bundle_identifiers)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, &document)?;
        temporary
            .as_file()
            .set_permissions(fs::metadata(&path)?.permissions())?;
        temporary.as_file().sync_all()?;
        // The helper does not share our lock. Refuse an observed concurrent update instead of losing it.
        ensure!(
            fs::read(&path)? == before,
            "app approvals changed; reload Settings and retry"
        );
        temporary.persist(&path)?;
        self.get_app_approvals()
    }

    /// Implements `getSoundMode` against the original preferences domain.
    pub fn get_sound_mode(&self) -> Result<Option<SoundMode>> {
        if !cfg!(target_os = "macos") {
            return Ok(None);
        }
        let result = std::process::Command::new("/usr/bin/defaults")
            .arg("read")
            .arg(self.home.canonicalize()?.join(SOUND_DOMAIN))
            .arg("computerUseSoundMode")
            .output()
            .context("read Computer Use sound mode")?;
        if !result.status.success() {
            let error = String::from_utf8_lossy(&result.stderr);
            if error.contains("does not exist") {
                return Ok(None);
            }
            bail!("read Computer Use sound mode: {error}");
        }
        let mode = String::from_utf8(result.stdout)?;
        serde_json::from_value(Value::String(mode.trim().to_owned()))
            .context("invalid Computer Use sound mode")
            .map(Some)
    }

    /// Implements `setSoundMode(mode)`. Unsupported systems return an error rather than pretend to save.
    pub fn set_sound_mode(&self, mode: SoundMode) -> Result<SoundMode> {
        ensure!(
            cfg!(target_os = "macos"),
            "Computer Use sound requires macOS"
        );
        let result = std::process::Command::new("/usr/bin/defaults")
            .arg("write")
            .arg(self.home.canonicalize()?.join(SOUND_DOMAIN))
            .args(["computerUseSoundMode", "-string", mode.as_str()])
            .output()
            .context("write Computer Use sound mode")?;
        ensure!(
            result.status.success(),
            "write Computer Use sound mode: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(mode)
    }

    /// Installation and registration are observations, not proof of Accessibility or Screen Recording access.
    pub fn readiness(&self) -> Result<Readiness> {
        let config = self.codex_home.join("config.toml");
        let config: toml::Value = match fs::read_to_string(config) {
            Ok(text) => toml::from_str(&text).context("invalid Codex config")?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                toml::Value::Table(Default::default())
            }
            Err(error) => return Err(error).context("read Codex config"),
        };
        let provider = config
            .get("mcp_servers")
            .and_then(|servers| servers.get("lcu"));
        let supported = cfg!(target_os = "macos");
        Ok(Readiness {
            supported,
            relay_installed: self
                .codex_home
                .join("computer-use/lcu/current/adapters/codex.mjs")
                .is_file(),
            provider_registered: provider.is_some_and(|server| {
                server
                    .get("command")
                    .and_then(toml::Value::as_str)
                    .is_some_and(|command| !command.is_empty())
                    && server.get("enabled").and_then(toml::Value::as_bool) != Some(false)
            }),
            native_permissions: if supported {
                NativePermissions::Unknown
            } else {
                NativePermissions::Unsupported
            },
        })
    }

    fn read_approvals(&self) -> Result<Option<(Vec<u8>, Value)>> {
        let bytes = match fs::read(self.home.join(APPROVALS)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read app approvals"),
        };
        let document = serde_json::from_slice(&bytes).context("invalid app approvals JSON")?;
        Ok(Some((bytes, document)))
    }
}

fn approvals(document: &Value) -> Result<AppApprovals> {
    ensure!(document.is_object(), "app approvals must be an object");
    let ids = document
        .get("approvedBundleIdentifiers")
        .and_then(Value::as_array)
        .context("invalid approvedBundleIdentifiers")?;
    let mut normalized = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id
            .as_str()
            .context("app bundle identifier must be a string")?
            .trim();
        if !id.is_empty() && !normalized.iter().any(|existing| existing == id) {
            normalized.push(id.to_owned());
        }
    }
    Ok(AppApprovals {
        approved_apps: normalized
            .iter()
            .map(|id| AppApproval {
                bundle_identifier: id.clone(),
                display_name: id.clone(),
                icon_data_url: None,
            })
            .collect(),
        approved_bundle_identifiers: normalized,
    })
}
