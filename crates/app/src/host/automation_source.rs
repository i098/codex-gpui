use super::{Host, Identity, Request, json_body};
use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Local,
    Cloud,
}

impl Source {
    pub(super) fn from_params(params: &Value) -> Result<Self> {
        params
            .get("automationSource")
            .map_or(Ok(Self::Local), |source| {
                Ok(serde_json::from_value(source.clone())?)
            })
    }
}

pub(super) fn fetch(host: &dyn Host, endpoint: &str, params: &Value) -> Result<Value> {
    let identity = host
        .identity()
        .context("Cloud automations require a signed-in account")?;
    if !matches!(endpoint, "list-automations" | "read-automation") {
        ensure!(
            params["expectedIdentity"] == identity_value(&identity),
            "Automation account changed"
        );
    }
    match endpoint {
        "list-automations" => list(host, &identity),
        "read-automation" => {
            let id = id(params)?;
            let mut url = url::Url::parse("https://chatgpt.com/automation/")?;
            url.path_segments_mut().unwrap().pop_if_empty().push(id);
            let response = http(host, &identity, "GET", url.path(), None)?;
            ensure!(
                response["automation"]["id"] == id,
                "Cloud returned a different automation"
            );
            Ok(json!({"item": item(response["automation"].clone(), &identity)?}))
        }
        "automation-update" => {
            let id = id(params)?;
            let saved = http(
                host,
                &identity,
                "POST",
                "/automations/save",
                Some(update_body(id, params)?),
            )?;
            ensure!(saved["id"] == id, "Cloud returned a different automation");
            Ok(json!({"item": item(saved, &identity)?}))
        }
        "automation-set-status" => {
            let id = id(params)?;
            let enabled = match params["status"].as_str() {
                Some("ACTIVE") => true,
                Some("PAUSED") => false,
                _ => bail!("Invalid cloud automation status"),
            };
            let saved = http(
                host,
                &identity,
                "POST",
                "/automations/set_status",
                Some(json!({"jawbone_id": id, "is_enabled": enabled})),
            )?;
            ensure!(saved["id"] == id, "Cloud returned a different automation");
            Ok(json!({"item": item(saved, &identity)?}))
        }
        "automation-delete" => {
            http(
                host,
                &identity,
                "POST",
                "/automations/remove",
                Some(json!({"automation_id": id(params)?})),
            )?;
            Ok(json!({"success": true, "status": "deleted"}))
        }
        "automation-run-now" => {
            let mut url = url::Url::parse("https://chatgpt.com/automation/")?;
            url.path_segments_mut()
                .unwrap()
                .pop_if_empty()
                .push(id(params)?)
                .push("run");
            http(
                host,
                &identity,
                "POST",
                url.path(),
                Some(json!({"idempotency_key": crate::uuid_v4()})),
            )?;
            Ok(json!({"success": true}))
        }
        _ => bail!("Unsupported cloud automation endpoint: {endpoint}"),
    }
}

fn list(host: &dyn Host, identity: &Identity) -> Result<Value> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = url::Url::parse("https://chatgpt.com/automations")?;
        url.query_pairs_mut().append_pair("limit", "100");
        if let Some(cursor) = &cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let page = http(
            host,
            identity,
            "GET",
            &url[url::Position::BeforePath..],
            None,
        )?;
        for entry in page["items"]
            .as_array()
            .context("Cloud automation list has no items")?
        {
            if !entry["automation"].is_null() {
                items.push(item(entry["automation"].clone(), identity)?);
            }
        }
        let next = page["cursor"].as_str().filter(|cursor| !cursor.is_empty());
        ensure!(
            next.is_none() || next != cursor.as_deref(),
            "Cloud automation cursor did not advance"
        );
        cursor = next.map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(json!({"items": items, "automationSource": "cloud"}))
}

