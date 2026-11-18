use super::*;

impl FileDiff {
    pub(super) fn hunk_hidden(&self, index: usize) -> bool {
        self.imports
            .as_ref()
            .is_some_and(|filter| filter.hidden.get(index) == Some(&true))
    }

    pub(super) fn visible_hunks(&self) -> impl Iterator<Item = (usize, &Hunk)> {
        self.hunks
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.hunk_hidden(*index))
    }
}

impl DiffPanel {
    pub fn patch_params(&self) -> Option<serde_json::Value> {
        if !self.can_copy_patch() {
            return None;
        }
        let repository = self.repository.as_ref()?;
        Some(serde_json::json!({
            "cwd": self.working_directory(),
            "reviewSource": self.source.key(),
            "beforeRef": repository.before_ref,
            "afterRef": repository.after_ref,
        }))
    }

    pub(super) fn can_copy_patch(&self) -> bool {
        self.repository
            .as_ref()
            .is_some_and(|repository| !repository.not_repository)
            && !self.pending
            && self.error.is_none()
            && !matches!(self.source, DiffSource::LastTurn | DiffSource::Cloud)
    }

    pub(super) fn set_layout(&mut self, layout: DiffLayout, cx: &mut Context<Self>) {
        if self.layout == layout {
            return;
        }
        self.layout = layout;
        self.selection = None;
        let name = match layout {
            DiffLayout::Auto => "auto",
            DiffLayout::Unified => "unified",
            DiffLayout::Split => "split",
        };
        cx.emit(DiffRequest::Preference(
            "host.editorDiffViewMode",
            name.into(),
        ));
        cx.notify();
    }

    pub(super) fn toggle_whitespace(&mut self, cx: &mut Context<Self>) {
        self.hide_whitespace = !self.hide_whitespace;
        cx.emit(DiffRequest::Preference(
            "host.hideDiffWhitespace",
            self.hide_whitespace.into(),
        ));
        if !self.pending && !matches!(self.source, DiffSource::LastTurn | DiffSource::Cloud) {
            cx.emit(DiffRequest::Refresh);
        }
        cx.notify();
    }

    pub(super) fn toggle_imports(&mut self, cx: &mut Context<Self>) {
        self.hide_imports = !self.hide_imports;
        self.refresh_filtered_view();
        cx.emit(DiffRequest::Preference(
            "host.hideDiffImports",
            self.hide_imports.into(),
        ));
        cx.notify();
    }

    pub(super) fn refresh_filtered_view(&mut self) {
        let annotations = &self.annotations;
        for file in &mut self.files {
            let enabled = self.hide_imports
                && self.full_files
                && file.raw_kind.is_none()
                && !file.raw.lines().any(|line| {
                    line.starts_with("new file mode ") || line.starts_with("deleted file mode ")
                });
            if enabled && let Some(full) = &file.full {
                file.imports
                    .get_or_insert_with(|| imports::ImportFilter::new(&file.name, full))
                    .update(&file.hunks, |hunk| {
                        annotations.intersects(&file.dir, &file.name, hunk)
                    });
            } else if let Some(filter) = &mut file.imports {
                filter.hidden.clear();
            }
            let Some(filter) = &file.imports else {
                continue;
            };
            let mut end = 0;
            for (index, hunk) in file.hunks.iter_mut().enumerate() {
                if filter.hidden.get(index) == Some(&true) {
                    continue;
                }
                let count = hunk
                    .lines
                    .iter()
                    .filter(|line| line.kind != Kind::Removed)
                    .count() as u32;
                let start = hunk
                    .new_start
                    .saturating_sub(u32::from(count > 0 || hunk.lines.is_empty()));
                hunk.separator = content::label(start.saturating_sub(end));
                end = start + count;
            }
        }
        self.search_changed();
    }
    // 26.1029 labels the toggle from the panel-wide flag (`diffControls.expanded`), not from
    // the files: `View changes` opens one file and still offers "Expand all diffs".
    pub(super) fn all_expanded(&self) -> bool {
        self.files_expanded
    }

    pub(super) fn option_checked(&self, index: usize) -> Option<bool> {
        match index {
            1 => Some(self.wrap),
            4 => Some(self.full_files),
            5 => Some(self.rich_preview),
            6 => Some(self.word_diffs),
            7 => Some(self.hide_whitespace),
            8 => Some(self.hide_imports),
            _ => None,
        }
    }

    pub(super) fn option_icon(&self, index: usize) -> &'static str {
        match index {
            0 => "diff-refresh",
            1 => "diff-word-wrap",
            2 => self.layout.icon(),
            3 if self.all_expanded() => "diff-collapse-all",
            3 => "diff-expand-all",
            4 => "diff-load-file",
            5 => "diff-rich-preview",
            6 => "diff-word-changes",
            7 => "diff-hide-whitespace",
            8 => "diff-hide-imports",
            _ => "diff-copy-apply",
        }
    }
}
