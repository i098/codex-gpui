use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead as _, Read as _, Seek as _};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const RUNTIME_DIRECTORY: &str = "codex-primary-runtime";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    archive_url: String,
    archive_sha256: String,
    bundle_version: String,
    #[serde(default = "runtime_directory")]
    runtime_root_directory_name: String,
    target_arch: String,
    target_platform: String,
}

fn runtime_directory() -> String {
    RUNTIME_DIRECTORY.into()
}

pub(super) struct Dependencies {
    cache: PathBuf,
    child: Mutex<Option<Child>>,
    busy: AtomicBool,
    cancelled: AtomicBool,
    committing: AtomicBool,
}

impl Dependencies {
    pub(super) fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            child: Mutex::new(None),
            busy: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            committing: AtomicBool::new(false),
        }
    }

    pub(super) fn state(&self, probe: bool) -> Result<Value> {
        if probe {
            self.exclusive(|| self.inspect(&self.cache.join(RUNTIME_DIRECTORY), true))
        } else {
            self.inspect(&self.cache.join(RUNTIME_DIRECTORY), false)
        }
    }

    pub(super) fn cancel(&self) -> Result<Value> {
        let mut process = self
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("dependency process lock poisoned"))?;
        if !self.busy.load(Ordering::SeqCst) || self.committing.load(Ordering::SeqCst) {
            return Ok(json!({"canceled": false}));
        }
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = process.as_mut() {
            let _ = child.kill();
        }
        Ok(json!({"canceled": true}))
    }

    pub(super) fn reinstall(&self) -> Result<Value> {
        let (platform, arch) = target()?;
        let url = format!(
            "https://persistent.oaistatic.com/{RUNTIME_DIRECTORY}/latest/{platform}-{arch}/LATEST.json"
        );
        self.exclusive(|| self.install(&url, platform, arch))
    }

    fn exclusive(&self, operation: impl FnOnce() -> Result<Value>) -> Result<Value> {
        let process = self
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("dependency process lock poisoned"))?;
        ensure!(
            !self.busy.swap(true, Ordering::SeqCst),
            "workspace dependencies are busy"
        );
        self.cancelled.store(false, Ordering::SeqCst);
        self.committing.store(false, Ordering::SeqCst);
        drop(process);
        let result = operation();
        self.busy.store(false, Ordering::SeqCst);
        self.committing.store(false, Ordering::SeqCst);
        result
    }

    fn check_cancelled(&self) -> Result<()> {
        ensure!(
            !self.cancelled.load(Ordering::SeqCst),
            "workspace dependency download canceled"
        );
        Ok(())
    }

    fn install(&self, url: &str, platform: &str, arch: &str) -> Result<Value> {
        ensure!(
            self.cache.is_absolute(),
            "workspace dependency cache must be absolute"
        );
        fs::create_dir_all(&self.cache)?;
        let staging = tempfile::Builder::new()
            .prefix("codex-runtime-install-")
            .tempdir_in(&self.cache)?;
        let manifest_path = staging.path().join("manifest.json");
        self.download(url, &manifest_path, Some(1024 * 1024))?;
        let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)
            .context("invalid workspace dependency manifest")?;
        ensure!(
            manifest.target_arch == arch && manifest.target_platform == platform,
            "workspace dependency manifest targets another platform"
        );
        ensure!(
            manifest.runtime_root_directory_name == RUNTIME_DIRECTORY,
            "unexpected workspace dependency directory"
        );
        ensure!(
            manifest.archive_sha256.len() == 64
                && manifest
                    .archive_sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit()),
            "invalid workspace dependency checksum"
        );
        let archive = staging.path().join("runtime.archive");
        self.download(&manifest.archive_url, &archive, None)?;
        ensure!(
            digest(&archive)? == manifest.archive_sha256.to_ascii_lowercase(),
            "workspace dependency checksum mismatch"
        );
        self.check_cancelled()?;
        let entries = staging.path().join("entries.txt");
        self.run(
            Command::new("tar")
                .env_remove("TAR_OPTIONS")
                .arg("-tf")
                .arg(&archive),
            Some(&entries),
            Duration::from_secs(60),
        )?;
        for line in std::io::BufReader::new(File::open(&entries)?).lines() {
            validate_entry(&line?)?;
        }
        let payload = staging.path().join("payload");
        fs::create_dir(&payload)?;
        self.run(
            Command::new("tar")
                .env_remove("TAR_OPTIONS")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&payload),
            None,
            Duration::from_secs(300),
        )?;
        let candidate = payload.join(RUNTIME_DIRECTORY);
        let state = self.inspect(&candidate, false)?;
        ensure!(
            state["installed"] == true,
            "workspace dependency archive is incomplete: {}",
            state["problems"]
        );
        ensure!(
            state["bundleVersion"] == manifest.bundle_version,
            "workspace dependency bundle version mismatch"
        );
        self.check_cancelled()?;
        self.committing.store(true, Ordering::SeqCst);
        self.check_cancelled()?;
        let destination = self.cache.join(RUNTIME_DIRECTORY);
        let backup = staging.path().join("previous");
        let had_previous = destination.try_exists()? || destination.is_symlink();
        if had_previous {
            fs::rename(&destination, &backup)
                .context("preserve previous workspace dependencies")?;
        }
        if let Err(error) = fs::rename(&candidate, &destination) {
            if had_previous {
                fs::rename(&backup, &destination)
                    .context("restore previous workspace dependencies")?;
            }
            return Err(error).context("install workspace dependencies");
        }
        self.inspect(&destination, false)
    }

    fn download(&self, url: &str, path: &Path, limit: Option<u64>) -> Result<()> {
        let url = url::Url::parse(url).context("invalid workspace dependency URL")?;
        ensure!(
            url.scheme() == "https"
                || (url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))),
            "workspace dependencies require HTTPS"
        );
        let mut command = Command::new("curl");
        command
            .args([
                "--disable",
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--max-redirs",
                "5",
                "--proto-redir",
                "=https",
                "--user-agent",
                "codex-primary-runtime-installer",
                "--output",
            ])
            .arg(path);
        if let Some(limit) = limit {
            command.arg("--max-filesize").arg(limit.to_string());
        }
        command.arg("--").arg(url.as_str());
        self.run(&mut command, None, Duration::from_secs(1800))
    }

    fn run(&self, command: &mut Command, stdout: Option<&Path>, timeout: Duration) -> Result<()> {
        self.check_cancelled()?;
        let mut errors = tempfile::NamedTempFile::new()?;
        command.stdin(Stdio::null()).stderr(errors.reopen()?);
        match stdout {
            Some(path) => {
                command.stdout(File::create(path)?);
            }
            None => {
                command.stdout(Stdio::null());
            }
        }
        let child = command
            .spawn()
            .with_context(|| format!("start {}", command.get_program().to_string_lossy()))?;
        *self
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("dependency process lock poisoned"))? = Some(child);
        let started = Instant::now();
        let result = (|| {
            loop {
                self.check_cancelled()?;
                ensure!(
                    started.elapsed() < timeout,
                    "workspace dependency command timed out"
                );
                let status = self
                    .child
                    .lock()
                    .map_err(|_| anyhow::anyhow!("dependency process lock poisoned"))?
                    .as_mut()
                    .context("workspace dependency process disappeared")?
                    .try_wait()?;
                if let Some(status) = status {
                    self.check_cancelled()?;
                    errors.as_file_mut().rewind()?;
                    let mut detail = String::new();
                    errors
                        .as_file_mut()
                        .take(8192)
                        .read_to_string(&mut detail)?;
                    ensure!(
                        status.success(),
                        "workspace dependency command failed ({status}): {}",
                        detail.trim()
                    );
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        })();
        if let Some(mut child) = self
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("dependency process lock poisoned"))?
            .take()
        {
            if result.is_err() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        result
    }

    fn inspect(&self, root: &Path, probe: bool) -> Result<Value> {
        ensure!(
            root.is_absolute(),
            "workspace dependency cache must be absolute"
        );
        let metadata = match fs::read(root.join("runtime.json")) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .context("invalid installed workspace dependency metadata")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(
                    json!({"installed": false, "bundleVersion": null, "artifactToolVersion": null,
                    "problems": [if root.exists() { "Codex dependency bundle metadata is missing" } else { "Codex dependencies are not installed" }]}),
                );
            }
            Err(error) => return Err(error).context("read workspace dependency metadata"),
        };
        let modern = metadata["bundleFormatVersion"].as_u64().unwrap_or(1) >= 2;
        let node_root = if modern {
            root.join("dependencies/node")
        } else {
            root.to_owned()
        };
        let python_root = root.join(if modern {
            "dependencies/python"
        } else {
            "python"
        });
        let node = node_root.join("bin/node");
        let python3 = python_root.join("bin/python3");
        let python = if python3.is_file() {
            python3
        } else {
            python_root.join("bin/python")
        };
        let mut problems = Vec::new();
        for (name, path) in [("node", &node), ("python", &python)] {
            if !path.is_file() {
                problems.push(format!("Missing {name} executable: {}", path.display()));
                continue;
            }
            if probe {
                let mut command = Command::new(path);
                if name == "node" {
                    command.args([
                        "--input-type=commonjs",
                        "-e",
                        "require('node:crypto').createHash('sha256').update('codex').digest('hex')",
                    ]);
                } else {
                    command.args(["-I", "-B", "-c", "import sys, ssl, sqlite3, openpyxl"]);
                }
                command
                    .current_dir(root)
                    .env("ELECTRON_RUN_AS_NODE", "1")
                    .env("NODE_OPTIONS", "")
                    .env("NODE_PATH", "");
                if let Err(error) = self.run(&mut command, None, Duration::from_secs(10)) {
                    problems.push(format!(
                        "Bundled {name} failed its execution check: {error}"
                    ));
                }
            }
        }
        if !node_root.join("node_modules").is_dir() {
            problems.push("Missing bundled node packages directory".to_owned());
        }
        Ok(
            json!({"installed": problems.is_empty(), "bundleVersion": metadata["bundleVersion"],
            "artifactToolVersion": metadata["artifactToolVersion"], "libreOfficeVersion": metadata["libreOfficeVersion"], "problems": problems}),
        )
    }
}

