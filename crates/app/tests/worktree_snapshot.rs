use anyhow::{Context as _, Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
};

fn command(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn source(root: &Path) -> Result<()> {
    command(root, "git", &["init", "-q", "-b", "main"])?;
    command(root, "git", &["config", "user.name", "Snapshot proof"])?;
    command(
        root,
        "git",
        &["config", "user.email", "proof@example.invalid"],
    )?;
    fs::write(root.join("tracked"), "committed\n")?;
    fs::write(root.join(".gitignore"), "secret\n")?;
    command(root, "git", &["add", "."])?;
    command(root, "git", &["commit", "-qm", "Initial snapshot"])?;
    fs::write(root.join("tracked"), "working\n")?;
    fs::write(root.join("new-file"), "untracked\n")?;
    fs::write(root.join("secret"), "private\n")?;
    Ok(())
}

fn prepare(host: &LocalHost, root: &Path, branch: Option<&str>) -> Result<Value> {
    json_body(&host.fetch(&Request::post(
        "prepare-worktree-snapshot",
        Some(json!({"gitRoot": root, "snapshotBranch": branch})),
    ))?)
}

#[test]
fn snapshot_includes_working_files_or_the_selected_commit() -> Result<()> {
    let root = tempfile::tempdir()?;
    source(root.path())?;
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let working = prepare(&host, root.path(), None)?;
    let working_path = working["tarballPath"]
        .as_str()
        .context("missing working snapshot")?;
    assert_eq!(
        command(root.path(), "tar", &["-xOzf", working_path, "tracked"])?,
        "working\n"
    );
    assert_eq!(
        command(root.path(), "tar", &["-xOzf", working_path, "new-file"])?,
        "untracked\n"
    );
    let names = command(root.path(), "tar", &["-tzf", working_path])?;
    assert!(
        !names
            .lines()
            .any(|name| name == "secret" || name.starts_with(".git/"))
    );
    let committed = prepare(&host, root.path(), Some("main"))?;
    let committed_path = committed["tarballPath"]
        .as_str()
        .context("missing committed snapshot")?;
    assert_eq!(
        command(root.path(), "tar", &["-xOzf", committed_path, "tracked"])?,
        "committed\n"
    );
    assert!(
        !command(root.path(), "tar", &["-tzf", committed_path])?
            .lines()
            .any(|name| name == "new-file")
    );
    drop(host);
    assert!(!Path::new(working_path).exists());
    assert!(!Path::new(committed_path).exists());
    Ok(())
}

#[test]
fn uploads_send_the_archive_and_remove_only_owned_files() -> Result<()> {
    let root = tempfile::tempdir()?;
    source(root.path())?;
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    for status in [201, 500] {
        let snapshot = prepare(&host, root.path(), None)?;
        let path = snapshot["tarballPath"]
            .as_str()
            .context("missing snapshot")?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = format!("http://{}/snapshot", listener.local_addr()?);
        let receiver = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte)?;
                headers.push(byte[0]);
            }
            let headers = String::from_utf8_lossy(&headers).to_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            assert!(headers.contains("x-ms-blob-type: blockblob"));
            let mut body = vec![0; length];
            stream.read_exact(&mut body)?;
            write!(
                stream,
                "HTTP/1.1 {status} Result\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            Ok(body)
        });
        let result = json_body(&host.fetch(&Request::post("upload-worktree-snapshot", Some(json!({
            "tarballPath": path, "uploadUrl": address, "contentLength": snapshot["tarballSize"], "contentType": "application/gzip",
        }))))?)?;
        assert_eq!(result["success"], status == 201);
        assert!(!Path::new(path).exists());
        let body = receiver.join().unwrap()?;
        let received = root.path().join("received.tar.gz");
        fs::write(&received, body)?;
        assert_eq!(
            command(
                root.path(),
                "tar",
                &["-xOzf", received.to_str().unwrap(), "tracked"]
            )?,
            "working\n"
        );
    }
    let foreign = root.path().join("not-a-snapshot");
    fs::write(&foreign, "keep")?;
    let rejected = json_body(&host.fetch(&Request::post("upload-worktree-snapshot", Some(json!({
        "tarballPath": foreign, "uploadUrl": "http://127.0.0.1:1", "contentLength": 4, "contentType": "application/gzip",
    }))))?)?;
    assert_eq!(rejected["success"], false);
    assert_eq!(fs::read_to_string(foreign)?, "keep");
    Ok(())
}
