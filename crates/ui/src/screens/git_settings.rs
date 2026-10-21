use super::*;

impl SettingsPane {
    pub(super) fn git(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        const FIELDS: &[(&str, &str, &str)] = &[
            ("host.git-branch-prefix", "Branch prefix", "codex/"),
            (
                "host.git-pr-watch-instructions",
                "Watch and fix pull requests",
                "For example: Comment /merge after checks pass and approve unrelated Chromatic changes…",
            ),
            (
                "host.git-commit-instructions",
                "Commit instructions",
                "Add commit message guidance…",
            ),
            (
                "host.git-pr-instructions",
                "Pull request instructions",
                "Add pull request guidance…",
            ),
        ];
        if self.git_fields.is_empty() {
            for &(key, title, placeholder) in FIELDS {
                let multiline = key != "host.git-branch-prefix";
                let value = self
                    .page
                    .read(cx)
                    .choice(key, if multiline { "" } else { "codex/" })
                    .to_owned();
                let editor = cx.new(|cx| {
                    Composer::settings_field(placeholder, &value, multiline, cx)
                        .field_label(if key == "host.git-pr-watch-instructions" {
                            "Pull request watch instructions"
                        } else {
                            title
                        })
                        .with_field_rows(if key == "host.git-pr-watch-instructions" {
                            5
                        } else if multiline {
                            6
                        } else {
                            1
                        })
                });
                let field = editor.clone();
                if !multiline {
                    cx.on_blur(&editor.focus_handle(cx), window, move |this, _, cx| {
                        this.save_git_field(key, &field, cx);
                    })
                    .detach();
                }
                if multiline {
                    let mut pending = None;
                    let mut previous = value.clone();
                    cx.observe(&editor, move |this, editor, cx| {
                        let text = editor.read(cx).field_text().to_owned();
                        if previous == text {
                            return;
                        }
                        previous = text.clone();
                        drop(pending.take());
                        this.page
                            .update(cx, |page, _| page.draft_instructions(key, text.clone()));
                        if this.page.read(cx).choice(key, "") != text {
                            pending = Some(cx.spawn(async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_secs(3))
                                    .await;
                                let _ = this.update(cx, |this, cx| {
                                    if editor.read(cx).field_text() == text {
                                        this.save_git_field(key, &editor, cx);
                                    }
                                });
                            }));
                        }
                        cx.notify();
                    })
                    .detach();
                }
                self.git_fields.push((key, editor));
            }
        }
        let branch = self.git_fields[0].1.clone();
        let mut groups = vec![(
            "",
            vec![
                settings_row(
                    "Branch prefix",
                    Some("Prefix used when ChatGPT creates new branches"),
                    div().w(px(224.)).child(branch).into_any_element(),
                    t,
                ),
                settings_row(
                    "Pull request merge method",
                    Some("Choose how ChatGPT merges pull requests"),
                    self.segmented(
                        "host.git-pull-request-merge-method",
                        "merge",
                        &[("merge", "Merge"), ("squash", "Squash")],
                        t,
                        cx,
                    ),
                    t,
                ),
                settings_row(
                    "Always force push",
                    Some("Use --force-with-lease when pushing from ChatGPT"),
                    self.toggle("host.git-always-force-push", false, t, window, cx),
                    t,
                ),
                settings_row(
                    "Create draft pull requests",
                    Some("Use draft pull requests by default when creating PRs from ChatGPT"),
                    self.toggle("host.git-create-pull-request-as-draft", true, t, window, cx),
                    t,
                ),
                settings_row(
                    "Review delivery",
                    Some(
                        "Start /review in the current chat when possible or launch a separate review chat",
                    ),
                    self.segmented(
                        "desktop.reviewDelivery",
                        "inline",
                        &[("inline", "Inline"), ("detached", "Detached")],
                        t,
                        cx,
                    ),
                    t,
                ),
            ],
        )];
        for (index, &(key, title, _)) in FIELDS.iter().enumerate().skip(1) {
            let editor = self.git_fields[index].1.clone();
            let row = div().child(editor);
            let mut rows = Vec::new();
            if key == "host.git-pr-watch-instructions" {
                rows.push(settings_row(
                    "Auto-merge when ready",
                    Some("Continue watching until the pull request is merged"),
                    self.toggle("host.git-pr-watch-auto-merge", false, t, window, cx),
                    t,
                ));
            }
            rows.push(row);
            groups.push((title, rows));
        }
        groups
    }

    fn save_git_field(
        &mut self,
        key: &'static str,
        editor: &Entity<Composer>,
        cx: &mut Context<Self>,
    ) {
        let value = editor.read(cx).field_text().to_owned();
        let default = if key == "host.git-branch-prefix" {
            "codex/"
        } else {
            ""
        };
        if self.page.read(cx).choice(key, default) != value {
            self.page
                .update(cx, |page, cx| page.set(key, json!(value), cx));
            self.git_saved_until.insert(
                key,
                std::time::Instant::now() + std::time::Duration::from_secs(3),
            );
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(3))
                    .await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
    }

    pub(super) fn render_git(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let groups = self.git(t, window, cx);
        let mut content = Vec::new();
        for (index, (title, mut rows)) in groups.into_iter().enumerate() {
            if index == 0 {
                content.push(super::worktrees::card(rows, Some(&self.layer), t));
                continue;
            }
            let (key, editor) = self.git_fields[index].clone();
            let dirty = self.page.read(cx).choice(key, "") != editor.read(cx).field_text();
            let recently_saved = self
                .git_saved_until
                .get(key)
                .is_some_and(|until| *until > std::time::Instant::now());
            let is_watch = key == "host.git-pr-watch-instructions";
            let mut heading = div()
                .pb(px(if is_watch { 6. } else { 12. }))
                .flex()
                .when(is_watch, |heading| heading.min_h(px(46.)).items_center())
                .when(!is_watch, |heading| heading.items_start())
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(
                            title,
                            if is_watch { 14. } else { 16. },
                            if is_watch { 21. } else { 24.888_89 },
                            500.,
                            t.text,
                        ))
                        .children(
                            match key {
                                "host.git-commit-instructions" => {
                                    Some("Added to commit message generation prompts")
                                }
                                "host.git-pr-instructions" => {
                                    Some("Added to PR title/description generation prompts")
                                }
                                _ => None,
                            }
                            .map(|subtitle| text(subtitle, 13., 18., 400., t.text_secondary)),
                        ),
                );
            if dirty {
                heading = heading.child(button("Save", t.text_alpha_5, None, t).id(key).on_click(
                    cx.listener(move |this, _, _, cx| this.save_git_field(key, &editor, cx)),
                ));
            } else if recently_saved {
                heading = heading.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(icon("check", 12., t.text_secondary))
                        .child(text("Saved", 12., 16., 430., t.text_secondary)),
                );
            }
            let editor = rows.pop();
            content.push(
                div().flex().flex_col().child(heading).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .when(!rows.is_empty(), |group| {
                            group.child(super::worktrees::card(rows, Some(&self.layer), t))
                        })
                        .children(editor),
                ),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.platform && event.keystroke.key == "s" {
                    let fields = this.git_fields.clone();
                    for (key, editor) in fields {
                        this.save_git_field(key, &editor, cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div().pt(px(16.)).pb(px(1.)).child(
                            div().w_full().max_w(px(768.)).mx_auto().px(px(20.)).child(
                                div()
                                    .min_h(px(60.))
                                    .py(px(12.))
                                    .flex()
                                    .flex_col()
                                    .justify_center()
                                    .child(text("Git", 28., 33.6, 500., t.text)),
                            ),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(40.))
                            .children(content),
                    ),
            )
            .into_any_element()
    }
}