fn target() -> Result<(&'static str, &'static str)> {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        _ => bail!("workspace dependencies are unsupported on this platform"),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => bail!("workspace dependencies are unsupported on this architecture"),
    };
    Ok((platform, arch))
}

fn validate_entry(entry: &str) -> Result<()> {
    let mut components = Path::new(entry)
        .components()
        .filter(|c| *c != Component::CurDir);
    ensure!(
        components.next() == Some(Component::Normal(RUNTIME_DIRECTORY.as_ref())),
        "workspace archive has an unexpected root"
    );
    ensure!(
        components.all(|c| matches!(c, Component::Normal(_))),
        "workspace archive path escapes its root"
    );
    Ok(())
}

fn digest(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_entries_stay_inside_the_owned_runtime() {
        assert!(validate_entry("codex-primary-runtime/dependencies/node/bin/node").is_ok());
        for bad in [
            "../outside",
            "/codex-primary-runtime/runtime.json",
            "codex-primary-runtime/../../outside",
            "other/runtime.json",
        ] {
            assert!(validate_entry(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn missing_or_incomplete_runtime_is_not_healthy() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let dependencies = Dependencies::new(directory.path().to_owned());
        assert_eq!(dependencies.state(false)?["installed"], false);
        let root = directory.path().join(RUNTIME_DIRECTORY);
        fs::create_dir(&root)?;
        fs::write(
            root.join("runtime.json"),
            br#"{"bundleFormatVersion":2,"bundleVersion":"fixture"}"#,
        )?;
        let state = dependencies.state(false)?;
        assert_eq!(state["installed"], false);
        assert_eq!(state["bundleVersion"], "fixture");
        let problems = state["problems"].as_array().unwrap();
        assert!(problems.iter().any(|problem| {
            problem
                .as_str()
                .is_some_and(|text| text.starts_with("Missing node executable:"))
        }));
        assert!(problems.iter().any(|problem| {
            problem
                .as_str()
                .is_some_and(|text| text.starts_with("Missing python executable:"))
        }));
        assert!(
            problems
                .iter()
                .any(|problem| problem == "Missing bundled node packages directory")
        );
        Ok(())
    }
}
