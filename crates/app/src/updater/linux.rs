//! AppImage self-update: poll the signed manifest, stage the new AppImage next to the running
//! one, and swap it in and relaunch when the user installs.
use super::package::{self, Trust};
use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender};
use std::{
    os::unix::{fs::PermissionsExt as _, process::CommandExt as _},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use ui::updater::Status;

const POLL: Duration = Duration::from_secs(6 * 60 * 60);

enum Command {
    Install,
    Managed(bool),
}

pub struct Backend {
    commands: mpsc::Sender<Command>,
}

impl Backend {
    pub fn set_managed(&self, managed: bool) {
        let _ = self.commands.send(Command::Managed(managed));
    }

    pub fn install(&self) {
        let _ = self.commands.send(Command::Install);
    }
}

/// `None` when this run is not an AppImage or no update feed is configured.
pub fn start() -> Result<Option<(Backend, Receiver<Status>)>> {
    let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) else {
        return Ok(None);
    };
    let Some(trust) = Trust::configured()? else {
        return Ok(None);
    };
    let (events, receiver) = async_channel::unbounded();
    let (commands, inbox) = mpsc::channel();
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || run(&trust, &appimage, &events, &inbox))
        .context("cannot start the updater thread")?;
    Ok(Some((Backend { commands }, receiver)))
}

fn run(trust: &Trust, appimage: &Path, events: &Sender<Status>, inbox: &mpsc::Receiver<Command>) {
    let staged = {
        let mut name = appimage.as_os_str().to_owned();
        name.push(".update");
        PathBuf::from(name)
    };
    let emit = |status| {
        let _ = events.try_send(status);
    };
    let mut managed = false;
    let mut ready = false;
    let mut next_check = Some(Duration::ZERO);
    loop {
        let command = match next_check.take() {
            Some(Duration::ZERO) => None,
            Some(wait) => inbox.recv_timeout(wait).ok(),
            None => inbox.recv().ok(),
        };
        match command {
            Some(Command::Managed(value)) => {
                managed = value;
                next_check = Some(if managed || ready {
                    POLL
                } else {
                    Duration::ZERO
                });
            }
            Some(Command::Install) if ready && !managed => {
                emit(Status::Installing);
                match swap_and_relaunch(appimage, &staged) {
                    Ok(never) => match never {},
                    Err(error) => {
                        eprintln!("codex-gpui: cannot install the update: {error:#}");
                        ready = false;
                        emit(Status::Idle);
                    }
                }
                next_check = Some(POLL);
            }
            Some(Command::Install) => next_check = Some(POLL),
            None if managed || ready => next_check = Some(POLL),
            None => {
                emit(Status::Checking);
                ready = check_and_stage(trust, &staged, events).unwrap_or_else(|error| {
                    eprintln!("codex-gpui: update check failed: {error:#}");
                    false
                });
                emit(if ready { Status::Ready } else { Status::Idle });
                next_check = Some(POLL);
            }
        }
    }
}

fn check_and_stage(trust: &Trust, staged: &Path, events: &Sender<Status>) -> Result<bool> {
    let Some(manifest) = package::check(trust, env!("CARGO_PKG_VERSION"))? else {
        return Ok(false);
    };
    package::download(trust, &manifest, staged, |percent| {
        let _ = events.try_send(Status::Downloading(percent));
    })?;
    Ok(true)
}

/// Rename over the running AppImage (atomic, and safe while it runs), then exec it.
fn swap_and_relaunch(appimage: &Path, staged: &Path) -> Result<std::convert::Infallible> {
    let mode = std::fs::metadata(appimage)?.permissions().mode() | 0o111;
    std::fs::set_permissions(staged, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(staged, appimage).context("cannot replace the AppImage")?;
    let error = std::process::Command::new(appimage)
        .args(std::env::args_os().skip(1))
        .exec();
    Err(error).context("cannot relaunch")
}
