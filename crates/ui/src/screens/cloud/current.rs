use super::*;

#[derive(Clone, Copy)]
enum CurrentControl {
    Create,
    Retry,
    Tab(bool),
}

#[derive(Default)]
pub(super) struct Current {
    pub records: Vec<Value>,
    pub config: Option<Value>,
    pub fields: BTreeMap<String, Entity<Composer>>,
    pub creating: bool,
    pub repositories: Vec<Value>,
    pub network: String,
    pub sharing: String,
    pub deleting: Option<String>,
    pub vault: bool,
    pub vault_entries: Vec<Value>,
}

impl CloudPage {
    pub fn open_path(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.access_loaded {
            self.route_pending = Some(path.into());
            return;
        }
        if path == "/cloud-environments/new" {
            self.current_mode = Some(true);
            self.unavailable = !self.access.cloud;
            if !self.unavailable {
                self.current.creating = true;
                self.repository_picker = true;
                self.load(Operation::Repositories(String::new()), cx);
            }
        } else if path == "/settings/codex-cloud" {
            self.current_mode = Some(true);
            self.refresh(cx);
        } else {
            self.current_mode = Some(false);
            let path = path
                .strip_prefix("/settings/cloud-environments")
                .unwrap_or("");
            let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
            if self.access.legacy {
                match parts.as_slice() {
                    ["new"] => self.edit(None, cx),
                    [id] => self.load(Operation::Detail((*id).into()), cx),
                    [id, "edit"] => {
                        self.edit_requested = true;
                        self.load(Operation::Detail((*id).into()), cx);
                    }
                    _ => self.refresh(cx),
                }
            }
        }
        cx.notify();
    }
    pub fn open_section(&mut self, section: &str, cx: &mut Context<Self>) {
        let current = section == "Codex Cloud";
        if self.current_mode == Some(current) {
            return;
        }
        self.current_mode = Some(current);
        self.editor = None;
        self.current.config = None;
        self.current.creating = false;
        self.view = View::List;
        if self.access_loaded {
            self.refresh(cx);
        }
        cx.notify();
    }

    pub(super) fn receive_current(
        &mut self,
        operation: &Operation,
        body: Value,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match operation {
            Operation::Vault => {
                self.current.vault_entries = body
                    .as_array()
                    .ok_or("Couldn’t load all personal vault entries")?
                    .clone();
            }
            Operation::Configs => {
                self.current.records = body
                    .as_array()
                    .ok_or("Couldn’t load saved environments")?
                    .clone()
            }
            Operation::Config(_) | Operation::ConfigSave { .. } => {
                if body["id"].as_str().is_none() || body["name"].as_str().is_none() {
                    return Err("Couldn’t load environment".into());
                }
                self.open_config(body, cx);
            }
            Operation::ConfigDelete(_) => {
                self.current.config = None;
                self.confirm = None;
                self.refresh(cx);
            }
            Operation::ConfigCreate(_) => {
                self.current.creating = false;
                self.open_config(body, cx);
            }
            _ => return Err("Unexpected cloud configuration response".into()),
        }
        Ok(())
    }

