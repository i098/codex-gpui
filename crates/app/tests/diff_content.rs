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
                oid: None,
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
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "oversized before file"])?;
    let mut before = one_side(root, "file.txt", None, false);
    before.before = Some(0);
    before.after = None;
    ensure!(read_full_file(&host, &before).err() == Some(FullError::Unavailable));
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

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use bridge::Event;
    use common::{Fx, SUBTRACT};
    use serde_json::json;

    #[test]
    fn restored_wrapped_context_keeps_rows_and_headers_in_bounds() -> Result<()> {
        let repo = repository()?;
        let root = repo.path();
        let suffix = " context".repeat(40);
        let before = (1..=80)
            .map(|line| format!("line {line}{suffix}\n"))
            .collect::<String>();
        fs::write(root.join("file.txt"), &before)?;
        fs::write(root.join("zz.txt"), "tail\n".repeat(100))?;
        git(root, &["add", "."])?;
        git(root, &["commit", "-qm", "before"])?;
        fs::write(
            root.join("file.txt"),
            before.replacen("line 40", "changed 40", 1),
        )?;
        fs::write(root.join("zz.txt"), "changed tail\n".repeat(100))?;
        let patch = git(root, &["diff", "--full-index"])?
            .lines()
            .filter(|line| !line.starts_with("index "))
            .collect::<Vec<_>>()
            .join("\n");
        let mut recording = common::script(None, |_| {})?;
        for row in &mut recording {
            if let Some(thread) = row.pointer_mut("/msg/result/thread") {
                thread["cwd"] = root.to_string_lossy().into_owned().into();
            }
        }
        let mut fx = Fx::scripted(recording)?;
        fx.keys("cmd-2")?;
        fx.wait_thread(SUBTRACT)?;
        fx.keys("alt-cmd-b")?;
        let turn = fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .unwrap()
                .id
                .clone()
        });
        fx.root.update(&mut fx.cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: "turn/diff/updated".into(),
                    params: json!({"threadId":SUBTRACT,"turnId":turn,"diff":patch}),
                },
                cx,
            )
        });
        fx.cx.run_until_parked();
        fx.resize(1440., 900.)?;
        fx.click((1314., 68.))?;
        ensure!(fx.read(|root, cx| root.diff_panel().unwrap().read(cx).wraps_lines()));
        fx.resize(1280., 832.)?;
        let marker = |image: &image::RgbaImage| {
            (0..image.height()).find(|y| {
                let pixel = image.get_pixel(1325, *y);
                pixel[0] > 200 && pixel[1] < 100 && pixel[2] < 100
            })
        };
        let before_image = fx.reference_shot()?;
        let before_y =
            marker(&before_image).ok_or_else(|| anyhow::anyhow!("missing changed line"))?;
        let output = std::env::var_os("CGX_FULL_CONTEXT_OUT").map(std::path::PathBuf::from);
        if let Some(output) = &output {
            fs::create_dir_all(output)?;
            before_image.save(output.join("before.png"))?;
        }
        fx.click((786., 148.))?;
        fx.keys("cmd-f")?;
        fx.type_text("line 20")?;
        fx.wait("expanded context is searchable", |fx| {
            fx.read(|root, cx| {
                root.diff_panel()
                    .unwrap()
                    .read(cx)
                    .search()
                    .is_some_and(|(_, matches)| matches == 1)
            })
        })?;
        ensure!(fx.read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .full_file_error(0)
                .is_none()
        }));
        fx.keys("escape")?;
        let after_image = fx.reference_shot()?;
        let after_y = marker(&after_image)
            .ok_or_else(|| anyhow::anyhow!("changed line left the viewport"))?;
        ensure!(
            before_y.abs_diff(after_y) <= 1,
            "wrapped context moved the reviewed line"
        );
        if let Some(output) = &output {
            after_image.save(output.join("expanded.png"))?;
        }
        fx.click((1000., 110.))?;
        fx.scroll((1000., 200.), 100_000.)?;
        let before_scroll = fx.reference_shot()?;
        fx.scroll((1000., 200.), -31.)?;
        let after_scroll = fx.reference_shot()?;
        ensure!(
            image::imageops::crop_imm(&before_scroll, 1323, 88, 1229, 96).to_image()
                == image::imageops::crop_imm(&after_scroll, 1323, 88, 1229, 96).to_image(),
            "a sticky file header painted outside the scroll viewport"
        );
        Ok(())
    }
}
