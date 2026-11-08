use crate::host::{Request, json_body};
use crate::{Root, Toast};
use anyhow::{Context as _, Result, bail, ensure};
use bridge::types::{FileUpdateChange, ThreadItem, Turn};
use gpui::{AppContext as _, Context};
use serde_json::json;
use std::fmt::Write as _;
use std::path::{Component, Path};
use ui::screens::thread::view::PatchTurn;

impl Root {
    pub(crate) fn patch_turn(&mut self, action: &PatchTurn, cx: &mut Context<Self>) {
        let Some(view) = self.thread.clone() else {
            return;
        };
        let state = view.read(cx);
        if state.id() != action.thread_id {
            return;
        }
        let Some(turn) = state
            .turns()
            .iter()
            .find(|turn| turn.id == action.turn_id)
            .cloned()
        else {
            return;
        };
        let cwd = state.cwd().to_owned();
        let Some(revert) = view.update(cx, |view, cx| view.begin_patch(&turn.id, cx)) else {
            return;
        };
        let host = self.host.clone();
        let id = turn.id.clone();
        let work =
            cx.background_spawn(async move {
                let repository =
                    json_body(&host.fetch(&Request::post("git-root", Some(json!({"cwd": cwd}))))?)?;
                let root = Path::new(repository["root"].as_str().context("missing Git root")?);
                let mut patches = turn_patches(&turn, Path::new(&cwd), root)?;
                if revert {
                    patches.reverse();
                }
                for diff in patches {
                    let result = json_body(&host.fetch(&Request::post("apply-patch", Some(json!({
                    "cwd": cwd, "diff": diff, "revert": revert, "operationSource": "thread_diff"
                }))))?)?;
                    ensure!(result["status"] == "success", "{result}");
                }
                Ok::<_, anyhow::Error>(())
            });
        let task = cx.spawn(async move |root, cx| {
            let result = work.await;
            view.update(cx, |view, cx| {
                view.finish_patch(&id, revert, result.is_ok(), cx)
            });
            root.update(cx, |root, cx| {
                let message = match (revert, result.is_ok()) {
                    (true, true) => "Changes reverted",
                    (false, true) => "Changes reapplied",
                    (true, false) => "Failed to revert changes",
                    (false, false) => "Failed to reapply changes",
                };
                root.toast(
                    if result.is_ok() {
                        Toast::success(message)
                    } else {
                        Toast::danger(message)
                    },
                    cx,
                );
            })?;
            result
        });
        crate::log_err(task, cx);
    }
}

fn turn_patches(turn: &Turn, cwd: &Path, root: &Path) -> Result<Vec<String>> {
    let mut patches = Vec::new();
    for item in &turn.items {
        let ThreadItem::FileChange { changes, .. } = item else {
            continue;
        };
        let mut patch = String::new();
        for change in changes {
            file_patch(&mut patch, change, cwd, root)?;
        }
        if !patch.is_empty() {
            patches.push(patch);
        }
    }
    ensure!(!patches.is_empty(), "turn has no file changes");
    Ok(patches)
}

fn patch_path(path: &str, cwd: &Path, root: &Path) -> Result<String> {
    let absolute = cwd.join(path);
    let absolute = if !absolute.starts_with(root) && absolute.starts_with(cwd) {
        cwd.canonicalize()?.join(absolute.strip_prefix(cwd)?)
    } else {
        absolute
    };
    let relative = absolute
        .strip_prefix(root)
        .context("file is outside the Git repository")?;
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "invalid patch path"
    );
    Ok(relative
        .to_str()
        .context("patch path is not UTF-8")?
        .to_owned())
}

fn quote(path: &str) -> String {
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b' '..=b'~' if byte != b'"' && byte != b'\\' => quoted.push(char::from(byte)),
            _ => write!(quoted, "\\{byte:03o}").unwrap(),
        }
    }
    quoted.push('"');
    quoted
}

