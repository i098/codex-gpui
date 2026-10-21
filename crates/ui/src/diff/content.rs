use super::*;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobRequest {
    pub path: String,
    pub oid: Option<String>,
    pub fallback_to_disk: bool,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BlobReply {
    Success { lines: Vec<String> },
    Error { error: BlobError },
}

#[derive(serde::Deserialize)]
pub struct BlobError {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone)]
pub struct FullFileRequest {
    pub index: usize,
    pub generation: u64,
    pub cwd: String,
    pub requests: Vec<BlobRequest>,
    pub before: Option<usize>,
    pub after: Option<usize>,
}

#[derive(Clone)]
pub struct FullText {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FullError {
    Unavailable,
    Failed,
}

impl FullFileRequest {
    pub fn decode(&self, replies: Vec<BlobReply>) -> Result<FullText, FullError> {
        let mut replies = replies.into_iter().map(Some).collect::<Vec<_>>();
        let mut take = |index: Option<usize>| match index {
            None => Ok(Vec::new()),
            Some(index) => match replies.get_mut(index).and_then(Option::take) {
                Some(BlobReply::Success { lines }) => Ok(lines),
                Some(BlobReply::Error { error })
                    if matches!(error.kind.as_str(), "not-found" | "too-large") =>
                {
                    Err(FullError::Unavailable)
                }
                _ => Err(FullError::Failed),
            },
        };
        Ok(FullText {
            before: take(self.before)?,
            after: take(self.after)?,
        })
    }
}

fn line_text(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn end(hunk: &Hunk) -> (u32, u32) {
    (
        hunk.old_start
            + hunk
                .lines
                .iter()
                .filter(|line| line.kind != Kind::Added)
                .count() as u32,
        hunk.new_start
            + hunk
                .lines
                .iter()
                .filter(|line| line.kind != Kind::Removed)
                .count() as u32,
    )
}

fn same_line(before: &str, after: &str, ignore_whitespace: bool) -> bool {
    if !ignore_whitespace {
        return before == after;
    }
    let visible = |byte: &u8| !matches!(*byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b);
    before
        .bytes()
        .filter(visible)
        .eq(after.bytes().filter(visible))
}

impl FullText {
    pub(super) fn validate(&self, hunks: &[Hunk], ignore_whitespace: bool) -> bool {
        for hunk in hunks {
            let (mut old, mut new) = (hunk.old_start, hunk.new_start);
            for line in &hunk.lines {
                if line.kind != Kind::Added {
                    if old
                        .checked_sub(1)
                        .and_then(|i| self.before.get(i as usize))
                        .is_none_or(|actual| {
                            !same_line(line_text(actual), &line.text, ignore_whitespace)
                        })
                    {
                        return false;
                    }
                    old += 1;
                }
                if line.kind != Kind::Removed {
                    if new
                        .checked_sub(1)
                        .and_then(|i| self.after.get(i as usize))
                        .is_none_or(|actual| {
                            !same_line(line_text(actual), &line.text, ignore_whitespace)
                        })
                    {
                        return false;
                    }
                    new += 1;
                }
            }
        }
        true
    }

    fn context(
        &self,
        old: u32,
        new: u32,
        count: u32,
        ignore_whitespace: bool,
    ) -> Option<Vec<Line>> {
        (0..count)
            .map(|offset| {
                let before = self
                    .before
                    .get(old.checked_sub(1)? as usize + offset as usize)?;
                let after = self
                    .after
                    .get(new.checked_sub(1)? as usize + offset as usize)?;
                if !same_line(before, after, ignore_whitespace) {
                    return None;
                }
                Some(Line {
                    kind: Kind::Context,
                    number: new + offset,
                    old_number: old + offset,
                    text: line_text(after).to_owned().into(),
                    matches: Vec::new(),
                    match_start: 0,
                    word_changes: Vec::new(),
                    syntax: Vec::new(),
                })
            })
            .collect()
    }
}

pub(super) fn label(count: u32) -> Option<SharedString> {
    (count > 0).then(|| {
        format!(
            "{count} unmodified line{}",
            if count == 1 { "" } else { "s" }
        )
        .into()
    })
}

pub(super) fn append_trailing_gap(hunks: &mut Vec<Hunk>, full: &FullText) {
    let Some(last) = hunks.last() else { return };
    let (old, new) = end(last);
    let old_end = full.before.len() as u32 + 1;
    let new_end = full.after.len() as u32 + 1;
    if let (Some(old_gap), Some(new_gap)) = (old_end.checked_sub(old), new_end.checked_sub(new))
        && old_gap > 0
        && old_gap == new_gap
    {
        hunks.push(Hunk {
            old_start: old_end,
            new_start: new_end,
            separator: label(new_gap),
            lines: Vec::new(),
        });
    }
}

#[derive(Clone, Copy)]
pub(super) enum Expand {
    Before,
    After,
    All,
}

pub(super) fn expand(
    hunks: &mut [Hunk],
    full: &FullText,
    index: usize,
    direction: Expand,
    ignore_whitespace: bool,
) -> bool {
    let Some(current) = hunks.get(index) else {
        return false;
    };
    let (old, new) = if index == 0 {
        (1, 1)
    } else {
        end(&hunks[index - 1])
    };
    let Some(count) = current.new_start.checked_sub(new) else {
        return false;
    };
    if current.old_start.checked_sub(old) != Some(count) || count == 0 {
        return false;
    }
    let amount = if matches!(direction, Expand::All) {
        count
    } else {
        count.min(20)
    };
    let append = matches!(direction, Expand::After) && index > 0;
    let (start_old, start_new) = if append {
        (old, new)
    } else {
        (current.old_start - amount, current.new_start - amount)
    };
    let Some(lines) = full.context(start_old, start_new, amount, ignore_whitespace) else {
        return false;
    };
    if append {
        hunks[index - 1].lines.extend(lines);
    } else {
        hunks[index].old_start -= amount;
        hunks[index].new_start -= amount;
        hunks[index].lines.splice(0..0, lines);
    }
    hunks[index].separator = label(count - amount);
    true
}

pub(super) fn object_ids(patch: &str) -> (Option<String>, Option<String>) {
    let Some(index) = patch.lines().find_map(|line| line.strip_prefix("index ")) else {
        return (None, None);
    };
    let Some((before, after)) = index
        .split_whitespace()
        .next()
        .and_then(|ids| ids.split_once(".."))
    else {
        return (None, None);
    };
    let oid = |value: &str| {
        (!value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|b| b.is_ascii_hexdigit())
            && value.bytes().any(|b| b != b'0'))
        .then(|| value.to_owned())
    };
    (oid(before), oid(after))
}

impl DiffPanel {
    pub fn loads_full_files(&self) -> bool {
        self.full_files
    }

