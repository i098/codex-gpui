//! `HookReviewDialog` (`composer.hooks.review.*`): the composer's hooks that cannot run until
//! approved, each a selectable card with a `Details` disclosure, then `Not now`, `Allow all` and
//! `Allow selected`. Nothing is selected when it opens.
use super::*;
use crate::screens::settings::integrations::hook_event;
use serde_json::Value;
use theme::Appearance;

pub(super) struct HookReview {
    hooks: Vec<Value>,
    selected: Vec<bool>,
    open: Vec<bool>,
    /// The last request failed; `Some(true)` when another config layer overrides the approval.
    overridden: Option<bool>,
    /// Per hook its checkbox, then its `Details` button.
    rows: Vec<[FocusHandle; 2]>,
}

/// `settings`, `Not now`, `Allow all`, `Allow selected` in `controls`.
const LINK: usize = 0;
const NOT_NOW: usize = 1;
const ALLOW_ALL: usize = 2;
const ALLOW: usize = 3;

/// `HooksSettingsHookTitle`: `{index} - {statusMessage}`, else `Hook {index}`.
fn title(hook: &Value, index: usize) -> String {
    match hook["statusMessage"].as_str().map(str::trim) {
        Some(status) if !status.is_empty() => format!("{} - {status}", index + 1),
        _ => format!("Hook {}", index + 1),
    }
}

/// `HookReviewSource`: the plugin, `This project`, or the config layer that defines `hook`.
fn source(hook: &Value) -> String {
    match hook["source"].as_str().unwrap_or("unknown") {
        "plugin" => match hook["pluginId"]
            .as_str()
            .and_then(|id| id.split('@').next())
        {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => "Plugin".into(),
        },
        "project" => "This project".into(),
        "user" => "User config".into(),
        "sessionFlags" => "Session flags".into(),
        "unknown" => "Unknown source".into(),
        _ => "Admin config".into(),
    }
}

/// The `HooksSettingsHookDetails` rows of `hook`: label, value, and whether the value is code.
fn details(hook: &Value) -> Vec<(&'static str, String, bool)> {
    let field = |key: &str| hook[key].as_str().unwrap_or("").to_owned();
    let mut rows = vec![
        ("Source", source(hook), false),
        (
            "Hook type",
            hook_event(hook["eventName"].as_str().unwrap_or(""))
                .0
                .to_owned(),
            false,
        ),
    ];
    match hook["handlerType"].as_str() {
        Some("prompt") => rows.push(("Handler", "Prompt".into(), false)),
        Some("agent") => rows.push(("Handler", "Agent".into(), false)),
        Some("command") => rows.push(("Command", field("command"), true)),
        Some("mcpTool") => {
            rows.push(("MCP server", field("server"), true));
            rows.push(("Tool", field("tool"), true));
        }
        _ => {}
    }
    if let Some(matcher) = hook["matcher"].as_str() {
        rows.push(("Matcher", matcher.to_owned(), true));
    }
    rows.push((
        "Timeout",
        format!("{}s", hook["timeoutSec"].as_u64().unwrap_or_default()),
        false,
    ));
    rows
}

impl Dialog {
    pub fn hook_review(hooks: Vec<Value>, cx: &mut Context<Self>) -> Self {
        Self {
            hook_review: Some(HookReview {
                selected: vec![false; hooks.len()],
                open: vec![false; hooks.len()],
                rows: hooks
                    .iter()
                    .map(|_| [cx.focus_handle(), cx.focus_handle()])
                    .collect(),
                hooks,
                overridden: None,
            }),
            ..Self::new(Kind::HookReview, cx)
        }
    }

    fn review(&self) -> &HookReview {
        self.hook_review.as_ref().expect("hook review")
    }

    /// The approval request finished: `None` closes the dialog, `Some(overridden)` brings the
    /// buttons back with the error under the list.
    pub fn hook_review_finished(&mut self, error: Option<bool>, cx: &mut Context<Self>) {
        self.saving = false;
        match (error, &mut self.hook_review) {
            (None, _) => cx.emit(Outcome::Dismiss),
            (overridden, Some(review)) => review.overridden = overridden,
            _ => {}
        }
        cx.notify();
    }

    fn toggle_hook(&mut self, index: usize, details: bool, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if let Some(review) = &mut self.hook_review {
            let flag = if details {
                &mut review.open
            } else {
                &mut review.selected
            };
            flag[index] = !flag[index];
            cx.notify();
        }
    }

    /// `Allow all` or `Allow selected`: emits the (key, current hash) of the approved hooks.
    fn allow_hooks(&mut self, all: bool, cx: &mut Context<Self>) {
        let Some(review) = &mut self.hook_review else {
            return;
        };
        let hooks: Vec<(String, String)> = review
            .hooks
            .iter()
            .zip(&review.selected)
            .filter(|(_, selected)| all || **selected)
            .filter_map(|(hook, _)| {
                Some((
                    hook["key"].as_str()?.to_owned(),
                    hook["currentHash"].as_str()?.to_owned(),
                ))
            })
            .collect();
        if self.saving || hooks.is_empty() {
            return;
        }
        review.overridden = None;
        self.saving = true;
        cx.emit(Outcome::TrustHooks(hooks));
        cx.notify();
    }

