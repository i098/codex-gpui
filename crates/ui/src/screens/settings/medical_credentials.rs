//! Settings ▸ Account ▸ Medical credentials (`CmeProfileSettings`): the clinician's CME profile
//! from the ChatGPT backend's `/cme/account`: NPI, degree, medical license state and number,
//! and the physician eligibility confirmation. `PATCH /cme/account` saves it.
use super::{
    account_page::{Kind, action, checkbox, status},
    backend::{self, Call},
    settings_row_content,
    worktrees::card,
};
use crate::{composer::Composer, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use theme::{ActiveTheme as _, Theme};

/// One `/cme/account` profile, as typed or as the server holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credentials {
    pub npi: String,
    pub degree: String,
    pub state: String,
    pub number: String,
    pub eligible: bool,
}

/// `(label, placeholder, error)` of the four text fields, in field order.
const FIELDS: [(&str, &str, &str); 4] = [
    (
        "National Provider Identifier (NPI)",
        "Enter your 10-digit NPI",
        "NPI must be 10 digits",
    ),
    (
        "Degree",
        "Enter your degree (e.g. MD)",
        "Degree is required",
    ),
    (
        "Medical license state",
        "Enter a 2-letter state (e.g. DE)",
        "Medical license state must be 2 letters",
    ),
    (
        "Medical license number",
        "Enter your medical license number",
        "Medical license number is required",
    ),
];

impl Credentials {
    fn from_value(v: &Value) -> Self {
        let s = |key: &str| v[key].as_str().unwrap_or_default().to_owned();
        Self {
            npi: s("npi"),
            degree: s("degree"),
            state: s("medical_license_state"),
            number: s("medical_license_number"),
            eligible: v["physician_eligible_for_cme"] == true,
        }
    }

    /// The original's save form: NPI digits only, trimmed text, an upper-case state.
    pub fn normalized(&self) -> Self {
        Self {
            npi: self.npi.chars().filter(char::is_ascii_digit).collect(),
            degree: self.degree.trim().to_owned(),
            state: self.state.trim().to_uppercase(),
            number: self.number.trim().to_owned(),
            eligible: self.eligible,
        }
    }

    /// Which of NPI, degree, state, number and eligibility fail the original's checks.
    pub fn invalid(&self) -> [bool; 5] {
        let n = self.normalized();
        [
            n.npi.len() != 10,
            n.degree.is_empty(),
            !(n.state.len() == 2 && n.state.chars().all(|c| c.is_ascii_uppercase())),
            n.number.is_empty(),
            !n.eligible,
        ]
    }

    fn body(&self) -> Value {
        json!({
            "npi": self.npi,
            "degree": self.degree,
            "medical_license_state": self.state,
            "medical_license_number": self.number,
            "physician_eligible_for_cme": self.eligible,
        })
    }
}

enum Load {
    Loading,
    /// No CME profile for this account: the section stays out of the page.
    Hidden,
    Failed,
    Ready,
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("medical-credentials", cx, |window, cx| {
            let page = cx.entity();
            window.defer(cx, move |_, cx| page.update(cx, MedicalCredentials::reload));
            MedicalCredentials {
                load: Load::Loading,
                saved: Credentials::default(),
                npi_editable: true,
                fields: None,
                eligible: false,
                saving: false,
                message: None,
            }
        })
        .into_any_element()
}

pub struct MedicalCredentials {
    load: Load,
    saved: Credentials,
    /// `is_npi_editable`: `false` shows the NPI as text.
    npi_editable: bool,
    fields: Option<[Entity<Composer>; 4]>,
    eligible: bool,
    saving: bool,
    message: Option<(SharedString, bool)>,
}

