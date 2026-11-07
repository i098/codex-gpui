#![cfg(target_os = "macos")]
//! Captures the native states of the retained original `diffCommentDraft` and
//! `diffCommentSaved` screenshots (dark, 1280x832) for the parity comparison.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, LocalHost, Request};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// The recorded file before the recorded `fileChange` patch.
const RECORDED_BEFORE: &str = "def add(a, b):\n    return a + b\n";

/// Serves file reads for the recorded paths from the fixture repository.
struct FixtureHost {
    inner: common::ActivityFixtureHost,
    root: PathBuf,
    recorded_roots: Vec<PathBuf>,
    reads: Arc<AtomicUsize>,
}

impl FixtureHost {
    fn relocate(&self, path: &str) -> String {
        self.recorded_roots
            .iter()
            .find_map(|root| Path::new(path).strip_prefix(root).ok())
            .map_or_else(
                || path.to_owned(),
                |relative| self.root.join(relative).to_string_lossy().into_owned(),
            )
    }
}

impl Host for FixtureHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Fetch { method, url, body } = request else {
            return self.inner.fetch(request);
        };
        let endpoint = url.rsplit('/').next().unwrap_or("");
        if !matches!(
            endpoint,
            "git-cat-file" | "read-git-file-binary" | "read-file" | "read-file-binary"
        ) {
            return self.inner.fetch(request);
        }
        let mut params: Value = serde_json::from_str(body)?;
        if let Some(path) = params["cwd"].as_str() {
            params["cwd"] = self.relocate(path).into();
        }
        if let Some(path) = params["path"].as_str() {
            params["path"] = self.relocate(path).into();
        }
        if let Some(requests) = params["requests"].as_array_mut() {
            for request in requests {
                if let Some(path) = request["path"].as_str() {
                    request["path"] = self.relocate(path).into();
                }
            }
        }
        let response = self.inner.fetch(&Request::Fetch {
            method: method.clone(),
            url: url.clone(),
            body: params.to_string(),
        })?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(response)
    }

    // The activity bell needs the fixture identity and its recorded feature gate.
    fn identity(&self) -> Option<app::host::Identity> {
        self.inner.identity()
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        self.inner.projects()
    }

    fn review_repositories(
        &self,
        thread: &str,
        cwd: &str,
    ) -> Result<Vec<ui::screens::diff::panel::ReviewRepository>> {
        self.inner.review_repositories(thread, &self.relocate(cwd))
    }
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let result = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        result.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

fn output(name: &str) -> Result<PathBuf> {
    let root =
        PathBuf::from(std::env::var("CGX_COMMENT_EVIDENCE").context("CGX_COMMENT_EVIDENCE")?);
    fs::create_dir_all(&root)?;
    Ok(root.join(name))
}

#[test]
#[ignore = "requires CGX_COMMENT_EVIDENCE and the retained original comment screenshots"]
fn capture_original_comment_states() -> Result<()> {
    common::install_panic_hook();
    let recorded = common::recorded_thread(common::SUBTRACT)?;
    let change = &recorded["turns"]
        .as_array()
        .context("turns")?
        .iter()
        .flat_map(|turn| turn["items"].as_array().into_iter().flatten())
        .find(|item| item["type"] == "fileChange")
        .context("recorded file change")?["changes"][0];
    let relative =
        Path::new(change["path"].as_str().context("recorded path")?).strip_prefix(common::CWD)?;
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("repository");
    let file = root.join(relative);
    fs::create_dir_all(file.parent().context("file parent")?)?;
    fs::write(&file, RECORDED_BEFORE)?;
    git(&root, &["init", "-q"])?;
    git(&root, &["config", "user.name", "Recorded file proof"])?;
    git(&root, &["config", "user.email", "proof@example.invalid"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-qm", "recorded before file"])?;
    let patch = directory.path().join("recorded.patch");
    fs::write(
        &patch,
        format!(
            "--- a/{0}\n+++ b/{0}\n{1}",
            relative.display(),
            change["diff"].as_str().context("recorded patch")?
        ),
    )?;
    git(
        &root,
        &[
            "apply",
            "--unidiff-zero",
            patch.to_str().context("patch path")?,
        ],
    )?;
    let reads = Arc::new(AtomicUsize::new(0));
    let host = FixtureHost {
        inner: common::ActivityFixtureHost(LocalHost::new(directory.path().join("home"))),
        root,
        recorded_roots: vec![
            common::CWD.into(),
            recorded["cwd"].as_str().context("recorded cwd")?.into(),
        ],
        reads: reads.clone(),
    };
    let mut fx = common::Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    fx.wait("recorded partial-file state", |fx| {
        fx.read(|root, cx| {
            root.diff_panel().unwrap().read(cx).full_file_error(0)
                == Some(ui::screens::diff::panel::FullError::Unavailable)
        })
    })?;
    fx.advance(Duration::from_millis(100))?;
    ensure!(
        reads.load(Ordering::SeqCst) == 0,
        "partial metadata must not guess a before revision"
    );
    // The originals show the Explore dot; main draws it from fetched unseen destinations, which
    // this replay cannot supply.
    let shell = fx.read(|root, _| root.shell().clone());
    shell.update(&mut fx.cx, |shell, _| {
        shell.unread_destinations = Some(["unseen".into()].into());
    });
    fx.hover((814., 239.6))?;
    fx.click((704., 239.6))?;
    fx.wait("comment draft", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .comment_draft()
                .is_some()
        })
    })?;
    fx.type_text("Check this line.")?;
    fx.reference_shot()?.save(output("comment-draft.png")?)?;
    fx.click((1221., 357.))?;
    fx.wait("saved comment", |fx| {
        fx.read(|root, cx| {
            root.composer(cx)
                .read(cx)
                .diff_comments()
                .first()
                .is_some_and(|comment| comment.text() == "Check this line.")
        })
    })?;
    fx.reference_shot()?.save(output("comment-saved.png")?)?;
    Ok(())
}
