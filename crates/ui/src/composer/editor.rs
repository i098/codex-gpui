use super::*;
use crate::screens::thread::view::menu::{ContextMenu, Item, selection_url};
use gpui::TransformationMatrix;
use std::time::Instant;

type EditState = (String, Range<usize>, Vec<mention::Mention>);

#[derive(Default)]
struct TransparentCaret(bool);

impl gpui::Global for TransparentCaret {}

/// ProseMirror's history: edits closer than `newGroupDelay` (500 ms) that touch the last
/// edit's end form one undo step; undo and redo close the group.
#[derive(Default)]
pub(super) struct History {
    undo: Vec<EditState>,
    redo: Vec<EditState>,
    /// When the last edit happened and where it ended.
    pub(super) last: Option<(Instant, usize)>,
}

const NEW_GROUP_DELAY: Duration = Duration::from_millis(500);

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn normalized_range(units: impl Iterator<Item = u16>, range: Range<usize>) -> Range<usize> {
    let mut start = range.start;
    let mut end = range.end;
    let mut previous = 0;
    for (index, unit) in units.take(range.end).enumerate() {
        if previous == u16::from(b'\r') && unit == u16::from(b'\n') {
            if index < range.start {
                start -= 1;
            }
            end -= 1;
        }
        previous = unit;
    }
    start..end
}

impl Composer {
    pub(super) fn normalize_document(&mut self) {
        for (content, selection, _) in self.history.undo.iter_mut().chain(&mut self.history.redo) {
            if content.contains('\r') {
                *selection = normalized_range(content.bytes().map(u16::from), selection.clone());
                *content = normalize_newlines(content).into_owned();
            }
        }
        if !self.content.contains('\r') {
            return;
        }
        self.selected_range = normalized_range(
            self.content.bytes().map(u16::from),
            self.selected_range.clone(),
        );
        self.marked_range = self
            .marked_range
            .take()
            .map(|range| normalized_range(self.content.bytes().map(u16::from), range));
        if let Some((_, offset)) = &mut self.history.last {
            *offset = normalized_range(self.content.bytes().map(u16::from), *offset..*offset).start;
        }
        self.content = normalize_newlines(&self.content).into_owned();
    }

    /// Makes the captured caret transparent without changing focus, selection, or input.
    pub fn set_caret_transparent(cx: &mut App, transparent: bool) {
        cx.default_global::<TransparentCaret>().0 = transparent;
    }

    pub fn caret_transparent(cx: &App) -> bool {
        cx.try_global::<TransparentCaret>()
            .is_some_and(|state| state.0)
    }

    pub(super) fn cursor(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub(super) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.refresh_slash(cx);
        cx.notify();
    }

    pub(super) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    // ponytail: char boundaries, not grapheme clusters; add unicode-segmentation if
    // combining sequences need one-step deletion. A chip counts as one character.
    pub(super) fn previous_boundary(&self, offset: usize) -> usize {
        let at = self.content[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
        self.snap(at, Some(false))
    }

    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        let at = self.content[offset..]
            .chars()
            .next()
            .map_or(offset, |c| offset + c.len_utf8());
        self.snap(at, Some(true))
    }

    /// Start of the word before `offset` (skipping spaces first), as `moveWordLeft` does.
    // ponytail: words are alphanumeric runs; ICU word breaks (CJK, punctuation) if asked for.
    fn word_start(&self, offset: usize) -> usize {
        let before = self.content[..offset].trim_end_matches(|c| !is_word(c));
        self.snap(before.trim_end_matches(is_word).len(), Some(false))
    }

    /// End of the word after `offset` (skipping spaces first), as `moveWordRight` does.
    fn word_end(&self, offset: usize) -> usize {
        let rest = self.content[offset..].trim_start_matches(|c| !is_word(c));
        self.snap(
            self.content.len() - rest.trim_start_matches(is_word).len(),
            Some(true),
        )
    }

    /// Records the state before an edit of `range`, unless it joins the last edit's group.
    pub(super) fn remember(&mut self, range: &Range<usize>) {
        let joins = self.history.last.is_some_and(|(at, end)| {
            at.elapsed() < NEW_GROUP_DELAY && range.start <= end && end <= range.end
        });
        if !joins {
            self.history.undo.push((
                self.content.clone(),
                self.selected_range.clone(),
                self.mentions.clone(),
            ));
            self.history.redo.clear();
        }
    }

