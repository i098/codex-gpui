use super::*;
use crate::screens::computer_use::Load;

impl SettingsPane {
    pub(super) fn profile(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let page = self.page.read(cx);
        let content = match &page.profile {
            Load::Loading => div()
                .h(px(256.))
                .flex()
                .items_center()
                .justify_center()
                .child(text("Loading…", 14., 20., 430., t.text_tertiary)),
            Load::Failed(_) => div()
                .h(px(256.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(16.))
                .child(text(
                    "We’re having trouble loading your profile. Please try again later.",
                    14.,
                    20.,
                    430.,
                    t.text_tertiary,
                ))
                .child(
                    button("Retry", t.text_alpha_5, None, t)
                        .id("profile-retry")
                        .focusable()
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(crate::focus::trigger_focus)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.page.update(cx, |page, cx| {
                                page.profile_requested = false;
                                page.profile = Load::Loading;
                                cx.notify();
                            });
                        })),
                ),
            Load::Ready(None) => div()
                .min_h(px(256.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(
                    div()
                        .size(px(112.))
                        .rounded_full()
                        .bg(t.primary_soft_alpha)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("settings-profile", 52., t.text_tertiary)),
                )
                .child(text(
                    "Profile not found or private",
                    14.,
                    20.,
                    430.,
                    t.text_tertiary,
                )),
            Load::Ready(Some(profile)) => {
                let details = &profile["profile_details"];
                let name = details["display_name"]
                    .as_str()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or("ChatGPT user");
                let username = details["username"].as_str().filter(|name| !name.is_empty());
                let mut content =
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(24.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .children(details["profile_picture_url"].as_str().map(|url| {
                                    img(url.to_owned())
                                        .size(px(112.))
                                        .rounded_full()
                                        .into_any_element()
                                }))
                                .child(text(name.to_owned(), 28., 34., 500., t.text))
                                .children(username.map(|username| {
                                    text(format!("@{username}"), 14., 20., 430., t.text_secondary)
                                })),
                        )
                        .children(details["bio"].as_str().filter(|bio| !bio.is_empty()).map(
                            |bio| text(bio.to_owned(), 14., 21., 430., t.text).whitespace_normal(),
                        ));
                let settings = &profile["page"]["display_settings"];
                if settings["show_usage_stats_section"] != false {
                    let stats = &profile["page"]["stats"];
                    content = if stats.is_null() {
                        content.child(text(
                            "Profile stats unavailable",
                            14.,
                            20.,
                            430.,
                            t.text_tertiary,
                        ))
                    } else {
                        content.child(
                            div().flex().gap(px(24.)).children(
                                [
                                    (
                                        "Current streak",
                                        stats["current_streak_days"]
                                            .as_u64()
                                            .map(|value| value.to_string()),
                                    ),
                                    (
                                        "Longest streak",
                                        stats["longest_streak_days"]
                                            .as_u64()
                                            .map(|value| value.to_string()),
                                    ),
                                    (
                                        "Lifetime tokens",
                                        stats["agentic"]["lifetime_tokens"]
                                            .as_u64()
                                            .map(|value| value.to_string()),
                                    ),
                                ]
                                .into_iter()
                                .filter_map(|(label, value)| {
                                    value.map(|value| {
                                        div()
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap(px(4.))
                                            .child(text(value, 20., 26., 500., t.text))
                                            .child(text(label, 14., 20., 430., t.text_tertiary))
                                    })
                                }),
                            ),
                        )
                    };
                }
                content
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(46.)))
            .child(
                div()
                    .id("profile-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(24.))
                    .child(div().max_w(px(768.)).w_full().mx_auto().child(content)),
            )
            .into_any_element()
    }
}
