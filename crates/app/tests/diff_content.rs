#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::{diff_content::read_full_file, host::LocalHost};
use std::{fs, path::Path, process::Command};
use ui::screens::diff::panel::{BlobRequest, FullError, FullFileRequest};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

fn repository() -> Result<tempfile::TempDir> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"])?;
    git(repo.path(), &["config", "user.name", "Full file proof"])?;
    git(
        repo.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    Ok(repo)
}

fn one_side(root: &Path, path: &str, oid: Option<String>, fallback: bool) -> FullFileRequest {
    FullFileRequest {
        cwd: root.to_string_lossy().into_owned(),
        requests: vec![BlobRequest {
            path: path.into(),
            oid,
            fallback_to_disk: fallback,
        }],
        before: None,
        after: Some(0),
    }
}

#[test]
fn full_file_reads_preserve_sides_line_endings_and_disk_fallback() -> Result<()> {
    let repo = repository()?;
    let root = repo.path();
    fs::write(root.join("file.txt"), "before\r\nlast")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "before"])?;
    let before = git(root, &["rev-parse", "HEAD:file.txt"])?;
    fs::write(root.join("file.txt"), "after\r\n尾\n")?;
    let after = git(root, &["hash-object", "file.txt"])?;
    let request = FullFileRequest {
        cwd: root.to_string_lossy().into_owned(),
        requests: vec![
            BlobRequest {
                path: "file.txt".into(),
                oid: Some(after),
                fallback_to_disk: true,
            },
            BlobRequest {
                path: "file.txt".into(),
                oid: Some(before),
                fallback_to_disk: false,
            },
        ],
        before: Some(1),
        after: Some(0),
    };
    let content = read_full_file(&LocalHost::new(root), &request)
        .map_err(|error| anyhow::anyhow!("full file failed: {error:?}"))?;
    ensure!(content.before == ["before\r\n", "last"]);
    ensure!(content.after == ["after\r\n", "尾\n"]);
    ensure!(fs::read_to_string(root.join("file.txt"))? == "after\r\n尾\n");
    Ok(())
}

#[test]
fn missing_and_oversized_full_files_remain_unavailable() -> Result<()> {
    let repo = repository()?;
    let root = repo.path();
    fs::write(root.join("file.txt"), vec![b'x'; 5_242_881])?;
    let host = LocalHost::new(root);
    let missing = one_side(root, "file.txt", Some("1".repeat(40)), false);
    ensure!(read_full_file(&host, &missing).err() == Some(FullError::Unavailable));
    let oversized = one_side(root, "file.txt", None, true);
    ensure!(read_full_file(&host, &oversized).err() == Some(FullError::Unavailable));
    Ok(())
}

#[test]
fn full_file_reads_reject_an_existing_file_outside_the_repository() -> Result<()> {
    let repo = repository()?;
    let outside = tempfile::tempdir()?;
    let path = outside.path().join("private.txt");
    fs::write(&path, "outside data\n")?;
    let request = one_side(repo.path(), &path.to_string_lossy(), None, true);
    let result = read_full_file(&LocalHost::new(repo.path()), &request);
    ensure!(result.err() == Some(FullError::Failed));
    ensure!(fs::read_to_string(&path)? == "outside data\n");
    Ok(())
}
