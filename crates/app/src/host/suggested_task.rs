use super::LocalHost;
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};

const INSTRUCTIONS: &str = "Help the user set up the suggested scheduled task. Ask for missing details and confirm the schedule before creating anything. Do not execute the task now. Use an automation proposal for the user to review rather than silently creating an automation.";

impl LocalHost {
    pub(super) fn start_suggested_task(&self, suggestion: &Value) -> Result<Value> {
        ensure!(
            suggestion["locked"]["is_locked"] != true,
            "Locked task suggestions require an upgrade before starting."
        );
        let prompt = suggestion["user_message"]
            .as_str()
            .filter(|prompt| !prompt.trim().is_empty())
            .context("The task suggestion has no prompt")?;
        let cwd = suggestion["cwd"].as_str().context("Missing workspace")?;
        let bridge = self
            .bridge
            .as_ref()
            .context("The app-server is unavailable")?;
        let tools = self.app_tools()?;
        let tool_config = tools
            .as_ref()
            .map(|tools| tools.thread_config())
            .transpose()?;
        pollster::block_on(async {
            let models = bridge.model_list().await?;
            let model = models
                .data
                .iter()
                .find(|model| model.is_default && !model.hidden)
                .or_else(|| models.data.iter().find(|model| !model.hidden))
                .context("The app-server has no available model")?;
            let mut instructions = INSTRUCTIONS.to_owned();
            if let Some(hint) = suggestion["system_hint"].as_str() {
                instructions.push_str("\nSuggested task mode: ");
                instructions.push_str(hint);
            }
            let mut params = json!({
                "cwd": cwd,
                "model": model.model,
                "approvalPolicy": "on-request",
                "sandbox": "read-only",
                "threadSource": "automation",
                "developerInstructions": instructions,
            });
            if let Some(config) = tool_config {
                params["config"] = config;
                params["dynamicTools"] = json!([]);
            }
            let started: Value = bridge.request("thread/start", params).await?;
            let id = started["thread"]["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .context("The app-server returned no thread")?;
            if let Some(tools) = &tools {
                tools.register_thread(id);
            }
            let result = bridge.turn_start(id, prompt).await;
            if let Err(error) = result {
                let _ = bridge
                    .request::<_, Value>("thread/archive", json!({"threadId": id}))
                    .await;
                return Err(error);
            }
            Ok(started)
        })
    }
}