    pub fn full_file_error(&self, index: usize) -> Option<FullError> {
        self.files.get(index).and_then(|file| file.full_error)
    }

    pub fn set_preferences(&mut self, values: &serde_json::Map<String, serde_json::Value>) {
        self.rich_preview = values
            .get("diffRichPreview")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.full_files = values
            .get("load-full-files")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        self.word_diffs = values
            .get("wordDiffsEnabled.2")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        self.wrap = values
            .get("wrapCodeDiff.2")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.hide_whitespace = values
            .get("hideDiffWhitespace")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.hide_imports = values
            .get("hideDiffImports")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.layout = match values
            .get("editorDiffViewMode")
            .and_then(serde_json::Value::as_str)
        {
            Some("split") => DiffLayout::Split,
            Some("unified") => DiffLayout::Unified,
            _ => DiffLayout::Auto,
        };
    }

    fn request_full_file(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .files
            .get(index)
            .is_some_and(|file| preview::enabled(&file.name, self.rich_preview))
        {
            return;
        }
        let Some(path) = self.file_path(index) else {
            return;
        };
        let cwd = if self.source == DiffSource::LastTurn {
            self.cwd.clone()
        } else {
            self.file_root().to_owned()
        };
        let fallback = matches!(
            self.source,
            DiffSource::LastTurn | DiffSource::Unstaged | DiffSource::Uncommitted
        );
        let Some(file) = self.files.get_mut(index) else {
            return;
        };
        if file.full_loading
            || file.full.is_some()
            || file.full_error.is_some()
            || file.problem.is_some()
        {
            return;
        }
        let (before_oid, after_oid) = object_ids(&file.raw);
        let before = file.raw_kind != Some(Kind::Added)
            && !file
                .raw
                .lines()
                .any(|line| line == "--- /dev/null" || line.starts_with("new file mode "));
        let after = file.raw_kind != Some(Kind::Removed)
            && !file
                .raw
                .lines()
                .any(|line| line == "+++ /dev/null" || line.starts_with("deleted file mode "));
        if before && before_oid.is_none() {
            file.full_error = Some(FullError::Unavailable);
            return;
        }
        let mut requests = Vec::new();
        let before = before.then(|| {
            requests.push(BlobRequest {
                path: path.clone(),
                oid: before_oid,
                fallback_to_disk: false,
            });
            requests.len() - 1
        });
        let after = after.then(|| {
            requests.push(BlobRequest {
                path,
                oid: after_oid,
                fallback_to_disk: fallback,
            });
            requests.len() - 1
        });
        file.full_loading = true;
        cx.emit(DiffRequest::LoadFullFile(FullFileRequest {
            index,
            generation: self.content_generation,
            cwd,
            requests,
            before,
            after,
        }));
    }

