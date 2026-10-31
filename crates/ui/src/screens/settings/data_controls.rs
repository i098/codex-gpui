//! `Settings → Data controls`: the ChatGPT account data export and the Marketing privacy
//! subpage (`data-controls-settings`). The app drives the ChatGPT backend through [`Dispatch`].

use super::{Layer, deco, on_card, probe, settings_row, settings_row_content, switch};
use crate::screens::computer_use::Load;
use crate::{focus, text};
use gpui::{
    AnyElement, App, Context, Div, Global, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

/// A Marketing privacy toggle and its `granular_consent` key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    Marketing,
    Personalization,
}

impl Consent {
    pub fn key(self) -> &'static str {
        match self {
            Consent::Marketing => "marketing",
            Consent::Personalization => "personalization",
        }
    }
}

/// `GET /user_granular_consent`; a missing choice shows on unless consent is required.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Consents {
    pub marketing: Option<bool>,
    pub personalization: Option<bool>,
    pub required: bool,
}

impl Consents {
    pub fn from_response(value: &Value) -> Option<Self> {
        let consent = &value["granular_consent"];
        let choice = |key: &str| match &consent[key] {
            Value::Null => Some(None),
            Value::Bool(on) => Some(Some(*on)),
            _ => None,
        };
        Some(Self {
            marketing: choice("marketing")?,
            personalization: choice("personalization")?,
            required: value["is_consent_required"].as_bool()?,
        })
    }

    pub fn checked(&self, consent: Consent) -> bool {
        match consent {
            Consent::Marketing => self.marketing,
            Consent::Personalization => self.personalization,
        }
        .unwrap_or(!self.required)
    }

