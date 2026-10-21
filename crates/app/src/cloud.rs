use crate::{Root, host};
use anyhow::{Context as _, Result, ensure};
use gpui::{AppContext as _, Context, Entity};
use serde_json::{Value, json};
use ui::screens::{
    cloud::{Access, CloudPage, Operation, Request},
    settings::SettingsPage,
};
use url::Url;

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

pub(crate) fn attach(settings: &Entity<SettingsPage>, cx: &mut Context<Root>) -> Entity<CloudPage> {
    let page = cx.new(CloudPage::new);
    settings.update(cx, |settings, cx| settings.set_cloud(page.clone(), cx));
    let observed = settings.downgrade();
    cx.observe(&page, move |_, _, cx| {
        observed.update(cx, |_, cx| cx.notify()).ok();
    })
    .detach();
    cx.subscribe(&page, |root, page, request: &Request, cx| {
        let request = request.clone();
        let host = root.host.clone();
        let bridge = root.bridge.clone();
        let weak = page.downgrade();
        cx.spawn(async move |_, cx| {
            if matches!(request.operation, Operation::Access) {
                let result = async {
                    let account = bridge.account_read().await?;
                    let plan = match account.account {
                        Some(bridge::types::Account::Chatgpt { plan_type, .. }) => plan_type,
                        _ => return Ok(Access::default()),
                    };
                    cx.background_spawn(async move { access(host.as_ref(), &plan) })
                        .await
                }
                .await
                .map_err(|_: anyhow::Error| "Unable to load cloud access".to_owned());
                weak.update(cx, |page, cx| page.set_access(result, cx)).ok();
            } else {
                let operation = request.operation.clone();
                let result = cx
                    .background_spawn(async move { execute(host.as_ref(), &operation) })
                    .await
                    .map_err(|_| error_label(&request.operation).to_owned());
                weak.update(cx, |page, cx| page.apply(&request, result, cx))
                    .ok();
            }
        })
        .detach();
    })
    .detach();
    page.update(cx, |page, cx| page.load(Operation::Access, cx));
    page
}

pub(crate) fn shell(root: &mut Root, path: &str, cx: &mut Context<Root>) -> Entity<ui::Shell> {
    let section = if path == "/cloud-environments/new" || path == "/settings/codex-cloud" {
        "Codex Cloud"
    } else {
        "Legacy Codex Cloud"
    };
    let mut config = root.config.clone();
    config["host"] = cx
        .try_global::<crate::host_settings::Settings>()
        .map(|s| Value::Object(s.read_all().clone()))
        .unwrap_or(Value::Null);
    let (shell, settings) = ui::screens::settings::shell(&config, &root.skills, Some(section), cx);
    root.settings_page = Some(settings.clone());
    cx.subscribe(
        &settings,
        |root, _, event: &ui::screens::settings::SettingChanged, cx| root.write_setting(event, cx),
    )
    .detach();
    let page = attach(&settings, cx);
    page.update(cx, |page, cx| page.open_path(path, cx));
    shell
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
    if response["status"] == 204 {
        return Ok(Value::Null);
    }
    host::json_body(response)
}

fn gate(gates: &Value, name: &str) -> bool {
    gates["feature_gates"][name]["value"] == true
}

pub(crate) fn access(host: &dyn host::Host, plan: &str) -> Result<Access> {
    let Some(identity) = host.identity() else {
        return Ok(Access::default());
    };
    let bootstrap = request(
        host,
        &identity,
        "POST",
        "/wham/statsig/bootstrap",
        Some(json!({
            "app_session_id":identity.device_id,"app_version":env!("CARGO_PKG_VERSION"),
            "brand_name":"chatgpt","build_flavor":"prod","locale":"en-US",
            "stable_id":identity.device_id,"system_name":std::env::consts::OS,
            "system_version":"","window_type":"electron",
        })),
    )?;
    let gates: Value = serde_json::from_str(
        bootstrap["statsigPayload"]
            .as_str()
            .context("Missing Statsig payload")?,
    )?;
    let consumer = matches!(plan, "free" | "go" | "plus" | "prolite" | "pro" | "promax");
    let policy = if consumer {
        Value::Null
    } else {
        request(
            host,
            &identity,
            "GET",
            &path(&["accounts", &identity.account_id, "settings"]),
            None,
        )?
    };
    Ok(evaluate_access(&gates, &policy, consumer))
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
        legacy: consumer || policy["beta_settings"]["wham_access"] == true,
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

pub(crate) fn execute(host: &dyn host::Host, operation: &Operation) -> Result<Value> {
    let identity = host
        .identity()
        .context("Cloud authentication is unavailable")?;
    let (method, url, body) = match operation {
        Operation::List { query, cursor } => {
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
        Operation::Repositories(query) => {
            let mut url = Url::parse("https://chatgpt.com/wham/github/list-repositories")?;
            if !query.is_empty() {
                url.set_path("/wham/github/repositories/search/all-installations");
            }
            url.query_pairs_mut()
                .append_pair("page", "1")
                .append_pair("per_page", "20");
            if !query.is_empty() {
                url.query_pairs_mut()
                    .append_pair("query", query)
                    .append_pair("limit", "20");
            }
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
        Operation::Configs => return configs(host, &identity),
        Operation::Config(id) => ("GET", cloud_path(&[id]), None),
        Operation::ConfigDelete(id) => ("DELETE", cloud_path(&[id]), None),
        Operation::ConfigCreate(body) => ("POST", cloud_path(&[]), Some(body.clone())),
        Operation::ConfigSave {
            id,
            config,
            changes,
            metadata,
        } => return save_config(host, &identity, id, config, changes, metadata),
        Operation::Access => anyhow::bail!("Cloud access uses the account response"),
    };
    request(host, &identity, method, &url, body)
}

fn cloud_path(parts: &[&str]) -> String {
    let mut url = Url::parse("https://codex-cloud-backend.chatgpt.com/v1/environment-configs")
        .expect("static cloud URL");
    url.path_segments_mut()
        .expect("HTTP URL")
        .extend(parts.iter().copied());
    url.into()
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

fn error_label(operation: &Operation) -> &'static str {
    match operation {
        Operation::List { .. } => "Unable to load cloud environments",
        Operation::Detail(_) => "Unable to load this cloud environment",
        Operation::Machines => "Unable to load container images",
        Operation::Repositories(_) => "Unable to load repositories",
        Operation::Save { original: None, .. } => "Unable to create cloud environment",
        Operation::Save { .. } => "Unable to update cloud environment. Reload and try again",
        Operation::Delete(_) => "Unable to delete cloud environment",
        Operation::Reset(_) => "Unable to reset cloud environment cache",
        Operation::Access => "Unable to load cloud access",
        Operation::Configs => "Couldn’t load saved environments",
        Operation::Config(_) => "Couldn’t load environment",
        Operation::ConfigSave { .. } => "Couldn’t save environment. Reload and try again",
        Operation::ConfigDelete(_) => "Couldn’t delete environment",
        Operation::ConfigCreate(_) => "Could not start cloud environment setup",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
