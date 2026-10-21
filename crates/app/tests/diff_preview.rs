#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        result.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn output() -> Result<PathBuf> {
    let path = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-diff-preview"));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn pdf() -> Vec<u8> {
    let streams = [
        "1 0 0 rg 0 100 200 100 re f\n",
        "0 0 1 rg 0 0 200 100 re f\n",
    ];
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents 4 0 R >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            streams[0].len(),
            streams[0]
        ),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents 6 0 R >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            streams[1].len(),
            streams[1]
        ),
    ];
    let mut bytes = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let start = bytes.len();
    bytes.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn set_patch(fx: &mut Fx, patch: &str) -> Result<()> {
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
            bridge::Event::Notification {
                method: "turn/diff/updated".into(),
                params: json!({"threadId":SUBTRACT,"turnId":turn,"diff":patch}),
            },
            cx,
        )
    });
    fx.reference_shot()?;
    Ok(())
}

fn page(fx: &Fx, side: usize) -> Option<(usize, usize)> {
    fx.read(|root, cx| root.diff_panel()?.read(cx).preview_page(0, side))
}

fn toggle_preview(fx: &mut Fx) -> Result<()> {
    fx.click((1192., 68.))?;
    let option = fx
        .read(|root, cx| {
            root.diff_panel()?
                .read(cx)
                .menu_items()
                .position(|(label, enabled)| label == "Rich preview" && enabled)
        })
        .context("Rich preview option")?;
    fx.keys("home")?;
    for _ in 0..option {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

fn colored(image: &image::RgbaImage, x: u32, y: u32, channel: usize) -> bool {
    let pixel = image.get_pixel(x * 2, y * 2).0;
    pixel[channel] > 240 && pixel[(channel + 1) % 3] < 15 && pixel[(channel + 2) % 3] < 15
}

#[test]
fn changed_images_and_pdf_pages_use_real_file_reads() -> Result<()> {
    common::install_panic_hook();
    let output = output()?;
    let repo = tempfile::tempdir_in(&output)?;
    git(repo.path(), &["init", "-q"])?;
    git(repo.path(), &["config", "user.name", "Preview proof"])?;
    git(
        repo.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    let image_path = repo.path().join("picture.png");
    image::RgbaImage::from_pixel(128, 64, image::Rgba([255, 0, 0, 255])).save(&image_path)?;
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "before"])?;
    image::RgbaImage::from_pixel(128, 64, image::Rgba([0, 0, 255, 255])).save(&image_path)?;
    fs::write(repo.path().join("pages.pdf"), pdf())?;
    git(repo.path(), &["add", "-N", "pages.pdf"])?;
    let image_patch = git(repo.path(), &["diff", "--full-index", "--", "picture.png"])?;
    let pdf_patch = git(repo.path(), &["diff", "--full-index", "--", "pages.pdf"])?;
    let mut recording = common::script(None, |_| {})?;
    for row in &mut recording {
        if let Some(thread) = row.pointer_mut("/msg/result/thread") {
            thread["cwd"] = repo.path().to_string_lossy().into_owned().into();
        }
    }
    let mut fx = Fx::scripted(recording)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    set_patch(&mut fx, &image_patch)?;
    fx.wait("before and after images", |fx| {
        page(fx, 0) == Some((1, 1)) && page(fx, 1) == Some((1, 1))
    })?;
    let image = fx.reference_shot()?;
    image.save(output.join("image-before-after.png"))?;
    ensure!(
        colored(&image, 812, 209, 0),
        "The previous image was not red"
    );
    ensure!(
        colored(&image, 1118, 209, 2),
        "The working image was not blue"
    );

    toggle_preview(&mut fx)?;
    set_patch(&mut fx, &pdf_patch)?;
    fx.wait("first PDF page", |fx| page(fx, 1) == Some((1, 2)))?;
    let image = fx.reference_shot()?;
    image.save(output.join("pdf-first-page.png"))?;
    ensure!(
        colored(&image, 950, 250, 0),
        "The first PDF page is not upright"
    );
    fx.click((1258., 144.))?;
    fx.wait("next PDF page", |fx| page(fx, 1) == Some((2, 2)))?;
    let image = fx.reference_shot()?;
    image.save(output.join("pdf-next-page.png"))?;
    ensure!(
        colored(&image, 950, 650, 2),
        "The second PDF page is not upright"
    );
    fx.click((1209., 144.))?;
    fx.wait("previous PDF page", |fx| page(fx, 1) == Some((1, 2)))?;

    toggle_preview(&mut fx)?;
    ensure!(
        !fx.read(|root, cx| root.diff_panel().unwrap().read(cx).shows_rich_preview()),
        "Rich preview stayed enabled"
    );
    ensure!(
        page(&fx, 1) == Some((1, 2)),
        "The PDF preview must remain visible with rich preview disabled"
    );
    fx.reference_shot()?
        .save(output.join("pdf-rich-preview-off.png"))?;
    Ok(())
}
