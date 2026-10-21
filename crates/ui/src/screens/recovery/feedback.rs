use crate::{SANS, composer::{self, Composer}, icon, text};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Toggled, Window, div, prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;

const OPTIONS: [(&str, &str); 6] = [
    ("bug", "Bug"), ("bad-result", "Bad result"), ("good-result", "Good result"),
    ("safety_check", "Safety check"), ("research-failure", "Research failure"), ("other", "Other"),
];

pub enum Event {
    Close,
    Submit { classification: &'static str, reason: String, include_logs: bool },
}

pub struct Feedback {
    details: Entity<Composer>,
    category: Option<usize>,
    include_logs: bool,
    pending: bool,
    choices: [FocusHandle; 6],
    logs: FocusHandle,
    learn_more: FocusHandle,
    submit: FocusHandle,
    close: FocusHandle,
    return_focus: Option<FocusHandle>,
}

impl EventEmitter<Event> for Feedback {}
impl Focusable for Feedback {
    fn focus_handle(&self, cx: &App) -> FocusHandle { self.details.focus_handle(cx) }
}

impl Feedback {
    pub fn new(window: &Window, cx: &mut Context<Self>) -> Self {
        let details = cx.new(|cx| Composer::settings_field("Share details (required)", "", true, cx).with_field_rows(5));
        cx.observe(&details, |_, _, cx| cx.notify()).detach();
        Self {
            details, category: None, include_logs: true, pending: false,
            choices: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            logs: cx.focus_handle().tab_stop(true), submit: cx.focus_handle().tab_stop(true),
            learn_more: cx.focus_handle().tab_stop(true),
            close: cx.focus_handle().tab_stop(true), return_focus: window.focused(cx),
        }
    }

    pub fn set_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        self.pending = pending;
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = &self.return_focus { focus.focus(window, cx); }
        cx.emit(Event::Close);
    }