fn update_body(id: &str, params: &Value) -> Result<Value> {
    let original = &params["cloudAutomation"];
    ensure!(
        original["id"] == id,
        "Cloud automation details do not match the update"
    );
    let mut body = serde_json::Map::new();
    for key in [
        "default_timezone",
        "email_enabled",
        "is_enabled",
        "notifications_enabled",
        "emoji",
        "model",
        "reasoning_effort",
        "schedule",
        "timing_mode",
    ] {
        if let Some(value) = original.get(key) {
            body.insert(key.into(), value.clone());
        }
    }
    body.insert("jawbone_id".into(), json!(id));
    body.insert(
        "title".into(),
        json!(
            params["name"]
                .as_str()
                .context("Missing automation name")?
                .trim()
        ),
    );
    let prompt = params["prompt"]
        .as_str()
        .context("Missing automation prompt")?;
    body.insert(
        "prompt".into(),
        json!(if ui::screens::automations::event_triggered(prompt) {
            prompt
        } else {
            prompt.trim()
        }),
    );
    for (native, cloud) in [("model", "model"), ("reasoningEffort", "reasoning_effort")] {
        if let Some(value) = params.get(native) {
            ensure!(
                value.is_null() || value.is_string(),
                "Invalid automation {native}"
            );
            body.insert(cloud.into(), value.clone());
        }
    }
    let rule = params["rrule"]
        .as_str()
        .context("Missing automation schedule")?;
    let old_schedule = original["schedule"]
        .as_str()
        .context("Missing cloud schedule")?;
    if rule.trim_start_matches("RRULE:") != rrule(old_schedule) {
        ensure!(!rule.trim().is_empty(), "Missing automation schedule");
        let lines: Vec<_> = rule
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                if line.contains(':') {
                    line.to_owned()
                } else {
                    format!("RRULE:{line}")
                }
            })
            .collect();
        body.insert(
            "schedule".into(),
            json!(format!("BEGIN:VEVENT\n{}\nEND:VEVENT", lines.join("\n"))),
        );
    }
    Ok(Value::Object(body))
}

fn id(params: &Value) -> Result<&str> {
    params["id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .context("Missing automation ID")
}

fn identity_value(identity: &Identity) -> Value {
    json!({"accountId": identity.account_id, "userId": identity.user_id})
}

fn http(
    host: &dyn Host,
    identity: &Identity,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    ensure!(
        host.identity().as_ref() == Some(identity),
        "Automation account changed"
    );
    let response = host.fetch(&Request::Http(json!({
        "url": path,
        "method": method,
        "body": body.map(|body| body.to_string()),
        "headers": {
            "oai-did": identity.device_id,
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop",
            "OAI-Product-Sku": "CODEX",
            "ChatGPT-Account-ID": identity.account_id
        },
        "expectedIdentity": identity_value(identity),
        "returnErrorResponse": true
    })))?;
    ensure!(
        host.identity().as_ref() == Some(identity),
        "Automation account changed"
    );
    json_body(&response)
}

fn rrule(schedule: &str) -> &str {
    schedule
        .lines()
        .find(|line| line.starts_with("RRULE:"))
        .unwrap_or(schedule)
        .trim_start_matches("RRULE:")
}

fn item(automation: Value, identity: &Identity) -> Result<Value> {
    let id = automation["id"]
        .as_str()
        .context("Missing cloud automation ID")?;
    let name = automation["title"]
        .as_str()
        .context("Missing cloud automation title")?;
    let prompt = automation["prompt"]
        .as_str()
        .context("Missing cloud automation prompt")?;
    let schedule = automation["schedule"]
        .as_str()
        .context("Missing cloud automation schedule")?;
    let enabled = automation["is_enabled"]
        .as_bool()
        .context("Missing cloud automation status")?;
    let times = automation["next_run_times"]
        .as_array()
        .context("Missing cloud automation run times")?;
    let completed = automation["executor"] != "local"
        && automation["timing_mode"] != "condition_watch"
        && times.is_empty();
    let millis = |value: &Value| {
        value
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|time| time.timestamp_millis())
    };
    Ok(json!({
        "id": id,
        "name": name,
        "prompt": prompt,
        "rrule": rrule(schedule),
        "status": if completed { "COMPLETED" } else if enabled { "ACTIVE" } else { "PAUSED" },
        "createdAt": millis(&automation["created_at"]).unwrap_or(0),
        "nextRunAt": times.first().and_then(millis),
        "model": automation["model"],
        "reasoningEffort": automation["reasoning_effort"],
        "automationSource": "cloud",
        "expectedIdentity": identity_value(identity),
        "cloudAutomation": automation
    }))
}
