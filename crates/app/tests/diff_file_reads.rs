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

fn exported_bytes(value: &Value) -> Result<Vec<u8>> {
    Ok(STANDARD.decode(
        value["contentsBase64"]
            .as_str()
            .context("exported patch bytes")?,
    )?)
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
    git(root, &["branch", "fixture-base"])?;
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
    let root_commit = read("git-changes", json!({"cwd":root,"reviewSource":"commits"}))?;
    ensure!(root_commit["before_ref"].is_null());
    let absent = read(
        "read-git-file-binary",
        json!({
            "cwd":root,"path":"bytes.dat","ref":root_commit["before_ref"],
        }),
    )?;
    ensure!(absent["contentsBase64"].is_null());
    git(root, &["commit", "-qm", "changed"])?;
    fs::write(&path, [0, 5, 5, 5])?;
    for source in ["branch", "commits"] {
        let changes = read(
            "git-changes",
            json!({
                "cwd": root, "reviewSource": source, "baseBranch": "fixture-base",
            }),
        )?;
        for (field, expected) in [("before_ref", [0, 1, 2, 3]), ("after_ref", [0, 9, 8, 7])] {
            let version = read(
                "read-git-file-binary",
                json!({
                    "cwd": root, "path": "bytes.dat", "ref": changes[field],
                }),
            )?;
            ensure!(
                STANDARD.decode(
                    version["contentsBase64"]
                        .as_str()
                        .context("revision bytes")?
                )? == expected,
                "{source} {field} selected the wrong image version",
            );
        }
    }
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

#[test]
fn file_reads_preserve_text_and_identify_image_formats() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    let host = LocalHost::new(root);
    let text = "line α\nsecond\n";
    let text_path = root.join("file.txt");
    fs::write(&text_path, text)?;
    let value = json_body(&host.fetch(&Request::post(
        "read-file",
        Some(json!({
            "cwd":root,"path":text_path,
        })),
    ))?)?;
    ensure!(value["contents"] == text);
    let webp = STANDARD.decode("UklGRhwAAABXRUJQVlA4TA8AAAAvAAAAAAcQ/Y/+ByKi/wEA")?;
    for (name, bytes, mime) in [
        (
            "image.png",
            &include_bytes!("../../ui/assets/avatar.png")[..],
            "image/png",
        ),
        ("image.webp", webp.as_slice(), "image/webp"),
        (
            "image.SVG",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"></svg>".as_slice(),
            "image/svg+xml",
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, bytes)?;
        let value = json_body(&host.fetch(&Request::post(
            "read-file-binary",
            Some(json!({
                "cwd":root,"path":path,
            })),
        ))?)?;
        ensure!(value["mimeType"] == mime, "Wrong MIME type for {name}");
        ensure!(
            STANDARD.decode(value["contentsBase64"].as_str().context("image bytes")?)? == bytes
        );
    }
    Ok(())
}

#[test]
fn github_file_links_use_the_original_branch_and_path_rules() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let canonical = repo.path().canonicalize()?;
    let root = canonical.as_path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Link proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    fs::create_dir(root.join("src"))?;
    fs::write(root.join("src/a b#c.rs"), "fn value() {}\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    git(root, &["checkout", "-qb", "feature/link-encoding"])?;
    let host = LocalHost::new(root);
    let request = Request::post(
        "git-file-url",
        Some(json!({
            "cwd":root, "path":root.join("src/a b#c.rs"), "line":7,
        })),
    );
    ensure!(json_body(&host.fetch(&request)?)?["url"].is_null());
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:example/project.git",
        ],
    )?;
    for remote in [
        "git@github.com:example/project.git",
        "https://github.com/example/project.git",
        "ssh://git@github.com/example/project.git",
        "https://github.com/example/project",
    ] {
        git(root, &["remote", "set-url", "origin", remote])?;
        ensure!(
            json_body(&host.fetch(&request)?)?["url"]
                == "https://github.com/example/project/blob/feature/link-encoding/src/a%20b%23c.rs#L7",
            "Wrong GitHub URL for {remote}",
        );
    }
    let untracked = Request::post(
        "git-file-url",
        Some(json!({"cwd":root,"path":"src/../new.txt","line":1})),
    );
    ensure!(
        json_body(&host.fetch(&untracked)?)?["url"]
            == "https://github.com/example/project/blob/feature/link-encoding/new.txt#L1"
    );
    let outside = Request::post(
        "git-file-url",
        Some(json!({"cwd":root,"path":"../outside.txt"})),
    );
    ensure!(json_body(&host.fetch(&outside)?)?["url"].is_null());
    git(
        root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://gitlab.com/example/project.git",
        ],
    )?;
    ensure!(json_body(&host.fetch(&request)?)?["url"].is_null());
    git(
        root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/example/project.git",
        ],
    )?;
    git(root, &["checkout", "--detach", "-q", "HEAD"])?;
    ensure!(json_body(&host.fetch(&request)?)?["url"].is_null());
    Ok(())
}

