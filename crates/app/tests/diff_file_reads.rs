#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

#[test]
fn object_reads_preserve_lines_and_bound_disk_fallbacks() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "File read proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(root.join("file.txt"), "before\nsecond\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    let before = git(root, &["rev-parse", "HEAD:file.txt"])?;
    fs::write(root.join("file.txt"), "after\nsecond\n")?;
    let after = git(root, &["hash-object", "file.txt"])?;
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let read = |requests: Value, limit: usize| -> Result<Value> {
        json_body(&host.fetch(&Request::post(
            "git-cat-file",
            Some(json!({
                "cwd": root, "maxObjectBytes": limit, "requests": requests,
            })),
        ))?)
    };
    let values = read(
        json!([
            {"path":"file.txt","oid":before,"fallbackToDisk":false},
            {"path":"file.txt","oid":after,"fallbackToDisk":true},
            {"path":"file.txt","oid":"1111111111111111111111111111111111111111","fallbackToDisk":false}
        ]),
        100,
    )?;
    ensure!(values[0] == json!({"type":"success","lines":["before\n","second\n"]}));
    ensure!(values[1] == json!({"type":"success","lines":["after\n","second\n"]}));
    ensure!(values[2]["error"]["type"] == "not-found");
    let oversized = read(
        json!([{"path":"file.txt","oid":after,"fallbackToDisk":true}]),
        4,
    )?;
    ensure!(oversized[0] == json!({"type":"error","error":{"type":"too-large","limitBytes":4}}));
    let escaped = read(
        json!([{"path":"../outside.txt","oid":null,"fallbackToDisk":true}]),
        100,
    )?;
    ensure!(escaped[0]["type"] == "error");
    ensure!(fs::read_to_string(root.join("file.txt"))? == "after\nsecond\n");
    Ok(())
}

#[test]
fn file_and_index_reads_enforce_the_project_boundary() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "File read proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    let path = root.join("bytes.dat");
    fs::write(&path, [0, 1, 2, 3])?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    fs::write(&path, [0, 9, 8, 7])?;
    git(root, &["add", "."])?;
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let read = |endpoint: &str, params: Value| -> Result<Value> {
        json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)
    };
    let head = read(
        "read-git-file-binary",
        json!({"cwd":root,"path":"bytes.dat","ref":"head"}),
    )?;
    let index = read(
        "read-git-file-binary",
        json!({"cwd":root,"path":"bytes.dat","ref":"index"}),
    )?;
    ensure!(
        STANDARD.decode(head["contentsBase64"].as_str().context("head bytes")?)? == [0, 1, 2, 3]
    );
    ensure!(
        STANDARD.decode(index["contentsBase64"].as_str().context("index bytes")?)? == [0, 9, 8, 7]
    );
    let limited = read(
        "read-file-binary",
        json!({"cwd":root,"path":path,"maxBytes":3}),
    )?;
    ensure!(limited["contentsBase64"].is_null());
    ensure!(
        read(
            "read-file-binary",
            json!({"cwd":root,"path":path,"maxBytes":-1})
        )
        .is_err()
    );
    let outside = home.path().join("outside.txt");
    fs::write(&outside, "outside")?;
    ensure!(read("read-file", json!({"cwd":root,"path":outside})).is_err());
    #[cfg(unix)]
    {
        let link = root.join("escape.txt");
        std::os::unix::fs::symlink(&outside, &link)?;
        ensure!(read("read-file-binary", json!({"cwd":root,"path":link})).is_err());
    }
    Ok(())
}
