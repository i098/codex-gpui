use super::*;

impl Composer {
    /// `Select effort` popover: effort title, model row, slider of the model's efforts.
    pub(super) fn effort_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(model) = self.current_model() else {
            return div();
        };
        let efforts = model.efforts.clone();
        let selected = efforts.iter().position(|e| *e == self.effort).unwrap_or(0);
        let corner = |id: &'static str, name: &'static str, color: Hsla| {
            let hover = t.ghost_hover;
            div()
                .id(id)
                .absolute()
                .top_0()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(26.))
                        .rounded(px(7.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |s| s.bg(hover))
                        .child(icon(name, 16., color)),
                )
        };
        let bolt = if self.fast { t.accent } else { t.text_tertiary };
        let header = div()
            .relative()
            .h(px(48.))
            .px(px(32.))
            .flex()
            .justify_center()
            .child(
                corner("fast-mode", "composer-bolt", bolt)
                    .track_focus(&self.menu_focus.items[SPEED_ITEM])
                    .left_0()
                    .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                    .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                        if *hovered {
                            this.hover_speed(window, cx);
                        } else {
                            this.speed_timer = None;
                        }
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.activate_menu_item(SPEED_ITEM, window, cx)
                    }))
                    .when(self.speed_open, |d| {
                        // Right of the 32 px trigger plus 5 px, top-aligned with the menu's
                        // box (4 px above its padding box).
                        d.child(
                            deferred(
                                div()
                                    .absolute()
                                    .left(px(37.))
                                    .top(px(-4.))
                                    .child(self.speed_menu(t, cx)),
                            )
                            .with_priority(2),
                        )
                    }),
            )
            .child(
                div()
                    .id("select-model")
                    .track_focus(&self.menu_focus.items[0])
                    .p(px(4.))
                    .rounded(px(8.))
                    .child(
                        div()
                            .py(px(2.))
                            .px(px(4.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(text(
                                effort_label(&self.effort).to_string(),
                                14.,
                                20.,
                                500.,
                                t.accent,
                            ))
                            .child(
                                div()
                                    .relative()
                                    .px(px(16.))
                                    .child(tabular(text(
                                        model.label.clone(),
                                        12.,
                                        16.,
                                        400.,
                                        t.text_secondary,
                                    )))
                                    .child(
                                        icon("composer-chevron-right-small", 12., t.text_tertiary)
                                            .absolute()
                                            .right(px(4.))
                                            .top(px(2.)),
                                    ),
                            ),
                    )
                    .on_click(
                        cx.listener(|this, _, window, cx| this.activate_menu_item(0, window, cx)),
                    ),
            )
            .child(
                corner("reset-effort", "composer-reset", t.text_tertiary)
                    .track_focus(&self.menu_focus.items[2])
                    .right_0()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.activate_menu_item(2, window, cx)),
                    ),
            );
        // Track 230 px; dot centers 13 + 51·i; the thumb (28 px) centers on the selected dot.
        let n = efforts.len().max(2);
        let step = (230. - 26.) / (n - 1) as f32;
        let dot_x = |i: usize| 13. + step * i as f32;

        let slider = div().py(px(4.)).child(
            div()
                .h(px(32.))
                .py(px(2.))
                .px(px(8.))
                .flex()
                .items_center()
                .child(
                    div()
                        .id("effort-slider")
                        .relative()
                        .w_full()
                        .h(px(24.))
                        .child(shapes::decoration(deco(
                            12.,
                            // `_Track`: `color(srgb 1 1 1 / 0.1)`, an sRGB mix (8-bit).
                            t.text.opacity(0.1),
                            vec![shadow(t.border, 0., 0., 0.5, true)],
                        )))
                        .when(selected > 0, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(px(dot_x(selected)))
                                    .child(shapes::decoration(shapes::Decoration {
                                        radius: Corners {
                                            top_left: px(12.),
                                            bottom_left: px(12.),
                                            ..Corners::all(px(0.))
                                        },
                                        fill: Some(t.accent),
                                        ..Default::default()
                                    })),
                            )
                        })
                        .children((0..efforts.len()).map(|i| {
                            let color = if i <= selected {
                                gpui::rgba(0xffffff4c).into()
                            } else {
                                Hsla { a: 0.249, ..t.text }
                            };
                            div()
                                .absolute()
                                .left(px(dot_x(i) - 2.))
                                .top(px(10.))
                                .size(px(4.))
                                .child(shapes::decoration(deco(9999., color, vec![])))
                        }))
                        // `_ThumbSpring` (a transform) is its own composited layer over the
                        // track, drawn into the popup's render surface (its backdrop-filter):
                        // a layer nested in the popover's. cc bounds its ink (the `0 0 2px`
                        // shadow) at 3 sigma, 3 px out.
                        .child(
                            div()
                                .absolute()
                                .left(px(dot_x(selected) - 17.))
                                .top(px(-5.))
                                .size(px(34.))
                                .layer()
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(3.))
                                        .top(px(3.))
                                        .size(px(28.))
                                        .child(shapes::decoration(shapes::Decoration {
                                            border: Some((px(0.5), t.border_strong)),
                                            ..deco(
                                                9999.,
                                                t.control_thumb,
                                                vec![shadow(
                                                    gpui::rgba(0x0000001a).into(),
                                                    0.,
                                                    2.,
                                                    0.,
                                                    false,
                                                )],
                                            )
                                        })),
                                ),
                        )
                        .children(efforts.iter().enumerate().map(|(i, effort)| {
                            let effort = effort.clone();
                            div()
                                .id(("effort", i))
                                .absolute()
                                .left(px(dot_x(i) - step / 2.))
                                .top_0()
                                .w(px(step))
                                .h_full()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.effort = effort.clone();
                                    this.picked(cx);
                                    cx.notify();
                                }))
                        })),
                ),
        );
        popover(t, 254., |d| d.child(header).child(slider))
    }

    /// The `Speed` submenu (Radix `MenuSub`): `Standard`, `Fast`, `Ultrafast`, the current one
    /// checked. Pointer movement focuses an item, as in the permissions menu.
    fn speed_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        let current = usize::from(self.fast);
        popover(t, 233., |d| {
            d.children(SPEEDS.iter().enumerate().map(|(i, &(title, description))| {
                div()
                    .id(("speed", i))
                    .track_focus(&self.speed_focus.items[i])
                    .focus(move |s| s.bg(hover))
                    .on_mouse_move(cx.listener(move |this, _, window, cx| {
                        this.speed_focus.items[i].focus(window, cx);
                        cx.stop_propagation();
                    }))
                    // The root menu's mouse-down-out guard: the submenu lies outside its box.
                    .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        this.menu_trigger_hovered = *hovered
                    }))
                    .px(px(8.))
                    .py(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(15.))
                    .hover(move |s| s.bg(hover))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text(title, 13., 18.5714, 430., t.text))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(14.))
                                    .text_color(t.text_tertiary)
                                    .child(description),
                            ),
                    )
                    .when(i == current, |d| {
                        d.child(icon("menu-check", 16., t.text).opacity(0.75))
                    })
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.pick_speed(i, window, cx)),
                    )
            }))
        })
    }

    /// `Select model` list.
    pub(super) fn models_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        popover(t, 254., |d| {
            d.child(div().px(px(8.)).py(px(6.)).child(text(
                "Select model",
                12.,
                16.,
                400.,
                t.text_tertiary,
            )))
            .children(self.models.iter().enumerate().map(|(i, m)| {
                div()
                    .id(("model", i))
                    .track_focus(&self.menu_focus.items[i])
                    .focus(move |s| s.bg(hover))
                    .h(px(32.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(12.))
                    .when(i == self.model, |d| d.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .child(text(m.label.clone(), 14., 20., 430., t.text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_menu_item(i, window, cx)
                    }))
            }))
        })
    }

    /// `How should ChatGPT actions be approved?`
    pub(super) fn permissions_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.menu_item_hover;
        popover(t, 320., |d| {
            d.child(div().px(px(8.)).py(px(6.)).child(text(
                "How should ChatGPT actions be approved?",
                12.,
                16.,
                400.,
                t.text_tertiary,
            )))
            .children(Permission::ALL.into_iter().enumerate().map(|(i, p)| {
                div()
                    .id(p.short_label())
                    .track_focus(&self.menu_focus.items[i])
                    .focus(move |s| s.bg(hover))
                    .on_mouse_move(cx.listener(move |this, _, window, cx| {
                        this.menu_focus.items[i].focus(window, cx);
                        cx.stop_propagation();
                    }))
                    .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        if !hovered && this.menu_focus.items[i].is_focused(window) {
                            this.menu_focus.content.focus(window, cx);
                        }
                    }))
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .hover(move |s| s.bg(hover))
                    .child(text(p.option_label(), 14., 20., 430., t.text))
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(t.text_tertiary)
                            .child(p.description()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_menu_item(i, window, cx)
                    }))
            }))
        })
    }

    /// Pending approval request card (`PendingRequestItemPanel`).
    pub(super) fn approval_card(&self, a: &Approval, t: &Theme, cx: &mut Context<Self>) -> Div {
        let button = |id: &'static str, s: SharedString, bg: Hsla, fg: Hsla| {
            pill(id, t).px(px(10.)).bg(bg).child(label(s, fg))
        };
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .relative()
            .child(shapes::decoration(card_decoration(t)))
            .child(text(a.title.clone(), 14., 20., 500., t.text))
            .when_some(a.detail.clone(), |d, detail| {
                d.child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(t.code_block_background)
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(t.text)
                        .child(detail),
                )
            })
            .when_some(a.reason.clone(), |d, reason| {
                d.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(label("Reason", t.text_tertiary))
                        .child(label(reason, t.text_secondary)),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button("deny", "Deny".into(), gpui::transparent_black(), t.text).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.decide(ApprovalDecision::Decline, window, cx)
                            }),
                        ),
                    )
                    .child(
                        button(
                            "allow-session",
                            a.scoped_label.clone(),
                            t.button_secondary,
                            t.text,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.decide(ApprovalDecision::AcceptForSession, window, cx)
                        })),
                    )
                    .child(
                        button(
                            "allow-once",
                            "Allow once".into(),
                            t.button_primary_solid,
                            t.text_inverse,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.decide(ApprovalDecision::Accept, window, cx)
                        })),
                    ),
            )
    }

    /// Popover anchored above a footer button. `width`: the box the original centers on the
    /// button (`None`: left-aligned).
    pub(super) fn anchored_menu(
        &self,
        menu: Div,
        center_width: Option<f32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // `occlude`: a click in the menu must not reach the editor under it, whose mouse
        // down closes the menu.
        let menu = div()
            .id("menu")
            .occlude()
            .track_focus(&self.menu_focus.content)
            .on_key_down(cx.listener(Self::menu_key))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if !this.menu_trigger_hovered && this.menu.take().is_some() {
                    window.blur(cx);
                    cx.notify();
                }
            }))
            .child(menu);
        let slot = div().absolute().bottom(px(36.)).flex();
        deferred(match center_width {
            Some(w) => slot
                .left_0()
                .right_0()
                .justify_center()
                .child(div().flex_none().w(px(w)).pl(px(1.)).pb(px(1.)).child(menu)),
            None => slot.left_0().child(menu),
        })
        .with_priority(1)
        .into_any_element()
    }
}