fn file_patch(
    patch: &mut String,
    change: &FileUpdateChange,
    cwd: &Path,
    root: &Path,
) -> Result<()> {
    let path = patch_path(&change.path, cwd, root)?;
    let target = change.kind["move_path"]
        .as_str()
        .map(|path| patch_path(path, cwd, root))
        .transpose()?
        .unwrap_or_else(|| path.clone());
    let old = quote(&format!("a/{path}"));
    let new = quote(&format!("b/{target}"));
    writeln!(patch, "diff --git {old} {new}")?;
    match change.kind["type"].as_str() {
        Some("update") => {
            if path != target {
                writeln!(
                    patch,
                    "rename from {}\nrename to {}",
                    quote(&path),
                    quote(&target)
                )?;
            }
            if !change.diff.is_empty() {
                writeln!(patch, "--- {old}\n+++ {new}")?;
                patch.push_str(&change.diff);
                if !change.diff.ends_with('\n') {
                    patch.push('\n');
                }
            }
        }
        Some(kind @ ("add" | "delete")) => {
            let add = kind == "add";
            writeln!(
                patch,
                "{} file mode 100644",
                if add { "new" } else { "deleted" }
            )?;
            if !change.diff.is_empty() {
                writeln!(
                    patch,
                    "--- {}\n+++ {}",
                    if add { "/dev/null" } else { &old },
                    if add { &new } else { "/dev/null" }
                )?;
                let count = change.diff.split_terminator('\n').count();
                if add {
                    writeln!(patch, "@@ -0,0 +1,{count} @@")?;
                } else {
                    writeln!(patch, "@@ -1,{count} +0,0 @@")?;
                }
                for line in change.diff.split_terminator('\n') {
                    writeln!(patch, "{}{line}", if add { '+' } else { '-' })?;
                }
                if !change.diff.ends_with('\n') {
                    patch.push_str("\\ No newline at end of file\n");
                }
            }
        }
        _ => bail!("unsupported file change kind"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Host, LocalHost};
    use std::{fs, process::Command};

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)] // the fixture path holds a quote, which Windows forbids
    #[test]
    fn patches_preserve_paths_content_and_index_in_both_directions() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q"]);
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/edit.txt"), "before\n").unwrap();
        fs::write(root.join("deleted.txt"), "deleted without newline").unwrap();
        fs::write(root.join("old name.txt"), "rename\n").unwrap();
        git(root, &["add", "."]);
        git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
        );
        let index = fs::read(root.join(".git/index")).unwrap();
        let host = LocalHost::new(root.join(".git/profile"));
        let changes = [
            FileUpdateChange {
                path: root.join("nested/edit.txt").to_str().unwrap().into(),
                kind: json!({"type": "update"}),
                diff: "@@ -1 +1 @@\n-before\n+after\n".into(),
            },
            FileUpdateChange {
                path: root.join("new \"é\".txt").to_str().unwrap().into(),
                kind: json!({"type": "add"}),
                diff: "added without newline".into(),
            },
            FileUpdateChange {
                path: root.join("deleted.txt").to_str().unwrap().into(),
                kind: json!({"type": "delete"}),
                diff: "deleted without newline".into(),
            },
            FileUpdateChange {
                path: root.join("old name.txt").to_str().unwrap().into(),
                kind: json!({"type": "update", "move_path": root.join("new name.txt")}),
                diff: String::new(),
            },
            FileUpdateChange {
                path: root.join("crlf.txt").to_str().unwrap().into(),
                kind: json!({"type": "add"}),
                diff: "first\r\nsecond\r\n".into(),
            },
        ];
        let mut patch = String::new();
        for change in changes {
            file_patch(&mut patch, &change, &root.join("nested"), root).unwrap();
        }
        for revert in [false, true, false] {
            let result = json_body(
                &host
                    .fetch(&Request::post(
                        "apply-patch",
                        Some(json!({
                            "cwd":root, "diff":patch, "revert":revert
                        })),
                    ))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(result["status"], "success", "{result}");
            assert_eq!(
                fs::read_to_string(root.join("nested/edit.txt")).unwrap(),
                if revert { "before\n" } else { "after\n" }
            );
            assert_eq!(root.join("deleted.txt").exists(), revert);
            assert_eq!(root.join("old name.txt").exists(), revert);
            assert_eq!(root.join("new name.txt").exists(), !revert);
            if !revert {
                assert_eq!(
                    fs::read_to_string(root.join("new \"é\".txt")).unwrap(),
                    "added without newline"
                );
                assert_eq!(
                    fs::read(root.join("crlf.txt")).unwrap(),
                    b"first\r\nsecond\r\n"
                );
            }
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        }
    }

    #[test]
    fn rejects_paths_outside_the_repository() {
        let root = Path::new("/repo");
        for path in ["/elsewhere/a.txt", "../outside.txt", "/repo/../outside.txt"] {
            assert!(patch_path(path, root, root).is_err(), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolves_a_workspace_alias_for_a_new_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert_eq!(
            patch_path(alias.join("new.txt").to_str().unwrap(), &alias, &root).unwrap(),
            "new.txt"
        );
    }
}
