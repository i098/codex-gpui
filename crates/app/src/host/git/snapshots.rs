use super::{checked, command, commit_id, current_branch, run};
use crate::host::LocalHost;
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
};
use tempfile::NamedTempFile;

pub(in crate::host) type Uploads = Mutex<HashMap<PathBuf, NamedTempFile>>;

impl LocalHost {
    pub(in crate::host) fn prepare_worktree_snapshot(&self, params: &Value) -> Result<Value> {
        let cwd = params["gitRoot"]
            .as_str()
            .or_else(|| params["cwd"].as_str())
            .context("gitRoot is required")?;
        let cwd = std::path::Path::new(cwd);
        ensure!(cwd.is_absolute(), "gitRoot must be absolute");
        let root = String::from_utf8(checked(run(cwd, &["rev-parse", "--show-toplevel"])?)?)?;
        let root = std::path::Path::new(root.trim());
        let head = commit_id(root, "HEAD")?
            .context("Cannot create a worktree snapshot without a HEAD commit")?;
        let branch = current_branch(root)?;
        let repo = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repo");
        let filename = format!(
            "{repo}-snapshot-{}.tar.gz",
            chrono::Utc::now().timestamp_millis()
        );
        let archive = tempfile::Builder::new()
            .prefix("codex-worktree-snapshot-")
            .suffix(".tar.gz")
            .tempfile()?;
        let selected = params["snapshotBranch"].as_str();
        if let Some(selected) = selected.filter(|branch| !branch.trim().is_empty()) {
            let revision = format!("{selected}^{{commit}}");
            let revision = String::from_utf8(checked(run(
                root,
                &["rev-parse", "--verify", "--end-of-options", &revision],
            )?)?)?;
            checked(
                command(root, &["archive", "--format=tar.gz", "-o"])
                    .arg(archive.path())
                    .arg(revision.trim())
                    .output()?,
            )?;
        } else {
            let files = checked(run(
                root,
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ],
            )?)?;
            let mut list = NamedTempFile::new()?;
            list.write_all(&files)?;
            list.flush()?;
            let output = Command::new("tar")
                .args(["--null", "-czf"])
                .arg(archive.path())
                .arg("-C")
                .arg(root)
                .arg("--files-from")
                .arg(list.path())
                .stdin(Stdio::null())
                .output()
                .context("create worktree archive")?;
            ensure!(
                output.status.success(),
                "Failed to create worktree snapshot: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let size = archive.as_file().metadata()?.len();
        let mut remotes = serde_json::Map::new();
        let remote_text = String::from_utf8(checked(run(root, &["remote", "-v"])?)?)?;
        for line in remote_text.lines() {
            let mut fields = line.split_whitespace();
            if let (Some(name), Some(url), Some("(push)")) =
                (fields.next(), fields.next(), fields.next())
            {
                remotes.entry(name.to_owned()).or_insert_with(|| json!(url));
            }
        }
        let path = archive.path().to_path_buf();
        self.worktree_uploads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(path.clone(), archive);
        Ok(json!({
            "tarballPath": path, "tarballFilename": filename, "tarballSize": size,
            "repoName": repo, "branch": branch, "snapshotBranch": selected.or(branch.as_deref()).unwrap_or(&head),
            "commitSha": head, "remotes": remotes, "contentType": "application/gzip",
        }))
    }

    pub(in crate::host) fn upload_worktree_snapshot(&self, params: &Value) -> Result<Value> {
        let Some(path) = params["tarballPath"].as_str() else {
            return Ok(json!({"success": false}));
        };
        let archive = self
            .worktree_uploads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(std::path::Path::new(path));
        let Some(mut archive) = archive else {
            return Ok(json!({"success": false}));
        };
        let uploaded = (|| -> Result<()> {
            let address = params["uploadUrl"]
                .as_str()
                .context("uploadUrl is required")?;
            let url = url::Url::parse(address)?;
            ensure!(
                matches!(url.scheme(), "http" | "https"),
                "unsupported upload URL"
            );
            let length = params["contentLength"]
                .as_u64()
                .context("contentLength is required")?;
            ensure!(
                archive.as_file().metadata()?.len() == length,
                "snapshot length changed"
            );
            let content_type = params["contentType"]
                .as_str()
                .context("contentType is required")?;
            ensure!(
                content_type == "application/gzip",
                "invalid snapshot content type"
            );
            archive.as_file_mut().seek(SeekFrom::Start(0))?;
            let response = ureq::put(address)
                .header("Content-Type", content_type)
                .header("Content-Length", length.to_string())
                .header("x-ms-blob-type", "BlockBlob")
                .send(ureq::SendBody::from_reader(archive.as_file_mut()))?;
            ensure!(response.status().is_success(), "snapshot upload failed");
            Ok(())
        })();
        Ok(json!({"success": uploaded.is_ok()}))
    }
}
