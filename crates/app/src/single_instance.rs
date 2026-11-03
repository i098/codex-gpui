//! Forwards arguments from a second process to the running app.

use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender};
use std::fs;
use std::io::{Read as _, Write};
use std::path::Path;
use std::thread;

#[cfg(target_os = "linux")]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(target_os = "windows")]
use uds_windows::{UnixListener, UnixStream};

pub fn start(arguments: Vec<String>) -> Result<Option<Receiver<Vec<String>>>> {
    let path = crate::user_data::user_data_dir("codex-gpui").join("instance.sock");
    forward_or_bind(&path, arguments)
}

fn forward_or_bind(path: &Path, arguments: Vec<String>) -> Result<Option<Receiver<Vec<String>>>> {
    fs::create_dir_all(path.parent().context("instance socket has no parent")?)
        .context("create instance socket directory")?;
    match UnixStream::connect(path) {
        Ok(mut stream) => {
            forward(&mut stream, &arguments)?;
            return Ok(None);
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            remove_stale_socket(path);
        }
        Err(error) => return Err(error).context("connect to running app"),
    }

    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let mut stream = UnixStream::connect(path).context("connect to running app")?;
            forward(&mut stream, &arguments)?;
            return Ok(None);
        }
        Err(error) => return Err(error).context("bind instance socket"),
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .context("secure instance socket")?;
    }

    let (sender, receiver) = async_channel::unbounded();
    thread::spawn(move || serve(listener, sender));
    Ok(Some(receiver))
}

fn forward(stream: &mut UnixStream, arguments: &[String]) -> Result<()> {
    serde_json::to_writer(&mut *stream, arguments).context("encode launch arguments")?;
    stream.write_all(b"\n").context("send launch arguments")?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_stale_socket(path: &Path) {
    use std::os::unix::fs::FileTypeExt as _;
    if fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket()) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(target_os = "windows")]
fn remove_stale_socket(path: &Path) {
    let _ = path;
}

fn serve(listener: UnixListener, sender: Sender<Vec<String>>) {
    for connection in listener.incoming() {
        let Ok(mut stream) = connection else { continue };
        let mut message = Vec::new();
        if stream.read_to_end(&mut message).is_err() {
            continue;
        }
        if let Ok(arguments) = serde_json::from_slice::<Vec<String>>(&message)
            && sender.send_blocking(arguments).is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_second_launch_arguments_to_primary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.sock");
        let receiver = forward_or_bind(&path, vec![]).unwrap().unwrap();
        let arguments = vec!["codex://threads/new".to_owned(), "/tmp/project".to_owned()];

        assert!(forward_or_bind(&path, arguments.clone()).unwrap().is_none());
        assert_eq!(receiver.recv_blocking().unwrap(), arguments);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn does_not_remove_a_regular_file_at_the_socket_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.sock");
        fs::write(&path, b"keep").unwrap();

        assert!(forward_or_bind(&path, vec![]).is_err());
        assert_eq!(fs::read(path).unwrap(), b"keep");
    }
}