impl MedicalCredentials {
    /// `GET /cme/account`. A server error offers Try again; a refusal (no CME access) or no
    /// backend hides the section.
    // shortcut: the backend's answer stands in for the `chatgpt.cme` capability; read the
    // workspace capability when the app has it.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        backend::request(
            Call::new("GET", "/cme/account", None),
            cx,
            |this, result, cx| {
                this.load = match result {
                    Ok(response) if response.ok() => {
                        this.show(&response.body, cx);
                        Load::Ready
                    }
                    Ok(response) if response.status >= 500 => Load::Failed,
                    _ => Load::Hidden,
                };
            },
        );
        cx.notify();
    }

    /// Puts the server's profile into the form.
    fn show(&mut self, body: &Value, cx: &mut Context<Self>) {
        self.saved = Credentials::from_value(body);
        self.npi_editable = body["is_npi_editable"] != false;
        let saved = &self.saved;
        let values = [&saved.npi, &saved.degree, &saved.state, &saved.number];
        let fields = std::array::from_fn(|i| {
            let field = cx.new(|cx| Composer::settings_compact_field(FIELDS[i].1, values[i], cx));
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            field
        });
        self.fields = Some(fields);
        self.eligible = saved.eligible;
    }

    fn draft(&self, cx: &App) -> Credentials {
        let Some(fields) = &self.fields else {
            return self.saved.clone();
        };
        let text = |i: usize| fields[i].read(cx).field_text().to_owned();
        Credentials {
            npi: if self.npi_editable {
                text(0)
            } else {
                self.saved.npi.clone()
            },
            degree: text(1),
            state: text(2),
            number: text(3),
            eligible: self.eligible,
        }
    }

    /// Save is on for a valid draft that differs from the saved profile.
    fn can_save(&self, cx: &App) -> bool {
        let draft = self.draft(cx);
        !self.saving
            && !draft.invalid().contains(&true)
            && draft.normalized() != self.saved.normalized()
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        let draft = self.draft(cx).normalized();
        self.saving = true;
        self.message = None;
        let call = Call::new("PATCH", "/cme/account", Some(draft.body()));
        backend::request(call, cx, move |this, result, cx| {
            this.saving = false;
            this.message = Some(match result {
                Ok(response) if response.ok() => {
                    let body = if response.body.is_object() {
                        response.body
                    } else {
                        draft.body()
                    };
                    this.show(&body, cx);
                    ("Medical credentials saved".into(), false)
                }
                _ => (
                    "Unable to save medical credentials. Please try again.".into(),
                    true,
                ),
            });
        });
        cx.notify();
    }

    fn form(&self, fields: &[Entity<Composer>; 4], t: &Theme, cx: &mut Context<Self>) -> Div {
        let draft = self.draft(cx);
        // The original shows errors once the form was edited.
        let edited = draft != self.saved;
        let invalid = draft.invalid();
        let error = |i: usize, message: &'static str| {
            (edited && invalid[i])
                .then(|| text(message, 12., 16., 430., t.text_error).into_any_element())
        };
        let mut rows: Vec<Div> = FIELDS
            .iter()
            .enumerate()
            .map(|(i, &(label, _, message))| {
                let control = if i == 0 && !self.npi_editable {
                    text(self.saved.npi.clone(), 13., 18., 430., t.text_secondary)
                        .into_any_element()
                } else {
                    div()
                        .w(px(240.))
                        .debug_selector(move || format!("medical-credentials-field-{i}"))
                        .child(fields[i].clone())
                        .into_any_element()
                };
                settings_row_content(label, error(i, message), control, t)
            })
            .collect();
        rows.push(settings_row_content(
            "I am a physician eligible for CME credits",
            error(4, "Physician eligibility confirmation is required"),
            checkbox("medical-credentials-eligible", self.eligible, "", t)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.eligible = !this.eligible;
                    cx.notify();
                }))
                .into_any_element(),
            t,
        ));
        let ready = self.can_save(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(card(rows, None, t))
            .child(
                div().flex().justify_end().child(
                    action(
                        "medical-credentials-save",
                        "Save",
                        Kind::Secondary,
                        !ready,
                        t,
                    )
                    .when(ready, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                    }),
                ),
            )
    }
}

impl Render for MedicalCredentials {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body = match (&self.load, &self.fields) {
            (Load::Hidden, _) => return div(),
            (Load::Loading, _) => status("Loading medical credentials…", false, &t),
            (Load::Failed, _) => settings_row_content(
                "Unable to load medical credentials",
                None,
                action(
                    "medical-credentials-retry",
                    "Try again",
                    Kind::Secondary,
                    false,
                    &t,
                )
                .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                .into_any_element(),
                &t,
            ),
            (Load::Ready, Some(fields)) => self.form(&fields.clone(), &t, cx),
            (Load::Ready, None) => div(),
        };
        div()
            .debug_selector(|| "medical-credentials".into())
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text("Medical credentials", 14., 21., 500., t.text))
                    .child(
                        text(
                            "Used for CME credits and other clinician features. Confirm physician eligibility to claim CME credits.",
                            13.,
                            18.,
                            430.,
                            t.text_secondary,
                        )
                        .whitespace_normal(),
                    ),
            )
            .child(body)
            .children(
                self.message
                    .clone()
                    .map(|(message, error)| status(message, error, &t)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> Credentials {
        Credentials {
            npi: " 123-456-7890 ".into(),
            degree: " MD ".into(),
            state: " de ".into(),
            number: " A123 ".into(),
            eligible: true,
        }
    }

    #[test]
    fn normalizes_like_the_original() {
        assert_eq!(
            filled().normalized(),
            Credentials {
                npi: "1234567890".into(),
                degree: "MD".into(),
                state: "DE".into(),
                number: "A123".into(),
                eligible: true,
            }
        );
        assert_eq!(filled().invalid(), [false; 5]);
    }

    #[test]
    fn each_check_fails_on_its_own() {
        type Edit = fn(&mut Credentials);
        let cases: [(Edit, usize); 6] = [
            (|c| c.npi = "123456789".into(), 0),
            (|c| c.degree = "  ".into(), 1),
            (|c| c.state = "D".into(), 2),
            (|c| c.state = "D1".into(), 2),
            (|c| c.number.clear(), 3),
            (|c| c.eligible = false, 4),
        ];
        for (change, field) in cases {
            let mut c = filled();
            change(&mut c);
            let mut expected = [false; 5];
            expected[field] = true;
            assert_eq!(c.invalid(), expected, "{c:?}");
        }
    }
}