    pub(super) fn request_visible_full_files(&mut self, cx: &mut Context<Self>) {
        if !self.full_files || self.files.is_empty() {
            return;
        }
        let (first, last) = if self.capped() {
            (self.active_file, self.active_file)
        } else {
            (self.scroll.top_item(), self.scroll.bottom_item())
        };
        for index in first..=last.min(self.files.len() - 1) {
            if self.files[index].open {
                self.request_full_file(index, cx);
            }
        }
    }

    pub fn full_file_loaded(
        &mut self,
        request: &FullFileRequest,
        result: Result<FullText, FullError>,
        cx: &mut Context<Self>,
    ) {
        if request.generation != self.content_generation {
            return;
        }
        let Some(file) = self.files.get_mut(request.index) else {
            return;
        };
        file.full_loading = false;
        let pending = file.pending_expansion.take();
        match result {
            Ok(full)
                if full.validate(
                    file.original_hunks.as_deref().unwrap_or(&file.hunks),
                    self.hide_whitespace,
                ) =>
            {
                if self.full_files || pending.is_some() {
                    if file.original_hunks.is_none() {
                        file.original_hunks = Some(file.hunks.clone());
                    }
                    append_trailing_gap(&mut file.hunks, &full);
                }
                file.full = Some(full);
                file.full_error = None;
            }
            Ok(_) => file.full_error = Some(FullError::Unavailable),
            Err(error) => file.full_error = Some(error),
        }
        if file.full.is_some() {
            if let Some((hunk, direction)) = pending {
                self.expand_context(request.index, hunk, direction, cx);
            }
        } else {
            file.pending_expansion = pending;
        }
        self.refresh_filtered_view();
        cx.notify();
    }

    pub(super) fn toggle_full_files(&mut self, cx: &mut Context<Self>) {
        self.full_files = !self.full_files;
        for file in &mut self.files {
            if !self.full_files {
                if let Some(hunks) = file.original_hunks.take() {
                    file.hunks = hunks;
                }
            } else if let Some(full) = &file.full {
                if file.original_hunks.is_none() {
                    file.original_hunks = Some(file.hunks.clone());
                }
                append_trailing_gap(&mut file.hunks, full);
            }
        }
        self.refresh_filtered_view();
        self.request_visible_full_files(cx);
        cx.emit(DiffRequest::Preference(
            "host.load-full-files",
            self.full_files.into(),
        ));
        cx.notify();
    }

    pub(super) fn expand_context(
        &mut self,
        file: usize,
        hunk: usize,
        direction: Expand,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.files.get_mut(file) else {
            return;
        };
        if let Some(full) = &entry.full {
            if expand(
                &mut entry.hunks,
                full,
                hunk,
                direction,
                self.hide_whitespace,
            ) {
                syntax::apply(&entry.name, &mut entry.hunks);
                self.selection = None;
                self.refresh_filtered_view();
                cx.notify();
            }
        } else {
            entry.pending_expansion = Some((hunk, direction));
            entry.full_error = None;
            self.request_full_file(file, cx);
            cx.notify();
        }
    }

    pub(super) fn retry_full_file(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(index) {
            file.full_error = None;
        }
        self.request_full_file(index, cx);
        cx.notify();
    }