#[test]
fn whitespace_filtered_review_still_exports_the_complete_patch() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Patch proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(root.join("spacing.txt"), "value = 1\n")?;
    fs::write(root.join("value.txt"), "value = 1\n")?;
    fs::write(root.join("bytes.bin"), [0, 1, 2, 3])?;
    fs::write(root.join("latin1.txt"), b"caf\xe9\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    fs::write(root.join("spacing.txt"), "value  =  1\n")?;
    git(root, &["add", "spacing.txt"])?;
    fs::write(root.join("value.txt"), "value = 2\n")?;
    fs::write(root.join("bytes.bin"), [0, 9, 8, 7])?;
    fs::write(root.join("latin1.txt"), b"th\xe9\n")?;
    fs::write(root.join("new file.txt"), "new content\n")?;
    let host = LocalHost::new(root);
    let review = json_body(&host.fetch(&Request::post(
        "git-changes",
        Some(json!({
            "cwd":root, "reviewSource":"uncommitted", "hideWhitespace":true,
        })),
    ))?)?;
    ensure!(
        review["staged"]
            .as_array()
            .context("staged files")?
            .is_empty()
    );
    let names = review["review"]
        .as_array()
        .context("review files")?
        .iter()
        .map(|file| {
            Path::new(file["path"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(
        names
            == ["bytes.bin", "latin1.txt", "new file.txt", "value.txt"]
                .map(String::from)
                .into_iter()
                .collect()
    );
    let exported = json_body(&host.fetch(&Request::post(
        "git-review-patch",
        Some(json!({
            "cwd":root, "reviewSource":"uncommitted",
        })),
    ))?)?;
    let destination = tempfile::tempdir()?;
    let clone = destination.path().join("copy");
    git(
        root,
        &[
            "clone",
            "--quiet",
            "--no-hardlinks",
            root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    )?;
    let patch = destination.path().join("change.patch");
    fs::write(&patch, exported_bytes(&exported)?)?;
    git(&clone, &["apply", "--3way", patch.to_str().unwrap()])?;
    ensure!(fs::read(clone.join("bytes.bin"))? == [0, 9, 8, 7]);
    ensure!(fs::read(clone.join("latin1.txt"))? == b"th\xe9\n");
    ensure!(fs::read_to_string(clone.join("spacing.txt"))? == "value  =  1\n");
    ensure!(fs::read_to_string(clone.join("value.txt"))? == "value = 2\n");
    ensure!(fs::read_to_string(clone.join("new file.txt"))? == "new content\n");
    Ok(())
}

#[test]
fn raw_export_keeps_tracked_changes_when_untracked_files_exceed_the_limit() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Patch proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(root.join("tracked.txt"), "before\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    fs::write(root.join("tracked.txt"), "after\n")?;
    fs::create_dir(root.join("generated"))?;
    for index in 0..257 {
        fs::write(root.join(format!("generated/{index}.txt")), "generated\n")?;
    }
    let host = LocalHost::new(root);
    let params = json!({"cwd":root, "reviewSource":"uncommitted"});
    let exported = json_body(&host.fetch(&Request::post("git-review-patch", Some(params)))?)?;
    let destination = tempfile::tempdir()?;
    let clone = destination.path().join("copy");
    git(
        root,
        &[
            "clone",
            "--quiet",
            "--no-hardlinks",
            root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    )?;
    let patch = destination.path().join("change.patch");
    fs::write(&patch, exported_bytes(&exported)?)?;
    git(&clone, &["apply", "--3way", patch.to_str().unwrap()])?;
    ensure!(fs::read_to_string(clone.join("tracked.txt"))? == "after\n");
    ensure!(!clone.join("generated").exists());
    Ok(())
}

#[test]
fn review_exports_respect_index_and_commit_boundaries() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Patch proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(root.join("file.txt"), "base\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    let before = git(root, &["rev-parse", "HEAD"])?;
    fs::write(root.join("file.txt"), "committed\n")?;
    git(root, &["commit", "-qam", "changed"])?;
    let after = git(root, &["rev-parse", "HEAD"])?;
    fs::write(root.join("file.txt"), "staged\n")?;
    git(root, &["add", "."])?;
    fs::write(root.join("file.txt"), "working\n")?;
    let host = LocalHost::new(root);
    for (source, revision, expected) in [
        ("staged", after.as_str(), "staged\n"),
        ("unstaged", after.as_str(), "working\n"),
        ("branch", before.as_str(), "committed\n"),
        ("commits", before.as_str(), "committed\n"),
    ] {
        let exported = json_body(&host.fetch(&Request::post(
            "git-review-patch",
            Some(json!({
                "cwd":root, "reviewSource":source, "beforeRef":before, "afterRef":after,
            })),
        ))?)?;
        let destination = tempfile::tempdir()?;
        let clone = destination.path().join("copy");
        git(
            root,
            &[
                "clone",
                "--quiet",
                "--no-hardlinks",
                root.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        )?;
        git(&clone, &["reset", "--hard", revision])?;
        if source == "unstaged" {
            fs::write(clone.join("file.txt"), "staged\n")?;
            git(&clone, &["add", "."])?;
        }
        let patch = destination.path().join("change.patch");
        fs::write(&patch, exported_bytes(&exported)?)?;
        git(&clone, &["apply", "--3way", patch.to_str().unwrap()])?;
        ensure!(
            fs::read_to_string(clone.join("file.txt"))? == expected,
            "{source} used the wrong revision"
        );
    }
    ensure!(fs::read_to_string(root.join("file.txt"))? == "working\n");
    ensure!(git(root, &["show", ":file.txt"])? == "staged");
    Ok(())
}

#[test]
fn uncommitted_export_supports_a_repository_without_commits() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"])?;
    fs::write(repo.path().join("new.txt"), "new content\n")?;
    let host = LocalHost::new(repo.path());
    let exported = json_body(&host.fetch(&Request::post(
        "git-review-patch",
        Some(json!({"cwd":repo.path(),"reviewSource":"uncommitted"})),
    ))?)?;
    let destination = tempfile::tempdir()?;
    git(destination.path(), &["init", "-q"])?;
    let patch = destination.path().join("change.patch");
    fs::write(&patch, exported_bytes(&exported)?)?;
    git(
        destination.path(),
        &["apply", "--3way", patch.to_str().unwrap()],
    )?;
    ensure!(fs::read_to_string(destination.path().join("new.txt"))? == "new content\n");
    Ok(())
}

#[test]
fn filtered_mutations_preserve_hidden_whitespace_changes() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let base = (1..=30)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let spacing = base
        .replace("line 2\n", "line  2\n")
        .replace("line 24\n", "line  24\n");
    let modified = spacing.replace("line 25\n", "changed 25\n");
    let content_only = base.replace("line 25\n", "changed 25\n");
    for (endpoint, source, legacy) in [
        ("git-stage", "unstaged", false),
        ("git-unstage", "staged", false),
        ("git-revert", "unstaged", false),
        ("git-revert", "staged", false),
        ("git-revert", "unstaged", true),
        ("git-revert", "staged", true),
    ] {
        let repo = tempfile::tempdir()?;
        let root = repo.path();
        git(root, &["init", "-q"])?;
        git(root, &["config", "user.name", "Patch proof"])?;
        git(root, &["config", "user.email", "proof@example.invalid"])?;
        fs::write(root.join("file.txt"), &base)?;
        git(root, &["add", "."])?;
        git(root, &["commit", "-qm", "base"])?;
        fs::write(root.join("file.txt"), &modified)?;
        if source == "staged" {
            git(root, &["add", "."])?;
        }
        let host = LocalHost::new(root);
        let review = json_body(&host.fetch(&Request::post(
            "git-changes",
            Some(json!({"cwd":root,"hideWhitespace":true})),
        ))?)?;
        let diff = review[source][0]["diff"]
            .as_str()
            .context("filtered diff")?;
        let mut params = json!({"cwd":root,"source":source,"hideWhitespace":true});
        if legacy {
            params["path"] = "file.txt".into();
            params["diff"] = diff.into();
        } else {
            params["files"] = json!([{"path":"file.txt","diff":diff}]);
        }
        let response = json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)?;
        ensure!(
            response["status"] == "success",
            "{endpoint} {source} failed"
        );
        let expected_index = match (endpoint, source) {
            ("git-stage", _) => &content_only,
            (_, "staged") => &spacing,
            _ => &base,
        };
        let expected_working = if endpoint == "git-revert" {
            &spacing
        } else {
            &modified
        };
        ensure!(
            git(root, &["show", ":file.txt"])? == expected_index.trim_end(),
            "{endpoint} {source} changed hidden index whitespace"
        );
        ensure!(
            fs::read_to_string(root.join("file.txt"))? == *expected_working,
            "{endpoint} {source} changed hidden working whitespace"
        );
    }
    Ok(())
}

#[test]
fn filtered_mutations_still_reject_a_stale_snapshot() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Patch proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(root.join("file.txt"), "before\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    fs::write(root.join("file.txt"), "after\n")?;
    let host = LocalHost::new(root);
    let review = json_body(&host.fetch(&Request::post(
        "git-changes",
        Some(json!({"cwd":root,"hideWhitespace":true})),
    ))?)?;
    fs::write(root.join("file.txt"), "changed again\n")?;
    let error = host
        .fetch(&Request::post(
            "git-stage",
            Some(json!({"cwd":root,"source":"unstaged","hideWhitespace":true,
            "files":[{"path":"file.txt","diff":review["unstaged"][0]["diff"]}]})),
        ))
        .unwrap_err();
    ensure!(
        error
            .to_string()
            .contains("file changed since its diff was shown")
    );
    ensure!(git(root, &["show", ":file.txt"])? == "before");
    ensure!(fs::read_to_string(root.join("file.txt"))? == "changed again\n");
    Ok(())
}
