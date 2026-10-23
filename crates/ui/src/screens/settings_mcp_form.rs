use super::{Action, Activate as _, State, action_button, card, dispatch};
use crate::{composer::Composer, text};
use gpui::{
    AppContext as _, Context, Div, Entity, EventEmitter, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::{Map, Value, json};
use theme::{ActiveTheme as _, Theme};

pub struct Close;

pub struct Form {
    key: Option<String>,
    state: Entity<State>,
    expected_version: Option<String>,
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
    rows: Vec<Vec<Entity<Composer>>>,
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

impl List {
    fn value(&self, cx: &gpui::App) -> Option<Value> {
        if self.kind.is_map() {
            let pairs = self
                .rows
                .iter()
                .filter_map(|row| {
                    let key = row[0].read(cx).field_text().trim();
                    (!key.is_empty())
                        .then(|| (key.to_owned(), Value::from(row[1].read(cx).field_text())))
                })
                .collect::<Map<String, Value>>();
            return (!pairs.is_empty()).then_some(Value::Object(pairs));
        }
        let values = self
            .rows
            .iter()
            .map(|row| {
                let value = row[0].read(cx).field_text();
                if self.kind == Kind::Args {
                    value
                } else {
                    value.trim()
                }
            })
            .filter(|value| self.kind == Kind::Args || !value.is_empty())
            .map(Value::from)
            .collect::<Vec<_>>();
        (!values.is_empty()).then_some(Value::Array(values))
    }
}

impl Form {
    pub fn new(
        key: Option<String>,
        config: Value,
        state: Entity<State>,
        cx: &mut Context<Self>,
    ) -> Self {
        let expected_version =
            configuration_version(&state.read(cx).configuration).map(str::to_owned);
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let mut field = |placeholder, value: &str| {
            let field = cx.new(|cx| Composer::settings_field(placeholder, value, false, cx));
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
            "https://example.com/mcp",
            config["url"].as_str().unwrap_or(""),
        );
        let bearer = field("", config["bearer_token_env_var"].as_str().unwrap_or(""));
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
                    .map(|(key, value)| {
                        vec![
                            field("Key", key),
                            field("Value", value.as_str().unwrap_or("")),
                        ]
                    })
                    .collect::<Vec<_>>()
            } else {
                config[kind.key()]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|value| vec![field("", value.as_str().unwrap_or(""))])
                    .collect::<Vec<_>>()
            };
            lists.push(List { kind, rows });
        }
        let mut form = Self {
            key,
            state,
            expected_version,
            http: config.get("url").is_some(),
            initial: config,
            name,
            command,
            cwd,
            url,
            bearer,
            lists,
            layer: super::Layer::default(),
        };
        for index in 0..form.lists.len() {
            if form.lists[index].rows.is_empty() && form.lists[index].kind != Kind::Args {
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
                let field = cx.new(|cx| Composer::settings_field(placeholder, "", false, cx));
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            })
            .collect();
        self.lists[index].rows.push(row);
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
            if let Some(value) = list.value(cx) {
                object.insert(list.kind.key().into(), value);
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
        let Some(expected_version) = self.expected_version.clone() else {
            self.state.update(cx, |state, cx| {
                state.form_error = Some(
                    "Configuration version unavailable. Reload settings before saving.".into(),
                );
                cx.notify();
            });
            return;
        };
        let key = self
            .key
            .clone()
            .unwrap_or_else(|| server_key(name, &self.state.read(cx).configuration));
        dispatch(
            &self.state,
            Action::SaveServer {
                key,
                config: value,
                expected_version,
            },
            cx,
        );
    }

    fn field(&self, title: &'static str, field: &Entity<Composer>, t: &Theme) -> Div {
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text(title, 14., 21., 500., t.text))
            .child(field.clone())
    }

    fn list(&self, index: usize, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let list = &self.lists[index];
        let rows = list.rows.iter().enumerate().map(|(row_index, fields)| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(
                    fields
                        .iter()
                        .map(|field| div().flex_1().min_w_0().child(field.clone())),
                )
                .child(
                    action_button(
                        format!("remove-{}-{row_index}", list.kind.key()),
                        "Remove entry",
                        t,
                    )
                    .on_activate(window, cx, move |this, _, cx| {
                        this.lists[index].rows.remove(row_index);
                        cx.notify();
                    }),
                )
        });
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text(list.kind.label(), 14., 21., 500., t.text))
            .children(rows)
            .child(
                action_button(format!("add-{}", list.kind.key()), list.kind.add_label(), t)
                    .on_activate(window, cx, move |this, _, cx| this.add_row(index, cx)),
            )
    }

    fn connection_fields(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let mut fields = Vec::new();
        if self.http {
            fields.push(self.field("URL", &self.url, t));
            fields.push(self.field("Bearer token env var", &self.bearer, t));
        } else {
            fields.push(self.field("Command to launch", &self.command, t));
        }
        for (index, list) in self.lists.iter().enumerate() {
            if self.http == matches!(list.kind, Kind::Headers | Kind::EnvHeaders) {
                fields.push(self.list(index, t, window, cx));
            }
        }
        if !self.http {
            fields.push(self.field("Working directory", &self.cwd, t));
        }
        fields
    }
}
fn configuration_version(configuration: &Value) -> Option<&str> {
    configuration["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            configuration["origins"]
                .as_object()
                .into_iter()
                .flat_map(|origins| origins.values()),
        )
        .filter(|layer| layer["name"]["type"] == "user")
        .find_map(|layer| {
            layer["version"]
                .as_str()
                .filter(|version| !version.is_empty())
        })
}

