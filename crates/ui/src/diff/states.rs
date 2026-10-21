use super::*;

#[derive(serde::Deserialize)]
pub struct OmittedFiles {
    pub count: usize,
    pub limit: usize,
}

#[derive(Clone, Copy)]
pub enum LastTurnState {
    Reverted,
    CommittedOrReverted,
    Unavailable,
}

impl LastTurnState {
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Reverted => "The last turn was reverted.",
            Self::CommittedOrReverted => "The last turn was committed or reverted.",
            Self::Unavailable => "The latest diffs are no longer available.",
        }
    }
}

impl DiffPanel {
    pub fn watch_root(&self) -> Option<&str> {
        if matches!(self.source, DiffSource::LastTurn | DiffSource::Cloud) {
            return None;
        }
        self.repository
            .as_ref()
            .filter(|repo| !repo.not_repository)
            .map(|repo| repo.root.as_str())
    }

    pub fn watch_limited(&self) -> bool {
        self.watch_limited
    }

    pub fn set_watch_limited(&mut self, limited: bool, cx: &mut Context<Self>) {
        if self.watch_limited != limited {
            self.watch_limited = limited;
            cx.notify();
        }
    }

    pub fn set_last_turn_state(&mut self, state: LastTurnState, cx: &mut Context<Self>) {
        self.last_turn_state = Some(state);
        if self.source == DiffSource::LastTurn {
            self.replace_files(Vec::new());
        } else {
            self.turn_files.clear();
        }
        cx.notify();
    }

    pub(super) fn capped(&self) -> bool {
        self.files.len() > 128
            || self
                .files
                .iter()
                .map(|file| file.added + file.removed)
                .sum::<usize>()
                > 9_000
            || self
                .files
                .iter()
                .map(|file| file.changed_bytes)
                .sum::<usize>()
                > 12_582_912
    }

    fn select_page(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(index) {
            self.active_file = index;
            file.open = true;
            self.selection = None;
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }
    }

    fn page_button(&self, previous: bool, cx: &Context<Self>) -> Stateful<Div> {
        let index = if previous {
            self.active_file.checked_sub(1)
        } else {
            (self.active_file + 1 < self.files.len()).then_some(self.active_file + 1)
        };
        let label = if previous {
            "Previous file"
        } else {
            "Next file"
        };
        div()
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .child(icon(
                if previous {
                    "diff-page-previous"
                } else {
                    "diff-page-next"
                },
                14.,
                cx.theme().text,
            ))
            .id(label)
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(index.is_none())
            .when(index.is_some(), |d| {
                d.focusable()
                    .cursor_pointer()
                    .hover(|d| d.bg(cx.theme().text_alpha_10))
            })
            .when(index.is_none(), |d| d.opacity(0.4))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(index) = index {
                    this.select_page(index, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    if let Some(index) = index {
                        this.select_page(index, cx);
                    }
                    cx.stop_propagation();
                }
            }))
    }

    fn refresh_button(&self, id: &'static str, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label("Refresh")
            .aria_disabled(self.pending)
            .focusable()
            .cursor_pointer()
            .rounded(px(14.))
            .px(px(12.))
            .py(px(4.))
            .bg(cx.theme().text_alpha_10)
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.pending {
                    cx.emit(DiffRequest::Refresh);
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if !this.pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.emit(DiffRequest::Refresh);
                    cx.stop_propagation();
                }
            }))
            .child(text("Refresh", 13., 20., 430., cx.theme().text))
    }

    pub(super) fn state_notices(
        &self,
        capped: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let omitted = self
            .repository
            .as_ref()
            .and_then(|repo| repo.untracked_files_omitted.as_ref())
            .filter(|_| matches!(self.source, DiffSource::Uncommitted | DiffSource::Unstaged));
        let watching = self.watch_limited && self.watch_root().is_some();
        if omitted.is_none() && !watching && !capped {
            return None;
        }
        let t = cx.theme();
        let wide = if self.standalone {
            window.viewport_size().width
        } else {
            panel_width(window, cx)
        } >= px(560.);
        Some(div().flex().flex_col().gap(px(8.)).p(px(8.))
            .when(!self.standalone, |d| d.border_b_1().border_color(t.border))
            .when(watching, |d| d.child(notice(
                "diff-watch-limited", text("Some files can't be watched for changes. Refresh to update this view.", 13., 18.571_428, 430., t.text_tertiary),
                self.refresh_button("diff-watch-refresh", cx).into_any_element(), Notice::Compact, wide, t,
            )))
            .when_some(omitted, |d, omitted| {
                let actions = div().flex().items_center().gap(px(8.))
                    .child(div().id("diff-copy-cleanup").role(gpui::Role::Button)
                        .aria_label("Copy cleanup command").focusable().cursor_pointer()
                        .px(px(12.)).py(px(4.)).rounded(px(14.)).bg(t.text_alpha_10)
                        .on_click(cx.listener(|_, _, _, cx| copy_cleanup(cx)))
                        .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                copy_cleanup(cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(text("Copy cleanup command", 13., 20., 430., t.text)))
                    .child(self.refresh_button("diff-omitted-refresh", cx));
                d.child(notice("diff-untracked-omitted",
                    div().flex().flex_col().gap(px(6.))
                        .child(text("Showing tracked changes only", 13., 18.571_428, 600., t.text))
                        .child(text(format!("The Changes tab skipped {} untracked files to stay responsive. If these files are generated, clean them up and refresh", grouped_count(omitted.count)), 13., 18.571_428, 430., t.text_tertiary)),
                    actions.into_any_element(), Notice::Warning, wide, t))
            })
            .when(capped, |d| d.child(notice(
                "diff-large-notice", text("This diff is large, showing one file at a time", 13., 21.125, 430., t.text),
                div().flex().gap(px(8.)).child(self.page_button(true, cx)).child(self.page_button(false, cx)).into_any_element(),
                Notice::Info, wide, t,
            ))))
    }
}

