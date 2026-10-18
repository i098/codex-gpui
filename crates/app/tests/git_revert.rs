#![cfg(target_os = "macos")]

use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

struct Repo {
    dir: tempfile::TempDir,
    host: LocalHost,
    _home: tempfile::TempDir,
}

impl Repo {
    fn new() -> Result<Self> {
        let home = tempfile::tempdir()?;
        let repo = Self {
            dir: tempfile::tempdir()?,
            host: LocalHost::new(home.path()),
            _home: home,
        };
        repo.git(&["init", "-q"])?;
        repo.git(&["config", "user.email", "proof@example.invalid"])?;
        repo.git(&["config", "user.name", "Native proof"])?;
        Ok(repo)
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .current_dir(self.root())
            .args(args)
            .output()?;
        ensure!(
            output.status.success(),
            "Git: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }

    fn write(&self, path: &str, content: &str) -> Result<()> {
        Ok(fs::write(self.root().join(path), content)?)
    }

    fn read(&self, path: &str) -> Result<String> {
        Ok(fs::read_to_string(self.root().join(path))?)
    }

    fn commit(&self, path: &str, content: &str) -> Result<()> {
        self.write(path, content)?;
        self.git(&["add", path])?;
        self.git(&["commit", "-qm", "base"])?;
        Ok(())
    }

    fn call(&self, endpoint: &str, params: Value) -> Result<Value> {
        let mut params = params;
        params["cwd"] = self.root().to_string_lossy().into();
        json_body(&self.host.fetch(&Request::post(endpoint, Some(params)))?)
    }

    /// The diff of `path` that Changes shows for `source`.
    fn shown(&self, source: &str, path: &str) -> Result<String> {
        let body = self.call("git-changes", json!({}))?;
        body[source]
            .as_array()
            .context("patches")?
            .iter()
            .find(|file| file["path"] == path)
            .and_then(|file| file["diff"].as_str())
            .map(str::to_owned)
            .with_context(|| format!("no {source} diff for {path}"))
    }

    fn revert(&self, source: &str, path: &str, hunk: Option<usize>) -> Result<Value> {
        let mut params = json!({
            "path": path,
            "source": source,
            "diff": self.shown(source, path)?,
        });
        if let Some(hunk) = hunk {
            params["hunk"] = hunk.into();
        }
        self.call("git-revert", params)
    }
}

fn lines(edits: &[(usize, &str)]) -> String {
    (1..=20)
        .map(|n| {
            let text = edits.iter().find(|(line, _)| *line == n).map(|(_, t)| *t);
            format!("{}\n", text.map_or(format!("line {n}"), str::to_owned))
        })
        .collect()
}

#[test]
fn unstaged_revert_restores_the_working_tree_and_keeps_the_index() -> Result<()> {
    let repo = Repo::new()?;
    repo.commit("a.txt", "base\n")?;
    repo.write("a.txt", "index\n")?;
    repo.git(&["add", "a.txt"])?;
    repo.write("a.txt", "worktree\n")?;
    let body = repo.revert("unstaged", "a.txt", None)?;
    ensure!(body["status"] == "success");
    ensure!(repo.read("a.txt")? == "index\n");
    ensure!(repo.git(&["show", ":a.txt"])? == "index\n");
    ensure!(body["unstaged"] == json!([]));
    ensure!(body["staged"][0]["path"] == "a.txt");

    repo.commit("gone.txt", "kept\n")?;
    fs::remove_file(repo.root().join("gone.txt"))?;
    repo.revert("unstaged", "gone.txt", None)?;
    ensure!(repo.read("gone.txt")? == "kept\n");

    repo.write("new.txt", "untracked\n")?;
    repo.write("other.txt", "untouched\n")?;
    let body = repo.revert("unstaged", "new.txt", None)?;
    ensure!(!repo.root().join("new.txt").exists());
    ensure!(repo.read("other.txt")? == "untouched\n");
    ensure!(body["unstaged"].as_array().is_some_and(|f| f.len() == 1));
    Ok(())
}

#[test]
fn staged_revert_restores_the_index_then_the_working_tree() -> Result<()> {
    let repo = Repo::new()?;
    repo.commit("a.txt", "base\n")?;
    repo.write("a.txt", "index\n")?;
    repo.git(&["add", "a.txt"])?;
    let body = repo.revert("staged", "a.txt", None)?;
    ensure!(body["status"] == "success");
    ensure!(repo.git(&["show", ":a.txt"])? == "base\n");
    ensure!(repo.read("a.txt")? == "base\n");
    ensure!(repo.git(&["status", "--porcelain"])?.is_empty());

    repo.write("b.txt", "added\n")?;
    repo.git(&["add", "b.txt"])?;
    repo.revert("staged", "b.txt", None)?;
    ensure!(!repo.root().join("b.txt").exists());
    ensure!(!repo.git(&["ls-files"])?.contains("b.txt"));
    Ok(())
}

#[test]
fn staged_revert_reports_a_partial_result_and_keeps_user_edits() -> Result<()> {
    let repo = Repo::new()?;
    repo.commit("a.txt", "base\n")?;
    repo.write("a.txt", "index\n")?;
    repo.git(&["add", "a.txt"])?;
    repo.write("a.txt", "worktree\n")?;
    let body = repo.revert("staged", "a.txt", None)?;
    ensure!(body["status"] == "partial-success");
    ensure!(repo.git(&["show", ":a.txt"])? == "base\n");
    ensure!(repo.read("a.txt")? == "worktree\n");
    ensure!(body["staged"] == json!([]));
    ensure!(body["unstaged"][0]["path"] == "a.txt");
    Ok(())
}

#[test]
fn hunk_revert_changes_only_the_chosen_hunk() -> Result<()> {
    let repo = Repo::new()?;
    repo.commit("a.txt", &lines(&[]))?;
    let edited = lines(&[(2, "early"), (18, "late")]);
    repo.write("a.txt", &edited)?;
    ensure!(repo.shown("unstaged", "a.txt")?.matches("\n@@ ").count() == 2);
    let body = repo.revert("unstaged", "a.txt", Some(1))?;
    ensure!(body["status"] == "success");
    ensure!(repo.read("a.txt")? == lines(&[(2, "early")]));

    repo.write("a.txt", &edited)?;
    repo.git(&["add", "a.txt"])?;
    repo.revert("staged", "a.txt", Some(0))?;
    ensure!(repo.git(&["show", ":a.txt"])? == lines(&[(18, "late")]));
    ensure!(repo.read("a.txt")? == lines(&[(18, "late")]));

    let error = repo.revert("staged", "a.txt", Some(3)).unwrap_err();
    ensure!(error.to_string().contains("hunk is no longer"), "{error}");
    Ok(())
}

#[test]
fn staged_revert_never_moves_onto_a_look_alike_block() -> Result<()> {
    let repo = Repo::new()?;
    let block = |word: &str| format!("a1\na2\na3\n{word}\na4\na5\na6\n");
    let filler = "f\n".repeat(10);
    let file = |first: &str, second: &str| format!("{}{filler}{}", block(first), block(second));
    repo.commit("a.txt", &file("old", "old"))?;
    repo.write("a.txt", &file("new", "old"))?;
    repo.git(&["add", "a.txt"])?;
    repo.write("a.txt", &file("custom", "new"))?;
    let body = repo.revert("staged", "a.txt", None)?;
    ensure!(body["status"] == "partial-success");
    ensure!(repo.git(&["show", ":a.txt"])? == file("old", "old"));
    ensure!(
        repo.read("a.txt")? == file("custom", "new"),
        "user edit lost"
    );
    Ok(())
}

#[test]
fn hunk_revert_keeps_the_file_mode() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let repo = Repo::new()?;
    repo.commit("run.sh", &lines(&[]))?;
    repo.write("run.sh", &lines(&[(2, "early"), (18, "late")]))?;
    fs::set_permissions(
        repo.root().join("run.sh"),
        fs::Permissions::from_mode(0o755),
    )?;
    let executable = || -> Result<bool> {
        Ok(fs::metadata(repo.root().join("run.sh"))?
            .permissions()
            .mode()
            & 0o111
            != 0)
    };
    repo.revert("unstaged", "run.sh", Some(1))?;
    ensure!(repo.read("run.sh")? == lines(&[(2, "early")]));
    ensure!(executable()?, "unstaged hunk revert dropped the mode");
    repo.git(&["add", "run.sh"])?;
    repo.revert("staged", "run.sh", Some(0))?;
    ensure!(repo.read("run.sh")? == lines(&[]));
    ensure!(executable()?, "staged hunk revert dropped the working mode");
    ensure!(
        repo.git(&["ls-files", "-s", "run.sh"])?
            .starts_with("100755")
    );
    Ok(())
}

#[test]
fn binary_files_revert_whole() -> Result<()> {
    let repo = Repo::new()?;
    repo.commit("img.bin", "old\0data\n")?;
    repo.write("img.bin", "new\0data\n")?;
    repo.revert("unstaged", "img.bin", None)?;
    ensure!(repo.read("img.bin")? == "old\0data\n");
    repo.write("img.bin", "new\0data\n")?;
    repo.git(&["add", "img.bin"])?;
    let body = repo.revert("staged", "img.bin", None)?;
    ensure!(body["status"] == "success");
    ensure!(repo.read("img.bin")? == "old\0data\n");
    ensure!(repo.git(&["status", "--porcelain"])?.is_empty());
    repo.write("fresh.bin", "x\0y\n")?;
    repo.revert("unstaged", "fresh.bin", None)?;
    ensure!(!repo.root().join("fresh.bin").exists());
    Ok(())
}

#[test]
fn revert_refuses_stale_input_and_reports_git_failures() -> Result<()> {
    let repo = Repo::new()?;
    repo.commit("a.txt", "base\n")?;
    repo.write("a.txt", "shown\n")?;
    let shown = repo.shown("unstaged", "a.txt")?;
    repo.write("a.txt", "edited after display\n")?;
    let stale = json!({"path": "a.txt", "source": "unstaged", "diff": shown});
    let error = repo.call("git-revert", stale).unwrap_err();
    ensure!(error.to_string().contains("changed since"), "{error}");
    ensure!(repo.read("a.txt")? == "edited after display\n");

    repo.write("a.txt", "staged\n")?;
    repo.git(&["add", "a.txt"])?;
    fs::write(repo.root().join(".git/index.lock"), "owned proof lock")?;
    let error = repo.revert("staged", "a.txt", None).unwrap_err();
    ensure!(error.to_string().contains("index.lock"), "{error}");
    ensure!(repo.read("a.txt")? == "staged\n");
    fs::remove_file(repo.root().join(".git/index.lock"))?;
    ensure!(repo.git(&["show", ":a.txt"])? == "staged\n");

    let diff = repo.shown("staged", "a.txt")?;
    let bad = |params: Value| repo.call("git-revert", params).is_err();
    ensure!(bad(
        json!({"path": "a.txt", "source": "head", "diff": diff})
    ));
    ensure!(bad(json!({"path": "a.txt", "source": "staged"})));
    ensure!(bad(
        json!({"path": "../a.txt", "source": "staged", "diff": diff})
    ));
    ensure!(bad(
        json!({"path": "a.txt", "source": "staged", "diff": diff, "hunk": "0"})
    ));
    ensure!(bad(
        json!({"path": "missing.txt", "source": "staged", "diff": diff})
    ));
    ensure!(repo.read("a.txt")? == "staged\n");
    ensure!(repo.git(&["show", ":a.txt"])? == "staged\n");
    Ok(())
}
