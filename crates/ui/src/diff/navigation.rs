use super::*;
use crate::screens::sidebar::MatchPattern;
use gpui::{Anchor, AnyElement, anchored, point};

pub(super) struct JumpToFile {
    pub(super) query: controls::TextField,
    matches: Vec<usize>,
    active: usize,
}

fn matching_files(files: &[FileDiff], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        let mut matches = (0..files.len()).collect::<Vec<_>>();
        matches.sort_by_cached_key(|index| {
            (
                files[*index].name.to_lowercase(),
                files[*index].dir.to_lowercase(),
            )
        });
        return matches;
    }
    let pattern = MatchPattern::new(query);
    let mut matches = files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            let name_score = pattern.score(&file.name);
            let score = if name_score > 0 {
                name_score
            } else {
                pattern.score(&format!("{}{}", file.dir, file.name))
            };
            (score > 0).then_some((index, score))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
    matches.into_iter().map(|(index, _)| index).collect()
}

impl DiffPanel {
    pub(super) fn scrolling_file_header(
        &self,
        index: usize,
        file: &FileDiff,
        top: (usize, Pixels),
        t: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
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
                this.files[index].open ^= true;
                this.active_file = index;
                this.selection = None;
                cx.notify();
            }));
        if offset > px(0.) {
            deferred(header.relative().top(offset).bg(t.surface))
                .with_priority(1)
                .into_any_element()
        } else {
            header.into_any_element()
        }
    }

    pub(super) fn open_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.jump = Some(JumpToFile {
            query: controls::TextField::default(),
            matches: matching_files(&self.files, ""),
            active: 0,
        });
        if let Some(focus) = &self.focus {
            window.focus(focus, cx);
        }
        cx.notify();
    }

    pub(super) fn refresh_jump(&mut self) {
        if let Some(jump) = &mut self.jump {
            jump.matches = matching_files(&self.files, &jump.query.text);
            jump.active = jump.active.min(jump.matches.len().saturating_sub(1));
        }
    }

    pub fn reveal_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.reveal_path = Some(path);
        self.apply_reveal_path();
        cx.notify();
    }

    pub(super) fn apply_reveal_path(&mut self) {
        let Some(path) = self.reveal_path.as_deref() else {
            return;
        };
        let path = path
            .strip_prefix(&self.workspace)
            .and_then(|path| path.strip_prefix('/'))
            .unwrap_or(path);
        if let Some(index) = self
            .files
            .iter()
            .position(|file| path.strip_prefix(file.dir.as_ref()) == Some(file.name.as_ref()))
        {
            self.select_jump(index);
        }
    }

    pub(super) fn select_jump(&mut self, index: usize) {
        if let Some(file) = self.files.get_mut(index) {
            file.open = true;
            self.active_file = index;
            self.scroll.scroll_to_top_of_item(index);
        }
        self.jump = None;
        self.reveal_path = None;
    }

    pub(super) fn jump_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(jump) = &mut self.jump else { return };
        let count = jump.matches.len();
        match event.keystroke.key.as_str() {
            "escape" => self.jump = None,
            "enter" => {
                if let Some(index) = jump.matches.get(jump.active).copied() {
                    self.select_jump(index);
                }
            }
            "up" if count > 0 => jump.active = (jump.active + count - 1) % count,
            "down" if count > 0 => jump.active = (jump.active + 1) % count,
            "home" => jump.active = 0,
            "end" => jump.active = count.saturating_sub(1),
            _ => {
                if !jump.query.key(event) {
                    return;
                }
                jump.active = 0;
                self.refresh_jump();
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn jump_overlay(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let jump = self.jump.as_ref().expect("open file jump");
        let t = cx.theme();
        let rows = jump.matches.iter().enumerate().map(|(row, &index)| {
            let file = &self.files[index];
            div()
                .id(("diff-jump-result", index))
                .role(gpui::Role::MenuItem)
                .aria_label(format!("{}{}", file.dir, file.name))
                .aria_selected(row == jump.active)
                .min_h(px(29.))
                .px(px(8.))
                .py(px(5.))
                .rounded(px(12.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .when(row == jump.active, |d| d.bg(t.menu_item_hover))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && let Some(jump) = &mut this.jump {
                        jump.active = row;
                        cx.notify();
                    }
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_jump(index);
                    cx.notify();
                }))
                .child(text(file.name.clone(), 13., 18.571_428, 430., t.text))
                .child(text(
                    file.dir.trim_end_matches('/').to_owned(),
                    13.,
                    18.571_428,
                    430.,
                    t.text_tertiary,
                ))
        });
        let popup = crate::composer::popover(t, 360., |d| {
            d.child(
                div()
                    .id("diff-jump-input")
                    .role(gpui::Role::TextInput)
                    .aria_label("Jump to file")
                    .aria_placeholder("Jump to file")
                    .aria_value(jump.query.text.clone())
                    .px(px(12.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("search", 12., t.text_tertiary))
                    .child(
                        text(
                            if jump.query.text.is_empty() {
                                "Jump to file"
                            } else {
                                &jump.query.text
                            },
                            13.,
                            18.571_428,
                            430.,
                            if jump.query.text.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .when(jump.query.selected, |d| d.bg(t.text_selection)),
                    ),
            )
            .child(
                div()
                    .id("diff-jump-results")
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows)
                    .when(jump.matches.is_empty(), |d| {
                        d.child(
                            text("No matching files", 13., 18.571_428, 430., t.text_secondary)
                                .p(px(12.)),
                        )
                    }),
            )
        })
        .ml_0()
        .pl_0();
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.jump = None;
                            cx.notify();
                        }),
                    )
                    .child(
                        anchored()
                            .position(point(window.viewport_size().width - px(44.), px(83.)))
                            .anchor(Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(popup.on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })),
                    ),
            ),
        )
        .with_priority(4)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_jump_prefers_basename_matches_and_keeps_same_name_paths() {
        let files =
            ["tests/math.rs", "src/math.rs", "src/main.rs"].map(|path| FileDiff::patch(path, ""));
        assert_eq!(matching_files(&files, ""), vec![2, 1, 0]);
        assert_eq!(matching_files(&files, "missing"), Vec::<usize>::new());
    }

    #[test]
    fn pending_review_file_opens_after_load_and_manual_selection_cancels_it() {
        let mut panel = DiffPanel::from_patch("", "");
        panel.workspace = "/repo".into();
        panel.reveal_path = Some("/repo/src/second.rs".into());
        panel.replace_files(vec![FileDiff::patch("src/first.rs", "")]);
        assert!(panel.reveal_path.is_some());

        let mut target = FileDiff::patch("src/second.rs", "");
        target.open = false;
        panel.replace_files(vec![FileDiff::patch("src/first.rs", ""), target]);
        assert_eq!(panel.active_file, 1);
        assert!(panel.files[1].open);
        assert!(panel.reveal_path.is_none());

        panel.reveal_path = Some("/repo/src/third.rs".into());
        panel.select_jump(0);
        panel.replace_files(vec![
            FileDiff::patch("src/first.rs", ""),
            FileDiff::patch("src/second.rs", ""),
            FileDiff::patch("src/third.rs", ""),
        ]);
        assert_eq!(panel.active_file, 0);
        assert!(panel.reveal_path.is_none());
    }
}
