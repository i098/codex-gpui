//! Starting a cloud task from a local checkout (PROTO-17): the original packs the working
//! tree into a tarball, asks `/wham/worktree_snapshots/upload_url` for a blob URL, PUTs the
//! tarball, calls `finish_upload`, and creates the task with a `local_worktree` environment.
//! Local files and the Git index are only read.

use crate::{
    Root,
    cloud::{CloudTaskRequest, post_task, request},
    host,
};
use anyhow::{Context as _, Result, bail, ensure};
use gpui::AppContext as _;
use serde_json::{Map, Value, json};
use std::{
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
};

const CONTENT_TYPE: &str = "application/gzip";

pub(crate) struct Snapshot {
    tarball: tempfile::TempPath,
    filename: String,
    size: u64,
    repo_name: String,
    snapshot_branch: String,
    commit_sha: String,
    remotes: Map<String, Value>,
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "git {} failed: {}",
        args.first().copied().unwrap_or_default(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `git remote -v` as `{name: push url}`.
fn push_remotes(listing: &str) -> Map<String, Value> {
    let mut remotes = Map::new();
    for line in listing.lines() {
        let mut fields = line.split_whitespace();
        if let (Some(name), Some(url), Some("(push)")) =
            (fields.next(), fields.next(), fields.next())
        {
            remotes.entry(name).or_insert_with(|| url.into());
        }
    }
    remotes
}

/// Packs the tracked and untracked, non-ignored files of `root` as they are on disk; with a
/// `branch`, packs that branch's commit instead (`git archive`).
pub(crate) fn prepare(root: &Path, branch: Option<&str>) -> Result<Snapshot> {
    let commit_sha = git(root, &["rev-parse", "HEAD"])
        .map(|sha| sha.trim().to_owned())
        .ok()
        .filter(|sha| !sha.is_empty())
        .context("Cannot create a worktree snapshot without a HEAD commit")?;
    let current = git(root, &["branch", "--show-current"])?.trim().to_owned();
    let repo_name = root
        .file_name()
        .map_or_else(|| "repo".into(), |name| name.to_string_lossy().into_owned());
    let tarball = tempfile::Builder::new()
        .prefix(&format!("{repo_name}-snapshot-"))
        .suffix(".tar.gz")
        .tempfile()?
        .into_temp_path();
    let branch = branch.map(str::trim).filter(|branch| !branch.is_empty());
    if let Some(branch) = branch {
        let path = tarball.to_string_lossy();
        git(root, &["archive", "--format=tar.gz", "-o", &path, branch])?;
    } else {
        pack_files(root, &tarball)?;
    }
    let size = std::fs::metadata(&tarball)?.len();
    let snapshot_branch = branch
        .map(str::to_owned)
        .or_else(|| Some(current).filter(|current| !current.is_empty()))
        .unwrap_or_else(|| commit_sha.clone());
    Ok(Snapshot {
        filename: tarball
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        tarball,
        size,
        repo_name,
        snapshot_branch,
        remotes: push_remotes(&git(root, &["remote", "-v"])?),
        commit_sha,
    })
}

fn pack_files(root: &Path, tarball: &Path) -> Result<()> {
    let listing = git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    // Tracked files deleted in the working tree are listed by Git but cannot be packed.
    let mut names = tempfile::NamedTempFile::new()?;
    for name in listing
        .split('\0')
        .filter(|name| !name.is_empty() && root.join(name).symlink_metadata().is_ok())
    {
        names.write_all(name.as_bytes())?;
        names.write_all(&[0])?;
    }
    names.flush()?;
    let output = Command::new("tar")
        .args(["--null", "-czf"])
        .arg(tarball)
        .arg("-C")
        .arg(root)
        .arg("--files-from")
        .arg(names.path())
        .stdin(Stdio::null())
        .output()
        .context("Cannot run tar")?;
    if !output.status.success() {
        bail!(
            "Exited with code {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// PUTs the tarball to the blob URL the backend issued.
fn upload(snapshot: &Snapshot, url: &str) -> Result<()> {
    let file = std::fs::File::open(&snapshot.tarball)?;
    ureq::put(url)
        .header("Content-Type", CONTENT_TYPE)
        .header("Content-Length", snapshot.size.to_string())
        .header("x-ms-blob-type", "BlockBlob")
        .send(file)
        .map(drop)
        .map_err(|error| anyhow::anyhow!("Snapshot upload failed: {error}"))
}

/// Starts a cloud task from the local checkout at `root`, uncommitted work included. Any
/// failed step returns an error before a task exists, so the caller can restore its draft.
pub fn create_cloud_task_from_worktree(
    host: &dyn host::Host,
    task: &CloudTaskRequest,
    root: &Path,
    branch: Option<&str>,
) -> Result<Value> {
    ensure!(
        !task.prompt.trim().is_empty(),
        "A cloud task needs a prompt"
    );
    let identity = host
        .identity()
        .context("Cloud authentication is unavailable")?;
    let snapshot = prepare(root, branch)?;
    let session = request(
        host,
        &identity,
        "POST",
        "/wham/worktree_snapshots/upload_url",
        Some(json!({
            "repo_name": snapshot.repo_name,
            "filename": snapshot.filename,
            "content_type": CONTENT_TYPE,
            "anticipated_file_size": snapshot.size,
        })),
    )?;
    let upload_url = session["upload_url"]
        .as_str()
        .context("The snapshot upload response has no upload_url")?;
    let file_id = session["file_id"]
        .as_str()
        .context("The snapshot upload response has no file_id")?;
    upload(&snapshot, upload_url)?;
    let finished = request(
        host,
        &identity,
        "POST",
        "/wham/worktree_snapshots/finish_upload",
        Some(json!({"file_id": file_id, "etag": session["etag"]})),
    )?;
    ensure!(
        finished["file_id"].is_string(),
        "The finished snapshot upload has no file_id"
    );
    let environment = json!({"repos": [{
        "kind": "local_worktree",
        "name": snapshot.repo_name,
        "remotes": snapshot.remotes,
        "commit_sha": snapshot.commit_sha,
        "branch": snapshot.snapshot_branch,
        "file_id": finished["file_id"],
    }]});
    let task = CloudTaskRequest {
        prompt: task.prompt.clone(),
        branch: if task.branch.is_empty() {
            snapshot.snapshot_branch.clone()
        } else {
            task.branch.clone()
        },
        environment_id: String::new(),
        model: task.model.clone(),
        read_only: task.read_only,
    };
    post_task(host, &task, Some(environment))
}

impl Root {
    /// Home's `Cloud` start without a saved environment: uploads the project's working tree
    /// and starts the task from it. A failure restores the draft.
    pub(crate) fn start_local_cloud_task(
        &mut self,
        submission: ui::composer::Submission,
        options: ui::screens::hotkey::Options,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let host = self.host.clone();
        let hotkey = self.popout.is_some();
        let task = CloudTaskRequest {
            prompt: submission.text.clone(),
            branch: String::new(),
            environment_id: String::new(),
            model: (!submission.model.is_empty()).then(|| submission.model.clone()),
            read_only: false,
        };
        let root = options.root.clone().map(std::path::PathBuf::from);
        let branch = options.branch.filter(|branch| branch != "HEAD");
        cx.spawn_in(window, async move |this, cx| {
            let created = cx
                .background_spawn(async move {
                    let root = root.context("Select a project to start a cloud task")?;
                    let response = create_cloud_task_from_worktree(
                        host.as_ref(),
                        &task,
                        &root,
                        branch.as_deref(),
                    )?;
                    response
                        .pointer("/task/id")
                        .or_else(|| response.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .context("The cloud task response has no task id")
                })
                .await;
            this.update_in(cx, |this, window, cx| match created {
                Ok(task_id) => this.open_remote_task(task_id, hotkey, window, cx),
                Err(error) => {
                    eprintln!("codex-gpui: cannot start cloud task: {error:#}");
                    this.restore_start_draft(&submission, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read as _, net::TcpListener, sync::Mutex};

    fn run(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("tracked.txt"), "committed\n").unwrap();
        std::fs::write(root.join("gone.txt"), "deleted later\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-q", "-m", "init"]);
        run(
            root,
            &["remote", "add", "origin", "https://example.com/o/r.git"],
        );
        std::fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        std::fs::write(root.join("staged.txt"), "staged\n").unwrap();
        run(root, &["add", "staged.txt"]);
        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "skip\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        dir
    }

    fn entries(tarball: &Path) -> Vec<String> {
        let output = Command::new("tar")
            .arg("-tzf")
            .arg(tarball)
            .output()
            .unwrap();
        let mut names: Vec<_> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn archive_holds_the_working_tree_and_leaves_the_index_alone() {
        let dir = repo();
        let root = dir.path();
        let before = git(root, &["status", "--porcelain=v1"]).unwrap();
        let snapshot = prepare(root, None).unwrap();
        assert_eq!(
            entries(&snapshot.tarball),
            [".gitignore", "staged.txt", "tracked.txt", "untracked.txt"]
        );
        let extract = tempfile::tempdir().unwrap();
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&snapshot.tarball)
            .arg("-C")
            .arg(extract.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(extract.path().join("tracked.txt")).unwrap(),
            "edited\n"
        );
        assert_eq!(git(root, &["status", "--porcelain=v1"]).unwrap(), before);
        assert_eq!(snapshot.snapshot_branch, "main");
        assert_eq!(snapshot.remotes["origin"], "https://example.com/o/r.git");
        assert_eq!(
            snapshot.size,
            std::fs::metadata(&snapshot.tarball).unwrap().len()
        );
        assert!(snapshot.filename.ends_with(".tar.gz"));
    }

    #[test]
    fn a_named_branch_packs_its_commit_without_local_edits() {
        let dir = repo();
        let snapshot = prepare(dir.path(), Some("main")).unwrap();
        assert_eq!(
            entries(&snapshot.tarball),
            [".gitignore", "gone.txt", "tracked.txt"]
        );
        assert_eq!(snapshot.snapshot_branch, "main");
    }

    #[test]
    fn a_repository_without_a_commit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]);
        let error = prepare(dir.path(), None).err().unwrap();
        assert!(error.to_string().contains("without a HEAD commit"));
    }

    struct Backend {
        calls: Mutex<Vec<(String, Value)>>,
        upload_url: String,
        fail_task: bool,
    }

    impl host::Host for Backend {
        fn fetch(&self, request: &host::Request) -> Result<Value> {
            let host::Request::Http(request) = request else {
                bail!("unexpected request");
            };
            let path = request["url"].as_str().unwrap().to_owned();
            let body: Value = serde_json::from_str(request["body"].as_str().unwrap()).unwrap();
            self.calls.lock().unwrap().push((path.clone(), body));
            let (status, body) = match path.as_str() {
                "/wham/worktree_snapshots/upload_url" => (
                    200,
                    json!({"file_id": "file-1", "upload_url": self.upload_url, "etag": "etag-1"}),
                ),
                "/wham/worktree_snapshots/finish_upload" => (200, json!({"file_id": "file-2"})),
                "/wham/tasks" if self.fail_task => (500, json!({})),
                "/wham/tasks" => (200, json!({"task": {"id": "task_e_1"}})),
                other => panic!("unexpected path {other}"),
            };
            Ok(json!({"status": status, "body": body.to_string()}))
        }

        fn identity(&self) -> Option<host::Identity> {
            Some(host::Identity {
                account_id: "account".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            })
        }
    }

    /// One-shot blob server: returns the request head and body it received.
    fn blob_server(status: &'static str) -> (String, std::thread::JoinHandle<(String, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/blob", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut chunk = [0; 8192];
            let (head, length) = loop {
                let read = stream.read(&mut chunk).unwrap();
                received.extend_from_slice(&chunk[..read]);
                if let Some(end) = received.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&received[..end]).into_owned();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")?
                                .parse()
                                .ok()
                        })
                        .unwrap_or(0usize);
                    break (head, length);
                }
            };
            let start = received
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            while received.len() - start < length {
                let read = stream.read(&mut chunk).unwrap();
                received.extend_from_slice(&chunk[..read]);
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            (head, received[start..].to_vec())
        });
        (url, handle)
    }

    fn request() -> CloudTaskRequest {
        CloudTaskRequest {
            prompt: "fix it".into(),
            branch: String::new(),
            environment_id: String::new(),
            model: Some("gpt".into()),
            read_only: false,
        }
    }

    #[test]
    fn upload_flow_creates_the_task_from_the_snapshot() {
        let dir = repo();
        let (upload_url, server) = blob_server("201 Created");
        let backend = Backend {
            calls: Mutex::new(Vec::new()),
            upload_url,
            fail_task: false,
        };
        let created =
            create_cloud_task_from_worktree(&backend, &request(), dir.path(), None).unwrap();
        assert_eq!(created["task"]["id"], "task_e_1");

        let calls = backend.calls.lock().unwrap();
        let paths: Vec<_> = calls.iter().map(|call| call.0.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/wham/worktree_snapshots/upload_url",
                "/wham/worktree_snapshots/finish_upload",
                "/wham/tasks"
            ]
        );
        let (head, body) = server.join().unwrap();
        let head = head.to_ascii_lowercase();
        assert!(head.starts_with("put /blob "));
        assert!(head.contains("x-ms-blob-type: blockblob"));
        assert!(head.contains("content-type: application/gzip"));
        assert!(!head.contains("transfer-encoding"));
        assert_eq!(calls[0].1["anticipated_file_size"], body.len());
        assert_eq!(calls[0].1["content_type"], "application/gzip");
        assert_eq!(&body[..2], [0x1f, 0x8b]);
        assert_eq!(calls[1].1, json!({"file_id": "file-1", "etag": "etag-1"}));
        let new_task = &calls[2].1["new_task"];
        assert!(new_task.get("environment_id").is_none());
        assert_eq!(new_task["branch"], "main");
        let repo = &new_task["environment"]["repos"][0];
        assert_eq!(repo["kind"], "local_worktree");
        assert_eq!(repo["file_id"], "file-2");
        assert_eq!(repo["branch"], "main");
        assert_eq!(repo["remotes"]["origin"], "https://example.com/o/r.git");
        assert_eq!(repo["commit_sha"].as_str().unwrap().len(), 40);
        assert_eq!(calls[2].1["metadata"]["model_slug"], "gpt");
    }

    #[test]
    fn a_failed_upload_stops_before_a_task_exists() {
        let dir = repo();
        let (upload_url, server) = blob_server("403 Forbidden");
        let backend = Backend {
            calls: Mutex::new(Vec::new()),
            upload_url,
            fail_task: false,
        };
        let error = create_cloud_task_from_worktree(&backend, &request(), dir.path(), None)
            .err()
            .unwrap();
        server.join().unwrap();
        assert!(error.to_string().contains("Snapshot upload failed"));
        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
    }
}
