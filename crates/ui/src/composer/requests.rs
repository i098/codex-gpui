use super::*;
use gpui::AppContext as _;
use serde_json::json;

#[derive(Clone, Debug)]
pub(super) struct Form {
    method: String,
    params: Value,
    fields: Vec<Value>,
    unsupported: bool,
}

impl Approval {
    pub fn form(id: Value, method: &str, params: &Value) -> Option<Self> {
        params.as_object()?;
        let (title, fields, unsupported) = match method {
            "item/tool/requestUserInput" => {
                params["turnId"].as_str()?;
                params["itemId"].as_str()?;
                let fields = params["questions"].as_array()?;
                if fields.is_empty() {
                    return None;
                }
                ("Answer the questions".to_owned(), fields.clone(), false)
            }
            "item/tool/call" => match params["tool"].as_str()? {
                "request_onboarding_input" => {
                    let mut fields = params["arguments"]["questions"].as_array()?.clone();
                    if !(1..=3).contains(&fields.len()) {
                        return None;
                    }
                    for field in &mut fields {
                        field.as_object()?;
                        if field["options"].as_array()?.len() < 2 {
                            return None;
                        }
                        field["isOther"] = json!(true);
                        field["isSecret"] = json!(false);
                    }
                    ("Answer the questions".to_owned(), fields, false)
                }
                "request_option_picker" => {
                    let mut field = params["arguments"].clone();
                    field.as_object()?;
                    field["options"].as_array()?;
                    field["id"] = "selection".into();
                    (field["question"].as_str()?.to_owned(), vec![field], false)
                }
                _ => return None,
            },
            "mcpServer/elicitation/request" => {
                if params["mode"] == "url" {
                    let url = params["url"].as_str()?;
                    if !url.starts_with("https://") && !url.starts_with("http://") {
                        return None;
                    }
                    return Some(Self {
                        title: params["message"].as_str()?.to_owned().into(),
                        detail: Some(url.to_owned()),
                        reason: params["serverName"].as_str().map(str::to_owned),
                        scoped_label: "".into(),
                        permissions: None,
                        identity: Rc::new(RequestIdentity {
                            id,
                            thread: params["threadId"].as_str()?.to_owned(),
                        }),
                        computer_use: None,
                        form: Some(Form {
                            method: method.to_owned(),
                            params: params.clone(),
                            fields: vec![],
                            unsupported: false,
                        }),
                    });
                }
                let schema = &params["requestedSchema"];
                let supported = matches!(
                    params["mode"].as_str(),
                    Some("form" | "openai/form" | "openaiForm")
                ) && schema["type"] == "object"
                    && schema["properties"].is_object();
                let mut fields = Vec::new();
                let mut unsupported = !supported;
                if supported {
                    for (name, schema) in schema["properties"].as_object()? {
                        if !schema.is_object() {
                            unsupported = true;
                            continue;
                        }
                        let mut field = schema.clone();
                        field["id"] = name.clone().into();
                        field["question"] = schema["title"].as_str().unwrap_or(name).into();
                        field["required"] = json!(
                            params["requestedSchema"]["required"]
                                .as_array()
                                .is_some_and(|required| required.contains(&json!(name)))
                        );
                        unsupported |= !matches!(
                            schema["type"].as_str(),
                            Some("string" | "number" | "integer" | "boolean" | "array")
                        ) || schema.get("x-openai-input").is_some()
                            || (schema["type"] == "array"
                                && schema["items"]["enum"].as_array().is_none()
                                && schema["items"]["anyOf"].as_array().is_none());
                        fields.push(field);
                    }
                }
                (
                    if unsupported {
                        "This form isn’t supported in this version".to_owned()
                    } else {
                        params["message"].as_str().unwrap_or_default().to_owned()
                    },
                    fields,
                    unsupported,
                )
            }
            _ => return None,
        };
        if method != "mcpServer/elicitation/request"
            && fields.iter().any(|field| {
                !field.is_object()
                    || field["id"].as_str().is_none()
                    || field["question"].as_str().is_none()
                    || field["options"].as_array().is_some_and(|options| {
                        options
                            .iter()
                            .any(|option| option["label"].as_str().is_none())
                    })
            })
        {
            return None;
        }
        if method != "mcpServer/elicitation/request" && fields.len() > 1 {
            let mut ids = std::collections::HashSet::new();
            if fields
                .iter()
                .any(|field| !ids.insert(field["id"].as_str().unwrap()))
            {
                return None;
            }
        }
        Some(Self {
            title: title.into(),
            detail: None,
            reason: params["serverName"].as_str().map(str::to_owned),
            scoped_label: "".into(),
            permissions: None,
            identity: Rc::new(RequestIdentity {
                id,
                thread: params["threadId"].as_str()?.to_owned(),
            }),
            computer_use: None,
            form: Some(Form {
                method: method.to_owned(),
                params: params.clone(),
                fields,
                unsupported,
            }),
        })
    }
}

impl Form {
    fn picker(&self) -> bool {
        self.method == "item/tool/call" && self.params["tool"] == "request_option_picker"
    }

    fn choices(field: &Value) -> Vec<(String, Value)> {
        if field["type"] == "boolean" {
            return vec![("Yes".into(), json!(true)), ("No".into(), json!(false))];
        }
        let schema = if field["type"] == "array" {
            &field["items"]
        } else {
            field
        };
        if let Some(options) = schema["enum"].as_array() {
            return options
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    (
                        schema["enumNames"][i]
                            .as_str()
                            .or(value.as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        value.clone(),
                    )
                })
                .collect();
        }
        if let Some(options) = schema["oneOf"]
            .as_array()
            .or_else(|| schema["anyOf"].as_array())
        {
            return options
                .iter()
                .map(|option| {
                    (
                        option["title"]
                            .as_str()
                            .or(option["const"].as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        option["const"].clone(),
                    )
                })
                .collect();
        }
        field["options"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|option| {
                let label = option["label"].as_str().unwrap_or_default();
                (label.to_owned(), json!(label))
            })
            .collect()
    }

