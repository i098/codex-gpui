//! Shared by the Linux and Windows updaters: read the signed release manifest, download the
//! package, and check its SHA-256 and Ed25519 signature before anyone installs it.
use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
    time::Duration,
};

const MAX_MANIFEST: u64 = 1 << 20;

/// `latest.json`: the Ed25519 signature covers the raw 32-byte SHA-256 of the package.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signature: String,
}

pub struct Trust {
    pub feed: String,
    pub key: VerifyingKey,
}

impl Trust {
    /// Build-time `CODEX_GPUI_UPDATE_FEED` and `CODEX_GPUI_UPDATE_PUBLIC_KEY` (the names
    /// `tools/bundle.sh` uses); the process environment wins so a local build can be pointed
    /// at a test feed. Returns `None` when no feed is configured.
    pub fn configured() -> Result<Option<Self>> {
        let pick = |name: &str, built: Option<&str>| {
            std::env::var(name)
                .ok()
                .or_else(|| built.map(str::to_owned))
                .filter(|value| !value.is_empty())
        };
        let feed = pick(
            "CODEX_GPUI_UPDATE_FEED",
            option_env!("CODEX_GPUI_UPDATE_FEED"),
        );
        let key = pick(
            "CODEX_GPUI_UPDATE_PUBLIC_KEY",
            option_env!("CODEX_GPUI_UPDATE_PUBLIC_KEY"),
        );
        match (feed, key) {
            (None, None) => Ok(None),
            (Some(feed), Some(key)) => {
                require_https(&feed)?;
                Ok(Some(Self {
                    feed,
                    key: parse_key(&key)?,
                }))
            }
            _ => bail!("an update feed needs both a feed URL and a public key"),
        }
    }
}

fn parse_key(encoded: &str) -> Result<VerifyingKey> {
    let bytes: [u8; 32] = STANDARD
        .decode(encoded)
        .context("invalid update public key")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("the update public key must contain 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).context("invalid update public key")
}

/// Plain HTTP is only for a feed on this machine.
fn require_https(url: &str) -> Result<()> {
    let url = url::Url::parse(url).context("invalid update URL")?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "update URLs require HTTPS"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "update URLs must not contain credentials"
    );
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .into()
}

/// The newest release when it is newer than `current`.
pub fn check(trust: &Trust, current: &str) -> Result<Option<Manifest>> {
    let body = agent()
        .get(&trust.feed)
        .call()
        .context("cannot read the update feed")?
        .into_body()
        .with_config()
        .limit(MAX_MANIFEST)
        .read_to_vec()
        .context("cannot read the update feed")?;
    let manifest: Manifest = serde_json::from_slice(&body).context("invalid update feed")?;
    let latest = semver::Version::parse(&manifest.version).context("invalid feed version")?;
    if latest <= semver::Version::parse(current).context("invalid app version")? {
        return Ok(None);
    }
    // A package URL is only followed over HTTPS (or localhost), whatever the feed says.
    require_https(&manifest.url)?;
    Ok(Some(manifest))
}

/// Download to `dest`, report percent, then verify the hash and signature. `dest` is removed
/// when anything fails.
pub fn download(
    trust: &Trust,
    manifest: &Manifest,
    dest: &Path,
    mut progress: impl FnMut(Option<u8>),
) -> Result<()> {
    let result = fetch(trust, manifest, dest, &mut progress);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn fetch(
    trust: &Trust,
    manifest: &Manifest,
    dest: &Path,
    progress: &mut impl FnMut(Option<u8>),
) -> Result<()> {
    let response = agent()
        .get(&manifest.url)
        .call()
        .context("update download failed")?;
    let total = response.body().content_length();
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(dest).context("cannot stage the update")?;
    let (mut hash, mut done, mut last) = (Sha256::new(), 0u64, None);
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = reader.read(&mut buffer).context("update download failed")?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .context("cannot stage the update")?;
        done += read as u64;
        let percent = total
            .filter(|total| *total > 0)
            .map(|total| (done * 100 / total).min(100) as u8);
        if percent != last {
            last = percent;
            progress(percent);
        }
    }
    file.sync_all().context("cannot stage the update")?;
    verify(&trust.key, manifest, &hash.finalize())
}

fn verify(key: &VerifyingKey, manifest: &Manifest, digest: &[u8]) -> Result<()> {
    ensure!(
        manifest.sha256.eq_ignore_ascii_case(&hex(digest)),
        "the downloaded update does not match its checksum"
    );
    let signature: [u8; 64] = STANDARD
        .decode(&manifest.signature)
        .context("invalid update signature")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid update signature"))?;
    key.verify(digest, &Signature::from_bytes(&signature))
        .context("the update signature is not valid")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn signed(payload: &[u8], signer: &SigningKey) -> (Manifest, Vec<u8>) {
        let digest = Sha256::digest(payload).to_vec();
        let manifest = Manifest {
            version: "1.2.3".into(),
            url: "https://example.test/app".into(),
            sha256: hex(&digest),
            signature: STANDARD.encode(signer.sign(&digest).to_bytes()),
        };
        (manifest, digest)
    }

    #[test]
    fn only_a_matching_hash_and_signature_from_the_trusted_key_pass() {
        let signer = SigningKey::from_bytes(&[7; 32]);
        let other = SigningKey::from_bytes(&[8; 32]);
        let (manifest, digest) = signed(b"package", &signer);
        assert!(verify(&signer.verifying_key(), &manifest, &digest).is_ok());
        assert!(verify(&other.verifying_key(), &manifest, &digest).is_err());
        let tampered = Sha256::digest(b"tampered");
        assert!(verify(&signer.verifying_key(), &manifest, &tampered).is_err());
        let forged = Manifest {
            signature: STANDARD.encode(other.sign(&digest).to_bytes()),
            ..manifest
        };
        assert!(verify(&signer.verifying_key(), &forged, &digest).is_err());
    }

    #[test]
    fn update_urls_must_be_https_or_local() {
        assert!(require_https("https://updates.example/latest.json").is_ok());
        assert!(require_https("http://127.0.0.1:9/latest.json").is_ok());
        for url in [
            "http://updates.example/latest.json",
            "file:///tmp/x",
            "https://u:p@updates.example/x",
        ] {
            assert!(require_https(url).is_err());
        }
    }
}
