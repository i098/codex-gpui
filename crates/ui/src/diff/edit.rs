//! The Files tab as a text editor (`textFileEditor`): a double-clicked line opens the editor
//! there, edits save to disk 3 s after they start or on `⌘S`, and closing a tab whose changes
//! could not be saved asks `Discard file changes?`.

use super::*;
use crate::composer::Composer;
use crate::confirmation::{Confirmation, Outcome};
use gpui::{Focusable as _, Task};
use std::time::Duration;

/// The original's `X = 10485760`: a larger text file opens read-only.
pub(super) const LARGE_FILE_BYTES: usize = 10 * 1024 * 1024;
/// The original's autosave delay (`gn = 3e3`).
const AUTOSAVE: Duration = Duration::from_secs(3);

pub(super) struct FileEdit {
    pub(super) editor: Entity<Composer>,
    /// The text of the save in flight.
    saving: Option<String>,
    /// A tab close waits for this save; a failed one asks to discard.
    close_after_save: bool,
    _autosave: Option<Task<()>>,
}

/// A result of the host's `write-file`: the file's new `mtimeMs`, or why nothing was written.
pub type SaveResult = Result<Option<u64>, String>;

impl FileView {
    /// Lets the tab edit and save its text; only owners that handle
    /// [`FileTreeRequest::Save`] turn it on.
    pub fn set_editable(&mut self, editable: bool) {
        self.editable = editable;
    }

    /// The `mtimeMs` the host read the text at; a save expects the file unchanged since.
    pub fn set_disk_mtime(&mut self, mtime: Option<u64>) {
        self.disk_mtime = mtime;
    }

    pub(super) fn large_file(&self) -> bool {
        matches!(&self.contents, Some(Ok(text)) if text.len() > LARGE_FILE_BYTES)
    }

    fn can_edit(&self) -> bool {
        self.editable
            && self.media.is_none()
            && !self.shows_markdown_preview()
            && matches!(self.contents, Some(Ok(_)))
            && !self.large_file()
    }

    pub fn has_unsaved_changes(&self, cx: &App) -> bool {
        match (&self.edit, &self.contents) {
            (Some(edit), Some(Ok(saved))) => edit.editor.read(cx).field_text() != saved,
            _ => false,
        }
    }

    /// `Discard file changes?` is open.
    pub fn confirming_discard(&self) -> bool {
        self.discard.is_some()
    }

    /// Opens the editor with the caret at the start of `line`.
    pub(super) fn edit_at(&mut self, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit.is_none() {
            if !self.can_edit() {
                return;
            }
            let Some(Ok(text)) = &self.contents else {
                return;
            };
            let editor = cx.new(|cx| {
                Composer::settings_field("", text, true, cx)
                    .monospace_field()
                    .with_text_metrics(px(CODE_SIZE), px(FILE_LINE_HEIGHT))
            });
            cx.observe(&editor, |view, _, cx| view.edited(cx)).detach();
            self.edit = Some(FileEdit {
                editor,
                saving: None,
                close_after_save: false,
                _autosave: None,
            });
            self.go_to_line = None;
            self.close_find(cx);
            self.pin_preview(cx);
        }
        let Some(edit) = &self.edit else { return };
        let editor = edit.editor.clone();
        editor.update(cx, |editor, cx| {
            let offset = line_offset(editor.field_text(), line);
            editor.set_cursor(offset, cx);
        });
        window.focus(&editor.focus_handle(cx), cx);
        cx.notify();
    }

    /// Starts the autosave once the text differs from disk. The editor also notifies for
    /// caret moves, so a pending timer keeps running instead of restarting.
    fn edited(&mut self, cx: &mut Context<Self>) {
        let dirty = self.has_unsaved_changes(cx);
        if let Some(edit) = &mut self.edit
            && dirty
            && edit._autosave.is_none()
        {
            edit._autosave = Some(cx.spawn(async move |view, cx| {
                cx.background_executor().timer(AUTOSAVE).await;
                view.update(cx, |view, cx| {
                    if let Some(edit) = &mut view.edit {
                        edit._autosave = None;
                    }
                    view.save(cx);
                })
                .ok();
            }));
        }
    }

