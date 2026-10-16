use super::*;

impl Composer {
    pub(super) fn footer(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let open = matches!(self.menu, Some(Menu::Effort | Menu::Models));
        let model = self.current_model();
        let model_label = model.map_or("Loading model…".into(), |m| m.label.clone());
        let labels = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(tabular(label(model_label, t.text)).when(open, |d| d.invisible()))
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
                        // Over the open pill: blitted directly, unlike the footer labels.
                        .child(text("Select effort", 13., 18., 400., t.text_tertiary)),
                )
            });
        let trigger = if self.footer_width <= XS_WIDTH {
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
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        this.menu_trigger_hovered = *hovered
                    }))
                    .on_key_down(cx.listener(|this, event, window, cx| {
                        this.trigger_key(Menu::Effort, event, window, cx)
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_menu(Menu::Effort, window, cx)
                    })),
            )
            .when(self.menu == Some(Menu::Effort), |d| {
                d.child(self.anchored_menu(self.effort_menu(t, cx), Some(EFFORT_MENU_SLOT), cx))
            })
            .when(self.menu == Some(Menu::Models), |d| {
                d.child(self.anchored_menu(self.models_menu(t, cx), Some(EFFORT_MENU_SLOT), cx))
            });
        let has_input = !self.content.is_empty() || !self.attachments.is_empty();
        // The docked body is a cc layer: its icons blend onto its transparent tile, not onto
        // the card as the home composer's do (whose fitted pixels those are).
        let docked = self.docked;
        let footer_icon =
            move |name: &str, color| icon(name, 16., color).when(docked, |i| i.own_layer(None));
        let left = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                pill("add-files", t)
                    .w(px(28.))
                    .justify_center()
                    .child(footer_icon("plus", t.text))
                    .on_click(cx.listener(|this, _, _, cx| this.add_files(cx))),
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
                                d.child(label(self.permission.short_label(), t.text_tertiary))
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::Permissions, window, cx)
                            })),
                    )
                    .when(self.menu == Some(Menu::Permissions), |d| {
                        d.child(self.anchored_menu(self.permissions_menu(t, cx), None, cx))
                    }),
            );
        let right = div()
            .flex()
            .items_center()
            .child(model_button)
            .child(
                pill("dictate", t)
                    .w(px(28.))
                    .justify_center()
                    .child(footer_icon("mic", t.text)),
            )
            .child(
                div()
                    .id("send")
                    .ml(px(8.))
                    .size(px(28.))
                    .p(px(2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .relative()
                    .child(shapes::decoration(deco(9999., t.composer_primary, vec![])))
                    // Over the disc in the docked body's layer, without the page's clips.
                    .child(
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
                    .on_click(cx.listener(|this, _, window, cx| this.submit(&Submit, window, cx))),
            );
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