pub(super) fn grouped_count(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub(super) fn signed_count(count: usize, sign: char) -> String {
    let mut label = grouped_count(count);
    label.insert(0, sign);
    label
}

#[derive(Clone, Copy)]
enum Notice {
    Info,
    Compact,
    Warning,
}

fn copy_cleanup(cx: &mut Context<DiffPanel>) {
    cx.write_to_clipboard(ClipboardItem::new_string(
        "cd \"$(git rev-parse --show-toplevel)\" && git clean -d --interactive".into(),
    ));
    cx.emit(DiffRequest::Notice(
        "Copied interactive cleanup command to the clipboard",
    ));
}

fn notice(
    id: &'static str,
    body: Div,
    actions: gpui::AnyElement,
    style: Notice,
    wide: bool,
    t: &Theme,
) -> Stateful<Div> {
    let compact = matches!(style, Notice::Compact);
    let warning = matches!(style, Notice::Warning);
    div()
        .id(id)
        .role(gpui::Role::Status)
        .relative()
        .flex_none()
        .flex()
        .items_start()
        .when(wide, |d| d.items_center())
        .gap(px(12.))
        .pl(px(if compact { 12. } else { 20. }))
        .pr(px(if compact { 8. } else { 12. }))
        .py(px(if compact { 8. } else { 16. }))
        .child(shapes::decoration(Decoration {
            radius: Corners::all(px(25.)),
            corner: shapes::Corner::Squircle,
            fill: Some(if warning {
                Hsla::from(rgb(if t.appearance == Appearance::Dark {
                    0x281105
                } else {
                    0xfff5f0
                }))
                .opacity(0.15)
            } else {
                t.control_background.opacity(0.96)
            }),
            shadows: vec![
                shapes::Shadow {
                    color: t.border_strong,
                    offset: gpui::point(px(0.), px(0.)),
                    blur: px(0.),
                    spread: px(0.5),
                    inset: false,
                },
                shapes::Shadow {
                    color: gpui::black().opacity(0.05),
                    offset: gpui::point(px(0.), px(0.)),
                    blur: px(2.),
                    spread: px(0.),
                    inset: false,
                },
                shapes::Shadow {
                    color: gpui::black().opacity(0.02),
                    offset: gpui::point(px(0.), px(4.)),
                    blur: px(6.),
                    spread: px(0.),
                    inset: false,
                },
            ],
            ..Default::default()
        }))
        .when(!compact, |d| {
            d.child(
                icon(
                    if warning { "diff-warning" } else { "diff-info" },
                    18.,
                    if warning {
                        t.text_warning
                    } else {
                        t.text_accent
                    },
                )
                .mt(px(2.)),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .gap(px(if wide { 32. } else { 8. }))
                .when(wide, |d| d.items_center())
                .when(!wide, |d| d.flex_col())
                .child(body.flex_1().min_w_0())
                .child(actions),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_diff_caps_match_original_boundaries() {
        let mut panel = DiffPanel::from_patch("", "");
        panel.files = (0..128)
            .map(|i| FileDiff::patch(&format!("{i}.txt"), ""))
            .collect();
        assert!(!panel.capped());
        panel.files.push(FileDiff::patch("next.txt", ""));
        assert!(panel.capped());
        panel.files.truncate(1);
        panel.files[0].added = 9_000;
        assert!(!panel.capped());
        panel.files[0].added += 1;
        assert!(panel.capped());
    }

    #[test]
    fn only_separate_index_sources_allow_mutations() {
        for source in [DiffSource::Unstaged, DiffSource::Staged] {
            assert!(source.mutable());
        }
        for source in [
            DiffSource::Uncommitted,
            DiffSource::Branch,
            DiffSource::Commits,
            DiffSource::LastTurn,
            DiffSource::Cloud,
        ] {
            assert!(!source.mutable());
        }
    }

    #[test]
    fn line_totals_group_thousands() {
        for (count, expected) in [(0, "0"), (999, "999"), (1_000, "1,000"), (17_005, "17,005")] {
            assert_eq!(grouped_count(count), expected);
        }
        assert_eq!(signed_count(17_005, '+'), "+17,005");
        assert_eq!(signed_count(1_000, '-'), "-1,000");
    }
}