    fn can_submit(&self, cx: &App) -> bool {
        !self.pending && self.category.is_some() && !self.details.read(cx).field_text().trim().is_empty()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_submit(cx) { return; }
        let classification = OPTIONS[self.category.unwrap()].0;
        let reason = self.details.read(cx).field_text().trim().to_owned();
        self.pending = true;
        cx.emit(Event::Submit { classification, reason, include_logs: self.include_logs });
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.close(window, cx),
            "enter" if event.keystroke.modifiers.platform => self.submit(cx),
            "tab" => {
                let controls = [
                    self.choices[0].clone(), self.choices[1].clone(), self.choices[2].clone(),
                    self.choices[3].clone(), self.choices[4].clone(), self.choices[5].clone(),
                    self.details.focus_handle(cx), self.logs.clone(), self.learn_more.clone(), self.submit.clone(), self.close.clone(),
                ];
                let current = controls.iter().position(|handle| handle.contains_focused(window, cx)).unwrap_or(6);
                let direction = if event.keystroke.modifiers.shift { controls.len() - 1 } else { 1 };
                let mut next = (current + direction) % controls.len();
                if next == 9 && !self.can_submit(cx) { next = (next + direction) % controls.len(); }
                controls[next].focus(window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for Feedback {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let keyboard = window.last_input_was_keyboard();
        let mut options = div().id("feedback-options").flex().flex_wrap().gap(px(8.)).role(gpui::Role::RadioGroup).aria_label("Feedback options");
        for (index, (_, label)) in OPTIONS.iter().enumerate() {
            let selected = self.category == Some(index);
            let foreground = if selected { theme.text_inverse } else { theme.text };
            options = options.child(div().id(("feedback-category", index)).track_focus(&self.choices[index])
                .role(gpui::Role::RadioButton).aria_label(*label)
                .aria_toggled(if selected { Toggled::True } else { Toggled::False })
                .flex().items_center().gap(px(4.)).px(px(12.)).py(px(4.)).rounded_full().border_1().border_color(theme.border)
                .when(selected, |d| d.bg(theme.button_primary_solid))
                .when(keyboard, |d| d.focus(|s| s.border_color(theme.focus_ring)))
                .child(div().size(px(16.)).rounded_full().border_1().border_color(foreground).flex().items_center().justify_center()
                    .when(selected, |d| d.child(div().size(px(8.)).rounded_full().bg(foreground))))
                .child(text(*label, 14., 20., 430., foreground))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.category = Some(index);
                    this.choices[index].focus(window, cx);
                    cx.notify();
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    let next = match event.keystroke.key.as_str() {
                        "left" | "up" => (index + OPTIONS.len() - 1) % OPTIONS.len(),
                        "right" | "down" => (index + 1) % OPTIONS.len(),
                        "space" | "enter" => index,
                        _ => return,
                    };
                    this.category = Some(next);
                    this.choices[next].focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                })));
        }
        let submit_enabled = self.can_submit(cx);
        let logs = div().id("feedback-include-logs").track_focus(&self.logs).role(gpui::Role::CheckBox)
            .aria_label("Include ChatGPT diagnostic logs")
            .aria_toggled(if self.include_logs { Toggled::True } else { Toggled::False })
            .flex().items_center().gap(px(8.))
            .child(div().size(px(16.)).rounded(px(4.)).border_1().border_color(theme.border)
                .when(self.include_logs, |d| d.bg(theme.button_primary_solid).child(icon("menu-check", 14., theme.text_inverse))))
            .child(text("Include ChatGPT diagnostic logs", 14., 20., 430., theme.text_secondary))
            .on_click(cx.listener(|this, _, window, cx| {
                this.include_logs = !this.include_logs;
                this.logs.focus(window, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                    this.include_logs = !this.include_logs;
                    cx.stop_propagation();
                    cx.notify();
                }
            }));
        let close = div().id("feedback-close").track_focus(&self.close).role(gpui::Role::Button).aria_label("Close dialog")
            .absolute().right(px(16.)).top(px(16.)).size(px(28.)).flex().items_center().justify_center().rounded(px(8.))
            .child(icon("close", 16., theme.text_secondary))
            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") { this.close(window, cx); cx.stop_propagation(); }
            }));
        let submit = div().id("feedback-submit").track_focus(&self.submit).role(gpui::Role::Button)
            .aria_label("Submit").aria_disabled(!submit_enabled).w_full().h(px(32.)).rounded(px(12.5))
            .bg(theme.button_primary_solid).flex().items_center().justify_center()
            .when(!submit_enabled, |d| d.opacity(0.4))
            .child(text(if self.pending { "Uploading feedback" } else { "Submit" }, 14., 20., 510., theme.text_inverse))
            .on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") { this.submit(cx); cx.stop_propagation(); }
            }));
        let width = (f32::from(window.viewport_size().width) * 0.92).min(600.);
        let body = composer::surface(&theme, width, px(24.), |body| body.p(px(20.)).gap(px(8.))
            .child(text("Share feedback", 18., 28., 600., theme.text))
            .child(div().pt(px(12.)).flex().flex_col().gap(px(12.)).child(options).child(self.details.clone()).child(logs)
                .child(div().flex().flex_wrap().gap(px(3.))
                    .child(text("Your feedback can be used to improve ChatGPT.", 14., 20., 430., theme.text_secondary))
                    .child(div().id("feedback-learn-more").track_focus(&self.learn_more).role(gpui::Role::Link)
                        .child(text("Learn more.", 14., 20., 430., theme.text_link).underline())
                        .on_click(|_, _, cx| cx.open_url("https://help.openai.com/en/articles/5722486-how-your-data-is-used-to-improve-model-performance"))
                        .on_key_down(|event, _, cx| {
                            if event.keystroke.key == "enter" {
                                cx.open_url("https://help.openai.com/en/articles/5722486-how-your-data-is-used-to-improve-model-performance");
                                cx.stop_propagation();
                            }
                        }))))
            .child(div().pt(px(20.)).child(submit)).child(close));
        div().id("feedback-modal").size_full().absolute().top_0().left_0().bg(theme.dialog_backdrop).occlude()
            .font_family(SANS).flex().items_center().justify_center().capture_key_down(cx.listener(Self::key))
            .capture_action(cx.listener(|this, _: &composer::Dismiss, window, cx| this.close(window, cx)))
            .child(div().id("feedback-card").relative().max_w(window.viewport_size().width * 0.92)
                .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close(window, cx)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(body))
    }
}
