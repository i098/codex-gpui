use super::*;

const SPINNER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/icons/sidebar-cmd-spinner.svg"
));

fn paint_spinner(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window, cx: &App) {
    if !shapes::paint_icon(
        SPINNER,
        "composer-loading-spinner",
        bounds,
        false,
        color,
        Some(bounds.origin),
        window,
        cx,
    ) {
        window
            .paint_svg(
                bounds,
                "composer-loading-spinner".into(),
                Some(SPINNER),
                gpui::TransformationMatrix::default(),
                color,
                cx,
            )
            .ok();
    }
}

fn loading_send(background: Hsla, foreground: Hsla) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            shapes::with_composited_opacity(127, bounds, window, cx, |window| {
                shapes::paint(
                    &deco(9999., background, vec![]),
                    Some(bounds.origin),
                    bounds,
                    window,
                    cx,
                );
                let icon_bounds = Bounds::new(
                    bounds.center() - point(px(8.), px(8.)),
                    size(px(16.), px(16.)),
                );
                paint_spinner(icon_bounds, foreground, window, cx);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn loading_model(color: Hsla) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            shapes::with_composited_opacity(102, bounds, window, cx, |window| {
                paint_spinner(bounds, color, window, cx);
            });
        },
    )
    .size(px(12.))
    .flex_none()
}

fn effort_icon(effort: &str) -> &'static str {
    match effort {
        "none" | "minimal" => "composer-model-none",
        "low" => "composer-model",
        "high" => "composer-model-high",
        "xhigh" | "max" | "ultra" | "persistent" => "composer-model-extra-high",
        _ => "composer-model-medium",
    }
}

