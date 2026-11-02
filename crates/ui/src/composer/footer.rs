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
    /// The microphone, with the original's tooltip for its dictation state.
    pub(super) fn dictation_button(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> crate::tooltip::Tooltip {
        let label = self.dictation_label();
        let button = pill("dictate", t)
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
            .track_focus(&self.dictate_focus)
            .focus_visible(crate::focus::trigger_focus)
            .size(px(28.))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.dictate(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.dictate(window, cx)));
        crate::tooltip::tooltip(
            button,
            self.dictation_tooltip(),
            None,
            crate::tooltip::Side::Top,
        )
        .focus_handle(&self.dictate_focus)
    }
    pub(super) fn footer(&self, window: &mut Window, t: &Theme, cx: &mut Context<Self>) -> Div {
        let open = matches!(self.menu, Some(Menu::Effort | Menu::Models));
        let hotkey_home = self.hotkey_controls.is_some();
        let model = self.current_model();
        let loading = model.is_none();
        let model_label = model.map_or("Loading model…".into(), |m| m.label.clone());
        // The original keeps the open trigger at least as wide as it was at open and as its
        // `Select effort` placeholder (`minimumContentWidth`), so picking an effort or a model
        // does not move the trigger or the menu centered on it.
        let closed_width = window.use_keyed_state("model-trigger-width", cx, |_, _| px(0.));
        let placeholder = "Select effort";
        let min_width = open.then(|| {
            let placeholder_width = window
                .text_system()
                .layout_line(
                    placeholder,
                    px(13.),
                    &[TextRun {
                        len: placeholder.len(),
                        font: gpui::font(crate::SANS),
                        color: t.text_tertiary,
                        ..Default::default()
                    }],
                    None,
                )
                .width;
            (*closed_width.read(cx)).max(crate::run_width(placeholder_width, window))
        });
        let labels = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .when_some(min_width, |d, w| d.min_w(w).justify_center())
            // The simple view shows the placeholder; the advanced view the live selection.
            .map(|d| {
                if self.menu == Some(Menu::Effort) {
                    d.child(text(placeholder, 13., 18., 400., t.text_tertiary))
                } else {
                    d.child(
                        tabular(text(model_label, 13., 18., 400., t.text))
                            .when(self.docked, |d| d.top_layer()),
                    )
                    .when_some(model, |d, _| {
                        d.child(label(
                            effort_label(self.displayed_effort()).to_string(),
                            t.text_tertiary,
                        ))
                    })
                }
            })
            .when(!open, |d| {
                d.child(
                    gpui::canvas(
                        move |bounds, _, cx| closed_width.update(cx, |w, _| *w = bounds.size.width),
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            });
        let trigger = if loading {
            let value = "Loading model…";
            let width = crate::run_width(
                window
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
                    .width,
                window,
            );
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
            .id("model-picker-slot")
            .relative()
            // The tooltip owns the trigger's hover listener; the slot's matches its bounds.
            .when(!loading, |d| {
                d.on_hover(
                    cx.listener(|this, hovered: &bool, _, _| this.menu_trigger_hovered = *hovered),
                )
            })
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
            .child({
                let trigger = trigger
                    .track_focus(&self.model_focus)
                    .focus_visible(crate::focus::trigger_focus)
                    .when(!loading, |d| {
                        d.on_key_down(cx.listener(|this, event, window, cx| {
                            this.trigger_key(Menu::Effort, event, window, cx)
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_menu(Menu::Effort, window, cx)
                        }))
                    });
                // The original passes the menu's open state to its tooltip, which stays closed.
                if loading || open {
                    trigger.into_any_element()
                } else {
                    // `codex.command.composer.openModelPicker` (recorded in f1c step 32).
                    crate::tooltip::tooltip(
                        trigger,
                        "Select effort",
                        Some("⌃⇧M"),
                        crate::tooltip::Side::Top,
                    )
                    .focus_handle(&self.model_focus)
                    .into_any_element()
                }
            })
            .when(open, |d| {
                d.child(self.anchored_menu(
                    self.picker_menu(t, window, cx),
                    Some(EFFORT_MENU_SLOT),
                    cx,
                ))
            });
        let has_input = !self.content.is_empty()
            || !self.attachments.is_empty()
            || !self.diff_comments.is_empty();
        let stopping = self.running && !has_input;
        let waiting_for_model = loading && !self.running;
        let shell_disabled = self.shell_disabled_reason().is_some();
        let armed = stopping && self.stop_armed.is_some();
        // An empty draft turns Send into `Start new voice chat` (hotkey composers keep Send).
        let voice = !has_input && !hotkey_home && !self.running && !waiting_for_model;
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
                        .track_focus(&self.add_focus)
                        .focus_visible(crate::focus::trigger_focus)
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
                .focus_handle(&self.add_focus)
                // The 1 px trigger border leaves a 4 px visible gap with this content-edge offset.
                .side_offset(px(5.)),
            )
            .when(!self.chat, |d| {
                d.child(
                    div()
                        .id("permissions-slot")
                        .relative()
                        // The tooltip owns the trigger's hover listener; the slot's matches its bounds.
                        .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                            this.menu_trigger_hovered = *hovered
                        }))
                        .child({
                            let trigger = pill("permissions", t)
                                .role(gpui::Role::Button)
                                .aria_label("Change permissions")
                                .track_focus(&self.permissions_focus)
                                .focus_visible(crate::focus::trigger_focus)
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
                                            self.permission.short_label().to_owned(),
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
                                }));
                            if self.menu == Some(Menu::Permissions) {
                                trigger.into_any_element()
                            } else {
                                crate::tooltip::tooltip(
                                    trigger,
                                    "Change permissions",
                                    None,
                                    crate::tooltip::Side::Top,
                                )
                                .focus_handle(&self.permissions_focus)
                                .into_any_element()
                            }
                        })
                        .when(self.menu == Some(Menu::Permissions), |d| {
                            d.child(self.anchored_menu(self.permissions_menu(t, cx), None, cx))
                        }),
                )
            })
            .when(self.plan && !self.chat, |d| {
                // `composer.planModeIndicator`: a divider, then a chip whose click turns plan
                // mode off; hovering swaps its icon for the close icon.
                d.child(div().h(px(16.)).w(px(1.)).bg(t.border.opacity(0.7)))
                    .child(crate::tooltip::tooltip(
                        pill("plan-mode", t)
                            .group("plan-mode")
                            .role(gpui::Role::Button)
                            .aria_label("Plan")
                            .tab_index(0)
                            .focus_visible(crate::focus::trigger_focus)
                            .px(px(6.))
                            .gap(px(4.))
                            .child(
                                div()
                                    .group_hover("plan-mode", |s| s.hidden())
                                    .child(footer_icon(
                                        "settings-personalization",
                                        t.text_tertiary,
                                    )),
                            )
                            .child(
                                div()
                                    .hidden()
                                    .group_hover("plan-mode", |s| s.block())
                                    .child(footer_icon("close", t.text_tertiary)),
                            )
                            .when(self.footer_width > SM_WIDTH, |d| {
                                d.child(
                                    text("Plan", 13., 18., 400., t.text_tertiary)
                                        .when(self.docked, |d| d.top_layer()),
                                )
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_plan(cx))),
                        "Create a plan",
                        None,
                        crate::tooltip::Side::Top,
                    ))
            });
        let send = div()
            .id("send")
            .role(gpui::Role::Button)
            .aria_label(if voice {
                "Start new voice chat"
            } else {
                self.send_label()
            })
            .aria_disabled(waiting_for_model || (hotkey_home && !has_input) || shell_disabled)
            .track_focus(&self.send_focus)
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
            .flex_none()
            .size(px(28.))
            .p(px(2.))
            .when(hotkey_home && !has_input, |button| {
                button.opacity(0.5).layer()
            })
            .when(shell_disabled, |button| button.opacity(0.5).layer())
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
                        if voice { "voice" } else { "composer-arrow-up" },
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
                } else if voice {
                    window.dispatch_action(Box::new(crate::StartVoiceChat), cx);
                } else {
                    this.submit(&Submit, window, cx);
                }
            }));
        // The submit labels are accessible names only; the voice start has a tooltip.
        let send = if voice {
            crate::tooltip::tooltip(
                send,
                "Start new voice chat",
                None,
                crate::tooltip::Side::Top,
            )
            .focus_handle(&self.send_focus)
            .into_any_element()
        } else {
            send.into_any_element()
        };
        // The original's right group: the model picker's slot is `flex-1 min-w-0` and takes what
        // the microphone and send (`shrink-0`) leave, so it clips to nothing in a narrow
        // footer; the fixed pair overflows to the left of the footer's end.
        let right = div()
            .flex()
            .flex_1()
            .min_w_0()
            .justify_end()
            .items_center()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .overflow_hidden()
                    .children(self.context_donut(t))
                    .child(model_button),
            )
            .children(self.hotkey_controls.clone())
            .child(self.dictation_button(t, cx))
            .child(send);
        // Grid columns: controls, an empty column, then the right group, 5 px apart. The
        // ChatGPT pill puts the editor in the middle column, 5 px from each side.
        if self.chat {
            return div()
                .flex()
                .items_center()
                .gap(px(5.))
                .p(px(8.))
                .child(left)
                .child(
                    self.card_editor(false, window, cx)
                        .flex_1()
                        .min_w_0()
                        .px_0()
                        .pb_0()
                        .min_h(px(20.)),
                )
                .child(right.flex_none());
        }
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(if self.hotkey_controls.is_some() {
                0.
            } else {
                8.
            }))
            .pb(px(if self.hotkey_controls.is_some() {
                0.
            } else {
                8.
            }))
            .child(left.min_w_0().flex_shrink(1.))
            .child(right)
    }
}
