use crate::{Root, host};
use anyhow::{Context as _, Result, ensure};
use gpui::{AppContext as _, Context, Entity};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use ui::screens::{
    cloud::{Access, CloudPage, Operation, Request},
    settings::SettingsPage,
};
use url::Url;

const GITHUB_CONNECTOR: &str = "connector_76869538009648d5b282a4bb21c3d157";
const GITHUB_ENTERPRISE_TEMPLATE: &str = "templated_apps_GitHubEnterprise";

#[derive(Debug)]
struct HttpStatus(u64);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Cloud HTTP status {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

pub struct CloudTaskRequest {
    pub prompt: String,
    pub branch: String,
    pub environment_id: String,
    pub model: Option<String>,
    pub read_only: bool,
}

pub fn create_cloud_task(host: &dyn host::Host, task: &CloudTaskRequest) -> Result<Value> {
    ensure!(
        !task.prompt.trim().is_empty(),
        "A cloud task needs a prompt"
    );
    ensure!(
        !task.environment_id.is_empty(),
        "A cloud task needs an environment"
    );
    let identity = host
        .identity()
        .context("Cloud authentication is unavailable")?;
    let mut body = json!({
        "new_task": {
            "branch": task.branch,
            "environment_id": task.environment_id,
            "run_environment_in_qa_mode": task.read_only,
        },
        "input_items": [{
            "type":"message", "role":"user",
            "content":[{"content_type":"text","text":task.prompt}],
        }],
    });
    if let Some(model) = &task.model {
        body["metadata"] = json!({"model_slug":model});
    }
    request(host, &identity, "POST", "/wham/tasks", Some(body))
}

/// The id of the task a `POST /wham/tasks` created: `{ "task": { "id" } }`.
fn created_task_id(response: &Value) -> Result<String> {
    response
        .pointer("/task/id")
        .or_else(|| response.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .context("The cloud task response has no task id")
}

/// The account's cloud environments as `(label, id)`, for the `Cloud` start's environment
/// selector. Signed out or no Cloud access: an error.
pub(crate) fn environment_names(host: &dyn host::Host) -> Result<Vec<(String, String)>> {
    let identity = host
        .identity()
        .context("Cloud authentication is unavailable")?;
    let response = execute(host, &identity, &Operation::Names)?;
    Ok(environment_choices(&response))
}

fn environment_choices(response: &Value) -> Vec<(String, String)> {
    response
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|environment| {
            let id = environment["id"].as_str()?;
            let label = environment["label"].as_str().unwrap_or(id);
            Some((label.to_owned(), id.to_owned()))
        })
        .collect()
}

impl Root {
    /// Home's `Work in` > `Cloud` submit: `POST /wham/tasks` in the chosen environment and
    /// branch, then the task's `/remote/:taskId` page. A refusal restores the draft.
    pub(crate) fn start_cloud_task(
        &mut self,
        s: ui::composer::Submission,
        options: ui::screens::hotkey::Options,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let hotkey = self.popout.is_some();
        let task = CloudTaskRequest {
            prompt: s.text.clone(),
            branch: options
                .branch
                .filter(|branch| branch != "HEAD")
                .unwrap_or_else(|| "main".into()),
            environment_id: options.environment.unwrap_or_default(),
            model: (!s.model.is_empty()).then(|| s.model.clone()),
            read_only: false,
        };
        cx.spawn_in(window, async move |this, cx| {
            let created = cx
                .background_spawn(async move {
                    created_task_id(&create_cloud_task(host.as_ref(), &task)?)
                })
                .await;
            this.update_in(cx, |this, window, cx| match created {
                Ok(task_id) => this.open_remote_task(task_id, hotkey, window, cx),
                Err(error) => {
                    eprintln!("codex-gpui: cannot create cloud task: {error:#}");
                    this.restore_start_draft(&s, cx);
                    this.toast(
                        ui::toast::Toast::danger("Couldn't create cloud task")
                            .description(error.to_string()),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }
}

pub(crate) fn attach(
    root: &mut Root,
    settings: &Entity<SettingsPage>,
    window: &mut gpui::Window,
    cx: &mut Context<Root>,
) -> Entity<CloudPage> {
    let page = cx.new(CloudPage::new);
    root.cloud_modal = Some(cx.new(|cx| ui::screens::cloud::CloudModal::new(page.clone(), cx)));
    settings.update(cx, |settings, cx| settings.set_cloud(page.clone(), cx));
    let observed = settings.downgrade();
    cx.observe(&page, move |_, _, cx| {
        observed.update(cx, |_, cx| cx.notify()).ok();
    })
    .detach();
    let integrations = root.settings_integrations.clone();
    let gates_ready = integrations.read(cx).gates_loaded;
    page.update(cx, |_, cx| {
        cx.observe(&integrations, |page, state, cx| {
            if state.read(cx).gates_loaded {
                page.load_access(cx);
            }
        })
        .detach();
    });
    let owner: Arc<OnceLock<host::Identity>> = Arc::new(OnceLock::new());
    cx.subscribe_in(
        &page,
        window,
        move |root, page, request: &Request, window, cx| {
            let request = request.clone();
            let host = root.host.clone();
            let bridge = root.bridge.clone();
            let weak = page.downgrade();
            let owner = owner.clone();
            let account = root.activity_access.account.clone();
            let gates = integrations.read(cx).evaluations.clone();
            cx.spawn_in(window, async move |root, cx| {
                if matches!(request.operation, Operation::Access) {
                    let result = async {
                        let account = match account {
                            Some(account) => Some(account),
                            None => bridge.account_read().await?.account,
                        };
                        let plan = match account {
                            Some(bridge::types::Account::Chatgpt { plan_type, .. }) => plan_type,
                            _ => return Ok(Access::default()),
                        };
                        cx.background_spawn(async move {
                            let identity = host
                                .identity()
                                .context("Cloud authentication is unavailable")?;
                            let identity = owner.get_or_init(|| identity);
                            access(host.as_ref(), &plan, identity, &gates)
                        })
                        .await
                    }
                    .await
                    .map_err(|_: anyhow::Error| "Unable to load cloud access".to_owned());
                    weak.update(cx, |page, cx| page.set_access(result, cx)).ok();
                } else {
                    let operation = request.operation.clone();
                    let result = cx
                        .background_spawn(async move {
                            let identity = owner.get().context("Cloud access has not loaded")?;
                            execute(host.as_ref(), identity, &operation)
                        })
                        .await
                        .map_err(|_| error_label(&request.operation).to_owned());
                    let setup = match &request.operation {
                        Operation::ConfigCreate { repositories, .. } => {
                            result.as_ref().ok().and_then(|config| {
                                let id = config["id"].as_str()?.to_owned();
                                Some((id, setup_repositories(config, repositories)))
                            })
                        }
                        Operation::ConfigSetup(id) => {
                            result.as_ref().ok().map(|_| (id.clone(), Vec::new()))
                        }
                        _ => None,
                    };
                    let applied = weak.update(cx, |page, cx| page.apply(&request, result, cx));
                    if let Some((config_id, repositories)) = setup.filter(|_| applied.is_ok()) {
                        root.update_in(cx, |root, window, cx| {
                            root.navigate(
                                crate::Route::CloudSetup {
                                    config_id,
                                    repositories,
                                },
                                window,
                                cx,
                            );
                        })
                        .ok();
                    }
                }
            })
            .detach();
        },
    )
    .detach();
    if gates_ready {
        page.update(cx, |page, cx| page.load_access(cx));
    }
    page
}

impl Root {
    pub fn open_cloud_environment(
        &mut self,
        path: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(
            path == "/cloud-environments/new"
                || path == "/settings/codex-cloud"
                || path == "/settings/cloud-environments"
                || path == "/settings/cloud-environments?tab=preferences"
                || path.starts_with("/settings/cloud-environments/"),
            "Invalid cloud environment route"
        );
        self.navigate(crate::Route::CloudEnvironments(path.into()), window, cx);
        Ok(())
    }
}

fn request(
    host: &dyn host::Host,
    identity: &host::Identity,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    ensure!(
        host.identity().as_ref() == Some(identity),
        "Cloud authentication changed before the request"
    );
    let mut request = json!({
        "url":path,"method":method,
        "headers":{
            "OAI-Language":"en-US","X-OpenAI-Attach-Auth":"1",
            "X-OpenAI-Attach-Integrity-State":"1","originator":"Codex Desktop",
            "ChatGPT-Account-ID":identity.account_id,
        },
        "expectedIdentity":{"accountId":identity.account_id,"userId":identity.user_id},
        "returnErrorResponse":true,
    });
    if path.starts_with("https://codex-cloud-backend.chatgpt.com/") {
        request["headers"]["X-OpenAI-Product-Sku"] = "codex".into();
    }
    if path.starts_with("/aip/") || path.starts_with("/ps/") {
        request["headers"]["OAI-Product-Sku"] = "CODEX".into();
    }
    if let Some(body) = body {
        request["body"] = body.to_string().into();
    }
    let response = host.fetch(&host::Request::Http(request))?;
    let current = host
        .identity()
        .context("Cloud authentication is unavailable")?;
    ensure!(
        current == *identity,
        "Cloud authentication changed during the request"
    );
    let response = response.get("value").unwrap_or(&response);
    let response = response.pointer("/response/__response").unwrap_or(response);
    let status = response["status"]
        .as_u64()
        .context("Cloud response has no status")?;
    if !(200..300).contains(&status) {
        return Err(HttpStatus(status).into());
    }
    if status == 204 {
        return Ok(Value::Null);
    }
    host::json_body(response)
}

fn gate(gates: &Value, name: &str) -> bool {
    gates["feature_gates"][name]["value"] == true
}

pub(crate) fn access(
    host: &dyn host::Host,
    plan: &str,
    identity: &host::Identity,
    gates: &Value,
) -> Result<Access> {
    ensure!(
        gates["feature_gates"].is_object(),
        "Missing Cloud feature gates"
    );
    let consumer = matches!(plan, "free" | "go" | "plus" | "prolite" | "pro" | "promax");
    let policy = if consumer {
        Value::Null
    } else {
        request(
            host,
            identity,
            "GET",
            &path(&["accounts", &identity.account_id, "settings"]),
            None,
        )?
    };
    Ok(evaluate_access(gates, &policy, consumer))
}

fn evaluate_access(gates: &Value, policy: &Value, consumer: bool) -> Access {
    let enabled = gate(gates, "375130565");
    let permission = |name: &str| {
        policy["permissions"]
            .as_array()
            .is_some_and(|values| values.iter().any(|v| v == name))
    };
    let cloud = if !gate(gates, "3769444934") || consumer {
        true
    } else if gate(gates, "1355972088") {
        permission("chatgpt.workspace.feature.allow-codex-access.access")
    } else {
        policy["beta_settings"]["wham_access"] == true
    };
    Access {
        cloud: enabled && cloud,
        legacy: enabled && (consumer || policy["beta_settings"]["wham_access"] == true),
        workspace: !consumer,
        manage_workspace: !consumer
            && cloud
            && (!gate(gates, "1355972088")
                || permission(
                    "chatgpt.workspace.feature.codex-manage-workspace-environments.access",
                )),
        domain_secrets: gate(gates, "479474474"),
        network_allowed: consumer || policy["beta_settings"]["codex_agent_network_access"] == true,
    }
}

fn path(parts: &[&str]) -> String {
    let mut url = Url::parse("https://chatgpt.com/").expect("static URL");
    url.path_segments_mut()
        .expect("HTTP URL")
        .extend(parts.iter().copied());
    url.path().to_owned()
}

pub(crate) fn execute(
    host: &dyn host::Host,
    identity: &host::Identity,
    operation: &Operation,
) -> Result<Value> {
    let (method, url, body) = match operation {
        Operation::List { query, cursor } => {
            let url = environment_list_url(query, cursor.as_deref())?;
            (
                "GET",
                format!("{}?{}", url.path(), url.query().unwrap_or_default()),
                None,
            )
        }
        Operation::Detail(id) => (
            "GET",
            path(&["wham", "environments", id, "with-creator-and-machine"]),
            None,
        ),
        Operation::Machines => ("GET", "/wham/machines".into(), None),
        Operation::Names => {
            return request(host, identity, "GET", "/wham/environments", None).or_else(|error| {
                if error
                    .downcast_ref::<HttpStatus>()
                    .is_some_and(|status| status.0 == 404)
                {
                    Ok(json!([]))
                } else {
                    Err(error)
                }
            });
        }
        Operation::Connections => return github_connections(host, identity),
        Operation::Repositories {
            query,
            page,
            connector,
            link,
        } => {
            let url = repository_url(query, *page, connector.as_deref(), link.as_deref())?;
            (
                "GET",
                format!("{}?{}", url.path(), url.query().unwrap_or_default()),
                None,
            )
        }
        Operation::Save { original, body } => {
            ensure!(body.is_object(), "Invalid cloud environment body");
            (
                if original.is_some() { "PATCH" } else { "POST" },
                original.as_ref().map_or_else(
                    || "/wham/environments".to_owned(),
                    |id| path(&["wham", "environments", id]),
                ),
                Some(body.clone()),
            )
        }
        Operation::Delete(id) => ("DELETE", path(&["wham", "environments", id]), None),
        Operation::Reset(id) => (
            "POST",
            path(&["wham", "environments", id, "reset-cache"]),
            None,
        ),
        Operation::Preferences => {
            return Ok(json!({
                "user": request(host, identity, "GET", "/wham/settings/user", None)?,
                "config": request(host, identity, "GET", "/wham/settings/configs/user-preferences", None)?,
            }));
        }
        Operation::PreferencesSave(body) => {
            ensure!(body.is_object(), "Invalid cloud preference body");
            ("PATCH", "/wham/settings/user".into(), Some(body.clone()))
        }
        Operation::Configs
        | Operation::Vault
        | Operation::Config(_)
        | Operation::ConfigSetup(_)
        | Operation::ConfigDelete(_)
        | Operation::ConfigCreate { .. }
        | Operation::ConfigSave { .. }
        | Operation::Access => return execute_config(host, identity, operation),
    };
    request(host, identity, method, &url, body)
}

fn environment_list_url(query: &str, cursor: Option<&str>) -> Result<Url> {
    let mut url = Url::parse("https://chatgpt.com/wham/environments/search")?;
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("limit", if cursor.is_some() { "100" } else { "25" });
        if !query.is_empty() {
            params.append_pair("query", query);
        }
        if let Some(cursor) = cursor {
            params.append_pair("cursor", cursor);
        }
    }
    Ok(url)
}

fn repository_url(
    query: &str,
    page: u32,
    connector: Option<&str>,
    link: Option<&str>,
) -> Result<Url> {
    let mut url = Url::parse("https://chatgpt.com/wham/github/list-repositories")?;
    if !query.is_empty() {
        url.set_path("/wham/github/repositories/search/all-installations");
    }
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("page", &page.to_string());
        if query.is_empty() {
            params.append_pair("per_page", "20");
        } else {
            params
                .append_pair("query", query)
                .append_pair("limit", "20");
        }
        if let Some(connector) = connector {
            params.append_pair("connector_id", connector);
        }
        if let Some(link) = link {
            params.append_pair("link_id", link);
        }
    }
    Ok(url)
}

fn execute_config(
    host: &dyn host::Host,
    identity: &host::Identity,
    operation: &Operation,
) -> Result<Value> {
    let (method, url, body) = match operation {
        Operation::Configs => return configs(host, identity),
        Operation::Vault => return vault_entries(host, identity),
        Operation::Config(id) | Operation::ConfigSetup(id) => ("GET", cloud_path(&[id]), None),
        Operation::ConfigDelete(id) => ("DELETE", cloud_path(&[id]), None),
        Operation::ConfigCreate { body, .. } => ("POST", cloud_path(&[]), Some(body.clone())),
        Operation::ConfigSave {
            id,
            config,
            changes,
            metadata,
        } => return save_config(host, identity, id, config, changes, metadata),
        Operation::Access => anyhow::bail!("Cloud access uses the account response"),
        _ => unreachable!("legacy operation uses the legacy request dispatcher"),
    };
    request(host, identity, method, &url, body)
}

fn cloud_path(parts: &[&str]) -> String {
    let mut url = Url::parse("https://codex-cloud-backend.chatgpt.com/v1/environment-configs")
        .expect("static cloud URL");
    url.path_segments_mut()
        .expect("HTTP URL")
        .extend(parts.iter().copied());
    url.into()
}

fn vault_entries(host: &dyn host::Host, identity: &host::Identity) -> Result<Value> {
    let mut entries = Vec::new();
    for namespace in ["sensitive", "not_sensitive"] {
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let mut url =
                Url::parse("https://codex-cloud-backend.chatgpt.com/v1/personal-secrets")?;
            url.query_pairs_mut().append_pair("namespace", namespace);
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            let page = request(host, identity, "GET", url.as_str(), None)?;
            for secret in page["secrets"]
                .as_array()
                .context("Invalid personal vault response")?
            {
                entries.push(json!({"namespace":namespace,"secret":secret}));
            }
            cursor = page["next_cursor"].as_str().map(str::to_owned);
            let Some(next) = &cursor else {
                break;
            };
            ensure!(seen.insert(next.clone()), "Personal vault cursor repeated");
        }
    }
    Ok(entries.into())
}

fn configs(host: &dyn host::Host, identity: &host::Identity) -> Result<Value> {
    let mut records = Vec::<Value>::new();
    for scope in ["user", "workspace"] {
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let mut url = Url::parse(&cloud_path(&[]))?;
            url.query_pairs_mut()
                .append_pair("scope", scope)
                .append_pair("limit", "100")
                .append_pair("omitDraft", "true");
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            let page = request(host, identity, "GET", url.as_str(), None)?;
            for record in page["data"]
                .as_array()
                .context("Invalid cloud environment page")?
            {
                let id = record["id"]
                    .as_str()
                    .context("Cloud environment has no id")?;
                if !records.iter().any(|r| r["id"] == id) {
                    records.push(record.clone());
                }
            }
            cursor = page["next_cursor"].as_str().map(str::to_owned);
            let Some(next) = &cursor else {
                break;
            };
            ensure!(
                seen.insert(next.clone()),
                "Cloud environment cursor repeated"
            );
        }
    }
    Ok(records.into())
}

fn save_config(
    host: &dyn host::Host,
    identity: &host::Identity,
    id: &str,
    config: &Value,
    changes: &Value,
    metadata: &Value,
) -> Result<Value> {
    let mut current = config.clone();
    if let Some(fields) = changes.as_object().filter(|v| !v.is_empty()) {
        if let Some(policy) = fields.get("network_policy") {
            ensure!(
                matches!(policy["type"].as_str(), Some("restricted" | "unrestricted")),
                "Invalid network policy"
            );
            if let Some(rules) = policy["egress_rules"].as_array() {
                ensure!(rules.len() <= 1024, "Too many network domains");
                for rule in rules {
                    let host = rule["host"].as_str().context("Invalid network domain")?;
                    ensure!(
                        !host.contains(['/', ':', '?', '#', '@']) && !host.is_empty(),
                        "Invalid network domain"
                    );
                    url::Host::parse(host.strip_prefix("*.").unwrap_or(host))
                        .context("Invalid network domain")?;
                }
            }
        }
        let mut body = fields.clone();
        body.insert(
            "base_version_id".into(),
            config
                .pointer("/draft/base_version_id")
                .filter(|v| !v.is_null())
                .unwrap_or(&config["version_id"])
                .clone(),
        );
        if let Some(revision) = config.pointer("/draft/revision") {
            body.insert("expected_revision".into(), revision.clone());
        }
        current = request(
            host,
            identity,
            "PATCH",
            &cloud_path(&[id, "draft"]),
            Some(body.into()),
        )?;
        ensure!(
            current["id"] == id && current["draft"].is_object(),
            "Invalid saved cloud environment draft"
        );
    }
    if metadata.as_object().is_some_and(|v| !v.is_empty()) {
        if let Some(name) = metadata.get("name") {
            ensure!(
                name.as_str()
                    .is_some_and(|n| !n.trim().is_empty() && n.encode_utf16().count() <= 128),
                "Invalid environment name"
            );
        }
        current = request(
            host,
            identity,
            "PATCH",
            &cloud_path(&[id]),
            Some(metadata.clone()),
        )?;
        ensure!(current["id"] == id, "Invalid saved cloud environment");
    }
    Ok(current)
}

fn github_providers(host: &dyn host::Host, identity: &host::Identity) -> Result<Vec<Value>> {
    let primary = request(
        host,
        identity,
        "GET",
        &format!(
            "/aip/connectors/{GITHUB_CONNECTOR}?include_actions=false&apply_eligibility_filter=false"
        ),
        None,
    );
    let mut providers = match primary {
        Ok(provider) => vec![provider],
        Err(error)
            if error
                .downcast_ref::<HttpStatus>()
                .is_some_and(|status| matches!(status.0, 403 | 404)) =>
        {
            Vec::new()
        }
        Err(error) => return Err(error),
    };
    let mut enterprise = request(
        host,
        identity,
        "GET",
        &format!("/ps/connectors/by-template?template_id={GITHUB_ENTERPRISE_TEMPLATE}"),
        None,
    )?;
    let Value::Array(enterprise) = enterprise["connectors"].take() else {
        anyhow::bail!("Invalid GitHub connector response");
    };
    providers.extend(enterprise);
    providers.retain(|provider| {
        (provider["id"] == GITHUB_CONNECTOR
            || provider["template_id"] == GITHUB_ENTERPRISE_TEMPLATE)
            && provider["labels"]["admin_only_connector"] != "true"
    });
    for provider in &providers {
        ensure!(
            provider["id"].is_string() && provider["name"].is_string(),
            "Invalid GitHub connector"
        );
    }
    providers.sort_by(|a, b| {
        (b["id"] == GITHUB_CONNECTOR)
            .cmp(&(a["id"] == GITHUB_CONNECTOR))
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(providers)
}

fn github_connections(host: &dyn host::Host, identity: &host::Identity) -> Result<Value> {
    let providers = github_providers(host, identity)?;
    let links = request(
        host,
        identity,
        "POST",
        "/aip/connectors/links/list_accessible",
        Some(json!({"principals":[],"link_refresh_strategy":"BLOCKING"})),
    );
    ensure!(
        host.identity().as_ref() == Some(identity),
        "Cloud authentication changed"
    );
    let links = links
        .ok()
        .and_then(|mut response| match response["links"].take() {
            Value::Array(links) => Some(links),
            _ => None,
        });
    let failed = links.is_none();
    let mut links = links.unwrap_or_default();
    let mut connections = Vec::new();
    for mut provider in providers {
        let mut remaining = links
            .iter()
            .filter(|link| link_matches(&provider, link))
            .count();
        if remaining == 0 {
            connections.push(connection_row(provider, None, failed));
            continue;
        }
        for link in &mut links {
            if link_matches(&provider, link) {
                remaining -= 1;
                let row = if remaining == 0 {
                    provider.take()
                } else {
                    provider.clone()
                };
                connections.push(connection_row(row, Some(link.take()), failed));
            }
        }
    }
    Ok(json!({"connections":connections,"failed":failed}))
}

/// The setup chat's repositories. A legacy migration sends `{id, name}` rows without a ref,
/// so each repository and ref then comes from the created config's draft, as in the original.
fn setup_repositories(config: &Value, requested: &[Value]) -> Vec<Value> {
    if requested
        .iter()
        .all(|repository| repository["ref"].is_string())
    {
        return requested.to_vec();
    }
    let draft = config.get("draft").filter(|draft| !draft.is_null());
    draft.unwrap_or(config)["repositories"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|repository| {
            let id = &repository["repository_id"];
            let name = requested
                .iter()
                .find(|requested| requested["id"] == *id)
                .map_or(id, |requested| &requested["name"]);
            json!({"name": name, "ref": repository["ref"]})
        })
        .collect()
}

fn link_matches(provider: &Value, link: &Value) -> bool {
    link["connector_id"].as_str().is_some()
        && link["connector_id"] == provider["id"]
        && link["visibility"] != "HIDDEN"
        && link["id"]
            .as_str()
            .is_some_and(|id| !id.starts_with("implicit_link::"))
}

fn connection_row(mut provider: Value, link: Option<Value>, failed: bool) -> Value {
    provider["linkId"] = link.as_ref().map_or(Value::Null, |link| link["id"].clone());
    provider["link"] = link.unwrap_or(Value::Null);
    provider["loadFailed"] = failed.into();
    provider
}

fn error_label(operation: &Operation) -> &'static str {
    match operation {
        Operation::List { .. } => "Unable to load cloud environments",
        Operation::Detail(_) => "Unable to load this cloud environment",
        Operation::Machines => "Unable to load container images",
        Operation::Names => "Couldn’t check existing names. Retry",
        Operation::Connections => "GitHub connections couldn't load",
        Operation::Repositories { .. } => "Unable to load repositories",
        Operation::Save { original: None, .. } => "Unable to create cloud environment",
        Operation::Save { .. } => "Unable to update cloud environment. Reload and try again",
        Operation::Delete(_) => "Unable to delete cloud environment",
        Operation::Reset(_) => "Unable to reset cloud environment cache",
        Operation::Preferences => "Unable to load cloud preferences",
        Operation::PreferencesSave(_) => "Unable to save cloud preference",
        _ => config_error_label(operation),
    }
}

fn config_error_label(operation: &Operation) -> &'static str {
    match operation {
        Operation::Access => "Unable to load cloud access",
        Operation::Configs => "Couldn’t load saved environments",
        Operation::Vault => "Couldn’t load all personal vault entries",
        Operation::Config(_) | Operation::ConfigSetup(_) => "Couldn’t load environment",
        Operation::ConfigSave { .. } => "Couldn’t save environment. Reload and try again",
        Operation::ConfigDelete(_) => "Couldn’t delete environment",
        Operation::ConfigCreate { .. } => "Could not start cloud environment setup",
        _ => unreachable!("legacy operation uses the legacy error label"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_start_reads_the_created_task_and_environment_choices() {
        assert_eq!(
            created_task_id(&json!({"task": {"id": "task_e_1", "title": "t"}})).unwrap(),
            "task_e_1"
        );
        assert!(created_task_id(&json!({"task": {"id": ""}})).is_err());
        assert!(created_task_id(&json!({})).is_err());
        assert_eq!(
            environment_choices(&json!([
                {"id": "env_1", "label": "web"},
                {"id": "env_2"},
                {"label": "no id"}
            ])),
            [
                ("web".to_owned(), "env_1".to_owned()),
                ("env_2".to_owned(), "env_2".to_owned())
            ]
        );
    }
    #[test]
    fn legacy_migration_takes_refs_from_the_created_draft() {
        let requested = [json!({"id": "repo-1", "name": "openai/codex"})];
        let config = json!({"id": "config", "repositories": [], "draft": {"repositories": [
            {"repository_id": "repo-1", "ref": "main"},
            {"repository_id": "repo-2", "ref": "dev"},
        ]}});
        assert_eq!(
            setup_repositories(&config, &requested),
            [
                json!({"name": "openai/codex", "ref": "main"}),
                json!({"name": "repo-2", "ref": "dev"}),
            ]
        );
        let created = [json!({"name": "openai/codex", "ref": "main"})];
        assert_eq!(setup_repositories(&config, &created), created);
    }
    #[test]
    fn workspace_policy_requires_the_original_cloud_permission() {
        let gates = json!({"feature_gates":{"375130565":{"value":true},"3769444934":{"value":true},"1355972088":{"value":true}}});
        let denied = evaluate_access(&gates, &json!({"permissions":[]}), false);
        assert!(!denied.cloud);
        assert!(!denied.manage_workspace);
        let allowed = evaluate_access(
            &gates,
            &json!({"permissions":["chatgpt.workspace.feature.allow-codex-access.access"]}),
            false,
        );
        assert!(allowed.cloud);
        assert!(!allowed.manage_workspace);
        assert!(evaluate_access(&gates, &Value::Null, true).cloud);
    }

    #[test]
    fn disabled_cloud_gate_hides_legacy_environments_for_every_plan() {
        let policy = json!({"beta_settings":{"wham_access":true}});
        for consumer in [true, false] {
            let disabled = evaluate_access(&json!({}), &policy, consumer);
            assert!(!disabled.cloud);
            assert!(!disabled.legacy);
            let enabled = evaluate_access(
                &json!({"feature_gates":{"375130565":{"value":true}}}),
                &policy,
                consumer,
            );
            assert!(enabled.cloud);
            assert!(enabled.legacy);
        }
    }

    #[test]
    fn github_connections_hide_admin_and_implicit_links() {
        struct ConnectionsHost(std::sync::Mutex<std::collections::VecDeque<Value>>);
        impl host::Host for ConnectionsHost {
            fn fetch(&self, _: &host::Request) -> Result<Value> {
                let body = self.0.lock().unwrap().pop_front().unwrap();
                Ok(json!({"status":200,"body":body.to_string()}))
            }
            fn identity(&self) -> Option<host::Identity> {
                host::Host::identity(&StatusHost(200))
            }
        }
        let host = ConnectionsHost(std::sync::Mutex::new(std::collections::VecDeque::from([
            json!({"id":GITHUB_CONNECTOR,"name":"GitHub"}),
            json!({"connectors":[
                {"id":"work","name":"Enterprise","template_id":GITHUB_ENTERPRISE_TEMPLATE},
                {"id":"admin","template_id":GITHUB_ENTERPRISE_TEMPLATE,
                 "labels":{"admin_only_connector":"true"}}
            ]}),
            json!({"links":[
                {"id":"visible","connector_id":GITHUB_CONNECTOR},
                {"id":"hidden","connector_id":GITHUB_CONNECTOR,"visibility":"HIDDEN"},
                {"id":"implicit_link::system","connector_id":GITHUB_CONNECTOR}
            ]}),
        ])));
        let identity = host::Host::identity(&host).unwrap();
        let result = execute(&host, &identity, &Operation::Connections).unwrap();
        assert_eq!(result["connections"].as_array().unwrap().len(), 2);
        assert_eq!(result["connections"][0]["linkId"], "visible");
        assert_eq!(result["connections"][1]["id"], "work");
        assert!(result["connections"][1]["link"].is_null());
    }

    struct StatusHost(u16);

    impl host::Host for StatusHost {
        fn fetch(&self, _: &host::Request) -> Result<Value> {
            Ok(json!({"status":self.0,"body":"[]"}))
        }

        fn identity(&self) -> Option<host::Identity> {
            Some(host::Identity {
                account_id: "account".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            })
        }
    }

    #[test]
    fn name_lookup_only_treats_not_found_as_empty() {
        let identity = host::Host::identity(&StatusHost(200)).unwrap();
        assert_eq!(
            execute(&StatusHost(404), &identity, &Operation::Names).unwrap(),
            json!([])
        );
        assert!(execute(&StatusHost(403), &identity, &Operation::Names).is_err());
        assert!(execute(&StatusHost(500), &identity, &Operation::Names).is_err());
        assert!(execute(&StatusHost(404), &identity, &Operation::Machines).is_err());
    }

    struct UnreachableHost;

    impl host::Host for UnreachableHost {
        fn fetch(&self, _: &host::Request) -> Result<Value> {
            panic!("A stale page must not send an authenticated request");
        }

        fn identity(&self) -> Option<host::Identity> {
            host::Host::identity(&StatusHost(200))
        }
    }

    #[test]
    fn account_changes_reject_writes_before_host_dispatch() {
        for (account, user) in [("previous-account", "user"), ("account", "previous-user")] {
            let mut owner = host::Host::identity(&StatusHost(200)).unwrap();
            owner.account_id = account.into();
            owner.user_id = user.into();
            assert!(
                execute(
                    &UnreachableHost,
                    &owner,
                    &Operation::Delete("environment".into())
                )
                .is_err()
            );
        }
    }
}