    fn open_config(&mut self, config: Value, cx: &mut Context<Self>) {
        self.current.sharing = config["share_settings"]
            .as_str()
            .unwrap_or("private")
            .into();
        let draft = config
            .get("draft")
            .filter(|v| !v.is_null())
            .unwrap_or(&config);
        self.current.network = draft["network_policy"]["type"]
            .as_str()
            .unwrap_or("restricted")
            .into();
        let domains = draft["network_policy"]["egress_rules"]
            .as_array()
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|r| r["host"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let fields = [
            (
                "name",
                "Environment name",
                config["name"].as_str().unwrap_or(""),
                false,
            ),
            (
                "setup",
                "Setup script",
                draft["install_script"].as_str().unwrap_or(""),
                true,
            ),
            (
                "skill",
                "Start skill",
                draft["start_skill"].as_str().unwrap_or(""),
                false,
            ),
            ("domains", "Allowed domains", domains.as_str(), true),
        ];
        self.current.fields = fields
            .into_iter()
            .map(|(key, label, value, multiline)| {
                (
                    key.into(),
                    self.field(label, label, value, multiline, false, cx),
                )
            })
            .collect();
        self.current.config = Some(config);
        self.error = None;
    }

    fn current_save(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(config) = self.current.config.as_ref() else {
            return;
        };
        let name = self.current.fields["name"].read(cx).field_text().trim();
        if name.is_empty() || name.encode_utf16().count() > 128 {
            self.error = Some("Enter an environment name".into());
            cx.notify();
            return;
        }
        let source = config
            .get("draft")
            .filter(|v| !v.is_null())
            .unwrap_or(config);
        let mut metadata = serde_json::Map::new();
        if config["name"] != name {
            metadata.insert("name".into(), name.into());
        }
        if config["share_settings"] != self.current.sharing {
            metadata.insert("share_settings".into(), self.current.sharing.clone().into());
        }
        let mut changes = serde_json::Map::new();
        for (key, wire) in [("setup", "install_script"), ("skill", "start_skill")] {
            let value = self.current.fields[key].read(cx).field_text();
            if source[wire].as_str().unwrap_or("") != value {
                changes.insert(wire.into(), value.into());
            }
        }
        let domains: Vec<_> = self.current.fields["domains"]
            .read(cx)
            .field_text()
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|host| json!({"host":host}))
            .collect();
        let policy = if self.current.network == "unrestricted" {
            json!({"type":"unrestricted"})
        } else {
            let mut policy = source["network_policy"].clone();
            if !policy.is_object() {
                policy = json!({"type":"restricted","presets":["package_managers"]});
            }
            policy["type"] = "restricted".into();
            policy["egress_rules"] = domains.into();
            policy
        };
        if source["network_policy"] != policy {
            changes.insert("network_policy".into(), policy);
        }
        if changes.is_empty() && metadata.is_empty() {
            return;
        }
        self.load(
            Operation::ConfigSave {
                id: config["id"].as_str().unwrap().into(),
                config: config.clone(),
                changes: changes.into(),
                metadata: metadata.into(),
            },
            cx,
        );
    }

    pub(super) fn current_create(&mut self, cx: &mut Context<Self>) {
        if self.pending || self.current.repositories.is_empty() {
            return;
        }
        let repositories:Vec<_>=self.current.repositories.iter().filter_map(|repo|Some(json!({"repository_id":repo["id"].as_str()?,"ref":repo["default_branch"].as_str()?}))).collect();
        if repositories.len() != self.current.repositories.len() {
            self.error = Some("Could not start cloud environment setup".into());
            cx.notify();
            return;
        }
        let name = self.current.repositories[0]["name"]
            .as_str()
            .or_else(|| self.current.repositories[0]["repository_full_name"].as_str())
            .unwrap_or("");
        self.load(Operation::ConfigCreate(json!({"name":name,"repositories":repositories,"network_policy":{"type":"restricted","presets":["package_managers"],"egress_rules":[]},"share_settings":"private","start_onboarding":false})),cx);
    }

    pub(super) fn current_screen(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let secondary = gpui::Hsla { a: 0.65, ..t.text };
        div().size_full().relative().flex().flex_col()
            .child(div().h(px(46.)).flex_none())
            .child(div().id("cloud-current-page").overflow_y_scroll().flex_1()
                .child(div().pt(px(16.)).pb(px(1.)).flex().flex_col().items_center()
                    .child(div().w_full().max_w(px(768.)).px(px(20.))
                        .child(div().py(px(12.)).flex().flex_col().gap(px(4.))
                            .child(text("Codex Cloud",28.,33.6,500.,t.text))
                            .child(text("Set up cloud environments with the repositories, tools, and settings Codex needs to work on your projects",14.,24.,430.,secondary)))))
                .child(div().flex().flex_col().items_center()
                    .child(div().w_full().max_w(px(768.)).px(px(20.)).pb(px(20.))
                        .when(self.access.cloud,|d|d.child(self.current_page(cx))))))
            .children(self.overlay(cx)).children(self.menu_overlay(cx)).into_any_element()
    }