impl Composer {
    pub(super) fn recovery_card(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let recovery = self
            .dictation_recovery
            .as_ref()
            .expect("dictation recovery");
        let recovering = recovery.retrying;
        let recovered = recovery.text.is_some();
        let mut row = div()
            .id("dictation-recovery")
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_h(px(42.))
            .px(px(12.))
            .role(gpui::Role::Status)
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(16.)),
                corner: shapes::Corner::Squircle,
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(div().flex_1().min_w_0().child(text(
                if recovering {
                    "Recovering text…"
                } else if recovered {
                    "Text recovered"
                } else {
                    "Dictation stopped"
                },
                13.,
                18.,
                400.,
                t.text,
            )));
        for (action, label) in [
            (0, "View recording"),
            (1, if recovered { "Add to message" } else { "Retry" }),
            (2, "Dismiss"),
        ] {
            if (action == 0 && recovery.active.recording_url.is_none())
                || (action == 1 && recovering)
            {
                continue;
            }
            row = row.child(self.recovery_button(action, label, t, cx));
        }
        row
    }

    fn recovery_button(
        &self,
        action: usize,
        label: &'static str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let recovered = self
            .dictation_recovery
            .as_ref()
            .is_some_and(|recovery| recovery.text.is_some());
        let mut button = pill(label, t)
            .h(px(24.))
            .px(px(8.))
            .role(gpui::Role::Button)
            .aria_label(label)
            .tab_index(0)
            .focus_visible(menu::trigger_focus);
        button = if action == 2 {
            button.child(icon("close", 12., t.text_tertiary))
        } else {
            button.child(text(
                label,
                12.,
                16.,
                400.,
                if action == 0 { t.text_tertiary } else { t.text },
            ))
        };
        if action == 1 {
            button = button.bg(t.surface_elevated);
        }
        button
            .on_click(cx.listener(move |this, _, window, cx| {
                this.recovery_action(action, recovered, window, cx)
            }))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.recovery_action(action, recovered, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
    }

    fn recovery_action(
        &mut self,
        action: usize,
        recovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            0 => self.view_recording(cx),
            1 if recovered => self.insert_recovered_dictation(window, cx),
            1 => self.retry_dictation(cx),
            _ => {
                self.dictation_recovery = None;
                self.dictation_error = None;
                cx.notify();
            }
        }
    }

    fn recording_waveform(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let color = t.text;
        gpui::canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                let Some(active) = &entity.read(cx).dictation else {
                    return;
                };
                let count = (f32::from(bounds.size.width) / 6.).floor() as usize + 1;
                let available = active.levels.len().min(count);
                for index in 0..count {
                    let sample = index.checked_sub(count - available).and_then(|offset| {
                        active.levels.get(active.levels.len() - available + offset)
                    });
                    let height =
                        3. + (f32::from(bounds.size.height) - 3.) * sample.copied().unwrap_or(0.);
                    let rect = Bounds::new(
                        point(
                            bounds.origin.x + bounds.size.width
                                - px((count - index - 1) as f32 * 6.),
                            bounds.center().y - px(height / 2.),
                        ),
                        size(px(3.), px(height)),
                    );
                    shapes::paint(
                        &deco(
                            1.5,
                            color.opacity(if sample.is_some() { 0.5 } else { 0.2 }),
                            vec![],
                        ),
                        Some(rect.origin),
                        rect,
                        window,
                        cx,
                    );
                }
            },
        )
        .w_full()
        .h(px(28.))
    }

    fn recording_footer(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let active = self.dictation.as_ref().expect("active dictation");
        let transcribing = active.transcribing;
        let middle = div()
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .child(if transcribing {
                text("Transcribing", 14., 20., 400., t.text_tertiary).into_any_element()
            } else {
                self.recording_waveform(t, cx).into_any_element()
            });
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .pb(px(8.))
            .child(self.recording_button(0, t, cx))
            .child(middle)
            .child(self.recording_button(1, t, cx))
            .child(self.recording_button(2, t, cx))
    }

    fn recording_action(&mut self, action: usize, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            0 => self.cancel_dictation(cx),
            1 => self.finish_dictation(window, cx),
            _ => self.transcribe_and_send(window, cx),
        }
    }

    fn recording_button(&self, action: usize, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.dictation.as_ref().expect("active dictation");
        let transcribing = active.transcribing;
        let label = match action {
            0 if transcribing => "Cancel transcription",
            0 => "Cancel dictation",
            1 => "Stop dictation",
            _ => "Transcribe and send",
        };
        let disabled = action != 0 && transcribing;
        let foreground = if action == 2 {
            t.text_composer_primary
        } else {
            t.text
        };
        let background = if action == 2 {
            t.composer_primary
        } else {
            t.surface_elevated
        };
        pill(label, t)
            .size(px(28.))
            .justify_center()
            .relative()
            .child(shapes::decoration(deco(9999., background, vec![])))
            .child(self.recording_button_icon(action, foreground))
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(disabled)
            .tab_index(0)
            .focus_visible(menu::trigger_focus)
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    this.recording_action(action, window, cx);
                }
            }))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.recording_action(action, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
    }

    fn recording_button_icon(&self, action: usize, foreground: Hsla) -> AnyElement {
        let active = self.dictation.as_ref().expect("active dictation");
        let loading = active.transcribing
            && match action {
                1 => !active.submit_when_finished,
                2 => active.submit_when_finished,
                _ => false,
            };
        if loading {
            loading_model(foreground).into_any_element()
        } else if action == 1 {
            div()
                .size(px(8.8))
                .rounded(px(1.))
                .bg(foreground)
                .into_any_element()
        } else {
            icon(
                if action == 0 {
                    "close"
                } else {
                    "composer-arrow-up"
                },
                16.,
                foreground,
            )
            .into_any_element()
        }
    }

    pub(super) fn dictation_button(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = self.dictation_label();
        pill("dictate", t)
            .justify_center()
            .child(if self.dictation.is_some() {
                div()
                    .size(px(8.8))
                    .rounded(px(1.))
                    .bg(t.text)
                    .into_any_element()
            } else {
                icon("mic", 16., t.text)
                    .when(self.docked, |i| i.own_layer(None))
                    .into_any_element()
            })
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(label == "Transcribing")
            .tab_index(0)
            .focus_visible(menu::trigger_focus)
            .size(px(28.))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.dictate(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.dictate(window, cx)))
    }

    pub(super) fn footer(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Div {
        if self
            .dictation
            .as_ref()
            .is_some_and(|active| !active.starting)
        {
            return self.recording_footer(t, cx);
        }
        let open = matches!(self.menu, Some(Menu::Effort | Menu::Models));
        let model = self.current_model();
        let loading = model.is_none();
        let model_label = model.map_or("Loading model…".into(), |m| m.label.clone());
        let labels = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                tabular(text(model_label, 13., 18., 400., t.text))
                    .when(self.docked, |d| d.top_layer())
                    .when(open, |d| d.invisible()),
            )
            .when(model.is_some(), |d| {
                d.child(
                    label(
                        effort_label(self.displayed_effort()).to_string(),
                        t.text_tertiary,
                    )
                    .when(open, |d| d.invisible()),
                )
            })
            .when(open, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .justify_center()
                        .child(text("Select effort", 13., 18., 400., t.text_tertiary)),
                )
            });
        let trigger = if loading {
            let value = "Loading model…";
            let width = window
                .text_system()
                .layout_line(
                    value,
                    px(13.),
                    &[TextRun {
                        len: value.len(),
                        font: gpui::font(crate::SANS),
                        color: t.text_tertiary,
                        ..Default::default()
                    }],
                    None,
                )
                .width;
            pill("model-picker", t)
                .aria_label(value)
                .aria_disabled(true)
                .px(px(8.))
                .gap(px(4.))
                .child(loading_model(t.text_tertiary))
                .child(label(value, t.text_tertiary.opacity(0.4)).w(width))
        } else if self.footer_width <= XS_WIDTH {
            pill("model-picker", t)
                .w(px(28.))
                .justify_center()
                .child(icon(
                    effort_icon(self.displayed_effort()),
                    16.,
                    t.text_tertiary,
                ))
        } else {
            pill("model-picker", t)
                .px(px(8.))
                .gap(px(4.))
                .child(labels)
                .child(icon("chevron-down-small", 16., t.text_tertiary))
        };
        let model_button = div()
            .relative()
            // Chromium clips the open pill's fill to its 14 px rounded box: a rect under that
            // anti-aliased clip.
            .when(open, |d| {
                d.child(crate::rounded_clip(
                    px(14.),
                    shapes::Corner::Round,
                    div()
                        .absolute()
                        .inset_0()
                        .child(shapes::decoration(shapes::Decoration {
                            fill: Some(t.ghost_hover),
                            ..Default::default()
                        })),
                ))
            })
            .child(
                trigger
                    .track_focus(&self.model_focus)
                    .focus_visible(menu::trigger_focus)
                    .when(!loading, |d| {
                        d.on_hover(cx.listener(|this, hovered: &bool, _, _| {
                            this.menu_trigger_hovered = *hovered
                        }))
                        .on_key_down(cx.listener(|this, event, window, cx| {
                            this.trigger_key(Menu::Effort, event, window, cx)
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_menu(Menu::Effort, window, cx)
                        }))
                    }),
            )
            .when(self.menu == Some(Menu::Effort), |d| {
                d.child(self.anchored_menu(self.effort_menu(t, cx), Some(EFFORT_MENU_SLOT), cx))
            })
            .when(self.menu == Some(Menu::Models), |d| {
                d.child(self.anchored_menu(self.models_menu(t, cx), Some(EFFORT_MENU_SLOT), cx))
            });
        let has_input = !self.content.is_empty() || !self.attachments.is_empty();
        let stopping = self.running && !has_input;
        let waiting_for_model = loading && !self.running;
        let armed = stopping && self.stop_armed.is_some();
        // The docked body is a cc layer: its icons blend onto its transparent tile, not onto
        // the card as the home composer's do (whose fitted pixels those are).
        let docked = self.docked;
        let voice_ring = t.text;
        let footer_icon =
            move |name: &str, color| icon(name, 16., color).when(docked, |i| i.own_layer(None));
        let left = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                crate::tooltip::tooltip(
                    pill("add-files", t)
                        .role(gpui::Role::Button)
                        .aria_label("Add files and more")
                        .aria_expanded(self.slash.open && self.slash.mention)
                        .tab_index(0)
                        .focus_visible(menu::trigger_focus)
                        .w(px(28.))
                        .justify_center()
                        .child(footer_icon("plus", t.text))
                        .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                            this.slash.add_mouse_open = this.slash.open && this.slash.mention;
                        }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.slash.add_mouse_open = false;
                                this.toggle_add(window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_add(window, cx))),
                    "Add files and more",
                    Some("@"),
                    crate::tooltip::Side::Top,
                )
                // The 1 px trigger border leaves a 4 px visible gap with this content-edge offset.
                .side_offset(px(5.)),
            )
            .child(
                div()
                    .relative()
                    .child(
                        pill("permissions", t)
                            .track_focus(&self.permissions_focus)
                            .focus_visible(menu::trigger_focus)
                            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                this.menu_trigger_hovered = *hovered
                            }))
                            .on_key_down(cx.listener(|this, event, window, cx| {
                                this.trigger_key(Menu::Permissions, event, window, cx)
                            }))
                            .px(px(6.))
                            .gap(px(4.))
                            .child(footer_icon(
                                if self.permission == Permission::Ask {
                                    "composer-permission-ask"
                                } else {
                                    "permissions"
                                },
                                t.text_tertiary,
                            ))
                            .when(self.footer_width > SM_WIDTH, |d| {
                                d.child(
                                    text(
                                        self.permission.short_label(),
                                        13.,
                                        18.,
                                        400.,
                                        t.text_tertiary,
                                    )
                                    .when(self.docked, |d| d.top_layer()),
                                )
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::Permissions, window, cx)
                            })),
                    )
                    .when(self.menu == Some(Menu::Permissions), |d| {
                        d.child(self.anchored_menu(self.permissions_menu(t, cx), None, cx))
                    }),
            );
        let send = div()
            .id("send")
            .role(gpui::Role::Button)
            .aria_label(self.send_label())
            .aria_disabled(waiting_for_model)
            .tab_index(0)
            .rounded(Theme::RADIUS_FULL)
            .focus_visible(move |style| {
                style.shadow(vec![gpui::BoxShadow {
                    color: voice_ring,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .ml(px(8.))
            .size(px(28.))
            .p(px(2.))
            .flex()
            .items_center()
            .justify_center()
            .relative()
            .child(if waiting_for_model {
                loading_send(t.composer_primary, t.text_composer_primary).into_any_element()
            } else {
                shapes::decoration(deco(9999., t.composer_primary, vec![])).into_any_element()
            })
            .when(armed, |d| {
                // `keyboardShortcutLabel`: a 12 px `Esc` replaces the square while armed.
                d.child(text("Esc", 12., 16., 400., t.text_composer_primary))
            })
            .when(stopping && !armed, |d| {
                d.child(
                    div()
                        .size(px(8.8))
                        .rounded(px(1.))
                        .bg(t.text_composer_primary),
                )
            })
            .when(!stopping && !waiting_for_model, |d| {
                d.child(
                    icon(
                        if has_input {
                            "composer-arrow-up"
                        } else {
                            "voice"
                        },
                        16.,
                        t.text_composer_primary,
                    )
                    .when(docked, |i| i.runtime()),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if waiting_for_model {
                    cx.stop_propagation();
                    return;
                }
                if stopping {
                    this.stop_armed = None;
                    this.focus_handle.focus(window, cx);
                    window.dispatch_action(Box::new(Stop), cx);
                } else {
                    this.submit(&Submit, window, cx);
                }
            }));
        let right = div()
            .flex()
            .items_center()
            .child(model_button)
            .child(self.dictation_button(t, cx))
            .child(send);
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(8.))
            .pb(px(8.))
            .child(left)
            .child(right)
    }
}
