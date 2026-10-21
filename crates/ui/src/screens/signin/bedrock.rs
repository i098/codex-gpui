use super::{Command, Login, LoginEvent, Style};
use crate::{composer::Composer, icon::icon, text};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Profile,
    Environment,
    AccessKeys,
    ApiKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GovCloud {
    Clear,
    Warning,
    Unverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Detecting,
    Choices,
    Detected,
    Entry(Method),
    Finishing,
    GovCloud(GovCloud),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    name: String,
    region: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Discovery {
    profiles: Vec<Profile>,
    environment_credentials: Vec<Environment>,
}

#[derive(Deserialize)]
struct Environment {
    region: Option<String>,
}

pub(super) struct Bedrock {
    step: Step,
    profiles: Vec<Profile>,
    profile: usize,
    environment_region: Option<String>,
    environment: bool,
    discovery_supported: bool,
    fields: [Entity<Composer>; 6],
}

impl Bedrock {
    fn new(cx: &mut Context<Login>) -> Self {
        let fields = std::array::from_fn(|index| {
            let field = cx.new(|cx| {
                let mut field = Composer::settings_field("", "", false, cx);
                field.set_compact_field(cx);
                field.set_secret(matches!(index, 2 | 3 | 4), cx);
                field.set_field_label(
                    [
                        "AWS profile",
                        "AWS access key ID",
                        "AWS secret access key",
                        "AWS session token (optional)",
                        "Bedrock API key",
                        "AWS region",
                    ][index],
                    cx,
                );
                field
            });
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            field
        });
        Self {
            step: Step::Detecting,
            profiles: Vec::new(),
            profile: 0,
            environment_region: None,
            environment: false,
            discovery_supported: true,
            fields,
        }
    }

    fn field<'a>(&self, index: usize, cx: &'a gpui::App) -> &'a str {
        self.fields[index].read(cx).field_text()
    }

    fn request(&self, cx: &gpui::App) -> Option<(&'static str, Value)> {
        let Step::Entry(method) = self.step else {
            return None;
        };
        let region = self.field(5, cx).trim();
        if region.is_empty() {
            return None;
        }
        let key = self.field(0, cx).trim();
        Some(match method {
            Method::Profile if !key.is_empty() => (
                "account/bedrock/setup",
                json!({"type":"profile", "profile":key, "region":region}),
            ),
            Method::Environment => (
                "account/bedrock/setup",
                json!({"type":"environment", "region":region}),
            ),
            Method::AccessKeys
                if !self.field(1, cx).trim().is_empty() && !self.field(2, cx).trim().is_empty() =>
            {
                (
                    "account/login/start",
                    json!({"type":"amazonBedrockAccessKeys", "accessKeyId":self.field(1,cx), "secretAccessKey":self.field(2,cx), "sessionToken":(!self.field(3,cx).trim().is_empty()).then(||self.field(3,cx)), "region":region}),
                )
            }
            Method::ApiKey if !self.field(4, cx).trim().is_empty() => (
                "account/login/start",
                json!({"type":"amazonBedrock", "apiKey":self.field(4,cx), "region":region}),
            ),
            _ => return None,
        })
    }
}

impl Login {
    pub(super) fn open_bedrock(&mut self, cx: &mut Context<Self>) {
        if !self.policy.permits_api() || !self.options.bedrock || self.pending {
            return;
        }
        self.bedrock = Some(Bedrock::new(cx));
        self.error = None;
        self.discover_bedrock(cx);
    }

