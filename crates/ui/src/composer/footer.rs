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

impl Composer {
    pub(super) fn footer(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Div {
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
                    label(effort_label(&self.effort).to_string(), t.text_tertiary)
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
                .child(icon("composer-model", 16., t.text_tertiary))
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
        let voice_available = crate::HomeAccountAccess::available(cx);
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
                            .child(footer_icon("permissions", t.text_tertiary))
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
            .when(!voice_available && !has_input && !self.running, |d| {
                d.opacity(0.5)
            })
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
                        if has_input || !voice_available {
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
            .child(
                pill("dictate", t)
                    .tab_index(0)
                    .focus_visible(menu::trigger_focus)
                    .w(px(28.))
                    .justify_center()
                    .child(footer_icon("mic", t.text)),
            )
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
