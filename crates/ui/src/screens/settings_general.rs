use super::*;

impl SettingsPane {
    pub(super) fn general(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<SettingsRow>)> {
        let page = self.page.read(cx);
        // The `w-48` path span middle-truncates to the width it has (`zLi`).
        let mono = Font {
            weight: FontWeight(430.),
            ..font(MONO)
        };
        let root_path = page.choice("desktop.projectlessWorkspaceRoot", &page.projectless_root);
        let custom_root = !page.get("desktop.projectlessWorkspaceRoot").is_null();
        let pending = page.folder_pending;
        let loading = page.root_loading && !custom_root;
        let root = if loading {
            "Loading folder…".to_owned()
        } else {
            middle_truncate(root_path, |s| {
                let run = TextRun {
                    len: s.len(),
                    font: mono.clone(),
                    color: t.text_secondary,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line =
                    window
                        .text_system()
                        .shape_line(s.to_string().into(), px(12.), &[run], None);
                line.width <= px(192.)
            })
        };
        let path = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div().w(px(192.)).overflow_hidden().child(
                    text(root, 12., 16., 430., t.text_secondary)
                        .when(!loading, |d| d.font_family(MONO)),
                ),
            )
            .child(
                div()
                    .relative()
                    .child(
                        button("Change", t.text_alpha_5, None, t).opacity(if pending {
                            0.
                        } else {
                            1.
                        }),
                    )
                    .when(pending, |d| {
                        d.child(on_card(12.5, Squircle, t.text_alpha_5, None))
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon("sidebar-cmd-spinner", 14., t.text_secondary)),
                            )
                    })
                    .id("projectless-change")
                    .role(gpui::Role::Button)
                    .aria_label("Change projectless task folder")
                    .aria_disabled(pending)
                    .focusable()
                    .tab_index(0)
                    .rounded(px(12.5))
                    .focus_visible(crate::focus::trigger_focus)
                    .on_click(cx.listener(|this, _, _, cx| this.choose_projectless_folder(cx))),
            )
            .when(custom_root, |d| {
                d.child(
                    button("Reset", t.text_alpha_5, None, t)
                        .id("projectless-reset")
                        .role(gpui::Role::Button)
                        .focusable()
                        .tab_index(0)
                        .focus_visible(crate::focus::trigger_focus)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.page.update(cx, |page, cx| {
                                page.set("desktop.projectlessWorkspaceRoot", Value::Null, cx)
                            });
                        })),
                )
            })
            .into_any_element();
        let hotkey = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(text("Off", 13., 18.571_428, 430., t.text_secondary))
            .child(
                div()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("pencil", 16., t.text_tertiary)),
            )
            .into_any_element();
        let row = |title, description, control| settings_row(title, description, control, t);
        vec![
            (
                "Permissions",
                vec![
                    row(
                        "Default permissions",
                        Some(
                            "By default, ChatGPT can read and edit files in its workspace. It can ask for additional access when needed",
                        ),
                        // Default permissions are always shown.
                        crate::tooltip::tooltip(
                            switch("default-permissions", true, true, t),
                            "Default permissions are always shown",
                            None,
                            crate::tooltip::Side::Top,
                        ).into_any_element(),
                    ),
                    row(
                        "Full access",
                        Some(
                            "When ChatGPT runs with full access, it can edit any file on your computer and run commands with network, without your approval. This significantly increases the risk of data loss, leaks, or unexpected behavior. <a>Learn more</a> about elevated risks.",
                        ),
                        self.full_access_control(t, window, cx),
                    ),
                ],
            ),
            (
                "General",
                {
                    let mut rows = vec![
                    row(
                        "Projectless task folder",
                        Some(
                            "The location where tasks started outside of projects store their data by default.",
                        ),
                        path,
                    ),
                    row(
                        "Default file open destination",
                        Some("Where files and folders open by default"),
                        self.open_target_control(t, cx),
                    ),
                    row(
                        "Language",
                        Some("Language for the app UI"),
                        self.language_control(t, cx),
                    ),
                    row(
                        "Show in menu bar",
                        Some("Keep ChatGPT in the macOS menu bar when the main window is closed"),
                        self.toggle("host.mac-menu-bar-enabled", true, t, window, cx),
                    ),
                    row(
                        "Confirm before closing a window",
                        Some("Warn when a tab-close shortcut would close the window"),
                        self.select_menu("host.confirm-before-close", CLOSE_OPTIONS, t, cx),
                    ),
                    row(
                        "Full view by default",
                        Some("Start new tasks with Chat and content in a single tab strip"),
                        self.toggle("host.default-workspace-full-view", false, t, window, cx),
                    ),
                    row(
                        "Bottom panel",
                        Some("Show the bottom panel control in the app header"),
                        self.toggle("session.bottomPanel", true, t, window, cx),
                    ),
                    row(
                        "Default terminal location",
                        Some(
                            "Choose where the terminal shortcut and environment actions open terminal tabs",
                        ),
                        self.segmented(
                            "desktop.defaultTerminalLocation",
                            "bottom",
                            &[("bottom", "Bottom"), ("right", "Right")],
                            t,
                            cx,
                        ),
                    ),
                    row(
                        "Prevent sleep while running",
                        Some("Keep your computer awake while ChatGPT is running a task"),
                        self.toggle("desktop.preventSleepWhileRunning", false, t, window, cx),
                    ),
                    row(
                        "Compress local chat history",
                        Some(if self.page.read(cx).failed.contains("features.local_thread_store_compression") {
                            "Couldn’t save the compression setting. Try again."
                        } else {
                            "Save disk space by compressing older local chat history. Restart ChatGPT to apply changes."
                        }),
                        self.toggle(
                            "features.local_thread_store_compression",
                            false,
                            t,
                            window,
                            cx,
                        ),
                    )];
                    if let Some(control) = self.speed_control(t, cx) {
                        rows.push(row(
                            "Speed",
                            Some("Choose how quickly ChatGPT runs across chats, subagents, and compaction"),
                            control,
                        ));
                    }
                    rows.extend([
                    row(
                        "Suggested prompts",
                        Some(
                            "Suggest what to do next by searching project files and connected apps",
                        ),
                        self.toggle("host.ambient-suggestions-enabled", true, t, window, cx),
                    ),
                    row(
                        "Open source licenses",
                        Some("Third-party notices for bundled dependencies"),
                        button("View", t.text_alpha_5, None, t)
                            .id("open-source-licenses")
                            .role(gpui::Role::Button)
                            .focusable()
                            .tab_index(0)
                            .focus_visible(crate::focus::trigger_focus)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.page.update(cx, |page, cx| page.request_navigation("/settings/open-source-licenses".into(), window, cx));
                            })).into_any_element(),
                    ),
                    row(
                        "Plugins",
                        Some("Allow ChatGPT to use installed plugins"),
                        self.toggle("features.plugins", true, t, window, cx),
                    ),
                    ]);
                    rows
                },
            ),
            (
                "Composer",
                vec![
                    row(
                        "Show predictions",
                        Some("Predict your next message based on the conversation"),
                        self.toggle("session.messagePredictions", true, t, window, cx),
                    ),
                    row(
                        "Plain text composer",
                        Some(
                            "Keep code, Markdown, and links as literal text while writing messages",
                        ),
                        self.toggle("desktop.composerPlainTextMode", false, t, window, cx),
                    ),
                    row(
                        "Show context window usage",
                        None,
                        self.toggle("host.show-context-window-usage", false, t, window, cx),
                    ),
                    row(
                        "Send shortcut",
                        Some("Choose when Enter sends a prompt or inserts a new line"),
                        self.select_menu("desktop.composerEnterBehavior", SEND_OPTIONS, t, cx),
                    ),
                    row(
                        "Follow-up behavior",
                        Some(
                            "Queue follow-ups while ChatGPT runs or steer the current run. Press Ctrl+⏎ to do the opposite for one message",
                        ),
                        self.segmented(
                            "desktop.followUpQueueMode",
                            "steer",
                            &[("queue", "Queue"), ("steer", "Steer")],
                            t,
                            cx,
                        ),
                    ),
                ],
            ),
            (
                "Popout Window",
                vec![
                    row(
                        "Popout Window hotkey",
                        Some(
                            "Set a global shortcut for Popout Window. Leave unset to keep it off.",
                        ),
                        hotkey,
                    ),
                    row(
                        "Default to standalone chat",
                        Some("Start new chats outside of any project"),
                        self.toggle(
                            "host.hotkey-window-projectless-default-enabled",
                            false,
                            t,
                            window,
                            cx,
                        ),
                    ),
                ],
            ),
            (
                "Notifications",
                vec![
                    row(
                        "Notifications are off",
                        Some("Turn on notifications in System Settings to use the options below"),
                        button(
                            "Open System Settings",
                            t.primary_soft_alpha,
                            Some(t.border),
                            t,
                        )
                        .id("notification-system-settings")
                        .role(gpui::Role::Button)
                        .focusable()
                        .tab_index(0)
                        .focus_visible(crate::focus::trigger_focus)
                        .on_click(|_, _, cx| cx.open_url("x-apple.systempreferences:com.apple.Notifications-Settings.extension"))
                        .into_any_element(),
                    ),
                    row(
                        "Turn completion notifications",
                        Some("Set when ChatGPT alerts you that it's finished"),
                        select("Only when unfocused", None, true, false, t).into_any_element(),
                    ),
                    row(
                        "Enable permission notifications",
                        Some("Show alerts when notification permissions are required"),
                        switch("permissions-notifications", true, true, t).into_any_element(),
                    ),
                    row(
                        "Enable question notifications",
                        Some("Show alerts when input is needed to continue"),
                        switch("question-notifications", true, true, t).into_any_element(),
                    ),
                    row(
                        "Enable dot notifications",
                        Some("Show alerts when your dot sends you a message"),
                        switch("dot-notifications", true, true, t).into_any_element(),
                    ),
                    row(
                        "Notification sound",
                        Some("Sound for task completion, permission requests, and questions"),
                        select("Default", None, true, false, t).into_any_element(),
                    ),
                ],
            ),
            (
                "Toys",
                {
                    let mut rows = vec![row(
                        "Confetti cannon",
                        Some("Let Codex fire confetti in the app when you ask!"),
                        self.toggle("host.toys-confetti-enabled", false, t, window, cx),
                    )];
                    if self.page.read(cx).audio_available {
                        rows.push(row(
                            "Audio visualizer",
                            Some("Animate conversation rails with system audio. Audio is processed on your device, never saved or uploaded."),
                            self.toggle("host.toys-audio-visualizer-enabled", false, t, window, cx),
                        ));
                    }
                    rows
                },
            ),
        ]
    }

    fn choose_projectless_folder(&mut self, cx: &mut Context<Self>) {
        if self.page.read(cx).folder_pending {
            return;
        }
        self.page.update(cx, |page, cx| {
            page.folder_pending = true;
            cx.notify();
        });
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        let page = self.page.clone();
        cx.spawn(async move |_, cx| {
            let result = paths.await;
            let _ = page.update(cx, |page, cx| {
                page.folder_pending = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            page.set(
                                "desktop.projectlessWorkspaceRoot",
                                json!(path.to_string_lossy()),
                                cx,
                            );
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => page.notify_error("Unable to change the projectless task folder", cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn full_access_control(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = "host.composer-permission-mode-visibility";
        let on = self.page.read(cx).flag(key, true);
        let focus = focus::handle(key, 0, window, cx);
        let handle = focus.read(cx).handle.clone();
        focus::ring(switch(key, on, false, t).aria_label("Show Full access in the composer").relative(), &focus, Theme::RADIUS_FULL, px(2.), cx)
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                handle.focus(window, cx);
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.page.update(cx, |page, cx| {
                    if on { page.set(key, json!(false), cx); return; }
                    let modal = cx.new(|cx| crate::confirmation::Confirmation::new(
                        "Make Full Access available?",
                        "When selected, ChatGPT can access the internet and read and edit files without asking for approval — including potentially destructive commands\n\nTurning this on adds Full Access to the permissions menu. It does not turn Full Access on.",
                        "Confirm", window, cx,
                    ).warning().link("Learn more", "https://developers.openai.com/codex/config-basic", cx));
                    cx.subscribe(&modal, move |page, _, outcome: &crate::confirmation::Outcome, cx| {
                        if *outcome == crate::confirmation::Outcome::Confirm { page.set(key, json!(true), cx); }
                        page.modal = None;
                        cx.notify();
                    }).detach();
                    page.modal = Some(modal);
                    cx.notify();
                });
            })).into_any_element()
    }

    fn open_target_control(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.selects.contains_key("host.open-in-target-preferences") {
            return self.choose("host.open-in-target-preferences", Vec::new(), false, cx);
        }
        let options = match &self.page.read(cx).targets {
            crate::screens::computer_use::Load::Ready(value) => value["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|target| target["available"] != false)
                .filter_map(|target| {
                    Some((
                        target["id"].as_str()?.to_owned(),
                        target["label"].as_str()?.to_owned(),
                    ))
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        if options.is_empty() {
            return select("No targets found", None, true, false, t).into_any_element();
        }
        self.choose("host.open-in-target-preferences", options, false, cx)
    }

    fn language_control(&mut self, _t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut options = Vec::new();
        if !self.selects.contains_key("desktop.localeOverride") {
            options.push((String::new(), "Auto detect".to_owned()));
            options.extend(
                LANGUAGES
                    .iter()
                    .map(|&(code, native, _)| (code.to_owned(), native.to_owned())),
            );
        }
        let result = self.choose("desktop.localeOverride", options, true, cx);
        if let Some(select) = self.selects.get("desktop.localeOverride") {
            select.update(cx, |select, _| {
                select.search_aliases(|| {
                    LANGUAGES
                        .iter()
                        .map(|&(code, _, english)| (code.to_owned(), english.to_owned()))
                        .collect()
                })
            });
        }
        result
    }

    fn speed_control(&mut self, _t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let crate::screens::computer_use::Load::Ready((response, true)) =
            &self.page.read(cx).requirements
        else {
            return None;
        };
        let requirements = &response["requirements"]["featureRequirements"];
        let fast = requirements["fast_mode"] != false;
        let ultrafast = (response["supportsIndependentSpeedModes"] == true || fast)
            && requirements["ultrafast_mode"] != false;
        if !fast && !ultrafast {
            return None;
        }
        let mut options = Vec::new();
        if !self.selects.contains_key("service_tier") {
            options.push((String::new(), "Standard".to_owned()));
            if fast {
                options.push(("priority".into(), "Fast".into()));
            }
            if ultrafast {
                options.push(("ultrafast".into(), "Ultrafast".into()));
            }
        }
        Some(self.choose("service_tier", options, false, cx))
    }

    pub(super) fn licenses(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::screens::computer_use::Load;
        let body = match &self.page.read(cx).notices {
            Load::Loading => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(8.))
                .child(icon("sidebar-cmd-spinner", 16., t.text_secondary))
                .child(text("Loading…", 13., 18.571_428, 430., t.text_secondary))
                .into_any_element(),
            Load::Ready(body) if !body.is_empty() => div()
                .rounded(px(4.))
                .p(px(12.))
                .child(
                    text(body.clone(), 12., 19.5, 430., t.text)
                        .font_family(MONO)
                        .whitespace_normal(),
                )
                .into_any_element(),
            _ => div()
                .min_h(px(64.))
                .px(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    "No third-party notices were found.",
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ))
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(46.))
                    .flex_none()
                    .px(px(20.))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .id("licenses-back")
                            .h(px(28.))
                            .px(px(9.))
                            .rounded(px(12.5))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .role(gpui::Role::Button)
                            .focusable()
                            .tab_index(0)
                            .focus_visible(crate::focus::trigger_focus)
                            .hover(|style| style.bg(t.ghost_hover))
                            .child(icon("arrow-left", 16., t.text_tertiary))
                            .child(text("Back", 13., 18., 430., t.text_tertiary))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.page.update(cx, |page, cx| {
                                    page.request_navigation(
                                        "/settings/general-settings".into(),
                                        window,
                                        cx,
                                    );
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .id("licenses-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div().pt(px(16.)).pb(px(1.)).child(
                            div()
                                .w_full()
                                .max_w(px(768.))
                                .mx_auto()
                                .px(px(20.))
                                .py(px(12.))
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(text("Open source licenses", 28., 33.6, 500., t.text))
                                .child(text(
                                    "Third-party notices for dependencies included in this app",
                                    14.,
                                    24.,
                                    430.,
                                    t.text_secondary,
                                )),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .p(px(20.))
                            .child(body),
                    ),
            )
            .into_any_element()
    }
}

pub(super) fn present_select(
    key: &str,
    select: crate::settings_kit::Select,
) -> crate::settings_kit::Select {
    match key {
        "host.open-in-target-preferences" => select.with_icons(target_icon),
        "desktop.localeOverride" => select.keep_default_visible(),
        "service_tier" => select.with_icons(speed_icon).with_details(&[
            ("", "Default speed"),
            ("priority", "faster speed, increased usage"),
            (
                "ultrafast",
                "The fastest available responses for latency-sensitive work",
            ),
        ]),
        "desktop.composerEnterBehavior" => select.with_width(220.),
        _ => select,
    }
}

fn speed_icon(value: &str, t: &Theme) -> Option<AnyElement> {
    match value {
        "priority" => Some(icon("composer-bolt", 12., t.text).into_any_element()),
        "ultrafast" => Some(icon("settings-ultrafast", 12., t.text).into_any_element()),
        _ => None,
    }
}

fn target_icon(value: &str, t: &Theme) -> Option<AnyElement> {
    if value == "zed" {
        return Some(app_icon(over(t.primary_soft_alpha, t.background_panel)));
    }
    let path = match value {
        "vscode" => "apps/vscode.png",
        "vscodeInsiders" => "apps/vscode-insiders.png",
        "cursor" => "apps/cursor.png",
        "devin" => "apps/devin.svg",
        "bbedit" => "apps/bbedit.png",
        "emacs" => "apps/emacs.png",
        "sublimeText" => "apps/sublime-text.png",
        "windsurf" => "apps/windsurf.png",
        "antigravity" => "apps/antigravity.png",
        "systemDefault" => "apps/file-explorer.png",
        "fileManager" => "apps/finder.png",
        "terminal" => "apps/terminal.png",
        "iterm2" => "apps/iterm2.png",
        "kitty" => "apps/kitty.png",
        "ghostty" => "apps/ghostty.png",
        "warp" => "apps/warp.png",
        "xcode" => "apps/xcode.png",
        "androidStudio" => "apps/android-studio.png",
        "intellij" => "apps/intellij.png",
        "rider" => "apps/rider.png",
        "goland" => "apps/goland.png",
        "rustrover" => "apps/rustrover.png",
        "pycharm" => "apps/pycharm.png",
        "webstorm" => "apps/webstorm.svg",
        "phpstorm" => "apps/phpstorm.png",
        "textmate" => "apps/textmate.png",
        _ => return None,
    };
    Some(img(path).size(px(18.)).flex_none().into_any_element())
}

pub(super) const LANGUAGES: &[(&str, &str, &str)] = &[
    ("sq-AL", "Albanian", "Albanian"),
    ("hy-AM", "Armenian", "Armenian"),
    ("ms-MY", "Bahasa Melayu", "Malay"),
    ("bs-BA", "bosanski", "Bosnian"),
    ("my-MM", "Burmese", "Burmese"),
    ("ca-ES", "català", "Catalan"),
    ("cs-CZ", "čeština", "Czech"),
    ("da-DK", "dansk", "Danish"),
    ("de-DE", "Deutsch", "German"),
    ("et-EE", "eesti", "Estonian"),
    ("en-US", "English", "English"),
    ("es-ES", "español (España)", "Spanish (Spain)"),
    (
        "es-419",
        "español (Latinoamérica)",
        "Spanish (Latin America)",
    ),
    ("tl", "Filipino", "Filipino"),
    ("fr-CA", "français (Canada)", "French (Canada)"),
    ("fr-FR", "français (France)", "French (France)"),
    ("ka-GE", "Georgian", "Georgian"),
    ("hr-HR", "hrvatski", "Croatian"),
    ("is-IS", "Icelandic", "Icelandic"),
    ("id-ID", "Indonesia", "Indonesian"),
    ("it-IT", "italiano", "Italian"),
    ("sw-TZ", "Kiswahili", "Swahili"),
    ("lv-LV", "latviešu", "Latvian"),
    ("lt", "lietuvių", "Lithuanian"),
    ("mk-MK", "Macedonian", "Macedonian"),
    ("hu-HU", "magyar", "Hungarian"),
    ("mn", "Mongolian", "Mongolian"),
    ("nl-NL", "Nederlands", "Dutch"),
    ("nb-NO", "norsk bokmål", "Norwegian Bokmål"),
    ("pl-PL", "polski", "Polish"),
    ("pt-BR", "português (Brasil)", "Portuguese (Brazil)"),
    ("pt-PT", "português (Portugal)", "Portuguese (Portugal)"),
    ("ro-RO", "română", "Romanian"),
    ("sk-SK", "slovenčina", "Slovak"),
    ("sl-SI", "slovenščina", "Slovenian"),
    ("so-SO", "Somali", "Somali"),
    ("fi-FI", "suomi", "Finnish"),
    ("sv-SE", "svenska", "Swedish"),
    ("vi-VN", "Tiếng Việt", "Vietnamese"),
    ("tr-TR", "Türkçe", "Turkish"),
    ("el-GR", "Ελληνικά", "Greek"),
    ("bg-BG", "български", "Bulgarian"),
    ("kk", "қазақ тілі", "Kazakh"),
    ("ru-RU", "русский", "Russian"),
    ("sr-RS", "српски", "Serbian"),
    ("uk-UA", "українська", "Ukrainian"),
    ("ur", "اردو", "Urdu"),
    ("ar", "العربية", "Arabic"),
    ("fa", "فارسی", "Persian"),
    ("am", "አማርኛ", "Amharic"),
    ("mr-IN", "मराठी", "Marathi"),
    ("hi-IN", "हिन्दी", "Hindi"),
    ("bn-BD", "বাংলা", "Bangla"),
    ("pa", "ਪੰਜਾਬੀ", "Punjabi"),
    ("gu-IN", "ગુજરાતી", "Gujarati"),
    ("ta-IN", "தமிழ்", "Tamil"),
    ("te-IN", "తెలుగు", "Telugu"),
    ("kn-IN", "ಕನ್ನಡ", "Kannada"),
    ("ml", "മലയാളം", "Malayalam"),
    ("th-TH", "ไทย", "Thai"),
    ("ko-KR", "한국어", "Korean"),
    ("ja-JP", "日本語", "Japanese"),
    ("zh-CN", "简体中文", "Chinese (Simplified)"),
    ("zh-TW", "繁體中文（台灣）", "Chinese (Traditional, Taiwan)"),
    (
        "zh-HK",
        "繁體中文（香港）",
        "Chinese (Traditional, Hong Kong)",
    ),
];