    fn single_choice(field: &Value) -> bool {
        field["type"] == "string"
            && ["enum", "oneOf", "anyOf"]
                .iter()
                .any(|key| field[*key].is_array())
    }

    fn field_value(
        &self,
        i: usize,
        inputs: &[Entity<Composer>],
        selected: &[Vec<Value>],
        cx: &App,
    ) -> Result<Option<Value>, String> {
        let field = &self.fields[i];
        let raw = inputs[i].read(cx).field_text();
        if self.method == "mcpServer/elicitation/request" {
            if Self::single_choice(field) && selected[i].is_empty() {
                return if field["required"] == true {
                    Err("Choose a value".into())
                } else {
                    Ok(None)
                };
            }
            let value = if field["type"] == "array" {
                if selected[i].is_empty()
                    && field["required"] != true
                    && field["default"].as_array().is_none_or(Vec::is_empty)
                {
                    return Ok(None);
                }
                json!(selected[i])
            } else if let Some(value) = selected[i].first() {
                value.clone()
            } else if raw.is_empty()
                && field["required"] != true
                && field["default"].as_str().is_none_or(str::is_empty)
            {
                return Ok(None);
            } else if matches!(field["type"].as_str(), Some("number" | "integer")) {
                let text = raw.trim();
                if let Ok(number) = text.parse::<i64>() {
                    json!(number)
                } else if let Ok(number) = text.parse::<u64>() {
                    json!(number)
                } else {
                    let number: f64 = text.parse().map_err(|_| "Enter a number".to_owned())?;
                    if !number.is_finite() {
                        return Err("Enter a finite number".into());
                    }
                    if field["type"] == "integer" {
                        if number.fract() != 0. {
                            return Err("Enter a whole number".into());
                        }
                        if number >= i64::MIN as f64 && number < i64::MAX as f64 {
                            json!(number as i64)
                        } else if number >= 0. && number < u64::MAX as f64 {
                            json!(number as u64)
                        } else {
                            json!(number)
                        }
                    } else {
                        json!(number)
                    }
                }
            } else if field["type"] == "boolean" {
                return Err("Choose a value".into());
            } else {
                json!(raw)
            };
            validate(field, &value)?;
            return Ok(Some(value));
        }
        let text = raw.trim();
        let answer = if field["isOther"] == true && !text.is_empty() {
            text
        } else {
            selected[i].first().and_then(Value::as_str).unwrap_or(text)
        };
        let answer = answer.trim();
        Ok((!answer.is_empty()).then(|| json!(answer)))
    }

    fn response(
        &self,
        decision: ApprovalDecision,
        inputs: &[Entity<Composer>],
        selected: &[Vec<Value>],
        cx: &App,
    ) -> Result<Value, (usize, String)> {
        let accept = decision == ApprovalDecision::Accept;
        if self.method == "mcpServer/elicitation/request" {
            let action = if accept && !self.unsupported {
                "accept"
            } else if decision == ApprovalDecision::Cancel {
                "cancel"
            } else {
                "decline"
            };
            let mut content = serde_json::Map::new();
            if action == "accept" {
                for (i, field) in self.fields.iter().enumerate() {
                    if let Some(value) = self
                        .field_value(i, inputs, selected, cx)
                        .map_err(|error| (i, error))?
                    {
                        content.insert(field["id"].as_str().unwrap_or_default().to_owned(), value);
                    }
                }
            }
            return Ok(
                json!({ "action": action, "content": if action == "accept" { Value::Object(content) } else { Value::Null }, "_meta": null }),
            );
        }
        if self.picker() {
            let freeform = inputs[0].read(cx).field_text().trim();
            if accept && freeform.is_empty() && selected[0].is_empty() {
                return Err((0, "Select an option or type an answer".into()));
            }
            let dismissed = decision == ApprovalDecision::Cancel;
            return Ok(tool_response(json!({
                "action": if accept { "submit" } else if dismissed { "dismiss" } else { "skip" },
                "selectedOptions": if dismissed { vec![] } else { selected[0].clone() },
                "freeformAnswer": if !dismissed && !freeform.is_empty() { Some(freeform) } else { None }
            })));
        }
        if decision == ApprovalDecision::Cancel && self.method == "item/tool/requestUserInput" {
            return Ok(if self.params["isBlocking"] == false {
                json!({ "answers": {} })
            } else {
                Value::Null
            });
        }
        let mut answers = serde_json::Map::new();
        if accept {
            for (i, field) in self.fields.iter().enumerate() {
                if let Some(answer) = self
                    .field_value(i, inputs, selected, cx)
                    .map_err(|error| (i, error))?
                {
                    answers.insert(
                        field["id"].as_str().unwrap_or_default().to_owned(),
                        json!({ "answers": [answer] }),
                    );
                }
            }
        }
        let response = json!({ "answers": answers });
        Ok(if self.method == "item/tool/call" {
            tool_response(response)
        } else {
            response
        })
    }
}

fn tool_response(value: Value) -> Value {
    json!({ "contentItems": [{ "type": "inputText", "text": value.to_string() }], "success": true })
}