pub fn server_key(name: &str, configuration: &Value) -> String {
    let normalized = normalized_server_name(name);
    let used = |key: &str| {
        super::server_hidden(key) || configuration["config"]["mcp_servers"].get(key).is_some()
    };
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

fn normalized_server_name(name: &str) -> String {
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
    normalized
}

impl Render for Form {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = self.state.read(cx).clone();
        let title = self.key.as_ref().map_or_else(
            || "Connect to a custom MCP".to_owned(),
            |key| format!("Update {key} MCP"),
        );
        let mut header = div().flex().items_center().justify_between().child(
            action_button("mcp-back", "Back", &t)
                .on_activate(window, cx, |_, _, cx| cx.emit(Close)),
        );
        if let Some(key) = &self.key {
            let key = key.clone();
            header = header.child(action_button("uninstall-mcp", "Uninstall", &t).on_activate(
                window,
                cx,
                move |this, _, cx| dispatch(&this.state, Action::RemoveServer(key.clone()), cx),
            ));
        }
        let mut identity = Vec::new();
        if self.key.is_none() {
            identity.push(self.field("Name", &self.name, &t));
            identity.push(
                div()
                    .p(px(16.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Type", 13., 18.571_428, 500., t.text))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                action_button("mcp-stdio", "STDIO", &t)
                                    .aria_selected(!self.http)
                                    .on_activate(window, cx, |this, _, cx| {
                                        this.http = false;
                                        cx.notify();
                                    }),
                            )
                            .child(
                                action_button("mcp-http", "Streamable HTTP", &t)
                                    .aria_selected(self.http)
                                    .on_activate(window, cx, |this, _, cx| {
                                        this.http = true;
                                        cx.notify();
                                    }),
                            ),
                    ),
            );
        } else {
            identity.push(div().p(px(12.)).child(text(
                "If you would like to switch MCP server type, please uninstall first.",
                13.,
                18.,
                430.,
                t.text_secondary,
            )));
        }
        let fields = self.connection_fields(&t, window, cx);
        let value = self.value(cx);
        let disabled = state.pending
            || self.name.read(cx).field_text().trim().is_empty()
            || value[if self.http { "url" } else { "command" }]
                .as_str()
                .is_none_or(|value| value.trim().is_empty())
            || (self.key.is_some() && value == self.initial);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(header)
            .child(text(title, 20., 28., 500., t.text))
            .child(
                action_button("mcp-docs", "Docs", &t).on_activate(window, cx, |_, _, cx| {
                    cx.open_url("https://developers.openai.com/codex/mcp")
                }),
            )
            .child(card(identity, &self.layer, &t))
            .child(card(fields, &self.layer, &t));
        if let Some(error) = &state.form_error {
            body = body.child(text(error.clone(), 13., 18., 430., t.text_error));
        }
        body.child(
            div().flex().justify_end().child(
                action_button("mcp-save", "Save", &t)
                    .aria_disabled(disabled)
                    .when(disabled, |d| d.opacity(0.4))
                    .on_activate(window, cx, move |this, _, cx| {
                        if !disabled {
                            this.save(cx);
                        }
                    }),
            ),
        )
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
            let state = cx.new(|_| State::default());
            cx.new(|cx| Form::new(Some("service".into()), initial.clone(), state, cx))
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
            let state = cx.new(|_| State::default());
            cx.new(|cx| {
                Form::new(
                    Some("service".into()),
                    json!({"command":"server","env":{"TOKEN":"existing","EMPTY":""}}),
                    state,
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
    fn hidden_server_names_never_replace_internal_configuration() {
        let configuration = json!({"config":{"mcp_servers":{
            "codex_app":{"command":"internal"},
            "cua_repl":{"command":"internal"},
            "codex_app-2":{}
        }}});
        assert_eq!(server_key("codex_app", &configuration), "codex_app-3");
        assert_eq!(server_key("cua_repl", &configuration), "cua_repl-2");
        assert_eq!(server_key("codex_app", &Value::Null), "codex_app-2");
    }

    #[test]
    fn changing_the_working_directory_preserves_all_command_arguments() {
        super::super::test_panic_hook();
        let mut cx = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            std::sync::Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        let initial = json!({"command":"server","args":["--label"," padded ","--empty",""]});
        let form = cx.update(|cx| {
            let state = cx.new(|_| State::default());
            cx.new(|cx| Form::new(Some("service".into()), initial.clone(), state, cx))
        });
        let value = cx.update(|cx| {
            form.update(cx, |form, cx| {
                form.cwd
                    .update(cx, |field, cx| field.set_text("/tmp/project", cx));
                form.value(cx)
            })
        });
        assert_eq!(value["args"], initial["args"]);
        assert_eq!(value["cwd"], "/tmp/project");
    }

    #[test]
    fn configuration_versions_belong_to_the_user_file() {
        let user = json!({"name":{"type":"user","file":"/home/config.toml"},"version":"user-v1"});
        let project = json!({"name":{"type":"project","dotCodexFolder":"/project/.codex"},"version":"project-v2"});
        assert_eq!(
            configuration_version(&json!({"layers":[project, user.clone()]})),
            Some("user-v1")
        );
        assert_eq!(
            configuration_version(&json!({"origins":{"mcp_servers.work.command":user}})),
            Some("user-v1")
        );
        assert_eq!(configuration_version(&json!({"config":{}})), None);
    }
}
