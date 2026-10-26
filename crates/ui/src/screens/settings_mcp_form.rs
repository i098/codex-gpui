use super::{Action, Activate as _, State, action_button, card, dispatch, on_card, primary_button};
use crate::{composer::Composer, icon::icon, text};
use gpui::{
    AppContext as _, Context, Div, Entity, EventEmitter, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::{Map, Value, json};
use shapes::Corner::Squircle;
use theme::{ActiveTheme as _, Theme};

pub struct Close;

pub struct Form {
    key: Option<String>,
    initial: Value,
    http: bool,
    name: Entity<Composer>,
    command: Entity<Composer>,
    cwd: Entity<Composer>,
    url: Entity<Composer>,
    bearer: Entity<Composer>,
    lists: Vec<List>,
    layer: super::Layer,
}

impl EventEmitter<Close> for Form {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Args,
    Env,
    Passthrough,
    Headers,
    EnvHeaders,
}

struct List {
    kind: Kind,
    rows: Vec<Row>,
}

struct Row {
    fields: Vec<Entity<Composer>>,
    preserve_empty: bool,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Args => "args",
            Self::Env => "env",
            Self::Passthrough => "env_vars",
            Self::Headers => "http_headers",
            Self::EnvHeaders => "env_http_headers",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Args => "Arguments",
            Self::Env => "Environment variables",
            Self::Passthrough => "Environment variable passthrough",
            Self::Headers => "Headers",
            Self::EnvHeaders => "Headers from environment variables",
        }
    }
    fn add_label(self) -> &'static str {
        match self {
            Self::Args => "Add argument",
            Self::Env => "Add environment variable",
            Self::Headers => "Add header",
            _ => "Add variable",
        }
    }
    fn is_map(self) -> bool {
        matches!(self, Self::Env | Self::Headers | Self::EnvHeaders)
    }
}

