//! The Files tab against the original file editor (`refs/files-*`, route1 captures of
//! `proj/cgx-b-panel-files`): the Markdown preview, `Find in file…`, file tabs and the `Open`
//! menu, each at 0 px outside the text boxes. Metal and the Mac's fonts: macOS only.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use std::path::Path;
use ui::screens::diff::panel::OpenFile;

#[test]
fn proxy() {
    proxy_child();
}

const FILES: [(&str, &str); 3] = [
    (
        "README.md",
        "# Panel files\n\nThe **Files** tab renders Markdown.\n\n- find in the file\n- view the source\n- copy the Markdown\n\n```rust\nfn main() {}\n```\n",
    ),
    (
        "notes.txt",
        "the first line\nanother line\nthe third line mentions the word the\nlast\n",
    ),
    ("other.txt", "other\n"),
];

/// The subtract thread with `cgx-b-panel-files/<name>` open in the side panel.
fn open(fx: &mut Fx, name: &str) -> Result<()> {
    fx.action(Box::new(OpenFile {
        path: format!("cgx-b-panel-files/{name}"),
        line: None,
    }))?;
    fx.wait(name, |fx| {
        fx.read(|r, cx| {
            r.file_view().is_some_and(|f| {
                let f = f.read(cx);
                f.name() == name && matches!(f.contents, Some(Ok(_)))
            })
        })
    })?;
    recorded_targets(fx)
}

/// The original's recorded `openIn.getTargets` (Zed preferred), after this Mac's own answer.
fn recorded_targets(fx: &mut Fx) -> Result<()> {
    fx.cx.run_until_parked();
    let file = fx.read(|r, _| r.file_view().cloned()).expect("file view");
    fx.cx.update_window(fx.window, |_, window, cx| {
        file.update(cx, |file, cx| {
            let catalog =
                serde_json::from_str(include_str!("../../../fixtures/open-in-targets.json"))
                    .map_err(|e| e.to_string());
            file.show_open_targets(catalog, window, cx)
        })
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

fn thread() -> Result<Fx> {
    let dir = Path::new(CWD).join("cgx-b-panel-files");
    std::fs::create_dir_all(&dir)?;
    for (name, text) in FILES {
        std::fs::write(dir.join(name), text)?;
    }
    let mut fx = Fx::open("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    Ok(fx)
}

#[test]
fn files_tab_matches_the_original() -> Result<()> {
    let mut notes = Vec::new();
    let mut fx = thread()?;
    open(&mut fx, "README.md")?;
    fx.capture_evidence("CGX_FILES_SHOTS", "files-markdown")?;
    notes.push(fx.parity("files-markdown"));
    // The chevron of the `Open` split button lists the targets.
    let chevron = fx
        .cx
        .debug_bounds(fx.window, "file-open-options")?
        .map(|b| (f32::from(b.center().x), f32::from(b.center().y)));
    fx.click(chevron.context("no Open options button")?)?;
    recorded_targets(&mut fx)?;
    fx.capture_evidence("CGX_FILES_SHOTS", "files-open")?;
    notes.push(fx.parity("files-open"));

    // `notes.txt` pinned by a click in its text, then `other.txt`, which `README.md` replaces.
    let mut fx = thread()?;
    open(&mut fx, "notes.txt")?;
    fx.click((900., 300.))?;
    open(&mut fx, "other.txt")?;
    open(&mut fx, "README.md")?;
    // The capture's resting pointer is over `notes.txt`: its hover card shows.
    fx.hover(((1280. * 0.6_f32).round(), 6.))?;
    fx.capture_evidence("CGX_FILES_SHOTS", "files-tabs")?;
    notes.push(fx.parity("files-tabs"));

    // `Find in file…` for `the`, then the next match.
    let mut fx = thread()?;
    open(&mut fx, "notes.txt")?;
    fx.click((900., 300.))?;
    fx.keys("cmd-f")?;
    fx.type_text("the")?;
    fx.keys("enter")?;
    fx.capture_evidence("CGX_FILES_SHOTS", "files-find")?;
    notes.push(fx.parity("files-find"));
    let failed = notes.iter().any(Result::is_err);
    let notes: Vec<String> = notes
        .into_iter()
        .map(|n| n.unwrap_or_else(|e| format!("{e:#}")))
        .collect();
    println!("{}", notes.join("\n"));
    ensure!(!failed, "{}", notes.join("\n"));
    Ok(())
}