    pub(super) fn context_row(
        &self,
        label: SharedString,
        location: (usize, usize),
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let (file, hunk) = location;
        let enabled = self
            .files
            .get(file)
            .is_some_and(|file| file.full_error != Some(FullError::Unavailable));
        let t = cx.theme();
        let background = rgb(if t.appearance == Appearance::Dark {
            0x2f2f2f
        } else {
            0xf4f4f4
        });
        div()
            .h(px(32.))
            .w_full()
            .px(px(8.))
            .bg(t.surface)
            .child(
                div()
                    .h_full()
                    .w_full()
                    .relative()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .font_family(SANS)
                    .px(px(7.624_2))
                    .child(shapes::decoration(Decoration {
                        fill: Some(background.into()),
                        radius: Corners {
                            top_left: px(6.),
                            top_right: px(8.),
                            bottom_right: px(8.),
                            bottom_left: px(6.),
                        },
                        ..Default::default()
                    }))
                    .child(text(
                        label,
                        CODE_SIZE,
                        CODE_LINE,
                        430.,
                        line_colors(Kind::Context, t).2,
                    )),
            )
            .id(SharedString::from(format!("diff-context-{file}-{hunk}")))
            .group("diff-context-gap")
            .relative()
            .role(gpui::Role::Button)
            .aria_label("Expand unmodified lines")
            .aria_disabled(!enabled)
            .when(enabled, |d| d.focusable().cursor_pointer())
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    this.expand_context(file, hunk, Expand::All, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.expand_context(file, hunk, Expand::All, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .top_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .children(
                        [
                            (Expand::After, "composer-arrow-up", "Expand 20 lines above"),
                            (Expand::Before, "diff-chevron-down", "Expand 20 lines below"),
                            (
                                Expand::All,
                                "diff-load-full-files",
                                "Expand all unmodified lines",
                            ),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(button, (direction, name, label))| {
                            let focus = self.focus.clone();
                            round_icon(name, 20., t)
                                .id(SharedString::from(format!(
                                    "diff-context-button-{file}-{hunk}-{button}"
                                )))
                                .role(gpui::Role::Button)
                                .aria_label(label)
                                .aria_disabled(!enabled)
                                .opacity(0.)
                                .group_hover("diff-context-gap", |d| d.opacity(1.))
                                .focus(|d| d.opacity(1.))
                                .when(enabled, |d| d.focusable().cursor_pointer())
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    if let Some(focus) = &focus {
                                        window.focus(focus, cx);
                                    }
                                    cx.stop_propagation();
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if enabled {
                                        this.expand_context(file, hunk, direction, cx);
                                    }
                                    cx.stop_propagation();
                                }))
                                .on_key_down(cx.listener(
                                    move |this, event: &KeyDownEvent, _, cx| {
                                        if enabled
                                            && matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            )
                                        {
                                            this.expand_context(file, hunk, direction, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                        }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn full_error_notice(&self, index: usize, cx: &Context<Self>) -> Option<Div> {
        if self.files.get(index)?.full_error != Some(FullError::Failed) {
            return None;
        }
        let t = cx.theme();
        Some(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .child(text(
                    "Full file content failed to load",
                    13.,
                    20.,
                    400.,
                    t.text_secondary,
                ))
                .child(
                    div()
                        .id(("diff-full-retry", index))
                        .role(gpui::Role::Button)
                        .aria_label("Retry")
                        .focusable()
                        .cursor_pointer()
                        .px(px(8.))
                        .py(px(4.))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.retry_full_file(index, cx)),
                        )
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.retry_full_file(index, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(text("Retry", 13., 20., 430., t.text)),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_expands_twenty_lines_and_preserves_change_numbers() {
        let before = (1..=60).map(|n| format!("line {n}\n")).collect::<Vec<_>>();
        let mut after = before.clone();
        after[39] = "changed 40\n".into();
        let full = FullText { before, after };
        let mut hunks = parse("@@ -40 +40 @@\n-line 40\n+changed 40\n", None).unwrap();
        assert!(full.validate(&hunks, false));
        assert!(expand(&mut hunks, &full, 0, Expand::Before, false));
        assert_eq!(hunks[0].new_start, 20);
        assert_eq!(hunks[0].lines.last().unwrap().number, 40);
        assert_eq!(hunks[0].separator.as_deref(), Some("19 unmodified lines"));
        assert!(expand(&mut hunks, &full, 0, Expand::All, false));
        assert_eq!(hunks[0].new_start, 1);
        assert!(hunks[0].separator.is_none());
        append_trailing_gap(&mut hunks, &full);
        assert!(expand(&mut hunks, &full, 1, Expand::All, false));
        assert_eq!(hunks[1].lines.last().unwrap().number, 60);
    }

    #[test]
    fn stale_full_content_never_expands_the_reviewed_snapshot() {
        let mut hunks = parse("@@ -2 +2 @@\n-before\n+after\n", None).unwrap();
        let full = FullText {
            before: vec!["line\n".into(), "other\n".into()],
            after: vec!["changed\n".into(), "after\n".into()],
        };
        assert!(!full.validate(&hunks, false));
        assert!(!expand(&mut hunks, &full, 0, Expand::All, false));
    }

    #[test]
    fn ignored_whitespace_does_not_hide_stale_context() {
        let mut hunks = parse("@@ -2 +2 @@\n-old\n+new\n", None).unwrap();
        let mut full = FullText {
            before: vec!["let n = 1;\n".into(), "old\n".into()],
            after: vec!["let n=1;\n".into(), "new\n".into()],
        };
        assert!(!expand(&mut hunks, &full, 0, Expand::All, false));
        assert!(expand(&mut hunks, &full, 0, Expand::All, true));
        assert!(full.validate(&hunks, true));
        assert!(!full.validate(&hunks, false));
        full.before[0] = "different();\n".into();
        assert!(!full.validate(&hunks, true));
    }
}