    pub fn set(&mut self, consent: Consent, on: bool) {
        match consent {
            Consent::Marketing => self.marketing = Some(on),
            Consent::Personalization => self.personalization = Some(on),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Export,
    SetConsent(Consent, bool),
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    /// A personal ChatGPT account may request an export.
    pub export_available: bool,
    /// The original's `3157804138` gate shows `Marketing privacy`.
    pub privacy_available: bool,
    pub consents: Load<Consents>,
    pub exporting: bool,
    pub saving: bool,
    /// Bumped when an export request settles; the dialog closes on success.
    pub export_revision: u64,
    pub export_requested: bool,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

fn dispatch(action: Action, cx: &mut App) {
    if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
        dispatch(action, cx);
    }
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("data-controls-settings", cx, |_, cx| {
            cx.observe_global::<State>(|this: &mut DataControls, cx| {
                let state = cx.global::<State>();
                if state.export_revision != this.seen_export {
                    this.seen_export = state.export_revision;
                    if state.export_requested {
                        this.export_open = false;
                    }
                }
                cx.notify();
            })
            .detach();
            dispatch(Action::Refresh, cx);
            DataControls {
                layer: Layer::default(),
                privacy_open: false,
                export_open: false,
                seen_export: cx.default_global::<State>().export_revision,
            }
        })
        .into_any_element()
}

struct DataControls {
    layer: Layer,
    privacy_open: bool,
    export_open: bool,
    seen_export: u64,
}

impl DataControls {
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        primary: bool,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle(id, 0, window, cx);
        let on_click = Arc::new(on_click);
        let on_key = on_click.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(label))
                .relative()
                .h(px(28.))
                .px(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .child(on_card(
                    12.5,
                    shapes::Corner::Squircle,
                    if primary { t.text } else { t.text_alpha_5 },
                    None,
                ))
                .child(text(
                    label,
                    13.,
                    18.,
                    430.,
                    if primary { t.background_panel } else { t.text },
                ))
                .when(disabled, |d| d.opacity(0.6)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                on_click(this, cx);
                cx.notify();
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                on_key(this, cx);
                cx.notify();
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn consent_switch(
        &self,
        consent: Consent,
        label: &'static str,
        state: &State,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Load::Ready(consents) = &state.consents else {
            return div().into_any_element();
        };
        let checked = consents.checked(consent);
        let disabled = state.saving;
        let handle = focus::handle(label, 0, window, cx);
        focus::ring(
            switch(consent.key(), checked, disabled, cx.theme())
                .role(gpui::Role::CheckBox)
                .aria_label(SharedString::from(label))
                .aria_toggled(if checked {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                }),
            &handle,
            px(10.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            if !disabled {
                dispatch(Action::SetConsent(consent, !checked), cx);
            }
        }))
        .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                dispatch(Action::SetConsent(consent, !checked), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn card(&self, rows: Vec<Div>, t: &Theme) -> Div {
        let last = rows.len().saturating_sub(1);
        let divider = crate::over(t.border, t.background_panel);
        div()
            .relative()
            .p(px(1.))
            .child(deco(
                &self.layer,
                20.,
                shapes::Corner::Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(crate::rounded_clip(
                px(19.),
                shapes::Corner::Squircle,
                div()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().enumerate().map(|(i, row)| {
                        row.when(i < last, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left(px(16.))
                                    .right(px(16.))
                                    .bottom_0()
                                    .h(px(1.))
                                    .bg(divider),
                            )
                        })
                    })),
            ))
    }

    fn main_rows(&self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let t = cx.theme().clone();
        let mut rows = Vec::new();
        if state.export_available {
            let export = self.button(
                "data-controls-export",
                "Export",
                false,
                false,
                window,
                cx,
                |this, _| this.export_open = true,
            );
            rows.push(settings_row("Export data", None, export, &t));
        }
        if state.privacy_available {
            let manage = self.button(
                "data-controls-marketing-privacy",
                "Manage",
                false,
                false,
                window,
                cx,
                |this, _| this.privacy_open = true,
            );
            rows.push(settings_row("Marketing privacy", None, manage, &t));
        }
        rows
    }

    fn privacy_rows(&self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let t = cx.theme().clone();
        if let Load::Failed(_) = &state.consents {
            let retry = self.button(
                "data-controls-privacy-retry",
                "Retry",
                false,
                false,
                window,
                cx,
                |_, cx| dispatch(Action::Refresh, cx),
            );
            return vec![settings_row(
                "Couldn’t load marketing privacy settings",
                None,
                retry,
                &t,
            )];
        }
        [
            (
                Consent::Marketing,
                "Marketing measurement",
                "These cookies help us measure the effectiveness of our marketing campaigns.",
                "Allow marketing measurement",
            ),
            (
                Consent::Personalization,
                "Personalized marketing",
                "This helps us personalize and measure OpenAI’s own marketing on third-party platforms.",
                "Allow personalized marketing",
            ),
        ]
        .into_iter()
        .map(|(consent, title, detail, aria)| {
            let control = self.consent_switch(consent, aria, state, window, cx);
            settings_row_content(
                title,
                Some(
                    text(detail, 12., 16., 430., t.text_secondary)
                        .whitespace_normal()
                        .into_any_element(),
                ),
                control,
                &t,
            )
        })
        .collect()
    }

    fn export_dialog(&self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let cancel = self.button(
            "data-controls-export-cancel",
            "Cancel",
            false,
            state.exporting,
            window,
            cx,
            |this, _| this.export_open = false,
        );
        let confirm = self.button(
            "data-controls-export-confirm",
            "Confirm export",
            true,
            state.exporting,
            window,
            cx,
            |_, cx| dispatch(Action::Export, cx),
        );
        let exporting = state.exporting;
        let line = |s: &'static str| {
            text(s, 13., 19., 430., t.text_secondary)
                .whitespace_normal()
                .into_any_element()
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.4))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && !exporting {
                    this.export_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .relative()
                    .w(px(440.))
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(on_card(
                        20.,
                        shapes::Corner::Squircle,
                        t.background_panel,
                        Some(t.border),
                    ))
                    .child(text("Request data export?", 20., 26., 500., t.text))
                    .child(line("Your account details and chats will be included."))
                    .child(line(
                        "We’ll email a download link to your registered address.",
                    ))
                    .child(line("The download link expires after 24 hours."))
                    .child(line("Processing may take some time."))
                    .child(
                        div()
                            .pt(px(8.))
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(cancel)
                            .child(confirm),
                    ),
            )
    }
}

impl Render for DataControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        let t = cx.theme().clone();
        let privacy = self.privacy_open && state.privacy_available;
        let mut header = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .py(px(12.))
            .min_h(px(60.))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(8.));
        if privacy {
            header = header.child(div().flex().child(self.button(
                "data-controls-privacy-back",
                "Back",
                false,
                false,
                window,
                cx,
                |this, _| this.privacy_open = false,
            )));
        }
        header = header.child(text(
            if privacy {
                "Marketing privacy"
            } else {
                "Data controls"
            },
            28.,
            33.6,
            500.,
            t.text,
        ));
        let rows = if privacy {
            self.privacy_rows(&state, window, cx)
        } else {
            self.main_rows(&state, window, cx)
        };
        let body = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .pb(px(20.))
            .when(!rows.is_empty(), |d| d.child(self.card(rows, &t)));
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("data-controls-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
                            .pt(px(16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(header)
                            .child(body),
                    ),
            )
            .when(self.export_open && state.export_available, |d| {
                d.child(self.export_dialog(&state, window, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unset_choice_follows_whether_consent_is_required() {
        let mut consents = Consents::from_response(&json!({
            "granular_consent": {"marketing": null, "personalization": false},
            "is_consent_required": false,
        }))
        .unwrap();
        assert!(consents.checked(Consent::Marketing));
        assert!(!consents.checked(Consent::Personalization));
        consents.required = true;
        assert!(!consents.checked(Consent::Marketing));
        consents.set(Consent::Marketing, true);
        assert!(consents.checked(Consent::Marketing));
        assert_eq!(
            Consents::from_response(&json!({"granular_consent": null})),
            None
        );
    }
}