    fn restore(&mut self, (content, selection, mentions): EditState, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
        self.history.last = None;
        self.content = content;
        self.mentions = mentions;
        self.selected_range = selection;
        self.selection_reversed = false;
        self.marked_range = None;
        self.refresh_slash(cx);
        cx.notify();
    }

    pub(super) fn undo(&mut self, _: &crate::Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.history.undo.pop() {
            self.history.redo.push((
                self.content.clone(),
                self.selected_range.clone(),
                self.mentions.clone(),
            ));
            self.restore(state, cx);
        }
    }

    pub(super) fn redo(&mut self, _: &crate::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.history.redo.pop() {
            self.history.undo.push((
                self.content.clone(),
                self.selected_range.clone(),
                self.mentions.clone(),
            ));
            self.restore(state, cx);
        }
    }

    pub(super) fn insert(&mut self, s: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, s, window, cx);
    }

    pub(super) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", window, cx);
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    pub(super) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.word_start(self.cursor());
        self.move_to(to, cx);
    }

    pub(super) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.word_end(self.cursor());
        self.move_to(to, cx);
    }

    pub(super) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.word_start(self.cursor()), cx);
    }

    pub(super) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.word_end(self.cursor()), cx);
    }

    pub(super) fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.word_start(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    pub(super) fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.word_end(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    /// Command+Backspace deletes to the start of the line.
    // ponytail: the logical line (after the last `\n`); Chromium deletes to the wrapped
    // line's start.
    pub(super) fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            let to = self.content[..self.cursor()]
                .rfind('\n')
                .map_or(0, |i| i + 1);
            self.select_to(to, cx);
        }
        self.insert("", window, cx);
    }

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.previous_boundary(self.cursor())
        } else {
            self.selected_range.start
        };
        self.move_to(to, cx);
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.next_boundary(self.cursor())
        } else {
            self.selected_range.end
        };
        self.move_to(to, cx);
    }

    pub(super) fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_number {
            self.step_request_number(1., window, cx);
            return;
        }
        if self.move_slash(false, cx) {
            return;
        }
        let to = self.vertical(-1.).unwrap_or(0);
        self.move_to(to, cx);
    }

    pub(super) fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_number {
            self.step_request_number(-1., window, cx);
            return;
        }
        if self.move_slash(true, cx) {
            return;
        }
        let to = self.vertical(1.).unwrap_or(self.content.len());
        self.move_to(to, cx);
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(-1.).unwrap_or(0), cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(1.).unwrap_or(self.content.len()), cx);
    }

    pub(super) fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.content[..self.cursor()]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        self.move_to(to, cx);
    }

    pub(super) fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.cursor();
        let to = self.content[at..]
            .find('\n')
            .map_or(self.content.len(), |i| at + i);
        self.move_to(to, cx);
    }

    pub(super) fn select_all(
        &mut self,
        _: &crate::SelectAll,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    pub(super) fn copy(&mut self, _: &crate::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.secret && !self.selected_range.is_empty() {
            let s = self.content[self.selected_range.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(s));
        }
    }

    pub(super) fn cut(&mut self, _: &crate::Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&crate::Copy, window, cx);
        self.insert("", window, cx);
    }

    pub(super) fn speak_selection(
        &mut self,
        _: &crate::StartSpeaking,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = if self.selected_range.is_empty() {
            self.content.as_str()
        } else {
            &self.content[self.selected_range.clone()]
        };
        if let Err(error) = crate::native_text::speak(text, cx) {
            eprintln!("codex-gpui: cannot speak selection: {error:#}");
        }
    }

    /// Command+V, or the menu's Paste: images and long text (5,000 UTF-16 units or more)
    /// become attachments, except in the message editor, which takes clipboard text only.
    pub(super) fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let attach = self.on_cancel_edit.is_none() && self.plain_field.is_none();
        if attach
            && item
                .entries()
                .iter()
                .any(|e| matches!(e, gpui::ClipboardEntry::Image(_)))
        {
            for entry in item.into_entries() {
                if let gpui::ClipboardEntry::Image(image) = entry {
                    self.save_paste(attach::Content::Image(std::sync::Arc::new(image)), cx);
                }
            }
        } else if let Some(text) = item.text() {
            if attach && text.encode_utf16().take(5000).count() == 5000 {
                self.save_paste(attach::Content::Text(text), cx);
            } else {
                self.insert(&text, window, cx);
            }
        }
    }

    pub(super) fn paste_and_match_style(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text, window, cx);
        }
    }

    /// The open right-click menu's items as `(id, label, enabled)`; empty when none is open.
    pub fn context_menu_items(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.context_menu.iter().flat_map(ContextMenu::rows)
    }

    /// Where the open right-click menu's top-left corner is anchored (the pointer).
    pub fn context_menu_at(&self) -> Option<Point<Pixels>> {
        self.context_menu.as_ref().map(|menu| menu.position)
    }

    /// Electron's `context-menu` for an editable field (main's `installNativeContextMenu`):
    /// `Look Up “…”` and `Search with Google` with a selection, then Cut ⌘X, Copy ⌘C (both
    /// need a selection) and Paste ⌘V. A right-click outside the selection moves the caret
    /// there first, as Chromium does.
    fn on_right_click(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.secret {
            cx.stop_propagation();
            return;
        }
        window.focus(&self.focus_handle, cx);
        self.menu = None;
        self.context_spelling = None;
        self.context_image = None;
        let at = self.index_for_point(e.position);
        if !self.selected_range.contains(&at) {
            self.move_to(at, cx);
        }
        let spelling = crate::native_text::misspelling(&self.content, self.offset_to_utf16(at))
            .map(|spelling| {
                let range = self.range_from_utf16(&spelling.range_utf16);
                (spelling, range)
            });
        if let Some((_, range)) = &spelling
            && self.selected_range.is_empty()
        {
            self.selected_range = range.clone();
            self.selection_reversed = false;
        }
        let selection = self.content[self.selected_range.clone()].to_string();
        let has = !selection.is_empty();
        let mut items = Vec::new();
        if let Some((spelling, range)) = spelling.filter(|(_, range)| *range == self.selected_range)
        {
            self.context_spelling = Some((range.clone(), self.content[range].to_string()));
            if spelling.guesses.is_empty() {
                items.push(Item::new(
                    "dictionarySuggestions",
                    "No Guesses Found",
                    false,
                ));
            } else {
                for guess in spelling.guesses {
                    let label: SharedString = guess.into();
                    items.push(Item {
                        payload: Some(label.clone()),
                        ..Item::new("dictionarySuggestions", label, true)
                    });
                }
            }
            items.push(Item::separator("spelling-separator"));
            items.push(Item::new("learnSpelling", "Learn Spelling", true));
            items.push(Item::separator("learn-spelling-separator"));
        }
        if has {
            let trimmed = selection.trim();
            // `p$e(t, 25)`: the label shows the first 24 characters and an ellipsis.
            let shown = match trimmed.char_indices().nth(24) {
                Some((i, _)) if trimmed.chars().count() > 25 => format!("{}…", &trimmed[..i]),
                _ => trimmed.to_string(),
            };
            items.push(Item::new(
                "lookUpSelection",
                format!("Look Up “{shown}”"),
                true,
            ));
            items.push(Item::new("searchWithGoogle", "Search with Google", true));
            items.push(Item::separator("separator"));
        }
        for (id, label, keys, enabled) in [
            ("cut", "Cut", "⌘X", has),
            ("copy", "Copy", "⌘C", has),
            ("paste", "Paste", "⌘V", true),
        ] {
            items.push(Item {
                accelerator: Some(keys),
                ..Item::new(id, label, enabled)
            });
        }
        self.context_menu = Some(ContextMenu::open(items, selection, e.position, window, cx));
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn activate_context_item(
        &mut self,
        id: &str,
        selection: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            crate::image_context::COPY
            | crate::image_context::SAVE
            | crate::image_context::REVEAL => {
                if let Some(image) = self.context_image.take() {
                    image.activate(id, window, cx);
                }
            }
            "dictionarySuggestions" => {
                if let Some((range, _)) = self.take_spelling() {
                    self.selected_range = range;
                    self.selection_reversed = false;
                    self.insert(&selection, window, cx);
                }
            }
            "learnSpelling" => {
                if let Some((_, word)) = self.take_spelling() {
                    crate::native_text::learn(&word);
                }
            }
            // ponytail: `showDefinitionForSelection` has no GPUI call; Dictionary.app opens
            // the same entry from its URL scheme.
            "lookUpSelection" => cx.open_url(&selection_url("dict://", &selection)),
            "searchWithGoogle" => cx.open_url(&selection_url(
                "https://www.google.com/search?q=",
                &selection,
            )),
            "cut" => self.cut(&crate::Cut, window, cx),
            "copy" => self.copy(&crate::Copy, window, cx),
            "paste" => self.paste(window, cx),
            _ => {}
        }
    }

    fn take_spelling(&mut self) -> Option<(Range<usize>, String)> {
        self.context_spelling
            .take()
            .filter(|(range, word)| self.content.get(range.clone()) == Some(word.as_str()))
    }

    pub(super) fn on_mouse_down(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;
        self.menu = None;
        let at = self.index_for_point(e.position);
        if e.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    pub(super) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub(super) fn on_mouse_move(
        &mut self,
        e: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_selecting {
            self.select_to(self.index_for_point(e.position), cx);
        }
    }

    fn scroll_field_to_caret(&mut self, scale: f32) -> bool {
        if !self.field_autoscroll {
            return false;
        }
        let (Some(scroll), Some(bounds)) = (&self.field_scroll, self.bounds) else {
            return false;
        };
        let cursor = self.cursor();
        let Some((start, line_y, line)) = self
            .line_starts()
            .find(|(start, _, line)| cursor <= start + line.len())
        else {
            return false;
        };
        let Some(caret) = line.position_for_index(cursor - start, self.editor_line_height()) else {
            return false;
        };
        let font_height = f32::from(line.ascent() + line.descent());
        let leading = (f32::from(self.editor_line_height()) - font_height) / 2.;
        let origin = bounds.origin.y + line_y + caret.y;
        let top = origin + px((leading * scale).ceil() / scale);
        let bottom = origin + px(((leading + font_height) * scale).floor() / scale);
        let viewport = scroll.bounds();
        if viewport.size.height <= px(2.) {
            return false;
        }
        let start = viewport.origin.y;
        let end = viewport.origin.y + viewport.size.height;
        let adjustment = if top < start {
            start - top
        } else if bottom > end {
            end - bottom
        } else {
            px(0.)
        };
        self.field_autoscroll = false;
        let mut offset = scroll.offset();
        let next = px((f32::from(offset.y + adjustment) * scale).round() / scale);
        if offset.y == next {
            return false;
        }
        offset.y = next;
        scroll.set_offset(offset);
        true
    }

    /// Byte offset where each shaped line starts (lines are split at `\n`).
    pub(super) fn line_starts(&self) -> impl Iterator<Item = (usize, Pixels, &WrappedLine)> {
        let mut start = 0;
        let mut y = px(0.);
        let line_height = self.editor_line_height();
        self.lines.iter().map(move |line| {
            let item = (start, y, line);
            start += line.len() + 1;
            y += line.size(line_height).height;
            item
        })
    }

    /// Editor-local position of the caret before byte `offset`.
    pub(super) fn point_for_index(&self, offset: usize) -> Option<Point<Pixels>> {
        self.line_starts()
            .find(|(start, _, line)| offset <= start + line.len())
            .and_then(|(start, y, line)| {
                let p = line.position_for_index(offset - start, self.editor_line_height())?;
                Some(point(p.x, p.y + y))
            })
    }

    pub(super) fn index_for_local(&self, local: Point<Pixels>) -> usize {
        let mut last = 0;
        let line_height = self.editor_line_height();
        for (start, y, line) in self.line_starts() {
            let h = line.size(line_height).height;
            if local.y < y + h {
                let p = point(local.x, (local.y - y).max(px(0.)));
                let i = line
                    .closest_index_for_position(p, line_height)
                    .unwrap_or_else(|i| i);
                return self.snap(start + i, None);
            }
            last = start + line.len();
        }
        last
    }

    pub(super) fn index_for_point(&self, position: Point<Pixels>) -> usize {
        match (self.bounds, self.content.is_empty()) {
            (Some(b), false) => self.index_for_local(position - b.origin),
            _ => 0,
        }
    }

    /// Offset one row above (`-1`) or below (`1`) the caret; `None` past the first/last row.
    pub(super) fn vertical(&self, rows: f32) -> Option<usize> {
        let p = self.point_for_index(self.cursor())?;
        let line_height = self.editor_line_height();
        let y = p.y + line_height * rows + line_height / 2.;
        let total: Pixels = self.lines.iter().map(|l| l.size(line_height).height).sum();
        (y >= px(0.) && y < total).then(|| self.index_for_local(point(p.x, y)))
    }

    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

    pub(super) fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    pub(super) fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end)
    }

    pub(super) fn editor(&self, cx: &mut Context<Self>) -> Div {
        let compact_settings = self.request_parent.is_none() && self.request_compact;
        let comment = self.plain_field.is_some() && self.on_cancel_edit.is_some();
        div()
            .px(px(if self.search_field || comment {
                0.
            } else if self.compact_text() {
                10.
            } else {
                12.
            }))
            .pb(px(
                if self.search_field
                    || self.plain_field == Some(true)
                    || self.request_parent.is_some()
                    || self.request_compact
                {
                    0.
                } else {
                    4.
                },
            ))
            .min_h(if comment {
                self.editor_line_height()
            } else if self.search_field || self.request_parent.is_some() || self.request_compact {
                px(20.)
            } else {
                self.field_min_height
            })
            .cursor(CursorStyle::IBeam)
            .when(self.embedded_field.is_some(), |d| {
                d.px_0().pb_0().min_h(self.editor_line_height())
            })
            .text_size(if self.search_field {
                px(13.)
            } else {
                self.editor_font_size()
            })
            .line_height(self.editor_line_height())
            .font_weight(gpui::FontWeight(if self.field_text.is_some() {
                500.
            } else {
                430.
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_click))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            // Rich text uses Chromium's transparent scroll layer; its placeholder has opacity .5.
            // Settings textareas and single-line form inputs paint directly.
            .child(
                div()
                    .when(!self.search_field && self.field_text.is_none(), |d| {
                        d.when(
                            self.content.is_empty()
                                && self.embedded_field.is_none()
                                && !self.search_field
                                && !compact_settings,
                            |d| d.opacity(0.5),
                        )
                        .when(
                            (self.embedded_field.is_none()
                                && matches!(
                                    (self.plain_field, self.field_style),
                                    (None, _)
                                        | (Some(false), FieldStyle::Settings)
                                        | (Some(true), FieldStyle::Form | FieldStyle::Inline)
                                )
                                || self.search_field)
                                && !compact_settings,
                            |d| d.layer(),
                        )
                    })
                    .child(Editor(cx.entity())),
            )
    }
}