    fn current_button(
        &self,
        id: &'static str,
        label: &'static str,
        kind: CurrentControl,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let secondary = gpui::Hsla { a: 0.65, ..t.text };
        let clear = gpui::rgba(0).into();
        let (height, padding, size, line, weight, color, fill, radius, border) = match kind {
            CurrentControl::Create => (
                28.,
                9.,
                13.,
                18.,
                430.,
                t.control_background,
                t.button_primary_solid,
                12.5,
                Some(t.border),
            ),
            CurrentControl::Retry => (
                24.,
                9.,
                12.,
                16.,
                430.,
                t.text,
                t.text_alpha_5,
                12.5,
                Some(clear),
            ),
            CurrentControl::Tab(selected) => (
                26.571428,
                8.,
                13.,
                18.571428,
                500.,
                if selected { t.text } else { secondary },
                if selected { t.surface_secondary } else { clear },
                10.,
                None,
            ),
        };
        let content = div()
            .h(px(height))
            .px(px(padding))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .child(super::super::settings::on_card(
                radius,
                shapes::Corner::Squircle,
                fill,
                border,
            ))
            .when(matches!(kind, CurrentControl::Create), |d| {
                d.child(crate::icon::icon("cloud-create", 16., color))
            })
            .child(text(label, size, line, weight, color));
        self.wire_button(id.into(), label.into(), content, cx, action)
    }

