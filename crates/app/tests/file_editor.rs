//! PANEL-14: the Files tab edits its file (`textFileEditor`). A double-clicked line takes the
//! caret, `⌘S` writes through the host's `write-file`, and closing a tab whose save failed
//! asks `Discard file changes?` before the edit is dropped.
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::*;
use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point, px};
use std::{fs, path::Path, time::Duration};
use ui::screens::diff::panel::{CloseSideTab, OpenFile};

#[test]
fn proxy() {
    proxy_child();
}

fn double_click(fx: &mut Fx, (x, y): (f32, f32)) -> Result<()> {
    let position = point(px(x), px(y));
    for click_count in [1, 2] {
        fx.input(PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        }))?;
        fx.input(PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count,
        }))?;
    }
    Ok(())
}

/// The Mac recording's `/private/tmp` cwd is `/tmp` on Linux, where `/private` is absent.
fn unprivate(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) if text.starts_with("/private/tmp/") => {
            text.replace_range(.."/private".len(), "");
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(unprivate),
        serde_json::Value::Object(map) => map.values_mut().for_each(unprivate),
        _ => {}
    }
}

fn unsaved(fx: &Fx) -> bool {
    fx.read(|r, cx| {
        r.file_view()
            .is_some_and(|f| f.read(cx).has_unsaved_changes(cx))
    })
}

fn confirming(fx: &mut Fx) -> bool {
    fx.read(|r, cx| {
        r.file_view()
            .is_some_and(|f| f.read(cx).confirming_discard())
    })
}

#[test]
fn files_tab_saves_edits_and_asks_before_discarding() -> Result<()> {
    let dir = Path::new(CWD).join("cgx-file-editor");
    fs::create_dir_all(&dir)?;
    let path = dir.join("edit.txt");
    fs::write(&path, "one\ntwo\n")?;
    let mut fx = Fx::open("dark", (1280., 832.), script(None, unprivate)?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.action(Box::new(OpenFile {
        path: "cgx-file-editor/edit.txt".into(),
        line: None,
    }))?;
    fx.wait("file contents", |fx| {
        fx.read(|r, cx| r.file_view().is_some_and(|f| f.read(cx).contents.is_some()))
    })?;
    let contents = fx.read(|r, cx| r.file_view().and_then(|f| f.read(cx).contents.clone()));
    ensure!(matches!(contents, Some(Ok(_))), "{contents:?}");
    fx.capture_evidence("CGX_FILE_EDITOR_SHOTS", "1-read-view")?;
    let lines = fx
        .cx
        .debug_bounds(fx.window, "file-lines")?
        .context("no file lines")?;
    // The second 22 px row, past the line numbers.
    let second = (
        f32::from(lines.origin.x) + 80.,
        f32::from(lines.origin.y) + 33.,
    );
    double_click(&mut fx, second)?;
    fx.type_text("X")?;
    ensure!(unsaved(&fx), "typing did not edit the file");
    fx.capture_evidence("CGX_FILE_EDITOR_SHOTS", "2-editing")?;
    fx.keys(if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    })?;
    fx.wait("saved", |fx| {
        fs::read_to_string(&path).is_ok_and(|text| text == "one\nXtwo\n") && !unsaved(fx)
    })?;

    // An agent writes the file after the save: the next save must not overwrite it.
    fx.type_text("Y")?;
    std::thread::sleep(Duration::from_millis(5));
    fs::write(&path, "agent\n")?;
    fx.action(Box::new(CloseSideTab(0)))?;
    fx.wait("discard confirmation", confirming)?;
    fx.capture_evidence("CGX_FILE_EDITOR_SHOTS", "3-discard")?;
    ensure!(
        fx.read(|r, cx| r.toasts(cx).iter().any(|t| t == "Could not save file")),
        "no save failure toast"
    );
    ensure!(
        fs::read_to_string(&path)? == "agent\n",
        "a stale save overwrote the file"
    );
    ensure!(
        fx.read(|r, _| matches!(r.side_tabs(), [SidePanel::File(_)])),
        "the tab closed with unsaved changes"
    );
    // `Continue viewing` keeps the edit; `Discard changes` closes the tab.
    fx.keys("escape")?;
    ensure!(!confirming(&mut fx) && unsaved(&fx) && fx.read(|r, _| !r.side_tabs().is_empty()));
    fx.action(Box::new(CloseSideTab(0)))?;
    fx.wait("discard confirmation again", confirming)?;
    fx.keys("tab enter")?;
    ensure!(
        fx.read(|r, _| r.side_tabs().is_empty()),
        "discard kept the tab"
    );
    ensure!(fs::read_to_string(&path)? == "agent\n");
    Ok(())
}