    fn hook_review_activate(&mut self, control: usize, cx: &mut Context<Self>) {
        match control {
            LINK if !self.saving => cx.emit(Outcome::HookSettings),
            NOT_NOW => self.dismiss(cx),
            ALLOW_ALL => self.allow_hooks(true, cx),
            ALLOW => self.allow_hooks(false, cx),
            _ => {}
        }
    }

    /// `Tab` walks each card's checkbox and `Details`, the `settings` link, then the buttons;
    /// `Space` and `Enter` press the focused control; `Enter` elsewhere submits `Allow selected`.
    pub(super) fn hook_review_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handles: Vec<FocusHandle> = self
            .review()
            .rows
            .iter()
            .flatten()
            .chain(&self.controls[..4])
            .cloned()
            .collect();
        let rows = handles.len() - 4;
        let focused = handles.iter().position(|h| h.is_focused(window));
        match e.keystroke.key.as_str() {
            "escape" => self.dismiss(cx),
            "tab" => {
                let n = handles.len();
                let next = match focused {
                    Some(i) if e.keystroke.modifiers.shift => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                    None if e.keystroke.modifiers.shift => n - 1,
                    None => 0,
                };
                handles[next].focus(window, cx);
            }
            key @ ("enter" | "space") => match focused {
                Some(i) if i < rows => self.toggle_hook(i / 2, i % 2 == 1, cx),
                Some(i) => self.hook_review_activate(i - rows, cx),
                None if key == "enter" => self.allow_hooks(false, cx),
                None => {}
            },
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The dialog's height: padding, title, cards (open ones with their details), error, footer.
    pub(super) fn hook_review_height(&self) -> f32 {
        let review = self.review();
        let cards: f32 = review
            .hooks
            .iter()
            .zip(&review.open)
            .map(|(hook, open)| {
                let rows = details(hook).len() as f32;
                42. + if *open {
                    25. + rows * 18.5714 + (rows - 1.) * 8.
                } else {
                    0.
                }
            })
            .sum::<f32>()
            + 8. * review.hooks.len().saturating_sub(1) as f32;
        let error = if review.overridden.is_some() {
            30.5714
        } else {
            0.
        };
        // `max-h-[min(680px, 100dvh - 4rem)]`; the cards scroll.
        (20. + 28. + 12. + cards + error + 12. + 32. + 20.).min(680.)
    }

    pub(super) fn hook_review_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let review = self.review();
        let busy = self.saving;
        let cards = review.hooks.iter().enumerate().map(|(index, hook)| {
            let selected = review.selected[index];
            let open = review.open[index];
            let name = title(hook, index);
            let modified = hook["trustStatus"] == "modified";
            let badge = if modified {
                let color = t.accent_yellow;
                let label = if t.appearance == Appearance::Light {
                    t.text
                } else {
                    color
                };
                ("Modified", color.opacity(0.15), label)
            } else {
                ("New", t.text_success.opacity(0.15), t.text_success)
            };
            let checkbox = div()
                .id(("hook-review-check", index))
                .role(gpui::Role::CheckBox)
                .aria_label(name.clone())
                .aria_toggled(if selected {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .aria_disabled(busy)
                .track_focus(&review.rows[index][0])
                .focus_visible(crate::focus::trigger_focus)
                .flex_none()
                .size(px(14.))
                .rounded(px(5.))
                .border_1()
                .border_color(t.border_strong)
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |d| {
                    d.bg(t.surface_tertiary)
                        .child(icon("menu-check", 12., t.text))
                });
            let disclosure = tooltip(
                div()
                    .id(("hook-review-details", index))
                    .role(gpui::Role::Button)
                    .aria_label("Details")
                    .aria_expanded(open)
                    .track_focus(&review.rows[index][1])
                    .focus_visible(crate::focus::trigger_focus)
                    .flex_none()
                    .w(px(34.))
                    .h(px(24.))
                    .rounded(px(12.5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(t.ghost_hover))
                    .child(icon(
                        if open {
                            "chevron-up-small"
                        } else {
                            "chevron-down-small"
                        },
                        16.,
                        t.text_tertiary,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(review) = &mut this.hook_review {
                            review.open[index] = !review.open[index];
                            cx.notify();
                        }
                    })),
                "Details",
                None,
                Side::Top,
            );
            let header = div()
                .id(("hook-review-row", index))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_hook(index, false, cx)))
                .relative()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(8.))
                .child(checkbox)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(text(name, 13., 18.5714, 500., t.text).truncate())
                        .child(
                            div()
                                .flex_none()
                                .h(px(20.))
                                .px(px(8.))
                                .rounded_full()
                                .bg(badge.1)
                                .flex()
                                .items_center()
                                .child(text(badge.0, 12., 16., 500., badge.2)),
                        ),
                )
                .child(disclosure);
            let body = open.then(|| {
                let rows = details(hook);
                div().border_t_1().border_color(t.border).p(px(12.)).child(
                    div()
                        .px(px(12.))
                        .flex()
                        .gap(px(16.))
                        .child(div().flex().flex_col().gap(px(8.)).children(
                            rows.iter().map(|(label, ..)| {
                                text(*label, 13., 18.5714, 430., t.text_secondary)
                            }),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .children(rows.into_iter().map(|(_, value, code)| {
                                    if code {
                                        div().h(px(18.5714)).flex().items_center().child(
                                            text(value, 12., 16., 430., t.text)
                                                .font_family(crate::MONO)
                                                .truncate(),
                                        )
                                    } else {
                                        div().child(text(value, 13., 18.5714, 430., t.text))
                                    }
                                })),
                        ),
                )
            });
            div()
                .id(("hook-review-card", index))
                .flex_none()
                .rounded(px(12.5))
                .border_1()
                .border_color(if selected { t.border_strong } else { t.border })
                .when(selected, |d| d.bg(t.surface_secondary))
                .when(!busy, |d| d.hover(|s| s.bg(t.surface_secondary)))
                .child(header)
                .children(body)
        });
        let button = |control: usize, label: &'static str, primary: bool, disabled: bool| {
            div()
                .id(("hook-review-button", control))
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .track_focus(&self.controls[control])
                .focus_visible(crate::focus::trigger_focus)
                .h(px(32.))
                .px(px(16.))
                .rounded(px(12.5))
                .border_1()
                .flex()
                .items_center()
                .map(|d| {
                    if primary {
                        d.bg(t.text).border_color(t.border)
                    } else {
                        d.bg(t.text_alpha_5)
                            .border_color(gpui::transparent_black())
                            .when(!disabled, |d| d.hover(|s| s.bg(t.text_alpha_10)))
                    }
                })
                .when(disabled, |d| d.opacity(0.4))
                .child(text(
                    label,
                    14.,
                    18.,
                    430.,
                    if primary {
                        t.text_primary_solid
                    } else {
                        t.text
                    },
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.hook_review_activate(control, cx)))
        };
        let none_selected = !review.selected.contains(&true);
        div()
            .size_full()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text("Review hooks", 20., 28., 600., t.text))
            .child(
                div()
                    .id("hook-review-list")
                    .flex_1()
                    .min_h_0()
                    .pt(px(12.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(cards),
            )
            .children(review.overridden.map(|overridden| {
                div().pt(px(12.)).child(
                    text(
                        if overridden {
                            "Another config layer overrides these hook settings and must be changed before approval can take effect"
                        } else {
                            "Could not allow hooks"
                        },
                        13.,
                        18.5714,
                        430.,
                        t.text_error,
                    )
                    .whitespace_normal(),
                )
            }))
            .child(
                div()
                    .pt(px(12.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .child(text("You can change this later in ", 12., 16., 430., t.text_secondary))
                            .child(
                                div()
                                    .id("hook-review-settings")
                                    .role(gpui::Role::Link)
                                    .aria_label("settings")
                                    .track_focus(&self.controls[LINK])
                                    .focus_visible(crate::focus::trigger_focus)
                                    .rounded(px(7.5))
                                    .child(
                                        text("settings", 12., 16., 430., t.text_secondary).underline(),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.hook_review_activate(LINK, cx)
                                    })),
                            ),
                    )
                    .child(button(NOT_NOW, "Not now", false, busy))
                    .child(button(ALLOW_ALL, "Allow all", false, busy || review.hooks.is_empty()))
                    .child(button(ALLOW, "Allow selected", true, busy || none_selected)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn titles_number_hooks_and_fall_back_without_a_status_message() {
        assert_eq!(
            title(&json!({"statusMessage": " Checking "}), 0),
            "1 - Checking"
        );
        assert_eq!(title(&json!({"statusMessage": "  "}), 1), "Hook 2");
        assert_eq!(title(&json!({}), 2), "Hook 3");
    }

    #[test]
    fn details_follow_the_handler_and_skip_a_missing_matcher() {
        let command = json!({"source": "project", "eventName": "sessionStart",
            "handlerType": "command", "command": "./env.sh", "timeoutSec": 600});
        assert_eq!(
            details(&command),
            [
                ("Source", "This project".into(), false),
                ("Hook type", "SessionStart".into(), false),
                ("Command", "./env.sh".into(), true),
                ("Timeout", "600s".into(), false),
            ]
        );
        let tool = json!({"source": "plugin", "pluginId": "lint@market", "eventName": "stop",
            "handlerType": "mcpTool", "server": "s", "tool": "t", "matcher": "Bash", "timeoutSec": 5});
        let labels: Vec<_> = details(&tool)
            .iter()
            .map(|row| (row.0, row.1.clone()))
            .collect();
        assert_eq!(labels[0], ("Source", "lint".into()));
        assert_eq!(
            labels[2..].iter().map(|row| row.0).collect::<Vec<_>>(),
            ["MCP server", "Tool", "Matcher", "Timeout"]
        );
    }
}