impl Form {
    pub(super) fn new(
        key: Option<String>,
        config: Value,
        layer: super::Layer,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut field = |placeholder, value: &str| {
            let field = cx.new(|cx| Composer::settings_compact_field(placeholder, value, cx));
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            field
        };
        let name = field(
            "MCP server name",
            config["name"].as_str().or(key.as_deref()).unwrap_or(""),
        );
        let command = field(
            "openai-dev-mcp serve-sqlite",
            config["command"].as_str().unwrap_or(""),
        );
        let cwd = field("", config["cwd"].as_str().unwrap_or(""));
        let url = field(
            "https://mcp.example.com/mcp",
            config["url"].as_str().unwrap_or(""),
        );
        let bearer = field(
            "MCP_BEARER_TOKEN",
            config["bearer_token_env_var"].as_str().unwrap_or(""),
        );
        let mut lists = Vec::new();
        for kind in [
            Kind::Args,
            Kind::Env,
            Kind::Passthrough,
            Kind::Headers,
            Kind::EnvHeaders,
        ] {
            let rows = if kind.is_map() {
                config[kind.key()]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(key, value)| Row {
                        fields: vec![
                            field("Key", key),
                            field("Value", value.as_str().unwrap_or("")),
                        ],
                        preserve_empty: false,
                    })
                    .collect::<Vec<_>>()
            } else {
                config[kind.key()]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|value| Row {
                        fields: vec![field("", value.as_str().unwrap_or(""))],
                        preserve_empty: kind == Kind::Args,
                    })
                    .collect::<Vec<_>>()
            };
            lists.push(List { kind, rows });
        }
        let mut form = Self {
            key,
            http: config.get("url").is_some(),
            initial: config,
            name,
            command,
            cwd,
            url,
            bearer,
            lists,
            layer,
        };
        for index in 0..form.lists.len() {
            if form.lists[index].rows.is_empty() {
                form.add_row(index, cx);
            }
        }
        form
    }

    fn add_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let kind = self.lists[index].kind;
        let placeholders = if kind.is_map() {
            vec!["Key", "Value"]
        } else {
            vec![""]
        };
        let row = placeholders
            .into_iter()
            .map(|placeholder| {
                let field = cx.new(|cx| Composer::settings_compact_field(placeholder, "", cx));
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            })
            .collect();
        self.lists[index].rows.push(Row {
            fields: row,
            preserve_empty: false,
        });
        cx.notify();
    }

    fn value(&self, cx: &Context<Self>) -> Value {
        let mut config = self.initial.clone();
        if !config.is_object() {
            config = json!({});
        }
        let object = config.as_object_mut().expect("object established");
        for key in [
            "command",
            "args",
            "env",
            "env_vars",
            "cwd",
            "url",
            "bearer_token_env_var",
            "http_headers",
            "env_http_headers",
        ] {
            object.remove(key);
        }
        if self.http {
            object.insert("url".into(), self.url.read(cx).field_text().into());
            let bearer = self.bearer.read(cx).field_text().trim();
            if !bearer.is_empty() {
                object.insert("bearer_token_env_var".into(), bearer.into());
            }
        } else {
            object.insert("command".into(), self.command.read(cx).field_text().into());
            let cwd = self.cwd.read(cx).field_text().trim();
            if !cwd.is_empty() {
                object.insert("cwd".into(), cwd.into());
            }
        }
        for list in &self.lists {
            if self.http != matches!(list.kind, Kind::Headers | Kind::EnvHeaders) {
                continue;
            }
            if list.kind.is_map() {
                let pairs = list
                    .rows
                    .iter()
                    .filter_map(|row| {
                        let key = row.fields[0].read(cx).field_text().trim();
                        (!key.is_empty()).then(|| {
                            (
                                key.to_owned(),
                                Value::from(row.fields[1].read(cx).field_text()),
                            )
                        })
                    })
                    .collect::<Map<String, Value>>();
                if !pairs.is_empty() {
                    object.insert(list.kind.key().into(), Value::Object(pairs));
                }
            } else {
                let values = list
                    .rows
                    .iter()
                    .filter_map(|row| {
                        let text = row.fields[0].read(cx).field_text();
                        let value = if list.kind == Kind::Args {
                            text
                        } else {
                            text.trim()
                        };
                        (!value.is_empty() || row.preserve_empty).then(|| Value::from(value))
                    })
                    .collect::<Vec<_>>();
                if !values.is_empty() {
                    object.insert(list.kind.key().into(), Value::Array(values));
                }
            }
        }
        config
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let value = self.value(cx);
        let name = self.name.read(cx).field_text().trim();
        if name.is_empty()
            || value[if self.http { "url" } else { "command" }]
                .as_str()
                .is_none_or(|value| value.trim().is_empty())
        {
            return;
        }
        let key = self
            .key
            .clone()
            .unwrap_or_else(|| server_key(name, &cx.global::<State>().configuration));
        dispatch(Action::SaveServer { key, config: value }, cx);
    }

    fn field(&self, title: &'static str, field: &Entity<Composer>, t: &Theme) -> Div {
        div()
            .relative()
            .px(px(12.))
            .py(px(8.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(on_card(
                12.5,
                Squircle,
                t.control_background.opacity(0.96),
                None,
            ))
            .child(text(title, 14., 21., 500., t.text))
            .child(field.clone())
    }

    fn list(&self, index: usize, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let list = &self.lists[index];
        let rows = list.rows.iter().enumerate().map(|(row_index, row)| {
            let removable = row.preserve_empty
                || row
                    .fields
                    .iter()
                    .any(|field| !field.read(cx).field_text().is_empty());
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(
                    row.fields
                        .iter()
                        .map(|field| div().flex_1().min_w_0().child(field.clone())),
                )
                .child(
                    div()
                        .id(gpui::SharedString::from(format!(
                            "remove-{}-{row_index}",
                            list.kind.key()
                        )))
                        .role(gpui::Role::Button)
                        .aria_label("Remove entry")
                        .aria_disabled(!removable)
                        .size(px(24.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!removable, |d| d.opacity(0.4))
                        .child(icon("settings-remove", 14., t.text_tertiary))
                        .on_activate(window, cx, move |this, _, cx| {
                            if removable {
                                this.lists[index].rows.remove(row_index);
                                if this.lists[index].rows.is_empty() {
                                    this.add_row(index, cx);
                                }
                                cx.notify();
                            }
                        }),
                )
        });
        let mut muted = t.text_alpha_60;
        muted.a = 0.65 * 0.9;
        div()
            .relative()
            .px(px(12.))
            .py(px(8.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(on_card(
                12.5,
                Squircle,
                t.control_background.opacity(0.96),
                None,
            ))
            .child(text(list.kind.label(), 14., 21., 500., t.text))
            .child(div().flex().flex_col().gap(px(8.)).children(rows))
            .child(
                div()
                    .id(gpui::SharedString::from(format!("add-{}", list.kind.key())))
                    .role(gpui::Role::Button)
                    .aria_label(list.kind.add_label())
                    .relative()
                    .h(px(28.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .child(on_card(10., Squircle, t.text_alpha_5, None))
                    .child(icon("settings-mcp-plus", 14., muted))
                    .child(text(list.kind.add_label(), 13., 18., 430., muted))
                    .on_activate(window, cx, move |this, _, cx| this.add_row(index, cx)),
            )
    }
}

pub fn server_key(name: &str, configuration: &Value) -> String {
    let mut normalized = String::new();
    let mut previous_whitespace = false;
    for ch in name.trim().chars() {
        let whitespace = ch.is_whitespace();
        if whitespace && previous_whitespace {
            continue;
        }
        previous_whitespace = whitespace;
        let ch = if whitespace {
            '_'
        } else if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if ch != '-' || !normalized.ends_with('-') {
            normalized.push(ch);
        }
    }
    if normalized.is_empty() {
        normalized = "custom-server".into();
    }
    let used = |key: &str| configuration["config"]["mcp_servers"].get(key).is_some();
    if !used(&normalized) {
        return normalized;
    }
    for suffix in 2.. {
        let candidate = format!("{normalized}-{suffix}");
        if !used(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

impl Render for Form {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.global::<State>().clone();
        let title = self.key.as_ref().map_or_else(
            || "Connect to a custom MCP".to_owned(),
            |key| format!("Update {key} MCP"),
        );
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .pb(px(20.))
            .child(
                div()
                    .id("mcp-back")
                    .role(gpui::Role::Button)
                    .aria_label("Back")
                    .h(px(28.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(icon("settings-mcp-back", 16., t.text_tertiary))
                    .child(text("Back", 13., 18., 430., t.text_tertiary))
                    .on_activate(window, cx, |_, _, cx| cx.emit(Close)),
            );
        if let Some(key) = &self.key {
            let key = key.clone();
            header = header.child(action_button("uninstall-mcp", "Uninstall", &t).on_activate(
                window,
                cx,
                move |_, _, cx| dispatch(Action::RemoveServer(key.clone()), cx),
            ));
        }
        let mut identity = Vec::new();
        if self.key.is_none() {
            identity.push(self.field("Name", &self.name, &t));
            identity.push(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Type", 13., 18.571_428, 500., t.text))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(
                                div()
                                    .id("mcp-stdio")
                                    .role(gpui::Role::Button)
                                    .aria_label("STDIO")
                                    .aria_selected(!self.http)
                                    .h(px(28.))
                                    .px(px(9.))
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .when(!self.http, |d| {
                                        d.child(on_card(12.5, Squircle, t.text_alpha_5, None))
                                    })
                                    .child(text(
                                        "STDIO",
                                        13.,
                                        18.,
                                        430.,
                                        if self.http { t.text_tertiary } else { t.text },
                                    ))
                                    .on_activate(window, cx, |this, _, cx| {
                                        this.http = false;
                                        cx.notify();
                                    }),
                            )
                            .child(
                                div()
                                    .id("mcp-http")
                                    .role(gpui::Role::Button)
                                    .aria_label("Streamable HTTP")
                                    .aria_selected(self.http)
                                    .h(px(28.))
                                    .px(px(9.))
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .when(self.http, |d| {
                                        d.child(on_card(12.5, Squircle, t.text_alpha_5, None))
                                    })
                                    .child(text(
                                        "Streamable HTTP",
                                        13.,
                                        18.,
                                        430.,
                                        if self.http { t.text } else { t.text_tertiary },
                                    ))
                                    .on_activate(window, cx, |this, _, cx| {
                                        this.http = true;
                                        cx.notify();
                                    }),
                            ),
                    ),
            );
        } else {
            identity.push(div().child(text(
                "If you would like to switch MCP server type, please uninstall first.",
                13.,
                18.,
                430.,
                t.text_secondary,
            )));
        }
        let mut fields = Vec::new();
        if self.http {
            fields.push(self.field("URL", &self.url, &t));
            fields.push(self.field("Bearer token env var", &self.bearer, &t));
        } else {
            fields.push(self.field("Command to launch", &self.command, &t));
        }
        for (index, list) in self.lists.iter().enumerate() {
            if self.http == matches!(list.kind, Kind::Headers | Kind::EnvHeaders) {
                fields.push(self.list(index, &t, window, cx));
            }
        }
        if !self.http {
            fields.push(self.field("Working directory", &self.cwd, &t));
        }
        let value = self.value(cx);
        let disabled = state.pending
            || self.name.read(cx).field_text().trim().is_empty()
            || value[if self.http { "url" } else { "command" }]
                .as_str()
                .is_none_or(|value| value.trim().is_empty())
            || (self.key.is_some() && value == self.initial);
        let heading = div()
            .pb(px(20.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(text(title, 20., 26.6, 500., t.text))
            .child(
                div().h(px(21.)).flex().items_start().pt(px(1.5)).child(
                    div()
                        .id("mcp-docs")
                        .role(gpui::Role::Link)
                        .aria_label("Open MCP documentation")
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(text("Docs", 13., 18.571_428, 430., t.text_secondary))
                        .child(icon("settings-mcp-docs", 12., t.text_secondary))
                        .on_activate(window, cx, |_, _, cx| {
                            cx.open_url("https://developers.openai.com/codex/mcp")
                        }),
                ),
            );
        let mut body = div().flex().flex_col().gap(px(6.));
        if self.key.is_none() {
            body = body.child(card(identity, &self.layer, &t));
        } else {
            body = body.children(identity);
        }
        body = body.child(card(fields, &self.layer, &t));
        if let Some(error) = &state.form_error {
            body = body.child(text(error.clone(), 13., 18., 430., t.text_error));
        }
        body = body.child(
            div().flex().justify_end().child(
                primary_button("mcp-save", "Save", &t)
                    .aria_disabled(disabled)
                    .when(disabled, |d| d.opacity(0.4))
                    .on_activate(window, cx, move |this, _, cx| {
                        if !disabled {
                            this.save(cx);
                        }
                    }),
            ),
        );
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(heading)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_new_server_name_preserves_existing_configuration() {
        super::super::test_panic_hook();
        let configuration = json!({"config":{"mcp_servers":{"team_tools":{},"team_tools-2":{}}}});
        assert_eq!(server_key(" Team tools ", &configuration), "team_tools-3");
        assert_eq!(server_key("A.b/Service", &configuration), "a-b-service");
        assert_eq!(server_key(" Team   tools ", &configuration), "team_tools-3");
        let hidden =
            json!({"config":{"mcp_servers":{"cua_repl":{"command":"keep"},"codex_app":{}}}});
        assert_eq!(server_key("cua repl", &hidden), "cua_repl-2");
        assert_eq!(server_key("codex app", &hidden), "codex_app-2");
    }

    #[test]
    fn editing_http_keeps_authentication_and_timeout_settings() {
        use gpui::HeadlessAppContext;
        use std::sync::Arc;
        super::super::test_panic_hook();
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        let initial = json!({"url":"https://old.example/mcp","oauth":{"client_id":"public-client"},"scopes":["read"],"tool_timeout_sec":90});
        let form = cx.update(|cx| {
            cx.new(|cx| {
                Form::new(
                    Some("service".into()),
                    initial.clone(),
                    Default::default(),
                    cx,
                )
            })
        });
        let value = cx.update(|cx| {
            form.update(cx, |form, cx| {
                form.url.update(cx, |field, cx| {
                    field.set_text("https://new.example/mcp", cx)
                });
                form.value(cx)
            })
        });
        assert_eq!(
            value,
            json!({"url":"https://new.example/mcp","oauth":{"client_id":"public-client"},"scopes":["read"],"tool_timeout_sec":90})
        );
    }

    #[test]
    fn blank_added_rows_keep_existing_environment_values() {
        use gpui::HeadlessAppContext;
        use std::sync::Arc;
        super::super::test_panic_hook();
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        let form = cx.update(|cx| {
            cx.new(|cx| {
                Form::new(
                    Some("service".into()),
                    json!({"command":"server","env":{"TOKEN":"existing","EMPTY":""}}),
                    Default::default(),
                    cx,
                )
            })
        });
        let value = cx.update(|cx| {
            form.update(cx, |form, cx| {
                let index = form
                    .lists
                    .iter()
                    .position(|list| list.kind == Kind::Env)
                    .unwrap();
                form.add_row(index, cx);
                form.value(cx)
            })
        });
        assert_eq!(value["env"], json!({"TOKEN":"existing","EMPTY":""}));
    }

    #[test]
    fn editing_a_directory_preserves_exact_arguments() {
        use gpui::HeadlessAppContext;
        use std::sync::Arc;
        super::super::test_panic_hook();
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        let args = json!(["--label", " team ", "", "next"]);
        let form = cx.update(|cx| {
            cx.new(|cx| {
                Form::new(
                    Some("service".into()),
                    json!({"command":"server","args":args,"cwd":"/old"}),
                    Default::default(),
                    cx,
                )
            })
        });
        let saved = cx.update(|cx| {
            form.update(cx, |form, cx| {
                form.cwd.update(cx, |field, cx| field.set_text("/new", cx));
                let args_index = form
                    .lists
                    .iter()
                    .position(|list| list.kind == Kind::Args)
                    .unwrap();
                form.add_row(args_index, cx);
                form.value(cx)
            })
        });
        assert_eq!(saved["args"], args);
        assert_eq!(saved["cwd"], "/new");
    }
}