    /// `⌘S` and the autosave: asks the owner to write the edited text.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.has_unsaved_changes(cx) {
            return;
        }
        let Some(edit) = &mut self.edit else { return };
        if edit.saving.is_some() {
            return;
        }
        let contents = edit.editor.read(cx).field_text().to_owned();
        edit.saving = Some(contents.clone());
        edit._autosave = None;
        cx.emit(FileTreeRequest::Save {
            contents,
            mtime: self.disk_mtime,
        });
        cx.notify();
    }

    /// The owner's answer to [`FileTreeRequest::Save`].
    pub fn saved(&mut self, result: SaveResult, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.edit else { return };
        let Some(contents) = edit.saving.take() else {
            return;
        };
        let close = edit.close_after_save;
        match result {
            Ok(mtime) => {
                self.disk_mtime = mtime;
                self.set_saved_contents(contents);
                if self.has_unsaved_changes(cx) {
                    self.edited(cx);
                    if close {
                        self.save(cx);
                    }
                } else if close {
                    cx.emit(FileTreeRequest::Close);
                }
            }
            Err(_) if close => self.confirm_discard(window, cx),
            Err(_) => {}
        }
        cx.notify();
    }

    /// The saved text is the base the editor compares with; the read view shows it.
    fn set_saved_contents(&mut self, contents: String) {
        self.syntax = syntax::full_file(&self.path, &contents);
        self.line_count = controls::lines(&contents).count() as u32;
        self.contents = Some(Ok(contents));
    }

    /// A tab close: `true` when nothing is unsaved. Otherwise the tab saves first and closes
    /// itself ([`FileTreeRequest::Close`]) once clean, or asks to discard when that failed.
    pub fn request_close(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.has_unsaved_changes(cx) {
            return true;
        }
        if let Some(edit) = &mut self.edit {
            edit.close_after_save = true;
        }
        self.save(cx);
        false
    }

    fn confirm_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Discard file changes?",
                "Your unsaved changes to this file will be lost",
                "Discard changes",
                window,
                cx,
            )
            .cancel_label("Continue viewing")
            .destructive()
        });
        cx.subscribe(&dialog, |view, _, outcome, cx| {
            view.discard = None;
            if *outcome == Outcome::Confirm {
                view.edit = None;
                cx.emit(FileTreeRequest::Close);
            } else if let Some(edit) = &mut view.edit {
                edit.close_after_save = false;
            }
            cx.notify();
        })
        .detach();
        self.discard = Some(dialog);
    }

    /// `Escape` in a clean editor returns to the highlighted lines.
    pub(super) fn edit_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(edit) = &self.edit else {
            return false;
        };
        let keystroke = &event.keystroke;
        if keystroke.modifiers.secondary() && keystroke.key == "s" {
            self.save(cx);
            return true;
        }
        if keystroke.key == "escape" && edit.saving.is_none() && !self.has_unsaved_changes(cx) {
            self.edit = None;
            cx.notify();
            return true;
        }
        false
    }

    /// `textFileEditor.largeFileReadOnly`: `px-4 pt-3` around a `rounded-lg` bordered note.
    pub(super) fn large_file_banner(&self, t: &Theme) -> Div {
        div().flex_none().px(px(16.)).pt(px(12.)).child(
            div()
                .flex()
                .items_center()
                .rounded(px(8.))
                .border_1()
                .border_color(t.border)
                .px(px(12.))
                .py(px(8.))
                .child(text(
                    "Large file opened read-only",
                    13.,
                    20.,
                    430.,
                    t.text_secondary,
                )),
        )
    }
}

/// Byte offset where 1-based `line` starts; past the end is the text's end.
fn line_offset(text: &str, line: u32) -> usize {
    text.split_inclusive('\n')
        .take(line.saturating_sub(1) as usize)
        .map(str::len)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_offset_starts_at_the_line_and_clamps_past_the_end() {
        let text = "a\nbé\r\nc";
        assert_eq!(line_offset(text, 1), 0);
        assert_eq!(line_offset(text, 0), 0);
        assert_eq!(&text[line_offset(text, 2)..], "bé\r\nc");
        assert_eq!(&text[line_offset(text, 3)..], "c");
        assert_eq!(line_offset(text, 9), text.len());
    }
}