    fn discover_bedrock(&mut self, cx: &mut Context<Self>) {
        let Some(super::AppServer(bridge)) = cx.try_global::<super::AppServer>() else {
            return;
        };
        let bridge = bridge.clone();
        self.attempt += 1;
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let result = bridge
                .request::<_, Discovery>("account/bedrock/discover", json!({}))
                .await;
            this.update(cx, |this, cx| {
                if this.attempt != attempt {
                    return;
                }
                let Some(bedrock) = &mut this.bedrock else {
                    return;
                };
                match result {
                    Ok(discovery) => {
                        bedrock.profiles = discovery.profiles;
                        bedrock.environment = !discovery.environment_credentials.is_empty();
                        bedrock.environment_region = discovery
                            .environment_credentials
                            .into_iter()
                            .find_map(|entry| entry.region);
                        bedrock.step = if bedrock.profiles.is_empty() && !bedrock.environment {
                            Step::Choices
                        } else {
                            Step::Detected
                        };
                    }
                    Err(error) => {
                        let unsupported = error
                            .downcast_ref::<bridge::RpcError>()
                            .is_some_and(|error| error.code == -32601);
                        if unsupported {
                            bedrock.discovery_supported = false;
                            bedrock.step = Step::Entry(Method::ApiKey);
                        } else {
                            bedrock.step = Step::Choices;
                            this.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn choose_bedrock(
        &mut self,
        method: Method,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bedrock) = &mut self.bedrock else {
            return;
        };
        if self.pending || (!bedrock.discovery_supported && method != Method::ApiKey) {
            return;
        }
        if method == Method::AccessKeys && !self.supports_access_keys {
            return;
        }
        let region = if method == Method::Environment {
            bedrock.environment_region.clone()
        } else {
            bedrock
                .profiles
                .get(bedrock.profile)
                .and_then(|profile| profile.region.clone())
        };
        if method == Method::Profile {
            let name = bedrock
                .profiles
                .get(bedrock.profile)
                .map(|profile| profile.name.as_str())
                .unwrap_or_default();
            bedrock.fields[0].update(cx, |field, cx| field.set_text(name, cx));
        }
        bedrock.fields[5].update(cx, |field, cx| {
            field.set_text(region.as_deref().unwrap_or_default(), cx)
        });
        bedrock.step = Step::Entry(method);
        let focus = match method {
            Method::Profile => 0,
            Method::Environment => 5,
            Method::AccessKeys => 1,
            Method::ApiKey => 4,
        };
        bedrock.fields[focus].focus_handle(cx).focus(window, cx);
        self.error = None;
    }

    pub(super) fn submit_bedrock(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(bedrock) = &self.bedrock else {
            return;
        };
        let Some((method, params)) = bedrock.request(cx) else {
            return;
        };
        let region = params["region"].as_str().unwrap_or_default().to_owned();
        let Some(super::AppServer(bridge)) = cx.try_global::<super::AppServer>() else {
            return;
        };
        let bridge = bridge.clone();
        self.pending = true;
        self.error = None;
        self.restart_blocked = true;
        self.attempt += 1;
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let result = bridge.request::<_, Value>(method, params).await;
            this.update(cx, |this, cx| {
                if this.attempt != attempt {
                    return;
                }
                match result {
                    Ok(_) => {
                        if let Some(bedrock) = &mut this.bedrock {
                            bedrock.step = Step::Finishing;
                        }
                        cx.emit(LoginEvent::RestartBedrock { region });
                    }
                    Err(error) => {
                        this.pending = false;
                        this.error = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn bedrock_finished(&mut self, result: Result<GovCloud, String>, cx: &mut Context<Self>) {
        self.pending = false;
        match result {
            Ok(GovCloud::Clear) => cx.emit(LoginEvent::Authenticated),
            Ok(warning) => {
                if let Some(bedrock) = &mut self.bedrock {
                    bedrock.step = Step::GovCloud(warning);
                }
            }
            Err(error) => {
                self.error = Some(error);
                if let Some(bedrock) = &mut self.bedrock {
                    bedrock.step = Step::Choices;
                }
            }
        }
        cx.notify();
    }

    pub(super) fn bedrock_retry(&mut self, cx: &mut Context<Self>) {
        let Some(bedrock) = &self.bedrock else {
            return;
        };
        if self.pending || !matches!(bedrock.step, Step::GovCloud(_)) {
            return;
        }
        let region = bedrock.field(5, cx).to_owned();
        self.pending = true;
        cx.emit(LoginEvent::CheckGovCloud { region });
        cx.notify();
    }

    pub(super) fn bedrock_back(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.error = None;
        let Some(bedrock) = &mut self.bedrock else {
            return;
        };
        if matches!(bedrock.step, Step::Entry(_)) && bedrock.discovery_supported {
            bedrock.step = Step::Choices;
        } else {
            self.bedrock = None;
            self.attempt += 1;
        }
        cx.notify();
    }

    pub(super) fn bedrock_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let t = super::login_theme(cx);
        let bedrock = self.bedrock.as_ref().expect("Bedrock view requires state");
        let mut body = div().w_full().flex().flex_col().gap(px(12.));
        if let Some(error) = &self.error {
            body = body
                .child(text(error.clone(), 14., 20., 400., t.text_description).whitespace_normal());
        }
        match bedrock.step {
            Step::Detecting => {
                body = body.child(text(
                    "Checking for AWS credentials…",
                    14.,
                    20.,
                    400.,
                    t.text_description,
                ))
            }
            Step::Finishing => {
                body = body.child(text(
                    "Finishing Amazon Bedrock sign-in…",
                    14.,
                    20.,
                    400.,
                    t.text_description,
                ))
            }
            Step::GovCloud(warning) => {
                let message = if warning == GovCloud::Warning {
                    "You are using Codex with AWS GovCloud. Please ensure you or your administrator have read the application configuration and security guidance before proceeding."
                } else {
                    "Codex couldn’t complete the AWS GovCloud check. If you are using AWS GovCloud, please ensure you or your administrator have read the application configuration and security guidance before proceeding."
                };
                body=body.child(text(message,14.,20.,400.,t.text_description).whitespace_normal())
                    .child(div().id("govcloud-guidance").child(text("Application configuration and security guidance",14.,20.,400.,t.text).underline()).on_click(|_,_,cx|cx.open_url("https://learn.chatgpt.com/docs/enterprise/govcloud-configuration")))
                    .child(self.button("bedrock-finish","Continue",Command::BedrockFinish,Style::Primary,window,cx));
                if warning == GovCloud::Unverified {
                    body = body.child(self.button(
                        "bedrock-retry",
                        "Retry",
                        Command::BedrockRetry,
                        Style::Outline,
                        window,
                        cx,
                    ));
                }
            }
            Step::Detected => {
                for (index, profile) in bedrock.profiles.iter().enumerate() {
                    body = body.child(
                        div()
                            .id(("aws-profile", index))
                            .p(px(12.))
                            .border_1()
                            .border_color(t.border)
                            .child(text(profile.name.clone(), 14., 20., 500., t.text))
                            .children(profile.region.clone().map(|region| {
                                text(
                                    format!("Region: {region}"),
                                    12.,
                                    18.,
                                    400.,
                                    t.text_description,
                                )
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(bedrock) = &mut this.bedrock {
                                    bedrock.profile = index;
                                }
                                this.choose_bedrock(Method::Profile, window, cx);
                            })),
                    );
                }
                if bedrock.environment {
                    body = body.child(self.button(
                        "aws-environment",
                        "Continue with detected credentials",
                        Command::BedrockMethod(Method::Environment),
                        Style::Primary,
                        window,
                        cx,
                    ));
                }
                body = body.child(self.button(
                    "aws-other-methods",
                    "Other AWS sign-in methods",
                    Command::BedrockChoices,
                    Style::Outline,
                    window,
                    cx,
                ));
            }
            Step::Choices => {
                if bedrock.profiles.is_empty() && !bedrock.environment && self.error.is_none() {
                    body = body.child(text(
                        "No AWS credentials found",
                        14.,
                        20.,
                        400.,
                        t.text_description,
                    ));
                }
                if bedrock.discovery_supported {
                    body = body.child(self.button(
                        "aws-profile-method",
                        "AWS profile",
                        Command::BedrockMethod(Method::Profile),
                        Style::Outline,
                        window,
                        cx,
                    ));
                    if self.supports_access_keys {
                        body = body.child(self.button(
                            "aws-access-keys",
                            "AWS access keys",
                            Command::BedrockMethod(Method::AccessKeys),
                            Style::Outline,
                            window,
                            cx,
                        ));
                    }
                }
                body = body.child(self.button(
                    "bedrock-api-key",
                    "Bedrock API key",
                    Command::BedrockMethod(Method::ApiKey),
                    Style::Outline,
                    window,
                    cx,
                ));
            }
            Step::Entry(method) => {
                let fields: &[(usize, &str)] = match method {
                    Method::Profile => &[(0, "AWS profile"), (5, "AWS region")],
                    Method::Environment => &[(5, "AWS region")],
                    Method::AccessKeys => &[
                        (1, "AWS access key ID"),
                        (2, "AWS secret access key"),
                        (3, "AWS session token (optional)"),
                        (5, "AWS region"),
                    ],
                    Method::ApiKey => &[(4, "Bedrock API key"), (5, "AWS region")],
                };
                if method == Method::Environment {
                    body = body.child(
                        text(
                            "AWS credentials detected. Enter your AWS Region to continue.",
                            14.,
                            20.,
                            400.,
                            t.text_description,
                        )
                        .whitespace_normal(),
                    );
                }
                for &(index, label) in fields {
                    body = body.child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(text(label, 14., 20., 500., t.text))
                            .child(bedrock.fields[index].clone()),
                    );
                }
                body = body
                    .child(text(
                        "For example, us-east-2",
                        12.,
                        18.,
                        400.,
                        t.text_description,
                    ))
                    .child(
                        self.button(
                            "bedrock-continue",
                            "Continue",
                            Command::BedrockSubmit,
                            Style::Primary,
                            window,
                            cx,
                        )
                        .when(self.pending || bedrock.request(cx).is_none(), |button| {
                            button.opacity(0.5)
                        }),
                    );
            }
        }
        if !matches!(bedrock.step, Step::Finishing | Step::GovCloud(_)) {
            body = body.child(self.button(
                "bedrock-back",
                "Back",
                Command::BedrockBack,
                Style::Link,
                window,
                cx,
            ));
        }
        div()
            .w_full()
            .h_full()
            .id("bedrock-setup")
            .capture_action(cx.listener(|this, _: &crate::composer::Submit, _, cx| {
                if this
                    .bedrock
                    .as_ref()
                    .is_some_and(|bedrock| matches!(bedrock.step, Step::Entry(_)))
                {
                    this.submit_bedrock(cx);
                    cx.stop_propagation();
                }
            }))
            .overflow_y_scroll()
            .py(px(48.))
            .px(px(24.))
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(340.))
                    .flex()
                    .flex_col()
                    .gap(px(32.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(16.))
                            .child(icon("signin-chatgpt", 36., t.text))
                            .child(
                                text("Set up Amazon Bedrock", 28., 36., 400., t.text)
                                    .w(px(316.))
                                    .text_center(),
                            ),
                    )
                    .child(body),
            )
    }

    pub(super) fn bedrock_choices(&mut self, cx: &mut Context<Self>) {
        if !self.pending {
            if let Some(bedrock) = &mut self.bedrock {
                bedrock.step = Step::Choices;
            }
            self.error = None;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_session_tokens_are_null_without_trimming_credentials() {
        let mut app = gpui::HeadlessAppContext::new(textsys::text_system());
        let login = app.new(Login::new);
        login.update(&mut app, |_, cx| {
            let mut bedrock = Bedrock::new(cx);
            bedrock.step = Step::Entry(Method::AccessKeys);
            for (index, value) in [
                (1, " access-id "),
                (2, " secret "),
                (3, "   "),
                (5, " us-east-2 "),
            ] {
                bedrock.fields[index].update(cx, |field, cx| field.set_text(value, cx));
            }
            let (_, request) = bedrock.request(cx).unwrap();
            assert_eq!(request["accessKeyId"], " access-id ");
            assert_eq!(request["secretAccessKey"], " secret ");
            assert_eq!(request["sessionToken"], Value::Null);
            assert_eq!(request["region"], "us-east-2");
            bedrock.fields[3].update(cx, |field, cx| field.set_text(" token ", cx));
            assert_eq!(bedrock.request(cx).unwrap().1["sessionToken"], " token ");
        });
    }
}
