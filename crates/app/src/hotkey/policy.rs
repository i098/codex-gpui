use crate::{Root, host};
use anyhow::{Context as _, Result, ensure};
use gpui::{App, AppContext as _, Context};
use serde_json::{Value, json};
use ui::screens::settings::voice::Policy;
use url::Url;

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
    if let Some(body) = body {
        request["body"] = body.to_string().into();
    }
    let response = host.fetch(&host::Request::Http(request))?;
    let current = host
        .identity()
        .context("Dictation authentication is unavailable")?;
    ensure!(
        current == *identity,
        "Dictation authentication changed during the request"
    );
    let response = response.get("value").unwrap_or(&response);
    let response = response.pointer("/response/__response").unwrap_or(response);
    if response["status"] == 204 {
        return Ok(Value::Null);
    }
    host::json_body(response)
}

fn policy_inputs(
    host: &dyn host::Host,
    identity: &host::Identity,
    plan: &str,
) -> Result<(Value, Value, bool)> {
    let bootstrap = request(
        host,
        identity,
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
        let mut url = Url::parse("https://chatgpt.com/").expect("static URL");
        url.path_segments_mut().expect("HTTP URL").extend([
            "accounts",
            identity.account_id.as_str(),
            "settings",
        ]);
        request(host, identity, "GET", url.path(), None)?
    };
    Ok((gates, policy, consumer))
}

fn access(gates: &Value, workspace: &Value, consumer: bool, feature_enabled: bool) -> (bool, bool) {
    let enabled = |key: &str| gates["feature_gates"][key]["value"] == true;
    let workspace_allowed = !enabled("770071981")
        || consumer
        || workspace["permissions"]
            .as_array()
            .is_some_and(|permissions| {
                permissions
                    .iter()
                    .any(|permission| permission == "chatgpt.workspace.feature.dictation.access")
            });
    let composer = feature_enabled && workspace_allowed && enabled("4100906017");
    (composer, composer && enabled("1244621283"))
}

pub(crate) fn clear(cx: &mut App) {
    let runtime = cx.global_mut::<super::Runtime>();
    runtime.policy_generation = runtime.policy_generation.wrapping_add(1);
    runtime.policy_task = None;
    let mut policy = cx.try_global::<Policy>().cloned().unwrap_or_default();
    policy.dictation = false;
    policy.global_dictation = false;
    cx.set_global(policy);
}

pub(crate) fn refresh(root: &Root, cx: &mut Context<Root>) {
    if crate::qa::ENABLED || !cfg!(target_os = "macos") || root.popout.is_some() || !root.ready {
        return;
    }
    clear(cx);
    let generation = cx.global::<super::Runtime>().policy_generation;
    let bridge = root.bridge.clone();
    let host = root.host.clone();
    let task = cx.spawn(async move |_, cx| {
        let result = async {
            let account = bridge.account_read().await?;
            let plan = match account.account {
                Some(bridge::types::Account::Chatgpt { plan_type, .. }) => plan_type,
                _ => return Ok::<_, anyhow::Error>(None),
            };
            let mut cursor: Option<String> = None;
            let mut cursors = std::collections::HashSet::new();
            let mut feature_enabled = true;
            loop {
                let page: Value = bridge
                    .request(
                        "experimentalFeature/list",
                        json!({"cursor":cursor,"limit":100}),
                    )
                    .await?;
                let features = page["data"]
                    .as_array()
                    .context("Invalid feature capability response")?;
                feature_enabled &= !features.iter().any(|feature| {
                    feature["name"] == "in_app_dictation" && feature["enabled"] == false
                });
                cursor = page["nextCursor"].as_str().map(str::to_owned);
                let Some(next) = &cursor else { break };
                ensure!(
                    cursors.insert(next.clone()),
                    "Feature capability cursor repeated"
                );
            }
            cx.background_spawn(async move {
                let identity = host
                    .identity()
                    .context("No authenticated dictation identity")?;
                let (gates, workspace, consumer) = policy_inputs(&*host, &identity, &plan)?;
                let (dictation, global_dictation) =
                    access(&gates, &workspace, consumer, feature_enabled);
                Ok(Some((dictation, global_dictation)))
            })
            .await
        }
        .await;
        let _ = cx.update(|cx| {
            if cx.global::<super::Runtime>().policy_generation != generation {
                return;
            }
            match result {
                Ok(Some((dictation, global_dictation))) => {
                    let mut policy = cx.try_global::<Policy>().cloned().unwrap_or_default();
                    policy.dictation = dictation;
                    policy.global_dictation = global_dictation;
                    policy.microphone_selection = true;
                    cx.set_global(policy);
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("codex-gpui: could not load dictation capability: {error:#}")
                }
            }
        });
    });
    cx.global_mut::<super::Runtime>().policy_task = Some(task);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dictation_requires_rollouts_and_workspace_permission() {
        let mut gates = json!({"feature_gates": {
            "4100906017":{"value":true}, "1244621283":{"value":true}, "770071981":{"value":true}
        }});
        assert_eq!(access(&gates, &Value::Null, false, true), (false, false));
        let allowed = json!({"permissions":["chatgpt.workspace.feature.dictation.access"]});
        assert_eq!(access(&gates, &allowed, false, true), (true, true));
        assert_eq!(access(&gates, &Value::Null, true, true), (true, true));
        assert_eq!(access(&gates, &allowed, false, false), (false, false));
        gates["feature_gates"]["1244621283"]["value"] = false.into();
        assert_eq!(access(&gates, &allowed, false, true), (true, false));
    }
}
