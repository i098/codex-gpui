use super::*;

impl ThreadView {
    /// The thread's part of the window titlebar (`Chat toolbar`): project icon and title, then
    /// `Chat actions` and `Toggle summary`, laid over the shell's titlebar (open-sidebar
    /// layout) from the main panel's left edge to the shell's `New tab` group.
    pub fn titlebar_header(&self, cx: &Context<Self>) -> Div {
        div()
            .absolute()
            .top_0()
            .left(Theme::SIDEBAR_WIDTH)
            .right(px(43.))
            // Chromium composites the `group/titlebar` (it overlaps composited layers): text
            // blits into its transparent tile.
            .layer()
            .child(self.header(cx))
    }

    pub(super) fn header(&self, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        div()
            .relative()
            .h(Theme::TITLEBAR_HEIGHT)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .pl(px(8.))
            .pr(px(6.))
            // Divider between the sidebar's and the thread's titlebar parts.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(12.))
                    .w(px(1.))
                    .h(px(20.))
                    .bg(over(t.border, layout_color(t))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("thread-project", 16., t.text)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .ml(px(-2.))
                            .px(px(6.))
                            .overflow_hidden()
                            .child(
                                text(self.title.clone(), 14., 24., 500., t.text).text_ellipsis(),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .child(icon_button(
                        "chat-actions",
                        icon("thread-chat-actions", 16., t.text_tertiary),
                        t,
                    ))
                    .child(
                        if self.summary_open {
                            // Pressed: `bg-text/10` squircle, primary icon.
                            div()
                                .id("toggle-summary")
                                .relative()
                                .size(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .child(shapes::decoration(shapes::Decoration {
                                    radius: Corners::all(px(12.5)),
                                    corner: shapes::Corner::Squircle,
                                    // Translucent in the titlebar's transparent layer.
                                    fill: Some(t.text_alpha_10),
                                    ..Default::default()
                                }))
                                .child(icon("thread-summary", 16., t.text))
                        } else {
                            icon_button(
                                "toggle-summary",
                                icon("thread-summary", 16., t.text_tertiary),
                                t,
                            )
                        }
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.summary_open ^= true;
                            cx.notify();
                        })),
                    ),
            )
    }
}
