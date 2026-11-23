use super::package::{self, Manifest, Trust};
use anyhow::{Context as _, Result, ensure};
use async_channel::{Receiver, Sender};
use std::{
    os::windows::process::CommandExt as _,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use ui::updater::Status;

const POLL: Duration = Duration::from_secs(3600);
const PACKAGE: &str = "prepared.msix";

pub(super) enum Event {
    Status(Status),
    Available(String),
    Restart,
}

enum Command {
    Check,
    Automatic(bool),
    Managed(bool),
    Install(Status),
}

enum Candidate {
    Available(Manifest),
    Ready(Manifest, tempfile::TempDir),
}

pub(super) struct Backend {
    commands: mpsc::Sender<Command>,
    managed: Arc<AtomicBool>,
}

impl Backend {
    pub(super) fn check(&self) {
        let _ = self.commands.send(Command::Check);
    }

    pub(super) fn set_automatic(&self, on: bool) {
        let _ = self.commands.send(Command::Automatic(on));
    }

    pub(super) fn set_managed(&self, managed: bool) {
        self.managed.store(managed, Ordering::SeqCst);
        let _ = self.commands.send(Command::Managed(managed));
    }

    pub(super) fn install(&self, status: Status) {
        let _ = self.commands.send(Command::Install(status));
    }
}

pub(super) fn start(automatic: bool) -> Result<Option<(Backend, Receiver<Event>)>> {
    let Some(trust) = Trust::configured()? else {
        return Ok(None);
    };
    let Some(app_id) = app_id()? else {
        return Ok(None);
    };
    let root = std::env::temp_dir().join("windows-msix-updater");
    std::fs::create_dir_all(&root).context("cannot create the Windows update directory")?;
    let (events, receiver) = async_channel::unbounded();
    let (commands, inbox) = mpsc::channel();
    let managed = Arc::new(AtomicBool::new(false));
    let policy = managed.clone();
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || run(&trust, &app_id, &root, automatic, &policy, &events, &inbox))
        .context("cannot start the updater thread")?;
    Ok(Some((Backend { commands, managed }, receiver)))
}

fn run(
    trust: &Trust,
    app_id: &str,
    root: &Path,
    mut automatic: bool,
    managed: &AtomicBool,
    events: &Sender<Event>,
    inbox: &mpsc::Receiver<Command>,
) {
    let send = |event| {
        let _ = events.try_send(event);
    };
    let status = |state| send(Event::Status(state));
    let mut candidate = None;
    let mut next = if automatic {
        Command::Check
    } else {
        Command::Automatic(false)
    };
    loop {
        match next {
            Command::Managed(true) => {
                candidate = None;
                status(Status::Idle);
            }
            Command::Managed(false) if automatic => {
                next = Command::Check;
                continue;
            }
            Command::Managed(false) => {}
            Command::Automatic(on) => {
                automatic = on;
                if on {
                    next = Command::Check;
                    continue;
                }
            }
            Command::Check
                if !managed.load(Ordering::SeqCst)
                    && !matches!(candidate, Some(Candidate::Ready(..))) =>
            {
                candidate = None;
                status(Status::Checking);
                match package::check(trust, env!("CARGO_PKG_VERSION")) {
                    Ok(Some(manifest)) if !managed.load(Ordering::SeqCst) => {
                        send(Event::Available(manifest.version.clone()));
                        candidate = Some(Candidate::Available(manifest));
                    }
                    Ok(_) => status(Status::Idle),
                    Err(error) => {
                        eprintln!("codex-gpui: cannot check for a Windows update: {error:#}");
                        status(Status::Idle);
                    }
                }
            }
            Command::Install(expected) if !managed.load(Ordering::SeqCst) => {
                match candidate.take() {
                    Some(Candidate::Available(manifest)) if expected == Status::Available => {
                        status(Status::Downloading(None));
                        match stage(trust, &manifest, root, app_id, |percent| {
                            status(Status::Downloading(percent));
                        }) {
                            Ok(directory) => {
                                candidate = Some(Candidate::Ready(manifest, directory));
                                status(Status::Ready);
                            }
                            Err(error) => {
                                eprintln!("codex-gpui: cannot stage the Windows update: {error:#}");
                                candidate = Some(Candidate::Available(manifest));
                                status(Status::Available);
                            }
                        }
                    }
                    Some(Candidate::Ready(manifest, directory)) if expected == Status::Ready => {
                        status(Status::Installing);
                        match install(&manifest, directory.path(), app_id) {
                            Ok(()) => {
                                send(Event::Restart);
                                break;
                            }
                            Err(error) => {
                                eprintln!(
                                    "codex-gpui: cannot install the Windows update: {error:#}"
                                );
                                candidate = Some(Candidate::Ready(manifest, directory));
                                status(Status::Ready);
                            }
                        }
                    }
                    unchanged => candidate = unchanged,
                }
            }
            Command::Check | Command::Install(_) => {}
        }
        next = if automatic && !managed.load(Ordering::SeqCst) {
            match inbox.recv_timeout(POLL) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => Command::Check,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match inbox.recv() {
                Ok(command) => command,
                Err(_) => break,
            }
        };
    }
}