fn validate(field: &Value, value: &Value) -> Result<(), String> {
    let choices = Form::choices(field);
    if !choices.is_empty() {
        let valid = if let Some(items) = value.as_array() {
            items
                .iter()
                .all(|item| choices.iter().any(|(_, choice)| choice == item))
        } else {
            choices.iter().any(|(_, choice)| choice == value)
        };
        if !valid {
            return Err("Select one of the available answers".into());
        }
    }
    let valid = if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        field["minLength"].as_u64().is_none_or(|min| length >= min)
            && field["maxLength"].as_u64().is_none_or(|max| length <= max)
            && match field["format"].as_str() {
                Some("email") => text.split_once('@').is_some_and(|(local, host)| {
                    !local.is_empty()
                        && host.contains('.')
                        && !host.contains('@')
                        && !text.chars().any(char::is_whitespace)
                }),
                Some("uri") => text.split_once(':').is_some_and(|(scheme, rest)| {
                    !rest.is_empty()
                        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                        && scheme
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                        && !text.chars().any(char::is_whitespace)
                }),
                Some("date") => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
                Some("date-time") => chrono::DateTime::parse_from_rfc3339(text).is_ok(),
                _ => true,
            }
    } else if let Some(number) = value.as_f64() {
        let integer = |value: &Value| {
            value
                .as_i64()
                .map(i128::from)
                .or_else(|| value.as_u64().map(i128::from))
        };
        let minimum = match (integer(value), integer(&field["minimum"])) {
            (Some(value), Some(minimum)) => value >= minimum,
            _ => field["minimum"]
                .as_f64()
                .is_none_or(|minimum| number >= minimum),
        };
        let maximum = match (integer(value), integer(&field["maximum"])) {
            (Some(value), Some(maximum)) => value <= maximum,
            _ => field["maximum"]
                .as_f64()
                .is_none_or(|maximum| number <= maximum),
        };
        minimum && maximum
    } else if let Some(items) = value.as_array() {
        field["minItems"]
            .as_u64()
            .is_none_or(|min| items.len() as u64 >= min)
            && field["maxItems"]
                .as_u64()
                .is_none_or(|max| items.len() as u64 <= max)
    } else {
        value.is_boolean()
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Check the answer for {}",
            field["question"].as_str().unwrap_or_default()
        ))
    }
}

impl Composer {
    pub(super) fn request_listener<E: ?Sized>(
        &self,
        cx: &Context<Self>,
        listener: impl Fn(&mut Self, &E, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let identity = self
            .approval
            .as_ref()
            .map(|approval| approval.identity.clone());
        cx.listener(move |this, event, window, cx| {
            if this
                .approval
                .as_ref()
                .map(|approval| approval.identity.as_ref())
                == identity.as_deref()
            {
                listener(this, event, window, cx);
            }
        })
    }

    pub(super) fn prepare_request(&mut self, cx: &mut Context<Self>) {
        self.request_draft = RequestDraft::default();
        self.request_focus_pending = true;
        self.menu = None;
        let Some(form) = self
            .approval
            .as_ref()
            .and_then(|approval| approval.form.as_ref())
        else {
            return;
        };
        let approval = self.approval.as_ref().unwrap();
        if let Some(index) = self
            .request_drafts
            .iter()
            .position(|(identity, _)| *identity == approval.identity)
        {
            self.request_draft = self.request_drafts.swap_remove(index).1;
            return;
        }
        let parent = (cx.entity().downgrade(), approval.identity.clone());
        for (index, field) in form.fields.iter().enumerate() {
            let choices = Form::choices(field);
            let value =
                if form.method == "mcpServer/elicitation/request" && Form::single_choice(field) {
                    String::new()
                } else {
                    field["default"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if field["default"].is_number() {
                                field["default"].to_string()
                            } else {
                                String::new()
                            }
                        })
                };
            let editor = cx.new(|cx| {
                let placeholder = if form.picker() {
                    "Something else"
                } else if form.method == "mcpServer/elicitation/request" {
                    ""
                } else {
                    "Type here"
                };
                let mut editor = Composer::settings_field(placeholder, &value, false, cx);
                editor.request_label = Some(
                    field["question"]
                        .as_str()
                        .unwrap_or("Answer")
                        .to_owned()
                        .into(),
                );
                editor.secret = field["isSecret"] == true;
                editor.request_compact = editor.secret
                    || form.picker()
                    || form.method == "mcpServer/elicitation/request";
                editor.request_parent = Some((parent.0.clone(), parent.1.clone(), index));
                editor
            });
            cx.observe(&editor, |_, _, cx| cx.notify()).detach();
            self.request_draft.fields.push(editor);
            self.request_draft
                .selected
                .push(if field["type"] == "array" {
                    field["default"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|value| choices.iter().any(|(_, choice)| choice == *value))
                        .cloned()
                        .collect()
                } else if field["type"] == "boolean"
                    && choices.iter().any(|(_, value)| *value == field["default"])
                {
                    vec![field["default"].clone()]
                } else {
                    vec![]
                });
        }
    }

    pub fn retain_request_drafts(&mut self, mut keep: impl FnMut(&Value, &str) -> bool) {
        // ponytail: Draft pruning is O(n²); index identities if many requests stay pending.
        self.request_drafts
            .retain(|(identity, _)| keep(&identity.id, &identity.thread));
    }

