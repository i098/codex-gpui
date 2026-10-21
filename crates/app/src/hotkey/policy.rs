use crate::{Root, cloud};
use anyhow::{Context as _, ensure};
use gpui::{App, AppContext as _, Context};
use serde_json::{Value, json};
use ui::screens::settings::voice::Policy;

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
                let (gates, workspace, consumer) = cloud::policy_inputs(&*host, &identity, &plan)?;
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
