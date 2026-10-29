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
    TooLarge,
    Failed,
}

impl FullFileRequest {
    pub fn decode(&self, mut replies: Vec<BlobReply>) -> Result<FullText, FullError> {
        if replies.len() != self.requests.len()
            || (self.before.is_some() && self.before == self.after)
        {
            return Err(FullError::Failed);
        }
        let mut take = |index: Option<usize>| match index {
            None => Ok(Vec::new()),
            Some(index) => match replies.get_mut(index) {
                Some(BlobReply::Success { lines }) => Ok(std::mem::take(lines)),
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

#[derive(Clone)]
pub struct FullFileLoad {
    pub index: usize,
    pub generation: u64,
    pub request: FullFileRequest,
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
            layout: Default::default(),
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
) -> Result<bool, FullError> {
    let Some(current) = hunks.get(index) else {
        return Ok(false);
    };
    let (old, new) = if index == 0 {
        (1, 1)
    } else {
        end(&hunks[index - 1])
    };
    let Some(count) = current.new_start.checked_sub(new) else {
        return Ok(false);
    };
    if current.old_start.checked_sub(old) != Some(count) || count == 0 {
        return Ok(false);
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
    let Some(start) = start_new.checked_sub(1).map(|line| line as usize) else {
        return Ok(false);
    };
    let Some(context) = full.after.get(start..start + amount as usize) else {
        return Ok(false);
    };
    if exceeds_diff_limits(
        hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter().map(|line| line.text.as_ref()))
            .chain(context.iter().map(|line| line_text(line))),
    ) {
        return Err(FullError::TooLarge);
    }
    let Some(lines) = full.context(start_old, start_new, amount, ignore_whitespace) else {
        return Ok(false);
    };
    if append {
        hunks[index - 1].lines.extend(lines);
    } else {
        hunks[index].old_start -= amount;
        hunks[index].new_start -= amount;
        hunks[index].lines.splice(0..0, lines);
    }
    hunks[index].separator = label(count - amount);
    Ok(true)
}

#[derive(Default)]
pub(super) struct ContextLayout {
    first_line: Option<Pixels>,
    anchor: Option<(usize, Pixels)>,
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
    pub(super) fn scrolling_file_header(
        &self,
        index: usize,
        file: &FileDiff,
        top: (usize, Pixels),
        t: &Theme,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let offset = if top.0 == index {
            let height = if self.standalone { px(36.) } else { px(39.) };
            let maximum = self
                .scroll
                .bounds_for_item(index)
                .map_or(px(0.), |bounds| (bounds.size.height - height).max(px(0.)));
            (-top.1).max(px(0.)).min(maximum)
        } else {
            px(0.)
        };
        let header =
            file_header(index, file, self, t, cx).on_click(cx.listener(move |this, _, _, cx| {
                this.set_file_open(index, !this.files[index].open);
                this.active_file = index;
                this.selection = None;
                cx.notify();
            }));
        let header = header
            .absolute()
            .top(offset)
            .left_0()
            .right_0()
            .bg(t.surface);
        if self.standalone {
            return header.into_any_element();
        }
        // Sticky above the row gutters, which are deferred (priority 1) for their paint order.
        deferred(header)
            .with_priority(2)
            .clipped()
            .into_any_element()
    }

    pub(super) fn track_hunk_layout(&self, hunk: &Hunk, body: Div, cx: &Context<Self>) -> Div {
        let layout = hunk.layout.clone();
        let separator = usize::from(hunk.separator.is_some());
        let scroll = self.scroll.clone();
        let panel = cx.entity().downgrade();
        body.on_children_prepainted(move |bounds, _, cx| {
            let mut layout = layout.borrow_mut();
            layout.first_line = bounds.get(separator).map(|bounds| bounds.origin.y);
            let correction = layout.anchor.take().and_then(|(line, original)| {
                bounds
                    .get(line + separator)
                    .map(|bounds| bounds.origin.y - original)
            });
            drop(layout);
            if let Some(correction) = correction
                && f32::from(correction).abs() > 0.001
            {
                let mut offset = scroll.offset();
                offset.y -= correction;
                scroll.set_offset(offset);
                let _ = panel.update(cx, |_, cx| cx.notify());
            }
        })
    }

    pub fn loads_full_files(&self) -> bool {
        self.full_files
    }

    pub fn full_file_error(&self, index: usize) -> Option<FullError> {
        self.files.get(index).and_then(|file| file.full_error)
    }

    pub fn set_preferences(&mut self, values: &serde_json::Map<String, serde_json::Value>) {
        self.viewed_files = values
            .get("review-viewed-file-v1")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
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
        if self.pending
            || self
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
        cx.emit(DiffRequest::LoadFullFile(FullFileLoad {
            index,
            generation: self.content_generation,
            request: FullFileRequest {
                cwd,
                requests,
                before,
                after,
            },
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
        request: &FullFileLoad,
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
            let Some(start) = entry.hunks.get(hunk).map(|hunk| hunk.new_start) else {
                return;
            };
            let original = entry.hunks[hunk].layout.borrow().first_line;
            match expand(
                &mut entry.hunks,
                full,
                hunk,
                direction,
                self.hide_whitespace,
            ) {
                Ok(true) => {
                    let expanded = &entry.hunks[hunk];
                    if let Some(original) = original {
                        expanded.layout.borrow_mut().anchor =
                            Some(((start - expanded.new_start) as usize, original));
                    }
                    syntax::apply(&entry.name, &mut entry.hunks);
                    entry.full_error = None;
                    self.selection = None;
                    self.refresh_filtered_view();
                }
                Ok(false) => {}
                Err(error) => entry.full_error = Some(error),
            }
            cx.notify();
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
        gutter: Pixels,
        location: (usize, usize),
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let (file, hunk) = location;
        let entry = &self.files[file];
        let enabled = !self.pending && entry.full_error != Some(FullError::Unavailable);
        let t = cx.theme();
        let foreground = line_colors(Kind::Context, t).2;
        let [r, g, b] = if t.appearance == Appearance::Dark {
            [0.183478, 0.183485, 0.183483]
        } else {
            [0.954723, 0.954724, 0.954724]
        };
        let background = gpui::Rgba { r, g, b, a: 1. };
        let button = |direction: Expand, label: &'static str, name: &'static str| {
            let focus = self.focus.clone();
            div()
                .id(SharedString::from(format!(
                    "diff-context-{file}-{hunk}-{name}"
                )))
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(!enabled)
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
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.expand_context(file, hunk, direction, cx);
                        cx.stop_propagation();
                    }
                }))
        };
        div()
            .id(SharedString::from(format!("diff-context-{file}-{hunk}")))
            .group("diff-context-gap")
            .role(gpui::Role::Button)
            .aria_label("Expand all unmodified lines")
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
            .h(px(32.))
            .px(px(8.))
            .flex()
            .font_family(SANS)
            .text_size(px(12.))
            .font_weight(FontWeight(430.))
            .line_height(px(CODE_LINE))
            .text_color(foreground)
            .child(if entry.line_info_separators {
                div()
                    .relative()
                    .w_full()
                    .h_full()
                    .flex()
                    .child(
                        div()
                            .relative()
                            .w(px(f32::from(gutter).ceil()))
                            .flex_none()
                            .h_full()
                            .flex()
                            .flex_col()
                            .pr(px(1.))
                            .child(shapes::decoration(Decoration {
                                fill: Some(background.into()),
                                radius: gpui::Corners {
                                    top_left: px(8.),
                                    bottom_left: px(8.),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }))
                            .children(
                                [
                                    (Expand::After, hunk > 0, "Expand 20 lines below", "after"),
                                    (
                                        Expand::Before,
                                        !entry.hunks[hunk].lines.is_empty(),
                                        "Expand 20 lines above",
                                        "before",
                                    ),
                                ]
                                .into_iter()
                                .filter(|(_, show, _, _)| *show)
                                .enumerate()
                                .map(
                                    |(index, (direction, _, label, name))| {
                                        button(direction, label, name)
                                            .flex_1()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(index > 0, |d| {
                                                d.border_t_2().border_color(t.surface)
                                            })
                                            .hover(|s| s.bg(t.text_alpha_10))
                                            .child(icon(
                                                if matches!(direction, Expand::Before) {
                                                    "diff-context-expand-up"
                                                } else {
                                                    "diff-context-expand"
                                                },
                                                16.,
                                                foreground,
                                            ))
                                    },
                                ),
                            )
                            .child(div().absolute().right_0().top_0().w(px(1.)).h_full().child(
                                shapes::decoration(Decoration {
                                    fill: Some(t.surface),
                                    ..Default::default()
                                }),
                            )),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .pl(px(7.6242))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(shapes::decoration(Decoration {
                                fill: Some(background.into()),
                                radius: gpui::Corners {
                                    top_right: px(8.),
                                    bottom_right: px(8.),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }))
                            .child(label)
                            .child(
                                button(Expand::All, "Expand all unmodified lines", "all")
                                    .h_full()
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .opacity(0.)
                                    .group_hover("diff-context-gap", |d| d.opacity(1.))
                                    .focus(|d| d.opacity(1.))
                                    .child("Expand all"),
                            ),
                    )
            } else {
                div()
                    .relative()
                    .w_full()
                    .h_full()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .px(px(7.6242))
                    .child(shapes::decoration(Decoration {
                        fill: Some(background.into()),
                        radius: gpui::Corners {
                            top_left: px(6.),
                            bottom_left: px(6.),
                            top_right: px(8.),
                            bottom_right: px(8.),
                        },
                        ..Default::default()
                    }))
                    .child(label)
            })
            .into_any_element()
    }

    pub(super) fn full_error_notice(&self, index: usize, cx: &Context<Self>) -> Option<Div> {
        let error = self.files.get(index)?.full_error?;
        let message = match error {
            FullError::Failed => "Full file content failed to load",
            FullError::TooLarge => "Diff too large to display",
            FullError::Unavailable => return None,
        };
        let t = cx.theme();
        Some(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .child(text(message, 13., 20., 400., t.text_secondary))
                .when(error == FullError::Failed, |d| {
                    d.child(
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
                    )
                }),
        )
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn context_expands_twenty_lines_and_preserves_change_numbers() {
        let before = (1..=60).map(|n| format!("line {n}\n")).collect::<Vec<_>>();
        let mut after = before.clone();
        after[39] = "changed 40\n".into();
        let full = FullText { before, after };
        let mut hunks = parse("@@ -40 +40 @@\n-line 40\n+changed 40\n", None).unwrap();
        assert!(full.validate(&hunks, false));
        assert!(expand(&mut hunks, &full, 0, Expand::Before, false).unwrap());
        assert_eq!(hunks[0].new_start, 20);
        assert_eq!(hunks[0].lines.last().unwrap().number, 40);
        assert_eq!(hunks[0].separator.as_deref(), Some("19 unmodified lines"));
        assert!(expand(&mut hunks, &full, 0, Expand::All, false).unwrap());
        assert_eq!(hunks[0].new_start, 1);
        assert!(hunks[0].separator.is_none());
        append_trailing_gap(&mut hunks, &full);
        assert!(expand(&mut hunks, &full, 1, Expand::All, false).unwrap());
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
        assert!(!expand(&mut hunks, &full, 0, Expand::All, false).unwrap());
    }

    #[test]
    fn empty_ranges_expand_into_both_complete_file_versions() {
        for insertion in [true, false] {
            let before: Vec<_> = (1..=60).map(|n| format!("line {n}\n")).collect();
            let mut after = before.clone();
            let patch = if insertion {
                after.insert(40, "inserted\n".into());
                "@@ -40,0 +41 @@\n+inserted\n"
            } else {
                after.remove(39);
                "@@ -40 +39,0 @@\n-line 40\n"
            };
            let full = FullText { before, after };
            let mut hunks = parse(patch, None).unwrap();
            assert!(full.validate(&hunks, false));
            append_trailing_gap(&mut hunks, &full);
            assert!(expand(&mut hunks, &full, 0, Expand::All, false).unwrap());
            let last = hunks.len() - 1;
            assert!(expand(&mut hunks, &full, last, Expand::All, false).unwrap());
            for (excluded, expected) in [(Kind::Added, &full.before), (Kind::Removed, &full.after)]
            {
                let actual: Vec<_> = hunks
                    .iter()
                    .flat_map(|hunk| &hunk.lines)
                    .filter(|line| line.kind != excluded)
                    .map(|line| line.text.as_ref())
                    .collect();
                assert_eq!(
                    actual,
                    expected
                        .iter()
                        .map(|line| line_text(line))
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn oversized_expansion_preserves_the_patch_and_allows_a_bounded_step() {
        let before: Vec<_> = (1..=100_001).map(|n| format!("line {n}\n")).collect();
        let mut after = before.clone();
        after[39] = "changed 40\n".into();
        let full = FullText { before, after };
        let mut hunks = parse("@@ -40 +40 @@\n-line 40\n+changed 40\n", None).unwrap();
        append_trailing_gap(&mut hunks, &full);
        assert_eq!(
            expand(&mut hunks, &full, 1, Expand::All, false),
            Err(FullError::TooLarge)
        );
        assert!(hunks[1].lines.is_empty());
        assert_eq!(hunks[0].lines[1].text.as_ref(), "changed 40");
        assert!(expand(&mut hunks, &full, 1, Expand::After, false).unwrap());
        assert_eq!(hunks[0].lines.last().unwrap().number, 60);
    }

    #[gpui::test]
    fn new_turn_item_paths_do_not_keep_the_previous_aggregate_root(cx: &mut gpui::TestAppContext) {
        let first: Turn = serde_json::from_value(serde_json::json!({
            "id":"first", "status":"completed", "items":[]
        }))
        .unwrap();
        let second: Turn = serde_json::from_value(serde_json::json!({
            "id":"second", "status":"inProgress",
            "items":[{"type":"fileChange","id":"change","status":"completed","changes":[{
                "path":"/repo/worktrees/task/file.rs", "kind":{"type":"update"},
                "diff":"@@ -1 +1 @@\n-before\n+after\n"
            }]}]
        }))
        .unwrap();
        let panel = cx.new(|_| DiffPanel::last_turn("/repo", "/repo/worktrees/task", &[]));
        panel.update(cx, |panel, cx| {
            panel.apply(&Event::Notification {
                method:"turn/diff/updated".into(),
                params:serde_json::json!({"turnId":"first","diff":"diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-before\n+after\n"}),
            }, "/repo", &[first], cx);
            panel.apply(&Event::Notification {
                method:"item/completed".into(),
                params:serde_json::json!({"turnId":"second","item":{"type":"fileChange"}}),
            }, "/repo", &[second], cx);
            assert_eq!(panel.file_path(0).as_deref(), Some("/repo/worktrees/task/file.rs"));
        });
    }

    #[gpui::test]
    fn old_full_file_reply_cannot_clear_a_new_generation_load(cx: &mut gpui::TestAppContext) {
        let panel = cx.new(|_| DiffPanel::from_patch(
            "/repo",
            "diff --git a/file.txt b/file.txt\n--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-before\n+first\n",
        ));
        panel.update(cx, |panel, cx| {
            let request = FullFileLoad {
                index: 0,
                generation: panel.content_generation,
                request: FullFileRequest {
                    cwd: "/repo".into(),
                    requests: Vec::new(),
                    before: None,
                    after: None,
                },
            };
            panel.files[0].full_loading = true;
            panel.replace_files(vec![FileDiff::patch(
                "file.txt",
                "@@ -1 +1 @@\n-before\n+second\n",
            )]);
            assert!(!panel.files[0].full_loading);
            panel.files[0].full_loading = true;
            panel.full_file_loaded(
                &request,
                Ok(FullText {
                    before: vec!["before\n".into()],
                    after: vec!["first\n".into()],
                }),
                cx,
            );
            assert!(panel.files[0].full_loading);
            assert!(panel.files[0].full.is_none());
            assert!(panel.files[0].full_error.is_none());
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> FullFileRequest {
        FullFileRequest {
            cwd: "/repo".into(),
            requests: (0..2)
                .map(|_| BlobRequest {
                    path: "file.txt".into(),
                    oid: None,
                    fallback_to_disk: false,
                })
                .collect(),
            before: Some(0),
            after: Some(1),
        }
    }

    #[test]
    fn incomplete_or_reused_responses_fail() {
        let mut request = request();
        assert_eq!(request.decode(vec![]).err(), Some(FullError::Failed));
        request.after = request.before;
        let replies = (0..2)
            .map(|_| BlobReply::Success {
                lines: vec!["line\n".into()],
            })
            .collect();
        assert_eq!(request.decode(replies).err(), Some(FullError::Failed));
    }

    #[test]
    fn ignored_whitespace_does_not_hide_stale_context() {
        let mut hunks = parse("@@ -2 +2 @@\n-old\n+new\n", None).unwrap();
        let mut full = FullText {
            before: vec!["let n = 1;\n".into(), "old\n".into()],
            after: vec!["let n=1;\n".into(), "new\n".into()],
        };
        assert!(!expand(&mut hunks, &full, 0, Expand::All, false).unwrap());
        assert!(expand(&mut hunks, &full, 0, Expand::All, true).unwrap());
        assert!(full.validate(&hunks, true));
        assert!(!full.validate(&hunks, false));
        full.before[0] = "different();\n".into();
        assert!(!full.validate(&hunks, true));
    }

    #[gpui::test]
    fn headerless_turn_preserves_partial_metadata(cx: &mut gpui::TestAppContext) {
        let panel = cx.new(|_| DiffPanel::subtract());
        panel.update(cx, |panel, cx| {
            panel.request_full_file(0, cx);
            assert_eq!(panel.files[0].full_error, Some(FullError::Unavailable));
            assert!(!panel.files[0].line_info_separators);
            assert_eq!(
                panel.files[0].hunks[0].separator.as_deref(),
                Some("1 unmodified line")
            );
        });
    }
}