    pub(super) fn focus_request_field(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self
            .approval
            .as_ref()
            .and_then(|approval| approval.form.as_ref())
            .filter(|form| !form.unsupported)
            .and_then(|form| {
                form.fields.get(self.request_draft.index).filter(|field| {
                    form.picker()
                        || field["isOther"] == true
                        || (field["type"] != "boolean"
                            && field["type"] != "array"
                            && !Form::single_choice(field)
                            && field["options"].as_array().is_none_or(Vec::is_empty))
                })
            })
            .and_then(|_| self.request_draft.fields.get(self.request_draft.index));
        if let Some(input) = input {
            input.focus_handle(cx).focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
    }

    pub(super) fn request_card(
        &self,
        approval: &Approval,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let form = approval.form.as_ref().unwrap();
        let field = form.fields.get(self.request_draft.index);
        let mcp = form.method == "mcpServer/elicitation/request";
        let single_choice = mcp && field.is_some_and(Form::single_choice);
        let user_input = !mcp && !form.picker();
        let next = self.request_draft.index + 1 < form.fields.len();
        let empty = field.is_some_and(|_| {
            self.request_draft.fields[self.request_draft.index]
                .read(cx)
                .field_text()
                .trim()
                .is_empty()
                && self.request_draft.selected[self.request_draft.index].is_empty()
        });
        let title: SharedString = if user_input && !form.unsupported {
            field
                .and_then(|field| field["question"].as_str())
                .unwrap_or_default()
                .to_owned()
                .into()
        } else {
            approval.title.clone()
        };
        let title = div().id("request-title").flex_1().min_w_0().child(text(
            title,
            if form.picker() { 13. } else { 14. },
            20.,
            500.,
            t.text,
        ));
        let title = if let Some(server) = &approval.reason {
            crate::tooltip::tooltip(
                title.aria_description(format!("From {server}")),
                format!("From {server}"),
                None,
                crate::tooltip::Side::Top,
            )
            .into_any_element()
        } else {
            title.into_any_element()
        };
        let close = crate::icon_button("request-close", icon("close", 12., t.text_secondary), t)
            .size(px(if mcp {
                20.
            } else if user_input {
                26.
            } else {
                24.
            }))
            .role(gpui::Role::Button)
            .aria_label(
                if form.method == "item/tool/requestUserInput" && form.params["isBlocking"] != false
                {
                    "Stop task"
                } else {
                    "Cancel"
                },
            )
            .focusable()
            .tab_index(0)
            .on_key_down(self.request_listener(
                cx,
                |this, event: &gpui::KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space")
                        && event.keystroke.modifiers == gpui::Modifiers::default()
                    {
                        this.request_action(ApprovalDecision::Cancel, window, cx);
                        cx.stop_propagation();
                    }
                },
            ))
            .on_click(self.request_listener(cx, |this, _, window, cx| {
                this.request_action(ApprovalDecision::Cancel, window, cx)
            }));
        let mut controls = div().flex().items_center().gap(px(8.));
        if form.fields.len() > 1 {
            for (id, label, glyph, target, enabled) in [
                (
                    "request-previous",
                    "Previous question",
                    "‹",
                    self.request_draft.index.saturating_sub(1),
                    self.request_draft.index > 0,
                ),
                (
                    "request-next",
                    "Next question",
                    "›",
                    (self.request_draft.index + 1).min(form.fields.len() - 1),
                    next,
                ),
            ] {
                if id == "request-next" {
                    controls = controls.child(text(
                        format!("{} of {}", self.request_draft.index + 1, form.fields.len()),
                        12.,
                        20.,
                        400.,
                        t.text_secondary,
                    ));
                }
                controls = controls.child(
                    div()
                        .id(id)
                        .size(px(20.))
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .aria_disabled(!enabled)
                        .opacity(if enabled { 1. } else { 0.35 })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(text(glyph, 18., 20., 400., t.text_secondary))
                        .when(enabled, |d| {
                            d.focusable()
                                .tab_index(0)
                                .on_key_down(self.request_listener(
                                    cx,
                                    move |this, event: &gpui::KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                            && event.keystroke.modifiers
                                                == gpui::Modifiers::default()
                                        {
                                            this.request_draft.index = target;
                                            this.request_draft.error = None;
                                            this.request_focus_pending = true;
                                            this.focus_request_field(window, cx);
                                            cx.notify();
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                                .on_click(self.request_listener(cx, move |this, _, window, cx| {
                                    this.request_draft.index = target;
                                    this.request_draft.error = None;
                                    this.request_focus_pending = true;
                                    this.focus_request_field(window, cx);
                                    cx.notify();
                                }))
                        }),
                );
            }
        }
        let header = div()
            .pl(px(if mcp { 20. } else { 16. }))
            .pr(px(12.))
            .pt(px(16.))
            .pb(px(if user_input { 8. } else { 12. }))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(title)
                    .child(controls.child(close)),
            )
            .when(mcp, |d| {
                d.when_some(
                    field.filter(|field| field["type"] != "boolean"),
                    |d, field| {
                        d.child(text(
                            field["question"].as_str().unwrap_or_default().to_owned(),
                            14.,
                            20.,
                            430.,
                            t.text_secondary,
                        ))
                    },
                )
            });
        let mut body = div()
            .id("request-fields")
            .max_h(px(320.))
            .overflow_y_scroll()
            .px(px(if form.picker() { 12. } else { 16. }))
            .pt(px(if user_input { 8. } else { 0. }))
            .pb(px(if mcp {
                20.
            } else if form.picker() {
                16.
            } else {
                12.
            }));
        if !form.unsupported
            && let Some(field) = field
        {
            let i = self.request_draft.index;
            let boolean = field["type"] == "boolean";
            let choices = if boolean {
                vec![(
                    field["question"].as_str().unwrap_or_default().to_owned(),
                    json!(true),
                )]
            } else {
                Form::choices(field)
            };
            let mut row = div()
                .id("request-question")
                .role(gpui::Role::Group)
                .aria_label(field["question"].as_str().unwrap_or_default().to_owned())
                .flex()
                .flex_col()
                .gap(px(8.));
            let mut options = div()
                .flex()
                .gap(px(8.))
                .when(!form.picker(), |d| d.flex_col())
                .when(form.picker(), |d| d.flex_wrap());
            for (j, (label, value)) in choices.iter().enumerate() {
                let checked = self.request_draft.selected[i].contains(value);
                let value = value.clone();
                let multiple = field["type"] == "array" || field["allowMultiple"] == true;
                let description = field["options"][j]["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                options = options.child(
                    div()
                        .id(ElementId::Name(format!("request-choice-{i}-{j}").into()))
                        .px(px(if boolean {
                            4.
                        } else if form.picker() {
                            12.
                        } else {
                            8.
                        }))
                        .py(px(6.))
                        .rounded(px(if form.picker() { 999. } else { 8. }))
                        .when(!boolean, |d| d.border_1().border_color(t.border))
                        .when(boolean, |d| d.bg(t.text_alpha_5).rounded(px(10.)))
                        .when(checked, |d| d.bg(t.button_secondary))
                        .focusable()
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .aria_label(label.clone())
                        .aria_selected(checked)
                        .cursor_pointer()
                        .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .when(!form.picker(), |d| {
                                    d.child(
                                        div()
                                            .size(px(if boolean { 20. } else { 16. }))
                                            .border_1()
                                            .border_color(t.border)
                                            .when(checked, |d| d.bg(t.text))
                                            .rounded(px(if boolean || multiple {
                                                4.
                                            } else {
                                                999.
                                            }))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(text(
                                                if checked {
                                                    "✓".to_owned()
                                                } else if boolean || multiple {
                                                    String::new()
                                                } else {
                                                    (j + 1).to_string()
                                                },
                                                10.,
                                                14.,
                                                500.,
                                                if checked { t.text_inverse } else { t.text },
                                            )),
                                    )
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(text(
                                            label.clone(),
                                            13.,
                                            20.,
                                            if boolean { 500. } else { 430. },
                                            t.text,
                                        ))
                                        .when(!description.is_empty() && !form.picker(), |d| {
                                            d.child(text(
                                                description,
                                                12.,
                                                18.,
                                                400.,
                                                t.text_secondary,
                                            ))
                                        }),
                                ),
                        )
                        .on_key_down(self.request_listener(cx, {
                            let value = value.clone();
                            move |this, event: &gpui::KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                    && event.keystroke.modifiers == gpui::Modifiers::default()
                                {
                                    this.select_request_choice(i, &value, multiple, window, cx);
                                    cx.stop_propagation();
                                }
                            }
                        }))
                        .on_click(self.request_listener(cx, move |this, _, window, cx| {
                            this.select_request_choice(i, &value, multiple, window, cx)
                        })),
                );
            }
            row = row.when(!choices.is_empty(), |d| d.child(options));
            if choices.is_empty() || field["isOther"] == true || form.picker() {
                row = row.child(
                    div()
                        .on_mouse_down(
                            MouseButton::Left,
                            self.request_listener(cx, move |this, _, _, cx| {
                                if !this.request_draft.selected[i].is_empty() {
                                    this.request_draft.selected[i].clear();
                                    cx.notify();
                                }
                            }),
                        )
                        .child(self.request_draft.fields[i].clone()),
                );
            }
            body = body.child(row);
        }
        if form.params["mode"] == "url" {
            let url = form.params["url"].as_str().unwrap_or_default().to_owned();
            body = body.child(
                div()
                    .id("request-open-url")
                    .role(gpui::Role::Button)
                    .aria_label("Open link")
                    .focusable()
                    .tab_index(0)
                    .cursor_pointer()
                    .child(text(url.clone(), 14., 20., 400., t.text))
                    .on_click(self.request_listener(cx, move |_, _, _, cx| cx.open_url(&url))),
            );
        }
        let submit: SharedString = if form.picker() {
            form.params["arguments"]["submitLabel"]
                .as_str()
                .unwrap_or("Submit")
                .to_owned()
                .into()
        } else if user_input && empty {
            "Skip".into()
        } else if next {
            "Next".into()
        } else if user_input {
            "Submit".into()
        } else {
            "Continue".into()
        };
        let footer = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .px(px(if form.picker() { 12. } else { 16. }))
            .pt(px(if form.picker() { 0. } else { 8. }))
            .pb(px(if mcp { 8. } else { 12. }))
            .when(mcp, |d| d.border_t_1().border_color(t.border))
            .when(
                !user_input
                    && (!mcp
                        || next
                        || (single_choice && field.is_some_and(|field| field["required"] != true))
                        || form.unsupported
                        || form.params["mode"] == "url"),
                |d| {
                    d.child(self.request_button(
                        "request-decline",
                        if mcp && next {
                            "Skip for now".into()
                        } else {
                            form.params["arguments"]["skipLabel"]
                                .as_str()
                                .unwrap_or("Skip")
                                .to_owned()
                                .into()
                        },
                        if single_choice && !next {
                            ApprovalDecision::Accept
                        } else {
                            ApprovalDecision::Decline
                        },
                        false,
                        t,
                        cx,
                    ))
                },
            )
            .when(!form.unsupported && !single_choice, |d| {
                d.child(self.request_button(
                    "request-submit",
                    submit,
                    ApprovalDecision::Accept,
                    !(user_input && empty),
                    t,
                    cx,
                ))
            });
        div()
            .rounded(px(25.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface_elevated_secondary)
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .when_some(self.request_draft.error.clone(), |d, error| {
                d.child(
                    div()
                        .id("request-error")
                        .role(gpui::Role::Alert)
                        .px(px(16.))
                        .pb(px(8.))
                        .child(text(error, 13., 18., 400., t.text_error)),
                )
            })
            .child(footer)
    }

    pub(super) fn request_button(
        &self,
        id: &'static str,
        label: SharedString,
        decision: ApprovalDecision,
        primary: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let enabled = decision != ApprovalDecision::Accept
            || self
                .approval
                .as_ref()
                .and_then(|a| a.form.as_ref())
                .is_none_or(|form| {
                    !form.picker()
                        || !self.request_draft.selected[0].is_empty()
                        || !self.request_draft.fields[0]
                            .read(cx)
                            .field_text()
                            .trim()
                            .is_empty()
                });
        let native = self
            .approval
            .as_ref()
            .is_some_and(|approval| approval.computer_use.is_some());
        let permission = self
            .approval
            .as_ref()
            .is_some_and(|approval| approval.permissions.is_some());
        let user_input = self
            .approval
            .as_ref()
            .and_then(|a| a.form.as_ref())
            .is_some_and(|form| form.method == "item/tool/requestUserInput");
        let form_submit = primary
            && self
                .approval
                .as_ref()
                .and_then(|approval| approval.form.as_ref())
                .is_some_and(|form| form.method == "mcpServer/elicitation/request");
        div()
            .id(id)
            .relative()
            .h(px(28.))
            .px(px(if native { 8. } else { 10. }))
            .when(!native, |d| d.rounded(px(999.)))
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .aria_disabled(!enabled)
            .opacity(if enabled { 1. } else { 0.5 })
            .flex()
            .items_center()
            .justify_center()
            .gap(px(if native { 4. } else { 6. }))
            .focus_visible(|s| s.border_2().border_color(gpui::rgb(0x3a83f7)))
            .when(!primary && (permission || user_input) && !native, |d| {
                d.border_1().border_color(t.border)
            })
            .when(primary && !native, |d| d.bg(t.button_primary_solid))
            .when(native, |d| {
                d.px(px(9.))
                    .py(px(1.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(9999.)),
                        fill: Some(if primary {
                            t.button_primary_solid
                        } else {
                            t.text.opacity(0.03)
                        }),
                        border: Some((px(1.), t.border)),
                        ..Default::default()
                    }))
            })
            .child(text(
                label,
                13.,
                if native { 18. } else { 20. },
                430.,
                if primary {
                    if native {
                        t.surface_elevated_secondary
                    } else {
                        t.text_inverse
                    }
                } else if permission || user_input || native {
                    t.text
                } else {
                    t.text_secondary
                },
            ))
            .when(
                permission
                    || (native
                        && self.footer_width >= 448.
                        && decision != ApprovalDecision::AcceptForSession),
                |d| {
                    let color = if native {
                        if primary {
                            t.surface_elevated_secondary
                        } else {
                            t.text
                        }
                    } else if primary {
                        t.text_inverse
                    } else {
                        t.text_secondary
                    };
                    d.child(
                        div()
                            .relative()
                            .when(native, |d| d.top(px(0.5)))
                            .px(px(if native { 6. } else { 4. }))
                            .when(native, |d| d.min_w(px(16.)))
                            .when(!native, |d| d.rounded(px(4.)).bg(t.text_alpha_5))
                            .when(native, |d| {
                                d.child(shapes::decoration(shapes::Decoration {
                                    radius: Corners::all(px(10.)),
                                    corner: shapes::Corner::Squircle,
                                    fill: Some(if !primary {
                                        t.text_alpha_10
                                    } else {
                                        color.opacity(1. - (1. - 0.1))
                                    }),
                                    ..Default::default()
                                }))
                            })
                            .child(text(
                                if decision == ApprovalDecision::Decline {
                                    "Esc"
                                } else if native {
                                    "⏎"
                                } else {
                                    "↵"
                                },
                                if native { 12. } else { 10. },
                                16.,
                                if native { 430. } else { 500. },
                                color,
                            )),
                    )
                },
            )
            .when(form_submit, |d| {
                d.child(
                    div()
                        .px(px(4.))
                        .rounded(px(4.))
                        .bg(t.text_inverse.opacity(0.07))
                        .child(text(
                            if cfg!(target_os = "macos") {
                                "⌘↵"
                            } else {
                                "Ctrl+↵"
                            },
                            10.,
                            16.,
                            500.,
                            t.text_inverse,
                        )),
                )
            })
            .when(enabled, |d| {
                d.focusable()
                    .tab_index(0)
                    .cursor_pointer()
                    .on_key_down(self.request_listener(
                        cx,
                        move |this, event: &gpui::KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                && event.keystroke.modifiers == gpui::Modifiers::default()
                            {
                                this.request_action(decision, window, cx);
                                cx.stop_propagation();
                            }
                        },
                    ))
                    .on_click(self.request_listener(cx, move |this, _, window, cx| {
                        this.request_action(decision, window, cx)
                    }))
            })
    }

    pub(super) fn permission_request_card(
        &self,
        approval: &Approval,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let internet = approval.permissions.as_ref().is_some_and(|permissions| {
            permissions["fileSystem"].is_null() && permissions["network"]["enabled"] == true
        });
        let arrow = crate::icon_button(
            "request-scope",
            icon("chevron-down-small", 12., t.text_inverse),
            t,
        )
        .size(px(28.))
        .track_focus(&self.permissions_focus)
        .role(gpui::Role::Button)
        .aria_label("More approval options")
        .aria_expanded(self.menu == Some(Menu::Permissions))
        .tab_index(0)
        .on_key_down(
            self.request_listener(cx, |this, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && event.keystroke.modifiers == gpui::Modifiers::default()
                {
                    this.toggle_request_scope(window, cx);
                    cx.stop_propagation();
                }
            }),
        )
        .on_click(self.request_listener(cx, |this, _, window, cx| {
            this.toggle_request_scope(window, cx)
        }));
        div()
            .relative()
            .p(px(16.))
            .rounded(px(25.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface_elevated_secondary)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("hand", 16., t.text_secondary))
                    .child(text(
                        if internet {
                            "Internet access"
                        } else {
                            "Permissions"
                        },
                        13.,
                        20.,
                        430.,
                        t.text_secondary,
                    )),
            )
            .child(
                div()
                    .mt(px(8.))
                    .child(text(approval.title.clone(), 14., 20., 500., t.text)),
            )
            .when_some(approval.reason.clone(), |d, reason| {
                d.child(
                    div()
                        .mt(px(4.))
                        .child(text(reason, 13., 18., 430., t.text_secondary)),
                )
            })
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.request_button(
                        "request-deny",
                        "Deny".into(),
                        ApprovalDecision::Decline,
                        false,
                        t,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .rounded(px(999.))
                            .bg(t.button_primary_solid)
                            .child(self.request_button(
                                "request-allow",
                                "Allow once".into(),
                                ApprovalDecision::Accept,
                                true,
                                t,
                                cx,
                            ))
                            .child(arrow),
                    ),
            )
            .when(self.menu == Some(Menu::Permissions), |d| {
                d.child(
                    popover(t, 220., |d| {
                        d.child(
                            self.request_button(
                                "request-allow-session",
                                approval.scoped_label.clone(),
                                ApprovalDecision::AcceptForSession,
                                false,
                                t,
                                cx,
                            )
                            .track_focus(&self.menu_focus.items[0]),
                        )
                    })
                    .id("request-scope-menu")
                    .absolute()
                    .right(px(16.))
                    .bottom(px(48.))
                    .on_mouse_down_out(self.request_listener(cx, |this, _, window, cx| {
                        this.menu = None;
                        this.permissions_focus.focus(window, cx);
                        cx.notify();
                    })),
                )
            })
    }

    fn toggle_request_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            self.permissions_focus.focus(window, cx);
        } else {
            self.menu = Some(Menu::Permissions);
            self.menu_focus.prepare(1, cx);
            self.menu_focus.trigger = self.permissions_focus.clone();
            self.menu_focus.items[0].focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn request_action(
        &mut self,
        decision: ApprovalDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self
            .approval
            .as_ref()
            .and_then(|approval| approval.form.as_ref())
            && self.request_draft.index + 1 < form.fields.len()
            && matches!(
                decision,
                ApprovalDecision::Accept | ApprovalDecision::Decline
            )
        {
            if decision == ApprovalDecision::Accept
                && form.method == "mcpServer/elicitation/request"
                && let Err(error) = form.field_value(
                    self.request_draft.index,
                    &self.request_draft.fields,
                    &self.request_draft.selected,
                    cx,
                )
            {
                self.request_draft.error = Some(error);
                cx.notify();
                return;
            }
            self.request_draft.index += 1;
            self.request_draft.error = None;
            self.request_focus_pending = true;
            self.focus_request_field(window, cx);
            cx.notify();
            return;
        }
        self.decide(decision, window, cx);
    }

    fn select_request_choice(
        &mut self,
        field: usize,
        value: &Value,
        multiple: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if field != self.request_draft.index {
            return;
        }
        if self
            .approval
            .as_ref()
            .and_then(|approval| approval.form.as_ref())
            .is_some_and(|form| form.fields[field]["type"] == "boolean")
        {
            let checked = self.request_draft.selected[field].first() == Some(&json!(true));
            self.request_draft.selected[field] = vec![json!(!checked)];
            cx.notify();
            return;
        }
        let selection = &mut self.request_draft.selected[field];
        if selection.contains(value) {
            selection.retain(|item| item != value);
        } else {
            if !multiple {
                selection.clear();
            }
            selection.push(value.clone());
        }
        cx.notify();
        if self
            .approval
            .as_ref()
            .and_then(|approval| approval.form.as_ref())
            .is_some_and(|form| {
                form.method == "mcpServer/elicitation/request"
                    && Form::single_choice(&form.fields[field])
            })
        {
            self.request_action(ApprovalDecision::Accept, window, cx);
        }
    }

    pub(super) fn answer_request_parent(
        &self,
        decision: ApprovalDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((parent, identity, field)) = self.request_parent.clone() else {
            return false;
        };
        window.defer(cx, move |window, cx| {
            parent
                .update(cx, |parent, cx| {
                    if parent.request_draft.index == field
                        && parent
                            .approval
                            .as_ref()
                            .is_some_and(|approval| approval.identity == identity)
                    {
                        parent.request_action(decision, window, cx);
                    }
                })
                .ok();
        });
        true
    }

    pub(super) fn request_response(
        &self,
        decision: ApprovalDecision,
        cx: &App,
    ) -> Result<Value, (usize, String)> {
        let approval = self.approval.as_ref().unwrap();
        match &approval.form {
            Some(form) => form.response(
                decision,
                &self.request_draft.fields,
                &self.request_draft.selected,
                cx,
            ),
            None => Ok(approval.response(decision)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InputQueryViewport;

    impl Render for InputQueryViewport {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[test]
    fn secret_text_queries_hide_content_and_preserve_utf16_ranges() {
        let mut context = gpui::HeadlessAppContext::new(textsys::text_system());
        let window = context
            .open_window(gpui::size(px(200.), px(100.)), |_, cx| {
                cx.new(|_| InputQueryViewport)
            })
            .unwrap();
        context
            .update_window(window.into(), |_, window, cx| {
                let field = cx.new(|cx| Composer::settings_field("Secret", "a😀b", false, cx));
                field.update(cx, |field, cx| {
                    field.secret = true;
                    let mut actual = None;
                    let text = gpui::EntityInputHandler::text_for_range(
                        field,
                        0..usize::MAX,
                        &mut actual,
                        window,
                        cx,
                    );
                    assert_eq!(text.as_deref(), Some("****"));
                    assert_eq!(actual, Some(0..4));
                    assert_eq!(field.field_text(), "a😀b");
                });
            })
            .unwrap();
    }

    #[test]
    fn permission_answers_never_grant_more_than_the_request() {
        let request: PermissionsApprovalRequest = serde_json::from_value(json!({
            "threadId": "thread-a", "turnId": "turn-a", "itemId": "permissions-a",
            "reason": null, "permissions": { "network": { "enabled": true }, "fileSystem": null }
        }))
        .unwrap();
        let approval = Approval::permissions(json!(1), &request);
        assert_eq!(
            approval.response(ApprovalDecision::Accept),
            json!({ "permissions": { "network": { "enabled": true } }, "scope": "turn" })
        );
        assert_eq!(
            approval.response(ApprovalDecision::AcceptForSession),
            json!({ "permissions": { "network": { "enabled": true } }, "scope": "session" })
        );
        assert_eq!(
            approval.response(ApprovalDecision::Decline),
            json!({ "permissions": {}, "scope": "turn" })
        );
    }

    #[test]
    fn form_answers_keep_types_and_reject_invalid_values() {
        let mut context = gpui::HeadlessAppContext::new(textsys::text_system());
        context.update(|cx| {
            let params = json!({
                "threadId": "thread-a", "turnId": null, "serverName": "fixture", "mode": "form",
                "message": "Confirm the settings", "requestedSchema": {
                    "type": "object", "required": ["enabled", "count"],
                    "properties": {
                        "enabled": { "type": "boolean", "default": false },
                        "count": { "type": "integer", "minimum": 1, "maximum": 5, "default": 3 }
                    }
                }
            });
            let composer = cx.new(|cx| Composer::new("", false, cx));
            composer.update(cx, |composer, cx| {
                composer.set_approval(Approval::form(json!(1), "mcpServer/elicitation/request", &params), cx);
                assert_eq!(composer.request_response(ApprovalDecision::Accept, cx).unwrap(),
                    json!({ "action": "accept", "content": { "enabled": false, "count": 3 }, "_meta": null }));
                let form = composer.approval.as_ref().unwrap().form.as_ref().unwrap();
                let count = form.fields.iter().position(|field| field["id"] == "count").unwrap();
                composer.request_draft.fields[count].update(cx, |editor, cx| editor.set_text("2.0", cx));
                assert_eq!(composer.request_response(ApprovalDecision::Accept, cx).unwrap()["content"]["count"], json!(2));
                composer.request_draft.fields[count].update(cx, |editor, cx| editor.set_text("6", cx));
                assert!(composer.request_response(ApprovalDecision::Accept, cx).is_err());
                assert_eq!(composer.request_response(ApprovalDecision::Cancel, cx).unwrap(),
                    json!({ "action": "cancel", "content": null, "_meta": null }));
            });
        });
    }

    #[test]
    fn schema_presence_and_choice_defaults_do_not_add_answers() {
        let mut context = gpui::HeadlessAppContext::new(textsys::text_system());
        context.update(|cx| {
            let params = json!({ "threadId": "thread-a", "turnId": null, "serverName": "fixture", "mode": "form", "_meta": null,
                "message": "Choose the values", "requestedSchema": { "type": "object",
                    "required": ["empty", "required-list"], "properties": {
                        "empty": { "type": "string" },
                        "optional-list": { "type": "array", "items": { "type": "string", "enum": ["a"] } },
                        "required-list": { "type": "array", "items": { "type": "string", "enum": ["a"] } },
                        "cleared-default": { "type": "string", "default": "old" },
                        "choice": { "type": "string", "enum": ["a", "b"], "default": "a" }
                    } } });
            let composer = cx.new(|cx| Composer::new("", false, cx));
            composer.update(cx, |composer, cx| {
                composer.set_approval(Approval::form(json!(1), "mcpServer/elicitation/request", &params), cx);
                let form = composer.approval.as_ref().unwrap().form.as_ref().unwrap();
                let cleared = form.fields.iter().position(|field| field["id"] == "cleared-default").unwrap();
                composer.request_draft.fields[cleared].update(cx, |editor, cx| editor.set_text("", cx));
                assert_eq!(composer.request_response(ApprovalDecision::Accept, cx).unwrap(),
                    json!({ "action": "accept", "content": { "empty": "", "required-list": [], "cleared-default": "" }, "_meta": null }));
            });
        });
    }

    #[test]
    fn user_answers_keep_question_ids_and_dynamic_picker_actions() {
        let mut context = gpui::HeadlessAppContext::new(textsys::text_system());
        context.update(|cx| {
            let params = json!({ "threadId": "thread-a", "turnId": "turn-a", "itemId": "input-a",
                "questions": [{ "id": "language", "header": "Language", "question": "Choose a language",
                    "isOther": true, "isSecret": false, "options": [{ "label": "Rust" }] }]
            });
            let composer = cx.new(|cx| Composer::new("", false, cx));
            composer.update(cx, |composer, cx| {
                composer.set_approval(Approval::form(json!(1), "item/tool/requestUserInput", &params), cx);
                assert_eq!(composer.request_response(ApprovalDecision::Accept, cx).unwrap(), json!({ "answers": {} }));
                composer.request_draft.selected[0].push(json!("Rust"));
                assert_eq!(composer.request_response(ApprovalDecision::Accept, cx).unwrap(),
                    json!({ "answers": { "language": { "answers": ["Rust"] } } }));
                composer.request_draft.fields[0].update(cx, |editor, cx| editor.set_text("  Swift  ", cx));
                assert_eq!(composer.request_response(ApprovalDecision::Accept, cx).unwrap(),
                    json!({ "answers": { "language": { "answers": ["Swift"] } } }));
                let picker = json!({ "threadId": "thread-a", "turnId": "turn-a", "callId": "call-a",
                    "tool": "request_option_picker", "arguments": {
                        "question": "Choose a language", "options": [{ "label": "Rust" }], "allowMultiple": true
                    }
                });
                composer.set_approval(Approval::form(json!(2), "item/tool/call", &picker), cx);
                composer.request_draft.selected[0].push(json!("Rust"));
                let response = composer.request_response(ApprovalDecision::Decline, cx).unwrap();
                let content: Value = serde_json::from_str(response["contentItems"][0]["text"].as_str().unwrap()).unwrap();
                assert_eq!(content, json!({ "action": "skip", "selectedOptions": ["Rust"], "freeformAnswer": null }));
            });
        });
    }

    #[test]
    fn field_constraints_reject_invalid_formats_lengths_and_selections() {
        for (field, good, bad) in [
            (
                json!({ "format": "email" }),
                json!("demo@example.com"),
                json!("demo@example.com@other"),
            ),
            (
                json!({ "format": "uri" }),
                json!("https://example.com"),
                json!("not a URL"),
            ),
            (
                json!({ "format": "date" }),
                json!("2026-10-20"),
                json!("2026-02-30"),
            ),
            (
                json!({ "format": "date-time" }),
                json!("2026-10-20T12:30:00Z"),
                json!("2026-10-20T25:00:00Z"),
            ),
            (
                json!({ "minLength": 2, "maxLength": 3 }),
                json!("é日"),
                json!("a"),
            ),
            (
                json!({ "type": "array", "minItems": 1, "maxItems": 2, "items": { "enum": ["a", "b"] } }),
                json!(["a"]),
                json!(["outside"]),
            ),
            (
                json!({ "minimum": 9_007_199_254_740_993_u64 }),
                json!(9_007_199_254_740_993_u64),
                json!(9_007_199_254_740_992_u64),
            ),
        ] {
            assert!(validate(&field, &good).is_ok(), "{field}: {good}");
            assert!(validate(&field, &bad).is_err(), "{field}: {bad}");
        }
    }
}