    fn current_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let secondary = gpui::Hsla { a: 0.65, ..t.text };
        div()
            .relative()
            .pt(px(20.))
            .pb(px(8.))
            .child(
                div()
                    .pb(px(8.))
                    .flex()
                    .items_start()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .child(self.current_button(
                                "cloud-environments-tab",
                                "Environments",
                                CurrentControl::Tab(!self.current.vault),
                                cx,
                                |this, _, cx| {
                                    this.current.vault = false;
                                    this.current.config = None;
                                    this.refresh(cx);
                                },
                            ))
                            .child(self.current_button(
                                "cloud-vault-tab",
                                "Personal vault",
                                CurrentControl::Tab(self.current.vault),
                                cx,
                                |this, _, cx| {
                                    this.current.vault = true;
                                    this.current.config = None;
                                    this.load(Operation::Vault, cx);
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_end()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .w(px(160.))
                                    .h(px(28.))
                                    .px(px(13.))
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .child(super::super::settings::on_card(
                                        9999.,
                                        shapes::Corner::Round,
                                        gpui::rgba(0).into(),
                                        Some(t.border_strong),
                                    ))
                                    .child(crate::icon::icon("search", 16., secondary))
                                    .child(div().flex_1().min_w_0().child(self.query.clone())),
                            )
                            .when(!self.current.vault, |d| {
                                d.child(self.current_button(
                                    "config-create",
                                    "Create environment",
                                    CurrentControl::Create,
                                    cx,
                                    |this, _, cx| {
                                        this.current.creating = true;
                                        this.current.repositories.clear();
                                        this.repository_picker = true;
                                        this.load(Operation::Repositories(String::new()), cx);
                                    },
                                ))
                            }),
                    ),
            )
            .child(
                gpui::deferred(
                    div()
                        .absolute()
                        .top_full()
                        .left_0()
                        .right_0()
                        .h(px(32.))
                        .bg(gpui::linear_gradient(
                            180.,
                            gpui::linear_color_stop(t.surface, 0.),
                            gpui::linear_color_stop(gpui::Hsla { a: 0., ..t.surface }, 1.),
                        )),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    fn current_notice(&self, message: &str, retry: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let secondary = gpui::Hsla { a: 0.65, ..t.text };
        div()
            .relative()
            .p(px(1.))
            .child(super::super::settings::on_card(
                20.,
                shapes::Corner::Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(
                div()
                    .min_h(px(64.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text(message.to_owned(), 13., 18.571428, 430., secondary))
                    .when(retry, |d| {
                        d.child(self.current_button(
                            "cloud-current-retry",
                            "Retry",
                            CurrentControl::Retry,
                            cx,
                            |this, _, cx| {
                                if this.current.vault {
                                    this.load(Operation::Vault, cx);
                                } else {
                                    this.refresh(cx);
                                }
                            },
                        ))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn current_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        if let Some(config) = &self.current.config {
            let owner = config["is_owner"] != false;
            let editable = self.access.cloud
                && owner
                && (config["share_settings"] != "workspace" || self.access.manage_workspace);
            let name = config["name"].as_str().unwrap_or("").to_owned();
            let fields = [
                ("name", "Name"),
                ("setup", "Setup script"),
                ("skill", "Start skill"),
                ("domains", "Allowed domains"),
            ]
            .into_iter()
            .map(|(key, label)| {
                Self::row(
                    label,
                    None,
                    div()
                        .w(px(320.))
                        .child(self.current.fields[key].clone())
                        .into_any_element(),
                    &t,
                )
            })
            .collect();
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(text(name, 18., 27., 500., t.text))
                .child(Self::group("Configuration", fields, &t))
                .child(Self::row(
                    "Internet access",
                    None,
                    self.button(
                        "config-network",
                        if self.current.network == "unrestricted" {
                            "All domains"
                        } else {
                            "Custom domains"
                        },
                        false,
                        cx,
                        |this, _, cx| {
                            this.current.network = if this.current.network == "unrestricted" {
                                "restricted"
                            } else {
                                "unrestricted"
                            }
                            .into();
                            cx.notify();
                        },
                    ),
                    &t,
                ))
                .when(self.access.manage_workspace, |d| {
                    d.child(Self::row(
                        "Sharing",
                        None,
                        self.button(
                            "config-sharing",
                            if self.current.sharing == "private" {
                                "Only me"
                            } else {
                                "Shared with workspace"
                            },
                            false,
                            cx,
                            |this, _, cx| {
                                this.current.sharing = if this.current.sharing == "private" {
                                    "workspace"
                                } else {
                                    "private"
                                }
                                .into();
                                cx.notify();
                            },
                        ),
                        &t,
                    ))
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            self.button("config-cancel", "Cancel", false, cx, |this, _, cx| {
                                this.current.config = None;
                                this.refresh(cx);
                            }),
                        )
                        .when(editable, |d| {
                            d.child(self.button(
                                "config-save",
                                "Save changes",
                                true,
                                cx,
                                |this, _, cx| this.current_save(cx),
                            ))
                        }),
                )
                .into_any_element();
        }
        let search = self.query.read(cx).field_text().trim().to_lowercase();
        let rows = self
            .current
            .records
            .iter()
            .filter(|r| {
                r["name"]
                    .as_str()
                    .is_some_and(|s| s.to_lowercase().contains(&search))
            })
            .filter_map(|record| {
                let id = record["id"].as_str()?.to_owned();
                let name = record["name"].as_str()?.to_owned();
                Some(
                    div()
                        .flex()
                        .justify_between()
                        .gap(px(16.))
                        .p(px(12.))
                        .child(self.button(format!("config-{id}"), name, false, cx, {
                            let id = id.clone();
                            move |this, _, cx| this.load(Operation::Config(id.clone()), cx)
                        }))
                        .child(text(
                            if record["share_settings"] == "workspace" {
                                "Workspace"
                            } else {
                                "Private"
                            },
                            13.,
                            18.,
                            430.,
                            t.text_secondary,
                        ))
                        .when(record["is_owner"] != false, |d| {
                            d.child(self.button(
                                format!("config-delete-{id}"),
                                "Delete environment",
                                false,
                                cx,
                                move |this, _, cx| {
                                    this.current.deleting = Some(id.clone());
                                    this.confirm = Some("config-delete");
                                    cx.notify();
                                },
                            ))
                        })
                        .into_any_element(),
                )
            })
            .collect::<Vec<_>>();
        let contents = if let Some(error) = self.error.as_deref() {
            self.current_notice(error, true, cx)
        } else if self.pending {
            self.current_notice(
                if self.current.vault {
                    "Loading personal vault…"
                } else {
                    "Loading saved environments…"
                },
                false,
                cx,
            )
        } else if self.current.vault {
            let entries = self
                .current
                .vault_entries
                .iter()
                .filter_map(|entry| entry["secret"]["name"].as_str())
                .map(|name| text(name.to_owned(), 13., 18.571428, 430., t.text).into_any_element())
                .collect::<Vec<_>>();
            if entries.is_empty() {
                self.current_notice("No entries yet", false, cx)
            } else {
                Self::group("", entries, &t)
            }
        } else if self.current.records.is_empty() {
            self.current_notice("No saved environments", false, cx)
        } else {
            Self::group("", rows, &t)
        };
        div()
            .flex()
            .flex_col()
            .child(self.current_toolbar(cx))
            .child(div().pt(px(20.)).child(contents))
            .into_any_element()
    }
}