fn app_id() -> Result<Option<String>> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentApplicationUserModelId(length: *mut u32, id: *mut u16) -> i32;
    }
    const NO_PACKAGE: i32 = 15700;
    const NO_APPLICATION: i32 = 15703;
    let mut id = [0; 130];
    let mut len = id.len() as u32;
    // SAFETY: both pointers are valid for the duration of the call and len is the buffer capacity.
    match unsafe { GetCurrentApplicationUserModelId(&mut len, id.as_mut_ptr()) } {
        0 => Ok(Some(String::from_utf16(&id[..len as usize - 1])?)),
        NO_PACKAGE | NO_APPLICATION => Ok(None),
        code => {
            Err(std::io::Error::from_raw_os_error(code)).context("cannot read MSIX app identity")
        }
    }
}

fn powershell(script: &str) -> Result<Process> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    let root =
        std::env::var_os("SystemRoot").context("cannot find the Windows system directory")?;
    let mut command =
        Process::new(PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    let script: Vec<u8> = ["$ErrorActionPreference = 'Stop'; ", script]
        .into_iter()
        .flat_map(|part| part.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    command
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(STANDARD.encode(script));
    Ok(command)
}

fn run_script(command: &mut Process) -> Result<()> {
    let output = command.output().context("cannot run Windows PowerShell")?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn stage(
    trust: &Trust,
    manifest: &Manifest,
    root: &Path,
    app_id: &str,
    progress: impl FnMut(Option<u8>),
) -> Result<tempfile::TempDir> {
    let directory = tempfile::tempdir_in(root)?;
    let path = directory.path().join(PACKAGE);
    package::download(trust, manifest, &path, progress)?;
    run_script(powershell(r#"
        $signature = Get-AuthenticodeSignature -LiteralPath $env:CGX_MSIX
        if ($signature.Status -ne 'Valid') { throw "Windows rejected the MSIX signature: $($signature.Status)" }
        $family = $env:CGX_APP_ID.Split('!')[0]
        $current = Get-AppxPackage -Name CodexGPUI.App | Where-Object PackageFamilyName -CEQ $family
        if (!$current) { throw 'The current MSIX is not registered' }
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::OpenRead($env:CGX_MSIX)
        try {
            $entry = $archive.GetEntry('AppxManifest.xml')
            if (!$entry -or $entry.Length -gt 1MB) { throw 'Invalid MSIX manifest' }
            $reader = [IO.StreamReader]::new($entry.Open())
            try { $identity = ([xml]$reader.ReadToEnd()).Package.Identity }
            finally { $reader.Dispose() }
            if ($identity.Name -cne $current.Name -or $identity.Publisher -cne $current.Publisher -or
                $identity.ProcessorArchitecture -ne $current.Architecture -or
                [version]$identity.Version -le [version]$current.Version) {
                throw 'The MSIX is not a newer version of the installed package'
            }
        } finally { $archive.Dispose() }
    "#)?
        .env("CGX_MSIX", &path)
        .env("CGX_APP_ID", app_id))?;
    Ok(directory)
}

fn install(manifest: &Manifest, directory: &Path, app_id: &str) -> Result<()> {
    let path = directory.join(PACKAGE);
    let deploy = r#"
        if ((Get-FileHash -LiteralPath $env:CGX_MSIX).Hash -ne $env:CGX_SHA256) {
            throw 'The staged MSIX checksum has changed'
        }
        Add-AppxPackage -Path $env:CGX_MSIX -DeferRegistrationWhenPackagesAreInUse
    "#;
    run_script(
        powershell(deploy)?
            .env("CGX_MSIX", path)
            .env("CGX_SHA256", &manifest.sha256),
    )?;
    powershell(r#"
        Wait-Process -Id ([int]$env:CGX_PID) -ErrorAction SilentlyContinue
        Start-Process -FilePath "$env:SystemRoot\explorer.exe" -ArgumentList ('shell:AppsFolder\' + $env:CGX_APP_ID)
    "#)?
        .env("CGX_PID", std::process::id().to_string())
        .env("CGX_APP_ID", app_id)
        .spawn()
        .context("cannot start Windows relaunch helper")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer as _, SigningKey};
    use sha2::{Digest as _, Sha256};
    use std::{
        io::{BufRead as _, BufReader, Write as _},
        net::TcpListener,
    };

    #[test]
    fn windows_feed_finds_update_but_does_not_stage_an_unsigned_msix() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("unsigned.msix");
        let create_unsigned = r#"
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $zip = [IO.Compression.ZipFile]::Open($env:CGX_MSIX, 'Create')
            try { $null = $zip.CreateEntry('AppxManifest.xml') }
            finally { $zip.Dispose() }
        "#;
        run_script(powershell(create_unsigned)?.env("CGX_MSIX", &input))?;
        let bytes = std::fs::read(input)?;
        let key = SigningKey::from_bytes(&[7; 32]);
        let digest = Sha256::digest(&bytes);
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let base = format!("http://{}", listener.local_addr()?);
        let feed = serde_json::to_vec(&serde_json::json!({
            "version": "99.0.0",
            "url": format!("{base}/package"),
            "sha256": crate::private::hex(&digest),
            "signature": STANDARD.encode(key.sign(&digest).to_bytes()),
        }))?;
        let server = std::thread::spawn(move || -> Result<()> {
            for (path, body) in [("/feed", feed), ("/package", bytes)] {
                let (mut socket, _) = listener.accept()?;
                let mut request = BufReader::new(&socket);
                let mut line = String::new();
                request.read_line(&mut line)?;
                assert!(line.starts_with(&format!("GET {path} ")), "{line}");
                loop {
                    line.clear();
                    if request.read_line(&mut line)? == 0 || line == "\r\n" {
                        break;
                    }
                }
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )?;
                socket.write_all(&body)?;
            }
            Ok(())
        });
        let trust = Trust {
            feed: format!("{base}/feed"),
            key: key.verifying_key(),
        };
        let manifest = package::check(&trust, "1.0.0")?.context("no update from fixture feed")?;
        assert_eq!(manifest.version, "99.0.0");
        let staging = directory.path().join("windows-msix-updater");
        std::fs::create_dir(&staging)?;
        let error = stage(
            &trust,
            &manifest,
            &staging,
            "CodexGPUI.App_test!App",
            |_| {},
        )
        .err()
        .context("unsigned MSIX reached the Ready state")?;
        assert!(
            format!("{error:#}").contains("Windows rejected the MSIX signature"),
            "{error:#}"
        );
        assert!(staging.read_dir()?.next().is_none());
        server.join().expect("fixture server panicked")?;
        Ok(())
    }
}