impl Focusable for Composer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Composer {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(if self.secret {
            "*".repeat(self.content[range].encode_utf16().count())
        } else {
            self.content[range].to_owned()
        })
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.range_from_utf16(&range_utf16);
        self.selected_range = if range.is_empty() {
            let at = self.snap(range.start, None);
            at..at
        } else {
            self.snap(range.start, Some(false))..self.snap(range.end, Some(true))
        };
        self.selection_reversed = false;
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_and_mark_text_in_range(range_utf16, new_text, None, window, cx);
        self.marked_range = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_dictation(cx);
        let new_selected_range_utf16 = if self.document_field && new_text.contains('\r') {
            new_selected_range_utf16.map(|range| normalized_range(new_text.encode_utf16(), range))
        } else {
            new_selected_range_utf16
        };
        let normalized;
        let single_line;
        let new_text = if self.plain_field == Some(false) && new_text.contains(['\n', '\r']) {
            single_line = new_text.replace(['\n', '\r'], "");
            single_line.as_str()
        } else if self.document_field {
            normalized = normalize_newlines(new_text);
            normalized.as_ref()
        } else {
            new_text
        };
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.remember(&range);
        let range = self.splice(range, new_text);
        let end = range.start + new_text.len();
        self.history.last = Some((Instant::now(), end));
        self.marked_range = (!new_text.is_empty()).then_some(range.start..end);
        self.selected_range = match new_selected_range_utf16 {
            Some(sel) => {
                let base = self.offset_to_utf16(range.start);
                self.range_from_utf16(&(base + sel.start..base + sel.end))
            }
            None => end..end,
        };
        self.selection_reversed = false;
        self.refresh_slash(cx);
        cx.notify();
    }

    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> gpui::TextInputConfiguration {
        gpui::TextInputConfiguration {
            text_substitutions: !self.secret,
            ..Default::default()
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.point_for_index(range.start)?;
        let end = self.point_for_index(range.end)?;
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin + point(end.x, end.y + self.editor_line_height()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = p - self.bounds?.origin;
        Some(self.offset_to_utf16(self.index_for_local(local)))
    }
}

/// The text area: wraps to its width, grows one row per line.
pub(super) struct Editor(pub(super) Entity<Composer>);

impl IntoElement for Editor {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// The chip label color: `color-mix(text-info 80%, text 20%)` over `_Mention`.
pub(super) fn chip_color(t: &Theme) -> Hsla {
    crate::over(t.text_accent.opacity(0.8), t.text)
}

/// `text` cut to `max` with a trailing `…` (the longest prefix, without its trailing
/// whitespace, that fits), or `text` itself when it fits.
fn ellipsize(window: &Window, text: &str, size: Pixels, font: gpui::Font, max: Pixels) -> String {
    let width = |s: &str| {
        window
            .text_system()
            .layout_line(
                s,
                size,
                &[TextRun {
                    len: s.len(),
                    font: font.clone(),
                    color: gpui::black(),
                    ..Default::default()
                }],
                None,
            )
            .width
    };
    if width(text) <= max {
        return text.to_owned();
    }
    let cuts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let fits = |end: usize| width(&format!("{}…", text[..cuts[end]].trim_end())) <= max;
    let (mut low, mut high) = (0, cuts.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(mid.min(cuts.len() - 1)) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    format!("{}…", text[..cuts[low.min(cuts.len() - 1)]].trim_end())
}

/// Shapes `content` (or the placeholder) for one paint: one run per stretch of plain, marked
/// (underlined) or chip (`chip` color, weight 500) text.
pub(super) fn shape(
    composer: &Composer,
    color: Hsla,
    placeholder: Hsla,
    chip: Hsla,
    width: Option<Pixels>,
    window: &Window,
) -> Vec<WrappedLine> {
    let style = window.text_style();
    let size = composer.editor_font_size();
    let (s, color) = if composer.content.is_empty() {
        // The placeholder is one line that `text-overflow: ellipsis` cuts.
        let fitted = match width {
            Some(width) => ellipsize(window, &composer.placeholder, size, style.font(), width),
            None => composer.placeholder.to_string(),
        };
        (fitted, placeholder)
    } else {
        (
            if composer.secret {
                "*".repeat(composer.content.len())
            } else {
                composer.content.clone()
            },
            color,
        )
    };
    let marked = composer
        .marked_range
        .as_ref()
        .filter(|_| !composer.content.is_empty());
    let mut cuts = vec![0, s.len()];
    cuts.extend(marked.iter().flat_map(|m| [m.start, m.end]));
    cuts.extend(
        composer
            .mentions
            .iter()
            .flat_map(|m| [m.range.start, m.range.end]),
    );
    cuts.sort_unstable();
    cuts.dedup();
    let runs = cuts
        .windows(2)
        .map(|w| {
            let within = |r: &Range<usize>| r.start <= w[0] && w[1] <= r.end;
            let in_chip = composer.mentions.iter().any(|m| within(&m.range));
            let mut font = style.font();
            if in_chip {
                font.weight = gpui::FontWeight(500.);
            }
            TextRun {
                len: w[1] - w[0],
                font,
                color: if in_chip { chip } else { color },
                background_color: None,
                underline: marked.filter(|m| within(m)).map(|_| UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.),
                    wavy: false,
                }),
                strikethrough: None,
            }
        })
        .collect::<Vec<_>>();
    window
        .text_system()
        .shape_text(
            s.into(),
            size,
            &runs,
            if composer.embedded_field.is_some() {
                None
            } else {
                width.filter(|_| !composer.content.is_empty())
            },
            None,
        )
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default()
}

impl gpui::Element for Editor {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, LayoutId) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.0.clone();
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let black = gpui::black();
            let composer = input.read(cx);
            let lines = shape(composer, black, black, black, width, window);
            let height = lines
                .iter()
                .map(|l| l.size(composer.editor_line_height()).height)
                .sum();
            Size {
                width: width.unwrap_or_default(),
                height,
            }
        });
        (id, id)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let t = cx.theme().clone();
        let composer = self.0.read(cx);
        let lines = shape(
            composer,
            if composer.field_style == FieldStyle::Inline {
                t.text_secondary
            } else {
                composer.field_text.unwrap_or(t.text)
            },
            t.text_tertiary,
            chip_color(&t),
            Some(bounds.size.width),
            window,
        );
        let mut text_bounds = bounds;
        if self.0.read(cx).field_style == FieldStyle::Inline
            && let Some(line) = lines.first()
        {
            let offset = (bounds.size.width - line.unwrapped_layout.width).max(px(0.));
            text_bounds.origin.x += offset;
            text_bounds.size.width -= offset;
        }
        // The editor spans the card minus its 12 px side padding.
        let width = f32::from(bounds.size.width)
            + if self.0.read(cx).hotkey_controls.is_some() {
                0.
            } else {
                24.
            };
        let collapse = |w: f32| (w <= SM_WIDTH, w <= XS_WIDTH);
        let changed = self.0.update(cx, |c, _| {
            c.lines = lines;
            let mut bounds = text_bounds;
            if c.embedded_field.is_some() {
                let cursor = c
                    .point_for_index(c.cursor())
                    .map_or(px(0.), |point| point.x);
                bounds.origin.x -= (cursor - bounds.size.width + px(2.)).max(px(0.));
            }
            c.bounds = Some(bounds);
            let width_changed =
                collapse(std::mem::replace(&mut c.footer_width, width)) != collapse(width);
            c.scroll_field_to_caret(window.scale_factor()) || width_changed
        });
        if changed {
            let entity = self.0.clone();
            window.defer(cx, move |_, cx| entity.update(cx, |_, cx| cx.notify()));
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        id: &mut LayoutId,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let bounds = self.0.read(cx).bounds.unwrap_or(bounds);
        let focus = self.0.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        let selection_color = cx.theme().text_selection;
        let c = self.0.read(cx);
        let caret_color = c.field_text.unwrap_or(cx.theme().text);
        let bounds = if c.embedded_field.is_some() {
            c.bounds.unwrap_or(bounds)
        } else {
            bounds
        };
        let line_height = c.editor_line_height();
        if !c.selected_range.is_empty()
            && let (Some(a), Some(b)) = (
                c.point_for_index(c.selected_range.start),
                c.point_for_index(c.selected_range.end),
            )
        {
            let w = bounds.size.width;
            let mut row = a.y;
            while row <= b.y {
                let x0 = if row == a.y { a.x } else { px(0.) };
                let x1 = if row == b.y { b.x } else { w };
                let r = Bounds::from_corners(
                    bounds.origin + point(x0, row),
                    bounds.origin + point(x1, row + line_height),
                );
                window.paint_quad(fill(r, selection_color));
                row += line_height;
            }
        }
        let caret = (c.selected_range.is_empty()
            && focus.is_focused(window)
            && !cx.try_global::<TransparentCaret>().is_some_and(|s| s.0))
        .then(|| c.point_for_index(c.cursor()))
        .flatten();
        // Each chip's 16 px icon sits in its slot, 2 px under the line top (`_IconContainer`).
        let icons: Vec<_> = c
            .mentions
            .iter()
            .filter_map(|m| Some((c.point_for_index(m.range.start)?, m.icon)))
            .collect();
        let mut y = if c.plain_field == Some(true) {
            window.layout_origin_residual_y(*id)
        } else {
            px(0.)
        };
        let lines = self.0.update(cx, |c, _| std::mem::take(&mut c.lines));
        // Glyphs land on the unsnapped LayoutUnit x, as Blink paints text.
        let x = window.layout_origin_residual_x(*id);
        for line in &lines {
            let origin = bounds.origin + point(x, y);
            line.paint(origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
            y += line.size(line_height).height;
        }
        self.0.update(cx, |c, _| c.lines = lines);
        let chip = chip_color(cx.theme());
        for (p, icon) in icons {
            let r = Bounds::new(
                bounds.origin + point(p.x, p.y + px(2.)),
                size(px(16.), px(16.)),
            );
            window
                .paint_svg(
                    r,
                    format!("icons/{icon}.svg").into(),
                    None,
                    TransformationMatrix::unit(),
                    chip,
                    cx,
                )
                .ok();
        }
        if let Some(p) = caret {
            let r = Bounds::new(
                bounds.origin + point(p.x, p.y + px(1.5)),
                size(px(1.), line_height - px(3.)),
            );
            window.paint_quad(fill(r, caret_color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;
    use std::sync::Arc;

    fn with_editor(
        test: impl FnOnce(&mut Composer, &mut Window, &mut Context<Composer>),
    ) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut app = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
        });
        let window = app.open_window(size(px(800.), px(600.)), |_, cx| {
            cx.new(|cx| Composer::settings_field("", "", true, cx))
        })?;
        window.update(&mut app, test)?;
        Ok(())
    }

    #[test]
    fn document_ime_cursor_stays_before_following_text() -> anyhow::Result<()> {
        with_editor(|editor, window, cx| {
            editor.set_document_style(true, cx);
            for prefix in ["A", "😀"] {
                editor.set_text("XY", cx);
                let input = format!("{prefix}\r\nB");
                let cursor = input.encode_utf16().count();
                editor.replace_and_mark_text_in_range(
                    Some(0..0),
                    &input,
                    Some(cursor..cursor),
                    window,
                    cx,
                );
                editor.unmark_text(window, cx);
                editor.replace_text_in_range(None, "Z", window, cx);
                assert_eq!(editor.field_text(), format!("{prefix}\nBZXY"));
            }
        })
    }

    #[test]
    fn enabling_document_mode_preserves_undo_redo_and_cursor() -> anyhow::Result<()> {
        with_editor(|editor, window, cx| {
            for undo_before_enable in [false, true] {
                editor.set_document_style(false, cx);
                editor.set_text("First\r\nSecond\r\n", cx);
                editor.selected_range = 7..7;
                editor.replace_text_in_range(Some(7..13), "Edited", window, cx);
                if undo_before_enable {
                    editor.undo(&crate::Undo, window, cx);
                }
                editor.set_document_style(true, cx);
                if undo_before_enable {
                    editor.redo(&crate::Redo, window, cx);
                }
                assert_eq!(editor.field_text(), "First\nEdited\n");
                assert_eq!(editor.cursor(), 12);
                editor.undo(&crate::Undo, window, cx);
                assert_eq!(editor.field_text(), "First\nSecond\n");
                assert_eq!(editor.cursor(), 6);
                editor.redo(&crate::Redo, window, cx);
                assert_eq!(editor.field_text(), "First\nEdited\n");
                assert_eq!(editor.cursor(), 12);
            }
        })
    }

    #[test]
    fn spelling_actions_replace_learn_and_ignore_stale_words() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = gpui::TestAppContext::single();
        cx.update(|cx| cx.set_global(Theme::dark()));
        let window = cx.add_window(|_, cx| Composer::new("Do anything", false, cx));
        window
            .update(&mut cx, |composer, window, cx| {
                composer.set_text("teh", cx);
                composer.context_spelling = Some((0..3, "teh".into()));
                composer.activate_context_item("dictionarySuggestions", "the".into(), window, cx);
                assert_eq!(composer.content, "the");
                composer.undo(&crate::Undo, window, cx);
                assert_eq!(composer.content, "teh");

                composer.context_spelling = Some((0..3, "teh".into()));
                composer.activate_context_item("learnSpelling", String::new(), window, cx);
                assert_eq!(
                    crate::native_text::take_learned_word().as_deref(),
                    Some("teh")
                );

                composer.context_spelling = Some((0..3, "teh".into()));
                composer.set_text("safe", cx);
                composer.activate_context_item("dictionarySuggestions", "the".into(), window, cx);
                assert_eq!(composer.content, "safe");
            })
            .expect("dispatch spelling menu actions");
    }
}
